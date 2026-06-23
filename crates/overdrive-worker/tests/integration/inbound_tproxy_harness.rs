//! Shared Tier-3 fixture for the canonical-workload-address inbound-TPROXY
//! scenarios (GH #241, step 03-01): S-NRULES, S-DPORT, S-JOB0.
//!
//! All three deploy a workload through the PRODUCTION `start_alloc` driving
//! port and observe the live `overdrive-mtls` nft ruleset — they share the
//! identical non-trivial real-infra setup (the cross-process kernel-state lock,
//! the shared-infra scrub, the chain dump, the sim-double worker, the
//! `AllocationSpec` builder). Promoted to a shared helper per
//! `.claude/rules/development.md` § "Shared real-infra test fixtures" (≥2
//! consumers, non-trivial setup). Modelled on the established worker Tier-3
//! pattern in `start_alloc_installs_both_tproxy.rs`.
//!
//! The rule install MUST be the production `start_alloc` call site — these
//! helpers NEVER call `install_inbound_tproxy` themselves (vertical-slice rule,
//! CLAUDE.md § "Build vertical slices through production entry points").

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    reason = "Test harness; skip messages + evidence go to stderr; fixture preconditions must panic with informative messages"
)]

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::num::NonZeroU16;
use std::os::fd::AsRawFd as _;
use std::process::{Command, Stdio};
use std::sync::Arc;

use overdrive_core::AllocationId;
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::driver::{AllocationSpec, Resources};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_sim::adapters::{SimIdentityRead, SimMtlsResolve};
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;

/// Cross-PROCESS exclusion for the shared host-netns kernel state — the SAME
/// fixed lock path the sibling kernel-touching suites
/// (`start_alloc_installs_both_tproxy.rs`, `mtls_intercept_install.rs`,
/// `egress_tproxy_capture.rs`) hold, so they cannot race each other's
/// `overdrive-mtls` chain dumps. nextest runs each `#[test]` in a separate
/// process, so an in-process `serial_test` lock cannot serialise node-global
/// kernel state.
pub struct KernelStateLock {
    fd: std::os::fd::OwnedFd,
}

impl KernelStateLock {
    pub fn acquire() -> Self {
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

/// True iff this process is uid 0 (root). The Tier-3 nft/ip operations require
/// CAP_NET_ADMIN/CAP_SYS_ADMIN; a non-root run SKIPs.
pub fn is_root() -> bool {
    // SAFETY: getuid is always safe; it takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

/// Record the kernel the verdict is pinned to (spike.md discipline — the dev
/// Lima and pinned-6.18 appliance kernel differ).
pub fn record_uname(tag: &str) {
    let kr = Command::new("uname")
        .arg("-r")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    eprintln!("[{tag}] uname -r = {kr}");
}

/// Run `<prog> <args>` best-effort (teardown / tolerate-pre-existing).
fn run_quiet(prog: &str, args: &[&str]) {
    let _ = Command::new(prog).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// Scrub ALL `overdrive-mtls` nft state + the shared fwmark rule/route so a
/// clean-kernel ground-truth run is reproducible. Run at START (tolerate
/// pre-existing) AND END. Best-effort. The shared `overdrive-mtls` routing infra
/// PERSISTS by design (node-global converge-on-boot), so a clean-slate dump
/// requires explicitly razing it between tests.
pub fn clean_shared_infra() {
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
}

/// `nft -a list chain ip overdrive-mtls prerouting` — `Ok(dump)` on a present
/// chain, `Err(stderr)` on an absent table/chain. `-a` emits the per-rule
/// `# handle <N>`.
pub fn nft_list_chain() -> Result<String, String> {
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

/// Count the per-virt INBOUND capture rules whose match is
/// `ip daddr <workload_addr> tcp dport <service_port>` in the chain dump. nft
/// renders an appended inbound rule as e.g. `ip daddr 10.99.0.2 tcp dport 18555
/// tproxy to 127.0.0.1:36533 ... # handle 3`.
pub fn count_inbound_rules(dump: &str, workload_addr: Ipv4Addr, service_port: u16) -> usize {
    let daddr = format!("ip daddr {workload_addr}");
    let dport = format!("tcp dport {service_port}");
    dump.lines()
        .filter(|l| l.contains(&daddr) && l.contains(&dport) && l.contains("tproxy to 127.0.0.1:"))
        .count()
}

/// The `tproxy to 127.0.0.1:<port>` redirect TARGET of the inbound rule for
/// `(workload_addr, service_port)`, if present — the ephemeral leg-C port the
/// production install threads in. Parses the single matching rule line.
pub fn inbound_rule_tproxy_target_port(
    dump: &str,
    workload_addr: Ipv4Addr,
    service_port: u16,
) -> Option<u16> {
    let daddr = format!("ip daddr {workload_addr}");
    let dport = format!("tcp dport {service_port}");
    let marker = "tproxy to 127.0.0.1:";
    dump.lines().find(|l| l.contains(&daddr) && l.contains(&dport) && l.contains(marker)).and_then(
        |line| {
            let after = line.split(marker).nth(1)?;
            // The port runs up to the first non-digit (a space before `meta`).
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u16>().ok()
        },
    )
}

/// Build an [`AllocationSpec`] for the inbound scenarios. `workload_addr` +
/// `service_ports` are the channel the C3 provision seam + `WorkloadLifecycle`
/// populate in production (01-02); the AT supplies them directly to drive the
/// inbound-rule install. `host_veth = None` (no outbound egress rule — these
/// scenarios isolate the INBOUND install).
pub fn build_inbound_spec(
    alloc: &AllocationId,
    workload_addr: Option<Ipv4Addr>,
    service_ports: Vec<NonZeroU16>,
) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/sa/alloc/01")
            .expect("valid spiffe id"),
        command: "/bin/true".to_owned(),
        args: vec![],
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        netns: None,
        host_veth: None,
        service_ports,
        workload_addr,
    }
}

/// Build the worker with sim enforcement + resolve doubles. These scenarios
/// assert the INSTALL (nft rule shape), so neither port is driven by a
/// connection — the doubles only satisfy the constructor.
pub fn build_worker() -> Arc<MtlsInterceptWorker> {
    let identity: Arc<dyn IdentityRead> = Arc::new(SimIdentityRead::new(BTreeMap::new(), None));
    let enforcement: Arc<dyn MtlsEnforcement> =
        Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));
    let resolve: Arc<dyn MtlsResolve> =
        Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh));
    Arc::new(MtlsInterceptWorker::new(enforcement, resolve, Arc::new(SimClock::new())))
}
