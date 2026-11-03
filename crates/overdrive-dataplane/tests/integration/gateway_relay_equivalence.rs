//! Host/Sim `GatewayRelay` equivalence (G1; S-PIG-104).
//!
//! `HostGatewayRelay` (real kernel TLS on loopback against real rustls peers) and
//! `SimGatewayRelay` (in-memory legs on a `SimClock`) are driven through the one
//! `GatewayRelay` port with the same byte and event script and must return the
//! same octets and the same typed errors and leave the destination peer seeing
//! the same bytes and close. Only faults both adapters can produce are compared:
//! peer KeyUpdate (requested and not), close_notify, alerts, an unexpected record,
//! deadlines, source ends mid-body, a stalled destination (backpressure), the
//! close-delimited read and the lingering discard. Substrate lies (probe only),
//! scripted failed KeyUpdate answers and scripted arm refusals exist only on the
//! Sim and are excluded. A NewSessionTicket needs a client-role leg and is left
//! to the exact-peer mTLS suite.
//!
//! Every case also carries the expected results as literals, so two adapters
//! that were wrong in the same way cannot pass: the comparison is the guard
//! against drift, the literals are the oracle.
//!
//! Native metal only (the Host side arms kernel TLS).

#![cfg(target_os = "linux")]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::significant_drop_tightening,
    clippy::missing_const_for_fn,
    reason = "CONTRACT_SHAPE metadata, table-driven real-kernel scenarios and fixture preconditions"
)]

use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::public_ingress::PublicHostname;
use overdrive_core::traits::clock::Clock;
use overdrive_gateway::ports::{GatewayRelay, HeadTerminator, KtlsReadHalf, KtlsWriteHalf};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::gateway::{LegScript, PublicLegScript, SimGatewayRelay};

use super::helpers::relay_peers::{
    Drain, KtlsPeer, Offer, Received, Rig, blocking, nz64, nzu, pattern, server_config,
};

// ---------------------------------------------------------------------------
// Backend-neutral scripts
// ---------------------------------------------------------------------------

/// What the source peer does, in order.
#[derive(Debug, Clone)]
enum Peer {
    Send(Vec<u8>),
    KeyUpdate(bool),
    CloseNotify,
    Alert(u8),
    /// A record of this content type carrying a few octets.
    Record(u8),
    /// No event for this long.
    Quiet(Duration),
}

/// What the relay is asked to do, in order, through the port.
#[derive(Debug, Clone)]
enum Act {
    ReadHead { limit: usize, first_octet: Duration },
    RelayExact { len: u64, no_progress: Duration },
    RelayToEnd { max: u64 },
    Discard { max: u64, absolute: Duration },
    WriteAll(Vec<u8>),
    CloseWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dest {
    /// The destination peer reads everything the relay writes.
    Reading,
    /// The destination peer never reads (backpressure).
    Stalled,
}

struct Case {
    name: &'static str,
    peer: Vec<Peer>,
    acts: Vec<Act>,
    dest: Dest,
    /// The results each act must return, as `Debug` strings.
    expected: Vec<&'static str>,
    /// Octets the destination peer must receive (None: not compared).
    delivered: Option<Vec<u8>>,
    /// The destination peer must see the relay's close_notify.
    dest_close_notify: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    results: Vec<String>,
    delivered: Option<Vec<u8>>,
    dest_close_notify: bool,
}

const HEAD: &[u8] = b"POST /x HTTP/1.1\r\nHost: api.example.com\r\nContent-Length: 100\r\n\r\n";

fn head() -> Vec<u8> {
    HEAD.to_vec()
}

fn quiet() -> Duration {
    Duration::from_millis(1_500)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn read_head(limit: usize, first_octet: Duration) -> Act {
    Act::ReadHead { limit, first_octet }
}

fn head_and(body: &[u8]) -> Vec<u8> {
    let mut v = head();
    v.extend_from_slice(body);
    v
}

fn cases() -> Vec<Case> {
    let body = pattern(0, 100);
    vec![
        Case {
            name: "head_then_exact_body",
            peer: vec![Peer::Send(head_and(&body))],
            acts: vec![
                read_head(32_768, ms(5_000)),
                Act::WriteAll(head()),
                Act::RelayExact { len: 100, no_progress: ms(5_000) },
            ],
            dest: Dest::Reading,
            expected: vec![
                "Ok(\"POST /x HTTP/1.1\\r\\nHost: api.example.com\\r\\nContent-Length: 100\\r\\n\\r\\n\")",
                "Ok(())",
                "Ok(())",
            ],
            delivered: Some(head_and(&body)),
            dest_close_notify: false,
        },
        Case {
            name: "requesting_key_update_inside_the_head",
            peer: vec![
                Peer::Send(head()[..10].to_vec()),
                Peer::KeyUpdate(true),
                Peer::Send(head_and(&body)[10..].to_vec()),
            ],
            acts: vec![
                read_head(32_768, ms(5_000)),
                Act::WriteAll(head()),
                Act::RelayExact { len: 100, no_progress: ms(5_000) },
            ],
            dest: Dest::Reading,
            expected: vec![
                "Ok(\"POST /x HTTP/1.1\\r\\nHost: api.example.com\\r\\nContent-Length: 100\\r\\n\\r\\n\")",
                "Ok(())",
                "Ok(())",
            ],
            delivered: Some(head_and(&body)),
            dest_close_notify: false,
        },
        Case {
            name: "non_requesting_key_update_inside_the_body",
            peer: vec![
                Peer::Send(head_and(&body[..40])),
                Peer::KeyUpdate(false),
                Peer::Send(body[40..].to_vec()),
            ],
            acts: vec![
                read_head(32_768, ms(5_000)),
                Act::WriteAll(head()),
                Act::RelayExact { len: 100, no_progress: ms(5_000) },
            ],
            dest: Dest::Reading,
            expected: vec![
                "Ok(\"POST /x HTTP/1.1\\r\\nHost: api.example.com\\r\\nContent-Length: 100\\r\\n\\r\\n\")",
                "Ok(())",
                "Ok(())",
            ],
            delivered: Some(head_and(&body)),
            dest_close_notify: false,
        },
        Case {
            name: "close_notify_before_the_first_octet_is_a_clean_end",
            peer: vec![Peer::CloseNotify],
            acts: vec![read_head(32_768, ms(5_000))],
            dest: Dest::Reading,
            expected: vec!["Err(CleanEof)"],
            delivered: Some(vec![]),
            dest_close_notify: false,
        },
        Case {
            name: "close_notify_inside_a_head_is_truncated",
            peer: vec![Peer::Send(b"GET /x HT".to_vec()), Peer::CloseNotify],
            acts: vec![read_head(32_768, ms(5_000))],
            dest: Dest::Reading,
            expected: vec!["Err(Truncated)"],
            delivered: Some(vec![]),
            dest_close_notify: false,
        },
        Case {
            name: "alert_inside_a_head_carries_its_description",
            peer: vec![Peer::Send(b"GET /x HT".to_vec()), Peer::Alert(40)],
            acts: vec![read_head(32_768, ms(5_000))],
            dest: Dest::Reading,
            expected: vec!["Err(Alert { description: 40 })"],
            delivered: Some(vec![]),
            dest_close_notify: false,
        },
        Case {
            name: "unexpected_record_type_is_reported",
            peer: vec![Peer::Record(24)],
            acts: vec![read_head(32_768, ms(5_000))],
            dest: Dest::Reading,
            expected: vec!["Err(UnexpectedRecord { content_type: 24 })"],
            delivered: Some(vec![]),
            dest_close_notify: false,
        },
        Case {
            name: "a_head_without_a_terminator_hits_the_limit",
            peer: vec![Peer::Send(vec![b'X'; 100])],
            acts: vec![read_head(64, ms(5_000))],
            dest: Dest::Reading,
            expected: vec!["Err(LimitExceeded { limit: 64 })"],
            delivered: Some(vec![]),
            dest_close_notify: false,
        },
        Case {
            name: "a_silent_peer_ends_the_read_at_its_deadline",
            peer: vec![Peer::Quiet(quiet())],
            acts: vec![read_head(32_768, ms(400))],
            dest: Dest::Reading,
            expected: vec!["Err(Deadline)"],
            delivered: Some(vec![]),
            dest_close_notify: false,
        },
        Case {
            name: "a_source_that_ends_mid_body_reports_the_octets_still_owed",
            peer: vec![Peer::Send(pattern(0, 40)), Peer::CloseNotify],
            acts: vec![Act::RelayExact { len: 100, no_progress: ms(5_000) }],
            dest: Dest::Reading,
            expected: vec!["Err(SourceEndedMidBody { remaining: 60 })"],
            delivered: Some(pattern(0, 40)),
            dest_close_notify: false,
        },
        Case {
            name: "a_stalled_body_ends_at_no_progress",
            peer: vec![Peer::Send(pattern(0, 40)), Peer::Quiet(quiet())],
            acts: vec![Act::RelayExact { len: 100, no_progress: ms(400) }],
            dest: Dest::Reading,
            expected: vec!["Err(NoProgress)"],
            delivered: Some(pattern(0, 40)),
            dest_close_notify: false,
        },
        Case {
            name: "a_stalled_destination_ends_at_no_progress",
            peer: vec![Peer::Send(pattern(0, 16 * 1024 * 1024))],
            acts: vec![Act::RelayExact { len: 16 * 1024 * 1024, no_progress: ms(600) }],
            dest: Dest::Stalled,
            expected: vec!["Err(NoProgress)"],
            delivered: None,
            dest_close_notify: false,
        },
        Case {
            name: "close_delimited_body_ends_on_close_notify_with_its_count",
            peer: vec![Peer::Send(pattern(0, 70)), Peer::CloseNotify],
            acts: vec![Act::RelayToEnd { max: 1_000 }],
            dest: Dest::Reading,
            expected: vec!["Ok(70)"],
            delivered: Some(pattern(0, 70)),
            dest_close_notify: false,
        },
        Case {
            name: "close_delimited_body_over_the_ceiling_fails",
            peer: vec![Peer::Send(pattern(0, 200)), Peer::CloseNotify],
            acts: vec![Act::RelayToEnd { max: 100 }],
            dest: Dest::Reading,
            expected: vec!["Err(LimitExceeded { max: 100 })"],
            delivered: None,
            dest_close_notify: false,
        },
        Case {
            name: "discard_ends_at_the_peers_close_notify",
            peer: vec![Peer::Send(pattern(0, 300)), Peer::CloseNotify],
            acts: vec![Act::Discard { max: 1_000, absolute: Duration::from_secs(5) }],
            dest: Dest::Reading,
            expected: vec!["Ok(300)"],
            delivered: Some(vec![]),
            dest_close_notify: false,
        },
        Case {
            name: "discard_ends_at_max_octets",
            peer: vec![Peer::Send(pattern(0, 100))],
            acts: vec![Act::Discard { max: 100, absolute: Duration::from_secs(5) }],
            dest: Dest::Reading,
            expected: vec!["Ok(100)"],
            delivered: Some(vec![]),
            dest_close_notify: false,
        },
        Case {
            name: "discard_ends_at_the_deadline",
            peer: vec![Peer::Send(pattern(0, 10)), Peer::Quiet(quiet())],
            acts: vec![Act::Discard { max: 1_000, absolute: ms(400) }],
            dest: Dest::Reading,
            expected: vec!["Err(Deadline)"],
            delivered: Some(vec![]),
            dest_close_notify: false,
        },
        Case {
            name: "close_write_sends_close_notify_after_the_written_octets",
            peer: vec![],
            acts: vec![Act::WriteAll(b"final response".to_vec()), Act::CloseWrite],
            dest: Dest::Reading,
            expected: vec!["Ok(())", "Ok(())"],
            delivered: Some(b"final response".to_vec()),
            dest_close_notify: true,
        },
    ]
}

// ---------------------------------------------------------------------------
// The one driver: acts through the port
// ---------------------------------------------------------------------------

/// Runs `acts` against the port. `tick`: on the Sim, advance the clock in 50 ms
/// steps while an act is pending (the test, not the relay, drives logical time).
async fn run_acts(
    relay: &dyn GatewayRelay,
    now: impl Fn() -> Instant + Send + Sync,
    src: &mut KtlsReadHalf,
    dst: &mut KtlsWriteHalf,
    acts: &[Act],
    tick: Option<&SimClock>,
) -> Vec<String> {
    let mut results = Vec::new();
    for act in acts {
        let fut = async {
            match act {
                Act::ReadHead { limit, first_octet } => {
                    let deadlines = overdrive_gateway::ports::ReadDeadlines {
                        first_octet: now() + *first_octet,
                        complete_within: None,
                        complete_by: None,
                    };
                    let r = relay
                        .read_through(src, HeadTerminator::HeaderBlockEnd, nzu(*limit), deadlines)
                        .await;
                    format!("{:?}", r.map(|t| String::from_utf8_lossy(t.as_slice()).into_owned()))
                }
                Act::RelayExact { len, no_progress } => {
                    let deadlines = overdrive_gateway::ports::RelayDeadlines {
                        absolute: now() + Duration::from_secs(30),
                        no_progress: *no_progress,
                    };
                    format!("{:?}", relay.relay_exact(src, dst, nz64(*len), deadlines).await)
                }
                Act::RelayToEnd { max } => {
                    let deadlines = overdrive_gateway::ports::RelayDeadlines {
                        absolute: now() + Duration::from_secs(30),
                        no_progress: Duration::from_secs(5),
                    };
                    format!("{:?}", relay.relay_to_end(src, dst, nz64(*max), deadlines).await)
                }
                Act::Discard { max, absolute } => {
                    let deadlines = overdrive_gateway::ports::RelayDeadlines {
                        absolute: now() + *absolute,
                        no_progress: Duration::from_secs(5),
                    };
                    format!("{:?}", relay.discard_remaining(src, nz64(*max), deadlines).await)
                }
                Act::WriteAll(bytes) => {
                    format!(
                        "{:?}",
                        relay.write_all(dst, bytes, now() + Duration::from_secs(10)).await
                    )
                }
                Act::CloseWrite => {
                    format!("{:?}", relay.close_write(dst, now() + Duration::from_secs(10)).await)
                }
            }
        };
        let mut fut = Box::pin(fut);
        let result = match tick {
            None => fut.await,
            Some(clock) => loop {
                match tokio::time::timeout(ms(2), &mut fut).await {
                    Ok(r) => break r,
                    Err(_) => clock.tick(ms(50)),
                }
            },
        };
        results.push(result);
    }
    results
}

// ---------------------------------------------------------------------------
// Host side
// ---------------------------------------------------------------------------

fn play_peer(peer: KtlsPeer, script: Vec<Peer>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for step in script {
            let outcome = match step {
                Peer::Send(bytes) => peer.send(&bytes),
                Peer::KeyUpdate(requested) => peer.send_key_update(requested),
                Peer::CloseNotify => peer.close_notify(),
                Peer::Alert(description) => peer.alert(description),
                Peer::Record(content_type) => peer.send_raw_record(content_type, b"xx"),
                Peer::Quiet(span) => {
                    std::thread::sleep(span);
                    Ok(())
                }
            };
            if outcome.is_err() {
                return;
            }
        }
    })
}

async fn run_host(case: &Case) -> Outcome {
    let mut rig = Rig::new(Offer::Aes128).await;
    let source_peer = rig.public.peer.clone();
    let player = play_peer(source_peer, case.peer.clone());
    let dest_reader = (case.dest == Dest::Reading).then(|| Drain::start(&rig.upstream.peer, true));

    let Rig { relay, public, upstream } = &mut rig;
    let results = run_acts(
        relay.as_ref(),
        Instant::now,
        &mut public.read,
        &mut upstream.write,
        &case.acts,
        None,
    )
    .await;

    // Let the destination peer finish reading what was written, then collect it.
    tokio::time::sleep(ms(300)).await;
    let (delivered, dest_close_notify) = match dest_reader {
        Some(reader) => {
            let octets = reader.octets();
            let saw = matches!(*reader.ended.lock().unwrap(), Some(Received::CloseNotify));
            (Some(octets), saw)
        }
        None => (None, false),
    };
    drop(rig);
    blocking(move || {
        let _ = player.join();
    })
    .await;
    Outcome {
        results,
        delivered: case.delivered.as_ref().map(|_| delivered.clone().unwrap_or_default()),
        dest_close_notify,
    }
}

// ---------------------------------------------------------------------------
// Sim side
// ---------------------------------------------------------------------------

fn socket_pair_fd() -> (OwnedFd, TcpStream) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("loopback listener");
    let client = TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
    let (server, _) = listener.accept().expect("accept");
    server.set_nonblocking(true).expect("non-blocking");
    (OwnedFd::from(server), client)
}

fn to_script(steps: &[Peer], dest_stalled: bool) -> LegScript {
    let mut script = LegScript::new();
    for step in steps {
        script = match step {
            Peer::Send(bytes) => script.octets(bytes.clone()),
            Peer::KeyUpdate(requested) => script.key_update(*requested),
            Peer::CloseNotify => script.close_notify(),
            Peer::Alert(description) => script.alert(*description),
            Peer::Record(content_type) => script.unexpected_record(*content_type),
            Peer::Quiet(span) => script.quiet(*span),
        };
    }
    if dest_stalled {
        script = script.with_send_window(64 * 1024);
    }
    script
}

async fn run_sim(case: &Case) -> Outcome {
    let clock = Arc::new(SimClock::new());
    let relay = Arc::new(SimGatewayRelay::new(Arc::clone(&clock) as Arc<dyn Clock>));
    let sni: PublicHostname = "api.example.com".parse().expect("SNI");
    relay.script_public_connection(PublicLegScript {
        sni: sni.clone(),
        inbound: to_script(&case.peer, false),
    });
    relay.script_public_connection(PublicLegScript {
        sni,
        inbound: to_script(&[], case.dest == Dest::Stalled),
    });
    let (src_fd, _src_client) = socket_pair_fd();
    let (dst_fd, _dst_client) = socket_pair_fd();
    let deadline = clock.now() + Duration::from_secs(10);
    let src_leg =
        relay.accept_public(src_fd, server_config(), deadline).await.expect("Sim source leg");
    let dst_leg =
        relay.accept_public(dst_fd, server_config(), deadline).await.expect("Sim destination leg");
    let (mut src_read, _src_write) = src_leg.leg.into_halves();
    let (_dst_read, mut dst_write) = dst_leg.leg.into_halves();

    let results = run_acts(
        relay.as_ref(),
        || clock.now(),
        &mut src_read,
        &mut dst_write,
        &case.acts,
        Some(clock.as_ref()),
    )
    .await;

    let transcripts = relay.leg_transcripts();
    let destination = &transcripts[1];
    Outcome {
        results,
        delivered: case.delivered.as_ref().map(|_| destination.written.clone()),
        dest_close_notify: destination.close_notify_sent,
    }
}

// ---------------------------------------------------------------------------
// The equivalence assertion
// ---------------------------------------------------------------------------

async fn assert_equivalent(name: &str) {
    let case = cases().into_iter().find(|c| c.name == name).expect("case exists");
    let expected = Outcome {
        results: case.expected.iter().map(|s| (*s).to_owned()).collect(),
        delivered: case.delivered.clone(),
        dest_close_notify: case.dest_close_notify,
    };
    let host = run_host(&case).await;
    let sim = run_sim(&case).await;
    assert_eq!(
        host.results, expected.results,
        "{name}: HostGatewayRelay results against the literal oracle"
    );
    assert_eq!(
        sim.results, expected.results,
        "{name}: SimGatewayRelay results against the literal oracle"
    );
    assert_eq!(host.results, sim.results, "{name}: Host and Sim return the same results");
    if case.delivered.is_some() {
        assert_eq!(host.delivered, expected.delivered, "{name}: Host destination octets");
        assert_eq!(sim.delivered, expected.delivered, "{name}: Sim destination octets");
    }
    assert_eq!(host.dest_close_notify, expected.dest_close_notify, "{name}: Host close behaviour");
    assert_eq!(sim.dest_close_notify, expected.dest_close_notify, "{name}: Sim close behaviour");
}

macro_rules! equivalence_cases {
    ($($test:ident => $case:literal),+ $(,)?) => {
        $(
            /// S-PIG-104: Host and Sim `GatewayRelay` agree on this script.
            /// CONTRACT_SHAPE: bounded-change.
            #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
            #[ignore = "pending DELIVER kernel-relay-host-adapter"]
            async fn $test() {
                assert_equivalent($case).await;
            }
        )+
    };
}

equivalence_cases! {
    host_and_sim_agree_on_a_head_then_an_exact_body => "head_then_exact_body",
    host_and_sim_agree_on_a_requesting_key_update_inside_the_head => "requesting_key_update_inside_the_head",
    host_and_sim_agree_on_a_non_requesting_key_update_inside_the_body => "non_requesting_key_update_inside_the_body",
    host_and_sim_agree_that_close_notify_before_the_first_octet_is_a_clean_end => "close_notify_before_the_first_octet_is_a_clean_end",
    host_and_sim_agree_that_close_notify_inside_a_head_is_truncated => "close_notify_inside_a_head_is_truncated",
    host_and_sim_agree_on_an_alert_inside_a_head => "alert_inside_a_head_carries_its_description",
    host_and_sim_agree_on_an_unexpected_record_type => "unexpected_record_type_is_reported",
    host_and_sim_agree_on_the_head_limit => "a_head_without_a_terminator_hits_the_limit",
    host_and_sim_agree_on_the_first_octet_deadline => "a_silent_peer_ends_the_read_at_its_deadline",
    host_and_sim_agree_on_a_source_that_ends_mid_body => "a_source_that_ends_mid_body_reports_the_octets_still_owed",
    host_and_sim_agree_on_a_stalled_body => "a_stalled_body_ends_at_no_progress",
    host_and_sim_agree_on_a_stalled_destination => "a_stalled_destination_ends_at_no_progress",
    host_and_sim_agree_on_a_close_delimited_body => "close_delimited_body_ends_on_close_notify_with_its_count",
    host_and_sim_agree_on_a_close_delimited_body_over_the_ceiling => "close_delimited_body_over_the_ceiling_fails",
    host_and_sim_agree_on_a_discard_ended_by_close_notify => "discard_ends_at_the_peers_close_notify",
    host_and_sim_agree_on_a_discard_ended_by_max => "discard_ends_at_max_octets",
    host_and_sim_agree_on_a_discard_ended_by_the_deadline => "discard_ends_at_the_deadline",
    host_and_sim_agree_that_close_write_sends_close_notify => "close_write_sends_close_notify_after_the_written_octets",
}
