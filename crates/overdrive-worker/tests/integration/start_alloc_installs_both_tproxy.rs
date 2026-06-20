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
//!   AC2  the retired `cgroup_connect4_mtls` kernel-side program is GONE from
//!        the built `overdrive_bpf.o` — its ELF section `cgroup/connect4_mtls`
//!        is ABSENT from the `readelf -S` section table while the look-alike LB
//!        section `cgroup/connect4` is PRESENT (the deletion is observable at
//!        the ELF boundary, and the named false-positive is preserved). The
//!        worker holds no `MtlsDataplane`. (Re-adding the program to the ELF
//!        turns this RED — unlike the prior `bpftool prog show` check, which was
//!        vacuous because the test process never loads the object.)
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

/// Resolve the built `overdrive_bpf.o` path. Honours `OVERDRIVE_BPF_OBJECT`
/// (the env override the mutation runner + Lima wrapper set), else the
/// workspace-relative `target/bpf/overdrive_bpf.o`. `CARGO_MANIFEST_DIR` is
/// `crates/overdrive-worker`; pop twice to the workspace root. (Copied from
/// `overdrive-bpf/tests/integration/bpf_artifact.rs` per the test-helper
/// convention — the worker crate does not dep `overdrive-bpf`.)
fn bpf_object_path() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("OVERDRIVE_BPF_OBJECT").filter(|v| !v.is_empty()) {
        return std::path::PathBuf::from(p);
    }
    let mut root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root.pop();
    root.join("target/bpf/overdrive_bpf.o")
}

/// The set of `cgroup/connect4*` ELF section names in `overdrive_bpf.o`
/// (`readelf -S` section-header dump). The HONEST deletion litmus: the SWAP
/// retired the `cgroup_connect4_mtls` kernel-side program, so its section
/// `cgroup/connect4_mtls` MUST be ABSENT from the freshly-built object — while
/// the look-alike LB program's section `cgroup/connect4` MUST still be present
/// (proving the test actually parsed a real object, not an empty/missing file).
/// Re-adding the deleted program to the ELF turns the absence assertion RED.
fn cgroup_connect4_sections() -> Vec<String> {
    let obj = bpf_object_path();
    let out = Command::new("readelf")
        .args(["-S", "-W", &obj.to_string_lossy()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .expect("spawn readelf -S");
    assert!(
        out.status.success(),
        "readelf -S {} failed — run `cargo xtask bpf-build` first",
        obj.display()
    );
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .filter(|tok| tok.starts_with("cgroup/connect4"))
        .map(str::to_owned)
        .collect()
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

/// Build the worker with sim enforcement + resolve doubles — the AT asserts the
/// INSTALL (nft rule + listeners), never drives a connection, so neither port is
/// actually exercised (the resolve consumer is the 04-02 default-lane DST's job).
fn build_worker() -> Arc<MtlsInterceptWorker> {
    let identity: Arc<dyn IdentityRead> = Arc::new(SimIdentityRead::new(BTreeMap::new(), None));
    let enforcement: Arc<dyn MtlsEnforcement> =
        Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));
    let resolve: Arc<dyn overdrive_core::traits::mtls_resolve::MtlsResolve> =
        Arc::new(overdrive_sim::adapters::SimMtlsResolve::new(
            BTreeMap::new(),
            overdrive_core::traits::mtls_resolve::MtlsResolution::NonMesh,
        ));
    Arc::new(MtlsInterceptWorker::new(enforcement, resolve, Arc::new(SimClock::new())))
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

    // AC2: the retired cgroup_connect4_mtls program is GONE from the built ELF.
    // HONEST litmus (not vacuous): parse the actual `overdrive_bpf.o` section
    // table and assert `cgroup/connect4_mtls` is ABSENT while the look-alike LB
    // section `cgroup/connect4` is PRESENT. Re-adding the deleted program to the
    // ELF turns the first assertion RED; a missing/empty object turns the second
    // assertion RED. (The prior `bpftool prog show` check was vacuous: the test
    // process never loads overdrive_bpf.o, so it would pass identically before
    // the deletion.)
    let sections = cgroup_connect4_sections();
    assert!(
        !sections.iter().any(|s| s == "cgroup/connect4_mtls"),
        "AC2: the retired cgroup_connect4_mtls program must be ABSENT from overdrive_bpf.o's \
         section table (the deletion is observable at the ELF boundary), got cgroup/connect4* \
         sections: {sections:?}"
    );
    assert!(
        sections.iter().any(|s| s == "cgroup/connect4"),
        "AC2: the look-alike LB program section cgroup/connect4 must STILL be present (proves \
         the litmus parsed a real object, and the named false-positive was preserved), got: \
         {sections:?}"
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

    // AC4: stop_alloc removes the per-veth egress rule by handle; the SHARED
    // chain itself SURVIVES (per-veth teardown, NOT raze — the shared
    // overdrive-mtls routing infra is node-global converge-on-boot state, so a
    // single alloc's stop must not raze it out from under every other alloc).
    worker.stop_alloc(&alloc);
    // The blocking accept loops observe the cooperative stop flag between 200ms
    // poll slices, then exit; the guard Drop removes the nft rule synchronously
    // on stop_alloc. Re-dump and assert (a) the shared chain still EXISTS and
    // (b) only the per-veth rule is gone.
    let dump_after_stop = nft_list_chain().expect(
        "AC4: the shared overdrive-mtls prerouting chain must SURVIVE stop_alloc \
         (per-veth teardown, not raze) — its absence means the per-alloc stop razed \
         shared node-global infra",
    );
    assert!(
        !dump_has_egress_rule(&dump_after_stop, VETH_H),
        "AC4: stop_alloc must remove the per-veth egress rule for {VETH_H} by handle, \
         leaving the shared chain otherwise intact, got chain:\n{dump_after_stop}"
    );

    clean_shared_infra();
}
