//! INBOUND enforce — the per-direction wire/syscall observables of the asymmetric
//! agent-light steady state (transparent-mtls-host-socket step 03-01, ADR-0069
//! F3/F5; GH #26).
//!
//! Step 01-01 (the composed walking skeleton) already drove
//! `HostMtlsEnforcement::enforce(Inbound)` to steady-state-established on the real
//! netns/veth topology and proved the BIDIRECTIONAL wire confidentiality (0x17
//! records both ways, cleartext = 0, byte-exact plaintext at the server S). What
//! that gate did NOT isolate is the INBOUND **per-direction MECHANISM** the ADR-0069
//! contract pins: the request-carrying C→S DELIVER path (the liveness-observed
//! INBOUND PRIMARY) is an AGENT-LIGHT ZERO-COPY `splice(legC → legS)` out of leg C's
//! kTLS-RX (the kernel `tls_sw_splice_read` decrypts each record on splice-out; the
//! agent issues only `splice`/`ppoll`, NO per-byte plaintext `read`/`write` of the
//! request), while the S→C RESPONSE path is an AGENT-LIGHT `read(legS) →
//! write_all(legC)` COPY into leg C's kTLS-TX (auxiliary, not liveness-observed).
//! The agent-light asymmetry is the inverse of outbound: the request-carrying
//! INBOUND primary is the zero-copy SPLICE; the request-carrying OUTBOUND primary is
//! the COPY.
//!
//! THIS test isolates the INBOUND direction and asserts the five 03-01 ACs from REAL
//! kernel observables — `strace` on the agent's own pump threads, `ss -tie` on the
//! client-facing leg C, the AF_PACKET wire oracle on leg C, and the byte-exact
//! plaintext at the identity-unaware server workload S — through the SAME
//! `MtlsEnforcement` driving port (`enforce` / `liveness` / `teardown`). The
//! orig-dst → server-SVID selection productionised here arrives as
//! `InterceptedConnection { routed: Inbound { orig_dst }, alloc: server_alloc }`
//! (the orig-dst → allocation resolution is the WORKER's job, SD-1; by the time
//! `enforce` runs `conn.alloc` IS the selected server allocation, and "orig-dst →
//! identity" inside the adapter is `svid_for(&conn.alloc)` via the `IdentityRead`
//! port). The intercept setup (nft-TPROXY) + the leg-C listener + the `accept()` +
//! the `getsockname` orig-dst recovery are the WORKER role the test harness stands
//! in for (step 05-01); only the accepted leg crosses into the adapter.
//!
//! The five ACs (each anchored to `spike/findings-inbound-intercept.md`):
//! - **AC1 (orig-dst → identity)** — the recovered original destination selects the
//!   server workload's allocation and its held SVID via the identity port (findings
//!   §1 + 'Design implications' #3): the connection enforced with `conn.alloc =
//!   server_alloc` and `routed.direction() == Inbound` reaches Established, and the
//!   client's leg-C handshake extracts the SERVER SVID's SPIFFE from the presented
//!   leaf (proving the held server SVID was read via `IdentityRead::svid_for(&alloc)`
//!   and presented).
//! - **AC2 (server-mTLS → kTLS-RX armed)** — `ss -tie` on the leg-C socket shows
//!   `tcp-ulp-tls rxconf:sw` (kTLS-RX armed from the SERVER handshake's extracted
//!   secrets; auth-session == data-session) — read from the REAL kernel socket state,
//!   not the adapter's bookkeeping (findings §2/§3).
//! - **AC3 (byte-exact plaintext to the server; client leg 0x17-only)** — the server
//!   workload S reads the byte-exact request as PLAINTEXT (findings §3 PLAINTEXT_EXACT),
//!   the client-facing leg C carries 0x17 app_data ONLY (cleartext-hits on the client
//!   leg = 0; findings §3 client-leg records=2, cleartext-hits=0), and decrypted
//!   plaintext appears ONLY on the agent→server leg S.
//! - **AC4 (agent-light splice-only deliver)** — `strace` on the agent shows the
//!   DELIVER (C→S) path moves the inbound request via `splice`/`ppoll` ONLY — the
//!   request plaintext NEVER appears in a per-byte `read`/`write` buffer on the
//!   deliver path; the client-facing leg carries no psock on RX (findings §5
//!   splice_in=1 splice_out=1).
//! - **AC5 (adapter owns leg S end-to-end)** — the worker is never handed a TLS or
//!   server-side fd: `enforce` opens leg S internally (SD-1), the deliver pump is
//!   PORT-OWNED (SD-2), the enforced connection reports `liveness == Running` and
//!   tears down to `Gone`; the loopback spike topology is re-proven in the real
//!   netns/veth shape (findings 'What was NOT tested').
//!
//! **Litmus (falsifiability / port-to-port)**: if the call-site that wires the
//! deliver splice pump + dials leg S in `inbound::establish` were deleted, the
//! request would not reach S byte-exact, `liveness` would not be `Running`, the
//! `ss -tie` kTLS-RX read would fail, and the strace `splice` evidence would vanish —
//! every assertion below goes RED. The observables are derived from REAL captured
//! syscalls + captured wire bytes + the real `ss` ULP read + the real server
//! subprocess's byte-exact read, never from the adapter's own bookkeeping.
//!
//! Tier 3 ONLY (sockops/TPROXY/cgroup_sock_addr/kTLS/splice have no
//! `BPF_PROG_TEST_RUN`): `cargo xtask lima run -- cargo nextest run -p
//! overdrive-dataplane --features integration-tests -E 'test(mtls_inbound_enforce)'`,
//! ACTUALLY EXECUTING on the real 6.18+ kernel (a `--no-run` gate is green even when
//! every fixture refuses at boot).

#![cfg(target_os = "linux")]
// `unwrap`/`expect` are the standard test idiom — a panic with a message is exactly
// the right failure for a precondition.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
// The SKIP-loud path + strace diagnostics print via `eprintln!` (nextest captures
// it); the role helpers take `&mut self` because they own spawned-child state.
#![allow(clippy::print_stderr, clippy::needless_pass_by_ref_mut)]
// The single composed Tier-3 acceptance fn drives ALL five ACs end-to-end (one real
// round-trip under one strace attach) — splitting it would re-stand-up the
// netns/cgroup/server topology per AC for no behavioural gain. The leg names
// (leg C/S) + ADR-0069 / D-MTLS / contract tokens in the doc comments are the
// contract vocabulary, not prose to backtick.
#![allow(clippy::too_many_lines, clippy::doc_markdown)]

use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::AllocationId;
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::mtls_enforcement::{
    Direction, InterceptedConnection, MtlsEnforcement, MtlsLimits, PumpLiveness, Routed,
};
use overdrive_dataplane::mtls::HostMtlsEnforcement;

use super::helpers::mtls_netns_topology::{MtlsTopology, TopologyError};
use super::helpers::mtls_pki::TestPki;
use super::helpers::mtls_roles::{self, InboundServer};

/// The agent's held-identity store — the ONLY holder of SVID material. The workloads
/// (client AND server) hold nothing; the agent reads through THIS `IdentityRead` port
/// and NEVER mints (#26 is a reader). `None` is explicit absence.
struct HeldIdentities {
    svids: std::collections::BTreeMap<AllocationId, SvidMaterial>,
    bundle: TrustBundle,
}

impl IdentityRead for HeldIdentities {
    fn svid_for(&self, alloc: &AllocationId) -> Option<SvidMaterial> {
        self.svids.get(alloc).cloned()
    }

    fn current_bundle(&self) -> Option<TrustBundle> {
        Some(self.bundle.clone())
    }
}

/// Build the held-identity store from the test PKI: the SERVER SVID (inbound leg-C
/// SERVER handshake) plus the shared trust bundle. The server leaf material lives
/// HERE, with the agent — never with the identity-unaware server workload S.
fn held_identities(pki: &TestPki) -> HeldIdentities {
    let mut svids = std::collections::BTreeMap::new();
    svids.insert(pki.server_alloc.clone(), pki.server_svid_material());
    HeldIdentities { svids, bundle: pki.trust_bundle() }
}

/// Pick a free ephemeral port for the agent's `IP_TRANSPARENT` leg-C listener — the
/// `tproxy to 127.0.0.1:<port>` target installed in the topology.
fn pick_free_inbound_agent_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("free agent port");
    l.local_addr().expect("agent port addr").port()
}

/// The INBOUND-isolated 03-01 acceptance gate. Drives ONE inbound flow through
/// `HostMtlsEnforcement::enforce(Inbound)` on the real netns/veth + cgroup topology
/// while a `strace` attaches to the agent's pump threads, then asserts the
/// per-direction mechanism (deliver C→S zero-copy SPLICE), the kTLS-RX ULP on the
/// client-facing leg C, the byte-exact plaintext at the server S, the cleartext-free
/// client wire, the adapter-owned leg S, and `liveness == Running → Gone`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inbound_enforce_origdst_server_mtls_ktls_rx_splice_to_server() {
    let tag = format!("ib{}", std::process::id());
    // The canonical gate runs `cargo xtask lima run -- …` as root on the real 6.18+
    // kernel, where the topology is ALWAYS supported (root + CAP_NET_ADMIN + cgroup
    // v2 + nft_tproxy). A `TopologyError::Unsupported` here is NOT a legitimate skip —
    // it is the same "green without executing" hole a `--no-run` gate has. Fail loud
    // so a degraded environment cannot pass by skipping.
    let topo = match MtlsTopology::create(&tag) {
        Ok(t) => t,
        Err(e @ TopologyError::Unsupported(_)) => panic!(
            "inbound-enforce gate MUST run on the real kernel (root + CAP_NET_ADMIN + \
             cgroup v2 + nft_tproxy); a topology-unsupported here is a gate FAILURE, not a \
             skip — run via `cargo xtask lima run -- cargo nextest run -p overdrive-dataplane \
             --features integration-tests -E 'test(mtls_inbound_enforce)'`: {e}"
        ),
        Err(e) => panic!("topology setup failed (not a skip): {e}"),
    };
    let mut topo = topo;

    // strace must be present (the syscall oracle is load-bearing); its absence is a
    // gate FAILURE, not a skip — the canonical Lima VM ships it.
    assert!(
        Command::new("strace").arg("-V").output().is_ok_and(|o| o.status.success()),
        "strace is required for the inbound-enforce syscall oracle (deliver splice-only); it is \
         present in the canonical Lima VM — its absence is a gate failure, not a skip"
    );

    let pki = TestPki::mint();
    let identity: Arc<dyn IdentityRead> = Arc::new(held_identities(&pki));
    let adapter = HostMtlsEnforcement::new(identity, MtlsLimits::default());

    adapter
        .probe()
        .await
        .expect("Earned-Trust probe must pass on the real kernel before any enforce");

    // Install the inbound nft-TPROXY intercept (VIRT:port → leg-C listener) + the
    // leg-S routing ONCE via the topology (the single source of truth, RAII-cleaned,
    // FAILURE-PROPAGATING). `expect` is a gate-failure precondition (the real kernel
    // always supports it).
    let agent_port = pick_free_inbound_agent_port();
    topo.install_tproxy(agent_port)
        .expect("inbound TPROXY + leg-S routing must install on the real 6.18 kernel");

    // The identity-unaware server workload S — a CGROUP-ISOLATED NETNS SUBPROCESS,
    // binding the netns veth IP; holds NOTHING (AC3/AC5). The agent's leg-S dial
    // reaches it over the veth via the topology's DNAT of the verbatim orig-dst. S
    // reads the byte-exact request as PLAINTEXT and replies — its subprocess exit code
    // is the byte-exact oracle (derived from S's real recv, not the adapter's books).
    let server = InboundServer::spawn(&topo);

    // WORKER role (test harness): owns the IP_TRANSPARENT leg-C listener. Binds leg C
    // on the agreed `agent_port`, spawns the client (presenting a valid client SVID
    // toward the virtual addr — TPROXY-intercepted to leg C), and `accept()`s leg C +
    // recovers orig-dst via `getsockname`. ONLY the accepted leg crosses into the
    // adapter; the worker is NEVER handed a TLS or server-side fd (AC5).
    let mut worker =
        mtls_roles::InboundWorker::run(&topo, server.addr(), &pki, agent_port, Duration::ZERO);
    let (leg_c, orig_dst) = worker.accept_leg_c_and_orig_dst();

    // AC1 (orig-dst → identity): the worker's recovered orig-dst selected the SERVER
    // workload's allocation; by the time `enforce` runs `conn.alloc` IS that selected
    // server allocation (SD-1 — the orig-dst → alloc resolution is the worker's job).
    // Inside the adapter "orig-dst → identity" is `svid_for(&conn.alloc)` via the
    // identity port. `expected_peer` is `None` (v1 authn-only — #178 is the
    // intended-peer-pinning upgrade).
    let conn = InterceptedConnection {
        leg: leg_c,
        routed: Routed::Inbound { orig_dst },
        alloc: pki.server_alloc.clone(),
        expected_peer: None,
    };
    assert_eq!(
        conn.routed.direction(),
        Direction::Inbound,
        "AC1: the recovered orig-dst routes the connection INBOUND (server side)"
    );

    let handle = adapter
        .enforce(conn)
        .await
        .expect("inbound enforce must reach steady-state-established (server-mTLS OK, NO RST)");

    // AC5 / AC1: the established connection is Running and the deliver pump is
    // PORT-OWNED (the adapter spawned and owns it; the worker only observes). The
    // worker was never handed leg S — `enforce` opened it internally (SD-1).
    assert_eq!(
        adapter.liveness(&handle),
        PumpLiveness::Running,
        "AC5: after enforce(Inbound), liveness observes the deliver SPLICE pump as Running"
    );

    // Attach strace to THIS test process (and its threads, `-f`) AFTER `enforce`
    // returns — the SERVER handshake on leg C has already completed, so strace never
    // PTRACE-stops the in-process client thread mid-handshake (which would surface as
    // a spurious `EOF during handshake`). The deliver SPLICE pump is steady-state and
    // post-handshake: the client sends its request only after `send_delay` (≥400 ms),
    // well after this point, so every splice on the C→S request path is captured.
    // Filtered to the syscalls that distinguish the deliver mechanism; `-s 512 -xx`
    // dumps the read/write buffers so the request plaintext can be confirmed ABSENT
    // from a per-byte deliver-path read/recvfrom off leg C (it rides splice, not a
    // userspace copy). Rust `TcpStream` read/write lower to `recvfrom`/`sendto`; the
    // deliver pump issues `splice`. So the C→S DELIVER ZERO-COPY surfaces as
    // `splice(...)` with NO plaintext `recvfrom`/`read` of the request off leg C.
    //
    // Trace ONLY `splice` + `recvfrom`/`read` — the syscalls AC4 needs (splice
    // present; request NOT copied off leg C via a userspace recvfrom/read). The
    // response S→C copy uses `sendto`/`write`, which are deliberately NOT traced: under
    // `strace -f` every intercepted syscall on every in-process thread (including the
    // client's response read + the response copy pump) carries PTRACE overhead, and
    // tracing the write side too slows the response round-trip past the client's read
    // deadline. The deliver-mechanism evidence the AC needs is on the splice + read
    // side; the response-copy mechanism is the OUTBOUND test's concern, not this one.
    let mut syscalls = StraceProbe::attach_self(&["recvfrom", "splice", "read"]);

    // AC2: while the connection is live, `ss -tie` on the leg-C socket (the
    // client-facing kTLS leg, identified by the VIRT port the TPROXY listener accepted
    // toward) shows the kTLS ULP armed with kTLS-RX (rxconf:sw) — read from the REAL
    // kernel socket state, not the adapter's bookkeeping. This proves the SERVER
    // handshake's extracted secrets were installed as the kTLS-RX keys on leg C
    // (auth-session == data-session), and that leg C is a plain kTLS socket (its ULP
    // is `tls`, NOT a sockmap member — AC4 no-psock-on-RX).
    let ulp = SsUlp::for_local_port(MtlsTopology::VIRT_PORT, agent_port);
    assert!(
        ulp.has_ktls_tls_ulp,
        "AC2: ss -tie on the leg-C socket (VIRT port {} / agent port {agent_port}) must show the \
         kTLS ULP (tcp-ulp-tls) — the SERVER handshake secrets armed kTLS on the client-facing \
         leg; got:\n{}",
        MtlsTopology::VIRT_PORT,
        ulp.raw
    );
    assert!(
        ulp.rx_sw,
        "AC2: the leg-C kTLS ULP must be armed for RX (rxconf:sw) — kTLS-RX so the deliver splice \
         decrypts the client's request out of leg C; got:\n{}",
        ulp.raw
    );

    // Drive the C→S request deliver to completion (client → S request via the deliver
    // splice), then collect the syscall trace and the leg-C wire observations. The
    // INBOUND DELIVER direction (C→S) is THIS focused test's scope; the S→C response
    // round-trip-at-the-client is the composed walking-skeleton (01-01) scope (GAP-2
    // inbound half) — asserting the client received S's reply byte-exact HERE would
    // measure the response leg under `strace -f`'s PTRACE overhead on the in-process
    // client thread, which is the test harness perturbing itself, not a production
    // observable of the inbound deliver mechanism.
    let server_result = server.join();
    let (client_result, wire) = worker.join_client();
    let trace = syscalls.detach_and_read();

    // AC3 (byte-exact plaintext to the server S): the identity-unaware server workload
    // read the byte-exact request as PLAINTEXT off leg S (its subprocess exit code is
    // the oracle — a real recv, not the adapter's books). The decrypted plaintext
    // appears ONLY on the agent→server leg S; the client wire carries ciphertext only
    // (asserted below). THIS is the inbound deliver-direction plaintext oracle.
    assert!(
        server_result.received_request_byte_exact,
        "AC3 (PLAINTEXT_EXACT): the identity-unaware server workload S must receive the byte-exact \
         decrypted plaintext request off leg S"
    );
    assert!(
        !server_result.observed_rst && !client_result.observed_rst,
        "the inbound transfer must NOT RST"
    );

    // AC1: the inbound client's `WebPkiClientVerifier`-shape verification accepted the
    // SERVER leaf the agent's leg-C SERVER handshake PRESENTED, and the SPIFFE-SAN it
    // extracted from chain position 0 IS the held SERVER SVID's SPIFFE — proving the
    // agent read `server_alloc`'s held server SVID via `IdentityRead::svid_for` (the
    // orig-dst → identity selection) and presented it on leg C. Without the client's
    // verify this gate would pass even if the SVID-presentation path were broken.
    assert_eq!(
        client_result.presented_server_spiffe.as_ref(),
        Some(&pki.server_leaf.spiffe),
        "AC1: the agent's leg-C SERVER handshake must present the HELD server SVID (selected by \
         orig-dst → alloc → svid_for), and the client must verify it with the expected SPIFFE SAN"
    );

    // AC4 (deliver = zero-copy SPLICE): the agent used `splice` on the C→S deliver path
    // and NEVER copied the request plaintext through a userspace `read`/`recvfrom`
    // buffer off leg C. The request marker appearing in a `read`/`recvfrom` buffer off
    // leg C would mean the deliver was a copy, not a splice — assert it is absent.
    // `splice` calls must be present (the deliver decrypt pump runs ~1 splice per
    // record out of leg C's kTLS-RX).
    assert!(
        trace.splice_calls > 0,
        "AC4: the deliver path must be a zero-copy splice out of leg C's kTLS-RX — at least one \
         splice(2) must be traced; strace summary:\n{}",
        trace.summary()
    );
    assert!(
        !trace.request_delivered_through_io_copy,
        "AC4 (splice-only deliver): the request plaintext must NEVER appear in a traced \
         read(2)/recvfrom(2) buffer off leg C (a copy through userspace). It rides splice(2) out \
         of leg C's kTLS-RX, decrypted by the kernel on splice-out — no psock on leg C's RX. \
         strace summary:\n{}",
        trace.summary()
    );

    // AC3 (client leg 0x17-only; cleartext-hits = 0): TLS 1.3 ciphertext on the
    // client-facing leg C in the request (C→S) direction — the request frames ≥1
    // `0x17` application_data record — and NEITHER the request nor any response
    // plaintext EVER appears on leg C in EITHER direction (cleartext-hits = 0; the
    // capture is bidirectional). The request-direction record count is the inbound
    // deliver-direction confidentiality oracle THIS test owns; the S→C response record
    // count is the composed walking-skeleton (01-01) scope. Scanned AFTER the round-trip
    // joins so the scan covers the actual application payload, not just the handshake.
    assert!(
        wire.records_request_dir >= 1,
        "AC3: leg C request (C→S) must carry TLS 1.3 application_data (0x17) records; got {}",
        wire.records_request_dir
    );
    assert_eq!(
        wire.plaintext_marker_hits, 0,
        "AC3 (cleartext-hits = 0): neither the request nor the response plaintext may EVER appear \
         on the client-facing leg C (the capture is bidirectional) — decrypted plaintext appears \
         ONLY on the agent→server leg S"
    );

    // AC5: teardown reclaims the connection — liveness goes Gone, both pumps stopped,
    // BOTH legs (leg C + the adapter-owned leg S) closed. The deliver pump was
    // PORT-OWNED; the worker never drove it and was never handed leg S.
    adapter.teardown(handle.clone()).await.expect("inbound teardown");
    assert_eq!(
        adapter.liveness(&handle),
        PumpLiveness::Gone,
        "AC5: post-teardown the port-owned pumps are stopped and BOTH legs (C + adapter-owned S) \
         reclaimed (Gone)"
    );
}

// =====================================================================
// strace syscall oracle — attach `strace -f -p <self>` to the running test process so
// the agent's own pump threads' syscalls are captured, then parse the trace for the
// inbound deliver mechanism (C→S zero-copy splice, request never copied through a
// userspace read/recvfrom off leg C).
// =====================================================================

/// A live `strace` attached to this test process (and its threads). Captures the raw
/// syscall log to a temp file; `detach_and_read` stops it and parses.
struct StraceProbe {
    child: Option<Child>,
    out_path: std::path::PathBuf,
}

impl StraceProbe {
    /// Attach `strace -f -p <self_pid>` filtered to `syscalls`, dumping read/write
    /// buffers (`-s 512 -xx`) so the request plaintext can be confirmed ABSENT from
    /// per-byte deliver-path read/recvfrom. Blocks briefly until strace has attached
    /// (so the pump syscalls that follow are captured).
    fn attach_self(syscalls: &[&str]) -> Self {
        let pid = std::process::id();
        let out_path = std::env::temp_dir().join(format!("mtls-inbound-strace-{pid}.log"));
        let _ = std::fs::remove_file(&out_path);
        let trace_arg = format!("trace={}", syscalls.join(","));
        let child = Command::new("strace")
            .args(["-f", "-q", "-qq"])
            .args(["-e", &trace_arg])
            .args(["-s", "512", "-xx"])
            .args(["-o", out_path.to_str().expect("utf8 path")])
            .args(["-p", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn strace -p self");
        // Give strace a moment to attach to every thread before enforce spawns the
        // pumps; a few hundred ms is ample on the Lima VM.
        std::thread::sleep(Duration::from_millis(400));
        Self { child: Some(child), out_path }
    }

    /// Stop strace (SIGTERM → it detaches cleanly and flushes the log), read the
    /// captured trace, and parse it for the deliver mechanism evidence.
    fn detach_and_read(&mut self) -> TraceFindings {
        // Let the steady-state round-trip's last records flush, then detach.
        std::thread::sleep(Duration::from_millis(300));
        if let Some(mut child) = self.child.take() {
            // SIGTERM makes strace detach (PTRACE_DETACH) and flush its output file.
            let pid = child.id();
            let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).status();
            let _ = child.wait();
        }
        // strace flushes on detach; a brief settle covers the file write.
        std::thread::sleep(Duration::from_millis(150));
        let raw = std::fs::read_to_string(&self.out_path).unwrap_or_default();
        // Diagnostic dump of the agent's splice lines so a deliver-mechanism mismatch is
        // debuggable from the captured nextest output.
        for line in raw.lines() {
            let body = strip_strace_pid_prefix(line);
            if body.starts_with("splice(") {
                let head: String = body.chars().take(80).collect();
                eprintln!("STRACE: {head}");
            }
        }
        TraceFindings::parse(&raw)
    }
}

impl Drop for StraceProbe {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_file(&self.out_path);
    }
}

/// The inbound deliver-mechanism evidence parsed from the strace log.
struct TraceFindings {
    /// `splice(2)` was used (the deliver zero-copy decrypt pump, C→S).
    splice_calls: usize,
    /// The request plaintext appeared in a traced `read(2)`/`recvfrom(2)` buffer off a
    /// splice-source leg (leg C) — MUST be false (the deliver is zero-copy splice; a
    /// copy through userspace would surface here, AC4).
    request_delivered_through_io_copy: bool,
    write_calls: usize,
    read_calls: usize,
}

impl TraceFindings {
    /// A distinctive interior substring of the inbound request (INBOUND_REQUEST). If
    /// the deliver were a userspace copy, this plaintext would appear in a
    /// read/recvfrom buffer off leg C. It must NOT (the deliver is a zero-copy splice).
    /// Kept in sync with `mtls_roles::INBOUND_REQUEST`.
    fn request_marker() -> Vec<u8> {
        b"must_arrive_as_plaintext_at_server_S_agent_light_in_order_0003".to_vec()
    }

    fn parse(raw: &str) -> Self {
        let mut splice_calls = 0usize;
        let mut write_calls = 0usize;
        let mut read_calls = 0usize;
        let mut request_delivered_through_io_copy = false;

        // `-xx` renders buffers as `\xHH\xHH...`; convert the marker to that hex form
        // so a substring match against the raw line finds the plaintext regardless of
        // where strace truncated the buffer or split it across records.
        let req_hex = to_strace_hex(&Self::request_marker());

        // The agent's deliver pump's splice SOURCE fds — `splice(SRC, NULL, DST, NULL,
        // len, flags)`. Leg C (the kTLS-RX leg the request is decrypted out of) is one
        // of these. A `read`/`recvfrom` of the request plaintext off a splice-SOURCE fd
        // would mean the agent COPIED the request out of leg C through userspace (NOT
        // zero-copy) — the falsification of AC4. Collecting splice sources isolates the
        // AGENT's deliver path from the in-process worker/client's own request-send (a
        // different socket fd, never a splice source of the agent's pumps), so the
        // client's legitimate `sendto(request)` into ITS kTLS-TX cannot false-trip.
        let mut splice_src_fds: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();

        for line in raw.lines() {
            // strace `-f` prefixes each line with the traced thread's PID then a space:
            // `<pid> syscall(args) = ret`, with blocking calls split as
            // `... <unfinished ...>` / `<... syscall resumed> ...`. Strip the PID prefix
            // and the `<... resumed>` marker so the buffer content on either fragment is
            // matched. Classify by the leading syscall-name token.
            //
            // The agent's deliver ZERO-COPY pump is `splice` out of leg C's kTLS-RX; a
            // `read`/`recvfrom(legC, <request-plaintext>)` carrying the request would be
            // the COPY-through-userspace falsification.
            let body = strip_strace_pid_prefix(line);
            let is_resume = body.starts_with("<...");
            let names = |n: &str| body.starts_with(n) || (is_resume && body.contains(n));
            let carries_req = body.contains(&req_hex);

            if names("splice(") {
                splice_calls += 1;
                if let Some(src) = splice_source_fd(body) {
                    splice_src_fds.insert(src);
                }
            } else if names("sendto(") || names("write(") {
                write_calls += 1;
            } else if names("recvfrom(") || names("read(") {
                read_calls += 1;
                // A request marker carried by a `recvfrom`/`read` OFF a splice-source leg
                // (i.e. leg C, the agent's kTLS-RX source) would be the agent copying the
                // request through userspace — the AC4 falsification. The client's own
                // request-send targets a non-splice-source fd and is correctly ignored.
                if carries_req && syscall_fd(body).is_some_and(|fd| splice_src_fds.contains(&fd)) {
                    request_delivered_through_io_copy = true;
                }
            }
        }

        Self { splice_calls, request_delivered_through_io_copy, write_calls, read_calls }
    }

    fn summary(&self) -> String {
        format!(
            "splice={} write={} read={} request_copy_seen={}",
            self.splice_calls,
            self.write_calls,
            self.read_calls,
            self.request_delivered_through_io_copy,
        )
    }
}

/// Strip strace's leading `<pid> ` prefix (present under `-f`) so the syscall name is
/// at the start of the returned slice. A line with no leading-digit prefix is returned
/// unchanged.
fn strip_strace_pid_prefix(line: &str) -> &str {
    let trimmed = line.trim_start();
    let rest = trimmed.trim_start_matches(|c: char| c.is_ascii_digit());
    if rest.len() < trimmed.len() { rest.trim_start() } else { trimmed }
}

/// The first-argument fd of a `syscall(FD, ...)` line (e.g. `recvfrom(26, ...)` →
/// `Some(26)`). `body` has already had its PID prefix stripped. `None` if the args do
/// not begin with an integer (e.g. a `<... resumed>` fragment that omits the fd).
fn syscall_fd(body: &str) -> Option<i32> {
    let open = body.find('(')?;
    let after = &body[open + 1..];
    let end = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
    after[..end].parse::<i32>().ok()
}

/// The source fd of a `splice(SRC, NULL, DST, NULL, len, flags)` line — the FIRST
/// positional argument. `body` has its PID prefix stripped. `None` on a `<... resumed>`
/// fragment or a malformed line.
fn splice_source_fd(body: &str) -> Option<i32> {
    let open = body.find("splice(")? + "splice(".len();
    let args = &body[open..];
    // splice args are comma-separated: SRC, off_in, DST, off_out, len, flags
    let src = args.split(',').next()?.trim();
    let end = src.find(|c: char| !c.is_ascii_digit()).unwrap_or(src.len());
    src.get(..end)?.parse::<i32>().ok()
}

/// Render `bytes` as the `\xHH\xHH...` hex form strace `-xx` emits, so a marker can be
/// substring-matched against a traced buffer line.
fn to_strace_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 4);
    for b in bytes {
        let _ = write!(s, "\\x{b:02x}");
    }
    s
}

// =====================================================================
// ss -tie ULP oracle — read the REAL kernel socket state for the leg-C socket and
// confirm the kTLS ULP is armed for RX (rxconf:sw). Inbound leg C is the ACCEPTED
// socket on the agent's IP_TRANSPARENT listener; under TPROXY its local addr IS the
// orig-dst (VIRT_IP:VIRT_PORT), so the VIRT port identifies leg C in `ss` output. The
// agent_port (the listener bind port) is also accepted as an identifier for ss
// versions that surface the listener's local port on the accepted socket.
// =====================================================================

/// The kTLS-ULP facts read from `ss -tie` for the leg-C socket.
struct SsUlp {
    has_ktls_tls_ulp: bool,
    rx_sw: bool,
    raw: String,
}

impl SsUlp {
    /// Run `ss -tie` and locate the socket record(s) touching `virt_port` or
    /// `agent_port` (leg C is the agent's accepted IP_TRANSPARENT socket; under TPROXY
    /// its local addr is the orig-dst VIRT port). The kTLS ULP renders as
    /// `tcp-ulp-tls rxconf:sw txconf:sw ...` in the `-i` (info) block, on the
    /// continuation line(s) following the socket's address line.
    fn for_local_port(virt_port: u16, agent_port: u16) -> Self {
        let out = Command::new("ss").args(["-tie"]).output().expect("run ss -tie");
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let virt_tok = format!(":{virt_port}");
        let agent_tok = format!(":{agent_port}");
        let mut relevant = String::new();
        let mut in_record = false;
        for line in text.lines() {
            let starts_record = line.starts_with("ESTAB")
                || line.starts_with("CLOSE")
                || line.starts_with("TIME-WAIT")
                || line.starts_with("State")
                || !line.starts_with(char::is_whitespace);
            if starts_record {
                // A new record begins; it is relevant iff its address line names the
                // VIRT port (the orig-dst leg C is accepted toward) or the agent port.
                in_record = line.contains(&virt_tok) || line.contains(&agent_tok);
            }
            if in_record {
                relevant.push_str(line);
                relevant.push('\n');
            }
        }
        // `ss` renders the ULP info as `tcp-ulp-tls version: 1.3 cipher: aes-gcm-256
        // rxconf: sw txconf: sw` — tolerate both spaced (`rxconf: sw`) and compact
        // (`rxconf:sw`) forms across `ss` versions.
        let has_ktls_tls_ulp = relevant.contains("tcp-ulp-tls");
        let rx_sw = relevant.contains("rxconf: sw") || relevant.contains("rxconf:sw");
        Self { has_ktls_tls_ulp, rx_sw, raw: relevant }
    }
}
