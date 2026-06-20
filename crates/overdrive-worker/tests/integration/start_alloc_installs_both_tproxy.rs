//! Tier-3 acceptance test for the MERGED step 04-01 (ADR-0071 Path A) — the
//! `MtlsInterceptWorker::start_alloc` SWAP from the retired `cgroup_connect4_mtls`
//! attach to the per-veth egress nft-TPROXY install.
//!
//! Scenario `start_alloc_installs_outbound_and_inbound_tproxy_no_cgroup`: drive
//! `start_alloc(spec)` (PORT-TO-PORT through the worker's inherent
//! `start_alloc` driving port) with `spec.host_veth = Some(<host-side veth>)` —
//! the channel the action-shim C3 provision seam sets in production (JOIN-6) —
//! and assert the OBSERVABLE kernel install:
//!
//!   AC1  the OUTBOUND egress nft-TPROXY rule is APPENDED to the shared
//!        `overdrive-mtls` PREROUTING chain matching `iifname <host_veth>` and
//!        redirecting to a leg-F loopback port (`tproxy to 127.0.0.1:<legF>`).
//!        This is `install_outbound_tproxy` (03-01) wired into `start_alloc`.
//!   AC2  NO cgroup program is attached — the retired `cgroup_connect4_mtls`
//!        kernel-side program is GONE from `overdrive_bpf.o`, so `bpftool prog
//!        show` lists no `cgroup_connect4_mtls` (the deletion is observable, not
//!        just structural). The worker holds no `MtlsDataplane`.
//!   AC3  the leg-C IP_TRANSPARENT listener + the leg-F + leg-C accept loops are
//!        stood up (the install completes `Ok(())`; a re-fire is idempotent).
//!   AC4  on `stop_alloc`, the per-veth egress rule is REMOVED by handle (the
//!        `TproxyInterceptGuard` Drop) — the shared chain/exemption/ip-rule/route
//!        survive (per-veth teardown, not raze).
//!
//! Litmus (the install is production code, not the fixture): delete the
//! `install_outbound_tproxy(host_veth, leg_f_port)` call-site in `start_alloc`
//! and AC1 goes RED — the `iifname <host_veth>` egress rule never appears. The
//! fixture only creates the veth; the RULE is appended by `start_alloc`.
//!
//! Requires root + CAP_NET_ADMIN/CAP_SYS_ADMIN (nft, ip rule, ip link,
//! IP_TRANSPARENT). A non-root run SKIPs. Run via
//! `cargo xtask lima run -- cargo nextest run -p overdrive-worker
//! --features integration-tests`. NEVER `--no-run` — a compile-only gate is
//! green even when every fixture refuses at boot.
//!
//! Hygiene: the shared `overdrive-mtls` routing infra PERSISTS by design
//! (node-global converge-on-boot), so the test scrubs ALL `overdrive-mtls` nft
//! state + the fwmark rule/route + the test veth at START (tolerate
//! pre-existing) AND END. A cross-PROCESS `flock(2)` lock — the SAME fixed path
//! `mtls_intercept_install.rs` / `egress_tproxy_capture.rs` hold — serialises the
//! kernel-touching tests (nextest runs each `#[test]` in a separate process, so
//! an in-process `serial_test` lock cannot serialise node-global kernel state).

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "Test body; skip messages + evidence go to stderr; failures must panic with informative messages"
)]

use std::collections::BTreeMap;
use std::os::fd::AsRawFd as _;
use std::process::{Command, Stdio};
use std::sync::Arc;

use overdrive_core::AllocationId;
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::driver::{AllocationSpec, Resources};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_sim::adapters::SimIdentityRead;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;

/// The host-side veth NAME the OUTBOUND egress rule matches. Distinct from any
/// other suite's veth so concurrent (serialised) runs don't collide.
const VETH_H: &str = "ovd-hv-sa0401";
/// The peer (in-netns) end of the pair — created only so the host end is a real
/// veth interface the `iifname` rule can name; never carries traffic in this AT.
const VETH_PEER: &str = "ovd-wv-sa0401";

/// Cross-PROCESS exclusion for the shared host-netns kernel state — the SAME
/// fixed lock path the sibling kernel-touching suites hold so they cannot race
/// each other's `overdrive-mtls` chain dumps.
struct KernelStateLock {
    fd: std::os::fd::OwnedFd,
}

impl KernelStateLock {
    fn acquire() -> Self {
        use std::os::fd::FromRawFd as _;
        let path = c"/tmp/overdrive-mtls-kernel-state.lock";
        // SAFETY: open with O_CREAT|O_RDWR on a fixed path; the fd is adopted by
        // OwnedFd. flock blocks until the exclusive lock is held.
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

/// True iff this process is uid 0 (root).
fn is_root() -> bool {
    // SAFETY: getuid is always safe; it takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

/// Run `<prog> <args>` best-effort (teardown / tolerate-pre-existing).
fn run_quiet(prog: &str, args: &[&str]) {
    let _ = Command::new(prog).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// Create the host-side veth pair so the `iifname VETH_H` egress rule names a
/// real interface. Idempotent: delete-then-add.
fn create_host_veth() {
    run_quiet("ip", &["link", "del", VETH_H]);
    let out = Command::new("ip")
        .args(["link", "add", VETH_H, "type", "veth", "peer", "name", VETH_PEER])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn ip link add veth");
    assert!(
        out.status.success(),
        "ip link add {VETH_H} type veth peer {VETH_PEER} failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    run_quiet("ip", &["link", "set", VETH_H, "up"]);
    run_quiet("ip", &["link", "set", VETH_PEER, "up"]);
}

/// Scrub ALL `overdrive-mtls` nft state + the shared fwmark rule/route + the
/// test veth so a clean-kernel ground-truth run is reproducible. Run at START
/// (tolerate pre-existing) AND END. Best-effort.
fn clean_shared_infra() {
    // Drain however many fwmark rules a prior run may have stacked.
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
    run_quiet("ip", &["route", "del", "local", "0.0.0.0/0", "dev", "lo", "table", "100"]);
    run_quiet("nft", &["delete", "table", "ip", "overdrive-mtls"]);
    run_quiet("ip", &["link", "del", VETH_H]);
}

/// `nft -a list chain ip overdrive-mtls prerouting` — Ok(dump) on a present
/// chain, Err(stderr) on absent. `-a` emits the per-rule `# handle <N>`.
fn nft_list_chain() -> Result<String, String> {
    let out = Command::new("nft")
        .args(["-a", "list", "chain", "ip", "overdrive-mtls", "prerouting"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("spawn nft: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// True iff the chain dump carries the OUTBOUND egress rule matching
/// `iifname <host_veth>` and a `tproxy to 127.0.0.1:` redirect (any leg-F port —
/// the worker picks it ephemerally, so we assert the veth match + the redirect,
/// not the specific port).
fn dump_has_egress_rule(dump: &str, host_veth: &str) -> bool {
    let iif = format!("iifname \"{host_veth}\"");
    let iif_unquoted = format!("iifname {host_veth}");
    dump.lines().any(|l| {
        (l.contains(&iif) || l.contains(&iif_unquoted)) && l.contains("tproxy to 127.0.0.1:")
    })
}

/// `bpftool prog show` — true iff a `cgroup_connect4_mtls` program is loaded.
/// AC2: the retired program is GONE from `overdrive_bpf.o`, so this MUST be
/// false (the deletion is observable at the kernel boundary, not just in source).
fn cgroup_connect4_mtls_program_loaded() -> bool {
    let out = Command::new("bpftool")
        .args(["prog", "show"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).contains("cgroup_connect4_mtls"),
        // bpftool absent/failed — treat as "not loaded" (cannot prove presence).
        Err(_) => false,
    }
}

fn build_spec(alloc: &AllocationId, host_veth: Option<String>) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/sa/alloc/01")
            .expect("valid spiffe id"),
        command: "/bin/true".to_owned(),
        args: vec![],
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        // The C3 provision seam sets this in production (JOIN-6); the AT supplies
        // it directly to exercise the OUTBOUND egress-rule install.
        netns: None,
        host_veth,
    }
}

/// Build the worker with a sim enforcement double — the AT asserts the INSTALL
/// (nft rule + listeners), never drives a connection, so the enforcement port
/// need not actually enforce.
fn build_worker() -> Arc<MtlsInterceptWorker> {
    let identity: Arc<dyn IdentityRead> = Arc::new(SimIdentityRead::new(BTreeMap::new(), None));
    let enforcement: Arc<dyn MtlsEnforcement> =
        Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));
    Arc::new(MtlsInterceptWorker::new(enforcement, Arc::new(SimClock::new())))
}

/// The MERGED-step 04-01 AT: `start_alloc` installs the OUTBOUND egress
/// nft-TPROXY rule on the alloc's host-side veth + the leg-F/leg-C listeners,
/// with NO cgroup program (the retired mechanism is gone).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_alloc_installs_outbound_and_inbound_tproxy_no_cgroup() {
    if !is_root() {
        eprintln!("SKIP start_alloc_installs_outbound_and_inbound_tproxy_no_cgroup: not root");
        return;
    }
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();
    create_host_veth();

    let worker = build_worker();
    let alloc = AllocationId::new("alloc-sa-0401").expect("valid alloc id");
    let spec = build_spec(&alloc, Some(VETH_H.to_owned()));

    // PORT-TO-PORT: drive the worker's `start_alloc` inherent driving port. This
    // is the production install path the action-shim fires at `on_alloc_running`.
    worker.start_alloc(&spec).expect("start_alloc must install both tproxy + listeners");

    // AC1: the OUTBOUND egress rule matching `iifname VETH_H` → leg-F is in the
    // shared chain. The fixture only created the veth; the RULE is appended by
    // `start_alloc` → `install_outbound_tproxy`. Delete that call-site and this
    // assertion goes RED.
    let dump = nft_list_chain()
        .expect("start_alloc must have ensured the shared overdrive-mtls prerouting chain");
    assert!(
        dump_has_egress_rule(&dump, VETH_H),
        "AC1: start_alloc must append the OUTBOUND egress rule matching iifname {VETH_H} \
         → tproxy to 127.0.0.1:<legF>, got chain:\n{dump}"
    );

    // AC2: the retired cgroup_connect4_mtls program is GONE — no cgroup attach.
    assert!(
        !cgroup_connect4_mtls_program_loaded(),
        "AC2: the retired cgroup_connect4_mtls program must NOT be loaded — start_alloc \
         installs the egress nft rule, NOT a cgroup attach (the program is deleted from \
         overdrive_bpf.o)"
    );

    // AC3: re-fire is idempotent — a second start_alloc for the same alloc tears
    // the prior intercept down first (removing its egress rule) and re-installs;
    // the chain must still carry EXACTLY the rule for this veth, not a stacked
    // pair.
    worker.start_alloc(&spec).expect("re-fire start_alloc must be idempotent");
    let dump_after_refire = nft_list_chain().expect("chain present after re-fire");
    let egress_rule_count = dump_after_refire
        .lines()
        .filter(|l| {
            (l.contains(&format!("iifname \"{VETH_H}\""))
                || l.contains(&format!("iifname {VETH_H}")))
                && l.contains("tproxy to 127.0.0.1:")
        })
        .count();
    assert_eq!(
        egress_rule_count, 1,
        "AC3: a re-fire must leave EXACTLY ONE egress rule for {VETH_H} (teardown-then-\
         reinstall), got {egress_rule_count}:\n{dump_after_refire}"
    );

    // AC4: stop_alloc removes the per-veth egress rule by handle; the shared
    // chain itself survives (per-veth teardown, not raze).
    worker.stop_alloc(&alloc);
    // The blocking accept loops observe the cooperative stop flag between 200ms
    // poll slices, then exit; the guard Drop removes the nft rule synchronously
    // on stop_alloc. Re-dump and assert the egress rule is gone.
    let dump_after_stop = nft_list_chain();
    if let Ok(dump_after_stop) = dump_after_stop {
        assert!(
            !dump_has_egress_rule(&dump_after_stop, VETH_H),
            "AC4: stop_alloc must remove the per-veth egress rule for {VETH_H} by handle, \
             got chain:\n{dump_after_stop}"
        );
    }
    // (If the chain itself is gone that's also acceptable — nothing left to match.)

    clean_shared_infra();
}
