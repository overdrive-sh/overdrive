//! Test-harness roles for the composed transparent-mTLS walking skeleton (step
//! 01-01). These stand in for the WORKER composition-root role (step 07-01) +
//! the workload/peer/client/server processes — they own the intercept setup
//! (cgroup_connect4 attach / nft-TPROXY), the leg-F/leg-C listeners, the
//! `accept()`, and the real TLS peers/workloads. ONLY the accepted leg crosses
//! into the adapter via `InterceptedConnection`; the adapter API is the 4 pinned
//! methods, nothing here is adapter surface.
//!
//! Lifted from the proven spike orchestrators:
//! - `OutboundPeer` ← increment-f `role_peer` (kTLS-RX server that decrypts).
//! - `OutboundWorkload` (the WORKER) ← increment-e relay orchestrator (load BPF,
//!   attach cgroup_connect4_mtls, program MTLS_REDIRECT_DEST, spawn the
//!   cgroup-isolated workload, accept leg F).
//! - `InboundServer` ← increment-i `role_server` (plaintext S, holds nothing).
//! - `InboundWorker` (the WORKER) ← increment-i `role_agent` intercept half
//!   (nft-TPROXY install, IP_TRANSPARENT listener, accept leg C, orig-dst recover)
//!   + `role_client` spawn (the client presenting a client SVID).

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
// The harness prints subprocess diagnostics (the workload/client stderr) on a
// failure path so a flaky Tier-3 run is debuggable from the captured output.
#![allow(clippy::print_stderr)]
#![allow(dead_code)]
// Test-harness role mechanics: raw libc/socket glue (IP_TRANSPARENT, getsockname),
// subprocess plumbing, and TLS peer config. The leg names (leg F/B/C/S) are the
// ADR-0069 contract vocabulary; the FFI-width casts are on compile-time-constant
// struct sizes; the unwraps are the standard test idiom (a panic-with-message is
// the right failure for a precondition).
// `&mut self` on the accept-leg role methods is the harness's lifecycle API
// shape (siblings on the same role take the receiver by mut to drive the spawned
// child / capture state); clippy flags the accept methods that happen not to
// mutate, but splitting the receiver convention per-method would be inconsistent.
#![allow(
    clippy::similar_names,
    clippy::cast_possible_truncation,
    clippy::missing_panics_doc,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::missing_const_for_fn,
    clippy::unused_self,
    clippy::match_same_arms,
    clippy::needless_pass_by_ref_mut,
    reason = "test-harness role mechanics; raw socket glue + subprocess plumbing for the transparent-mTLS Tier-3 gates"
)]

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::SpiffeId;
use rustls::server::WebPkiClientVerifier;

use super::mtls_netns_topology::MtlsTopology;
use super::mtls_pki::TestPki;
use super::traffic::{WireCapture, WireScan};

/// The loopback interface — where the outbound peer leg (peer binds `127.0.0.1`)
/// and the inbound client-facing leg (client → `127.0.0.2` VIRT) physically carry
/// their TLS records, so the AF_PACKET confidentiality oracle captures there.
const LOOPBACK_IFACE: &str = "lo";

/// A multi-record request marker — large enough that kTLS frames it into ≥1 TLS
/// 1.3 application_data record (the forward F→B leg). The workload writes this;
/// the peer must reconstruct it byte-exact after kTLS-RX decrypt.
const OUTBOUND_REQUEST: &[u8] =
    b"OVERDRIVE_OUTBOUND_REQUEST_workload_speaks_first_then_steady_state_must_arrive_TLS13_decrypted_byte_exact_0001";
/// The peer's reply (the return B→F leg, GAP 2). The workload must read it back
/// byte-exact off leg F (proving the return splice).
const OUTBOUND_REPLY: &[u8] =
    b"OVERDRIVE_OUTBOUND_REPLY_peer_responds_return_leg_must_splice_back_to_workload_byte_exact_0002";
/// The inbound client request (C→S). S must receive it byte-exact as plaintext.
const INBOUND_REQUEST: &[u8] =
    b"OVERDRIVE_INBOUND_REQUEST_client_mtls_must_arrive_as_plaintext_at_server_S_agent_light_in_order_0003";
/// The inbound server response (S→C, GAP 2 inbound half). The client must read it
/// back byte-exact over leg C's kTLS.
const INBOUND_RESPONSE: &[u8] =
    b"OVERDRIVE_INBOUND_RESPONSE_server_replies_must_ride_back_over_legC_ktls_to_client_byte_exact_0004";

/// What the outbound round-trip observed (forward F→B + return B→F + RST).
pub struct OutboundRoundTrip {
    pub forward_delivered_byte_exact: bool,
    pub return_delivered_byte_exact: bool,
    pub observed_rst: bool,
}

/// What the inbound server S observed.
pub struct InboundServerResult {
    pub received_request_byte_exact: bool,
    pub observed_rst: bool,
}

/// What the inbound client observed (the S→C response leg).
pub struct InboundClientResult {
    pub received_response_byte_exact: bool,
    pub observed_rst: bool,
    /// The SERVER SPIFFE-id the inbound client extracted from the URI SAN of the
    /// leaf the agent's leg-C SERVER handshake PRESENTED — read from
    /// `conn.peer_certificates()[0]` (chain position 0 = the leaf) after the
    /// handshake completed and verified the server cert chains to the bundle root.
    /// `None` if the handshake never completed or the leaf carried no URI SAN.
    /// This is the inbound counterpart of `OutboundPeer::presented_client_spiffe`:
    /// it proves the agent presented `server_alloc`'s HELD server SVID (read via
    /// `IdentityRead`, AC3 inbound) — surfaced to the test, not swallowed here.
    pub presented_server_spiffe: Option<SpiffeId>,
}

/// Wire observations on a peer/client-facing leg — the confidentiality oracle,
/// bucketed BY DIRECTION (F3/F4). `records_request_dir` counts TLS 1.3 `0x17`
/// application_data records in the request/forward direction (toward the peer-facing
/// port); `records_response_dir` counts them in the response/return direction (from
/// it). `plaintext_marker_hits` counts cleartext appearances of EITHER marker
/// (request or response) on the peer-facing wire in EITHER direction (MUST be 0).
pub struct WireObservations {
    pub records_request_dir: u64,
    pub records_response_dir: u64,
    pub plaintext_marker_hits: u64,
}

// =====================================================================
// OUTBOUND peer — the real mTLS server the agent's leg B dials. Arms kTLS-RX to
// decrypt the workload's request and replies (the B→F response leg).
//
// Productionises increment-f `role_peer`: a tokio + tokio-rustls + ktls TLS-1.3
// server. Presents the peer SVID (chaining to the test CA, DNS SAN PEER_SNI so the
// adapter's leg-B SNI verification passes), arms kTLS-RX, reads the workload's
// request (proving decrypt — plaintext on the wire would break the TLS stream),
// replies. A pcap on the peer-facing veth leg is the confidentiality oracle.
// =====================================================================
pub struct OutboundPeer {
    addr: SocketAddrV4,
    handle: Option<std::thread::JoinHandle<PeerOutcome>>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    /// REAL AF_PACKET capture on `lo` filtered to the peer port — the
    /// confidentiality oracle is derived from these captured bytes (F2), NOT from
    /// the peer's handshake-success bookkeeping. Stopped+scanned by
    /// `wire_observations`; the cached scan answers repeat calls.
    wire: parking_lot::Mutex<WireCaptureState>,
    /// The CLIENT SPIFFE-id the peer's `WebPkiClientVerifier` REQUIRED and the peer
    /// thread extracted from the presented (and verified) client leaf's URI SAN —
    /// written by `outbound_peer_serve` BEFORE it extracts the kTLS secrets, read by
    /// the test via `presented_client_spiffe`. `None` until the handshake completes
    /// (or if the peer never saw a client cert — which cannot happen now the peer
    /// REQUIRES client auth). This is what proves the agent's leg-B client handshake
    /// actually PRESENTED the held client SVID (F1) — surfaced to the test, not
    /// swallowed in the peer thread.
    presented_client_spiffe: Arc<parking_lot::Mutex<Option<SpiffeId>>>,
}

/// Either the live capture (not yet scanned) or the cached scan result.
enum WireCaptureState {
    Live(WireCapture),
    Scanned(WireScan),
}

/// Stop+scan the capture in `state` (on first call) and cache the `WireScan`; repeat
/// calls return the cached scan. The Mutex guard is NOT held across the slow
/// `stop_and_scan` I/O — the capture is taken out under the lock, the lock dropped,
/// the scan run, then the result cached under a fresh lock (single-threaded from the
/// test, so the take→cache window is uncontended).
fn stop_scan_cached(
    state: &parking_lot::Mutex<WireCaptureState>,
    request_marker: &[u8],
    response_marker: &[u8],
) -> WireScan {
    // Phase 1: under the lock, either read the cached scan or take the live capture.
    // Replace the state with a placeholder `Scanned(default)`, drop the lock, then
    // dispatch on the prior state — so the guard's last use is the `mem::replace` and
    // the lock is not held during the slow scan.
    let mut guard = state.lock();
    let prior = std::mem::replace(&mut *guard, WireCaptureState::Scanned(WireScan::default()));
    drop(guard);
    let taken = match prior {
        WireCaptureState::Scanned(s) => {
            // Was already scanned — restore the cached value and return it.
            *state.lock() = WireCaptureState::Scanned(s);
            return s;
        }
        WireCaptureState::Live(capture) => capture,
    };
    // Phase 2: scan WITHOUT holding the lock (the capture I/O is slow).
    let scan = taken.stop_and_scan(request_marker, response_marker);
    // Phase 3: cache the real scan under a fresh lock.
    *state.lock() = WireCaptureState::Scanned(scan);
    scan
}

struct PeerOutcome {
    request_byte_exact: bool,
    plaintext_seen_on_wire: bool,
}

impl OutboundPeer {
    pub fn spawn(pki: &TestPki) -> Self {
        // Bind on loopback; the adapter's leg B dials this real addr. The
        // confidentiality oracle is a REAL AF_PACKET capture on `lo` (the peer leg's
        // wire): it counts genuine TLS 1.3 `0x17` application_data records by walking
        // the record framing and confirms the cleartext request marker never appears
        // — derived from captured bytes, not from the peer's decrypt-success.
        let cert = pki.peer_leaf.cert_der.clone();
        let key = pki.peer_leaf.key_der.clone_key();
        // Present `[leaf, intermediate]`: the agent's leg-B client verifies the peer's
        // server cert against the ROOT anchor only, so the peer must append the
        // intermediate chain material for the path to build (production root →
        // intermediate → leaf shape; F1).
        let intermediate = pki.intermediate_cert_der();
        // REQUIRE+VERIFY the client's SVID against the test CA (F1): the peer is the
        // outbound mTLS server, so it must request and verify the client cert the
        // agent's leg B presents — otherwise the SVID-presentation path is never
        // exercised and this gate would pass even if it were broken. Built from the
        // SAME root-store construction the inbound client uses (`ca_root_store`),
        // mirroring `src/mtls/tls_config::server_config`'s `WebPkiClientVerifier`.
        let client_verifier =
            WebPkiClientVerifier::builder(Arc::new(ca_root_store(pki.ca_cert_pem())))
                .build()
                .expect("peer client-cert verifier");
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("peer bind");
        let addr = match listener.local_addr().expect("peer addr") {
            std::net::SocketAddr::V4(a) => a,
            std::net::SocketAddr::V6(_) => unreachable!("bound on 127.0.0.1"),
        };
        // Start the wire capture BEFORE accepting, so the very first leg-B record is
        // on the wire after the capture is live.
        let capture = WireCapture::start(LOOPBACK_IFACE, addr.port());
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // Shared slot the peer thread writes the verified client SPIFFE-id into (F1).
        let presented_client_spiffe = Arc::new(parking_lot::Mutex::new(None));
        let spiffe_slot = Arc::clone(&presented_client_spiffe);
        let handle = std::thread::spawn(move || {
            outbound_peer_serve(&listener, cert, intermediate, key, client_verifier, &spiffe_slot)
        });
        Self {
            addr,
            handle: Some(handle),
            shutdown,
            wire: parking_lot::Mutex::new(WireCaptureState::Live(capture)),
            presented_client_spiffe,
        }
    }

    pub fn addr(&self) -> SocketAddrV4 {
        self.addr
    }

    /// The CLIENT SPIFFE-id the peer's `WebPkiClientVerifier` REQUIRED + verified and
    /// extracted from the presented client leaf's URI SAN (F1). `None` until the
    /// leg-B handshake has completed; the test reads this after the round-trip to
    /// assert the agent presented the held client SVID and its SAN matches.
    pub fn presented_client_spiffe(&self) -> Option<SpiffeId> {
        self.presented_client_spiffe.lock().clone()
    }

    /// Stop the wire capture (on first call) and scan it for the oracle. The scan is
    /// cached so repeat calls return the same observation. Called by the test AFTER
    /// the request/reply round-trip has completed, so every peer-leg record is on the
    /// captured wire.
    pub fn wire_observations(&self) -> WireObservations {
        // The peer port IS the wire_port. The request (OUTBOUND_REQUEST, forward F→B)
        // flows TOWARD it; the reply (OUTBOUND_REPLY, return B→F) flows FROM it.
        let scan = stop_scan_cached(&self.wire, OUTBOUND_REQUEST, OUTBOUND_REPLY);
        WireObservations {
            records_request_dir: scan.records_to_wire_port,
            records_response_dir: scan.records_from_wire_port,
            plaintext_marker_hits: scan.plaintext_marker_hits,
        }
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// The peer serve loop (synchronous rustls + raw kTLS arm — same shape as the
/// production inbound server side, no tokio/ktls-crate dep): accept leg B, complete
/// the rustls SERVER handshake, arm kTLS-TX+RX via raw `setsockopt`, read the
/// workload's request (decrypted by kTLS-RX — cleartext could not reconstruct it),
/// reply (the B→F return leg, encrypted by kTLS-TX). The byte-exact kTLS-RX
/// reconstruction IS the confidentiality oracle: the request arrived as TLS 1.3
/// application_data, never as cleartext on the peer wire.
fn outbound_peer_serve(
    listener: &TcpListener,
    cert: rustls::pki_types::CertificateDer<'static>,
    intermediate: rustls::pki_types::CertificateDer<'static>,
    key: rustls::pki_types::PrivateKeyDer<'static>,
    client_verifier: Arc<dyn rustls::server::danger::ClientCertVerifier>,
    spiffe_slot: &parking_lot::Mutex<Option<SpiffeId>>,
) -> PeerOutcome {
    let Some((tcp, _)) = accept_with_timeout(listener, Duration::from_secs(10)) else {
        return PeerOutcome { request_byte_exact: false, plaintext_seen_on_wire: false };
    };
    tcp.set_nodelay(true).ok();
    let fd = tcp.as_raw_fd();
    // REQUIRE+VERIFY the client SVID against the test CA (F1): without this the peer
    // never asks for the client cert, and the gate would pass even if the agent's
    // leg-B SVID-presentation path were broken. Mirrors the production inbound
    // server side (`src/mtls/tls_config::server_config`).
    // Present `[leaf, intermediate]` so the agent's root-anchor-only leg-B verifier
    // builds `leaf → intermediate → root` (F1).
    let mut cfg = rustls::ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(vec![cert, intermediate], key)
        .expect("peer server config");
    cfg.enable_secret_extraction = true;
    cfg.send_tls13_tickets = 0; // raw kTLS-RX hits EIO on a post-handshake ticket record
    let mut tcp = tcp;
    tcp.set_read_timeout(Some(Duration::from_secs(8))).ok();
    let mut conn = rustls::ServerConnection::new(Arc::new(cfg)).expect("peer ServerConnection");
    if !drive_server_handshake(&mut conn, &mut tcp) {
        return PeerOutcome { request_byte_exact: false, plaintext_seen_on_wire: false };
    }
    // F1: read the presented (and verifier-accepted) client cert and record its URI
    // SAN as a SpiffeId — BEFORE `dangerous_extract_secrets` consumes the connection
    // (the same Mechanics #6 ordering the production inbound `server_handshake` uses).
    // The verifier already REQUIRED + chain-verified it; this extracts the identity
    // so the TEST can assert it equals the held client SVID's SPIFFE.
    if let Some(spiffe) = peer_presented_client_spiffe(&conn) {
        *spiffe_slot.lock() = Some(spiffe);
    }
    // Drain any 0.5-RTT early plaintext rustls decrypted while finishing the
    // handshake BEFORE extract consumes the connection — those bytes seed `got` so
    // the peer never loses an early-arriving forward record (kTLS early-data
    // correctness; mirrors the production reader legs' `drain_early_plaintext`).
    let mut got = drain_early_plaintext(&mut conn.reader());
    let secrets = conn.dangerous_extract_secrets().expect("peer extract secrets");
    arm_ktls_raw(fd, &secrets);
    std::mem::forget(tcp); // keep the fd open for the kTLS read/write

    // Read the workload's request off the kTLS-RX leg (decrypted by the kernel).
    let stream = unsafe { TcpStream::from_raw_fd(fd) };
    stream.set_read_timeout(Some(Duration::from_secs(8))).ok();
    let mut buf = vec![0u8; 4096];
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while got.len() < OUTBOUND_REQUEST.len() && std::time::Instant::now() < deadline {
        match (&stream).read(&mut buf) {
            Ok(0) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(_) => break,
        }
    }
    let request_byte_exact = got == OUTBOUND_REQUEST;
    if !request_byte_exact {
        // On a forward-delivery miss, name how many bytes arrived (a partial forward
        // splice is the canonical Tier-3 flake signature) so a captured failure is
        // debuggable from the output.
        eprintln!(
            "OUTBOUND-PEER: forward miss — received {} of {} request bytes",
            got.len(),
            OUTBOUND_REQUEST.len()
        );
    }

    // The wire oracle is NOT derived from this decrypt-success bookkeeping — the REAL
    // AF_PACKET capture on `lo` (owned by `OutboundPeer`) counts the genuine `0x17`
    // records and confirms the cleartext marker is absent. The peer's byte-exact
    // reconstruction drives `forward_delivered_byte_exact` (the workload's exit code).

    // Reply (the return B→F leg, GAP 2) — encrypted by kTLS-TX. Reply ONLY when the
    // request reconstructed byte-exact, so the workload's exit code reflects forward
    // success: a partial/missing forward → no reply → the workload's read-reply
    // times out → exit != 0 → `forward_delivered_byte_exact == false`. This keeps
    // the workload-side assertion (line 176) and the peer-wire assertion (line 192)
    // consistent rather than letting the workload succeed on an unconditional reply.
    if request_byte_exact {
        // F4: split the reply into TWO post-arm writes with an inter-write delay
        // larger than the agent's decrypt-pump poll window (40 ms), so kTLS-TX frames
        // ≥2 distinct TLS records on the return leg B (B→F direction). The workload
        // reconstructs the concatenation byte-exact (its read loop accumulates until
        // the full marker length).
        let mid = OUTBOUND_REPLY.len() / 2;
        let _ = (&stream).write_all(&OUTBOUND_REPLY[..mid]);
        let _ = (&stream).flush();
        std::thread::sleep(Duration::from_millis(150));
        let _ = (&stream).write_all(&OUTBOUND_REPLY[mid..]);
        let _ = (&stream).flush();
    }
    std::thread::sleep(Duration::from_millis(600));

    PeerOutcome { request_byte_exact, plaintext_seen_on_wire: false }
}

/// Extract the CLIENT SPIFFE-id (URI SAN) from the leaf certificate the peer's
/// `WebPkiClientVerifier` REQUIRED + verified — the F1 identity proof. Reads
/// `conn.peer_certificates()` (which must be called BEFORE
/// `dangerous_extract_secrets` consumes the connection), parses the leaf DER with
/// `x509-parser`, and returns the sole URI SAN parsed as a `SpiffeId`. `None` if no
/// client cert was presented (cannot happen now the peer REQUIRES client auth) or
/// the leaf carries no URI SAN. Mirrors the workspace's established URI-SAN
/// extraction (`overdrive-host` `rcgen_ca_chain_verify` test).
fn peer_presented_client_spiffe(conn: &rustls::ServerConnection) -> Option<SpiffeId> {
    peer_presented_leaf_spiffe(conn.peer_certificates())
}

/// Extract the SPIFFE-id (sole URI SAN) from chain position 0 (the leaf) of a
/// presented certificate chain. Direction-agnostic — the OUTBOUND peer feeds it
/// `ServerConnection::peer_certificates()` (the verified CLIENT leaf), the INBOUND
/// client feeds it `ClientConnection::peer_certificates()` (the verified SERVER
/// leaf). Returns `None` when no chain was presented or the leaf carries no URI
/// SAN. Mirrors the workspace's established URI-SAN extraction (`overdrive-host`
/// `rcgen_ca_chain_verify` test).
fn peer_presented_leaf_spiffe(
    certs: Option<&[rustls::pki_types::CertificateDer<'_>]>,
) -> Option<SpiffeId> {
    use x509_parser::prelude::FromDer as _;

    let leaf = certs?.first()?;
    let (_, parsed) = x509_parser::certificate::X509Certificate::from_der(leaf.as_ref()).ok()?;
    let san = parsed.subject_alternative_name().ok()??;
    let uri = san.value.general_names.iter().find_map(|gn| match gn {
        x509_parser::extensions::GeneralName::URI(uri) => Some(*uri),
        _ => None,
    })?;
    uri.parse::<SpiffeId>().ok()
}

/// Drain every byte of already-decrypted plaintext rustls buffered during the
/// handshake, BEFORE `dangerous_extract_secrets` consumes the connection — the
/// test-harness mirror of the production `mtls::drain_early_plaintext`. The peer's
/// `read_seq` already counts these records, so the kTLS-RX arm resumes at the next
/// on-wire record; without this the peer would lose any 0.5-RTT forward record the
/// proxy's leg-B client sent coalesced with its `Finished`.
fn drain_early_plaintext(reader: &mut dyn Read) -> Vec<u8> {
    let mut early = Vec::new();
    let mut buf = [0u8; 16384];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => early.extend_from_slice(&buf[..n]),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
    early
}

/// Drive a rustls SERVER handshake to completion (synchronous); false on failure.
fn drive_server_handshake(conn: &mut rustls::ServerConnection, tcp: &mut TcpStream) -> bool {
    use std::io::ErrorKind;
    loop {
        while conn.wants_write() {
            if conn.write_tls(tcp).is_err() {
                return false;
            }
        }
        if !conn.is_handshaking() {
            while conn.wants_write() {
                if conn.write_tls(tcp).is_err() {
                    return false;
                }
            }
            return true;
        }
        match conn.read_tls(tcp) {
            Ok(0) => return false,
            Ok(_) => {
                if conn.process_new_packets().is_err() {
                    return false;
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(_) => return false,
        }
    }
}

/// Arm kTLS-TX+RX on `fd` from rustls-extracted secrets via raw `setsockopt`
/// (mirrors the spike + the production `mtls::ktls`; AES-256-GCM TLS 1.3).
fn arm_ktls_raw(fd: RawFd, secrets: &rustls::ExtractedSecrets) {
    let ulp = b"tls\0";
    // SAFETY: 3-byte "tls" ULP option on a connected fd.
    let rc = unsafe { libc::setsockopt(fd, libc::SOL_TCP, libc::TCP_ULP, ulp.as_ptr().cast(), 3) };
    assert!(rc == 0, "peer TCP_ULP: {}", std::io::Error::last_os_error());
    set_crypto_info(fd, libc::TLS_TX, &secrets.tx);
    set_crypto_info(fd, libc::TLS_RX, &secrets.rx);
}

fn set_crypto_info(fd: RawFd, dir: libc::c_int, sec: &(u64, rustls::ConnectionTrafficSecrets)) {
    use rustls::ConnectionTrafficSecrets;
    #[repr(C)]
    struct Info {
        version: u16,
        cipher: u16,
        iv: [u8; 8],
        key: [u8; 32],
        salt: [u8; 4],
        rec_seq: [u8; 8],
    }
    let (seq, traffic) = sec;
    let ConnectionTrafficSecrets::Aes256Gcm { key, iv } = traffic else {
        panic!("peer kTLS arm requires AES-256-GCM TLS 1.3");
    };
    let ivb = iv.as_ref();
    let mut info = Info {
        version: 0x0304,
        cipher: 52,
        iv: [0; 8],
        key: [0; 32],
        salt: [0; 4],
        rec_seq: seq.to_be_bytes(),
    };
    info.key.copy_from_slice(key.as_ref());
    info.salt.copy_from_slice(&ivb[0..4]);
    info.iv.copy_from_slice(&ivb[4..12]);
    // SAFETY: `Info` is `#[repr(C)]` matching `tls12_crypto_info_aes_gcm_256`.
    let rc = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_TLS,
            dir,
            std::ptr::from_ref(&info).cast(),
            std::mem::size_of::<Info>() as libc::socklen_t,
        )
    };
    assert!(rc == 0, "peer SOL_TLS dir={dir}: {}", std::io::Error::last_os_error());
}

// =====================================================================
// OUTBOUND workload + WORKER — loads BPF, attaches cgroup_connect4_mtls to the
// workload cgroup, programs MTLS_REDIRECT_DEST[real_peer -> leg-F listener],
// spawns the cgroup-isolated workload (into the netns), accepts leg F.
//
// Productionises increment-e relay orchestrator. The workload is a real
// cgroup-isolated subprocess (python3) in the workload netns; its connect() to the
// real peer addr is transparently rewritten by cgroup_connect4_mtls to the agent's
// leg-F listener (bound on the host veth IP, reachable from the netns). Leg F is an
// ORDINARY accepted socket — the forward path is the adapter's agent-light
// splice(legF -> legB) pump, NOT a sockmap egress redirect, so there is no sockops
// enroll / verdict attach / FPORT / agent-cgroup setup here.
// =====================================================================
pub struct OutboundWorkload {
    _bpf: aya::Ebpf,
    _connect_link: aya::programs::cgroup_sock_addr::CgroupSockAddrLink,
    leg_f_listener: TcpListener,
    workload: Option<Child>,
    handshake_delay: Duration,
}

/// Fixed fake-peer address the workload aims at (rewritten by cgroup_connect4_mtls
/// to the agent's leg-F listener). The adapter's leg B dials the REAL peer addr.
const OUTBOUND_FAKE_PEER: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 200);
const OUTBOUND_FAKE_PEER_PORT: u16 = 9443;

impl OutboundWorkload {
    pub fn run(topo: &MtlsTopology, real_peer: SocketAddrV4, handshake_delay: Duration) -> Self {
        // The leg-F listener binds on the host veth IP (10.66.0.1) so the workload
        // in the netns can reach it over the veth. cgroup_connect4_mtls rewrites the
        // workload's connect(fake_peer) -> (host_veth_ip, leg_f_port). Leg F is an
        // ordinary accepted socket (the forward path is the adapter's read->write_all
        // copy pump into leg B's kTLS TX), so no agent cgroup / sockops enroll / FPORT
        // setup is needed.
        let host_veth_ip = Ipv4Addr::new(10, 66, 0, 1);

        let leg_f_listener =
            TcpListener::bind((host_veth_ip, 0)).expect("leg-F listener bind on host veth");
        let leg_f_port = leg_f_listener.local_addr().expect("leg-F addr").port();

        // Load the embedded BPF object, attach cgroup_connect4_mtls to the WORKLOAD
        // cgroup subtree (F5 — workload subtree only; the agent's own leg-B dial
        // runs on the host, outside this cgroup, so it is never re-intercepted).
        // The shared object also carries the phase-2 SERVICE_MAP HoM, which aya
        // 0.13.x cannot create from the ELF alone — so pre-pin it by name into a
        // test-owned bpffs dir and load with `map_pin_path` (the same `pinning =
        // ByName` workaround the adapter uses; `.claude/rules/development.md`
        // § "Sharing the outer HoM … `pinning = ByName`").
        let obj = build_bpf_object_path();
        let mut bpf = load_workload_bpf(&obj);
        let cgroup_file =
            std::fs::File::open(topo.cgroup_path()).expect("open workload cgroup for attach");
        let connect_link = {
            use aya::programs::CgroupSockAddr;
            let prog: &mut CgroupSockAddr = bpf
                .program_mut("cgroup_connect4_mtls")
                .expect("cgroup_connect4_mtls program present")
                .try_into()
                .expect("program is CgroupSockAddr");
            prog.load().expect("load cgroup_connect4_mtls");
            let link_id = prog
                .attach(&cgroup_file, aya::programs::CgroupAttachMode::Single)
                .expect("attach cgroup_connect4_mtls to workload cgroup");
            prog.take_link(link_id).expect("take cgroup link")
        };

        // Program MTLS_REDIRECT_DEST[fake_peer] = leg-F listener (host-order keys).
        program_redirect_dest(
            &mut bpf,
            OUTBOUND_FAKE_PEER,
            OUTBOUND_FAKE_PEER_PORT,
            host_veth_ip,
            leg_f_port,
        );

        // Spawn the cgroup-isolated workload in the netns: it connects to the FAKE
        // peer (rewritten), writes the request, then (after the proxy arms) reads
        // the reply. real_peer is unused by the workload (it aims at the fake peer);
        // the adapter dials the real peer on leg B.
        let _ = real_peer;
        let workload = spawn_outbound_workload(topo);

        Self {
            _bpf: bpf,
            _connect_link: connect_link,
            leg_f_listener,
            workload: Some(workload),
            handshake_delay,
        }
    }

    /// Accept the transparently-redirected workload connection — leg F (the owned
    /// plaintext leg the adapter takes ownership of).
    pub fn accept_leg_f(&mut self) -> OwnedFd {
        self.leg_f_listener.set_nonblocking(false).expect("blocking leg-F listener");
        // Bounded accept so a failed intercept does not hang the harness.
        let (leg_f, _peer) = accept_with_timeout(&self.leg_f_listener, Duration::from_secs(10))
            .expect("leg-F accept (cgroup_connect4_mtls intercept must deliver the connection)");
        leg_f.set_nodelay(true).ok();
        // Honour the timing-regime delay: a deliberate handshake-window delay
        // before the adapter is handed the leg, to defeat the increment-e
        // throwaway-harness RST under traced/delayed timing.
        if !self.handshake_delay.is_zero() {
            std::thread::sleep(self.handshake_delay);
        }
        OwnedFd::from(leg_f)
    }

    /// Join the workload child and report the bidirectional round-trip outcome.
    pub fn join(mut self) -> OutboundRoundTrip {
        self.workload.take().map_or(
            OutboundRoundTrip {
                forward_delivered_byte_exact: false,
                return_delivered_byte_exact: false,
                observed_rst: true,
            },
            |mut child| {
                let stderr = child.stderr.take();
                let status = wait_child_bounded(&mut child, Duration::from_secs(12));
                if let (true, Some(mut e)) = (status != Some(0), stderr) {
                    let mut s = String::new();
                    let _ = e.read_to_string(&mut s);
                    eprintln!("OUTBOUND-WORKLOAD exit={status:?} stderr={}", s.trim());
                }
                read_workload_outcome(status)
            },
        )
    }
}

/// Parse the workload subprocess's exit code into the round-trip outcome. The
/// python workload exits 0 on full success (forward sent + reply byte-exact, no
/// RST), 10 if the reply was not byte-exact, 20 on a connection reset.
fn read_workload_outcome(code: Option<i32>) -> OutboundRoundTrip {
    match code {
        Some(0) => OutboundRoundTrip {
            forward_delivered_byte_exact: true,
            return_delivered_byte_exact: true,
            observed_rst: false,
        },
        Some(10) => OutboundRoundTrip {
            forward_delivered_byte_exact: true,
            return_delivered_byte_exact: false,
            observed_rst: false,
        },
        Some(20) => OutboundRoundTrip {
            forward_delivered_byte_exact: false,
            return_delivered_byte_exact: false,
            observed_rst: true,
        },
        _ => OutboundRoundTrip {
            forward_delivered_byte_exact: false,
            return_delivered_byte_exact: false,
            observed_rst: true,
        },
    }
}

/// Spawn the cgroup-isolated outbound workload: a python3 process placed into the
/// workload cgroup (via `pre_exec`) and run inside the workload netns (via `ip
/// netns exec`). It connects to the FAKE peer (rewritten by cgroup_connect4_mtls),
/// writes OUTBOUND_REQUEST, then reads OUTBOUND_REPLY byte-exact.
fn spawn_outbound_workload(topo: &MtlsTopology) -> Child {
    // The workload writes the request in TWO phases to exercise BOTH the lossless
    // pre-arm capture AND the steady-state forward pump (GAP 1):
    //   phase 1 (immediately on connect) — the PRE-ARM portion the agent drains
    //            losslessly during the handshake window and flushes through leg B;
    //   phase 2 (after a delay) — the STEADY-STATE portion that arrives AFTER the
    //            agent has armed kTLS + spawned the forward encrypt pump, so it rides
    //            the agent-light read->write_all COPY (legF → legB) into leg B's kTLS
    //            TX (the kernel tls_sw_sendmsg encrypts each write). The peer
    //            reconstructs both, in order, byte-exact.
    let split = OUTBOUND_REQUEST.len() / 2;
    let script = format!(
        r#"
import socket, sys, time
part1 = {part1}
part2 = {part2}
reply = {reply}
try:
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(12)
    s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    s.connect(("{fake_ip}", {fake_port}))
    # phase 1: pre-arm plaintext (drained losslessly during the handshake window)
    s.sendall(part1)
    # phase 2: steady-state bytes — written well after the agent has armed kTLS +
    # spawned the forward encrypt pump (generous margin over the 400ms handshake-delay
    # regime + the proxy's ~400ms drain/handshake/arm/settle), so they ride the
    # agent-light read->write_all COPY (legF -> legB) into leg B's kTLS TX (NOT a
    # splice -- a splice into kTLS-TX loses records), not the pre-arm drain.
    time.sleep(2.0)
    s.sendall(part2)
    # read the peer's reply back over the spliced return leg (B->F).
    got = b""
    s.settimeout(5)
    while len(got) < len(reply):
        b = s.recv(4096)
        if not b:
            break
        got += b
    if got == reply:
        sys.exit(0)
    else:
        sys.stderr.write("reply mismatch got=%d want=%d\n" % (len(got), len(reply)))
        sys.exit(10)
except (ConnectionResetError, BrokenPipeError) as e:
    sys.stderr.write("RST: %s\n" % e)
    sys.exit(20)
except Exception as e:
    sys.stderr.write("workload err: %s\n" % e)
    sys.exit(30)
"#,
        part1 = PyBytes(&OUTBOUND_REQUEST[..split]),
        part2 = PyBytes(&OUTBOUND_REQUEST[split..]),
        reply = PyBytes(OUTBOUND_REPLY),
        fake_ip = OUTBOUND_FAKE_PEER,
        fake_port = OUTBOUND_FAKE_PEER_PORT,
    );
    let procs = format!("{}/cgroup.procs", topo.cgroup_path());
    let mut cmd = Command::new("ip");
    cmd.args(["netns", "exec", topo.netns(), "python3", "-c", &script])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // Place THIS process (the `ip netns exec` wrapper, which execs python) into the
    // workload cgroup before exec, so the workload's connect() fires
    // cgroup_connect4_mtls. cgroup membership is inherited across exec.
    // SAFETY: pre_exec runs in the forked child before exec; writing our own pid to
    // cgroup.procs is async-signal-safe enough for this fixture (single write).
    unsafe {
        cmd.pre_exec(move || {
            let pid = std::process::id();
            std::fs::write(&procs, pid.to_string())
                .map_err(|e| std::io::Error::other(format!("join cgroup: {e}")))?;
            Ok(())
        });
    }
    cmd.spawn().expect("spawn outbound workload")
}

// =====================================================================
// INBOUND server S — the identity-unaware plaintext server WORKLOAD; holds nothing.
//
// GAP 3: S is a CGROUP-ISOLATED NETNS SUBPROCESS (matching the outbound workload's
// `spawn_outbound_workload` shape — `ip netns exec` + `cgroup.procs` pre_exec), NOT
// a host-side sibling thread. It binds the netns veth IP (`server_netns_ip:VIRT_PORT`)
// so the agent's leg-S dial reaches it over the veth (after the harness DNATs the
// verbatim orig-dst the production adapter dials). It reads the decrypted request
// the agent splices to it and replies (the S→C response leg).
// =====================================================================
pub struct InboundServer {
    addr: SocketAddrV4,
    server: Option<Child>,
}

impl InboundServer {
    /// Spawn S as a cgroup-isolated netns subprocess binding the netns veth IP. The
    /// agent dials the orig-dst (`VIRT_IP:VIRT_PORT`) verbatim; the topology's DNAT
    /// (`install_tproxy`) routes that marked dial to THIS address over the veth.
    pub fn spawn(topo: &MtlsTopology) -> Self {
        let netns_ip: Ipv4Addr = MtlsTopology::SERVER_NETNS_IP.parse().expect("server netns ip");
        let addr = SocketAddrV4::new(netns_ip, MtlsTopology::VIRT_PORT);
        let server = spawn_inbound_server_workload(topo, netns_ip, MtlsTopology::VIRT_PORT);
        // Block until S is actually LISTENING before returning. S is a python3
        // subprocess (interpreter startup + bind takes time); the agent's leg-S dial
        // (inside `enforce`, which runs shortly after) would otherwise race S's bind
        // and get ConnectionRefused. Poll the netns for the listening socket.
        wait_netns_listening(topo.netns(), MtlsTopology::VIRT_PORT, Duration::from_secs(10));
        Self { addr, server: Some(server) }
    }

    /// S's real netns listener address (leg S — the agent dials the orig-dst, which
    /// the harness DNATs here).
    pub fn addr(&self) -> SocketAddrV4 {
        self.addr
    }

    pub fn join(mut self) -> InboundServerResult {
        self.server.take().map_or(
            InboundServerResult { received_request_byte_exact: false, observed_rst: true },
            |mut child| {
                let stderr = child.stderr.take();
                let status = wait_child_bounded(&mut child, Duration::from_secs(14));
                if let (true, Some(mut e)) = (status != Some(0), stderr) {
                    let mut s = String::new();
                    let _ = e.read_to_string(&mut s);
                    eprintln!("INBOUND-SERVER exit={status:?} stderr={}", s.trim());
                }
                read_inbound_server_outcome(status)
            },
        )
    }

    pub fn shutdown(&self) {}
}

/// Parse S's subprocess exit code into its result. The python server exits 0 when it
/// received the byte-exact request (and replied), 10 on a request mismatch, 20 on a
/// connection reset, 30 on any other error.
fn read_inbound_server_outcome(code: Option<i32>) -> InboundServerResult {
    match code {
        Some(0) => InboundServerResult { received_request_byte_exact: true, observed_rst: false },
        Some(20) => InboundServerResult { received_request_byte_exact: false, observed_rst: true },
        _ => InboundServerResult { received_request_byte_exact: false, observed_rst: false },
    }
}

/// Spawn the cgroup-isolated inbound server workload S: a python3 process placed in
/// the workload cgroup (`pre_exec` → `cgroup.procs`) and run inside the workload netns
/// (`ip netns exec`). It binds `bind_ip:bind_port` on the netns veth, accepts the
/// agent's leg-S dial, reads INBOUND_REQUEST byte-exact, and replies INBOUND_RESPONSE.
fn spawn_inbound_server_workload(topo: &MtlsTopology, bind_ip: Ipv4Addr, bind_port: u16) -> Child {
    let script = format!(
        r#"
import socket, sys, time
request = {request}
response = {response}
try:
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("{bind_ip}", {bind_port}))
    srv.listen(16)
    srv.settimeout(14)
    conn, _ = srv.accept()
    conn.settimeout(8)
    conn.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    got = b""
    try:
        while len(got) < len(request):
            b = conn.recv(65536)
            if not b:
                break
            got += b
    except ConnectionResetError:
        sys.stderr.write("RST during recv\n")
        sys.exit(20)
    if got != request:
        sys.stderr.write("request mismatch got=%d want=%d\n" % (len(got), len(request)))
        sys.exit(10)
    # reply over the S->C response leg (GAP 2 inbound half); the agent splices it
    # back over leg C's kTLS. F4: split into TWO writes with an inter-write delay
    # larger than the agent's encrypt-pump read window (40 ms), so the agent's
    # write_all into leg C's kTLS-TX frames >=2 distinct TLS records on the S->C
    # direction. The client reconstructs the concatenation byte-exact.
    mid = len(response) // 2
    conn.sendall(response[:mid])
    time.sleep(0.15)
    conn.sendall(response[mid:])
    time.sleep(0.6)
    sys.exit(0)
except (ConnectionResetError, BrokenPipeError) as e:
    sys.stderr.write("RST: %s\n" % e)
    sys.exit(20)
except Exception as e:
    sys.stderr.write("server err: %s\n" % e)
    sys.exit(30)
"#,
        request = PyBytes(INBOUND_REQUEST),
        response = PyBytes(INBOUND_RESPONSE),
        bind_ip = bind_ip,
        bind_port = bind_port,
    );
    let procs = format!("{}/cgroup.procs", topo.cgroup_path());
    let mut cmd = Command::new("ip");
    cmd.args(["netns", "exec", topo.netns(), "python3", "-c", &script])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // Place the `ip netns exec` wrapper (which execs python) into the workload cgroup
    // before exec, so S runs cgroup-isolated (GAP 3). cgroup membership is inherited
    // across exec.
    // SAFETY: pre_exec runs in the forked child before exec; one write to cgroup.procs
    // is async-signal-safe enough for this fixture.
    unsafe {
        cmd.pre_exec(move || {
            let pid = std::process::id();
            std::fs::write(&procs, pid.to_string())
                .map_err(|e| std::io::Error::other(format!("join cgroup: {e}")))?;
            Ok(())
        });
    }
    cmd.spawn().expect("spawn inbound server workload S")
}

// =====================================================================
// INBOUND worker + client — installs nft-TPROXY (VIRT:port -> leg-C listener),
// spawns the client (presents a client SVID toward the virtual addr), accepts
// leg C, recovers orig-dst via getsockname.
//
// Productionises increment-i `role_agent` intercept half + `role_client`.
// =====================================================================
pub struct InboundWorker {
    leg_c_listener: TcpListener,
    client: Option<std::thread::JoinHandle<InboundClientResult>>,
    /// REAL AF_PACKET capture on `lo` filtered to the VIRT port — the
    /// confidentiality oracle for the client-facing leg C is derived from captured
    /// bytes (F2), NOT from the client's handshake-success. Stopped+scanned by
    /// `client_wire_observations`.
    wire: parking_lot::Mutex<WireCaptureState>,
    handshake_delay: Duration,
}

/// IP_TRANSPARENT is option 19 (IPPROTO_IP) — libc 0.2 does not expose it by name.
const IP_TRANSPARENT: libc::c_int = 19;

impl InboundWorker {
    /// `agent_port` is the agent's `IP_TRANSPARENT` leg-C listener port — the SAME
    /// port the test installed the `nft`-TPROXY rule's `tproxy to 127.0.0.1:agent_port`
    /// target on (via `MtlsTopology::install_tproxy`, the single source of truth; the
    /// duplicate standalone installer is removed — F4). The worker no longer installs
    /// TPROXY; it binds leg C on the agreed port and starts the wire capture.
    pub fn run(
        topo: &MtlsTopology,
        _server_addr: SocketAddrV4,
        pki: &TestPki,
        agent_port: u16,
        handshake_delay: Duration,
    ) -> Self {
        let _ = topo;
        // The agent's IP_TRANSPARENT leg-C listener (where TPROXY lands the
        // intercepted client connection). Bind on the agreed agent_port with
        // SO_REUSEADDR so both timing regimes can re-bind it sequentially.
        let leg_c_listener = make_transparent_listener(agent_port);

        // Start the REAL AF_PACKET capture on `lo` filtered to the VIRT port — the
        // client-facing leg-C confidentiality oracle (F2). Start it BEFORE the client
        // connects so the first record is on the captured wire.
        let capture = WireCapture::start(LOOPBACK_IFACE, MtlsTopology::VIRT_PORT);

        // Spawn the inbound client: presents the CLIENT SVID, connects toward the
        // VIRTUAL addr (TPROXY-intercepted to leg C). Runs on a thread (the client is
        // host-side — the remote-endpoint analogue of the outbound peer, which is
        // accepted as GAP-3-closing; there is no outbound intercept on the inbound
        // client, so it does NOT need the workload cgroup).
        let client_cert = pki.client_leaf.cert_der.clone();
        // Present `[client_leaf, intermediate]`: the peer/server's verifier (the
        // agent's leg-C server-mTLS `WebPkiClientVerifier`, root-anchor-only) needs
        // the intermediate to build the path to the verified client leaf (F1). The
        // SPIFFE-SAN assertion still reads chain position 0 (the leaf).
        let client_intermediate = pki.intermediate_cert_der();
        let client_key = pki.client_leaf.key_der.clone_key();
        let ca_pem = pki.ca_cert_pem().to_string();
        let virt_addr = SocketAddrV4::new(
            MtlsTopology::VIRT_IP.parse().expect("VIRT_IP"),
            MtlsTopology::VIRT_PORT,
        );
        let send_delay = handshake_delay.max(Duration::from_millis(400));
        let client = std::thread::spawn(move || {
            inbound_client_run(
                virt_addr,
                client_cert,
                client_intermediate,
                client_key,
                &ca_pem,
                send_delay,
            )
        });

        Self {
            leg_c_listener,
            client: Some(client),
            wire: parking_lot::Mutex::new(WireCaptureState::Live(capture)),
            handshake_delay,
        }
    }

    /// Accept leg C (the client-facing kTLS leg the adapter takes ownership of) and
    /// recover the original destination via `getsockname` (selects the server SVID).
    pub fn accept_leg_c_and_orig_dst(&mut self) -> (OwnedFd, SocketAddrV4) {
        let (leg_c, peer) = accept_with_timeout(&self.leg_c_listener, Duration::from_secs(12))
            .expect("leg-C accept (nft-TPROXY intercept must deliver the connection)");
        let _ = peer;
        leg_c.set_nodelay(true).ok();
        let orig_dst = getsockname_orig(leg_c.as_raw_fd());
        if !self.handshake_delay.is_zero() {
            std::thread::sleep(self.handshake_delay);
        }
        (OwnedFd::from(leg_c), orig_dst)
    }

    /// Join the inbound client thread, completing the C↔S round-trip, and only THEN
    /// stop plus scan the client-facing leg-C wire capture (F2). Keeping the capture
    /// live until after the client thread joins makes the confidentiality scan cover
    /// the application request and response payload rather than just the
    /// encrypted-handshake flight, whose outer TLS content type is also application
    /// data, so a record count of one or more would otherwise be satisfiable by the
    /// handshake alone. This mirrors the already-correct outbound ordering, where
    /// `wire_observations` runs only after `workload.join()`. Returns the client's
    /// round-trip result paired with the leg-C wire observations.
    pub fn join_client(mut self) -> (InboundClientResult, WireObservations) {
        let fail = || InboundClientResult {
            received_response_byte_exact: false,
            observed_rst: true,
            presented_server_spiffe: None,
        };
        let client_result =
            self.client.take().map_or_else(fail, |h| h.join().unwrap_or_else(|_| fail()));
        // The round-trip is done (client thread joined) — NOW stop + scan the capture
        // so the confidentiality scan covers the application payload. The VIRT port IS
        // the wire_port: the request (INBOUND_REQUEST, C→S) flows TOWARD it; the
        // response (INBOUND_RESPONSE, S→C) flows FROM it.
        let scan = stop_scan_cached(&self.wire, INBOUND_REQUEST, INBOUND_RESPONSE);
        let wire = WireObservations {
            records_request_dir: scan.records_to_wire_port,
            records_response_dir: scan.records_from_wire_port,
            plaintext_marker_hits: scan.plaintext_marker_hits,
        };
        (client_result, wire)
    }
}

/// Alias kept for the inbound client role referenced by name in docs.
pub type InboundClient = InboundWorker;

/// The inbound client: a rustls TLS-1.3 client presenting the CLIENT SVID, aimed
/// at the VIRTUAL addr (TPROXY-intercepted to the agent's leg C). Verifies the
/// server cert chains to the CA. Sends INBOUND_REQUEST after a delay (so it lands
/// AFTER the agent arms kTLS-RX), reads INBOUND_RESPONSE byte-exact (GAP 2).
fn inbound_client_run(
    virt_addr: SocketAddrV4,
    cert: rustls::pki_types::CertificateDer<'static>,
    intermediate: rustls::pki_types::CertificateDer<'static>,
    key: rustls::pki_types::PrivateKeyDer<'static>,
    ca_pem: &str,
    send_delay: Duration,
) -> InboundClientResult {
    use rustls::pki_types::ServerName;
    use rustls::{ClientConfig, ClientConnection};

    let roots = ca_root_store(ca_pem);
    // Present `[client_leaf, intermediate]` so the agent's leg-C root-anchor-only
    // `WebPkiClientVerifier` builds `leaf → intermediate → root` (F1).
    let cfg = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(vec![cert, intermediate], key)
        .expect("inbound client config");
    let Ok(tcp) = TcpStream::connect(virt_addr) else {
        return InboundClientResult {
            received_response_byte_exact: false,
            observed_rst: true,
            presented_server_spiffe: None,
        };
    };
    tcp.set_nodelay(true).ok();
    let sni = ServerName::try_from(TestPki::SERVER_SNI.to_string()).expect("server SNI");
    let mut conn = ClientConnection::new(Arc::new(cfg), sni).expect("inbound ClientConnection");
    let mut tcp = tcp;
    tcp.set_read_timeout(Some(Duration::from_secs(6))).ok();

    // Drive the handshake.
    if !drive_client_handshake(&mut conn, &mut tcp) {
        return InboundClientResult {
            received_response_byte_exact: false,
            observed_rst: true,
            presented_server_spiffe: None,
        };
    }
    // The handshake completed AND verified the server cert chains to the bundle
    // root (the client's `ClientConfig` anchors on the CA root store, so a
    // server leaf that did not chain would have aborted the handshake above).
    // Extract the SERVER SPIFFE-id from the presented leaf's URI SAN — this is
    // the agent's HELD server SVID (read via `IdentityRead`), the inbound AC3
    // identity proof. Read BEFORE any further connection use.
    let presented_server_spiffe = peer_presented_leaf_spiffe(conn.peer_certificates());
    // The wire oracle is NO LONGER set here from handshake-success — the REAL
    // AF_PACKET capture on `lo` (owned by `InboundWorker`, filtered to the VIRT port)
    // counts the genuine `0x17` records on the client-facing leg and confirms the
    // cleartext request marker is absent. This client only drives the app round-trip.
    // Delay the first app write so the request lands AFTER the agent arms kTLS-RX.
    std::thread::sleep(send_delay);

    let mut observed_rst = false;
    {
        // F4: split the request into TWO writes with an inter-write delay larger than
        // the agent's decrypt-pump poll window (40 ms), so rustls emits >=2 distinct
        // TLS 1.3 application_data records on leg C (C→S direction). The agent splices
        // each record to leg S; the server S reconstructs the concatenation
        // byte-exact (its recv loop accumulates until the full marker length).
        let mid = INBOUND_REQUEST.len() / 2;
        {
            let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
            if tls.write_all(&INBOUND_REQUEST[..mid]).and_then(|()| tls.flush()).is_err() {
                observed_rst = true;
            }
        }
        if !observed_rst {
            std::thread::sleep(Duration::from_millis(150));
            let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
            if tls.write_all(&INBOUND_REQUEST[mid..]).and_then(|()| tls.flush()).is_err() {
                observed_rst = true;
            }
        }
    }

    // Read the server's response back over leg C's kTLS (GAP 2 inbound half).
    let mut got = Vec::new();
    if !observed_rst {
        let deadline = std::time::Instant::now() + Duration::from_secs(6);
        let mut buf = vec![0u8; 4096];
        while got.len() < INBOUND_RESPONSE.len() && std::time::Instant::now() < deadline {
            let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
            match tls.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(ref e) if e.kind() == std::io::ErrorKind::ConnectionReset => {
                    observed_rst = true;
                    break;
                }
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    break;
                }
                Err(_) => break,
            }
        }
    }
    let received_response_byte_exact = got == INBOUND_RESPONSE;
    // (the wire oracle is the AF_PACKET capture owned by `InboundWorker`, not set here.)
    std::thread::sleep(Duration::from_millis(300));
    InboundClientResult { received_response_byte_exact, observed_rst, presented_server_spiffe }
}

// ---- shared helpers ----

/// Hex/escape a byte string into a Python bytes literal body (for the workload
/// script). Emits `\xNN` for every byte so the literal is unambiguous.
struct PyBytes<'a>(&'a [u8]);
impl std::fmt::Display for PyBytes<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("b\"")?;
        for b in self.0 {
            write!(f, "\\x{b:02x}")?;
        }
        f.write_str("\"")
    }
}

/// Resolve the BPF object path the way the adapter does (the build-time embedded
/// path env), so the workload-side BPF load uses the SAME object.
fn build_bpf_object_path() -> std::path::PathBuf {
    // OVERDRIVE_BPF_OBJECT_PATH is set by build.rs / the mutation env override.
    if let Ok(p) = std::env::var("OVERDRIVE_BPF_OBJECT_PATH") {
        return std::path::PathBuf::from(p);
    }
    // Fall back to the canonical workspace-relative path.
    let manifest = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest)
        .join("../../target/bpf/overdrive_bpf.o")
        .canonicalize()
        .expect("BPF object path")
}

/// The test-owned bpffs pin dir for the workload-side shared-object load. Distinct
/// from the adapter's `/sys/fs/bpf/overdrive-mtls` so the two loads do not collide
/// on the SERVICE_MAP pin (each `Ebpf` instance reuses its OWN pinned outer map).
const WORKLOAD_PIN_DIR: &str = "/sys/fs/bpf/overdrive-mtls-workload";

/// Load the shared BPF object for the workload-side `cgroup_connect4_mtls` attach
/// via the `pinning = ByName` SERVICE_MAP workaround (aya 0.13.x cannot create the
/// phase-2 HoM from the ELF alone). Pre-pins SERVICE_MAP into the test-owned dir,
/// then loads with `map_pin_path`.
fn load_workload_bpf(obj: &std::path::Path) -> aya::Ebpf {
    use overdrive_dataplane::maps::ServiceKey;
    use overdrive_dataplane::maps::hash_of_maps::HashOfMapsHandle;

    let pin_dir = std::path::Path::new(WORKLOAD_PIN_DIR);
    std::fs::create_dir_all(pin_dir).expect("create workload bpffs pin dir");
    let pin_path = pin_dir.join("SERVICE_MAP");
    let _ = std::fs::remove_file(&pin_path); // clean any stale pin
    // 4096 outer / Maglev-default inner — the SSOT capacities the adapter uses.
    let inner = overdrive_core::dataplane::MaglevTableSize::DEFAULT.get();
    let service_map = HashOfMapsHandle::<ServiceKey, u32>::new_pinned_with_array_inner(
        "SERVICE_MAP",
        4096,
        inner,
        pin_dir,
    )
    .expect("pre-pin workload SERVICE_MAP");
    // Leak the handle for the test's lifetime so the pin stays valid while the ELF
    // is loaded + the workload runs.
    std::mem::forget(service_map);
    aya::EbpfLoader::new()
        .map_pin_path(pin_dir)
        // Tolerate the HASH_OF_MAPS SERVICE_MAP (aya 0.13.x has no typed variant);
        // the pinned outer map is reused via `map_pin_path`.
        .allow_unsupported_maps()
        .load_file(obj)
        .unwrap_or_else(|e| panic!("load workload BPF object {}: {e}", obj.display()))
}

/// `MtlsDestKey` / `MtlsAddrPort` userspace mirrors (8-byte host-order PODs,
/// matching `overdrive-bpf::maps::mtls_redirect_dest`).
#[repr(C)]
#[derive(Clone, Copy)]
struct MtlsDestKey {
    ip_host: u32,
    port_host: u16,
    _pad: u16,
}
// SAFETY: 8-byte `#[repr(C)]` POD with no padding-derived invariants beyond `_pad`.
unsafe impl aya::Pod for MtlsDestKey {}

#[repr(C)]
#[derive(Clone, Copy)]
struct MtlsAddrPort {
    ip_host: u32,
    port_host: u16,
    _pad: u16,
}
// SAFETY: 8-byte `#[repr(C)]` POD.
unsafe impl aya::Pod for MtlsAddrPort {}

/// Program `MTLS_REDIRECT_DEST[fake_peer] = agent_leg_f_listener` (host-order keys
/// per the endianness lockstep — `u32::from(Ipv4Addr)` is host-order).
fn program_redirect_dest(
    bpf: &mut aya::Ebpf,
    fake_ip: Ipv4Addr,
    fake_port: u16,
    agent_ip: Ipv4Addr,
    agent_port: u16,
) {
    use aya::maps::HashMap as AyaHashMap;
    let mut redir: AyaHashMap<_, MtlsDestKey, MtlsAddrPort> =
        AyaHashMap::try_from(bpf.map_mut("MTLS_REDIRECT_DEST").expect("MTLS_REDIRECT_DEST"))
            .expect("MTLS_REDIRECT_DEST handle");
    let key = MtlsDestKey { ip_host: u32::from(fake_ip), port_host: fake_port, _pad: 0 };
    let val = MtlsAddrPort { ip_host: u32::from(agent_ip), port_host: agent_port, _pad: 0 };
    redir.insert(key, val, 0).expect("program MTLS_REDIRECT_DEST");
}

/// Build a `RootCertStore` from a CA cert PEM.
fn ca_root_store(ca_cert_pem: &str) -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    let mut rd = std::io::BufReader::new(ca_cert_pem.as_bytes());
    for c in rustls_pemfile::certs(&mut rd) {
        roots.add(c.expect("ca cert")).expect("add ca cert");
    }
    roots
}

/// Drive a rustls client handshake to completion; returns false on failure.
fn drive_client_handshake(conn: &mut rustls::ClientConnection, tcp: &mut TcpStream) -> bool {
    use std::io::ErrorKind;
    loop {
        while conn.wants_write() {
            if conn.write_tls(tcp).is_err() {
                return false;
            }
        }
        if !conn.is_handshaking() {
            return true;
        }
        match conn.read_tls(tcp) {
            Ok(0) => return false,
            Ok(_) => {
                if conn.process_new_packets().is_err() {
                    return false;
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(_) => return false,
        }
    }
}

/// Make an IP_TRANSPARENT loopback listener (accepts TPROXY-redirected conns).
fn make_transparent_listener(port: u16) -> TcpListener {
    // SAFETY: socket + IP_TRANSPARENT/SO_REUSEADDR + bind + listen with checked rc.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        assert!(fd >= 0, "socket: {}", std::io::Error::last_os_error());
        let one: libc::c_int = 1;
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            std::ptr::from_ref(&one).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        let rc = libc::setsockopt(
            fd,
            libc::IPPROTO_IP,
            IP_TRANSPARENT,
            std::ptr::from_ref(&one).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        assert!(
            rc == 0,
            "IP_TRANSPARENT: {} (need CAP_NET_ADMIN/root)",
            std::io::Error::last_os_error()
        );
        let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
        let sa = sockaddr_in_from(addr);
        let rc = libc::bind(
            fd,
            std::ptr::from_ref(&sa).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        );
        assert!(rc == 0, "bind: {}", std::io::Error::last_os_error());
        assert!(libc::listen(fd, 16) == 0, "listen: {}", std::io::Error::last_os_error());
        TcpListener::from_raw_fd(fd)
    }
}

fn sockaddr_in_from(addr: SocketAddrV4) -> libc::sockaddr_in {
    let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    sa.sin_family = libc::AF_INET as libc::sa_family_t;
    sa.sin_port = addr.port().to_be();
    sa.sin_addr.s_addr = u32::from_ne_bytes(addr.ip().octets());
    sa
}

/// `getsockname` on a TPROXY-intercepted socket returns the ORIGINAL destination.
fn getsockname_orig(fd: RawFd) -> SocketAddrV4 {
    let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockname(fd, std::ptr::from_mut(&mut sa).cast(), std::ptr::from_mut(&mut len))
    };
    if rc != 0 {
        return SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0);
    }
    let ip = Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr));
    let port = u16::from_be(sa.sin_port);
    SocketAddrV4::new(ip, port)
}

/// Accept one connection within `timeout`, or None. Bounded so a failed intercept
/// never hangs the harness.
fn accept_with_timeout(
    listener: &TcpListener,
    timeout: Duration,
) -> Option<(TcpStream, std::net::SocketAddr)> {
    let lfd = listener.as_raw_fd();
    let deadline = std::time::Instant::now() + timeout;
    listener.set_nonblocking(true).ok();
    let result = loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break None;
        }
        let mut pfd = libc::pollfd { fd: lfd, events: libc::POLLIN, revents: 0 };
        let ms = remaining.as_millis().min(200) as libc::c_int;
        let pr = unsafe { libc::poll(std::ptr::from_mut(&mut pfd), 1, ms) };
        if pr <= 0 {
            continue;
        }
        match listener.accept() {
            Ok((stream, peer)) => break Some((stream, peer)),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => break None,
        }
    };
    listener.set_nonblocking(false).ok();
    if let Some((ref stream, _)) = result {
        stream.set_nonblocking(false).ok();
    }
    result
}

/// Poll the netns for a TCP listener on `port` until it appears or `timeout`
/// elapses. Used to gate the agent's leg-S dial behind S's bind — S is a python3
/// subprocess whose interpreter-startup + `bind` race the agent's `connect`, and a
/// dial that loses the race gets `ConnectionRefused` (the netns S has no listener
/// yet). `ss -ltnH 'sport = :<port>'` inside the netns reports the listener; a
/// non-empty line means S is ready.
fn wait_netns_listening(netns: &str, port: u16, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    let filter = format!("sport = :{port}");
    while std::time::Instant::now() < deadline {
        let out = Command::new("ip")
            .args(["netns", "exec", netns, "ss", "-ltnH", &filter])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        if out.is_ok_and(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty()) {
            return; // S is listening
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // Fall through on timeout: the leg-S dial will surface the real failure
    // (ConnectionRefused) with a clearer test-assertion message than a silent stall.
    eprintln!("INBOUND-SERVER: S did not reach LISTENING on port {port} within {timeout:?}");
}

/// Wait for a child within `grace`, else kill by handle and reap. Returns the exit
/// code (None on kill/overrun).
fn wait_child_bounded(child: &mut Child, grace: Duration) -> Option<i32> {
    let deadline = std::time::Instant::now() + grace;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.code(),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
}
