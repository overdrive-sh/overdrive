//! S-NRULES — N listeners install exactly N inbound rules (GH #241, step 03-01,
//! Tier-3 real nft observation).
//!
//! A Service with N=2 declared listeners → `start_alloc` installs exactly 2
//! inbound capture rules, each keyed `ip daddr <workload_addr> tcp dport
//! <port_i>` per listener; 2 RAII guards retained, both released on alloc
//! teardown (no leftover nft state). Maps D-A1 (N listeners → N rules via the
//! per-port `install_inbound_tproxy` loop).
//!
//! Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-NRULES.
//!
//! Litmus (the install is production code, not the fixture): revert
//! `start_alloc` to `tproxy_guard = None` and this test goes RED — the two
//! per-virt rules never appear. The fixture only supplies `workload_addr` +
//! `service_ports` on the spec; the RULES are appended by `start_alloc`. The
//! install MUST be the production `start_alloc` call site, never a test-installed
//! `install_inbound_tproxy`.
//!
//! Requires root + `CAP_NET_ADMIN`/`CAP_SYS_ADMIN`; a non-root run SKIPs. Run via
//! `cargo xtask lima run -- cargo nextest run -p overdrive-worker
//! --features integration-tests`. NEVER `--no-run`.

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    reason = "Test body; skip messages + evidence go to stderr; failures must panic with informative messages"
)]

use std::net::Ipv4Addr;
use std::num::NonZeroU16;

use overdrive_core::AllocationId;

use super::inbound_tproxy_harness::{
    KernelStateLock, build_inbound_spec, build_worker, clean_shared_infra, count_inbound_rules,
    is_root, nft_list_chain, record_uname,
};

/// The canonical per-workload address the 2-listener Service was provisioned
/// into (the in-netns `/30` end the C3 seam sets in production). A test-distinct
/// /32 so concurrent (serialised) runs don't collide with the sibling suites.
const WORKLOAD_ADDR: &str = "10.99.0.2";
/// The two declared Service listener ports — the SAME values `service_backends`
/// advertises (D-BLOCKER1 one-source/two-readers).
const PORT_A: u16 = 18555;
const PORT_B: u16 = 18666;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_declared_listeners_install_exactly_two_inbound_capture_rules() {
    if !is_root() {
        eprintln!(
            "SKIP two_declared_listeners_install_exactly_two_inbound_capture_rules: not root"
        );
        return;
    }
    record_uname("03-01-nrules");
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();

    let workload_addr: Ipv4Addr = WORKLOAD_ADDR.parse().expect("workload addr");
    let ports =
        vec![NonZeroU16::new(PORT_A).expect("port a"), NonZeroU16::new(PORT_B).expect("port b")];

    let worker = build_worker();
    let alloc = AllocationId::new("alloc-sa-0301-nrules").expect("valid alloc id");
    let spec = build_inbound_spec(&alloc, Some(workload_addr), ports);

    // PORT-TO-PORT: drive the worker's `start_alloc` inherent driving port — the
    // production install path the action-shim fires at `on_alloc_running`. The
    // fixture supplied `workload_addr` + two `service_ports`; the per-port
    // `install_inbound_tproxy` loop in `start_alloc` appends the rules.
    worker
        .start_alloc(&spec)
        .expect("start_alloc must install the per-port inbound rules + listeners");

    let dump = nft_list_chain()
        .expect("start_alloc must have ensured the shared overdrive-mtls prerouting chain");

    // Exactly ONE rule per declared port, keyed on the workload_addr + that port.
    let count_a = count_inbound_rules(&dump, workload_addr, PORT_A);
    let count_b = count_inbound_rules(&dump, workload_addr, PORT_B);
    assert_eq!(
        count_a, 1,
        "S-NRULES: exactly one inbound rule keyed `ip daddr {workload_addr} tcp dport {PORT_A}` \
         must be installed, got {count_a}:\n{dump}"
    );
    assert_eq!(
        count_b, 1,
        "S-NRULES: exactly one inbound rule keyed `ip daddr {workload_addr} tcp dport {PORT_B}` \
         must be installed, got {count_b}:\n{dump}"
    );
    // And no spurious THIRD rule for this workload (N declared → N rules).
    let total_for_workload = dump
        .lines()
        .filter(|l| {
            l.contains(&format!("ip daddr {workload_addr}")) && l.contains("tproxy to 127.0.0.1:")
        })
        .count();
    assert_eq!(
        total_for_workload, 2,
        "S-NRULES: a 2-listener Service must install EXACTLY 2 inbound rules for \
         {workload_addr} (N listeners → N rules), got {total_for_workload}:\n{dump}"
    );

    // Both RAII guards released on teardown — no leftover nft state for the
    // workload. `stop_alloc` drops the retained `Vec<TproxyInterceptGuard>`,
    // whose `Drop` removes each per-virt rule by handle.
    worker.stop_alloc(&alloc);
    let dump_after_stop = nft_list_chain().expect(
        "S-NRULES: the shared overdrive-mtls prerouting chain must SURVIVE stop_alloc \
         (per-virt teardown, not raze)",
    );
    let leftover = dump_after_stop
        .lines()
        .filter(|l| {
            l.contains(&format!("ip daddr {workload_addr}")) && l.contains("tproxy to 127.0.0.1:")
        })
        .count();
    assert_eq!(
        leftover, 0,
        "S-NRULES: both inbound rules for {workload_addr} must be released on teardown \
         (RAII guards dropped), got {leftover} leftover:\n{dump_after_stop}"
    );

    clean_shared_infra();
}
