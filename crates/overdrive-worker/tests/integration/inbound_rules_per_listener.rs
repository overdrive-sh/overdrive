//! S-NRULES — N listeners install exactly N inbound captures (GH #241,
//! step 03-01, Tier-3 real nft observation).
//!
//! A Service with N=2 declared listeners → `start_alloc` installs exactly 2
//! `inbound_destinations` elements, each keyed by the exact
//! `<workload_addr>:<port_i>` tuple; two element guards are retained and both
//! are released on allocation teardown. The one node-scoped constant inbound
//! rule and its live leg-C target survive unchanged.
//!
//! Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-NRULES.
//!
//! Litmus (the install is production code, not the fixture): revert
//! `start_alloc` to omit inbound acquisition and this test goes RED — the two
//! destination elements never appear. The fixture only supplies
//! `workload_addr` + `service_ports` on the spec; production `start_alloc`
//! acquires the captures. The fixture never installs them itself.
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

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::NonZeroU16;

use overdrive_core::AllocationId;
use overdrive_netlink::nft;

use super::inbound_tproxy_harness::{
    KernelStateLock, build_inbound_spec, build_worker, clean_shared_infra, is_root, nft_list_chain,
    record_uname,
};

/// The canonical per-workload address the 2-listener Service was provisioned
/// into (the in-netns `/30` end the C3 seam sets in production). A test-distinct
/// /32 so concurrent (serialised) runs don't collide with the sibling suites.
const WORKLOAD_ADDR: &str = "10.99.0.2";
/// The two declared Service listener ports — the SAME values `service_backends`
/// advertises (D-BLOCKER1 one-source/two-readers).
const PORT_A: u16 = 18555;
const PORT_B: u16 = 18666;

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_declared_listeners_install_exactly_two_inbound_captures() {
    if !is_root() {
        eprintln!("SKIP two_declared_listeners_install_exactly_two_inbound_captures: not root");
        return;
    }
    record_uname("03-01-nrules");
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();

    let workload_addr: Ipv4Addr = WORKLOAD_ADDR.parse().expect("workload addr");
    let ports =
        vec![NonZeroU16::new(PORT_A).expect("port a"), NonZeroU16::new(PORT_B).expect("port b")];

    let worker = build_worker();
    worker
        .start_shared_owner()
        .await
        .expect("the node-shared listener owner publishes before allocation admission");
    let alloc = AllocationId::new("alloc-sa-0301-nrules").expect("valid alloc id");
    let spec = build_inbound_spec(&alloc, Some(workload_addr), ports);

    // PORT-TO-PORT: drive the worker's `start_alloc` inherent driving port — the
    // production install path the action-shim fires at `on_alloc_running`. The
    // fixture supplied `workload_addr` + two `service_ports`; the production
    // per-port install loop acquires the destination elements.
    worker
        .start_alloc(&spec)
        .await
        .expect("start_alloc must install the two declared inbound captures");

    let expected_destinations = BTreeSet::from([
        SocketAddrV4::new(workload_addr, PORT_A),
        SocketAddrV4::new(workload_addr, PORT_B),
    ]);
    let before_stop = nft::observe_shared_ip_intercept_state()
        .expect("the typed shared-IP observation succeeds after start_alloc")
        .expect("the node-scoped constant intercept program remains present after start_alloc");
    assert_eq!(
        before_stop.inbound_destinations(),
        &expected_destinations,
        "S-NRULES: two declared listeners must produce exactly their two declared \
         destination tuples and no third capture"
    );
    let program_before_stop = before_stop.identity().clone();

    let leg_c_addr = worker
        .leg_c_addr(&alloc)
        .expect("the active allocation refers to the live node-shared leg-C listener");
    let dump = nft_list_chain()
        .expect("start_alloc must leave the shared overdrive-mtls prerouting chain present");
    assert!(
        dump.lines().any(|line| {
            line.contains("ip daddr . tcp dport @inbound_destinations")
                && line.contains(&format!("tproxy to {leg_c_addr} accept"))
        }),
        "S-NRULES: the one constant inbound rule must match @inbound_destinations and target \
         the live leg-C listener {leg_c_addr}:\n{dump}"
    );

    // Both element guards are released by awaited teardown. The dynamic set is
    // empty again while the complete node-scoped constant program survives.
    worker.stop_alloc(&alloc).await.expect("allocation teardown succeeds");
    let after_stop = nft::observe_shared_ip_intercept_state()
        .expect("the typed shared-IP observation succeeds after stop_alloc")
        .expect("the node-scoped constant intercept program survives stop_alloc");
    assert_eq!(
        after_stop.inbound_destinations(),
        &BTreeSet::new(),
        "S-NRULES: awaited teardown must remove both declared destination tuples"
    );
    assert_eq!(
        after_stop.identity(),
        &program_before_stop,
        "S-NRULES: allocation teardown must preserve the complete node-scoped constant program"
    );

    worker.shutdown_owner().await.expect("the node-shared listener owner shuts down cleanly");
    clean_shared_infra();
}
