//! Walking skeleton of the vsock attachment (GH #295, P6-S13; Roadmap impact
//! 10-01, V-6): a VM workload reaches a destination off the node over TCP
//! and UDP, deployed through the operator's verb against a real
//! `overdrive serve`, on native metal.
//!
//! The VM is an ordinary guest: its application opens a plain TCP
//! connection and a plain UDP socket to a remote peer. Nothing in it knows
//! about vsock. The bytes travel: guest application → the guest's capture
//! and its flow owner (`overdrive-init`) → the guest's forwarder → the VM's
//! vsock → the host's flow owner and forwarder in `serve` → the host's
//! connection to the destination. The reply comes back the same way.
//!
//! **Oracle (observable outcomes only).** The operator sees the Job end
//! `Terminated` (the guest application's exit 0 means both replies came back
//! byte-exact); the remote peer served the request and the datagram once
//! each, both from the node's own address on the peer's link — the host
//! opened the connections, the guest's packets were never routed; the VM's
//! console shows both round trips. After the Job ends its VMM is gone and no
//! process holds a vhost-vsock device for its CID (CID-NOCLAIM).
//!
//! Lane: `integration-tests,kvm-tests`, `cargo xtask metal run --` (root,
//! the BPF object built). Module in `host-kernel-shared`. Pending DELIVER
//! step 10-01 (the attachment end to end, after steps 1–15).

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(clippy::doc_markdown, reason = "kernel and protocol names in prose")]

use std::time::Duration;

use overdrive_control_plane::api::AllocStateWire;
use overdrive_testing::guest_app::{RemotePeer, subcommand};
use serial_test::serial;

use super::vm_walking_skeleton::TeardownBound;
use super::vsock_support::{Lane, alloc_of, arg, console_of, hypervisor_of};

/// P6-S13 — a VM workload deployed through `overdrive deploy` against a
/// real `overdrive serve` completes a TCP round trip and a UDP datagram
/// round trip with a remote peer, and its attachment is gone with it.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 10-01"]
async fn a_vm_workload_round_trips_tcp_and_udp_through_serve_and_deploy() {
    let _bound = TeardownBound::arm();
    let peer = RemotePeer::start("vsock-ws", 0).expect("precondition: the remote peer starts");
    let lane = Lane::boot("vsock-ws").await;
    let tag = "vsock-ws";
    let id = lane
        .deploy_job(
            tag,
            &[subcommand::ROUND_TRIP, &arg(peer.tcp_addr()), &arg(peer.udp_addr()), tag],
        )
        .await;

    let rows = lane
        .wait_rows(&id, Duration::from_secs(120), "a terminal allocation", |rows| {
            rows.first().is_some_and(|r| {
                matches!(r.state, AllocStateWire::Terminated | AllocStateWire::Failed)
            })
        })
        .await;
    let row = rows.first().expect("one allocation");
    let alloc = alloc_of(row);
    assert_eq!(
        row.state,
        AllocStateWire::Terminated,
        "the guest application exits 0 only when both replies came back byte-exact; row {row:?}; \
         console:\n{}",
        console_of(&alloc)
    );

    let served = peer.snapshot();
    assert_eq!(served.requests.len(), 1, "the TCP request was served once: {served:?}");
    assert_eq!(served.requests[0].tag, tag);
    assert_eq!(
        *served.requests[0].from.ip(),
        peer.node_side_addr(),
        "the node opened the TCP connection: its source is the node's own address"
    );
    assert_eq!(served.datagrams.len(), 1, "the datagram was served once: {served:?}");
    assert_eq!(served.datagrams[0].tag, tag);
    assert_eq!(*served.datagrams[0].from.ip(), peer.node_side_addr(), "the node sent the datagram");
    let console = console_of(&alloc);
    assert!(console.contains("round-trip: tcp ok"), "console:\n{console}");
    assert!(console.contains("round-trip: udp ok"), "console:\n{console}");

    // The attachment ends with the VM: no VMM serves the allocation, so no
    // process holds its vhost-vsock device (the VMM is its only holder).
    assert!(hypervisor_of(&alloc).is_none(), "no VMM serves the ended allocation");

    lane.shutdown().await;
}
