//! OUTBOUND enforce — the per-direction wire/syscall observables of the
//! asymmetric agent-light steady state (transparent-mtls-host-socket step 02-03,
//! ADR-0069 F3/F5; GH #26).
//!
//! Step 01-01 (the composed walking skeleton) and 02-02 (the handshake-identity
//! proof) already drove `HostMtlsEnforcement::enforce(Outbound)` to
//! steady-state-established on the real netns/veth topology and proved the
//! BIDIRECTIONAL wire confidentiality (0x17 records both ways, cleartext = 0).
//! What NEITHER proved is the **per-direction MECHANISM asymmetry** the D-MTLS-13
//! contract pins: the forward path is an AGENT-LIGHT `read(legF) → write_all(legB)`
//! COPY into leg B's kTLS-TX (a per-record `read`+`write`, plaintext through a
//! userspace buffer — NOT zero-copy, NOT a splice into TX, which loses records),
//! while the return path is an AGENT-LIGHT ZERO-COPY `splice(legB → legF)` out of
//! leg B's kTLS-RX (only `splice`/`ppoll`, no plaintext `read`/`write`).
//!
//! THIS test isolates the OUTBOUND direction and asserts the five 02-03 ACs from
//! REAL kernel observables — `strace` on the agent's own pump threads, `ss -tie`
//! on the peer-facing leg, and the AF_PACKET wire oracle — through the SAME
//! `MtlsEnforcement` driving port (`enforce` / `liveness` / `teardown`). The
//! intercept setup + the leg-F listener + the `accept()` are the WORKER role the
//! test harness stands in for (step 07-01); only the accepted leg crosses into the
//! adapter.
//!
//! The five ACs:
//! - **AC1** — the handshake's extracted secrets ARM kTLS on leg B (auth-session ==
//!   data-session): proven by the peer reconstructing the byte-exact request via
//!   kTLS-RX decrypt + the wire carrying 0x17 (no separately negotiated session).
//! - **AC2 (forward agent-light COPY)** — `strace` on the agent shows the forward
//!   path moves through a per-record `read`+`write` whose `write` buffer CARRIES the
//!   request plaintext (copy through userspace); `tcpdump`/AF_PACKET shows 0x17 on
//!   the peer wire (the kernel `tls_sw_sendmsg` encrypted each blocking `write`).
//!   NOT zero-copy, NOT a splice into TX.
//! - **AC3 (return agent-light ZERO-COPY)** — `strace` shows the return path uses
//!   ONLY `splice`/`ppoll` and NEVER a plaintext `read`/`write` of the reply; the
//!   reply arrives byte-exact at the workload, ~1 splice per record.
//! - **AC4 (kTLS ULP on the peer leg)** — `ss -tie` on the leg-B socket shows
//!   `tcp-ulp-tls` `rxconf:sw txconf:sw`; the peer wire cleartext count = 0 (the K1
//!   North-Star observable, unchanged by the forward-mechanism pivot since the
//!   encrypt is still kernel-side kTLS-TX).
//! - **AC5 (Tier-3 invariant)** — leg B carries NO psock on EITHER direction (the
//!   sockmap is gone, D-MTLS-13): the leg-B socket's ULP is `tls` (not a sockmap
//!   member), the return splice DELIVERS byte-exact (a psock'd RX leg would break
//!   `tls_sw_splice_read`), and `enforce` returns `liveness == Running` with the
//!   return pump PORT-OWNED (SD-2). The former "SOCKMAP-after-TCP_ULP → EINVAL →
//!   ArmingOrderViolation" invariant is RETIRED (2026-06-13) — there is no sockmap
//!   insert on any path now, so it is NOT tested.
//!
//! **Litmus (falsifiability / port-to-port)**: if the call-site that wires the
//! forward write_all pump + the return splice pump in `outbound::establish` were
//! deleted, the round-trip would not deliver, `liveness` would not be `Running`, and
//! the strace `splice`/`write` evidence would vanish — every assertion below goes
//! RED. The observables are derived from REAL captured syscalls + captured wire
//! bytes + the real `ss` ULP read, never from the adapter's own bookkeeping.
//!
//! Tier 3 ONLY (sockops/cgroup_connect4/kTLS/splice have no `BPF_PROG_TEST_RUN`):
//! `cargo xtask lima run -- cargo nextest run -p overdrive-dataplane --features
//! integration-tests -E 'test(mtls_outbound_enforce)'`, ACTUALLY EXECUTING on the
//! real 6.18+ kernel (a `--no-run` gate is green even when every fixture refuses at
//! boot).

#![cfg(target_os = "linux")]
// `unwrap`/`expect` are the standard test idiom — a panic with a message is exactly
// the right failure for a precondition.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
// The SKIP-loud path + strace diagnostics print via `eprintln!` (nextest captures
// it); the role helpers take `&mut self` because they own spawned-child state.
#![allow(clippy::print_stderr, clippy::needless_pass_by_ref_mut)]
// The single composed Tier-3 acceptance fn drives ALL five ACs end-to-end (one
// real round-trip under one strace attach) — splitting it would re-stand-up the
// netns/cgroup/peer topology per AC for no behavioural gain. The leg names
// (leg F/B/C/S) + ADR-0069 / D-MTLS / contract tokens in the doc comments are the
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
use super::helpers::mtls_roles::{OutboundPeer, OutboundWorkload};

/// The agent's held-identity store — the ONLY holder of SVID material. The
/// workloads hold nothing; the agent reads through THIS `IdentityRead` port and
/// NEVER mints (#26 is a reader). `None` is explicit absence.
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

/// Build the held-identity store from the test PKI: the client SVID (outbound leg)
/// plus the shared trust bundle. The leaf material lives HERE, with the agent —
/// never with the workload.
fn held_identities(pki: &TestPki) -> HeldIdentities {
    let mut svids = std::collections::BTreeMap::new();
    svids.insert(pki.client_alloc.clone(), pki.client_svid_material());
    HeldIdentities { svids, bundle: pki.trust_bundle() }
}

/// The OUTBOUND-isolated 02-03 acceptance gate. Drives ONE outbound flow through
/// `HostMtlsEnforcement::enforce` on the real netns/veth + cgroup topology while a
/// `strace` attaches to the agent's pump threads, then asserts the per-direction
/// mechanism asymmetry (forward write_all COPY / return zero-copy SPLICE), the kTLS
/// ULP on the peer leg, the cleartext-free wire, and `liveness == Running`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn outbound_enforce_ktls_tx_forward_writeall_copy_return_zerocopy_splice_tls13_wire() {
    let tag = format!("ob{}", std::process::id());
    // The canonical gate runs `cargo xtask lima run -- …` as root on the real 6.18+
    // kernel, where the topology is ALWAYS supported. A `TopologyError::Unsupported`
    // here is NOT a legitimate skip — it is the same "green without executing" hole a
    // `--no-run` gate has. Fail loud so a degraded environment cannot pass by skipping.
    let topo = match MtlsTopology::create(&tag) {
        Ok(t) => t,
        Err(e @ TopologyError::Unsupported(_)) => panic!(
            "outbound-enforce gate MUST run on the real kernel (root + CAP_NET_ADMIN + \
             cgroup v2); a topology-unsupported here is a gate FAILURE, not a skip — run via \
             `cargo xtask lima run -- cargo nextest run -p overdrive-dataplane \
             --features integration-tests -E 'test(mtls_outbound_enforce)'`: {e}"
        ),
        Err(e) => panic!("topology setup failed (not a skip): {e}"),
    };
    // strace must be present (the syscall oracle is load-bearing); its absence is a
    // gate FAILURE, not a skip — the canonical Lima VM ships it.
    assert!(
        Command::new("strace").arg("-V").output().is_ok_and(|o| o.status.success()),
        "strace is required for the outbound-enforce syscall oracle (forward read+write copy / \
         return splice-only); it is present in the canonical Lima VM — its absence is a gate \
         failure, not a skip"
    );

    let pki = TestPki::mint();
    let identity: Arc<dyn IdentityRead> = Arc::new(held_identities(&pki));
    let adapter = HostMtlsEnforcement::new(identity, MtlsLimits::default());

    adapter
        .probe()
        .await
        .expect("Earned-Trust probe must pass on the real kernel before any enforce");

    // The real outbound mTLS peer the agent's leg B dials. It REQUIRE+VERIFYs the
    // client SVID, arms kTLS-RX to decrypt the workload's request, and replies (the
    // B→F return leg). Its AF_PACKET capture on `lo` is the confidentiality oracle.
    let peer = OutboundPeer::spawn(&pki);
    let peer_port = peer.addr().port();

    // WORKER role (test harness): owns the leg-F listener + cgroup_connect4 intercept,
    // spawns the cgroup-isolated workload, and accepts leg F. Only the accepted leg
    // crosses into the adapter.
    let mut workload = OutboundWorkload::run(&topo, peer.addr(), Duration::ZERO);
    let leg_f = workload.accept_leg_f();

    let conn = InterceptedConnection {
        leg: leg_f,
        routed: Routed::Outbound { peer: peer.addr() },
        alloc: pki.client_alloc.clone(),
        expected_peer: None, // v1 authn-only (#178 is the intended-peer-pinning upgrade)
    };
    assert_eq!(conn.routed.direction(), Direction::Outbound);

    // Attach strace to THIS test process (and its threads, `-f`) BEFORE `enforce`
    // spawns the pump threads, filtered to the syscalls that distinguish the two
    // mechanisms. The agent pumps run as `std::thread::spawn` threads inside
    // `HostMtlsEnforcement`; `strace -f -p <self>` follows them. `-yy` annotates fds
    // with their socket identity (proto + endpoint) so the leg-B fd is identifiable;
    // `-s 512 -xx` dumps the read/write buffers so the forward-copy plaintext is
    // visible and the return path can be confirmed plaintext-free.
    // Trace the syscalls the agent's pumps actually issue. Rust's `TcpStream`
    // read/write lower to `recvfrom`/`sendto` (with `MSG_NOSIGNAL`), NOT `read`/`write`;
    // the return splice pump issues `splice`. So the forward COPY surfaces as a
    // `sendto(legB, <plaintext>)` + `recvfrom(legF, ...)` per record, and the return
    // ZERO-COPY surfaces as `splice(...)` with NO plaintext `sendto`/`recvfrom` of the
    // reply. `read`/`write` are also traced to catch any non-socket copy.
    let mut syscalls = StraceProbe::attach_self(&["sendto", "recvfrom", "splice", "read", "write"]);

    let handle = adapter
        .enforce(conn)
        .await
        .expect("outbound enforce must reach steady-state-established (NO RST)");

    // AC5 / AC1: the established connection is Running and the return pump is
    // PORT-OWNED (the adapter spawned and owns it; the worker only observes).
    assert_eq!(
        adapter.liveness(&handle),
        PumpLiveness::Running,
        "AC5: after enforce(Outbound), liveness observes the forward COPY pump as Running"
    );

    // AC4: while the connection is live, `ss -tie` on the leg-B socket (the
    // peer-facing kTLS leg, identified by the peer port) shows the kTLS ULP armed in
    // BOTH directions (rxconf:sw txconf:sw) — read from the REAL kernel socket state,
    // not the adapter's bookkeeping. This proves the handshake's extracted secrets
    // were installed as the kTLS keys on leg B (auth-session == data-session, AC1),
    // and that leg B is a plain kTLS socket (its ULP is `tls`, NOT a sockmap member —
    // AC5 no-psock).
    let ulp = SsUlp::for_peer_port(peer_port);
    assert!(
        ulp.has_ktls_tls_ulp,
        "AC4/AC1: ss -tie on the leg-B socket (peer port {peer_port}) must show the kTLS ULP \
         (tcp-ulp-tls) — the handshake secrets armed kTLS on the peer-facing leg; got:\n{}",
        ulp.raw
    );
    assert!(
        ulp.tx_sw && ulp.rx_sw,
        "AC4: the leg-B kTLS ULP must be armed in BOTH directions (rxconf:sw txconf:sw) — \
         kTLS-TX so the forward write_all encrypts, kTLS-RX so the return splice decrypts; \
         got:\n{}",
        ulp.raw
    );

    // Drive the bidirectional round-trip to completion (forward F→B request, return
    // B→F reply), then collect the syscall trace.
    let round_trip = workload.join();
    let trace = syscalls.detach_and_read();

    assert!(
        round_trip.forward_delivered_byte_exact,
        "AC2: the outbound forward F→B must deliver the workload's request byte-exact to the \
         peer (the kTLS-TX write_all copy)"
    );
    assert!(
        round_trip.return_delivered_byte_exact,
        "AC3: the outbound return B→F must deliver the peer's reply byte-exact to the workload \
         (the kTLS-RX zero-copy splice)"
    );
    assert!(!round_trip.observed_rst, "the outbound post-arm transfer must NOT RST");

    // AC1: the peer's WebPkiClientVerifier accepted the presented client leaf and the
    // SPIFFE-SAN it extracted IS the held client SVID's SPIFFE — the auth session
    // whose secrets became the kTLS data session.
    assert_eq!(
        peer.presented_client_spiffe().as_ref(),
        Some(&pki.client_leaf.spiffe),
        "AC1: the agent's leg-B handshake must present the held client SVID (auth-session whose \
         extracted secrets ARE the kTLS keys on leg B)"
    );

    // AC2 (forward = COPY, NOT zero-copy / NOT splice-into-TX): the agent moved the
    // forward path through a per-record `read` + `write` whose `write` buffer CARRIED
    // the request plaintext through a userspace buffer. The request was written by the
    // workload in two phases (pre-arm + steady-state); the STEADY-STATE phase rides
    // the forward encrypt pump's `read(legF) → write(legB)` copy, so the steady-state
    // request bytes appear in a traced `write(...)` buffer. A splice into kTLS-TX
    // would show `splice` toward the TX leg and NO such write — and would lose records.
    assert!(
        trace.forward_request_copied_through_write,
        "AC2: the forward path must be a per-record read→write COPY — the steady-state request \
         plaintext must appear in a traced sendto(2)/write(2) buffer (Rust TcpStream::write lowers \
         to sendto; the agent copies plaintext through a userspace buffer into leg B's kTLS-TX, \
         where the kernel tls_sw_sendmsg encrypts each write). A splice into kTLS-TX (which loses \
         records) would show no such write. strace summary:\n{}",
        trace.summary()
    );

    // AC3 (return = zero-copy SPLICE): the agent used `splice` on the return path and
    // NEVER copied the reply plaintext through a userspace `read`/`write` buffer. The
    // reply marker appearing in a `write(...)` buffer would mean the return was a copy,
    // not a splice — assert it is absent. `splice` calls must be present (the return
    // decrypt pump runs ~1 splice per record out of leg B's kTLS-RX).
    assert!(
        trace.splice_calls > 0,
        "AC3: the return path must be a zero-copy splice out of leg B's kTLS-RX — at least one \
         splice(2) must be traced; strace summary:\n{}",
        trace.summary()
    );
    assert!(
        !trace.return_reply_copied_through_io,
        "AC3: the return path must be ZERO-COPY — the reply plaintext must NEVER appear in a \
         traced read(2)/write(2) buffer (a copy through userspace). It rides splice(2) out of \
         leg B's kTLS-RX, decrypted by the kernel on splice-out. strace summary:\n{}",
        trace.summary()
    );

    // AC4 (the K1 North-Star observable, unchanged by the D-MTLS-13 pivot): MULTI-record
    // TLS 1.3 ciphertext on the peer-facing leg B in BOTH directions, and NEITHER the
    // request nor the reply plaintext EVER appears on the peer wire (cleartext = 0).
    let wire = peer.wire_observations();
    assert!(
        wire.records_request_dir >= 1,
        "AC2/AC4: leg B forward (F→B) must carry TLS 1.3 application_data (0x17) records; got {}",
        wire.records_request_dir
    );
    assert!(
        wire.records_response_dir >= 1,
        "AC3/AC4: leg B return (B→F) must carry TLS 1.3 application_data (0x17) records; got {}",
        wire.records_response_dir
    );
    assert_eq!(
        wire.plaintext_marker_hits, 0,
        "AC4 (K1): neither the request nor the reply plaintext may EVER appear on the \
         peer-facing leg B — cleartext count on that leg MUST be 0"
    );

    // AC5: teardown reclaims the connection — liveness goes Gone, both pumps stopped,
    // legs closed (the return pump was PORT-OWNED; the worker never drove it).
    adapter.teardown(handle.clone()).await.expect("outbound teardown");
    assert_eq!(
        adapter.liveness(&handle),
        PumpLiveness::Gone,
        "AC5: post-teardown the port-owned pumps are stopped and the legs reclaimed (Gone)"
    );
    peer.shutdown();
}

// =====================================================================
// strace syscall oracle — attach `strace -f -p <self>` to the running test process
// so the agent's own pump threads' syscalls are captured, then parse the trace for
// the per-direction mechanism asymmetry.
// =====================================================================

/// A live `strace` attached to this test process (and its threads). Captures the
/// raw syscall log to a temp file; `detach_and_read` stops it and parses.
struct StraceProbe {
    child: Option<Child>,
    out_path: std::path::PathBuf,
}

impl StraceProbe {
    /// Attach `strace -f -p <self_pid>` filtered to `syscalls`, dumping read/write
    /// buffers (`-s 512 -xx`) so the forward-copy plaintext is visible and the return
    /// path can be confirmed copy-free. Blocks briefly until strace has attached (so
    /// the pump syscalls that follow are captured).
    fn attach_self(syscalls: &[&str]) -> Self {
        let pid = std::process::id();
        let out_path = std::env::temp_dir().join(format!("mtls-outbound-strace-{pid}.log"));
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
    /// captured trace, and parse it for the per-direction mechanism evidence.
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
        // Diagnostic dump of the agent's leg sendto/recvfrom/splice lines so a
        // forward/return mismatch is debuggable from the captured nextest output. (The
        // `ss` subprocess's own stdout write(1,...) is filtered out — fd 1.)
        for line in raw.lines() {
            let body = strip_strace_pid_prefix(line);
            // Show the sendto's that carried data and the splices; skip the idle
            // recvfrom EAGAIN poll noise (the forward pump's 40 ms-timeout reads).
            let sendto_with_data = body.starts_with("sendto(") && !body.contains("EAGAIN");
            if sendto_with_data || body.starts_with("splice(") {
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

/// The per-direction mechanism evidence parsed from the strace log.
struct TraceFindings {
    /// `splice(2)` was used (the return zero-copy decrypt pump).
    splice_calls: usize,
    /// The STEADY-STATE forward request plaintext appeared in a traced `write(2)`
    /// buffer — the forward is a per-record read→write COPY through userspace (AC2).
    forward_request_copied_through_write: bool,
    /// The return reply plaintext appeared in a traced `read(2)`/`write(2)` buffer —
    /// MUST be false (the return is zero-copy splice; a copy would surface here, AC3).
    return_reply_copied_through_io: bool,
    write_calls: usize,
    read_calls: usize,
}

impl TraceFindings {
    /// The steady-state forward marker (the SECOND half of the request the workload
    /// sends AFTER the agent arms kTLS + spawns the forward pump — so it rides the
    /// steady-state read→write copy, not the pre-arm drain). Kept in sync with
    /// `mtls_roles::OUTBOUND_REQUEST` (split at the midpoint by the workload script).
    fn steady_state_forward_marker() -> Vec<u8> {
        // The role harness splits OUTBOUND_REQUEST at len/2; the steady-state phase is
        // the second half (`[split..]`). This substring lies ENTIRELY inside that
        // second half (verified: index 72 of a 110-byte request, split at 55), so a
        // match proves the steady-state request plaintext rode a userspace `write`
        // through the forward COPY pump — not the pre-arm prelude, not a splice.
        b"arrive_TLS13_decrypted_byte_exact_0001".to_vec()
    }

    /// A distinctive interior substring of the return reply (OUTBOUND_REPLY) — if the
    /// return were a userspace copy, this plaintext would appear in a read/write
    /// buffer. It must NOT (the return is a zero-copy splice).
    fn return_reply_marker() -> Vec<u8> {
        b"return_leg_must_splice_back_to_workload_byte_exact_0002".to_vec()
    }

    fn parse(raw: &str) -> Self {
        let mut splice_calls = 0usize;
        let mut write_calls = 0usize;
        let mut read_calls = 0usize;
        let mut forward_request_copied_through_write = false;
        let mut return_reply_copied_through_io = false;

        // `-xx` renders buffers as `\xHH\xHH...`; convert the markers to that hex form
        // so a substring match against the raw line finds the plaintext regardless of
        // where strace truncated the buffer or split it across records.
        let fwd_hex = to_strace_hex(&Self::steady_state_forward_marker());
        let reply_hex = to_strace_hex(&Self::return_reply_marker());

        // The agent's return pump's splice DESTINATION fds — `splice(src, NULL, DST,
        // NULL, len, flags)`. Leg F (the plaintext destination the workload reads) is
        // one of these. A `sendto`/`write` to a splice-destination fd carrying the reply
        // would mean the agent COPIED the reply into that leg (NOT zero-copy) — the
        // falsification of AC3. Collecting splice destinations isolates the AGENT's
        // return path from the in-process PEER's own reply-send (a different socket fd,
        // never a splice destination of the agent's pumps), so the peer's legitimate
        // `sendto(reply)` into ITS kTLS-TX cannot false-trip the assertion.
        let mut splice_dst_fds: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();

        for line in raw.lines() {
            // strace `-f` prefixes each line with the traced thread's PID then a space:
            // `<pid> syscall(args) = ret` (NOT bracketed), with blocking calls split as
            // `... <unfinished ...>` / `<... syscall resumed> ...`. Strip the PID prefix
            // and the `<... resumed>` marker so the buffer content on either fragment is
            // matched. Classify by the leading syscall-name token.
            //
            // The agent's forward COPY pump is Rust `TcpStream` read/write, which lower
            // to `recvfrom`/`sendto` (NOT `read`/`write`); the return ZERO-COPY pump is
            // `splice`. So `sendto(legB, <plaintext>)` carrying the forward request is
            // the COPY-through-userspace evidence; `splice` is the return zero-copy
            // evidence.
            let body = strip_strace_pid_prefix(line);
            let is_resume = body.starts_with("<...");
            let names = |n: &str| body.starts_with(n) || (is_resume && body.contains(n));
            let carries_fwd = body.contains(&fwd_hex);
            let carries_reply = body.contains(&reply_hex);

            if names("splice(") {
                splice_calls += 1;
                if let Some(dst) = splice_destination_fd(body) {
                    splice_dst_fds.insert(dst);
                }
            } else if names("sendto(") || names("write(") {
                write_calls += 1;
                if carries_fwd {
                    // The forward request plaintext rode a userspace write buffer — the
                    // forward path is a per-record COPY (NOT a splice into kTLS-TX).
                    forward_request_copied_through_write = true;
                }
                // A reply carried by a `sendto`/`write` INTO a splice-destination leg
                // (i.e. leg F, the agent's plaintext return destination) would be the
                // agent copying the reply through userspace — the AC3 falsification.
                // The peer's own reply-send targets a non-splice-destination fd and is
                // correctly ignored.
                if carries_reply && syscall_fd(body).is_some_and(|fd| splice_dst_fds.contains(&fd))
                {
                    return_reply_copied_through_io = true;
                }
            } else if names("recvfrom(") || names("read(") {
                read_calls += 1;
                // The agent's return pump NEVER reads the reply through a userspace
                // recvfrom/read of leg B (the kTLS-RX leg) — it splices. A reply marker
                // in a recvfrom/read off a splice-related fd would be a copy.
                if carries_reply && syscall_fd(body).is_some_and(|fd| splice_dst_fds.contains(&fd))
                {
                    return_reply_copied_through_io = true;
                }
            }
        }

        Self {
            splice_calls,
            forward_request_copied_through_write,
            return_reply_copied_through_io,
            write_calls,
            read_calls,
        }
    }

    fn summary(&self) -> String {
        format!(
            "splice={} write={} read={} forward_copy_seen={} return_copy_seen={}",
            self.splice_calls,
            self.write_calls,
            self.read_calls,
            self.forward_request_copied_through_write,
            self.return_reply_copied_through_io,
        )
    }
}

/// Strip strace's leading `<pid> ` prefix (present under `-f`) so the syscall name
/// is at the start of the returned slice. A line with no leading-digit prefix is
/// returned unchanged.
fn strip_strace_pid_prefix(line: &str) -> &str {
    let trimmed = line.trim_start();
    let rest = trimmed.trim_start_matches(|c: char| c.is_ascii_digit());
    if rest.len() < trimmed.len() {
        // there WAS a leading digit run — skip the following whitespace to the syscall
        rest.trim_start()
    } else {
        trimmed
    }
}

/// The first-argument fd of a `syscall(FD, ...)` line (e.g. `sendto(26, ...)` →
/// `Some(26)`). `body` has already had its PID prefix stripped. `None` if the args
/// do not begin with an integer (e.g. a `<... resumed>` fragment that omits the fd).
fn syscall_fd(body: &str) -> Option<i32> {
    let open = body.find('(')?;
    let after = &body[open + 1..];
    let end = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
    after[..end].parse::<i32>().ok()
}

/// The destination fd of a `splice(SRC, NULL, DST, NULL, len, flags)` line — the
/// THIRD positional argument. `body` has its PID prefix stripped. `None` on a
/// `<... resumed>` fragment or a malformed line.
fn splice_destination_fd(body: &str) -> Option<i32> {
    let open = body.find("splice(")? + "splice(".len();
    let args = &body[open..];
    // splice args are comma-separated: SRC, off_in, DST, off_out, len, flags
    let dst = args.split(',').nth(2)?.trim();
    let end = dst.find(|c: char| !c.is_ascii_digit()).unwrap_or(dst.len());
    dst.get(..end)?.parse::<i32>().ok()
}

/// Render `bytes` as the `\xHH\xHH...` hex form strace `-xx` emits, so a marker can
/// be substring-matched against a traced buffer line.
fn to_strace_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 4);
    for b in bytes {
        let _ = write!(s, "\\x{b:02x}");
    }
    s
}

// =====================================================================
// ss -tie ULP oracle — read the REAL kernel socket state for the leg-B socket and
// confirm the kTLS ULP is armed in both directions (rxconf:sw txconf:sw).
// =====================================================================

/// The kTLS-ULP facts read from `ss -tie` for the leg-B socket (identified by the
/// peer port).
struct SsUlp {
    has_ktls_tls_ulp: bool,
    tx_sw: bool,
    rx_sw: bool,
    raw: String,
}

impl SsUlp {
    /// Run `ss -tie` and locate the socket line(s) touching `peer_port` (leg B is the
    /// agent's outbound connection to the peer). The kTLS ULP renders as
    /// `tcp-ulp-tls rxconf:sw txconf:sw ...` in the `-i` (info) block, which `ss`
    /// prints on the continuation line(s) following the socket's address line.
    fn for_peer_port(peer_port: u16) -> Self {
        let out = Command::new("ss").args(["-tie"]).output().expect("run ss -tie");
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        // ss prints one socket per logical record; the address line + its info
        // continuation lines form a record. Collect the records whose address line
        // references the peer port, then scan their info block for the ULP tokens.
        let port_tok = format!(":{peer_port}");
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
                // peer port (the leg-B connection to the peer).
                in_record = line.contains(&port_tok);
            }
            if in_record {
                relevant.push_str(line);
                relevant.push('\n');
            }
        }
        // `ss` renders the ULP info as `tcp-ulp-tls version: 1.3 cipher: aes-gcm-256
        // rxconf: sw txconf: sw` — note the SPACE after each colon. Tolerate both the
        // spaced (`rxconf: sw`) and compact (`rxconf:sw`) forms across `ss` versions.
        let has_ktls_tls_ulp = relevant.contains("tcp-ulp-tls");
        let tx_sw = relevant.contains("txconf: sw") || relevant.contains("txconf:sw");
        let rx_sw = relevant.contains("rxconf: sw") || relevant.contains("rxconf:sw");
        Self { has_ktls_tls_ulp, tx_sw, rx_sw, raw: relevant }
    }
}
