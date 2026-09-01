//! Tier-3 OUTBOUND enforce-substrate agent-light BIDIRECTIONAL SPLICE (step 05-03,
//! re-oracled at the increment-m promotion) — on the Path-A egress nft-TPROXY
//! mechanism.
//!
//! ## What this pins
//!
//! Both OUTBOUND directions are agent-light zero-copy SPLICE pumps
//! (`crates/overdrive-dataplane/src/mtls/splice.rs`):
//!   - **FORWARD** (plaintext workload → ciphertext backend, `legF → legB`) is a
//!     BLOCKING `splice(legF → pipe → legB)` into leg B's kTLS-TX. The kernel
//!     `tls_sw_sendmsg` (`MSG_SPLICE_PAGES`) encrypts each spliced chunk INSIDE the
//!     blocking call; the agent does ZERO crypto and NO userspace copy of the
//!     steady-state payload (`findings-ktls-tx-blocking-splice.md`; the retired
//!     loss class was NON-blocking `MSG_DONTWAIT` delivery into kTLS-TX —
//!     `PumpHandle::spawn_encrypt`). The ONE `write_all` the forward keeps is the
//!     pre-arm `prelude` — in-memory bytes captured BEFORE the kTLS arm have no
//!     source fd and cannot ride `splice`.
//!   - **RETURN** (ciphertext backend → plaintext workload, `legB → legF`) is a
//!     zero-copy `splice(legB → pipe → legF)` out of leg B's kTLS-RX (the kernel
//!     `tls_sw_splice_read` decrypts each record on splice-out —
//!     `PumpHandle::spawn_decrypt`).
//!
//! Structural mirror of the SURVIVING
//! `overdrive-dataplane/tests/integration/mtls_inbound_enforce.rs` (the deliver
//! direction's zero-copy oracle for inbound).
//!
//! ## The two-phase traffic shape (why a SECOND request)
//!
//! Phase-1 (`OUTBOUND_REQUEST`) is sent by the workload during the handshake
//! window, so it is captured as the PRE-ARM prelude and legitimately rides the
//! prelude `write_all` — its mechanism is timing-dependent (prelude vs steady
//! state) and is deliberately NOT asserted. Phase-2 (`OUTBOUND_REQUEST2`) is sent
//! only AFTER the workload has read the phase-1 response — the response rode the
//! established pumps, so by then the steady state is provably live and REQ2 MUST
//! ride the forward SPLICE. The mechanism oracle keys on the REQ2 marker only.
//!
//! ## How this is OBSERVABLE (syscall side effects only — testing.md Tier-3 rules)
//!
//! The pump mechanism is observable via `strace` on the agent's own pump threads.
//! The test process runs the production accept loop in-process, so the pump
//! threads (`PumpHandle::spawn_encrypt`/`spawn_decrypt` → `std::thread::spawn`)
//! are CLONE_THREAD threads of THIS process — they share the test's thread group
//! (tgid), and their TID is recovered race-free from the `clone`/`clone3` lines in
//! the strace log (see "Thread-group isolation" below). The netns workload client,
//! by contrast, is a SEPARATE process (`ip netns exec … python3`, a distinct tgid,
//! a `clone` WITHOUT CLONE_THREAD). Rust `TcpStream` `read`/`write_all` lower to
//! `recvfrom`/`sendto` (or `read`/`write`); the pumps issue `splice(2)`. So:
//!   - the FORWARD SPLICE surfaces as the ABSENCE of the steady-state (REQ2)
//!     plaintext from every `write(2)`/`sendto(2)` buffer issued BY A THREAD OF THE
//!     TEST PROCESS (the agent never copies the steady-state payload through
//!     userspace), while the round-trip proves REQ2 REACHED the backend decrypted
//!     byte-exact — the only agent path into leg B besides a userspace write is the
//!     splice, and splices are present; and
//!   - the BIDIRECTIONAL SPLICE topology surfaces as ≥1 traced fd that is BOTH a
//!     `splice` SOURCE and a `splice` DESTINATION — the leg fds: leg B is the
//!     return pump's source AND the forward pump's destination (one TX+RX kTLS fd),
//!     leg F the inverse.
//! These are REAL captured syscalls, never the adapter's own bookkeeping.
//!
//! ### Thread-group isolation — the zero-copy oracle MUST attribute to the agent (RACE-FREE)
//!
//! `strace -f` follows the netns client's forked `python3` descendant, whose own
//! `s.sendall(OUTBOUND_REQUEST2)` lowers to a `sendto(<plaintext incl. marker>)` —
//! so the REQ2 marker legitimately appears in the trace on the workload client's
//! send. The zero-copy oracle ("no AGENT write carries the marker") is therefore
//! meaningful only under attribution: a marker-carrying write counts against the
//! agent ONLY when its TID belongs to this process's thread group; the netns
//! `python3`'s TID is a separate-process fork and is the EXCLUDED population. The
//! client's captured send doubles as the capture-works control — the marker IS
//! traceable in a write buffer, so the agent's zero count is a genuine zero, not a
//! capture failure (see the FALSIFICATION block in the test body).
//!
//! The test's thread group is derived RACE-FREE (NOT by live polling — a 15 ms
//! `/proc/self/task` poll races a sub-15 ms pump thread and misses it ~29% of runs).
//! Two combined sources: (1) a SINGLE `/proc/self/task` snapshot taken at strace-attach
//! time — race-free for every PRE-EXISTING thread (the tokio runtime + accept-loop
//! threads, all alive at that instant); (2) the transitive `CLONE_THREAD` closure
//! parsed from the strace log itself — every thread created AFTER attach emits a
//! `clone`/`clone3({flags=...CLONE_THREAD...}) = <child_tid>` line whose parent TID is
//! already in the set, so the closure reaches the short-lived pump threads regardless
//! of how briefly they lived (their clone lines are PERMANENTLY in the log). A process
//! fork (the netns client) is a `clone` WITHOUT `CLONE_THREAD` → never added →
//! deterministically excluded.
//!
//! ## Driven through the PRODUCTION composition root (port-to-port / TBU defense)
//!
//! The connection is driven END-TO-END through the SHIPPING production seams —
//! `MtlsInterceptWorker::start_alloc` → the spawned outbound `accept_loop`
//! (getsockname → resolve(Mesh) → the real `HostMtlsEnforcement::enforce`) — NOT a
//! hand-rolled replica. The ONLY injected double is the `resolve` port (a
//! `ScriptedResolve`; the production resolve index 01-03 is its own DST's job). The
//! enforce substrate is the REAL `HostMtlsEnforcement` (ADR-0069, UNCHANGED). If the
//! production wiring that drives the outbound enforce substrate were removed, this
//! test goes RED: the netns workload's round-trip would not complete and the
//! `splice` evidence would vanish.
//!
//! ## Authn-only boundary (Q4 / #242)
//!
//! `expected_peer` stays `None` for the enforced connection (v1 authn-only; the
//! intended-peer pinning is #242). This AT asserts encryption + the substrate
//! mechanism — it MUST NOT assert intended-peer "protection". Identical authn-only
//! discipline to mtls_inbound_enforce.rs and 05-01's last criterion.
//!
//! Requires root + CAP_NET_ADMIN/CAP_SYS_ADMIN (IP_TRANSPARENT, nft, ip netns, ip
//! rule) AND `strace` (the syscall oracle is load-bearing — present in the canonical
//! Lima VM). A non-root run SKIPs. Run via `cargo xtask lima run -- cargo nextest
//! run -p overdrive-worker --features integration-tests -E
//! 'test(outbound_enforce_substrate_bidirectional_splice_zero_copy)'`. NEVER
//! `--no-run` (a compile-only gate is green even when every fixture refuses at
//! boot). `uname -r` is recorded (spike.md: the verdict is pinned to a kernel).
//!
//! Hygiene: the shared `overdrive-mtls` routing infra PERSISTS by design
//! (node-global converge-on-boot), so the test scrubs ALL `overdrive-mtls` nft state
//! + the fwmark rule/route + the test netns/veth/lo-addr at START (tolerate
//! pre-existing) AND END. A cross-PROCESS `flock(2)` lock (`KernelStateLock`, on the
//! SAME path the sibling kernel-touching suites use) serialises the kernel-touching
//! tests — nextest runs each `#[test]` in a separate process, so an in-process lock
//! cannot serialise node-global state.

#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::unnecessary_cast,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::match_wildcard_for_single_variants,
    clippy::missing_panics_doc,
    clippy::missing_const_for_fn,
    clippy::format_collect,
    reason = "Tier-3 outbound-substrate test body; the bidirectional-splice narrative in the module docstring is prose; skip messages + strace diagnostics go to stderr; failures must panic with informative messages; the libc FFI casts are width conversions on compile-time constants (ETH_P_ALL.to_be() as i32 mirrors traffic.rs); leg F/B are the ADR-0069 contract vocabulary; the single composed Tier-3 scenario drives the round-trip under one strace attach; the SocketAddr wildcard arm is the V6 case a v4-only fixture cannot hit; the per-byte \\xNN python-literal fold reads clearer than a write! accumulator in a test fixture; const-fn-ability on test constructors is not load-bearing"
)]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddrV4, TcpListener};
use std::os::fd::AsRawFd as _;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
use overdrive_core::traits::driver::{AllocationSpec, Resources};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{AllocationId, CertSerial};
use overdrive_dataplane::mtls::HostMtlsEnforcement;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_worker::mtls_intercept_port::HostMtlsIntercept;
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;

use async_trait::async_trait;
use overdrive_core::traits::mtls_resolve::{
    MtlsResolution, MtlsResolve, MtlsResolveError, ResolvedBackend,
};
use rcgen::string::Ia5String;
use rcgen::{CertificateParams, Issuer, KeyPair, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

// ============================================================================
// topology constants (mirror the increment-b egress spike + the 05-01/05-02 harness)
// ============================================================================

const NS_W: &str = "nsW-asym0503";
const VETH_W: &str = "vethW-asym05";
const VETH_H: &str = "vethH-asym05";
const HOST_GW: &str = "10.99.0.1";
const WL_ADDR: &str = "10.99.0.2";
const SUBNET_LEN: &str = "24";

/// The mesh backend the OUTBOUND workload dials — a host-side lo-bound address it
/// routes to via the gateway, so its egress genuinely INGRESSES vethH and hits
/// PREROUTING. This is the dialed `orig_dst` the resolve consumer classifies
/// `Mesh`, and the address the real mesh mTLS server (leg-B's peer) binds.
const MESH_BACKEND_IP: &str = "10.200.0.1";
const MESH_BACKEND_PORT: u16 = 18831;

/// `lo` — where leg-B's TLS records (agent → the lo-bound mesh backend) physically
/// carry their bytes, so the AF_PACKET 0x17 confidentiality oracle captures there.
const LOOPBACK_IFACE: &str = "lo";

/// The PHASE-1 OUTBOUND application request the workload sends through leg-F →
/// (mTLS leg-B) → the mesh server. Sent during the handshake window, it is captured
/// as the PRE-ARM prelude and legitimately rides the prelude `write_all` (or, if it
/// arrived late, the forward splice) — its mechanism is timing-dependent and NOT
/// asserted. Its job is the phase-1 round-trip that proves the steady state is live
/// before phase 2.
const OUTBOUND_REQUEST: &[u8] =
    b"OVERDRIVE_0503_OUTBOUND_REQUEST_phase1_prearm_workload_to_mesh_legF_to_legB_prelude_ok";
/// The OUTBOUND application response the mesh server replies to phase 1; it rides
/// back over leg-B's kTLS-RX via the RETURN `splice(legB -> legF)` pump (zero-copy,
/// decrypted on splice-out) to the workload byte-exact. The workload sends phase 2
/// only after reading this — the steady-state gate.
const OUTBOUND_RESPONSE: &[u8] =
    b"OVERDRIVE_0503_OUTBOUND_RESPONSE_return_splice_mesh_reply_rides_back_over_legB_ktls_rx";
/// The PHASE-2 STEADY-STATE request. Sent only AFTER the workload read the phase-1
/// response (⇒ establish completed, the prelude was consumed, the pumps run), so it
/// MUST ride the forward BLOCKING `splice(legF → pipe → legB)` — zero userspace
/// copy. Its distinctive interior bytes are the ZERO-COPY marker: it must NEVER
/// appear in a `write`/`sendto` buffer issued by a thread of the TEST process (the
/// agent), while the mesh peer must still receive it decrypted byte-exact. NOTE:
/// the netns workload client sends this plaintext itself (`s.sendall`), so the
/// marker legitimately appears on the client's `sendto` — the thread-group filter
/// (see `TraceFindings::parse`) excludes it and doubles as the capture-works
/// control.
const OUTBOUND_REQUEST2: &[u8] =
    b"OVERDRIVE_0503_OUTBOUND_REQUEST2_phase2_steady_state_marker_rides_forward_splice_no_copy";

// ============================================================================
// Cross-process kernel-state exclusion (shared path with the sibling suites)
// ============================================================================

/// Cross-PROCESS exclusion for the shared host-netns kernel state. The
/// `overdrive-mtls` nft table, the fwmark ip-rule, and the table-100 local route
/// are NODE-GLOBAL. nextest runs each `#[test]` in a SEPARATE PROCESS, so an
/// in-process lock cannot serialise them — an `flock(2)` on the fixed path (shared
/// with `egress_tproxy_capture.rs` / `bidirectional_walking_skeleton.rs`) spans
/// processes.
struct KernelStateLock {
    fd: std::os::fd::OwnedFd,
}

impl KernelStateLock {
    fn acquire() -> Self {
        use std::os::fd::FromRawFd as _;
        let path = c"/tmp/overdrive-mtls-kernel-state.lock";
        // SAFETY: open with O_CREAT|O_RDWR on a fixed path; the returned fd is
        // adopted by OwnedFd. flock blocks until the exclusive lock is held.
        let fd = unsafe {
            let raw = libc::open(path.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600);
            assert!(raw >= 0, "open kernel-state lock file: {}", std::io::Error::last_os_error());
            let rc = libc::flock(raw, libc::LOCK_EX);
            assert!(rc == 0, "flock LOCK_EX: {}", std::io::Error::last_os_error());
            std::os::fd::OwnedFd::from_raw_fd(raw)
        };
        Self { fd }
    }
}

impl Drop for KernelStateLock {
    fn drop(&mut self) {
        // SAFETY: fd is the live lock fd; LOCK_UN releases the advisory lock.
        unsafe {
            libc::flock(self.fd.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

/// True iff this process is uid 0 (root). IP_TRANSPARENT, nft, `ip netns`, and
/// `ip rule` all need root + CAP_NET_ADMIN/CAP_SYS_ADMIN; a non-root run cannot
/// stand up the fixture, so we SKIP rather than fail.
fn is_root() -> bool {
    // SAFETY: getuid is always safe; takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

/// PANIC-SAFE teardown (F3). Owns the node-global kernel-state scrub + the production
/// worker stop, run from `Drop` so a panicking assertion CANNOT leak the
/// `overdrive-mtls` nft table, the test netns/veth/lo-addr, or the fwmark rule/route,
/// and cannot hang 120 s on the leaked production `accept_loop` (`stop_alloc` removes
/// the egress rule and stops the loop). Mirrors the `AllocCleanup` RAII discipline in
/// `.claude/rules/testing.md` § "Leaked workload cgroups". Declared AFTER
/// `KernelStateLock` at the call site so it drops FIRST (Rust drops in reverse
/// declaration order) — the scrub runs while the cross-process lock is still held.
struct TopologyGuard {
    /// The production worker whose alloc must be stopped (egress rule removed,
    /// accept_loop halted). `None` once stopped, so a manual end-of-test stop and the
    /// Drop-path stop do not double-fire.
    worker: Option<Arc<MtlsInterceptWorker>>,
    client_alloc: AllocationId,
}

impl TopologyGuard {
    fn new(worker: Arc<MtlsInterceptWorker>, client_alloc: AllocationId) -> Self {
        Self { worker: Some(worker), client_alloc }
    }

    /// Complete the production worker teardown before removing shared kernel
    /// infrastructure. A teardown failure is authoritative on the clean path:
    /// the caller receives it only after the bounded worker-owned task tree has
    /// finished, and the topology scrub still runs before returning.
    async fn finish(mut self) -> Result<(), String> {
        let result = if let Some(worker) = self.worker.take() {
            let result = worker.stop_alloc(&self.client_alloc).await;
            if result.is_ok() {
                assert!(
                    worker.alloc_stop_converged_for_test(&self.client_alloc),
                    "awaited stop owns the joined task/connection complement"
                );
                let dump = nft_dump_table();
                assert!(
                    !dump.contains(&format!("iifname \"{VETH_H}\"")),
                    "awaited stop removes the allocation rule before shared teardown: {dump}"
                );
            }
            result.map_err(|error| error.to_string())
        } else {
            Ok(())
        };
        teardown_topology();
        clean_shared_infra();
        result
    }
}

impl Drop for TopologyGuard {
    fn drop(&mut self) {
        // Emergency panic fallback only. Never detach async cleanup from Drop:
        // the normal path calls `finish().await`, while a panic synchronously
        // scrubs shared topology and lets ordinary Arc/task-owner destruction
        // close the worker. Spawning here made runtime cancellation and nft
        // deletion race an unowned teardown task.
        self.worker.take();
        teardown_topology();
        clean_shared_infra();
    }
}

// ============================================================================
// command shims (mirror egress_tproxy_capture.rs / bidirectional_walking_skeleton.rs)
// ============================================================================

fn ip(args: &[&str]) {
    let out = Command::new("ip")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn ip");
    assert!(
        out.status.success(),
        "ip {args:?} exited {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
}

fn ip_quiet(args: &[&str]) {
    let _ = Command::new("ip").args(args).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

fn sysctl_w(kv: &str) {
    let _ = Command::new("sysctl")
        .args(["-w", kv])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn nft_dump_table() -> String {
    Command::new("nft")
        .args(["list", "table", "ip", "overdrive-mtls"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Scrub ALL `overdrive-mtls` nft state + the shared fwmark rule/route so a
/// clean-kernel ground-truth run is reproducible. Run at test START (tolerate
/// pre-existing) AND END. Best-effort: every failure is "nothing to clean".
fn clean_shared_infra() {
    for _ in 0..64 {
        let ok = Command::new("ip")
            .args(["rule", "del", "fwmark", "0x1", "lookup", "100"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            break;
        }
    }
    ip_quiet(&["route", "del", "local", "0.0.0.0/0", "dev", "lo", "table", "100"]);
    let _ = Command::new("nft")
        .args(["delete", "table", "ip", "overdrive-mtls"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Tear down the per-test netns + veth pair + the lo-bound mesh backend addr. The
/// shared `overdrive-mtls` infra is handled by `clean_shared_infra`.
fn teardown_topology() {
    ip_quiet(&["link", "del", VETH_H]);
    ip_quiet(&["netns", "del", NS_W]);
    ip_quiet(&["addr", "del", &format!("{MESH_BACKEND_IP}/32"), "dev", "lo"]);
}

/// Stand up the netns + veth pair + addresses + host routing hygiene EXACTLY as the
/// increment-b egress spike does, plus the lo-bound mesh backend the OUTBOUND dial
/// targets.
fn setup_topology() {
    teardown_topology();

    ip(&["netns", "add", NS_W]);
    ip(&["link", "add", VETH_W, "type", "veth", "peer", "name", VETH_H]);
    ip(&["link", "set", VETH_W, "netns", NS_W]);

    // Host side: address + up.
    ip(&["addr", "add", &format!("{HOST_GW}/{SUBNET_LEN}"), "dev", VETH_H]);
    ip(&["link", "set", VETH_H, "up"]);

    // Workload side (inside netns): lo up + address + up + default route.
    ip(&["netns", "exec", NS_W, "ip", "link", "set", "lo", "up"]);
    ip(&[
        "netns",
        "exec",
        NS_W,
        "ip",
        "addr",
        "add",
        &format!("{WL_ADDR}/{SUBNET_LEN}"),
        "dev",
        VETH_W,
    ]);
    ip(&["netns", "exec", NS_W, "ip", "link", "set", VETH_W, "up"]);
    ip(&["netns", "exec", NS_W, "ip", "route", "add", "default", "via", HOST_GW]);

    // The OUTBOUND dial target lives on host lo (the host binds+listens on it; the
    // workload routes to it via the gateway).
    ip(&["addr", "add", &format!("{MESH_BACKEND_IP}/32"), "dev", "lo"]);

    // Host-side routing hygiene (NOT a TPROXY concession; spike § Edge cases):
    // forwarding + rp_filter relaxation so the asymmetric ingress is not dropped.
    sysctl_w("net.ipv4.ip_forward=1");
    sysctl_w(&format!("net.ipv4.conf.{VETH_H}.rp_filter=0"));
    sysctl_w("net.ipv4.conf.all.rp_filter=0");
    sysctl_w("net.ipv4.conf.lo.rp_filter=0");

    // bpf.md Rule 2 / spike: disable TX-checksum-offload on the host veth.
    let _ = Command::new("ethtool")
        .args(["-K", VETH_H, "tx", "off"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

// ============================================================================
// Fresh focused PKI (re-authored — replicates the dataplane `mtls_pki.rs` reference
// + the 05-01 walking-skeleton: a real root → intermediate → leaf chain)
// ============================================================================

/// A minted leaf — the PEM cert + key + the SPIFFE SAN, plus the DER forms.
struct Leaf {
    cert_pem: String,
    key_pem: String,
    cert_der: CertificateDer<'static>,
    key_der: PrivateKeyDer<'static>,
    spiffe: overdrive_core::SpiffeId,
    serial: CertSerial,
}

/// The shared test PKI: root self-signs; intermediate signed by root; every leaf
/// signed by the intermediate (production issuance shape).
struct TestPki {
    ca_cert_pem: String,
    intermediate_cert_pem: String,
    intermediate_cert_der: CertificateDer<'static>,
    /// The OUTBOUND client SVID (workload-as-client; the agent presents on leg-B).
    client_leaf: Leaf,
    /// The OUTBOUND real mesh peer leaf: a SERVER cert with a DNS SAN matching the
    /// fixed leg-B SNI (`peer.overdrive.local`, per mtls/outbound.rs) so the agent's
    /// leg-B client handshake verifies the mesh server's cert.
    peer_leaf: Leaf,
    client_alloc: AllocationId,
}

impl TestPki {
    /// The DNS SAN the OUTBOUND mesh peer presents (matches the FIXED SNI the
    /// adapter's leg-B client handshake uses in `mtls::outbound::client_handshake` —
    /// `peer.overdrive.local`).
    const PEER_SNI: &'static str = "peer.overdrive.local";

    fn mint() -> Self {
        let root = MintedCa::mint_root("overdrive-mtls-05-03-ROOT-CA");
        let intermediate = root.mint_intermediate("overdrive-mtls-05-03-INTERMEDIATE-CA");

        let client_spiffe = "spiffe://overdrive.local/ns/default/sa/client";
        let client_leaf = intermediate.mint_leaf(client_spiffe, None, true);
        let peer_leaf = intermediate.mint_leaf(
            "spiffe://overdrive.local/ns/default/sa/peer",
            Some(Self::PEER_SNI),
            false,
        );

        Self {
            ca_cert_pem: root.cert_pem,
            intermediate_cert_pem: intermediate.cert_pem.clone(),
            intermediate_cert_der: CertificateDer::from(intermediate.cert_der),
            client_leaf,
            peer_leaf,
            client_alloc: AllocationId::new("alloc-asym-client").expect("valid alloc"),
        }
    }

    fn ca_cert_pem(&self) -> &str {
        &self.ca_cert_pem
    }

    fn intermediate_cert_der(&self) -> CertificateDer<'static> {
        self.intermediate_cert_der.clone()
    }

    /// The shared trust bundle: root anchor = the ROOT; intermediate chain material
    /// = the INTERMEDIATE (the agent reads this via `IdentityRead`).
    fn trust_bundle(&self) -> TrustBundle {
        TrustBundle::new(
            CaCertPem::new(self.ca_cert_pem.clone()),
            Some(CaCertPem::new(self.intermediate_cert_pem.clone())),
        )
    }

    fn client_svid_material(&self) -> SvidMaterial {
        svid_from_leaf(&self.client_leaf)
    }
}

/// A minted signing authority (root OR intermediate) retaining its
/// `CertificateParams` + `KeyPair` so it can build a reusable rcgen 0.14 `Issuer`.
struct MintedCa {
    params: CertificateParams,
    key: KeyPair,
    cert_pem: String,
    cert_der: Vec<u8>,
}

impl MintedCa {
    fn mint_root(cn: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name.push(rcgen::DnType::CommonName, cn);
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let cert = params.self_signed(&key).unwrap();
        let cert_pem = cert.pem();
        let cert_der = cert.der().to_vec();
        Self { params, key, cert_pem, cert_der }
    }

    fn mint_intermediate(&self, cn: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
        params.distinguished_name.push(rcgen::DnType::CommonName, cn);
        params.use_authority_key_identifier_extension = true;
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let root_issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&key, &root_issuer).unwrap();
        let cert_pem = cert.pem();
        let cert_der = cert.der().to_vec();
        Self { params, key, cert_pem, cert_der }
    }

    fn mint_leaf(&self, spiffe: &str, dns_san: Option<&str>, client_auth: bool) -> Leaf {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        let uri = Ia5String::try_from(spiffe).expect("spiffe URI is a valid IA5 string");
        let mut sans = vec![SanType::URI(uri)];
        if let Some(dns) = dns_san {
            let dns_ia5 = Ia5String::try_from(dns).expect("dns SAN is a valid IA5 string");
            sans.push(SanType::DnsName(dns_ia5));
        }
        params.subject_alt_names = sans;
        params.distinguished_name.push(rcgen::DnType::CommonName, spiffe);
        params.use_authority_key_identifier_extension = true;
        params.extended_key_usages = if client_auth {
            vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth]
        } else {
            vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth]
        };
        let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&leaf_key, &issuer).unwrap();
        let cert_pem = cert.pem();
        let key_pem = leaf_key.serialize_pem();
        let cert_der = CertificateDer::from(cert.der().to_vec());
        let key_der = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        Leaf {
            cert_pem,
            key_pem,
            cert_der,
            key_der,
            spiffe: spiffe.parse().expect("valid spiffe id"),
            serial: CertSerial::new("0a0b0c0d").expect("valid serial"),
        }
    }
}

/// Assemble `SvidMaterial` from a minted leaf (cert PEM/DER + leaf key PEM +
/// far-future `not_after`).
fn svid_from_leaf(leaf: &Leaf) -> SvidMaterial {
    let not_after = UnixInstant::from_unix_duration(Duration::from_secs(4_102_444_800)); // 2100
    SvidMaterial::new(
        CaCertPem::new(leaf.cert_pem.clone()),
        CaCertDer::new(leaf.cert_der.as_ref().to_vec()),
        leaf.serial.clone(),
        leaf.spiffe.clone(),
        CaKeyPem::new(leaf.key_pem.clone()),
        not_after,
    )
}

/// The agent's held-identity store — the ONLY holder of SVID material (workloads
/// hold nothing; the agent reads through THIS `IdentityRead` port and NEVER mints,
/// #26 is a reader). `None` is explicit absence.
struct HeldIdentities {
    svids: BTreeMap<AllocationId, SvidMaterial>,
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

fn held_identities(pki: &TestPki) -> HeldIdentities {
    let mut svids = BTreeMap::new();
    svids.insert(pki.client_alloc.clone(), pki.client_svid_material());
    HeldIdentities { svids, bundle: pki.trust_bundle() }
}

/// The `AllocationSpec` the OUTBOUND production `start_alloc` consumes: keyed on the
/// CLIENT alloc id (so production's `enforce` selects the held client SVID for the
/// leg-B handshake) with `host_veth = Some(VETH_H)` (the channel the action-shim C3
/// provision seam sets in production — drives the egress nft-TPROXY install matching
/// `iifname VETH_H`).
fn build_client_spec(pki: &TestPki, host_veth: Option<String>) -> AllocationSpec {
    AllocationSpec {
        alloc: pki.client_alloc.clone(),
        identity: pki.client_leaf.spiffe.clone(),
        driver: overdrive_core::traits::driver::DriverPayload::Exec(
            overdrive_core::traits::driver::ExecPayload {
                command: "/bin/true".to_owned(),
                args: vec![],
            },
        ),
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        netns: None,
        host_veth,
        service_ports: Vec::new(),
        workload_addr: None,
        guest_tap: None,
        guest_mac: None,
        guest_gateway: None,
        guest_prefix_len: None,
        guest_dns: None,
    }
}

// ============================================================================
// Scriptable resolve double (replicates SimMtlsResolve's role — maps a fixed
// orig_dst → MtlsResolution arm so the OUTBOUND accept loop drives the Mesh arm)
// ============================================================================

/// A scripted [`MtlsResolve`]: each `orig_dst` maps to a pre-programmed
/// [`MtlsResolution`] arm. `Mesh` carries the RESOLVED backend addr (the agent's
/// leg-B dial target — the real mesh mTLS server). `expected_svid` is `None` (v1
/// authn-only). An unscripted addr resolves `NonMesh` (the conservative pass-through
/// default).
struct ScriptedResolve {
    table: BTreeMap<SocketAddrV4, MtlsResolution>,
}

impl ScriptedResolve {
    fn new(table: BTreeMap<SocketAddrV4, MtlsResolution>) -> Self {
        Self { table }
    }
}

#[async_trait]
impl MtlsResolve for ScriptedResolve {
    async fn probe(&self) -> Result<(), MtlsResolveError> {
        Ok(())
    }

    async fn resolve(&self, orig_dst: SocketAddrV4) -> Result<MtlsResolution, MtlsResolveError> {
        Ok(self.table.get(&orig_dst).cloned().unwrap_or(MtlsResolution::NonMesh))
    }
}

// ============================================================================
// 0x17 wire scan (re-authored — replicates the dataplane `traffic.rs` technique:
// AF_PACKET capture on `lo`, walk TLS record framing, count 0x17 app-data records
// per direction, scan for cleartext markers)
// ============================================================================

const TLS_CONTENT_TYPE_APPLICATION_DATA: u8 = 0x17;
const TLS_LEGACY_RECORD_VERSION_TLS12: [u8; 2] = [0x03, 0x03];
const TLS_LEGACY_RECORD_VERSION_TLS10: [u8; 2] = [0x03, 0x01];
const TLS_RECORD_HEADER_LEN: usize = 5;
const ETH_HDR_LEN: usize = 14;
const IPV4_HDR_LEN: usize = 20;
const ETH_P_ALL: std::os::raw::c_int = 0x0003;

fn is_tls_record_version(version: [u8; 2]) -> bool {
    version == TLS_LEGACY_RECORD_VERSION_TLS12 || version == TLS_LEGACY_RECORD_VERSION_TLS10
}

/// The result of scanning a captured wire on `wire_port`: how many genuine `0x17`
/// application_data records crossed in each direction, and how many times EITHER
/// cleartext marker appeared (MUST be 0 on the encrypted leg-B wire).
#[derive(Debug, Clone, Copy, Default)]
struct WireScan {
    records_to_wire_port: u64,
    records_from_wire_port: u64,
    plaintext_marker_hits: u64,
}

impl WireScan {
    /// 0x17 records present in EITHER direction.
    fn has_app_data(&self) -> bool {
        self.records_to_wire_port > 0 || self.records_from_wire_port > 0
    }
}

/// A live AF_PACKET/SOCK_RAW capture on `iface` that records every frame into a
/// buffer on a background thread until `stop_and_scan`. Filtered (at scan time) to
/// TCP frames touching `wire_port` (as src OR dst).
struct WireCapture {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Vec<Vec<u8>>>>,
    wire_port: u16,
}

impl WireCapture {
    fn start(iface: &str, wire_port: u16) -> Self {
        let ifindex = if_nametoindex(iface).expect("wire-capture: if_nametoindex");
        // SAFETY: AF_PACKET / SOCK_RAW socket on the bound iface.
        let fd = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW, ETH_P_ALL.to_be() as i32) };
        assert!(fd >= 0, "wire-capture: socket: {}", std::io::Error::last_os_error());

        let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        sll.sll_family = libc::AF_PACKET as u16;
        sll.sll_protocol = (ETH_P_ALL as u16).to_be();
        sll.sll_ifindex = ifindex as i32;
        // SAFETY: bind an AF_PACKET socket to the resolved ifindex.
        let rc = unsafe {
            libc::bind(
                fd,
                std::ptr::from_ref(&sll).cast(),
                std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            )
        };
        assert!(rc == 0, "wire-capture: bind {iface}: {}", std::io::Error::last_os_error());
        // SAFETY: fcntl on our own fd; non-blocking so the loop can poll `stop`.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL, 0);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let handle = std::thread::spawn(move || -> Vec<Vec<u8>> {
            let mut frames: Vec<Vec<u8>> = Vec::new();
            let mut buf = vec![0u8; 65536];
            while !stop_thread.load(Ordering::SeqCst) {
                // SAFETY: recv into our owned buffer on the bound AF_PACKET fd.
                let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
                if n > 0 {
                    frames.push(buf[..n as usize].to_vec());
                } else {
                    std::thread::sleep(Duration::from_micros(200));
                }
            }
            // Final drain so records written right before `stop` are not lost.
            loop {
                // SAFETY: same bounded recv on our fd.
                let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
                if n > 0 {
                    frames.push(buf[..n as usize].to_vec());
                } else {
                    break;
                }
            }
            // SAFETY: fd created above; close on capture-thread exit.
            unsafe { libc::close(fd) };
            frames
        });
        Self { stop, handle: Some(handle), wire_port }
    }

    fn stop_and_scan(mut self, cleartext_markers: &[&[u8]]) -> WireScan {
        self.stop.store(true, Ordering::SeqCst);
        let frames = self.handle.take().expect("wire-capture handle").join().expect("capture join");
        scan_frames(&frames, self.wire_port, cleartext_markers)
    }
}

fn scan_frames(frames: &[Vec<u8>], wire_port: u16, cleartext_markers: &[&[u8]]) -> WireScan {
    let mut streams: BTreeMap<(u16, u16), Vec<u8>> = BTreeMap::new();
    for frame in frames {
        let Some((src_port, dst_port, payload)) = parse_tcp_payload(frame) else {
            continue;
        };
        if src_port != wire_port && dst_port != wire_port {
            continue;
        }
        if payload.is_empty() {
            continue;
        }
        streams.entry((src_port, dst_port)).or_default().extend_from_slice(payload);
    }
    let mut records_to_wire_port: u64 = 0;
    let mut records_from_wire_port: u64 = 0;
    let mut plaintext_marker_hits: u64 = 0;
    for (&(src_port, dst_port), stream) in &streams {
        let records = count_tls_app_data_records(stream);
        if dst_port == wire_port {
            records_to_wire_port += records;
        } else if src_port == wire_port {
            records_from_wire_port += records;
        }
        // The leg-B wire (toward/from MESH_BACKEND_PORT) is ENCRYPTED end-to-end, so
        // a cleartext request/response marker on it WOULD be a breach. The DIRECTIONAL
        // 0x17 counts are the load-bearing confidentiality oracle; the marker counter
        // is the belt-and-braces "no plaintext leaked onto the encrypted wire" check.
        for marker in cleartext_markers {
            plaintext_marker_hits += count_subslices(stream, marker);
        }
    }
    WireScan { records_to_wire_port, records_from_wire_port, plaintext_marker_hits }
}

fn parse_tcp_payload(frame: &[u8]) -> Option<(u16, u16, &[u8])> {
    if frame.len() < ETH_HDR_LEN + IPV4_HDR_LEN {
        return None;
    }
    if frame.get(12).copied()? != 0x08 || frame.get(13).copied()? != 0x00 {
        return None;
    }
    let ip = ETH_HDR_LEN;
    let vihl = frame.get(ip).copied()?;
    if vihl >> 4 != 4 {
        return None;
    }
    let ihl = ((vihl & 0x0f) as usize) * 4;
    if ihl < IPV4_HDR_LEN {
        return None;
    }
    if frame.get(ip + 9).copied()? != 0x06 {
        return None; // not TCP
    }
    let tcp = ip + ihl;
    if frame.len() < tcp + 20 {
        return None;
    }
    let src_port = u16::from_be_bytes([frame.get(tcp).copied()?, frame.get(tcp + 1).copied()?]);
    let dst_port = u16::from_be_bytes([frame.get(tcp + 2).copied()?, frame.get(tcp + 3).copied()?]);
    let data_off = ((frame.get(tcp + 12).copied()? >> 4) as usize) * 4;
    if data_off < 20 {
        return None;
    }
    let payload_start = tcp + data_off;
    if payload_start > frame.len() {
        return None;
    }
    Some((src_port, dst_port, &frame[payload_start..]))
}

fn count_tls_app_data_records(stream: &[u8]) -> u64 {
    let mut count: u64 = 0;
    let mut i = 0usize;
    while i + TLS_RECORD_HEADER_LEN <= stream.len() {
        let content_type = stream[i];
        let version = [stream[i + 1], stream[i + 2]];
        let length = u16::from_be_bytes([stream[i + 3], stream[i + 4]]) as usize;
        if !is_tls_record_version(version) {
            break;
        }
        if content_type == TLS_CONTENT_TYPE_APPLICATION_DATA {
            count += 1;
        }
        let next = i + TLS_RECORD_HEADER_LEN + length;
        if next <= i {
            break;
        }
        i = next;
    }
    count
}

fn count_subslices(haystack: &[u8], needle: &[u8]) -> u64 {
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }
    let mut count: u64 = 0;
    let mut i = 0usize;
    while i + needle.len() <= haystack.len() {
        if &haystack[i..i + needle.len()] == needle {
            count += 1;
            i += needle.len();
        } else {
            i += 1;
        }
    }
    count
}

fn if_nametoindex(iface: &str) -> std::io::Result<u32> {
    let cstr = std::ffi::CString::new(iface).expect("iface name has no NUL");
    // SAFETY: thin syscall wrapper; pointer not retained past call.
    let idx = unsafe { libc::if_nametoindex(cstr.as_ptr()) };
    if idx == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(idx)
}

// ============================================================================
// real mTLS mesh peer — the agent's leg-B dial target (re-authored fresh from the
// 05-01 walking skeleton's spawn_mesh_peer)
// ============================================================================

/// Spawn the OUTBOUND mesh peer: a real rustls TLS-1.3 SERVER on
/// `MESH_BACKEND_IP:MESH_BACKEND_PORT` (host lo) presenting the PEER SVID and
/// REQUIRE+VERIFYing the client SVID chains to the bundle. This is the real backend
/// the agent's leg-B client handshake reaches. Reads `OUTBOUND_REQUEST` byte-exact
/// (decrypted), replies `OUTBOUND_RESPONSE`. Returns a join handle whose `bool`
/// reports the byte-exact request receipt.
fn spawn_mesh_peer(pki: &TestPki) -> std::thread::JoinHandle<bool> {
    let bind = SocketAddrV4::new(MESH_BACKEND_IP.parse().expect("mesh ip"), MESH_BACKEND_PORT);
    let peer_cert = pki.peer_leaf.cert_der.clone();
    let intermediate = pki.intermediate_cert_der();
    let peer_key = pki.peer_leaf.key_der.clone_key();
    let ca_pem = pki.ca_cert_pem().to_string();
    std::thread::spawn(move || mesh_peer_run(bind, peer_cert, intermediate, peer_key, &ca_pem))
}

fn mesh_peer_run(
    bind: SocketAddrV4,
    cert: CertificateDer<'static>,
    intermediate: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    ca_pem: &str,
) -> bool {
    use rustls::server::WebPkiClientVerifier;
    let roots = Arc::new(ca_root_store(ca_pem));
    let verifier = match WebPkiClientVerifier::builder(roots).build() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[05-03] mesh peer client verifier: {e}");
            return false;
        }
    };
    // Present [peer_leaf, intermediate] so the agent's root-anchor-only client
    // verifier can build leaf → intermediate → root.
    let mut cfg = match rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![cert, intermediate], key)
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[05-03] mesh peer server config: {e}");
            return false;
        }
    };
    // Suppress the TLS 1.3 NewSessionTicket: the agent's leg-B is kTLS-RX-armed
    // immediately after the handshake, and a raw kTLS-RX hits EIO on a post-handshake
    // ticket record (mtls/outbound.rs sentinel_peer_recv sets the same
    // `send_tls13_tickets = 0` for exactly this reason). Without this the return
    // splice pump errors on the ticket and the workload sees an EOF with no response.
    cfg.send_tls13_tickets = 0;
    let listener = match TcpListener::bind(bind) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[05-03] mesh peer bind {bind}: {e}");
            return false;
        }
    };
    let (tcp, _peer) = match accept_with_timeout(&listener, Duration::from_secs(12)) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("[05-03] mesh peer accept: {e}");
            return false;
        }
    };
    tcp.set_nodelay(true).ok();
    let mut tcp = tcp;
    tcp.set_read_timeout(Some(Duration::from_secs(8))).ok();
    let mut conn = match rustls::ServerConnection::new(Arc::new(cfg)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[05-03] mesh peer ServerConnection: {e}");
            return false;
        }
    };
    if !drive_server_handshake(&mut conn, &mut tcp) {
        eprintln!("[05-03] mesh peer handshake failed");
        return false;
    }
    // PHASE 1: read the workload's request (decrypted) byte-exact, then reply.
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut buf = vec![0u8; 4096];
    while got.len() < OUTBOUND_REQUEST.len() && Instant::now() < deadline {
        let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
        match tls.read(&mut buf) {
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
    let request_ok = got == OUTBOUND_REQUEST;
    {
        let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
        let _ = tls.write_all(OUTBOUND_RESPONSE).and_then(|()| tls.flush());
    }
    // PHASE 2 (steady state): the workload sends REQUEST2 only after reading the
    // phase-1 response, so it necessarily rides the FORWARD splice. Read it to EOF
    // — the workload closes after sending, and the agent's forward pump mirrors the
    // FIN onto leg B as `shutdown(SHUT_WR)` WITHOUT a TLS close_notify (nothing in
    // userspace holds the TLS state; kTLS does not synthesize one), which rustls
    // surfaces as `UnexpectedEof` — the expected clean end here.
    let mut got2 = Vec::new();
    let deadline2 = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline2 {
        let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
        match tls.read(&mut buf) {
            Ok(0) => break, // clean close_notify EOF (not expected, but terminal)
            Ok(n) => got2.extend_from_slice(&buf[..n]),
            Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(_) => break,
        }
    }
    let request2_ok = got2 == OUTBOUND_REQUEST2;
    if !request2_ok {
        eprintln!(
            "[05-03] mesh peer phase-2 mismatch: got {} bytes (want {})",
            got2.len(),
            OUTBOUND_REQUEST2.len()
        );
    }
    std::thread::sleep(Duration::from_millis(300));
    request_ok && request2_ok
}

// ---- shared TLS + socket helpers (re-authored from the 05-01 skeleton) ----

fn ca_root_store(ca_cert_pem: &str) -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    let mut rd = std::io::BufReader::new(ca_cert_pem.as_bytes());
    for c in rustls_pemfile::certs(&mut rd) {
        roots.add(c.expect("ca cert")).expect("add ca cert");
    }
    roots
}

fn drive_server_handshake(
    conn: &mut rustls::ServerConnection,
    tcp: &mut std::net::TcpStream,
) -> bool {
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

/// Accept one connection within `timeout` by polling a non-blocking accept.
fn accept_with_timeout(
    listener: &TcpListener,
    timeout: Duration,
) -> std::io::Result<(std::net::TcpStream, std::net::SocketAddr)> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + timeout;
    let result = loop {
        match listener.accept() {
            Ok(pair) => {
                pair.0.set_nonblocking(false).ok();
                break Ok(pair);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    break Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "no connection within timeout",
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => break Err(e),
        }
    };
    listener.set_nonblocking(false).ok();
    result
}

/// Run a `/dev/tcp`-style client INSIDE the workload netns: connect to `dst`, send
/// `request` (phase 1), read back `want` bytes, THEN send `request2` (phase 2 — the
/// steady-state payload, sent only after the phase-1 response proves the pumps are
/// live) and close. Returns the captured process output (stdout = the bytes read
/// back, stderr = `CLIENT-FAIL:...` on any error).
fn run_netns_client(
    dst: SocketAddrV4,
    request: &[u8],
    want: usize,
    request2: &[u8],
) -> std::process::Output {
    let req_literal: String = request.iter().map(|b| format!("\\x{b:02x}")).collect();
    let req2_literal: String = request2.iter().map(|b| format!("\\x{b:02x}")).collect();
    let script = format!(
        "\
import socket,sys
s=socket.socket(socket.AF_INET,socket.SOCK_STREAM)
s.settimeout(12)
try:
    s.connect(('{ip}',{port}))
    s.sendall(b'{req}')
    got=b''
    while len(got)<{want}:
        b=s.recv(65536)
        if not b: break
        got+=b
    s.sendall(b'{req2}')
    sys.stdout.buffer.write(got)
    sys.stdout.flush()
except Exception as e:
    sys.stderr.write('CLIENT-FAIL:'+str(e))
",
        ip = dst.ip(),
        port = dst.port(),
        req = req_literal,
        want = want,
        req2 = req2_literal,
    );
    Command::new("ip")
        .args(["netns", "exec", NS_W, "python3", "-c", &script])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn netns client")
}

// ============================================================================
// THE deliverable scenario (ADR-0071 / ADR-0069 OUTBOUND substrate mechanism)
// ============================================================================

/// THE OUTBOUND enforce-substrate bidirectional-splice zero-copy mechanism (the
/// increment-m promotion). Drives a TWO-PHASE outbound flow through PRODUCTION
/// `start_alloc` → `accept_loop` (getsockname → resolve(Mesh) → the real
/// `HostMtlsEnforcement::enforce`) on the real netns/veth + egress nft-TPROXY
/// topology while a `strace` attaches to the agent's pump threads, then asserts
/// BOTH directions are agent-light splice pumps: the phase-2 steady-state request
/// (sent only after the phase-1 response proves the pumps are live) reaches the
/// backend decrypted byte-exact WITHOUT its plaintext ever appearing in an
/// agent-thread `write`/`sendto` buffer (the forward BLOCKING splice into leg B's
/// kTLS-TX — zero userspace copy), and ≥1 traced fd is BOTH a splice source and a
/// splice destination (the leg fds — the bidirectional splice topology). Plus
/// encryption on the leg-B wire and the authn-only boundary (`expected_peer` None —
/// never asserted here because production owns the enforced connection internally;
/// the authn-only discipline is honoured by NOT asserting any intended-peer
/// protection claim).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn outbound_enforce_substrate_bidirectional_splice_zero_copy() {
    if !is_root() {
        eprintln!("SKIP outbound_enforce_substrate_bidirectional_splice_zero_copy: not root");
        return;
    }

    // Pin the verdict to a kernel (spike.md discipline).
    let kr = Command::new("uname")
        .arg("-r")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    eprintln!("[05-03] uname -r = {kr}");

    // strace must be present (the syscall oracle is load-bearing); its absence is a
    // gate FAILURE, not a skip — the canonical Lima VM ships it.
    assert!(
        Command::new("strace").arg("-V").output().is_ok_and(|o| o.status.success()),
        "strace is required for the outbound-substrate syscall oracle (bidirectional splice + \
         zero-copy); it is present in the canonical Lima VM — its absence is a gate failure, not \
         a skip"
    );

    // The composition root rustls CryptoProvider (installed once per process, as
    // overdrive-control-plane's serve boot does — a library must not mutate
    // process-global crypto state; the test IS the composition root here).
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Cross-process exclusion + clean baseline.
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();
    setup_topology();

    let pki = TestPki::mint();
    let identity: Arc<dyn IdentityRead> = Arc::new(held_identities(&pki));
    let adapter = Arc::new(HostMtlsEnforcement::new(identity, MtlsLimits::default()));

    // Earned-Trust probe BEFORE any enforce (the wire→probe→use invariant). On the
    // real 6.18/7.0 kernel this MUST pass.
    adapter
        .probe()
        .await
        .expect("Earned-Trust probe must pass on the real kernel before any enforce");

    let mesh_backend = SocketAddrV4::new(MESH_BACKEND_IP.parse().unwrap(), MESH_BACKEND_PORT);

    // The scripted resolve table the PRODUCTION accept_loop consumes:
    // mesh_backend → Mesh(backend.addr = mesh_backend, expected_svid = None). The
    // `expected_svid` None is the authn-only boundary (Q4 / #242) carried into the
    // resolved arm — production enforces with `expected_peer = None`.
    let mut table = BTreeMap::new();
    table.insert(
        mesh_backend,
        MtlsResolution::Mesh(ResolvedBackend { addr: mesh_backend, expected_svid: None }),
    );
    let resolve: Arc<dyn MtlsResolve> = Arc::new(ScriptedResolve::new(table));

    // Build the PRODUCTION worker over the REAL enforce substrate + the injected
    // resolve double, then drive `start_alloc` — this binds the PRODUCTION leg-F,
    // installs the egress rule on VETH_H, and spawns the PRODUCTION outbound
    // accept_loop. `spec.alloc = client_alloc` so production's `enforce` selects the
    // held CLIENT SVID for the leg-B handshake.
    let enforcement: Arc<dyn MtlsEnforcement> = Arc::clone(&adapter) as Arc<dyn MtlsEnforcement>;
    let worker = Arc::new(MtlsInterceptWorker::new(
        enforcement,
        Arc::clone(&resolve),
        Arc::new(SimClock::new()),
        Arc::new(HostMtlsIntercept::new()),
    ));
    let spec = build_client_spec(&pki, Some(VETH_H.to_owned()));
    worker.start_alloc(&spec).await.expect(
        "PRODUCTION start_alloc must bind leg-F + install the egress rule + spawn accept_loop",
    );

    // PANIC-SAFE teardown (F3): from here on, ANY panicking assertion scrubs the
    // node-global kernel state + stops the production alloc via this guard's Drop —
    // no leaked nft table / netns / lo-addr / fwmark rule, no 120 s hang on the
    // leaked accept_loop. Declared AFTER `_kernel_lock` so it drops FIRST (scrub runs
    // while the cross-process lock is still held).
    let topology_guard = TopologyGuard::new(Arc::clone(&worker), pki.client_alloc.clone());

    // The PRODUCTION install appended the `iifname VETH_H` egress rule (observable
    // kernel side effect; the worker — not the fixture — installed it).
    let dump = nft_dump_table();
    assert!(
        dump.contains(&format!("iifname \"{VETH_H}\"")) && dump.contains("tproxy to"),
        "start_alloc must install the iifname egress rule in the shared chain, got:\n{dump}"
    );

    // Start the leg-B wire capture (filtered to the mesh-backend port) BEFORE the
    // workload dials so the first leg-B record is on the captured wire (encryption
    // oracle). The leg-B records carry src/dst = mesh_backend_port.
    let outbound_wire = WireCapture::start(LOOPBACK_IFACE, MESH_BACKEND_PORT);
    let mesh_peer = spawn_mesh_peer(&pki);
    // Give the mesh peer a moment to bind before the workload dials / the agent
    // dials leg-B.
    std::thread::sleep(Duration::from_millis(200));

    // Attach strace to THIS test process (and its threads, `-f`) BEFORE the workload
    // dials, so every pump syscall on the forward + return SPLICE paths is captured.
    // Trace `splice` (both pumps' signature — sources AND destinations recovered) +
    // `sendto`/`write` (the ZERO-COPY oracle's negative surface: an agent write
    // carrying the phase-2 plaintext would be the copy-through-userspace
    // falsification; the netns client's own send is the excluded capture-works
    // control) + `recvfrom`/`read` (completeness) + `clone`/`clone3` (the RACE-FREE
    // thread-group closure: every post-attach CLONE_THREAD child is recovered from
    // the log, so the short-lived pump threads' TIDs are attributed
    // deterministically — see `TraceFindings::thread_group_closure`). `-s 512 -xx`
    // dumps the read/write buffers so the phase-2 plaintext can be located in (or
    // proven absent from) a `write`/`sendto` buffer. The attach also snapshots
    // `/proc/self/task` ONCE (the closure seed of pre-existing threads).
    let mut syscalls = StraceProbe::attach_self(&[
        "splice", "sendto", "write", "recvfrom", "read", "clone", "clone3",
    ]);

    // The workload (inside the netns) dials the mesh backend, sends the phase-1
    // request, reads the response, then sends the phase-2 STEADY-STATE request
    // (which must ride the forward splice — see the module docstring). Its egress
    // ingresses vethH → PREROUTING → TPROXY → PRODUCTION leg-F → PRODUCTION
    // accept_loop → getsockname → resolve(Mesh) → enforce. NO test code touches the
    // accept path — production owns it.
    let req = OUTBOUND_REQUEST.to_vec();
    let want_resp = OUTBOUND_RESPONSE.len();
    let req2 = OUTBOUND_REQUEST2.to_vec();
    let mesh_client =
        std::thread::spawn(move || run_netns_client(mesh_backend, &req, want_resp, &req2));

    // Drive the round-trip to completion (the workload reads the mesh server's
    // response byte-exact; the mesh server received the workload's request
    // byte-exact), then collect the strace trace + the leg-B wire scan.
    let client_out = mesh_client.join().expect("outbound mesh client thread");
    let client_read = client_out.stdout.clone();
    let mesh_request_ok = mesh_peer.join().expect("mesh peer thread");
    // Detach strace and parse. The test's thread group is derived RACE-FREE inside
    // `detach_and_read` (attach-time `/proc/self/task` seed ∪ CLONE_THREAD closure
    // parsed from the log) — no live sampling, so the short-lived forward pump cannot
    // be missed. Returns the recovered thread group for the falsification re-parse.
    let (trace, raw_trace, test_thread_group) = syscalls.detach_and_read();
    let scan =
        outbound_wire.stop_and_scan(&[OUTBOUND_REQUEST, OUTBOUND_RESPONSE, OUTBOUND_REQUEST2]);

    eprintln!(
        "[05-03] netns client exit={:?} stdout_len={} stderr={} | mesh_request_ok={}",
        client_out.status.code(),
        client_read.len(),
        String::from_utf8_lossy(&client_out.stderr).trim(),
        mesh_request_ok,
    );
    eprintln!("[05-03] leg-B wire scan = {scan:?}");
    eprintln!(
        "[05-03] test thread group (size {}) = {:?}",
        test_thread_group.len(),
        test_thread_group
    );
    eprintln!("[05-03] strace summary = {}", trace.summary());

    // The round-trip completed through the PRODUCTION accept_loop's Mesh arm — the
    // substrate genuinely ran end-to-end (a wrong getsockname/resolve/enforce would
    // never complete this round-trip). This is the precondition that makes the
    // syscall-mechanism assertions below meaningful: the pumps actually pumped.
    assert!(
        client_out.status.success(),
        "the netns workload client must exit cleanly (got {:?}, stderr={}) — the substrate must \
         have run for the mechanism to be observable",
        client_out.status.code(),
        String::from_utf8_lossy(&client_out.stderr).trim()
    );
    assert_eq!(
        client_read,
        OUTBOUND_RESPONSE,
        "the workload must read the mesh server's response byte-exact back over the RETURN splice \
         pump (leg-B kTLS-RX → leg-F) — through the PRODUCTION accept_loop Mesh arm (got {} bytes)",
        client_read.len()
    );
    assert!(
        mesh_request_ok,
        "the mesh server must receive BOTH requests byte-exact (decrypted): the phase-1 pre-arm \
         request AND the phase-2 steady-state request that rode the FORWARD blocking splice \
         (leg-F → leg-B kTLS-TX)"
    );

    // Encryption oracle: the leg-B wire shows TLS-1.3 application_data records in
    // BOTH directions and NO cleartext marker. The DIRECTIONAL 0x17 counts are the
    // load-bearing confidentiality proof (a cleartext leg-B would have zero records
    // in at least one direction).
    assert!(
        scan.has_app_data(),
        "the leg-B wire must carry TLS-1.3 0x17 application_data records (encryption), got {scan:?}"
    );
    assert!(
        scan.records_to_wire_port > 0,
        "the request direction (toward the mesh backend) must carry 0x17 records"
    );
    assert!(
        scan.records_from_wire_port > 0,
        "the response direction (from the mesh backend) must carry 0x17 records"
    );
    assert_eq!(
        scan.plaintext_marker_hits, 0,
        "NO cleartext request/response marker may appear on the encrypted leg-B wire"
    );

    // ----------------------------------------------------------------
    // THE mechanism assertions (the increment-m bidirectional-splice substrate).
    // ----------------------------------------------------------------

    // SPLICE PRESENT: the agent's pumps splice. At least one `splice(2)` must be
    // traced (the return decrypt pump out of leg-B's kTLS-RX, plus the forward
    // encrypt pump's phase-2 splice into leg-B's kTLS-TX).
    assert!(
        trace.splice_calls > 0,
        "MECHANISM (splice present): the pumps must move bytes via splice(2) — at least one must \
         be traced; strace summary:\n{}",
        trace.summary()
    );
    // BIDIRECTIONAL TOPOLOGY: ≥1 recovered fd is BOTH a splice SOURCE and a splice
    // DESTINATION — the leg fds. Leg B (one TX+RX kTLS fd) is the return pump's
    // splice SOURCE and the forward pump's splice DESTINATION; leg F is the inverse.
    // The pumps' pipe fds never qualify (a pipe read half is only ever a source, a
    // write half only ever a destination), so a non-empty intersection pins the
    // splice topology to the connection's legs — both directions genuinely splice
    // through the same leg fds, not an incidental splice elsewhere in the process.
    assert!(
        trace.leg_fd_spliced_in_both_directions(),
        "MECHANISM (bidirectional splice through the legs): at least one traced fd must be BOTH a \
         splice source and a splice destination (leg B: return-splice source + forward-splice \
         destination; leg F the inverse). No such fd was recovered — one of the two directions \
         did not splice through a shared leg fd. strace summary:\n{}",
        trace.summary()
    );

    // FORWARD = ZERO-COPY SPLICE: the phase-2 steady-state request reached the
    // backend decrypted byte-exact (asserted above via `mesh_request_ok`), yet its
    // plaintext marker must NEVER appear in a traced `write(2)`/`sendto(2)` buffer
    // issued BY THE AGENT (a thread of THIS process) — the forward pump moves it
    // with a blocking splice(legF → pipe → legB); the kernel encrypts inside the
    // splice, so no userspace buffer ever carries the steady-state payload. The
    // AGENT-attribution (thread-group filter) is what makes this load-bearing: the
    // netns client legitimately sends the same plaintext from a separate process —
    // excluded (and used below as the capture-works control).
    assert!(
        !trace.steady_marker_copied_by_agent,
        "MECHANISM (forward = zero-copy splice): the phase-2 steady-state request plaintext must \
         NEVER appear in a traced write(2)/sendto(2) buffer issued by a THREAD OF THE TEST PROCESS \
         (the agent) — it rides the blocking splice into leg-B's kTLS-TX. It DID appear (agent \
         marker writes = {}), which means the forward copied the steady-state payload through \
         userspace (the retired copy-pump shape). strace summary:\n{}",
        trace.agent_marker_writes,
        trace.summary()
    );

    // ----------------------------------------------------------------
    // FLAGS ORACLE (review finding m2): the FORWARD encrypt pump delivers into leg-B's
    // kTLS-TX with a BLOCKING `splice(SPLICE_F_MOVE)` — no `SPLICE_F_NONBLOCK`.
    // `tls_sw_sendmsg` waits for send-buffer space INSIDE the blocking call, which is the
    // spike-proven lossless invariant (findings-ktls-tx-blocking-splice.md). NON-blocking
    // delivery into kTLS-TX is the RETIRED silent-loss class: its failure mode is
    // `n_out == len, errno == 0` while the peer receives nothing under send-buffer
    // pressure — so a small-payload fast-reader Tier-3 run (this test) stays GREEN even
    // AFTER the regression, and every OTHER assertion here (round-trip, zero-copy,
    // bidirectional) passes. The hardcoded-flags INVARIANT comment lives on
    // `blocking_splice` (crates/overdrive-dataplane/src/mtls/splice.rs); this is the
    // RUNTIME oracle that a future edit re-adding `SPLICE_F_NONBLOCK` to the encrypt path
    // (e.g. copy-pasted from the decrypt pump, which legitimately uses MOVE|NONBLOCK)
    // would otherwise slip past.
    //
    // IDENTIFICATION — byte-length correlation (no leg-B fd needed). `OUTBOUND_REQUEST2`
    // is the ONLY message whose plaintext length rides the forward encrypt pump: REQ1
    // rides the pre-arm prelude `write_all` (or, if late, a len(REQ1)-byte forward
    // splice), and the RESPONSE rides the RETURN decrypt pump (NONBLOCK, len(RESPONSE)).
    // len(REQ2) is distinct from both (the debug_assert below locks that in), so a
    // MOVE-only splice returning len(REQ2) uniquely identifies the phase-2 forward
    // encrypt — the length IS the phase-2 scope; no temporal window is needed.
    //
    // REGRESSION-COMPLETE by construction: `blocking_splice` is the SOLE splice primitive
    // the forward pump uses for BOTH its legF→pipe splice-in AND its pipe→kTLS-TX
    // splice-out, and it hardcodes `SPLICE_F_MOVE`. If it regresses to add
    // `SPLICE_F_NONBLOCK`, BOTH forward splices flip to MOVE|NONBLOCK and NO MOVE-only
    // splice returns len(REQ2) — this assertion FAILS (it does not merely "miss" the
    // regression). The decrypt pump uses `libc::splice` directly (not `blocking_splice`),
    // so its legitimate NONBLOCK is unaffected.
    //
    // CAPTURE-BREAKAGE GUARD (non-vacuous — the flags analogue of the F4
    // present-and-excluded control below). The `nonblock_splice_lines ≥ 1` control keys
    // on the RETURN decrypt pump's legitimate MOVE|NONBLOCK splices; it covers these
    // breakage directions: (a) NO splice lines captured at all → the positive EXISTS is
    // false AND nonblock == 0 → both assertions fail; (b) splice lines present but flags
    // un-rendered / numerically-rendered / dropped by an `-e` filter → `splice_flags`
    // returns `None` → nonblock == 0 → the control fails (not a vacuous green); (c) flags
    // rendered but the forward regressed to NONBLOCK → the positive EXISTS is false →
    // fails on the assertion itself. A positive EXISTS oracle is inherently non-vacuous on
    // an empty capture (∄ ⇒ false ⇒ fail); the NONBLOCK control additionally proves the
    // ABSENCE of NONBLOCK on the forward is a GENUINE absence (flags ARE capturable and
    // this parser DOES read them), not a parse gap.
    debug_assert!(
        OUTBOUND_REQUEST2.len() != OUTBOUND_REQUEST.len()
            && OUTBOUND_REQUEST2.len() != OUTBOUND_RESPONSE.len(),
        "the flags oracle keys on len(REQ2) being UNIQUE in the transfer window \
         (REQ1={}, RESPONSE={}, REQ2={}); a length collision would let a non-forward-REQ2 \
         splice false-trip (or mask) the MOVE-only-returns-len(REQ2) oracle",
        OUTBOUND_REQUEST.len(),
        OUTBOUND_RESPONSE.len(),
        OUTBOUND_REQUEST2.len(),
    );
    assert!(
        trace.nonblock_splice_lines >= 1,
        "FLAGS ORACLE guard (present-and-distinct control): the RETURN decrypt pump splices \
         out of leg-B's kTLS-RX with SPLICE_F_MOVE|SPLICE_F_NONBLOCK, so ≥ 1 NONBLOCK splice \
         MUST be captured — this proves strace rendered splice flags AND the parser read \
         them, so the forward pump's MOVE-only-ness (asserted next) is a GENUINE absence of \
         NONBLOCK, not a capture/parse gap that would make the oracle vacuous. Got zero \
         NONBLOCK splices. strace summary:\n{}",
        trace.summary()
    );
    assert!(
        trace.forward_move_only_splice_of_req2_len(),
        "FLAGS ORACLE (forward = BLOCKING splice into kTLS-TX): ≥ 1 splice returning \
         len(REQ2)={} with SPLICE_F_MOVE ONLY (no SPLICE_F_NONBLOCK) MUST be captured — the \
         forward encrypt pump moves REQ2's steady-state bytes into leg-B's kTLS-TX with a \
         BLOCKING splice (findings-ktls-tx-blocking-splice.md), which is the lossless \
         invariant. NONE was found: the forward regressed to NON-blocking delivery into \
         kTLS-TX (the retired silent-loss class), which this fast-reader small-payload \
         round-trip does NOT otherwise catch (it stays green). MOVE-only splice return \
         lengths seen = {:?}. strace summary:\n{}",
        OUTBOUND_REQUEST2.len(),
        trace.move_only_splice_return_lens,
        trace.summary()
    );
    eprintln!(
        "[05-03] FLAGS ORACLE OK (m2): forward encrypt pump splices len(REQ2)={} into leg-B's \
         kTLS-TX with SPLICE_F_MOVE only (blocking, lossless) — MOVE-only splice return lengths \
         {:?}; NONBLOCK control satisfied ({} NONBLOCK splices from the return decrypt pump prove \
         flags are captured).",
        OUTBOUND_REQUEST2.len(),
        trace.move_only_splice_return_lens,
        trace.nonblock_splice_lines,
    );

    // ----------------------------------------------------------------
    // FALSIFICATION of the zero-copy oracle (the load-bearing S1 re-validation, F4).
    //
    // A NEGATIVE oracle ("the agent wrote no marker") is vacuous if the capture
    // missed marker-carrying writes entirely, or if the partition silently
    // mis-attributed them. The falsification HOLDS the netns client's plaintext
    // send CONSTANT and varies ONLY the attribution partition, proving both that
    // marker-carrying writes ARE capturable and that the partition SET — not the
    // bytes on the wire — is the discriminator:
    //
    //   (a) under the REAL race-free partition, the netns client's marker-carrying
    //       sendto exists in the trace (captured under `strace -f`) and is the EXCLUDED
    //       population — `excluded_marker_writes ≥ 1`, with the client's TID(s)
    //       recovered FROM the trace (a separate-process fork, no CLONE_THREAD, so
    //       never in the closure). This is the client held CONSTANT — and the proof
    //       the agent's ZERO is a genuine zero, not a capture failure.
    //   (b) re-parse with those SAME client TIDs ADDED to the attribution set. The
    //       identical client writes — same bytes, same lines — now flip to
    //       agent-attributed: `agent_marker_writes` rises by EXACTLY the excluded
    //       count, and `excluded_marker_writes` drops to zero. Nothing about the wire
    //       changed; only the partition did. So it is the PARTITION that attributes a
    //       marker write to the agent vs the client.
    //
    // This is race-free (it re-parses the captured log; no live sampling) and runs on
    // every PASS path (the round-trip asserts above already passed, so the client
    // necessarily sent the phase-2 plaintext).
    assert!(
        trace.excluded_marker_writes >= 1,
        "FALSIFICATION (client held CONSTANT, present-and-excluded): the netns workload client's \
         own phase-2 marker-carrying sendto MUST exist in the trace (it sent the steady-state \
         plaintext) yet be EXCLUDED under the race-free partition — got {} excluded marker writes. \
         If zero, the client's send was not captured and the zero-copy oracle would be vacuous. \
         summary:\n{}",
        trace.excluded_marker_writes,
        trace.summary()
    );
    assert!(
        !trace.excluded_marker_write_tids.is_empty(),
        "FALSIFICATION: the EXCLUDED client TID(s) must be recoverable from the trace to vary the \
         attribution against them. summary:\n{}",
        trace.summary()
    );
    // Vary ONLY the partition: add the client's recovered TIDs to the thread group
    // (`test_thread_group` is no longer needed after this, so move it).
    let mut group_with_client = test_thread_group;
    group_with_client.extend(trace.excluded_marker_write_tids.iter().copied());
    let reattributed = TraceFindings::parse(&raw_trace, &group_with_client);
    assert_eq!(
        reattributed.agent_marker_writes,
        trace.agent_marker_writes + trace.excluded_marker_writes,
        "FALSIFICATION (vary the partition, hold the client constant): adding the client's recovered \
         TID(s) to the attribution set MUST re-attribute its EXACT same marker writes to the \
         'agent' count (rising by the excluded count) — the bytes did not change, only the \
         partition did, proving the thread-group SET is the discriminator. Got agent={} (expected \
         {}+{}). summary (re-parsed):\n{}",
        reattributed.agent_marker_writes,
        trace.agent_marker_writes,
        trace.excluded_marker_writes,
        reattributed.summary()
    );
    assert_eq!(
        reattributed.excluded_marker_writes,
        0,
        "FALSIFICATION: once the client's TID(s) are in the attribution set, NO marker write may \
         remain excluded — every marker-carrying write is now attributed. Got {} still excluded. \
         summary:\n{}",
        reattributed.excluded_marker_writes,
        reattributed.summary()
    );
    eprintln!(
        "[05-03] FALSIFICATION OK (F4): forward oracle keys on the PARTITION, not the bytes — under \
         the race-free partition agent_marker_writes={} / excluded_marker_writes={} (client TIDs \
         {:?}); ADDING the client's TIDs re-attributes its identical writes to agent={} \
         (excluded→0). The client's plaintext send is excluded ONLY because its TID is a \
         separate-process fork, never in the CLONE_THREAD closure.",
        trace.agent_marker_writes,
        trace.excluded_marker_writes,
        trace.excluded_marker_write_tids,
        reattributed.agent_marker_writes,
    );

    eprintln!(
        "[05-03] VERDICT: WORKS — OUTBOUND enforce-substrate bidirectional splice validated on \
         kernel {kr}: FORWARD (workload → backend, leg-F → leg-B) is a ZERO-COPY blocking splice \
         into leg-B's kTLS-TX (the phase-2 steady-state request reached the backend byte-exact \
         with its plaintext in NO agent-thread write/sendto buffer — the netns client's identical \
         send is excluded and doubles as the capture control), RETURN (backend → workload, leg-B \
         → leg-F) is a splice out of leg-B's kTLS-RX; ≥1 leg fd recovered as BOTH splice source \
         and destination (the bidirectional topology). Encryption asserted (0x17 both directions, \
         no cleartext on the leg-B wire). Authn-only honoured (expected_svid None on the resolved \
         arm; no intended-peer protection claim, #242)."
    );

    // The clean path explicitly owns and awaits worker teardown before shared
    // nft/topology removal. Drop is only the synchronous panic fallback.
    topology_guard.finish().await.expect("splice fixture worker cleanup must converge");
}

// =====================================================================
// strace syscall oracle — attach `strace -f -p <self>` to the running test process
// so the agent's own pump threads' syscalls are captured, then parse the trace for
// the OUTBOUND bidirectional-splice mechanism (forward BLOCKING `splice` into the
// kTLS-TX leg with zero userspace copy of the steady-state payload; return
// zero-copy `splice` out of the kTLS-RX leg).
// =====================================================================

/// A live `strace` attached to this test process (and its threads). Captures the raw
/// syscall log to a temp file; `detach_and_read` stops it and parses.
///
/// `seed_tids` is a SINGLE `/proc/self/task` snapshot taken at attach time — it
/// captures the *pre-existing* thread group (the tokio runtime + accept-loop threads
/// alive at the attach instant). Combined with the CLONE_THREAD closure parsed from
/// the strace log (which captures every thread created AFTER attach, including the
/// short-lived pump threads), this derives the test's thread group RACE-FREE —
/// no live polling, so a pump thread shorter-lived than any poll interval cannot be
/// missed (the prior `TidSampler` 15 ms poll raced sub-15 ms pumps, ~29% miss).
struct StraceProbe {
    child: Option<Child>,
    out_path: std::path::PathBuf,
    seed_tids: std::collections::BTreeSet<i32>,
}

impl StraceProbe {
    /// Attach `strace -f -p <self_pid>` filtered to `syscalls`, dumping read/write
    /// buffers (`-s 512 -xx`) so the phase-2 plaintext can be located in (or proven
    /// absent from) a `write`/`sendto` buffer. `syscalls` MUST include
    /// `clone` + `clone3` (the thread-group-closure seed lines) — see
    /// `TraceFindings::thread_group_closure`. Blocks briefly until strace has attached
    /// (so the pump syscalls that follow are captured), then snapshots
    /// `/proc/self/task` ONCE (the race-free seed of pre-existing threads).
    fn attach_self(syscalls: &[&str]) -> Self {
        debug_assert!(
            syscalls.contains(&"clone") && syscalls.contains(&"clone3"),
            "the strace filter MUST include clone + clone3 — the post-attach CLONE_THREAD \
             closure (race-free thread-group derivation) is parsed from those lines"
        );
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
        // Give strace a moment to attach to every thread before the pumps spawn; a
        // few hundred ms is ample on the Lima VM.
        std::thread::sleep(Duration::from_millis(400));
        // SEED snapshot — taken ONCE, now, AFTER strace has attached and BEFORE the
        // dial. Every thread alive at this instant (the tokio worker pool, the
        // accept-loop thread) is captured race-free by this single read. Threads
        // created AFTER this instant — the pump threads in particular — are
        // captured instead by the CLONE_THREAD closure parsed from the log, whose
        // clone line is PERMANENTLY present regardless of how briefly the thread lived.
        let seed_tids = snapshot_proc_self_task();
        Self { child: Some(child), out_path, seed_tids }
    }

    /// Stop strace (SIGTERM → it detaches cleanly and flushes the log), read the
    /// captured trace, and parse it for the substrate mechanism evidence.
    ///
    /// The test's thread group is derived RACE-FREE inside `parse`: the attach-time
    /// `seed_tids` (pre-existing threads) UNION the transitive CLONE_THREAD closure
    /// parsed from the log (post-attach threads, incl. the short-lived forward pump).
    /// Returns `(findings, raw_trace, thread_group)`. The raw trace + the recovered
    /// thread group are returned so the caller can RE-PARSE against a DIFFERENT
    /// attribution set for the falsification (proving the live flag was set by an
    /// in-tgid agent TID, not the netns client) without re-reading the on-disk file
    /// (which `Drop` removes).
    fn detach_and_read(&mut self) -> (TraceFindings, String, std::collections::BTreeSet<i32>) {
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
        // Diagnostic dump of the agent's splice lines so a return-mechanism mismatch
        // is debuggable from the captured nextest output.
        for line in raw.lines() {
            let (tid_dbg, body) = split_strace_tid_prefix(line);
            // Dump BOTH the splice start (`splice(` — carries the flags arg on the
            // unfinished/inline form) AND the resumed half (`<... splice resumed> ... =
            // <ret>` — carries the return the m2 flags oracle correlates per tid), wide
            // enough to show flags + return, so the oracle is validatable from the captured
            // nextest output.
            if body.starts_with("splice(")
                || (body.starts_with("<...") && body.contains("splice resumed"))
            {
                let tid_dbg = tid_dbg.map_or_else(|| "?".to_owned(), |t| t.to_string());
                let head: String = body.chars().take(160).collect();
                eprintln!("STRACE[{tid_dbg}]: {head}");
            }
        }
        // RACE-FREE thread group: seed (pre-existing) ∪ CLONE_THREAD closure (post-attach).
        let thread_group = TraceFindings::thread_group_closure(&raw, &self.seed_tids);
        let findings = TraceFindings::parse(&raw, &thread_group);
        (findings, raw, thread_group)
    }
}

/// Snapshot the test process's current thread-group TIDs from `/proc/self/task`. A
/// single read — race-free for every thread alive at the call instant.
fn snapshot_proc_self_task() -> std::collections::BTreeSet<i32> {
    let mut tids = std::collections::BTreeSet::new();
    if let Ok(entries) = std::fs::read_dir("/proc/self/task") {
        for e in entries.flatten() {
            if let Some(t) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) {
                tids.insert(t);
            }
        }
    }
    tids
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

/// The OUTBOUND substrate-mechanism evidence parsed from the strace log.
struct TraceFindings {
    /// `splice(2)` was used (the pumps — forward encrypt INTO leg-B's kTLS-TX,
    /// return decrypt OUT of leg-B's kTLS-RX).
    splice_calls: usize,
    /// The set of recovered `splice(2)` SOURCE fds (the return pump splices OUT of
    /// leg-B's kTLS-RX; the forward pump splices OUT of leg F and its pipe read
    /// half). Intersected with `splice_dst_fds` to PIN the bidirectional splice
    /// topology to the connection's leg fds (S3).
    splice_src_fds: std::collections::BTreeSet<i32>,
    /// The set of recovered `splice(2)` DESTINATION fds (the forward pump splices
    /// INTO leg-B's kTLS-TX; the return pump INTO leg F and its pipe write half).
    /// Leg B is a SINGLE kTLS fd (TX+RX armed), so it appears in BOTH sets — as does
    /// leg F (forward source + return destination). A pipe fd never does (a read
    /// half is only ever a source, a write half only ever a destination).
    splice_dst_fds: std::collections::BTreeSet<i32>,
    /// The phase-2 STEADY-STATE request plaintext appeared in a traced
    /// `write(2)`/`sendto(2)` buffer issued BY A THREAD OF THE TEST PROCESS — the
    /// AGENT copying the steady-state payload through userspace. MUST be false (the
    /// FORWARD is a blocking zero-copy splice; only the pre-arm prelude — phase 1 —
    /// legitimately rides a `write_all`). The thread-group filter is what makes this
    /// attribute to the agent and NOT to the netns client's own identical send.
    steady_marker_copied_by_agent: bool,
    /// The count of phase-2 marker-carrying writes attributed to the AGENT (a TID in
    /// the test thread group). MUST be 0 — the zero-copy signal.
    agent_marker_writes: usize,
    /// The count of phase-2 marker-carrying writes attributed to a NON-agent TID
    /// (the netns workload client's own `s.sendall(request2)`, captured under
    /// `strace -f`). This is the EXCLUDED population — it exists in the trace but
    /// does NOT flip the zero-copy oracle. Tracked so the falsification can prove
    /// both that marker-carrying writes ARE capturable (the agent's zero is a
    /// genuine zero) and that the partition is the discriminator.
    excluded_marker_writes: usize,
    /// The TIDs of the EXCLUDED marker-carrying writes (the netns client's send,
    /// captured under `strace -f` but NOT in the test thread group). The falsification
    /// (F4) re-parses with these TIDs ADDED to the attribution set and shows the SAME
    /// client writes then flip to agent-attributed — holding the client's send
    /// CONSTANT and varying ONLY the partition, proving the thread-group set is the
    /// discriminator, not the bytes on the wire.
    excluded_marker_write_tids: std::collections::BTreeSet<i32>,
    /// FLAGS ORACLE (m2), present-and-distinct control: the count of captured inline
    /// `splice(2)` lines whose flags arg contains `SPLICE_F_NONBLOCK`. The RETURN decrypt
    /// pump legitimately splices out of leg-B's kTLS-RX with `SPLICE_F_MOVE|SPLICE_F_NONBLOCK`
    /// (`libc::splice` directly, not `blocking_splice`), so this MUST be ≥ 1 — it proves
    /// strace rendered splice flags AND this parser read them, which is what makes the
    /// forward pump's MOVE-only-ness a GENUINE absence of NONBLOCK rather than a
    /// capture/parse gap (the analogue of the F4 present-and-excluded control).
    nonblock_splice_lines: usize,
    /// FLAGS ORACLE (m2), positive surface: the set of return byte-counts of captured
    /// inline `splice(2)` lines whose flags are `SPLICE_F_MOVE` ONLY (no `SPLICE_F_NONBLOCK`)
    /// and whose return is > 0. The FORWARD encrypt pump moves REQ2's steady-state bytes
    /// into leg-B's kTLS-TX with a BLOCKING (`SPLICE_F_MOVE`-only) splice, so
    /// `len(OUTBOUND_REQUEST2)` MUST appear here — and it uniquely identifies the phase-2
    /// forward encrypt because len(REQ2) rides no other pump (REQ1 → prelude write_all,
    /// RESPONSE → NONBLOCK decrypt pump). If `blocking_splice` regresses to add
    /// `SPLICE_F_NONBLOCK`, both forward splices flip to MOVE|NONBLOCK and len(REQ2) never
    /// appears here.
    move_only_splice_return_lens: std::collections::BTreeSet<i64>,
    write_calls: usize,
    read_calls: usize,
}

impl TraceFindings {
    /// A distinctive interior substring of the phase-2 STEADY-STATE request
    /// (`OUTBOUND_REQUEST2`). Because the FORWARD is a zero-copy blocking splice
    /// into leg-B's kTLS-TX, this plaintext must NEVER appear in a `write`/`sendto`
    /// buffer off an agent thread — only the netns client's own send carries it.
    /// Derived as a real sub-slice of `OUTBOUND_REQUEST2` (S4: a `debug_assert!`
    /// pins it as an actual substring so silent drift of either the request or the
    /// marker cannot go unnoticed).
    fn steady_marker() -> &'static [u8] {
        // The interior bytes after the `OVERDRIVE_0503_OUTBOUND_REQUEST2_` prefix
        // (33 bytes) through end — a real sub-slice of OUTBOUND_REQUEST2
        // (`phase2_steady_state_marker_..._no_copy`).
        let marker = &OUTBOUND_REQUEST2[33..];
        debug_assert!(
            OUTBOUND_REQUEST2.windows(marker.len()).any(|w| w == marker),
            "steady_marker MUST be an actual sub-slice of OUTBOUND_REQUEST2 (S4 drift guard)"
        );
        marker
    }

    /// Derive the test process's thread group RACE-FREE from `(seed_tids,
    /// strace_log)`. `seed_tids` is the attach-time `/proc/self/task` snapshot
    /// (pre-existing threads). The strace log carries every `clone`/`clone3` issued
    /// AFTER attach; each thread-creating clone (one whose flag set contains
    /// `CLONE_THREAD`) is emitted by a parent TID (the strace `-f` line prefix) and
    /// returns the child TID. Seed the closure with `seed_tids`, then add any
    /// CLONE_THREAD child whose PARENT TID is already in the set, to a fixpoint.
    ///
    /// This is the structural defense against the prior `TidSampler` flake: a
    /// short-lived pump thread is created AFTER attach, so its clone line is
    /// PERMANENTLY in the log regardless of how briefly it lived — captured by the
    /// closure. Its parent (a tokio runtime / accept-loop thread) is captured by the
    /// seed (pre-existing) or by the closure (also post-attach), so the closure
    /// reaches the pump. A process fork (the netns `ip`/`python3` client) is a `clone`
    /// WITHOUT `CLONE_THREAD` → never added → deterministically excluded. Pure over
    /// its inputs; unit-tested against a captured fixture.
    fn thread_group_closure(
        raw: &str,
        seed_tids: &std::collections::BTreeSet<i32>,
    ) -> std::collections::BTreeSet<i32> {
        // Collect the post-attach CLONE_THREAD edges: (parent_tid -> child_tid).
        let edges = clone_thread_edges(raw);
        let mut group = seed_tids.clone();
        // Fixpoint: a CLONE_THREAD child whose parent is in the group joins the group;
        // iterate until no edge adds a new member (a thread can itself spawn threads).
        loop {
            let mut added = false;
            for &(parent, child) in &edges {
                if group.contains(&parent) && group.insert(child) {
                    added = true;
                }
            }
            if !added {
                break;
            }
        }
        group
    }

    /// Parse the strace log, attributing each phase-2 marker-carrying write to the
    /// AGENT (a TID in `test_thread_group`) or to the excluded netns client (any
    /// other TID).
    ///
    /// `test_thread_group` is the RACE-FREE thread group from `thread_group_closure`
    /// (attach-time seed ∪ CLONE_THREAD closure) — the agent's pump threads are
    /// CLONE_THREAD threads of the test process, so their TIDs are in this set; the
    /// netns `python3` client is a separate-process fork (no CLONE_THREAD) whose TID
    /// is not.
    fn parse(raw: &str, test_thread_group: &std::collections::BTreeSet<i32>) -> Self {
        let mut splice_calls = 0usize;
        let mut write_calls = 0usize;
        let mut read_calls = 0usize;
        let mut agent_marker_writes = 0usize;
        let mut excluded_marker_writes = 0usize;
        let mut excluded_marker_write_tids: std::collections::BTreeSet<i32> =
            std::collections::BTreeSet::new();

        // `-xx` renders buffers as `\xHH\xHH...`; convert the marker to that hex form
        // so a substring match against the raw line finds the plaintext regardless of
        // where strace truncated the buffer or split it across records.
        let req_hex = to_strace_hex(Self::steady_marker());

        // The pumps' splice SOURCE and DESTINATION fds — `splice(SRC, NULL, DST,
        // NULL, len, flags)`. Leg B is a SINGLE kTLS fd (TX+RX armed on the same fd,
        // mtls/outbound.rs): the return pump splices OUT of it (source) and the
        // forward pump splices INTO it (destination); leg F is the inverse. The
        // src ∩ dst intersection therefore recovers the leg fds and PINS the
        // bidirectional splice topology (S3) — a pipe fd never appears in both sets.
        let mut splice_src_fds: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();
        let mut splice_dst_fds: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();

        // FLAGS ORACLE (m2): pin the BLOCKING (`SPLICE_F_MOVE`-only) discipline of the
        // FORWARD encrypt pump. `nonblock_splice_lines` is the present-and-distinct control
        // (the RETURN decrypt pump's MOVE|NONBLOCK splices prove flags are captured);
        // `move_only_splice_return_lens` is the positive surface (the forward pump's
        // MOVE-only splice return byte-counts, keyed on len(REQ2) at the assertion site).
        //
        // The pumps' splices BLOCK, so strace renders each as an unfinished/resumed PAIR:
        // the FLAGS live on the `... <unfinished ...>` half and the RETURN on the
        // `<... splice resumed> ... = <ret>` half — split across lines. Correlate them per
        // tid: a pump thread is blocked inside its splice, so at most ONE splice is
        // outstanding per tid at a time (the same non-re-entrancy `clone_thread_edges`
        // relies on). `pending_splice_nonblock[tid]` carries the unfinished half's flag
        // kind (is-NONBLOCK) until the resumed half supplies the return.
        let mut nonblock_splice_lines = 0usize;
        let mut move_only_splice_return_lens: std::collections::BTreeSet<i64> =
            std::collections::BTreeSet::new();
        let mut pending_splice_nonblock: std::collections::BTreeMap<i32, bool> =
            std::collections::BTreeMap::new();

        for line in raw.lines() {
            // strace `-f` prefixes each line with the traced thread's TID then a
            // space: `<tid> syscall(args) = ret`, with blocking calls split as
            // `... <unfinished ...>` / `<... syscall resumed> ...`. Recover the TID
            // (for thread-group attribution) AND the body (syscall + args). Classify
            // by the leading syscall-name token.
            //
            // The agent's FORWARD pump moves the steady-state payload with a
            // blocking `splice(legF → pipe → legB)`; the kernel encrypts inside the
            // splice, so NO agent write buffer ever carries the phase-2 plaintext —
            // a `write`/`sendto(<marker2>)` from an agent TID would be the
            // copy-through-userspace falsification. The RETURN pump is `splice` out
            // of leg-B's kTLS-RX. The netns client legitimately sends the phase-2
            // plaintext itself, from a non-agent TID — excluded below (and used as
            // the capture-works control by the falsification).
            let (tid, body) = split_strace_tid_prefix(line);
            let is_resume = body.starts_with("<...");
            let names = |n: &str| body.starts_with(n) || (is_resume && body.contains(n));
            let carries_req = body.contains(&req_hex);

            let is_splice_start = body.starts_with("splice(");
            let is_splice_resume = is_resume && body.contains("splice resumed");
            if is_splice_start {
                splice_calls += 1;
                if let Some(src) = splice_source_fd(body) {
                    splice_src_fds.insert(src);
                }
                if let Some(dst) = splice_dest_fd(body) {
                    splice_dst_fds.insert(dst);
                }
                // FLAGS ORACLE (m2), START half. The flags arg (arg 5) is present on BOTH
                // the inline and the `<unfinished ...>` form. The RETURN is present only on
                // the inline form; a blocking splice's return arrives on its resumed half.
                if let Some(flags) = splice_flags(body) {
                    let is_nonblock = flags.contains("SPLICE_F_NONBLOCK");
                    if body.contains("<unfinished") {
                        // Blocking splice stopped mid-syscall: stash the flag kind per tid;
                        // the resumed half supplies the return and completes the pairing.
                        if let Some(t) = tid {
                            pending_splice_nonblock.insert(t, is_nonblock);
                        }
                    } else {
                        // Inline splice: flags AND return on one line — classify directly.
                        classify_splice(
                            is_nonblock,
                            splice_return(body),
                            &mut nonblock_splice_lines,
                            &mut move_only_splice_return_lens,
                        );
                    }
                }
            } else if is_splice_resume {
                // FLAGS ORACLE (m2), RESUMED half: the return is here; the flag kind was
                // stashed by the matching `<unfinished ...>` half (same tid). This is the
                // line that carries the `= len(REQ2)` the forward encrypt oracle keys on.
                if let Some(t) = tid
                    && let Some(is_nonblock) = pending_splice_nonblock.remove(&t)
                {
                    classify_splice(
                        is_nonblock,
                        splice_return(body),
                        &mut nonblock_splice_lines,
                        &mut move_only_splice_return_lens,
                    );
                }
            } else if names("sendto(") || names("write(") {
                write_calls += 1;
                if carries_req {
                    // ATTRIBUTION (S1): a phase-2 marker-carrying write counts
                    // against the zero-copy oracle only when its owning TID belongs
                    // to the TEST process's thread group (the agent's pump threads).
                    // The netns workload client sends the same plaintext from a
                    // SEPARATE process whose TID is not in the set — that send is
                    // EXCLUDED, so it cannot false-trip the oracle. The in-process
                    // mesh-peer thread writes CIPHERTEXT and never carries the
                    // plaintext marker, so it cannot false-trip here either.
                    match tid {
                        Some(t) if test_thread_group.contains(&t) => {
                            agent_marker_writes += 1;
                        }
                        _ => {
                            excluded_marker_writes += 1;
                            if let Some(t) = tid {
                                excluded_marker_write_tids.insert(t);
                            }
                        }
                    }
                }
            } else if names("recvfrom(") || names("read(") {
                read_calls += 1;
            }
        }

        Self {
            splice_calls,
            splice_src_fds,
            splice_dst_fds,
            steady_marker_copied_by_agent: agent_marker_writes > 0,
            agent_marker_writes,
            excluded_marker_writes,
            excluded_marker_write_tids,
            nonblock_splice_lines,
            move_only_splice_return_lens,
            write_calls,
            read_calls,
        }
    }

    /// True iff ≥1 recovered fd is BOTH a `splice` SOURCE and a `splice`
    /// DESTINATION — the connection's leg fds (leg B: return-splice source +
    /// forward-splice destination on one TX+RX kTLS fd; leg F the inverse). PINS
    /// the bidirectional splice topology (S3) rather than admitting any incidental
    /// splice: a pipe fd never qualifies (its read half is only ever a source, its
    /// write half only ever a destination). `None`-safe: empty when either set was
    /// unpopulated.
    fn leg_fd_spliced_in_both_directions(&self) -> bool {
        self.splice_src_fds.intersection(&self.splice_dst_fds).next().is_some()
    }

    /// FLAGS ORACLE (m2), positive verdict: `true` iff ≥ 1 captured `splice(2)` line
    /// carried `SPLICE_F_MOVE` ONLY (no `SPLICE_F_NONBLOCK`) AND returned exactly
    /// `len(OUTBOUND_REQUEST2)` bytes — the FORWARD encrypt pump's BLOCKING splice of the
    /// phase-2 steady-state request into leg-B's kTLS-TX. len(REQ2) rides no other pump
    /// (REQ1 → prelude write_all; RESPONSE → the NONBLOCK decrypt pump), so a MOVE-only
    /// splice returning it uniquely identifies the phase-2 forward encrypt — the byte
    /// length IS the phase-2 scope, no temporal window needed. Regression-complete: if
    /// `blocking_splice` (splice.rs) regresses to add `SPLICE_F_NONBLOCK`, BOTH forward
    /// splices flip to MOVE|NONBLOCK and len(REQ2) never lands in `move_only_splice_return_lens`.
    fn forward_move_only_splice_of_req2_len(&self) -> bool {
        self.move_only_splice_return_lens.contains(&(OUTBOUND_REQUEST2.len() as i64))
    }

    fn summary(&self) -> String {
        format!(
            "splice={} splice_srcs={:?} splice_dsts={:?} write={} read={} \
             agent_marker_writes={} excluded_marker_writes={} excluded_tids={:?} \
             steady_copy_seen={} leg_spliced_both_dirs={} nonblock_splice_lines={} \
             move_only_splice_return_lens={:?} forward_move_only_splice_of_req2_len={}",
            self.splice_calls,
            self.splice_src_fds,
            self.splice_dst_fds,
            self.write_calls,
            self.read_calls,
            self.agent_marker_writes,
            self.excluded_marker_writes,
            self.excluded_marker_write_tids,
            self.steady_marker_copied_by_agent,
            self.leg_fd_spliced_in_both_directions(),
            self.nonblock_splice_lines,
            self.move_only_splice_return_lens,
            self.forward_move_only_splice_of_req2_len(),
        )
    }
}

/// Split strace's leading `<tid> ` prefix (present under `-f`) into `(Some(tid),
/// body)` where `body` begins at the syscall name. A line with no leading-digit
/// prefix returns `(None, trimmed_line)`. The TID is the traced THREAD's id — for a
/// CLONE_THREAD thread it equals neither the leader pid nor a child process pid, so
/// it cleanly distinguishes the agent's in-process pump threads (members of
/// `/proc/self/task`) from the netns client's separate-process descendant.
fn split_strace_tid_prefix(line: &str) -> (Option<i32>, &str) {
    let trimmed = line.trim_start();
    let digits_end = trimmed.find(|c: char| !c.is_ascii_digit()).unwrap_or(trimmed.len());
    if digits_end == 0 {
        return (None, trimmed);
    }
    let tid = trimmed[..digits_end].parse::<i32>().ok();
    let rest = trimmed[digits_end..].trim_start();
    (tid, rest)
}

/// Recover every post-attach `CLONE_THREAD` edge `(parent_tid, child_tid)` from the
/// strace log. A thread-creating clone is `clone`/`clone3` whose flag set contains
/// `CLONE_THREAD`; the PARENT tid is the strace `-f` line prefix and the CHILD tid is
/// the clone's return value. Handles BOTH strace forms observed on the dev kernel
/// (strace 6.19):
///   - inline:  `<p> clone3({flags=...CLONE_THREAD...} ...) = <c>`
///   - split:   `<p> clone(... flags=...CLONE_THREAD... <unfinished ...>` then
///     `<p> <... clone resumed> ...) = <c>` (the flags are on the unfinished half,
///     the child tid on the resumed half; correlated by the shared parent-tid prefix
///     — clone is not re-entrant per thread, so at most one clone is outstanding per
///     parent at a time).
///
/// A process fork (the netns `ip`/`python3` client) is a `clone` WITHOUT
/// `CLONE_THREAD` (`flags=CLONE_VM|CLONE_VFORK|SIGCHLD`) → no edge emitted → the
/// forked subtree is never reachable from the test's thread group.
fn clone_thread_edges(raw: &str) -> Vec<(i32, i32)> {
    let mut edges: Vec<(i32, i32)> = Vec::new();
    // Per-parent pending CLONE_THREAD whose return value (child tid) has not yet been
    // seen — set on the `<unfinished ...>` half, cleared on the `<... resumed> ... = c`.
    let mut pending_thread_clone: std::collections::BTreeMap<i32, bool> =
        std::collections::BTreeMap::new();
    for line in raw.lines() {
        let (tid, body) = split_strace_tid_prefix(line);
        let Some(parent) = tid else { continue };
        let is_clone_start = body.starts_with("clone(") || body.starts_with("clone3(");
        let is_clone_resume = body.starts_with("<...")
            && (body.contains("clone resumed") || body.contains("clone3 resumed"));
        if !is_clone_start && !is_clone_resume {
            continue;
        }
        let has_clone_thread = body.contains("CLONE_THREAD");
        if body.contains("<unfinished ...>") {
            // Split form, first half: record whether this outstanding clone is a
            // thread clone; the child tid arrives on the resumed half.
            pending_thread_clone.insert(parent, has_clone_thread);
            continue;
        }
        // Either an inline clone (start + return on one line) or the resumed half.
        let is_thread = if is_clone_resume {
            // Resumed half: the flags lived on the unfinished half — consult pending.
            pending_thread_clone.remove(&parent).unwrap_or(false)
        } else {
            has_clone_thread
        };
        if !is_thread {
            continue;
        }
        if let Some(child) = clone_return_child_tid(body) {
            edges.push((parent, child));
        }
    }
    edges
}

/// The child TID a completed `clone`/`clone3` line returns — the integer after the
/// final `= ` on the line (e.g. `... ) = 20534` → `Some(20534)`). `body` has its
/// parent-tid prefix stripped. `None` on an error return (`= -1 EAGAIN ...`) or a
/// line with no resolved return.
fn clone_return_child_tid(body: &str) -> Option<i32> {
    let eq = body.rfind('=')?;
    let after = body[eq + 1..].trim_start();
    let end = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
    if end == 0 {
        return None; // negative / non-numeric return (error)
    }
    after[..end].parse::<i32>().ok()
}

/// The source fd of a `splice(SRC, NULL, DST, NULL, len, flags)` line — the FIRST
/// positional argument. `body` has its PID prefix stripped. `None` on a `<...
/// resumed>` fragment or a malformed line.
fn splice_source_fd(body: &str) -> Option<i32> {
    splice_arg_fd(body, 0)
}

/// The destination fd of a `splice(SRC, NULL, DST, NULL, len, flags)` line — the
/// THIRD positional argument. `body` has its PID prefix stripped. `None` on a
/// `<... resumed>` fragment or a malformed line.
fn splice_dest_fd(body: &str) -> Option<i32> {
    splice_arg_fd(body, 2)
}

/// The `index`-th comma-separated positional argument of a `splice(...)` line,
/// parsed as an fd. `None` when the argument is absent or non-numeric.
fn splice_arg_fd(body: &str, index: usize) -> Option<i32> {
    let open = body.find("splice(")? + "splice(".len();
    let args = &body[open..];
    // splice args are comma-separated: SRC, off_in, DST, off_out, len, flags
    let arg = args.split(',').nth(index)?.trim();
    let end = arg.find(|c: char| !c.is_ascii_digit()).unwrap_or(arg.len());
    arg.get(..end)?.parse::<i32>().ok()
}

/// FLAGS ORACLE (m2): the symbolic flags token (arg 5) of a
/// `splice(SRC, off_in, DST, off_out, len, flags)` line — e.g. `SPLICE_F_MOVE` or
/// `SPLICE_F_MOVE|SPLICE_F_NONBLOCK`. `body` has its PID prefix stripped. Returns the
/// leading run of `[A-Z_|]` after the 5th comma, so it stops at the closing `)` (inline
/// form), a space (before `<unfinished ...>`), or end-of-line — yielding just the flag
/// token. `None` when the 5th arg is absent OR strace rendered the flags numerically
/// (e.g. `0x1`) rather than symbolically — the LATTER is a deliberate fail-toward-guard:
/// an unrecognised numeric render yields `None`, so the flags oracle's present-and-distinct
/// control (`nonblock_splice_lines ≥ 1`) fails rather than passing vacuously.
fn splice_flags(body: &str) -> Option<&str> {
    let open = body.find("splice(")? + "splice(".len();
    let args = &body[open..];
    // splice args are comma-separated: SRC, off_in, DST, off_out, len, flags.
    let raw = args.split(',').nth(5)?.trim_start();
    let end =
        raw.find(|c: char| !(c.is_ascii_uppercase() || c == '_' || c == '|')).unwrap_or(raw.len());
    let token = &raw[..end];
    if token.is_empty() { None } else { Some(token) }
}

/// FLAGS ORACLE (m2): classify one COMPLETED splice (flags kind + parsed return) into the
/// oracle's two surfaces. A NONBLOCK splice increments the present-and-distinct control
/// (`nonblock_splice_lines`); a MOVE-only splice that moved > 0 bytes records its return
/// byte-count in `move_only_splice_return_lens` (the forward encrypt oracle keys on
/// `len(REQ2)` landing there). A return of 0 (clean EOF at teardown) or an error return
/// (`None`) records no length — those are not a steady-state move.
fn classify_splice(
    is_nonblock: bool,
    ret: Option<i64>,
    nonblock_splice_lines: &mut usize,
    move_only_splice_return_lens: &mut std::collections::BTreeSet<i64>,
) {
    if is_nonblock {
        *nonblock_splice_lines += 1;
    } else if let Some(n) = ret
        && n > 0
    {
        move_only_splice_return_lens.insert(n);
    }
}

/// FLAGS ORACLE (m2): the return byte-count of a completed inline / resumed
/// `splice(...)` line — the non-negative integer after the final `= `. `body` has its PID
/// prefix stripped. `None` on an `<unfinished ...>` fragment (no `= ret`) or a negative /
/// error return (`= -1 EAGAIN ...`). Splice args carry no `=`, so `rfind('=')` lands on
/// the return.
fn splice_return(body: &str) -> Option<i64> {
    let eq = body.rfind('=')?;
    let after = body[eq + 1..].trim_start();
    let end = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
    if end == 0 {
        return None; // negative / non-numeric return (error) or no resolved return
    }
    after[..end].parse::<i64>().ok()
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

// ============================================================================
// Pure-parser unit tests (default-lane; guard the F1 race-free TID partition).
//
// These do NOT need root / a kernel / strace — they pin the clone-tree TID partition
// (the F1 fix) as a pure function over a CAPTURED strace fixture. The fixture lines
// are REAL strace 6.19 output captured on the dev kernel (7.0) — the exact forms the
// live oracle parses. The load-bearing invariant: the CLONE_THREAD closure includes
// the (post-attach, short-lived) pump TID and EXCLUDES the netns-client fork pid.
// ============================================================================

/// A real `clone3` thread-spawn line (inline form): parent 20528 spawns thread 20533.
/// Captured verbatim from strace 6.19 on kernel 7.0.
const FIXTURE_CLONE3_THREAD: &str = "20528 clone3({flags=CLONE_VM|CLONE_FS|CLONE_FILES|CLONE_SIGHAND|CLONE_THREAD|CLONE_SYSVSEM|CLONE_SETTLS|CLONE_PARENT_SETTID|CLONE_CHILD_CLEARTID, child_tid=0xef6439aff588, parent_tid=0xef6439aff230, exit_signal=0, stack=0xef64392f0000, stack_size=0x80ea40, tls=0xef6439aff880} => {parent_tid=[20533]}, 88) = 20533";

/// A real process-fork `clone` (split across two lines): parent 20479 forks process
/// 20480 with CLONE_VFORK|SIGCHLD — NO CLONE_THREAD. This is the netns-client shape.
const FIXTURE_FORK_UNFINISHED: &str =
    "20479 clone(child_stack=0xffffdc8051a0, flags=CLONE_VM|CLONE_VFORK|SIGCHLD <unfinished ...>";
const FIXTURE_FORK_RESUMED: &str = "20479 <... clone resumed>)              = 20480";

#[test]
fn clone_tree_closure_includes_post_attach_thread_excludes_fork() {
    // Seed = the attach-time snapshot: only the parent thread 20528 pre-exists.
    let mut seed = std::collections::BTreeSet::new();
    seed.insert(20528_i32);

    // The log: a CLONE_THREAD spawn of 20533 by the in-group 20528 (the post-attach
    // short-lived pump shape), PLUS a process fork of 20480 by an UNRELATED pid 20479
    // (the netns-client shape — not in the seed, no CLONE_THREAD).
    let raw =
        format!("{FIXTURE_CLONE3_THREAD}\n{FIXTURE_FORK_UNFINISHED}\n{FIXTURE_FORK_RESUMED}\n");
    let group = TraceFindings::thread_group_closure(&raw, &seed);

    assert!(
        group.contains(&20533),
        "the CLONE_THREAD child of an in-group parent MUST join the thread group \
         (the post-attach short-lived pump). got {group:?}"
    );
    assert!(
        !group.contains(&20480),
        "the process fork (CLONE_VFORK|SIGCHLD, no CLONE_THREAD) MUST be excluded — it \
         is the netns-client subtree. got {group:?}"
    );
    assert!(group.contains(&20528), "the seed thread must remain. got {group:?}");
}

#[test]
fn clone_tree_closure_is_transitive() {
    // Seed = thread A (100). A spawns thread B (200); B spawns thread C (300). The
    // closure must reach C even though C's parent B was itself only added by the
    // closure — a fixpoint, not a single pass.
    let mut seed = std::collections::BTreeSet::new();
    seed.insert(100_i32);
    let a_spawns_b =
        "100 clone3({flags=CLONE_VM|CLONE_THREAD|CLONE_SETTLS} => {parent_tid=[200]}, 88) = 200";
    let b_spawns_c =
        "200 clone3({flags=CLONE_VM|CLONE_THREAD|CLONE_SETTLS} => {parent_tid=[300]}, 88) = 300";
    // Order the lines so the transitive edge appears BEFORE its parent is in the set,
    // to prove the fixpoint (not order-dependence).
    let raw = format!("{b_spawns_c}\n{a_spawns_b}\n");
    let group = TraceFindings::thread_group_closure(&raw, &seed);
    assert_eq!(
        group,
        [100, 200, 300].into_iter().collect::<std::collections::BTreeSet<i32>>(),
        "the closure must be transitive (A→B→C) and order-independent"
    );
}

#[test]
fn clone_thread_edges_parses_inline_and_split_thread_clones_only() {
    // Inline thread clone (edge), split process fork (NO edge).
    let raw =
        format!("{FIXTURE_CLONE3_THREAD}\n{FIXTURE_FORK_UNFINISHED}\n{FIXTURE_FORK_RESUMED}\n");
    let edges = clone_thread_edges(&raw);
    assert_eq!(
        edges,
        vec![(20528, 20533)],
        "only the CLONE_THREAD clone yields an edge; the CLONE_VFORK fork does not"
    );

    // A SPLIT thread clone (CLONE_THREAD on the unfinished half, child tid on the
    // resumed half) MUST also yield an edge — correlated by the parent-tid prefix.
    let split = "777 clone(child_stack=0xdead, flags=CLONE_VM|CLONE_THREAD|CLONE_SETTLS <unfinished ...>\n777 <... clone resumed>)              = 888\n";
    let edges = clone_thread_edges(split);
    assert_eq!(
        edges,
        vec![(777, 888)],
        "a split CLONE_THREAD clone (unfinished+resumed) yields the (parent, child) edge"
    );
}

#[test]
fn clone_return_child_tid_rejects_error_returns() {
    // A failed clone (= -1 EAGAIN ...) yields no child tid.
    assert_eq!(
        clone_return_child_tid(
            "clone3({flags=CLONE_THREAD} => {parent_tid=[0]}, 88) = -1 EAGAIN (Resource temporarily unavailable)"
        ),
        None
    );
    assert_eq!(clone_return_child_tid("<... clone resumed>)              = 888"), Some(888));
}

// =====================================================================
// FLAGS ORACLE (m2) pure-parser unit tests (default-lane; guard the flags-arg + return
// extraction the runtime oracle keys on). Same discipline as the clone-tree parser tests
// above: no root / kernel / strace needed — the fixture lines are the exact splice shapes
// strace 6.19 renders on the dev kernel (SPLICE_F_MOVE for the blocking encrypt pump,
// SPLICE_F_MOVE|SPLICE_F_NONBLOCK for the NONBLOCK decrypt pump).
// =====================================================================

#[test]
fn splice_flags_distinguishes_move_only_from_nonblock() {
    // Blocking encrypt-pump splice: MOVE-only.
    assert_eq!(
        splice_flags("splice(14, NULL, 20, NULL, 65536, SPLICE_F_MOVE) = 88"),
        Some("SPLICE_F_MOVE"),
    );
    // NONBLOCK decrypt-pump splice: the piped flag token is kept whole (stops at ')').
    assert_eq!(
        splice_flags("splice(18, NULL, 22, NULL, 65536, SPLICE_F_MOVE|SPLICE_F_NONBLOCK) = 86"),
        Some("SPLICE_F_MOVE|SPLICE_F_NONBLOCK"),
    );
    // The `<unfinished ...>` half of a split blocking splice: flags present, token stops
    // at the space before `<unfinished`.
    assert_eq!(
        splice_flags("splice(19, NULL, 16, NULL, 88, SPLICE_F_MOVE <unfinished ...>"),
        Some("SPLICE_F_MOVE"),
    );
    // A numeric flags render (strace not decoding the flag symbolically) yields None — the
    // deliberate fail-toward-guard: the present-and-distinct control then fails rather than
    // passing vacuously.
    assert_eq!(splice_flags("splice(14, NULL, 20, NULL, 65536, 0x1) = 88"), None);
    // The MOVE-only vs NONBLOCK discrimination the oracle depends on.
    assert!(
        !splice_flags("splice(14, NULL, 20, NULL, 65536, SPLICE_F_MOVE) = 88")
            .unwrap()
            .contains("SPLICE_F_NONBLOCK")
    );
    assert!(
        splice_flags("splice(18, NULL, 22, NULL, 65536, SPLICE_F_MOVE|SPLICE_F_NONBLOCK) = 86")
            .unwrap()
            .contains("SPLICE_F_NONBLOCK")
    );
}

#[test]
fn splice_return_reads_inline_and_rejects_unfinished_and_errors() {
    // Inline completed splice: the byte count after the final `= `.
    assert_eq!(splice_return("splice(14, NULL, 20, NULL, 65536, SPLICE_F_MOVE) = 88"), Some(88),);
    // The `<unfinished ...>` half carries no `= ret` → None (its return lands on the
    // resumed half, which `names("splice(")` does not match).
    assert_eq!(
        splice_return("splice(19, NULL, 16, NULL, 88, SPLICE_F_MOVE <unfinished ...>"),
        None,
    );
    // A negative / error return yields None (not a successful move).
    assert_eq!(
        splice_return(
            "splice(14, NULL, 20, NULL, 65536, SPLICE_F_MOVE) = -1 EAGAIN (Resource temporarily unavailable)"
        ),
        None,
    );
}

#[test]
fn parse_flags_oracle_recognises_forward_move_only_and_nonblock_control() {
    let empty_group = std::collections::BTreeSet::new();
    let req2_len = OUTBOUND_REQUEST2.len();

    // A realistic phase-2 window in the REAL capture shape: the pumps' splices BLOCK, so
    // strace splits each into an `<unfinished ...>` half (flags) and a `<... splice
    // resumed> ... = <ret>` half (return), interleaved across the two pump tids. The
    // FORWARD encrypt pump (tid 20530) splices len(REQ2) into leg-B's kTLS-TX with
    // SPLICE_F_MOVE only (its legF→pipe splice-in and pipe→kTLS-TX splice-out each return
    // len(REQ2)); the RETURN decrypt pump (tid 20531) splices the 86-byte RESPONSE out of
    // leg-B's kTLS-RX with SPLICE_F_MOVE|SPLICE_F_NONBLOCK. A trailing inline `= 0` models
    // the teardown EOF read (MOVE-only, 0 bytes → not a steady-state move).
    let blocking = format!(
        "20530 splice(14, NULL, 20, NULL, 65536, SPLICE_F_MOVE <unfinished ...>\n\
         20531 splice(16, NULL, 22, NULL, 65536, SPLICE_F_MOVE|SPLICE_F_NONBLOCK <unfinished ...>\n\
         20530 <... splice resumed>) = {req2_len}\n\
         20531 <... splice resumed>) = 86\n\
         20530 splice(19, NULL, 16, NULL, {req2_len}, SPLICE_F_MOVE <unfinished ...>\n\
         20531 splice(21, NULL, 14, NULL, 86, SPLICE_F_MOVE|SPLICE_F_NONBLOCK <unfinished ...>\n\
         20530 <... splice resumed>) = {req2_len}\n\
         20531 <... splice resumed>) = 86\n\
         20530 splice(14, NULL, 20, NULL, 65536, SPLICE_F_MOVE) = 0\n"
    );
    let ok = TraceFindings::parse(&blocking, &empty_group);
    assert!(
        ok.forward_move_only_splice_of_req2_len(),
        "a MOVE-only splice returning len(REQ2) must be recognised as the blocking forward \
         encrypt (correlated across the unfinished/resumed split); got {:?}",
        ok.move_only_splice_return_lens
    );
    assert!(
        ok.nonblock_splice_lines >= 1,
        "the RETURN decrypt pump's MOVE|NONBLOCK splices must be counted by the control; got {}",
        ok.nonblock_splice_lines
    );

    // The REGRESSION shape: the forward pump (tid 20530) copied the decrypt pump's flags —
    // its splices now carry SPLICE_F_MOVE|SPLICE_F_NONBLOCK. NO MOVE-only splice returns
    // len(REQ2), so the oracle flips false (it does not merely "miss" — the round-trip
    // would still be green). The NONBLOCK control STILL passes (it is a control, present in
    // both cases).
    let regressed = format!(
        "20530 splice(14, NULL, 20, NULL, 65536, SPLICE_F_MOVE|SPLICE_F_NONBLOCK <unfinished ...>\n\
         20530 <... splice resumed>) = {req2_len}\n\
         20530 splice(19, NULL, 16, NULL, {req2_len}, SPLICE_F_MOVE|SPLICE_F_NONBLOCK <unfinished ...>\n\
         20530 <... splice resumed>) = {req2_len}\n\
         20531 splice(16, NULL, 22, NULL, 65536, SPLICE_F_MOVE|SPLICE_F_NONBLOCK <unfinished ...>\n\
         20531 <... splice resumed>) = 86\n"
    );
    let bad = TraceFindings::parse(&regressed, &empty_group);
    assert!(
        !bad.forward_move_only_splice_of_req2_len(),
        "when the forward pump regresses to MOVE|NONBLOCK, no MOVE-only splice returns \
         len(REQ2) and the oracle must flip false; move_only lens = {:?}",
        bad.move_only_splice_return_lens
    );
    assert!(
        bad.nonblock_splice_lines >= 1,
        "the NONBLOCK control is present in the regressed shape too (it is a control, not the \
         signal); got {}",
        bad.nonblock_splice_lines
    );
}
