//! `HostGatewayRelay` adapter-contract suite on a real kernel
//! (G1, ADR-0239/0244/0246/0247; S-PIG-105 through 112, 114, 115, 117, 118, 123).
//!
//! Native x86_64 metal only (`cargo xtask metal run --`): every test arms real
//! kernel TLS on loopback sockets. Each test builds a [`Rig`] — the relay under
//! test and two legs it accepted (`public`: a client's connection; `upstream`: an
//! origin's connection) — and plays the exchange driver's port-call sequence
//! against it (the Message-framing contract: head, framing lines and write
//! through userspace, body octets by splice). The peers are test-owned real
//! TLS 1.3 endpoints ([`KtlsPeer`]); the oracles are octets compared at the
//! peers, `/proc/net/tls_stat` deltas, `/proc/self/fd` counts and
//! `/proc/self/statm`, none of which the code under test controls.
//!
//! Scenario/oracle table: feature-delta "Cross-boundary scenario obligations",
//! G1 rows 104-123. The production-path (conformance binary) arms of S-PIG-105,
//! 109 and 115 are not in this file.
//!
//! The test-local `relay_message`/`relay_chunked` helpers model exactly the
//! port-call sequence of the DESIGN's Message-framing contract and nothing the
//! exchange driver adds; they exist so each oracle drives the port, not a copy
//! of the relay.
//!
//! Honest limits: the legs are accepted by `accept_public` (the relay is the TLS
//! server on both), because that is the only pinned way for a test outside the
//! crate to obtain an armed leg; the client-role arm of an authenticated
//! upstream (`HostGatewayClientMtls`) and a NewSessionTicket on such a leg are
//! covered by the exact-peer mTLS suite. The port-level server configuration is
//! a fixture built per ADR-0226 (`relay_peers::server_config`); the snapshot's
//! own builder is asserted by the S-PIG-118 unit arm in `overdrive-gateway`.

#![cfg(target_os = "linux")]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::significant_drop_tightening,
    clippy::missing_const_for_fn,
    clippy::similar_names,
    reason = "CONTRACT_SHAPE metadata, long real-kernel scenario bodies and fixture preconditions"
)]

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use overdrive_dataplane::gateway_relay::HostGatewayRelay;
use overdrive_gateway::ports::{
    GatewayRelay, HeadTerminator, KtlsReadHalf, KtlsWriteHalf, LegReadError, PublicHandshakeError,
    RelayError, ThroughBytes,
};

use super::helpers::relay_peers::{
    ALL_AES, Drain, Lcg, Offer, PIPE_CAPACITY, Received, Rig, TlsStat, accept_leg, accept_leg_with,
    accept_raw, blocking, client_config, expect_exact, first_pattern_mismatch, host_relay,
    http_client_config, in_secs, joined, no_sni_name, nz64, nzu, open_fds, pattern, read_within,
    relay_within, rss_kib, send_in_background, sni_name, stream_pattern, tcp_buffer_ceiling,
};

const HEAD_LIMIT: usize = 32 * 1024;

// ---------------------------------------------------------------------------
// The exchange driver's port-call sequence, modelled test-locally
// ---------------------------------------------------------------------------

async fn read_head(relay: &HostGatewayRelay, from: &mut KtlsReadHalf) -> ThroughBytes {
    relay
        .read_through(from, HeadTerminator::HeaderBlockEnd, nzu(HEAD_LIMIT), read_within(20))
        .await
        .expect("a complete head arrives")
}

async fn read_line(relay: &HostGatewayRelay, from: &mut KtlsReadHalf) -> ThroughBytes {
    relay
        .read_through(from, HeadTerminator::LineEnd, nzu(HEAD_LIMIT), read_within(20))
        .await
        .expect("a complete framing line arrives")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Framing {
    None,
    Length(u64),
    Chunked,
}

fn framing_of(head: &[u8]) -> Framing {
    let text = String::from_utf8_lossy(head).to_ascii_lowercase();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("content-length:") {
            let n: u64 = v.trim().parse().expect("test heads carry a decimal length");
            return if n == 0 { Framing::None } else { Framing::Length(n) };
        }
        if line.starts_with("transfer-encoding:") && line.contains("chunked") {
            return Framing::Chunked;
        }
    }
    Framing::None
}

fn parse_chunk_size(line: &[u8]) -> u64 {
    let text = std::str::from_utf8(line).expect("chunk-size line is ASCII");
    let size = text.trim_end_matches("\r\n").split(';').next().unwrap();
    u64::from_str_radix(size.trim(), 16).expect("hex chunk size")
}

/// One message from `from` to `to`: head through userspace, then the body by the
/// framing the head declares. Returns the head. Exactly the Message-framing
/// contract's request-body sequence (the same sequence serves responses).
async fn relay_message(
    relay: &HostGatewayRelay,
    from: &mut KtlsReadHalf,
    to: &mut KtlsWriteHalf,
) -> Vec<u8> {
    let head = read_head(relay, from).await;
    relay.write_all(to, head.as_slice(), in_secs(20)).await.expect("head written");
    match framing_of(head.as_slice()) {
        Framing::None => {}
        Framing::Length(n) => relay
            .relay_exact(from, to, nz64(n), relay_within(60, Duration::from_secs(20)))
            .await
            .expect("Content-Length body relayed"),
        Framing::Chunked => relay_chunked(relay, from, to).await,
    }
    head.as_slice().to_vec()
}

async fn relay_chunked(relay: &HostGatewayRelay, from: &mut KtlsReadHalf, to: &mut KtlsWriteHalf) {
    loop {
        let line = read_line(relay, from).await;
        let size = parse_chunk_size(line.as_slice());
        if size == 0 {
            // Trailer lines are read and dropped; the gateway writes the last chunk.
            loop {
                let trailer = read_line(relay, from).await;
                if trailer.as_slice() == b"\r\n" {
                    break;
                }
            }
            relay.write_all(to, b"0\r\n\r\n", in_secs(20)).await.expect("last chunk");
            return;
        }
        relay
            .write_all(to, format!("{size:x}\r\n").as_bytes(), in_secs(20))
            .await
            .expect("size line");
        relay
            .relay_exact(from, to, nz64(size), relay_within(60, Duration::from_secs(20)))
            .await
            .expect("chunk relayed");
        let crlf = read_line(relay, from).await;
        assert_eq!(crlf.as_slice(), b"\r\n", "the chunk terminator is exactly CRLF");
        relay.write_all(to, b"\r\n", in_secs(20)).await.expect("chunk terminator");
    }
}

// ---------------------------------------------------------------------------
// Message builders
// ---------------------------------------------------------------------------

fn head_cl(method_path: &str, len: usize) -> Vec<u8> {
    format!("{method_path} HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: {len}\r\n\r\n")
        .into_bytes()
}

fn head_chunked(method_path: &str) -> Vec<u8> {
    format!("{method_path} HTTP/1.1\r\nHost: api.example.com\r\nTransfer-Encoding: chunked\r\n\r\n")
        .into_bytes()
}

fn status_cl(len: usize) -> Vec<u8> {
    format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\n\r\n").into_bytes()
}

fn status_chunked() -> Vec<u8> {
    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec()
}

/// Canonical chunked body: lowercase-hex size lines, pattern data, CRLF, last chunk.
fn chunked_wire(chunks: &[usize], mut offset: usize) -> Vec<u8> {
    let mut wire = Vec::new();
    for &size in chunks {
        wire.extend_from_slice(format!("{size:x}\r\n").as_bytes());
        wire.extend_from_slice(&pattern(offset, size));
        wire.extend_from_slice(b"\r\n");
        offset += size;
    }
    wire.extend_from_slice(b"0\r\n\r\n");
    wire
}

/// A complete message: head then body, plus the stream offset after its body.
fn message_cl(method_path: &str, len: usize, offset: usize) -> Vec<u8> {
    let mut wire = head_cl(method_path, len);
    wire.extend_from_slice(&pattern(offset, len));
    wire
}

fn message_chunked(method_path: &str, chunks: &[usize], offset: usize) -> Vec<u8> {
    let mut wire = head_chunked(method_path);
    wire.extend_from_slice(&chunked_wire(chunks, offset));
    wire
}

fn response_cl(len: usize, offset: usize) -> Vec<u8> {
    let mut wire = status_cl(len);
    wire.extend_from_slice(&pattern(offset, len));
    wire
}

fn response_chunked(chunks: &[usize], offset: usize) -> Vec<u8> {
    let mut wire = status_chunked();
    wire.extend_from_slice(&chunked_wire(chunks, offset));
    wire
}

/// Record sizes that straddle heads and chunk-size lines: odd small ones first.
const STRADDLING_RECORDS: [usize; 9] = [7, 1, 3, 16_384, 33, 4_096, 1_500, 9, 12_000];

/// Replaces the rig's upstream leg with a fresh one (one request per upstream connection).
async fn fresh_upstream(rig: &mut Rig, offer: Offer) {
    rig.upstream = accept_leg(rig.relay.as_ref(), offer).await;
}

// ---------------------------------------------------------------------------
// S-PIG-105
// ---------------------------------------------------------------------------

/// One exchange across the rig: the origin on the current upstream leg reads
/// `request` and answers `response`; the relay carries the request head and body
/// public -> upstream, then the response head and body upstream -> public.
async fn exchange(rig: &mut Rig, request: Vec<u8>, response: Vec<u8>) {
    let origin = rig.upstream.peer.clone();
    let origin_task = std::thread::spawn(move || -> io::Result<()> {
        let got = origin.read_exact_data(request.len())?;
        if got != request {
            return Err(io::Error::other("the request differs at the origin"));
        }
        origin.send_in_records(&response, &STRADDLING_RECORDS)
    });
    let Rig { relay, public, upstream } = rig;
    relay_message(relay, &mut public.read, &mut upstream.write).await;
    relay_message(relay, &mut upstream.read, &mut public.write).await;
    joined(origin_task, "the origin received the request octet-exact and answered").await;
}

/// S-PIG-105 (T3 half): a `Content-Length` exchange and a chunked exchange arrive
/// octet-exact at the real peers of both legs across two keep-alive exchanges on
/// one public connection, the second request sent directly behind the first body
/// so its head must come through untouched, for every body length around the
/// 64 KiB pipe capacity (cap-1, cap, cap+1, 2cap, 2cap+1), with records of mixed
/// sizes straddling heads and chunk-size lines, under both AES suites (P-12).
/// Peer-side receive sizes are logged; every descriptor is released afterwards.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER kernel-relay-host-adapter"]
async fn body_octets_are_exact_at_both_peers_across_keep_alive_exchanges_around_the_pipe_capacity()
{
    let lengths = [
        PIPE_CAPACITY - 1,
        PIPE_CAPACITY,
        PIPE_CAPACITY + 1,
        2 * PIPE_CAPACITY,
        2 * PIPE_CAPACITY + 1,
    ];
    for offer in ALL_AES {
        for &len in &lengths {
            let fds_start = open_fds();
            let mut rig = Rig::new(offer).await;

            let chunks = [len / 2 + 1, 1, len / 3 + 5];
            let (request_1, response_1) = (message_cl("POST /one", len, 0), response_cl(len, 7));
            let (request_2, response_2) =
                (message_chunked("POST /two", &chunks, 3), response_chunked(&chunks, 11));

            // The client sends both requests back to back on its one connection and
            // reads both responses; the relay serves them as two exchanges.
            let mut both_requests = request_1.clone();
            both_requests.extend_from_slice(&request_2);
            let mut both_responses = response_1.clone();
            both_responses.extend_from_slice(&response_2);
            let sender =
                send_in_background(&rig.public.peer, both_requests, STRADDLING_RECORDS.to_vec());
            let receiver = expect_exact(&rig.public.peer, both_responses);

            exchange(&mut rig, request_1, response_1).await;
            fresh_upstream(&mut rig, offer).await;
            exchange(&mut rig, request_2, response_2).await;
            joined(sender, "the client sent both requests").await;
            joined(receiver, "the client received both responses octet-exact").await;

            // Tier-3 obligation 16(j): the relay passes no SPLICE_F_MORE, a
            // configuration the spike did not run, so the record sizes the real
            // peers received are recorded (and bounded by the TLS record limit).
            for (peer_name, peer) in [("client", &rig.public.peer), ("origin", &rig.upstream.peer)]
            {
                let sizes = peer.recv_sizes();
                assert!(!sizes.is_empty(), "the {peer_name} received application records");
                assert!(
                    sizes.iter().all(|size| (1..=16_384).contains(size)),
                    "{peer_name}: every application record holds 1..=16384 octets, got {:?}",
                    sizes.iter().filter(|size| !(1..=16_384).contains(*size)).collect::<Vec<_>>(),
                );
                eprintln!(
                    "S-PIG-105 suite {offer:?} len {len}: {peer_name} received {} records, \
                     min {} max {} total {} octets, first {:?}",
                    sizes.len(),
                    sizes.iter().min().copied().unwrap_or(0),
                    sizes.iter().max().copied().unwrap_or(0),
                    sizes.iter().sum::<usize>(),
                    sizes.iter().take(8).collect::<Vec<_>>(),
                );
            }
            drop(rig);
            assert_eq!(
                settle_fds(fds_start).await,
                fds_start,
                "every socket and pipe was released (len {len})"
            );
        }
    }
}

/// Waits (bounded) for the process descriptor count to return to `target`.
async fn settle_fds(target: usize) -> usize {
    let stop = Instant::now() + Duration::from_secs(3);
    loop {
        let now = open_fds();
        if now <= target || Instant::now() >= stop {
            return now;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// ---------------------------------------------------------------------------
// S-PIG-106
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Position {
    InHead,
    InChunkSizeLine,
    AtBoundary,
    InBody,
}

enum Segment {
    Bytes(Vec<u8>),
    KeyUpdate { requested: bool },
}

/// The client's script for a request with a KeyUpdate at `position`, and the
/// octets the origin must receive (every `Bytes`, none of the control events).
fn key_update_script(position: Position, requested: bool) -> (Vec<Segment>, Vec<u8>) {
    let mut segments = Vec::new();
    match position {
        Position::InHead => {
            let head = head_cl("POST /ku", 10);
            let cut = 17;
            segments.push(Segment::Bytes(head[..cut].to_vec()));
            segments.push(Segment::KeyUpdate { requested });
            segments.push(Segment::Bytes(head[cut..].to_vec()));
            segments.push(Segment::Bytes(pattern(0, 10)));
        }
        Position::InChunkSizeLine => {
            segments.push(Segment::Bytes(head_chunked("POST /ku")));
            segments.push(Segment::Bytes(b"1".to_vec()));
            segments.push(Segment::KeyUpdate { requested });
            let mut rest = b"0\r\n".to_vec();
            rest.extend_from_slice(&pattern(0, 0x10));
            rest.extend_from_slice(b"\r\n0\r\n\r\n");
            segments.push(Segment::Bytes(rest));
        }
        Position::AtBoundary => {
            segments.push(Segment::Bytes(head_cl("POST /ku", 10)));
            segments.push(Segment::KeyUpdate { requested });
            segments.push(Segment::Bytes(pattern(0, 10)));
        }
        Position::InBody => {
            segments.push(Segment::Bytes(head_cl("POST /ku", 40_000)));
            segments.push(Segment::Bytes(pattern(0, 17_000)));
            segments.push(Segment::KeyUpdate { requested });
            segments.push(Segment::Bytes(pattern(17_000, 23_000)));
        }
    }
    let expected = segments
        .iter()
        .filter_map(|s| if let Segment::Bytes(b) = s { Some(b.as_slice()) } else { None })
        .flat_map(<[u8]>::iter)
        .copied()
        .collect();
    (segments, expected)
}

/// S-PIG-106: a peer KeyUpdate (requesting and not requesting) inside a head,
/// inside a chunk-size line, at a message boundary and inside a body leaves the
/// relayed octets exact, advances the receive/transmit rekey counters by exactly
/// what the protocol implies, leaves `TlsDecryptError` unchanged, is never
/// forwarded to the other peer, and the gateway originates none (P-17). A
/// negative control receiver that does not re-arm reads `EKEYEXPIRED`. Both AES
/// suites (P-12). The `tls_stat` counters are node-wide and include the two
/// peers' own kernel-TLS sockets, so the expected deltas are the relay's plus the
/// sender's: for one KeyUpdate sent with `requested = r`, `TlsRxRekeyReceived`
/// and `TlsRxRekeyOk` advance by 1 + r (relay receive, plus the sender receiving
/// the answer) and `TlsTxRekeyOk` by 1 + r (the sender's own re-arm, plus the
/// relay's after answering).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER kernel-relay-host-adapter"]
async fn peer_key_update_leaves_octets_exact_and_is_never_forwarded_or_originated() {
    let positions =
        [Position::InHead, Position::InChunkSizeLine, Position::AtBoundary, Position::InBody];
    let mut sent = 0u32;
    for offer in ALL_AES {
        for position in positions {
            for requested in [false, true] {
                sent += 1;
                assert!(sent <= 20, "at most 20 KeyUpdates per run");
                let mut rig = Rig::new(offer).await;
                let (segments, expected) = key_update_script(position, requested);
                let before = TlsStat::read();

                let client = rig.public.peer.clone();
                let origin = rig.upstream.peer.clone();
                let client_task = std::thread::spawn(move || -> io::Result<()> {
                    for segment in segments {
                        match segment {
                            Segment::Bytes(b) => client.send(&b)?,
                            Segment::KeyUpdate { requested } => {
                                client.send_key_update(requested)?;
                            }
                        }
                    }
                    Ok(())
                });
                let origin_expect = expected.clone();
                let origin_task = expect_exact(&origin, origin_expect);
                // The sender reads on its own thread so it can process the relay's answer.
                let client_reader = Drain::start(&rig.public.peer, false);

                let Rig { relay, public, upstream } = &mut rig;
                let head = relay_message(relay, &mut public.read, &mut upstream.write).await;
                assert!(!head.is_empty());
                joined(client_task, "client sent the whole script").await;
                joined(origin_task, "origin received exactly the application octets").await;

                if requested {
                    wait_until(|| rig.public.peer.key_updates_received() >= 1, 3).await;
                }
                let after = TlsStat::read();
                let r = i64::from(requested);
                assert_eq!(
                    after.delta(&before, "TlsDecryptError"),
                    0,
                    "{position:?} requested={requested}"
                );
                assert_eq!(
                    after.delta(&before, "TlsRxRekeyReceived"),
                    1 + r,
                    "{position:?} requested={requested}"
                );
                assert_eq!(
                    after.delta(&before, "TlsRxRekeyOk"),
                    1 + r,
                    "{position:?} requested={requested}"
                );
                assert_eq!(
                    after.delta(&before, "TlsTxRekeyOk"),
                    1 + r,
                    "{position:?} requested={requested}"
                );
                assert_eq!(
                    rig.upstream.peer.key_updates_received(),
                    0,
                    "a KeyUpdate is never forwarded to the other leg"
                );
                assert_eq!(
                    rig.public.peer.key_updates_received(),
                    u32::from(requested),
                    "the only KeyUpdate the sender receives is the answer to its own request (P-17: none originated)"
                );
                drop(client_reader);
            }
        }
    }

    // Negative control: a receiver that fetches the answering KeyUpdate but does
    // not re-arm its receive key reads EKEYEXPIRED, so the oracle above can see a
    // missed re-arm.
    let mut rig = Rig::new(Offer::Aes128).await;
    rig.public.peer.stop_rearming_rx();
    let client = rig.public.peer.clone();
    let sender = std::thread::spawn(move || -> io::Result<()> {
        client.send(&head_cl("POST /control", 0)[..10])?;
        client.send_key_update(true)?;
        client.send(&head_cl("POST /control", 0)[10..])
    });
    let Rig { relay, public, .. } = &mut rig;
    let head = read_head(relay, &mut public.read).await;
    assert_eq!(head.as_slice(), head_cl("POST /control", 0).as_slice());
    joined(sender, "control sender").await;
    relay.write_all(&mut public.write, b"after the answer", in_secs(10)).await.expect("write");
    let control = rig.public.peer.clone();
    let outcome = blocking(move || {
        let mut buf = [0u8; 64];
        control.set_read_timeout(Some(Duration::from_secs(5)));
        let first = control.recv(&mut buf);
        let second = control.recv(&mut buf);
        (first.ok(), second.err().and_then(|e| e.raw_os_error()))
    })
    .await;
    assert_eq!(outcome.0, Some(Received::Handled), "the answer is a KeyUpdate record");
    assert_eq!(outcome.1, Some(libc::EKEYEXPIRED), "without the re-arm the read stays EKEYEXPIRED");
}

async fn wait_until(mut condition: impl FnMut() -> bool, secs: u64) {
    let stop = Instant::now() + Duration::from_secs(secs);
    while !condition() && Instant::now() < stop {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// ---------------------------------------------------------------------------
// S-PIG-107
// ---------------------------------------------------------------------------

/// S-PIG-107: a KeyUpdate request that arrives on the public leg while a 64 MiB
/// splice-out to the same leg is in flight (the reader is throttled so the write
/// spans 5 to 20 s) never tears the record stream: the peer decrypts every octet
/// in order and `TlsDecryptError` is unchanged. The answer is sent after the
/// in-flight write step, never inside it (the transmit lock, assumption A-K5).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER kernel-relay-host-adapter"]
async fn key_update_request_during_a_large_splice_out_never_tears_the_record_stream() {
    const TOTAL: usize = 64 * 1024 * 1024;
    let mut rig = Rig::new(Offer::Aes256).await;
    let before = TlsStat::read();
    let started = Instant::now();

    // The origin streams 64 MiB of pattern at full speed.
    let sent = Arc::new(AtomicU64::new(0));
    let origin_task = stream_pattern(&rig.upstream.peer, 0, TOTAL, Arc::clone(&sent));

    // The client reads the response throttled (64 KiB per 10 ms), and 2 s in
    // sends a requesting KeyUpdate followed by the next request head.
    let client = rig.public.peer.clone();
    let next_head = head_cl("GET /after-key-update", 0);
    let client_next = next_head.clone();
    let client_task = std::thread::spawn(move || -> io::Result<(Duration, u64)> {
        let mut buf = vec![0u8; 64 * 1024];
        let mut got = 0usize;
        let begin = Instant::now();
        let mut sent_update = false;
        while got < TOTAL {
            if !sent_update && begin.elapsed() >= Duration::from_secs(2) {
                client.send_key_update(true)?;
                client.send(&client_next)?;
                sent_update = true;
            }
            match client.recv(&mut buf)? {
                Received::Data(n) => {
                    if let Some(at) = first_pattern_mismatch(got, &buf[..n]) {
                        return Err(io::Error::other(format!("torn or reordered stream at {at}")));
                    }
                    got += n;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Received::Handled => {}
                other => {
                    return Err(io::Error::other(format!("unexpected end {other:?} at {got}")));
                }
            }
        }
        Ok((begin.elapsed(), got as u64))
    });

    let Rig { relay, public, upstream } = &mut rig;
    let (moved, head) = tokio::join!(
        relay.relay_exact(
            &mut upstream.read,
            &mut public.write,
            nz64(TOTAL as u64),
            relay_within(120, Duration::from_secs(30))
        ),
        relay.read_through(
            &mut public.read,
            HeadTerminator::HeaderBlockEnd,
            nzu(HEAD_LIMIT),
            read_within(120)
        ),
    );
    moved.expect("the 64 MiB splice-out completed");
    let head = head.expect("the request head after the KeyUpdate arrived");
    assert_eq!(
        head.as_slice(),
        next_head.as_slice(),
        "the request head sent after the KeyUpdate is exact"
    );

    let (client_elapsed, client_got) = blocking(move || client_task.join().expect("client thread"))
        .await
        .expect("client decrypted every octet");
    joined(origin_task, "origin streamed 64 MiB").await;
    assert_eq!(client_got, TOTAL as u64);
    assert!(
        client_elapsed >= Duration::from_secs(5) && client_elapsed <= Duration::from_secs(20),
        "the throttle held the write across the KeyUpdate window ({client_elapsed:?}, total run {:?})",
        started.elapsed()
    );
    let after = TlsStat::read();
    assert_eq!(after.delta(&before, "TlsDecryptError"), 0, "no torn record reached a decryptor");
    assert_eq!(rig.public.peer.key_updates_received(), 1, "exactly the answer to the one request");
}

// ---------------------------------------------------------------------------
// S-PIG-108
// ---------------------------------------------------------------------------

/// S-PIG-108 (close_notify arms): a close_notify in the middle of a head is
/// `Truncated`, in the middle of a body is `SourceEndedMidBody` carrying the
/// octets still owed, and after a complete message is `CleanEof`: the exchange
/// ends cleanly only where the framing says the message is complete. The
/// NewSessionTicket arm needs a client-role leg (see the module documentation).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER kernel-relay-host-adapter"]
async fn close_notify_is_typed_by_where_it_falls_in_the_message() {
    // Mid-head.
    let mut rig = Rig::new(Offer::Aes128).await;
    let client = rig.public.peer.clone();
    let sender = std::thread::spawn(move || -> io::Result<()> {
        client.send(b"GET /half HT")?;
        client.close_notify()
    });
    let Rig { relay, public, .. } = &mut rig;
    let result = relay
        .read_through(
            &mut public.read,
            HeadTerminator::HeaderBlockEnd,
            nzu(HEAD_LIMIT),
            read_within(10),
        )
        .await;
    joined(sender, "mid-head sender").await;
    assert_eq!(result.err(), Some(LegReadError::Truncated), "close_notify inside a head");

    // Mid-body: 1 000 octets owed, 400 delivered.
    let mut rig = Rig::new(Offer::Aes128).await;
    let client = rig.upstream.peer.clone();
    let sender = std::thread::spawn(move || -> io::Result<()> {
        client.send(&pattern(0, 400))?;
        client.close_notify()
    });
    let Rig { relay, public, upstream } = &mut rig;
    let result = relay
        .relay_exact(
            &mut upstream.read,
            &mut public.write,
            nz64(1_000),
            relay_within(10, Duration::from_secs(5)),
        )
        .await;
    joined(sender, "mid-body sender").await;
    assert_eq!(
        result.err(),
        Some(RelayError::SourceEndedMidBody { remaining: 600 }),
        "close_notify inside a body"
    );

    // After a complete message: the next read is a clean end of stream.
    let mut rig = Rig::new(Offer::Aes256).await;
    let client = rig.public.peer.clone();
    let origin = rig.upstream.peer.clone();
    let message = message_cl("POST /whole", 100, 0);
    let wire = message.clone();
    let sender = std::thread::spawn(move || -> io::Result<()> {
        client.send(&wire)?;
        client.close_notify()
    });
    let origin_task = expect_exact(&origin, message);
    let Rig { relay, public, upstream } = &mut rig;
    relay_message(relay, &mut public.read, &mut upstream.write).await;
    let result = relay
        .read_through(
            &mut public.read,
            HeadTerminator::HeaderBlockEnd,
            nzu(HEAD_LIMIT),
            read_within(10),
        )
        .await;
    joined(sender, "boundary sender").await;
    joined(origin_task, "origin received the whole message").await;
    assert_eq!(result.err(), Some(LegReadError::CleanEof), "close_notify at a message boundary");
}

// ---------------------------------------------------------------------------
// S-PIG-109
// ---------------------------------------------------------------------------

/// S-PIG-109 (T3 half): the upstream read half answers a peer-injected KeyUpdate
/// by the same path as the public half (three requesting injections across one
/// response), relays the response octet-exact with `TlsDecryptError` unchanged,
/// forwards nothing to the client, and originates none (P-17): the origin
/// receives exactly one KeyUpdate record per requesting update it sent and the
/// client receives none. The 16 MiB production leg-C run (`TlsRxRekeyReceived`
/// from leg-C asserted equal to 0, assumption A-K7) is the conformance arm.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER kernel-relay-host-adapter"]
async fn upstream_half_answers_injected_key_updates_and_originates_none() {
    const BODY: usize = 2 * 1024 * 1024;
    let mut rig = Rig::new(Offer::Aes128).await;
    let before = TlsStat::read();
    let head = status_cl(BODY);
    let origin = rig.upstream.peer.clone();
    let origin_head = head.clone();
    let origin_task = std::thread::spawn(move || -> io::Result<()> {
        origin.send(&origin_head)?;
        let mut at = 0;
        let mut injected = 0;
        while at < BODY {
            let n = (64 * 1024).min(BODY - at);
            origin.send(&pattern(at, n))?;
            at += n;
            // Three requesting updates, each inside the body.
            if at == BODY / 4 || at == BODY / 2 || at == 3 * BODY / 4 {
                origin.send_key_update(true)?;
                injected += 1;
            }
        }
        assert_eq!(injected, 3);
        Ok(())
    });
    let expected_head = head.clone();
    let client_task = {
        let client = rig.public.peer.clone();
        std::thread::spawn(move || -> io::Result<()> {
            let got = client.read_exact_data(expected_head.len())?;
            if got != expected_head {
                return Err(io::Error::other("response head differs"));
            }
            client.verify_pattern(0, BODY)
        })
    };
    let origin_reader = Drain::start(&rig.upstream.peer, false);
    let Rig { relay, public, upstream } = &mut rig;
    relay_message(relay, &mut upstream.read, &mut public.write).await;
    joined(origin_task, "origin streamed the response with injected KeyUpdates").await;
    joined(client_task, "client received the response octet-exact").await;
    wait_until(|| rig.upstream.peer.key_updates_received() >= 3, 3).await;

    let after = TlsStat::read();
    assert_eq!(after.delta(&before, "TlsDecryptError"), 0);
    assert_eq!(
        rig.upstream.peer.key_updates_received(),
        3,
        "one answer per requesting update, none originated"
    );
    assert_eq!(rig.public.peer.key_updates_received(), 0, "no KeyUpdate reaches the client");
    drop(origin_reader);
}

// ---------------------------------------------------------------------------
// S-PIG-110
// ---------------------------------------------------------------------------

/// S-PIG-110: a keep-alive connection waiting for its next request holds no pipe
/// descriptor. The public read half's pipe (two descriptors) exists after a body
/// relay and is released when the half starts its next head read, for a
/// `Content-Length` body and for a chunked body (P-16 not adopted: no pipe is
/// held until close).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER kernel-relay-host-adapter"]
async fn a_connection_waiting_for_its_next_request_holds_no_pipe_descriptor() {
    for chunked in [false, true] {
        let mut rig = Rig::new(Offer::Aes128).await;
        let baseline = open_fds();
        let body = 100_000usize;
        let first = if chunked {
            message_chunked("POST /pipe", &[body], 0)
        } else {
            message_cl("POST /pipe", body, 0)
        };
        let next = head_cl("GET /next", 0);
        let client = rig.public.peer.clone();
        let origin = rig.upstream.peer.clone();
        let origin_task = expect_exact(&origin, first.clone());
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let (first_c, next_c) = (first.clone(), next.clone());
        let client_task = std::thread::spawn(move || -> io::Result<()> {
            client.send_in_records(&first_c, &STRADDLING_RECORDS)?;
            gate_rx.recv().map_err(io::Error::other)?;
            client.send(&next_c)
        });
        let Rig { relay, public, upstream } = &mut rig;
        relay_message(relay, &mut public.read, &mut upstream.write).await;
        joined(origin_task, "origin received the first message").await;
        let holding = open_fds();
        assert_eq!(
            holding,
            baseline + 2,
            "the receive half's pipe is two descriptors (chunked={chunked})"
        );

        // The next head read begins; the pipe is released before any octet of it arrives.
        let released = async {
            let stop = Instant::now() + Duration::from_secs(5);
            while open_fds() != baseline && Instant::now() < stop {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let now = open_fds();
            gate_tx.send(()).expect("release the client");
            now
        };
        let (second, fds_while_waiting) =
            tokio::join!(read_head(relay, &mut public.read), released);
        assert_eq!(second.as_slice(), next.as_slice());
        assert_eq!(
            fds_while_waiting, baseline,
            "waiting for the next head holds no pipe (chunked={chunked})"
        );
        joined(client_task, "client").await;
    }
}

// ---------------------------------------------------------------------------
// S-PIG-111
// ---------------------------------------------------------------------------

/// S-PIG-111 (stalled reader): a public reader that stops reading for 3 s freezes
/// the upstream sender within the socket buffers and the two pipes (in-flight
/// octets are bounded by the kernel's autotuning ceilings read at run time plus
/// 2 x the pipe capacity) and resident memory grows by at most 4 MiB; after the
/// reader resumes the whole 256 MiB stream arrives octet-exact.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER kernel-relay-host-adapter"]
async fn a_stalled_reader_freezes_the_sender_within_the_socket_buffers_and_memory_stays_flat() {
    const TOTAL: usize = 256 * 1024 * 1024;
    let mut rig = Rig::new(Offer::Aes256).await;
    let sent = Arc::new(AtomicU64::new(0));
    let origin_task = stream_pattern(&rig.upstream.peer, 0, TOTAL, Arc::clone(&sent));

    // The client reads 1 MiB, stalls 3 s (sampling what the origin got out), then drains.
    let client = rig.public.peer.clone();
    let (sample_tx, sample_rx) = std::sync::mpsc::channel::<(u64, u64, u64)>();
    let sent_probe = Arc::clone(&sent);
    let client_task = std::thread::spawn(move || -> io::Result<()> {
        client.verify_pattern(0, 1024 * 1024)?;
        let rss0 = rss_kib();
        std::thread::sleep(Duration::from_secs(1));
        let at_1s = sent_probe.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_secs(2));
        let at_3s = sent_probe.load(Ordering::SeqCst);
        let rss1 = rss_kib();
        sample_tx.send((at_1s, at_3s, rss1.saturating_sub(rss0))).map_err(io::Error::other)?;
        client.verify_pattern(1024 * 1024, TOTAL - 1024 * 1024)
    });

    let Rig { relay, public, upstream } = &mut rig;
    relay
        .relay_exact(
            &mut upstream.read,
            &mut public.write,
            nz64(TOTAL as u64),
            relay_within(120, Duration::from_secs(10)),
        )
        .await
        .expect("the stream completes once the reader resumes");
    let (at_1s, at_3s, rss_growth_kib) = sample_rx.recv().expect("stall samples");
    joined(client_task, "client received 256 MiB octet-exact").await;
    joined(origin_task, "origin streamed 256 MiB").await;

    // Two hops, each bounded by the sender's wmem ceiling plus the receiver's rmem
    // ceiling, plus the two relay pipes.
    let bound = 2 * (tcp_buffer_ceiling("tcp_wmem") + tcp_buffer_ceiling("tcp_rmem"))
        + 2 * PIPE_CAPACITY as u64;
    assert_eq!(at_1s, at_3s, "the sender is frozen while the reader is stalled");
    assert!(
        at_3s <= 1024 * 1024 + bound,
        "in-flight octets {at_3s} stay within the buffers ({bound})"
    );
    assert!(rss_growth_kib <= 4 * 1024, "resident set grew {rss_growth_kib} KiB while stalled");
}

/// S-PIG-111 (stalled sender): a body that stops arriving ends at the passed
/// no-progress window with `NoProgress`, not at the absolute deadline.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER kernel-relay-host-adapter"]
async fn a_stalled_body_ends_at_the_no_progress_window() {
    let mut rig = Rig::new(Offer::Aes128).await;
    let origin = rig.upstream.peer.clone();
    let sender = std::thread::spawn(move || -> io::Result<()> {
        origin.send(&pattern(0, 1024 * 1024))?;
        std::thread::sleep(Duration::from_secs(3));
        Ok(())
    });
    let client = rig.public.peer.clone();
    let drain = Drain::start(&client, false);
    let Rig { relay, public, upstream } = &mut rig;
    let begin = Instant::now();
    let result = relay
        .relay_exact(
            &mut upstream.read,
            &mut public.write,
            nz64(4 * 1024 * 1024),
            relay_within(30, Duration::from_secs(2)),
        )
        .await;
    let elapsed = begin.elapsed();
    joined(sender, "stalling origin").await;
    assert_eq!(result.err(), Some(RelayError::NoProgress));
    assert!(
        elapsed >= Duration::from_secs(2) && elapsed < Duration::from_millis(3_500),
        "ended at the 2 s no-progress window, not the 30 s absolute deadline ({elapsed:?})"
    );
    drop(drain);
}

/// S-PIG-111 (trickle): a `Content-Length` response delivered one octet at a time
/// reaches the client piece by piece without waiting for a following piece (no
/// `SPLICE_F_MORE`: each splice-out call ends its record); each piece arrives
/// before the next is sent.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER kernel-relay-host-adapter"]
async fn a_trickled_response_reaches_the_client_piece_by_piece() {
    const PIECES: usize = 20;
    let mut rig = Rig::new(Offer::Aes128).await;
    let origin = rig.upstream.peer.clone();
    let send_times = Arc::new(std::sync::Mutex::new(Vec::<Instant>::new()));
    let times = Arc::clone(&send_times);
    let sender = std::thread::spawn(move || -> io::Result<()> {
        for i in 0..PIECES {
            times.lock().unwrap().push(Instant::now());
            origin.send(&pattern(i, 1))?;
            std::thread::sleep(Duration::from_millis(300));
        }
        Ok(())
    });
    let client = rig.public.peer.clone();
    client.set_read_timeout(Some(Duration::from_secs(5)));
    let receive_times = Arc::new(std::sync::Mutex::new(Vec::<Instant>::new()));
    let rt = Arc::clone(&receive_times);
    let receiver = std::thread::spawn(move || -> io::Result<()> {
        for i in 0..PIECES {
            let got = client.read_exact_data(1)?;
            rt.lock().unwrap().push(Instant::now());
            if got[0] != pattern(i, 1)[0] {
                return Err(io::Error::other(format!("piece {i} differs")));
            }
        }
        Ok(())
    });
    let Rig { relay, public, upstream } = &mut rig;
    relay
        .relay_exact(
            &mut upstream.read,
            &mut public.write,
            nz64(PIECES as u64),
            relay_within(60, Duration::from_secs(2)),
        )
        .await
        .expect("trickled body relayed within the no-progress window");
    joined(sender, "trickling origin").await;
    joined(receiver, "receiving client").await;
    let sends = send_times.lock().unwrap().clone();
    let receives = receive_times.lock().unwrap().clone();
    for i in 0..PIECES - 1 {
        assert!(
            receives[i] < sends[i + 1],
            "piece {i} reached the client before piece {} was sent",
            i + 1
        );
    }
}

// ---------------------------------------------------------------------------
// S-PIG-112
// ---------------------------------------------------------------------------

/// S-PIG-112: bodies built from records of mixed sizes (1 octet to 16 KiB, drawn
/// from a seeded generator whose seed is printed) with heads and chunk-size lines
/// straddling record boundaries arrive octet-exact over repeated messages
/// (about 40 MiB in total); a splice-in that crossed a record boundary or moved
/// into a non-empty pipe would lose octets, which exactness over repeated bodies
/// observes (A-K6).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER kernel-relay-host-adapter"]
async fn mixed_record_sizes_straddling_heads_and_chunk_lines_arrive_octet_exact() {
    const SEED: u64 = 0x5eed_0112;
    eprintln!("S-PIG-112 seed {SEED:#x}");
    let mut rng = Lcg(SEED);
    let mut rig = Rig::new(Offer::Aes256).await;

    let mut stream = Vec::new();
    let mut messages = 0usize;
    while stream.len() < 40 * 1024 * 1024 {
        let offset = stream.len();
        let message = if messages.is_multiple_of(2) {
            message_cl("POST /mixed", rng.range(1, 2 * 1024 * 1024), offset)
        } else {
            let count = rng.range(1, 4);
            let chunks: Vec<usize> = (0..count).map(|_| rng.range(1, 700_000)).collect();
            message_chunked("POST /mixed", &chunks, offset)
        };
        stream.extend_from_slice(&message);
        messages += 1;
    }
    let sizes: Vec<usize> = (0..257).map(|_| rng.range(1, 16_384)).collect();
    let client_task = send_in_background(&rig.public.peer, stream.clone(), sizes);
    let origin_task = expect_exact(&rig.upstream.peer, stream);

    let Rig { relay, public, upstream } = &mut rig;
    for _ in 0..messages {
        relay_message(relay, &mut public.read, &mut upstream.write).await;
    }
    joined(client_task, "client sent mixed-size records").await;
    joined(origin_task, "origin received every message octet-exact").await;
}

// ---------------------------------------------------------------------------
// S-PIG-114
// ---------------------------------------------------------------------------

/// S-PIG-114: the probe on the honest kernel passes, and the probe's loopback
/// pair and pipe are released (the open descriptor count returns to its value
/// before the probe). No real-kernel stimulus makes the TLS ULP absent, so this
/// is the only real-kernel arm; the typed ULP refusals are exercised by the Sim
/// substrate lies (S-PIG-99) and the arm-failure typing by S-PIG-98.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "pending DELIVER relay-earned-trust-probe"]
async fn honest_kernel_probe_passes_and_releases_its_descriptors() {
    let relay = host_relay();
    let fds_before_probe = open_fds_after_settle().await;
    relay.probe(in_secs(10)).await.expect("the honest kernel passes the Earned-Trust probe");
    assert_eq!(
        open_fds_after_settle().await,
        fds_before_probe,
        "the probe's loopback pair and pipe are released"
    );
}

async fn open_fds_after_settle() -> usize {
    tokio::time::sleep(Duration::from_millis(200)).await;
    open_fds()
}

// ---------------------------------------------------------------------------
// S-PIG-115
// ---------------------------------------------------------------------------

/// The final responses of the early-answer and pre-exchange rejection cases.
const REJECTIONS: [(&str, &str); 6] = [
    ("413 Payload Too Large", "early origin answer or declared length"),
    ("401 Unauthorized", "early origin answer"),
    ("400 Bad Request", "malformed head"),
    ("404 Not Found", "Route miss"),
    ("421 Misdirected Request", "SNI differs from Host"),
    ("431 Request Header Fields Too Large", "oversize head"),
];

fn rejection(status: &str) -> Vec<u8> {
    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
}

/// S-PIG-115 (T3 half, close_notify arm): a client that keeps sending a body after
/// a final response reads the whole response and then the gateway's close_notify,
/// never a reset, for each rejection the gateway or an origin can send before the
/// request body is read; the discard ends at the peer's close_notify.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER relay-lingering-close"]
async fn lingering_close_lets_the_client_read_the_response_then_close_notify_not_a_reset() {
    for (status, why) in REJECTIONS {
        let mut rig = Rig::new(Offer::Aes128).await;
        let response = rejection(status);
        let client = rig.public.peer.clone();
        let wire = response.clone();
        let reader = std::thread::spawn(move || -> io::Result<()> {
            // The client keeps sending body octets while it reads the response.
            let sender = {
                let client = client.clone();
                std::thread::spawn(move || {
                    for i in 0..64 {
                        if client.send(&pattern(i * 4096, 4096)).is_err() {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                })
            };
            let got = client.read_exact_data(wire.len())?;
            if got != wire {
                return Err(io::Error::other("response differs"));
            }
            let tail = client.read_until_close_notify(0)?;
            let _ = sender.join();
            // A well-behaved client sends close_notify after reading the response.
            client.close_notify()?;
            assert!(tail.is_empty());
            Ok(())
        });
        let Rig { relay, public, .. } = &mut rig;
        relay.write_all(&mut public.write, &response, in_secs(10)).await.expect("response written");
        relay.close_write(&mut public.write, in_secs(10)).await.expect("close_notify sent");
        let dropped = relay
            .discard_remaining(
                &mut public.read,
                nz64(16 * 1024 * 1024),
                relay_within(30, Duration::from_secs(5)),
            )
            .await;
        assert!(
            dropped.is_ok(),
            "{status} ({why}): the discard ends at the peer's close_notify: {dropped:?}"
        );
        joined(reader, &format!("{status}: client read the response then close_notify")).await;
    }
}

/// S-PIG-115 (T3 half, bounds arm): the discard ends at `max` octets, at the
/// passed absolute deadline (which is the drain deadline when a drain begins during
/// a discard) and at the no-progress window. `discard_remaining` returns no count
/// (R-22), so each end is judged by its result class and by what the peer can
/// observe of its leg: after the `max` end the peer has pushed at least `max`
/// octets through and then freezes within the socket buffers (the relay reads
/// nothing more) without a reset; after the deadline and no-progress ends the
/// leg is still open to the peer (the lingering close, not the discard, closes
/// the socket).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER relay-lingering-close"]
async fn the_discard_ends_at_max_octets_at_the_deadline_and_at_the_no_progress_window() {
    const PIECE: usize = 64 * 1024;
    const MAX: u64 = 1024 * 1024;

    // max: a peer that never closes and keeps sending.
    let mut rig = Rig::new(Offer::Aes256).await;
    let client = rig.public.peer.clone();
    let accepted = Arc::new(AtomicU64::new(0));
    let flood = {
        let client = client.clone();
        let accepted = Arc::clone(&accepted);
        std::thread::spawn(move || -> io::Result<()> {
            // 256 MiB: far above `max` plus every socket buffer, so the sender
            // can only stall because the relay stopped reading.
            for i in 0..4096 {
                client.send(&pattern(i * PIECE, PIECE))?;
                accepted.fetch_add(PIECE as u64, Ordering::SeqCst);
            }
            Ok(())
        })
    };
    let Rig { relay, public, .. } = &mut rig;
    relay.close_write(&mut public.write, in_secs(10)).await.expect("close_notify");
    let ended = relay
        .discard_remaining(&mut public.read, nz64(MAX), relay_within(30, Duration::from_secs(5)))
        .await;
    assert_eq!(ended, Ok(()), "the discard ends at max with the Ok class");
    let at_return = accepted.load(Ordering::SeqCst);
    assert!(
        at_return + PIECE as u64 >= MAX,
        "the peer pushed at least max octets through before the discard ended ({at_return})"
    );
    // The relay reads nothing more: the sender freezes within the buffers.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let frozen_at = accepted.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        frozen_at,
        "no further octet is accepted once the discard stopped reading"
    );
    let bound =
        MAX + tcp_buffer_ceiling("tcp_wmem") + tcp_buffer_ceiling("tcp_rmem") + 2 * PIECE as u64;
    assert!(
        frozen_at <= bound,
        "the peer got {frozen_at} octets in, within max + buffers ({bound})"
    );
    assert!(!flood.is_finished(), "the sender is blocked, not failed: the leg was not reset");
    drop(rig);
    let _ = blocking(move || flood.join()).await;

    // deadline (a drain deadline passed as `absolute`): a silent peer.
    let mut rig = Rig::new(Offer::Aes128).await;
    let client = rig.public.peer.clone();
    let Rig { relay, public, .. } = &mut rig;
    let begin = Instant::now();
    let result = relay
        .discard_remaining(
            &mut public.read,
            nz64(1024 * 1024),
            relay_within(1, Duration::from_secs(30)),
        )
        .await;
    assert_eq!(result.err(), Some(LegReadError::Deadline), "ends at the absolute deadline");
    assert!(
        begin.elapsed() >= Duration::from_secs(1) && begin.elapsed() < Duration::from_millis(2_500)
    );
    client.send(&pattern(0, 10)).expect("the leg is still open to the peer after the deadline");

    // no-progress: a peer that sent some octets then went quiet.
    let mut rig = Rig::new(Offer::Aes128).await;
    let client = rig.public.peer.clone();
    client.send(&pattern(0, 100)).expect("some octets, then silence");
    let Rig { relay, public, .. } = &mut rig;
    let begin = Instant::now();
    let result = relay
        .discard_remaining(
            &mut public.read,
            nz64(1024 * 1024),
            relay_within(30, Duration::from_secs(1)),
        )
        .await;
    assert_eq!(
        result.err(),
        Some(LegReadError::Deadline),
        "a stalled discard ends at the no-progress window"
    );
    assert!(
        begin.elapsed() < Duration::from_secs(5),
        "ended within the no-progress window, not at 30 s"
    );
    client.send(&pattern(100, 10)).expect("the leg is still open to the peer after the stall");
}

// ---------------------------------------------------------------------------
// S-PIG-117
// ---------------------------------------------------------------------------

/// S-PIG-117: a close-delimited response (no length) over real kernel TLS is
/// relayed octet-exact and ends cleanly only on the origin's close_notify with
/// the count of octets moved; one over the ceiling fails `LimitExceeded` and
/// never ends cleanly (R-12).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER kernel-relay-host-adapter"]
async fn a_close_delimited_response_ends_cleanly_on_close_notify_and_is_bounded() {
    const BODY: usize = 1024 * 1024;
    let mut rig = Rig::new(Offer::Aes128).await;
    let origin = rig.upstream.peer.clone();
    let sender = std::thread::spawn(move || -> io::Result<()> {
        origin.send_in_records(&pattern(0, BODY), &STRADDLING_RECORDS)?;
        origin.close_notify()
    });
    let client_task = expect_exact(&rig.public.peer, pattern(0, BODY));
    let Rig { relay, public, upstream } = &mut rig;
    let moved = relay
        .relay_to_end(
            &mut upstream.read,
            &mut public.write,
            nz64(16 * 1024 * 1024),
            relay_within(60, Duration::from_secs(10)),
        )
        .await
        .expect("the close-delimited body ends cleanly");
    joined(sender, "origin").await;
    joined(client_task, "client").await;
    assert_eq!(moved, BODY as u64);

    // Over the ceiling: more than `max` octets before any close_notify.
    let mut rig = Rig::new(Offer::Aes256).await;
    let origin = rig.upstream.peer.clone();
    let sender = std::thread::spawn(move || {
        let _ = origin.send(&pattern(0, 300_000));
        let _ = origin.close_notify();
    });
    let drain = Drain::start(&rig.public.peer, false);
    let Rig { relay, public, upstream } = &mut rig;
    let result = relay
        .relay_to_end(
            &mut upstream.read,
            &mut public.write,
            nz64(100_000),
            relay_within(60, Duration::from_secs(10)),
        )
        .await;
    let _ = sender.join();
    assert_eq!(result.err(), Some(RelayError::LimitExceeded { max: 100_000 }));
    drop(drain);
}

// ---------------------------------------------------------------------------
// S-PIG-118
// ---------------------------------------------------------------------------

/// S-PIG-118 (T3 half): clients offering AES-128-GCM and AES-256-GCM complete the
/// handshake and relay octet-exact; a client offering only ChaCha20-Poly1305 is
/// refused before arming with `PublicHandshakeError::Tls` (the kernel TLS arming
/// accepts only the AES-GCM suites, R-8, P-12). The custody snapshot's server
/// configuration listing exactly the two suites is the unit arm in
/// `overdrive-gateway`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER kernel-relay-host-adapter"]
async fn both_aes_suites_relay_exact_and_a_chacha_only_client_is_refused_before_arming() {
    for offer in ALL_AES {
        let mut rig = Rig::new(offer).await;
        let message = message_cl("POST /suite", 50_000, 0);
        let client_task = send_in_background(&rig.public.peer, message.clone(), vec![16_384]);
        let origin_task = expect_exact(&rig.upstream.peer, message);
        let Rig { relay, public, upstream } = &mut rig;
        relay_message(relay, &mut public.read, &mut upstream.write).await;
        joined(client_task, "client").await;
        joined(origin_task, &format!("origin ({offer:?})")).await;
    }

    let relay = host_relay();
    let before = TlsStat::read();
    let refused = accept_leg_with(
        relay.as_ref(),
        client_config(Offer::ChaChaOnly, &[b"http/1.1"]),
        sni_name(),
    )
    .await;
    assert!(
        matches!(refused.err(), Some(PublicHandshakeError::Tls { .. })),
        "a ChaCha-only client is refused with a TLS error"
    );
    let after = TlsStat::read();
    for counter in ["TlsCurrTxSw", "TlsCurrRxSw", "TlsTxSw", "TlsRxSw"] {
        assert_eq!(
            after.delta(&before, counter),
            0,
            "{counter}: nothing was armed for the refused client"
        );
    }
}

// ---------------------------------------------------------------------------
// S-PIG-123
// ---------------------------------------------------------------------------

/// A userspace-only rustls client (no kernel TLS on the peer's side): drives a
/// handshake to completion or failure and reports whether it completed.
fn plain_client_handshake(
    addr: std::net::SocketAddr,
    config: Arc<rustls::ClientConfig>,
    name: rustls::pki_types::ServerName<'static>,
) -> io::Result<()> {
    let mut tcp = std::net::TcpStream::connect(addr)?;
    tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut conn = rustls::ClientConnection::new(config, name).map_err(io::Error::other)?;
    while conn.is_handshaking() {
        conn.complete_io(&mut tcp)?;
    }
    Ok(())
}

/// S-PIG-123: public handshake refusals on a real kernel, each with a real rustls
/// client peer on loopback: ALPN `h2` only, no SNI, silent past the passed
/// deadline, and a reset mid-handshake are refused with the typed variant, no HTTP
/// octet is read, nothing is armed (the `tls_stat` install counters are unchanged)
/// and the descriptor count returns to its start value. The stimulus-to-variant
/// mapping is the one the scenario list names as expected; DELIVER fixes the exact
/// variant per stimulus by observing the production adapter, and the assertion here
/// is that exact variant, never "any error".
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER kernel-relay-host-adapter"]
async fn public_handshake_refusals_are_typed_and_leave_nothing_armed() {
    let relay = host_relay();
    let fds_start = open_fds();
    let before = TlsStat::read();

    // ALPN h2 only.
    let (accepted, _) = accept_raw(relay.as_ref(), in_secs(5), |addr| {
        plain_client_handshake(addr, client_config(Offer::BothAes, &[b"h2"]), sni_name())
    })
    .await;
    assert_eq!(accepted.err(), Some(PublicHandshakeError::AlpnMismatch), "ALPN h2 only");

    // No SNI.
    let (accepted, _) = accept_raw(relay.as_ref(), in_secs(5), |addr| {
        plain_client_handshake(addr, http_client_config(Offer::BothAes), no_sni_name())
    })
    .await;
    assert_eq!(accepted.err(), Some(PublicHandshakeError::ServerNameMissing), "no SNI");

    // Silent until past the passed deadline (a 2 s deadline passed at port level).
    let started = Instant::now();
    let (accepted, silent) = accept_raw(relay.as_ref(), in_secs(2), |addr| {
        let tcp = std::net::TcpStream::connect(addr);
        std::thread::sleep(Duration::from_millis(2_300));
        tcp
    })
    .await;
    assert_eq!(accepted.err(), Some(PublicHandshakeError::Timeout), "silent client");
    assert!(
        started.elapsed() >= Duration::from_secs(2) && started.elapsed() < Duration::from_secs(4)
    );
    drop(silent);

    // Reset mid-handshake: a record header promising 512 octets, then RST.
    let (accepted, ()) = accept_raw(relay.as_ref(), in_secs(5), |addr| {
        use std::io::Write as _;
        let mut tcp = std::net::TcpStream::connect(addr).expect("connect");
        tcp.write_all(&[0x16, 0x03, 0x01, 0x02, 0x00]).expect("partial record header");
        let linger = libc::linger { l_onoff: 1, l_linger: 0 };
        // SAFETY: setsockopt with a live linger struct of the stated size on a live fd.
        let rc = unsafe {
            libc::setsockopt(
                std::os::fd::AsRawFd::as_raw_fd(&tcp),
                libc::SOL_SOCKET,
                libc::SO_LINGER,
                (&raw const linger).cast(),
                std::mem::size_of::<libc::linger>() as libc::socklen_t,
            )
        };
        assert_eq!(rc, 0);
        drop(tcp);
    })
    .await;
    assert!(
        matches!(accepted.err(), Some(PublicHandshakeError::Io { .. })),
        "a reset mid-handshake is an I/O refusal"
    );

    let after = TlsStat::read();
    for counter in ["TlsCurrTxSw", "TlsCurrRxSw", "TlsTxSw", "TlsRxSw"] {
        assert_eq!(
            after.delta(&before, counter),
            0,
            "{counter}: nothing was armed on a refused socket"
        );
    }
    assert_eq!(
        settle_fds(fds_start).await,
        fds_start,
        "descriptor count returned to its start value"
    );
}
