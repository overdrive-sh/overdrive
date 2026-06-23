//! S-JOB0 — a Job-kind alloc (0 listeners) installs 0 inbound rules (GH #241,
//! step 03-01, Tier-3, error/edge).
//!
//! `project_service_listen_ports` returns `Vec::new()` for `Job`/`Schedule`
//! (mirroring `project_probe_descriptors`), so a Job alloc carries empty
//! `service_ports` / `None` `workload_addr` → `start_alloc` installs ZERO inbound
//! capture rules (the host-netns/Job path, unchanged) and retains no
//! `TproxyInterceptGuard`. No spurious capture diverts unrelated traffic.
//!
//! Error/edge guard: a mutant that installs an all-TCP or hardcoded-port rule
//! for a Job fails this scenario.
//!
//! Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-JOB0.
//!
//! Litmus: an unconditional install (one that does not gate on
//! `spec.workload_addr.is_some()` + a non-empty `service_ports`) would append a
//! rule for the Job alloc → RED. The install MUST be the production `start_alloc`
//! call site.
//!
//! Requires root; non-root SKIPs. Run via `cargo xtask lima run -- cargo nextest
//! run -p overdrive-worker --features integration-tests`. NEVER `--no-run`.

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    reason = "Test body; skip messages + evidence go to stderr; failures must panic with informative messages"
)]

use overdrive_core::AllocationId;

use super::inbound_tproxy_harness::{
    KernelStateLock, build_inbound_spec, build_worker, clean_shared_infra, is_root, nft_list_chain,
    record_uname,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn job_kind_workload_with_no_listeners_installs_no_inbound_capture_rule() {
    if !is_root() {
        eprintln!(
            "SKIP job_kind_workload_with_no_listeners_installs_no_inbound_capture_rule: not root"
        );
        return;
    }
    record_uname("03-01-job0");
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();

    let worker = build_worker();
    let alloc = AllocationId::new("alloc-sa-0301-job0").expect("valid alloc id");
    // Job-kind shape: no canonical workload address, no declared service ports —
    // exactly what `WorkloadLifecycle` emits for a Job/Schedule alloc.
    let spec = build_inbound_spec(&alloc, None, Vec::new());

    // PORT-TO-PORT: drive production `start_alloc`. With `None` workload_addr +
    // empty service_ports, the per-port install loop must run ZERO iterations.
    worker
        .start_alloc(&spec)
        .expect("start_alloc must succeed for a Job-kind alloc with no listeners");

    // ZERO inbound capture rules. An absent table/chain is itself "zero rules"
    // (the Job path may not ensure the shared inbound infra at all); a present
    // chain must carry NO `tproxy to 127.0.0.1:` per-virt rule for this alloc.
    match nft_list_chain() {
        Ok(dump) => {
            let inbound_rules = dump
                .lines()
                .filter(|l| l.contains("ip daddr") && l.contains("tproxy to 127.0.0.1:"))
                .count();
            assert_eq!(
                inbound_rules, 0,
                "S-JOB0: a Job-kind alloc with empty service_ports must install ZERO inbound \
                 capture rules (no spurious all-TCP / hardcoded-port rule), got {inbound_rules}:\n\
                 {dump}"
            );
        }
        Err(stderr) => {
            // No overdrive-mtls prerouting chain at all → trivially zero rules.
            eprintln!(
                "[03-01-job0] no overdrive-mtls prerouting chain after Job start_alloc \
                 (zero inbound rules, as required): {stderr}"
            );
        }
    }

    worker.stop_alloc(&alloc);
    clean_shared_infra();
}
