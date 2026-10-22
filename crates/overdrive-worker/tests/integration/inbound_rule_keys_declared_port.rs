//! S-DPORT — the inbound capture keys on the DECLARED service port, not the
//! ephemeral leg-C port (GH #241, step 03-01, Tier-3, error/edge).
//!
//! D-BLOCKER1 / D-TME-10 one-source/two-readers: the installed
//! `inbound_destinations` element is keyed by the **declared Service listener
//! port** — the same value `service_backends` advertises and the egress
//! `MtlsResolve` keys on — NOT the ephemeral `leg_c_addr.port()`. The one
//! node-scoped constant rule's `tproxy to` TARGET is the live leg-C listener;
//! the dynamic match KEY remains the declared `(workload address, port)` tuple.
//!
//! Error/edge guard (the NEGATIVE pin): a mutant that keys the destination
//! element on `leg_c_addr.port()` passes a naive "the constant rule exists"
//! check but fails exact typed-set equality with the declared tuple.
//!
//! Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-DPORT.
//!
//! Litmus: omit the `start_alloc` inbound element acquisition → the declared
//! tuple is absent → RED. Key the element on `leg_c_addr.port()` instead of the
//! declared port → exact set equality is RED. The install MUST be the production
//! `start_alloc` call site, never a test-installed capture.
//!
//! Requires root; non-root SKIPs. Run via `cargo xtask lima run -- cargo nextest
//! run -p overdrive-worker --features integration-tests`. NEVER `--no-run`.

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

/// The canonical per-workload address the Service was provisioned into.
const WORKLOAD_ADDR: &str = "10.99.1.2";
/// The single declared Service listener port — the match KEY the rule must use.
const SERVICE_PORT: u16 = 18777;

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inbound_capture_key_matches_declared_service_port_not_ephemeral_leg_c_port() {
    if !is_root() {
        eprintln!(
            "SKIP inbound_capture_key_matches_declared_service_port_not_ephemeral_leg_c_port: \
             not root"
        );
        return;
    }
    record_uname("03-01-dport");
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();

    let workload_addr: Ipv4Addr = WORKLOAD_ADDR.parse().expect("workload addr");
    let ports = vec![NonZeroU16::new(SERVICE_PORT).expect("service port")];

    let worker = build_worker();
    worker
        .start_shared_owner()
        .await
        .expect("the node-shared listener owner publishes before allocation admission");
    let alloc = AllocationId::new("alloc-sa-0301-dport").expect("valid alloc id");
    let spec = build_inbound_spec(&alloc, Some(workload_addr), ports);

    // PORT-TO-PORT: drive production `start_alloc`. The fixture supplied the
    // declared service port; the install must key the dynamic capture on it.
    worker
        .start_alloc(&spec)
        .await
        .expect("start_alloc must install the inbound capture keyed on the declared service port");

    let expected_destinations = BTreeSet::from([SocketAddrV4::new(workload_addr, SERVICE_PORT)]);
    let before_stop = nft::observe_shared_ip_intercept_state()
        .expect("the typed shared-IP observation succeeds after start_alloc")
        .expect("the node-scoped constant intercept program remains present after start_alloc");
    assert_eq!(
        before_stop.inbound_destinations(),
        &expected_destinations,
        "S-DPORT: the exact inbound destination set must contain only the declared Service \
         tuple {workload_addr}:{SERVICE_PORT}; no leg-C-derived or spurious key is allowed"
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
        "S-DPORT: the one constant inbound rule must match @inbound_destinations and target \
         the live leg-C listener {leg_c_addr}:\n{dump}"
    );
    eprintln!(
        "[03-01-dport] capture key={workload_addr}:{SERVICE_PORT} (declared), \
         constant-rule target={leg_c_addr} (live leg-C)"
    );

    worker.stop_alloc(&alloc).await.expect("allocation teardown succeeds");
    let after_stop = nft::observe_shared_ip_intercept_state()
        .expect("the typed shared-IP observation succeeds after stop_alloc")
        .expect("the node-scoped constant intercept program survives stop_alloc");
    assert_eq!(
        after_stop.inbound_destinations(),
        &BTreeSet::new(),
        "S-DPORT: awaited teardown must remove the allocation's declared destination"
    );
    assert_eq!(
        after_stop.identity(),
        &program_before_stop,
        "S-DPORT: allocation teardown must preserve the complete node-scoped constant program"
    );

    worker.shutdown_owner().await.expect("the node-shared listener owner shuts down cleanly");
    clean_shared_infra();
}
