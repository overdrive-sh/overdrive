//! PR-4 (D14 item 2; S-55) — the inbound TPROXY install reads the declared
//! listener set through the TCP view: a Service declaring one TCP and one UDP
//! listener installs exactly one inbound capture (the TCP tuple) and none for the
//! UDP port.
//!
//! The spec carries ONLY `declared_listeners` (`service_ports` empty), so the
//! install can learn the TCP port only by reading the declared set via
//! `AllocationSpec::tcp_service_ports()`. Requires root; non-root SKIPs. Run via
//! `cargo xtask metal run -- cargo nextest run -p overdrive-worker --features
//! integration-tests`.
//! CONTRACT_SHAPE: bounded-change.

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    reason = "Test body; skip messages go to stderr; failures must panic with informative messages"
)]

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::NonZeroU16;

use overdrive_core::AllocationId;
use overdrive_core::aggregate::Listener;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_netlink::nft;

use super::inbound_tproxy_harness::{
    KernelStateLock, build_inbound_spec, build_worker, clean_shared_infra, is_root, record_uname,
};

const WORKLOAD_ADDR: &str = "10.99.0.2";
const TCP_PORT: u16 = 18777;
const UDP_PORT: u16 = 18778;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "pending DELIVER: PR-4 inbound install reads tcp_service_ports of declared_listeners"]
async fn a_udp_declared_listener_gets_no_inbound_capture_and_a_tcp_one_does() {
    if !is_root() {
        eprintln!("SKIP a_udp_declared_listener_gets_no_inbound_capture_and_a_tcp_one_does: not root");
        return;
    }
    record_uname("pr4-tcp-view");
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();

    let workload_addr: Ipv4Addr = WORKLOAD_ADDR.parse().expect("workload addr");
    let worker = build_worker();
    worker
        .start_shared_owner()
        .await
        .expect("the node-shared listener owner publishes before allocation admission");
    let alloc = AllocationId::new("alloc-pr4-tcpview").expect("valid alloc id");
    let mut spec = build_inbound_spec(&alloc, Some(workload_addr), Vec::new());
    spec.declared_listeners = vec![
        Listener { port: NonZeroU16::new(TCP_PORT).expect("port"), protocol: Proto::Tcp },
        Listener { port: NonZeroU16::new(UDP_PORT).expect("port"), protocol: Proto::Udp },
    ];

    worker.start_alloc(&spec).await.expect("start_alloc installs the TCP capture");

    let state = nft::observe_shared_ip_intercept_state()
        .expect("the typed shared-IP observation succeeds after start_alloc")
        .expect("the node-scoped constant intercept program is present after start_alloc");
    assert_eq!(
        state.inbound_destinations(),
        &BTreeSet::from([SocketAddrV4::new(workload_addr, TCP_PORT)]),
        "exactly the TCP declared listener is captured; the UDP one gets no rule"
    );

    worker.stop_alloc(&alloc).await.expect("allocation teardown succeeds");
    worker.shutdown_owner().await.expect("the node-shared listener owner shuts down cleanly");
    clean_shared_infra();
}
