//! Tier-3 NAME → RESOLVE → ENFORCE CONSISTENCY (step 05-02) — the SINGLE-SOURCE
//! invariant for ADR-0071 § Enforcement Tier-3 obligation (e) / Q5a / D-TME-9 /
//! D-TME-10: **a DNS-returned `service_backends` addr IS the addr `MtlsResolve`
//! recognizes** — one source, two readers, byte-consistent.
//!
//! The whole point: when the #243 responder lands it will return a
//! `service_backends` addr (headless v1, D-TME-10 — a `running` backend addr,
//! NOT a VIP); `MtlsResolve` ALSO reads a `service_backends` addr; there is NO
//! translation layer between them (feature-delta § "DNS-return contract
//! alignment"; `ResolvedBackend::addr` rustdoc verbatim: "the same
//! `service_backends` addr DNS returned — one source, two readers"). So the addr
//! a captured connection's `getsockname` recovers (== the addr the workload
//! dialed == what DNS would return) is byte-identical to the addr the resolve
//! port recognizes as a `running` mesh backend.
//!
//! ## DNS is STUBBED — the responder daemon (#243) is NOT built here
//!
//! Per ADR-0071 § Enforcement obligation (e) "Until #243's responder lands, this
//! is exercised with the DNS step stubbed." The workload connects to a KNOWN
//! `service_backends` addr **B** directly (the `getaddrinfo → connect(B)` DNS
//! step is the #243 stub), so the resolve-recognizes-orig_dst half is validated
//! INDEPENDENTLY of the responder. SCOPE GUARDRAIL (hard): this step builds NO
//! #243 DNS responder daemon and NO #167 VIP allocator. Headless v1 only.
//!
//! ## What this AT proves (the single-source consistency oracle)
//!
//!   1. **Capture + recover** — the netns workload's `connect(B)` ingresses
//!      vethH → PREROUTING → egress nft-TPROXY redirect → leg-F (IP_TRANSPARENT)
//!      → the PRODUCTION `accept_outbound_and_recover_orig_dst` recovers orig_dst
//!      via `getsockname`. The recovered orig_dst == **B** (the known
//!      `service_backends` addr the workload dialed; a wrong recovery would
//!      classify the wrong arm). This is the 03-03 / 05-01 capture half, reused.
//!   2. **Resolve recognizes the SAME addr** — feed the `getsockname`-recovered
//!      **B** to `SimMtlsResolve.resolve(B)` (scripted with B → `Mesh`) and it
//!      returns `Mesh(ResolvedBackend { addr: B, expected_svid: None })`. The
//!      addr the resolve port returns IS byte-identical to the captured orig_dst
//!      — the single-source invariant. (`SimMtlsResolve` stands in for the v1
//!      `ServiceBackendsResolve` host adapter; the production resolve index 01-03
//!      is its own DST's job — here the C1 contract is "the addr the capture
//!      recovers is the addr resolve recognizes", not the index internals.)
//!   3. **resolv.conf path/bind-mount convention surfaces the line** — the
//!      per-netns `/etc/netns/<netns>/resolv.conf` carries a byte-exact
//!      `nameserver <responder>` line (the D-TME-9 line shape,
//!      `resolv_conf_contents(responder)`) AND that line is visible inside the
//!      workload's namespace view via the stock iproute2 per-netns bind-mount.
//!      This oracle exercises the PATH/BIND-MOUNT plumbing the capture relies on —
//!      NOT the production injection mechanism itself: it STAGES the file with
//!      `std::fs::write` (the production writer `resolv_conf_write` is private to
//!      `veth_provisioner` and runs at the action-shim alloc lifecycle, upstream of
//!      the worker leg this AT drives). The production injection MECHANISM is
//!      proven, strictly more strongly, by 02-03's
//!      `provision_injects_node_local_responder_into_netns_resolv_conf` (drives
//!      production `provision_workload_netns` → `resolv_conf_write` with a
//!      non-vacuous host-negative assertion) — cited here, not re-proven. The
//!      responder the line points at is the #243 stub (no daemon built).
//!
//! ## Authn-only boundary (Q4 / #242)
//!
//! `expected_svid` stays `None` for the resolved backend (v1 authn-only; the
//! expected-SVID join is #242). This AT asserts the addr is recognized (the
//! single-source invariant) — it MUST NOT assert intended-peer "protection" /
//! `expected_peer` (None until #242), identical authn-only discipline to 05-01's
//! last criterion.
//!
//! ## Kernel-free cheap reproduction (the genuine default-lane pair)
//!
//! The cheap, kernel-free reproduction this step's criterion describes — script
//! addr B, feed the same B as orig_dst, assert `Mesh(ResolvedBackend { addr: B,
//! .. })` — lives in the GENUINE default lane as the unit test
//! `sim_mtls_resolve_returns_scripted_arm_per_orig_dst` in
//! `crates/overdrive-sim/src/adapters/mtls_resolve.rs` (that crate's unit tests run
//! under a plain `cargo nextest run` — this is the canonical default-lane
//! reproduction the step's criterion is satisfied by). This file ADDITIONALLY
//! carries an in-binary `single_source_invariant_holds_kernel_free_via_sim_mtls_resolve`
//! mirror (below) that echoes THIS step's exact single-source scenario with
//! `SimMtlsResolve` and no netns/root — kernel-free but NOT default-lane (it is
//! gated with the rest of this integration file, `--features integration-tests`);
//! it is the in-binary pair to the kernel AT, not the default-lane coverage.
//!
//! Requires root + CAP_NET_ADMIN/CAP_SYS_ADMIN (IP_TRANSPARENT, nft, ip netns,
//! ip rule, writing `/etc/netns/`). A non-root run SKIPs. Run via
//! `cargo xtask lima run -- cargo nextest run -p overdrive-worker
//! --features integration-tests`. NEVER `--no-run` (a compile-only gate is green
//! even when every fixture refuses at boot). `uname -r` is recorded.
//!
//! Hygiene: the shared `overdrive-mtls` routing infra PERSISTS by design
//! (node-global converge-on-boot), so the test scrubs ALL `overdrive-mtls` nft
//! state + the fwmark rule/route + the test netns/veth/lo-addr + the per-netns
//! `/etc/netns/<netns>/` dir at START (tolerate pre-existing) AND END. A
//! cross-PROCESS `flock(2)` lock (`KernelStateLock`, on the SAME path the sibling
//! kernel-touching suites use) serialises the kernel-touching tests — nextest
//! runs each `#[test]` in a separate process, so an in-process lock cannot
//! serialise node-global state.

#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::too_many_lines,
    clippy::match_wildcard_for_single_variants,
    clippy::format_collect,
    reason = "Tier-3 single-source-consistency test body; the numbered oracle list in the module docstring is a narrative; skip messages + evidence go to stderr; failures must panic with informative messages; the SocketAddr wildcard arm is the V6 case a v4-only fixture cannot hit; the single scenario is a long composed proof; the per-byte \\xNN python-literal fold reads clearer than a write! accumulator in a test fixture"
)]

use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::os::fd::AsRawFd as _;
use std::process::{Command, Stdio};
use std::time::Duration;

use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve, ResolvedBackend};
use overdrive_sim::adapters::SimMtlsResolve;
use overdrive_worker::mtls_intercept::{
    accept_outbound_and_recover_orig_dst, install_outbound_tproxy, make_transparent_listener,
};

// ============================================================================
// topology constants (mirror the increment-b egress spike + egress_tproxy_capture)
// ============================================================================

const NS_W: &str = "nsW-cons0502";
const VETH_W: &str = "vethW-con05";
const VETH_H: &str = "vethH-con05";
const HOST_GW: &str = "10.99.0.1";
const WL_ADDR: &str = "10.99.0.2";
const SUBNET_LEN: &str = "24";

/// The KNOWN `service_backends` addr **B** the workload dials (DNS stubbed — the
/// workload connects to it directly, standing in for the #243
/// `getaddrinfo → connect(B)` step). A host-side lo-bound addr the workload
/// routes to via the gateway, so its egress genuinely INGRESSES vethH and hits
/// PREROUTING. This is the addr the capture's `getsockname` recovers AND the addr
/// `MtlsResolve.resolve` recognizes — the single source, two readers.
const SERVICE_BACKEND_IP: &str = "10.200.0.1";
const SERVICE_BACKEND_PORT: u16 = 18821;

/// The node-local DNS responder addr written into the netns resolv.conf (the #243
/// stub the injected `nameserver` line points at — the daemon itself is NOT built
/// here). A plausible Fly-style node-local responder address.
const RESPONDER_ADDR: Ipv4Addr = Ipv4Addr::new(10, 100, 0, 53);

/// The application bytes the workload sends after connect. Their PRESENCE is the
/// positive interception signal (debugging.md §11): the netns client prints
/// `WL-SENT` on a successful connect+send, confirming the producer (the dial) ran.
/// The bytes are NOT echoed/compared in this topology — the egress redirect starves
/// the real backend (it never accepts), so there is no reader to compare them
/// against; the dial's SUCCESS, not a byte-exact echo, is what the oracle asserts.
const WL_MARKER: &[u8] = b"OVERDRIVE_0502_SINGLE_SOURCE_workload_dialed_B";

// ============================================================================
// Cross-process kernel-state exclusion (shared path with the sibling suites)
// ============================================================================

/// Cross-PROCESS exclusion for the shared host-netns kernel state. The
/// `overdrive-mtls` nft table, the fwmark ip-rule, and the table-100 local route
/// are NODE-GLOBAL. nextest runs each `#[test]` in a SEPARATE PROCESS, so an
/// in-process lock cannot serialise them — an `flock(2)` on the fixed path
/// (shared with `egress_tproxy_capture.rs` / `bidirectional_walking_skeleton.rs`)
/// spans processes.
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

/// True iff this process is uid 0 (root). IP_TRANSPARENT, nft, `ip netns`,
/// `ip rule`, and writing `/etc/netns/` all need root + CAP_NET_ADMIN/
/// CAP_SYS_ADMIN; a non-root run cannot stand up the fixture, so we SKIP.
fn is_root() -> bool {
    // SAFETY: getuid is always safe; takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

fn service_backend_addr() -> SocketAddrV4 {
    SocketAddrV4::new(SERVICE_BACKEND_IP.parse().expect("service backend ip"), SERVICE_BACKEND_PORT)
}

// ============================================================================
// command shims (mirror egress_tproxy_capture.rs)
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

/// The host-side per-netns resolv.conf dir (`/etc/netns/<netns>/`) and file —
/// the stock `ip netns` per-netns convention bind-mounted over `/etc/resolv.conf`
/// inside the namespace. The path convention MIRRORS `veth_provisioner`'s private
/// `resolv_conf_dir`/`resolv_conf_path` (the SAME convention — D-TME-9). The test
/// STAGES the file with `std::fs::write` (see `inject_resolv_conf`); it does NOT
/// invoke the production injection — the writer `resolv_conf_write` is private and
/// runs at the action-shim alloc lifecycle, upstream of the worker leg this AT
/// drives (feature-delta C3). Oracle 3 therefore exercises the path/bind-mount
/// convention, not the production mechanism — which 02-03's
/// `provision_injects_node_local_responder_into_netns_resolv_conf` proves directly.
fn resolv_conf_dir() -> String {
    format!("/etc/netns/{NS_W}")
}

fn resolv_conf_path() -> String {
    format!("{}/resolv.conf", resolv_conf_dir())
}

/// The byte-exact body the 02-03 `veth_provisioner::resolv_conf_contents`
/// produces for `responder` — a single `nameserver <responder>` line with a
/// trailing newline (D-TME-9 / Q5a, the Fly.io `fdaa::3` model). This is the
/// OBSERVABLE injection contract, asserted directly here rather than imported:
/// the production injection writer `resolv_conf_write` (and the `resolv_conf_dir` /
/// `resolv_conf_path` helpers) are PRIVATE to `veth_provisioner`; only the pure
/// `resolv_conf_contents` is `pub`, so the worker test tree cannot drive the
/// production injection regardless of the dependency graph. The line shape is the
/// stable D-TME-9 wire contract (not internal logic), so asserting it verbatim is
/// honest — `resolv_conf_write` writes exactly this body. SSOT for the shape is
/// `veth_provisioner::resolv_conf_contents`, pinned independently by the
/// `resolv_conf_contents_is_a_single_nameserver_line` unit test; keep this mirror
/// in sync (a production format change reddens that test, alerting the maintainer).
fn resolv_conf_contents(responder: Ipv4Addr) -> String {
    format!("nameserver {responder}\n")
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

/// Tear down the per-test netns + veth pair + lo-bound service backend + the
/// per-netns resolv.conf dir. The shared `overdrive-mtls` infra is handled by
/// `clean_shared_infra`.
fn teardown_topology() {
    ip_quiet(&["link", "del", VETH_H]);
    ip_quiet(&["netns", "del", NS_W]);
    ip_quiet(&["addr", "del", &format!("{SERVICE_BACKEND_IP}/32"), "dev", "lo"]);
    // The per-netns resolv.conf dir is a host-side /etc dir NOT reaped by
    // `ip netns del` (veth_provisioner § teardown reaps it explicitly); remove it
    // so a re-run starts clean (an absent dir is benign).
    let _ = std::fs::remove_dir_all(resolv_conf_dir());
}

/// Stand up the netns + veth pair + addresses + host routing hygiene EXACTLY as
/// the increment-b egress spike does, plus the lo-bound service backend B the
/// workload dials.
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

    // The KNOWN service backend B lives on host lo (the host binds+listens on it;
    // the workload routes to it via the gateway).
    ip(&["addr", "add", &format!("{SERVICE_BACKEND_IP}/32"), "dev", "lo"]);

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

/// STAGE the per-netns resolv.conf for Oracle 3: create `/etc/netns/<netns>/` and
/// write the byte-exact D-TME-9 line `resolv_conf_contents(RESPONDER_ADDR)`
/// (`"nameserver <responder>\n"`) with `std::fs::write`. This is a TEST FIXTURE
/// stand-in, NOT the production injection: the production writer `resolv_conf_write`
/// is private and runs at the action-shim alloc lifecycle, upstream of the worker
/// leg this AT drives (feature-delta C3). Oracle 3 then asserts the path/bind-mount
/// convention surfaces this line inside the netns view; the production injection
/// MECHANISM is proven by 02-03's
/// `provision_injects_node_local_responder_into_netns_resolv_conf`. The responder
/// the line points at is the #243 stub (no daemon built).
fn inject_resolv_conf() {
    std::fs::create_dir_all(resolv_conf_dir()).expect("create per-netns resolv.conf dir");
    std::fs::write(resolv_conf_path(), resolv_conf_contents(RESPONDER_ADDR)).expect(
        "stage per-netns resolv.conf fixture (test stand-in; production injection is the private \
         resolv_conf_write, proven by 02-03)",
    );
}

/// Run a `/dev/tcp` client INSIDE the workload netns: connect to `dst`, send
/// `marker`. The DNS step is STUBBED — the workload connects to the KNOWN
/// `service_backends` addr B directly (standing in for `getaddrinfo → connect`).
/// Returns the client's exit/stdout/stderr summary.
fn run_client_in_netns(dst: SocketAddrV4, marker: &[u8]) -> String {
    let req_literal: String = marker.iter().map(|b| format!("\\x{b:02x}")).collect();
    let script = format!(
        "\
import socket,sys
s=socket.socket(socket.AF_INET,socket.SOCK_STREAM)
s.settimeout(8)
try:
    s.connect(('{ip}',{port}))
    s.sendall(b'{req}')
    print('WL-SENT')
except Exception as e:
    print('CLIENT-FAIL:'+str(e))
",
        ip = dst.ip(),
        port = dst.port(),
        req = req_literal,
    );
    let out = Command::new("ip")
        .args(["netns", "exec", NS_W, "python3", "-c", &script])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    match out {
        Ok(o) => format!(
            "[exit={:?}] stdout={} stderr={}",
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).trim(),
            String::from_utf8_lossy(&o.stderr).trim()
        ),
        Err(e) => format!("spawn client failed: {e}"),
    }
}

/// Bound a blocking `accept()` on `listener` to `timeout` via `SO_RCVTIMEO` so
/// the PRODUCTION `accept_outbound_and_recover_orig_dst`'s internal blocking
/// accept returns a clean error after a bounded wait instead of hanging to
/// nextest's 120 s slow-timeout SIGKILL on a silent redirect failure (mirrors
/// `egress_tproxy_capture.rs::bound_listener_accept`). The production API is
/// UNCHANGED — this is a test-side socket option applied before handing the
/// listener to the production fn.
fn bound_listener_accept(listener: &TcpListener, timeout: Duration) {
    let tv = libc::timeval {
        tv_sec: timeout.as_secs() as libc::time_t,
        tv_usec: libc::suseconds_t::from(timeout.subsec_micros()),
    };
    // SAFETY: listener owns a live socket fd; SO_RCVTIMEO takes a `timeval` of the
    // size passed. A non-zero return is a best-effort failure (the bound is not
    // load-bearing for correctness, only the diagnostic failure shape).
    let rc = unsafe {
        libc::setsockopt(
            listener.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            std::ptr::from_ref(&tv).cast(),
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        eprintln!(
            "[05-02] warn: SO_RCVTIMEO on leg-F listener failed ({}); production accept may \
             hang to slow-timeout on a silent redirect failure",
            std::io::Error::last_os_error()
        );
    }
}

/// Run the async `resolve` on a fresh current-thread runtime — the `#[test]` body
/// is sync and cannot `.await` (mirrors the sim crate's `block_on` shape).
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("current-thread runtime builds")
        .block_on(fut)
}

// ============================================================================
// THE deliverable scenario (ADR-0071 Tier-3 obligation (e), Q5a, D-TME-10)
// ============================================================================

/// THE single-source invariant (ADR-0071 Tier-3 obligation (e)): a DNS-returned
/// `service_backends` addr B is what `getsockname` recovers from the captured
/// connection AND what `MtlsResolve.resolve` recognizes — one source, two
/// readers, byte-consistent. DNS STUBBED (the workload dials B directly; #243's
/// responder is not built). The resolv.conf injection (02-03) is asserted present
/// in the netns even though the responder is the #243 stub.
#[test]
fn dns_returned_service_backends_addr_is_recognized_by_mtls_resolve() {
    if !is_root() {
        eprintln!(
            "SKIP dns_returned_service_backends_addr_is_recognized_by_mtls_resolve: not root"
        );
        return;
    }

    // Pin the verdict to a kernel (spike.md discipline).
    let kr = Command::new("uname")
        .arg("-r")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    eprintln!("[05-02] uname -r = {kr}");

    // Cross-process exclusion + clean baseline.
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();
    setup_topology();

    // The single source: the KNOWN `service_backends` addr B. DNS would return
    // it (headless v1, D-TME-10); the workload dials it directly (DNS stubbed).
    let b = service_backend_addr();

    // ----------------------------------------------------------------
    // Oracle 3 (resolv.conf path/bind-mount convention, D-TME-9 line shape): STAGE
    // the per-netns resolv.conf (test fixture, NOT the production writer) and assert
    // it carries the byte-exact `nameserver <responder>` line AND that the stock
    // iproute2 per-netns bind-mount surfaces it inside the workload's namespace
    // view. This exercises the path/bind-mount plumbing the capture relies on — NOT
    // the production injection mechanism (private `resolv_conf_write`, proven by
    // 02-03's provision_injects_node_local_responder_into_netns_resolv_conf). The
    // responder the line points at is the #243 stub (no daemon built).
    // ----------------------------------------------------------------
    inject_resolv_conf();
    let injected = std::fs::read_to_string(resolv_conf_path())
        .expect("the per-netns resolv.conf must be readable after staging");
    let want_line = resolv_conf_contents(RESPONDER_ADDR);
    assert_eq!(
        injected, want_line,
        "Oracle 3: the staged per-netns /etc/netns/{NS_W}/resolv.conf must carry the byte-exact \
         `nameserver {RESPONDER_ADDR}` line (the D-TME-9 line shape; the production injection \
         MECHANISM is proven by 02-03's provision_workload_netns test), got {injected:?}"
    );
    // And the namespace bind-mounts it: `ip netns exec` reading /etc/resolv.conf
    // inside the netns sees the SAME injected line (the stock per-netns
    // convention; the injection is observable from WITHIN the workload's view).
    let in_ns = Command::new("ip")
        .args(["netns", "exec", NS_W, "cat", "/etc/resolv.conf"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    eprintln!("[05-02][Oracle 3] in-netns /etc/resolv.conf = {in_ns:?}");
    assert!(
        in_ns.contains(&format!("nameserver {RESPONDER_ADDR}")),
        "Oracle 3: the workload's in-netns /etc/resolv.conf (bind-mounted from the per-netns file) \
         must carry the injected `nameserver {RESPONDER_ADDR}` line, got {in_ns:?}"
    );

    // ----------------------------------------------------------------
    // Oracle 1 (capture + recover): install the egress nft-TPROXY rule, the
    // workload dials B (DNS stubbed), the PRODUCTION accept recovers orig_dst via
    // getsockname. The recovered orig_dst MUST equal B.
    // ----------------------------------------------------------------
    // leg-F MUST be IP_TRANSPARENT (TPROXY delivers orig-dst-addressed packets).
    let leg_f = make_transparent_listener(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("make_transparent_listener leg-F");
    let leg_f_port = match leg_f.local_addr().expect("leg-F local_addr") {
        std::net::SocketAddr::V4(a) => a.port(),
        other => panic!("expected V4 leg-F addr, got {other}"),
    };

    let guard = install_outbound_tproxy(VETH_H, leg_f_port)
        .expect("install_outbound_tproxy must append the iifname egress rule + shared infra");
    let dump = nft_dump_table();
    eprintln!("[05-02] nft table after install_outbound_tproxy:\n{dump}");
    assert!(
        dump.contains(&format!("iifname \"{VETH_H}\"")) && dump.contains("tproxy to"),
        "the iifname egress rule must be installed in the shared chain, got:\n{dump}"
    );

    // A real backend on B so the captured dial genuinely landed (POSITIVE
    // interception signal): if the redirect FAILED to fire, the dial would land
    // here instead of leg-F. Non-blocking so a no-arrival is observable.
    let backend = TcpListener::bind(b).expect("bind real service backend B");
    backend.set_nonblocking(true).ok();

    // The workload dials B directly (DNS stubbed — the #243 getaddrinfo→connect
    // step). Its egress ingresses vethH → PREROUTING → egress TPROXY → leg-F.
    let client = std::thread::spawn(move || run_client_in_netns(b, WL_MARKER));

    // Drive the PRODUCTION getsockname recovery on the TPROXY-intercepted leg-F
    // socket. Bound the internal blocking accept so a silent redirect failure
    // clean-fails after 8 s instead of hanging to the 120 s slow-timeout SIGKILL.
    bound_listener_accept(&leg_f, Duration::from_secs(8));
    let (leg, recovered) = accept_outbound_and_recover_orig_dst(&leg_f).expect(
        "accept_outbound_and_recover_orig_dst must recover orig_dst from the TPROXY redirect. A \
         clean error here (EAGAIN/timeout after 8 s) means the redirect did NOT deliver to leg-F \
         — egress capture did not fire (the dial reached the real backend instead). If this HANGS \
         instead, SO_RCVTIMEO did not bound the production accept on this kernel; treat a 120 s \
         slow-timeout SIGKILL as the same redirect-did-not-fire signal.",
    );

    // Oracle 1: the redirect fired (leg-F accepted, NOT the real backend) AND the
    // recovered orig_dst == B (the known service_backends addr the workload
    // dialed). This is the addr DNS would have returned (headless v1, stubbed).
    eprintln!("[05-02][Oracle 1] getsockname-recovered orig_dst = {recovered}");
    eprintln!("[05-02][Oracle 1] dialed service_backends addr B  = {b}");
    assert_eq!(
        recovered, b,
        "Oracle 1: getsockname-recovered orig_dst must equal the dialed service_backends addr B \
         (the headless-v1 addr DNS would return — one source) {b}"
    );
    assert_ne!(
        recovered.port(),
        leg_f_port,
        "Oracle 1: recovered orig_dst port must be B's port, NOT leg-F's bound port"
    );
    drop(leg);

    // POSITIVE interception signal (debugging.md §11): confirm the workload's
    // dial genuinely happened (it connected; a CLIENT-FAIL would mean no dial) and
    // that the real backend did NOT accept (the redirect took it to leg-F).
    let client_out = client.join().expect("netns client thread");
    eprintln!("[05-02][Oracle 1] netns client: {client_out}");
    assert!(
        client_out.contains("WL-SENT"),
        "Oracle 1 POSITIVE signal: the workload's connect+send to B must SUCCEED (a CLIENT-FAIL \
         means the dial never happened, so the capture proved nothing), got {client_out}"
    );
    assert!(
        backend.accept().is_err(),
        "Oracle 1: the redirect fired — the real service backend B must NOT have accepted the \
         workload's dial (it was redirected to leg-F)"
    );
    drop(backend);

    // ----------------------------------------------------------------
    // Oracle 2 (THE single-source invariant): feed the SAME getsockname-recovered
    // B to MtlsResolve.resolve — it recognizes it as the SAME `running` mesh
    // backend, returning `Mesh(ResolvedBackend { addr: B, expected_svid: None })`.
    // The addr resolve returns IS byte-identical to the captured orig_dst — one
    // source (service_backends), two readers (DNS-return and resolve), no
    // translation. SimMtlsResolve stands in for the v1 ServiceBackendsResolve
    // adapter (the production resolve index 01-03 is its own DST's job).
    // ----------------------------------------------------------------
    let mut scripted = std::collections::BTreeMap::new();
    scripted.insert(b, MtlsResolution::Mesh(ResolvedBackend { addr: b, expected_svid: None }));
    let resolve = SimMtlsResolve::new(scripted, MtlsResolution::NonMesh);

    let resolution =
        block_on(resolve.resolve(recovered)).expect("resolve of the recovered orig_dst is Ok");
    eprintln!("[05-02][Oracle 2] resolve(recovered orig_dst {recovered}) = {resolution:?}");
    assert_eq!(
        resolution,
        MtlsResolution::Mesh(ResolvedBackend { addr: b, expected_svid: None }),
        "Oracle 2 (single-source invariant): MtlsResolve must recognize the getsockname-recovered \
         orig_dst as the SAME `running` mesh backend B — `Mesh(ResolvedBackend {{ addr: B, \
         expected_svid: None }})`. The addr resolve returns IS byte-identical to the captured \
         orig_dst (one source, two readers); expected_svid stays None (v1 authn-only, #242)."
    );
    // The addr the resolve port returns is byte-identical to the captured orig_dst
    // — stated explicitly so the single-source invariant is the load-bearing
    // assertion, not an incidental consequence of the arm equality above.
    if let MtlsResolution::Mesh(backend) = &resolution {
        assert_eq!(
            backend.addr, recovered,
            "Oracle 2 (single-source invariant): the resolved backend addr must be byte-identical \
             to the getsockname-recovered orig_dst — no VIP→backend translation in headless v1 \
             (D-TME-10), one source two readers"
        );
        assert!(
            backend.expected_svid.is_none(),
            "authn-only boundary (Q4 / #242): v1 expected_svid stays None — this AT asserts the \
             addr is RECOGNIZED, never intended-peer protection"
        );
    } else {
        panic!("Oracle 2: expected a Mesh arm for the recognized backend B, got {resolution:?}");
    }

    eprintln!(
        "[05-02] VERDICT: WORKS — single-source invariant validated on kernel {kr}: the DNS-stubbed \
         service_backends addr B the workload dialed == the getsockname-recovered orig_dst == the \
         addr MtlsResolve recognizes as a running mesh backend (one source, two readers, \
         byte-consistent). resolv.conf injection (02-03) wired; authn-only (expected_svid None)."
    );

    // Teardown: drop the per-workload guard (removes ONLY the iifname rule), then
    // scrub the shared infra + topology + resolv.conf dir so a re-run reproduces.
    drop(guard);
    drop(leg_f);
    teardown_topology();
    clean_shared_infra();
}

// ============================================================================
// Default-lane cheap reproduction (kernel-free, pairs with the Tier-3 AT)
// ============================================================================

/// The kernel-free in-binary mirror (ADR-025 RED_UNIT): mirror THIS step's exact
/// single-source scenario with `SimMtlsResolve` and NO netns/root. Script addr
/// B → `Mesh(ResolvedBackend { addr: B })`, feed the SAME B as the orig_dst (the
/// addr the capture would have recovered), and assert resolve recognizes it
/// byte-identically. This is kernel-free but NOT default-lane — it is gated with
/// the rest of this integration file (`--features integration-tests`), so it runs
/// only under that feature, not a plain `cargo nextest run`. The GENUINE
/// default-lane cheap reproduction is the canonical
/// `sim_mtls_resolve_returns_scripted_arm_per_orig_dst` in
/// `crates/overdrive-sim/src/adapters/mtls_resolve.rs`; this mirror is the
/// in-binary pair to the kernel AT, not the default-lane coverage.
#[tokio::test]
async fn single_source_invariant_holds_kernel_free_via_sim_mtls_resolve() {
    // B is the single source: the addr DNS would return AND the addr the capture
    // recovers AND the addr resolve recognizes. Here there is no kernel capture —
    // we feed the SAME B as the orig_dst to model the one-source-two-readers
    // consistency the Tier-3 AT proves end-to-end.
    let b = service_backend_addr();
    let mut scripted = std::collections::BTreeMap::new();
    scripted.insert(b, MtlsResolution::Mesh(ResolvedBackend { addr: b, expected_svid: None }));
    let resolve = SimMtlsResolve::new(scripted, MtlsResolution::NonMesh);

    // The orig_dst the (stubbed) capture would have recovered IS B — same source.
    let recovered = b;
    let resolution = resolve.resolve(recovered).await.expect("resolve of B is Ok");

    assert_eq!(
        resolution,
        MtlsResolution::Mesh(ResolvedBackend { addr: b, expected_svid: None }),
        "single-source invariant (kernel-free): resolve recognizes the recovered orig_dst as the \
         SAME running backend B, byte-identical addr, expected_svid None (v1 authn-only)"
    );
    // The byte-identical-addr assertion stated explicitly (the single-source point).
    let MtlsResolution::Mesh(backend) = resolution else {
        panic!("expected a Mesh arm for B");
    };
    assert_eq!(
        backend.addr, recovered,
        "single-source invariant: the resolved backend addr is byte-identical to the recovered \
         orig_dst — one source (service_backends), two readers (DNS-return + resolve)"
    );
    assert!(backend.expected_svid.is_none(), "v1 authn-only: expected_svid stays None (#242)");
}
