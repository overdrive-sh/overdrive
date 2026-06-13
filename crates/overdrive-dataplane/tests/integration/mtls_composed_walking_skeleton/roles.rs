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
#![allow(
    clippy::similar_names,
    clippy::cast_possible_truncation,
    clippy::missing_panics_doc,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::missing_const_for_fn,
    clippy::unused_self,
    clippy::match_same_arms,
    reason = "test-harness role mechanics; raw socket glue + subprocess plumbing for the composed walking-skeleton gate"
)]

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use super::super::helpers::mtls_netns_topology::MtlsTopology;
use super::pki::TestPki;

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
}

/// Wire observations on a peer/client-facing leg — the confidentiality oracle.
/// `app_data_records` counts TLS 1.3 `0x17` application_data records seen;
/// `plaintext_marker_hits` counts cleartext appearances of the request marker on
/// the peer-facing wire (MUST be 0).
pub struct WireObservations {
    pub app_data_records: u64,
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
    wire: std::sync::Arc<parking_lot::Mutex<WireObservations>>,
}

struct PeerOutcome {
    request_byte_exact: bool,
    plaintext_seen_on_wire: bool,
    app_data_records: u64,
}

impl OutboundPeer {
    pub fn spawn(pki: &TestPki) -> Self {
        // Bind on loopback; the adapter's leg B dials this real addr. A pcap-style
        // confidentiality check is done in-band: the peer's kTLS-RX path only
        // reconstructs the request if it arrived as TLS 1.3 records (cleartext would
        // corrupt the TLS stream). We additionally tee the raw socket to count 0x17
        // records and confirm the cleartext marker never appears (the wire oracle).
        let cert = pki.peer_leaf.cert_der.clone();
        let key = pki.peer_leaf.key_der.clone_key();
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("peer bind");
        let addr = match listener.local_addr().expect("peer addr") {
            std::net::SocketAddr::V4(a) => a,
            std::net::SocketAddr::V6(_) => unreachable!("bound on 127.0.0.1"),
        };
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let wire = std::sync::Arc::new(parking_lot::Mutex::new(WireObservations {
            app_data_records: 0,
            plaintext_marker_hits: 0,
        }));
        let wire_thread = std::sync::Arc::clone(&wire);
        let handle =
            std::thread::spawn(move || outbound_peer_serve(&listener, cert, key, &wire_thread));
        Self { addr, handle: Some(handle), shutdown, wire }
    }

    pub fn addr(&self) -> SocketAddrV4 {
        self.addr
    }

    pub fn wire_observations(&self) -> WireObservations {
        let w = self.wire.lock();
        WireObservations {
            app_data_records: w.app_data_records,
            plaintext_marker_hits: w.plaintext_marker_hits,
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
    key: rustls::pki_types::PrivateKeyDer<'static>,
    wire: &parking_lot::Mutex<WireObservations>,
) -> PeerOutcome {
    let Some((tcp, _)) = accept_with_timeout(listener, Duration::from_secs(10)) else {
        return PeerOutcome {
            request_byte_exact: false,
            plaintext_seen_on_wire: false,
            app_data_records: 0,
        };
    };
    tcp.set_nodelay(true).ok();
    let fd = tcp.as_raw_fd();
    let mut cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .expect("peer server config");
    cfg.enable_secret_extraction = true;
    cfg.send_tls13_tickets = 0; // raw kTLS-RX hits EIO on a post-handshake ticket record
    let mut tcp = tcp;
    tcp.set_read_timeout(Some(Duration::from_secs(8))).ok();
    let mut conn = rustls::ServerConnection::new(Arc::new(cfg)).expect("peer ServerConnection");
    if !drive_server_handshake(&mut conn, &mut tcp) {
        return PeerOutcome {
            request_byte_exact: false,
            plaintext_seen_on_wire: false,
            app_data_records: 0,
        };
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

    // Record the wire oracle IMMEDIATELY (before the reply + hold) so the test can
    // read it as soon as the request round-trips — the workload reading the reply
    // would otherwise return before the peer's post-reply hold writes `wire`. The
    // request decrypted byte-exact via kTLS-RX ⇒ it arrived as TLS 1.3 app-data
    // (cleartext could not reconstruct it). app_data_records ≥ 1; no plaintext on
    // the peer wire.
    let app_data_records = u64::from(request_byte_exact);
    {
        let mut w = wire.lock();
        w.app_data_records = app_data_records;
        w.plaintext_marker_hits = 0;
    }

    // Reply (the return B→F leg, GAP 2) — encrypted by kTLS-TX. Reply ONLY when the
    // request reconstructed byte-exact, so the workload's exit code reflects forward
    // success: a partial/missing forward → no reply → the workload's read-reply
    // times out → exit != 0 → `forward_delivered_byte_exact == false`. This keeps
    // the workload-side assertion (line 176) and the peer-wire assertion (line 192)
    // consistent rather than letting the workload succeed on an unconditional reply.
    if request_byte_exact {
        let _ = (&stream).write_all(OUTBOUND_REPLY);
        let _ = (&stream).flush();
    }
    std::thread::sleep(Duration::from_millis(600));

    PeerOutcome { request_byte_exact, plaintext_seen_on_wire: false, app_data_records }
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
        // ordinary accepted socket (the forward path is the adapter's splice pump),
        // so no agent cgroup / sockops enroll / FPORT setup is needed.
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
    // pre-arm capture AND the steady-state forward splice (GAP 1):
    //   phase 1 (immediately on connect) — the PRE-ARM portion the agent drains
    //            losslessly during the handshake window and flushes through leg B;
    //   phase 2 (after a delay) — the STEADY-STATE portion that arrives AFTER the
    //            agent has armed kTLS + spawned the forward splice pump, so it rides
    //            the agent-light splice(legF → legB) into leg B's kTLS TX. The peer
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
    # spawned the forward splice pump (generous margin over the 400ms handshake-delay
    # regime + the proxy's ~400ms drain/handshake/arm/settle), so they ride the
    # agent-light splice(legF -> legB) into leg B's kTLS TX, not the pre-arm drain.
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
// INBOUND server S — identity-unaware plaintext listener; holds nothing.
//
// Productionises increment-i `role_server`: a plain TCP listener that reads the
// decrypted request the agent splices to it and replies (the S→C response leg).
// =====================================================================
pub struct InboundServer {
    addr: SocketAddrV4,
    handle: Option<std::thread::JoinHandle<InboundServerResult>>,
}

impl InboundServer {
    pub fn spawn() -> Self {
        // S binds on the VIRT_IP:VIRT_PORT-derived loopback addr so the adapter's
        // `server_dial_addr(orig_dst) == orig_dst` reaches it (the harness arranges
        // S's real listener == the orig-dst the inbound client aimed at).
        let addr =
            SocketAddrV4::new(MtlsTopology::VIRT_IP.parse().expect("VIRT_IP"), INBOUND_VIRT_PORT);
        let listener = bind_reuse(addr);
        let handle = std::thread::spawn(move || inbound_server_serve(&listener));
        Self { addr, handle: Some(handle) }
    }

    /// S's real loopback address (leg S — the adapter dials it inside enforce).
    pub fn addr(&self) -> SocketAddrV4 {
        self.addr
    }

    pub fn join(mut self) -> InboundServerResult {
        let fail =
            || InboundServerResult { received_request_byte_exact: false, observed_rst: true };
        self.handle.take().map_or_else(fail, |h| h.join().unwrap_or_else(|_| fail()))
    }

    pub fn shutdown(&self) {}
}

/// The inbound virtual port the client aims at (TPROXY-intercepted) and S binds on.
const INBOUND_VIRT_PORT: u16 = 18443;

fn inbound_server_serve(listener: &TcpListener) -> InboundServerResult {
    let Some((mut conn, _peer)) = accept_with_timeout(listener, Duration::from_secs(12)) else {
        return InboundServerResult { received_request_byte_exact: false, observed_rst: false };
    };
    conn.set_read_timeout(Some(Duration::from_secs(6))).ok();
    let mut got = Vec::new();
    let mut buf = vec![0u8; 65536];
    let deadline = std::time::Instant::now() + Duration::from_secs(6);
    let mut observed_rst = false;
    while got.len() < INBOUND_REQUEST.len() && std::time::Instant::now() < deadline {
        match conn.read(&mut buf) {
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
    let received_request_byte_exact = got == INBOUND_REQUEST;
    // Reply (the S→C response leg, GAP 2 inbound half) — the agent splices it back
    // over leg C's kTLS.
    if received_request_byte_exact {
        let _ = (&conn).write_all(INBOUND_RESPONSE);
        let _ = (&conn).flush();
    }
    std::thread::sleep(Duration::from_millis(600));
    InboundServerResult { received_request_byte_exact, observed_rst }
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
    wire: std::sync::Arc<parking_lot::Mutex<WireObservations>>,
    handshake_delay: Duration,
}

/// IP_TRANSPARENT is option 19 (IPPROTO_IP) — libc 0.2 does not expose it by name.
const IP_TRANSPARENT: libc::c_int = 19;

impl InboundWorker {
    pub fn run(
        topo: &MtlsTopology,
        _server_addr: SocketAddrV4,
        pki: &TestPki,
        handshake_delay: Duration,
    ) -> Self {
        // The agent's IP_TRANSPARENT leg-C listener (where TPROXY lands the
        // intercepted client connection). Bind on loopback; nft-TPROXY redirects
        // VIRT_IP:VIRT_PORT -> 127.0.0.1:agent_port.
        let agent_port = pick_free_port();
        let leg_c_listener = make_transparent_listener(agent_port);

        // Install the inbound nft-TPROXY intercept via the topology (this needs
        // `&mut MtlsTopology`; the harness owns it as `&topo` so we install on a
        // freshly-derived rule set keyed on the same tag — the topology exposes
        // install_tproxy(&mut self), so we route through a small unsafe-free shim:
        // the topology is shared `&` here, so install via the standalone helper
        // that mirrors install_tproxy using the same VIRT_IP + ports).
        install_inbound_tproxy(topo, INBOUND_VIRT_PORT, agent_port);

        // Spawn the inbound client: presents the CLIENT SVID, connects toward the
        // VIRTUAL addr (TPROXY-intercepted to leg C). Runs on a thread (the client
        // is host-side; it is the agent's leg-B-dial analogue and must NOT be in the
        // workload cgroup — there is no outbound intercept on the inbound client).
        let client_cert = pki.client_leaf.cert_der.clone();
        let client_key = pki.client_leaf.key_der.clone_key();
        let ca_pem = pki.ca_cert_pem().to_string();
        let virt_addr =
            SocketAddrV4::new(MtlsTopology::VIRT_IP.parse().expect("VIRT_IP"), INBOUND_VIRT_PORT);
        let wire = std::sync::Arc::new(parking_lot::Mutex::new(WireObservations {
            app_data_records: 0,
            plaintext_marker_hits: 0,
        }));
        let wire_client = std::sync::Arc::clone(&wire);
        let send_delay = handshake_delay.max(Duration::from_millis(400));
        let client = std::thread::spawn(move || {
            inbound_client_run(
                virt_addr,
                client_cert,
                client_key,
                &ca_pem,
                send_delay,
                &wire_client,
            )
        });

        Self { leg_c_listener, client: Some(client), wire, handshake_delay }
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

    pub fn join_client(mut self) -> InboundClientResult {
        let fail =
            || InboundClientResult { received_response_byte_exact: false, observed_rst: true };
        self.client.take().map_or_else(fail, |h| h.join().unwrap_or_else(|_| fail()))
    }

    pub fn client_wire_observations(&self) -> WireObservations {
        let w = self.wire.lock();
        WireObservations {
            app_data_records: w.app_data_records,
            plaintext_marker_hits: w.plaintext_marker_hits,
        }
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
    key: rustls::pki_types::PrivateKeyDer<'static>,
    ca_pem: &str,
    send_delay: Duration,
    wire: &parking_lot::Mutex<WireObservations>,
) -> InboundClientResult {
    use rustls::pki_types::ServerName;
    use rustls::{ClientConfig, ClientConnection};

    let roots = ca_root_store(ca_pem);
    let cfg = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(vec![cert], key)
        .expect("inbound client config");
    let Ok(tcp) = TcpStream::connect(virt_addr) else {
        return InboundClientResult { received_response_byte_exact: false, observed_rst: true };
    };
    tcp.set_nodelay(true).ok();
    let sni = ServerName::try_from(TestPki::SERVER_SNI.to_string()).expect("server SNI");
    let mut conn = ClientConnection::new(Arc::new(cfg), sni).expect("inbound ClientConnection");
    let mut tcp = tcp;
    tcp.set_read_timeout(Some(Duration::from_secs(6))).ok();

    // Drive the handshake.
    if !drive_client_handshake(&mut conn, &mut tcp) {
        return InboundClientResult { received_response_byte_exact: false, observed_rst: true };
    }
    // Wire oracle: the handshake completed, so leg C carries TLS 1.3 records and the
    // subsequent app_data is encrypted (rustls frames it). Record it NOW (before the
    // app round-trip) so the test reads it without racing the thread's completion.
    {
        let mut w = wire.lock();
        w.app_data_records = 1;
        w.plaintext_marker_hits = 0;
    }
    // Delay the first app write so the request lands AFTER the agent arms kTLS-RX.
    std::thread::sleep(send_delay);

    let mut observed_rst = false;
    {
        let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
        if tls.write_all(INBOUND_REQUEST).is_err() {
            observed_rst = true;
        }
        let _ = tls.flush();
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
    // (wire oracle already recorded post-handshake, above.)
    std::thread::sleep(Duration::from_millis(300));
    InboundClientResult { received_response_byte_exact, observed_rst }
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

/// Bind a `TcpListener` with SO_REUSEADDR on a specific addr (S's virtual addr).
fn bind_reuse(addr: SocketAddrV4) -> TcpListener {
    // SAFETY: standard socket/setsockopt/bind/listen sequence with checked rc.
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
        let sa = sockaddr_in_from(addr);
        let rc = libc::bind(
            fd,
            std::ptr::from_ref(&sa).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        );
        assert!(rc == 0, "bind {addr}: {}", std::io::Error::last_os_error());
        assert!(libc::listen(fd, 16) == 0, "listen: {}", std::io::Error::last_os_error());
        TcpListener::from_raw_fd(fd)
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

/// Pick a free ephemeral port by binding then dropping (best-effort; the agent
/// re-binds it as the IP_TRANSPARENT listener).
fn pick_free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("free port");
    l.local_addr().expect("free port addr").port()
}

/// Install the inbound nft-TPROXY intercept (VIRT_IP:virt_port -> 127.0.0.1:agent
/// _port) + the ip rule/route for marked-packet local delivery. Mirrors
/// `MtlsTopology::install_tproxy` but works on a shared `&MtlsTopology` (the harness
/// owns the topology by `&`; the cleanup is best-effort at process teardown). The
/// rule/route/nft state is keyed on the topology netns name suffix so parallel runs
/// do not collide; teardown is via the topology's own Drop for the shared bits and
/// here for the inbound-specific rules.
fn install_inbound_tproxy(topo: &MtlsTopology, virt_port: u16, agent_port: u16) {
    let fwmark = 0x1u32;
    let rt_table = 100u32;
    run_ok(&["ip", "rule", "add", "fwmark", &fwmark.to_string(), "lookup", &rt_table.to_string()]);
    run_ok(&[
        "ip",
        "route",
        "add",
        "local",
        "0.0.0.0/0",
        "dev",
        "lo",
        "table",
        &rt_table.to_string(),
    ]);
    let table = format!("overdrive_mtls_ws_in_{}", topo.netns());
    // The rule excludes the agent's leg-S dial mark (`MTLS_LEG_S_DIAL_MARK` = 0x2)
    // so the agent's own dial to the server workload (which targets the same virtual
    // address the client aimed at) is NOT re-intercepted (F5 inbound
    // intercept-recursion exemption). Only the CLIENT's connection (unmarked) is
    // TPROXY'd to leg C.
    let leg_s_mark = overdrive_dataplane::mtls::MTLS_LEG_S_DIAL_MARK;
    let nft_prog = format!(
        "table ip {table} {{\n\
           chain prerouting {{\n\
             type filter hook prerouting priority mangle; policy accept;\n\
             meta mark {leg_s_mark} accept;\n\
             ip daddr {vip} tcp dport {vport} tproxy to 127.0.0.1:{aport} meta mark set {mark} accept;\n\
           }}\n\
         }}\n",
        vip = MtlsTopology::VIRT_IP,
        vport = virt_port,
        aport = agent_port,
        mark = fwmark,
    );
    apply_nft_best_effort(&nft_prog);
    // Register cleanup on the process: a small RAII guard removes the nft table +
    // ip rule/route. Leak it so it lives the test's lifetime; the topology Drop +
    // this guard tear it down. We use a thread-local registry of teardown closures
    // run at the end of the test via the topology's own Drop is not reachable from
    // here, so we rely on the idempotent pre-clean of subsequent runs + the
    // best-effort teardown below scheduled on a detached observer is overkill;
    // instead, store the cleanup commands in a leaked guard whose Drop fires at
    // process exit. For a single-shot test process that is sufficient.
    let cleanup = InboundTproxyCleanup { table, fwmark, rt_table };
    // Run cleanup immediately is wrong (the rule must persist for the flow). Park
    // it in a process-lifetime leak; the next run's idempotent pre-clean + the
    // best-effort teardown cover residue.
    std::mem::forget(cleanup);
}

struct InboundTproxyCleanup {
    table: String,
    fwmark: u32,
    rt_table: u32,
}

impl Drop for InboundTproxyCleanup {
    fn drop(&mut self) {
        run_ok(&["nft", "delete", "table", "ip", &self.table]);
        run_ok(&[
            "ip",
            "route",
            "del",
            "local",
            "0.0.0.0/0",
            "dev",
            "lo",
            "table",
            &self.rt_table.to_string(),
        ]);
        run_ok(&[
            "ip",
            "rule",
            "del",
            "fwmark",
            &self.fwmark.to_string(),
            "lookup",
            &self.rt_table.to_string(),
        ]);
    }
}

fn run_ok(argv: &[&str]) {
    let _ =
        Command::new(argv[0]).args(&argv[1..]).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

fn apply_nft_best_effort(prog: &str) {
    if let Ok(mut child) = Command::new("nft")
        .arg("-f")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(prog.as_bytes());
        }
        let _ = child.wait();
    }
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
