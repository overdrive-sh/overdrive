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
//! step 14-01 (the attachment end to end, after steps 1–15).

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(clippy::doc_markdown, reason = "kernel and protocol names in prose")]

use std::time::Duration;

use overdrive_control_plane::api::{AllocStateWire, ProbeStatusJson};
use overdrive_core::aggregate::probe_descriptor::ProbeMechanic;
use overdrive_core::observation::ProbeRole;
use overdrive_core::{SpiffeId, WorkloadId};
use overdrive_testing::guest_app::{
    RemotePeer, V6_APP_PORT, V6_HTTP_PROBE_PORT, V6_POPULATION_MAX, V6_TCP_PROBE_PORT, subcommand,
};
use serial_test::serial;

use super::guest_stack_mtls_egress::assert_exact_operator_stopped_service_lifecycle_delta;
use super::vm_walking_skeleton::TeardownBound;
use super::vsock_support::{
    EphemeralRangeGuard, Lane, PacketCapture, UplinkGuard, V6ClientSpec, alloc_of, arg,
    assert_population_absent, assert_population_option_set, console_of, hypervisor_of, socket_rows,
};

/// P6-S13 — a VM workload deployed through `overdrive deploy` against a
/// real `overdrive serve` completes a TCP round trip and a UDP datagram
/// round trip with a remote peer, and its attachment is gone with it.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn a_vm_workload_round_trips_tcp_and_udp_through_serve_and_deploy() {
    let _bound = TeardownBound::arm();
    let ephemeral = EphemeralRangeGuard::narrow();
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
    assert_eq!(served.empty_datagrams.len(), 1, "one genuine empty datagram was served");
    assert_eq!(*served.empty_datagrams[0].ip(), peer.node_side_addr());
    let console = console_of(&alloc);
    assert!(console.contains("round-trip: tcp ok"), "console:\n{console}");
    assert!(console.contains("round-trip: udp ok"), "console:\n{console}");
    assert!(console.contains("round-trip: udp-empty ok"), "console:\n{console}");

    // The attachment ends with the VM: no VMM serves the allocation, so no
    // process holds its vhost-vsock device (the VMM is its only holder).
    assert!(hypervisor_of(&alloc).is_none(), "no VMM serves the ended allocation");

    let peer_service = lane.deploy_v6_peer("v6-peer", V6_POPULATION_MAX).await;
    let peer_alloc = lane.running_alloc(&peer_service, Duration::from_secs(120)).await;
    let peer_console = lane
        .wait_console(&peer_alloc, Duration::from_secs(30), |text| {
            text.contains("v6-peer: ready population=65")
        })
        .await
        .unwrap_or_else(|console| panic!("peer Service did not become ready:\n{console}"));
    assert!(peer_console.contains("v6-peer: ready population=65"));
    let peer_snapshot =
        snapshot_with_exact_identity(&lane, &peer_service, &peer_alloc, Duration::from_secs(10))
            .await;
    let peer_vip = peer_snapshot
        .vip
        .as_deref()
        .expect("the peer Service has its issued VIP")
        .parse()
        .expect("the issued VIP is IPv4");
    let peer_addr = peer_snapshot
        .rows
        .iter()
        .find(|row| row.alloc_id == peer_alloc.to_string())
        .and_then(|row| row.workload_addr)
        .expect("the peer Service has its leased workload address");
    assert!(peer_snapshot.listeners.iter().any(|listener| listener.port.get() == V6_APP_PORT));
    let initial_probe_times =
        wait_probe_state(&lane, &peer_service, &peer_alloc, true, Duration::from_secs(30)).await;

    lane.remote_mtls_round_trip("ovd-rp-00", std::net::SocketAddrV4::new(peer_addr, V6_APP_PORT));
    lane.wait_console(&peer_alloc, Duration::from_secs(10), |text| {
        text.contains("V6-REMOTE-REQUEST-route-disjoint-peer")
    })
    .await
    .unwrap_or_else(|console| panic!("remote inbound did not reach the peer Service:\n{console}"));

    let population = ephemeral.run_unset_control();
    let uplink = UplinkGuard::create();
    let uplink_capture = PacketCapture::start(&uplink.iface);
    UplinkGuard::positive_control();
    let loopback_capture = PacketCapture::start("lo");
    let client_service = lane
        .deploy_v6_client(V6ClientSpec {
            id: "v6-client",
            peer_name: "v6-peer.svc.overdrive.local",
            peer_vip,
            off_tcp: peer.tcp_addr(),
            off_udp: peer.udp_addr(),
            population: population.count,
            tag: "v6-client",
        })
        .await;
    let (client_alloc, paths_console) = lane
        .running_alloc_with_console(&client_service, Duration::from_secs(120), |text| {
            text.contains("v6-client: paths-ready")
        })
        .await;
    assert!(paths_console.contains("nameserver=100.95.0.1"));
    assert!(paths_console.contains("v6-client: resolved v6-peer.svc.overdrive.local="));
    assert!(paths_console.contains("v6-client: vip-tcp ok"));
    let client_snapshot = snapshot_with_exact_identity(
        &lane,
        &client_service,
        &client_alloc,
        Duration::from_secs(10),
    )
    .await;
    let client_running = client_snapshot
        .rows
        .iter()
        .find(|row| row.alloc_id == client_alloc.to_string())
        .expect("the client Service has its Running allocation row")
        .clone();
    let tls_evidence = loopback_capture.finish();
    tls_evidence.assert_tls13_no_cleartext(std::net::SocketAddrV4::new(peer_addr, V6_APP_PORT));

    lane.wait_console(&client_alloc, Duration::from_secs(40), |text| {
        text.contains("v6-client: state=unhealthy")
    })
    .await
    .unwrap_or_else(|console| panic!("client Service did not drive unhealthy:\n{console}"));
    let failed_probe_times =
        wait_probe_state(&lane, &peer_service, &peer_alloc, false, Duration::from_secs(5)).await;
    assert!(failed_probe_times[0] > initial_probe_times[0]);
    assert!(failed_probe_times[1] > initial_probe_times[1]);
    lane.wait_console(&client_alloc, Duration::from_secs(45), |text| {
        text.contains("v6-client: state=healthy")
    })
    .await
    .unwrap_or_else(|console| panic!("client Service did not restore healthy:\n{console}"));
    let recovered_probe_times =
        wait_probe_state(&lane, &peer_service, &peer_alloc, true, Duration::from_secs(5)).await;
    assert!(recovered_probe_times[0] > failed_probe_times[0]);
    assert!(recovered_probe_times[1] > failed_probe_times[1]);

    lane.wait_console(&client_alloc, Duration::from_secs(40), |text| {
        text.contains(&format!("v6-client: population-held={}", population.count))
    })
    .await
    .unwrap_or_else(|console| panic!("production option-set cohort did not hold:\n{console}"));
    assert_population_option_set(&ephemeral, peer_addr, population);
    let backend = std::net::SocketAddrV4::new(peer_addr, V6_APP_PORT);
    let leg_c = socket_rows()
        .into_iter()
        .filter(|row| row.local == backend && *row.peer.ip() == std::net::Ipv4Addr::LOCALHOST)
        .collect::<Vec<_>>();
    assert_eq!(leg_c.len(), 1, "one held leg-C socket has a loopback peer: {leg_c:?}");
    let uplink_evidence = uplink_capture.finish();
    uplink_evidence.assert_positive_control_and_no_tuple(backend);
    uplink.finish();

    let served = peer.snapshot();
    assert_eq!(served.requests.len(), 2, "Job and client Service each reached off-node TCP");
    assert_eq!(served.datagrams.len(), 2, "Job and client Service each reached off-node UDP");
    assert_eq!(served.empty_datagrams.len(), 2, "both actors carried a genuine empty UDP datagram");

    lane.stop(&client_service).await;
    lane.stop(&peer_service).await;
    for (id, alloc) in [(&client_service, &client_alloc), (&peer_service, &peer_alloc)] {
        let terminal_rows = lane
            .wait_rows(
                id,
                Duration::from_secs(30),
                "a cleanly terminated Service allocation",
                |rows| {
                    rows.iter().any(|row| {
                        row.alloc_id == alloc.to_string() && row.state == AllocStateWire::Terminated
                    })
                },
            )
            .await;
        let terminal = terminal_rows
            .iter()
            .find(|row| row.alloc_id == alloc.to_string())
            .expect("the stopped Service retains its allocation row");
        if alloc == &client_alloc {
            assert_exact_operator_stopped_service_lifecycle_delta(&client_running, terminal);
        }
        assert!(hypervisor_of(alloc).is_none(), "no VMM remains for stopped Service {id}");
    }

    lane.shutdown().await;
    assert_population_absent(peer_addr, population.count);
    ephemeral.finish();
}

async fn snapshot_with_exact_identity(
    lane: &Lane,
    service: &str,
    alloc: &overdrive_core::id::AllocationId,
    bound: Duration,
) -> overdrive_control_plane::api::AllocStatusResponse {
    let deadline = tokio::time::Instant::now() + bound;
    loop {
        let snapshot = lane.snapshot(service).await;
        let row = snapshot
            .rows
            .iter()
            .find(|row| row.alloc_id == alloc.to_string())
            .unwrap_or_else(|| panic!("Service {service} has no row for allocation {alloc}"));
        assert_eq!(row.alloc_id, alloc.to_string(), "the snapshot row names the requested alloc");
        let workload =
            WorkloadId::new(&row.workload_id).expect("the public row carries a valid workload id");
        let expected = SpiffeId::for_allocation(&workload, alloc);
        let exact = snapshot
            .issued_certificates
            .iter()
            .filter(|certificate| certificate.spiffe_id == expected)
            .count();
        if exact == 1 {
            assert_eq!(
                snapshot.issued_certificates.len(),
                1,
                "the one Running allocation has exactly its one current issued identity"
            );
            return snapshot;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Service {service} allocation {alloc} did not publish its exact per-allocation identity {expected}; certificates={:#?}",
            snapshot.issued_certificates
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_probe_state(
    lane: &Lane,
    service: &str,
    alloc: &overdrive_core::id::AllocationId,
    pass: bool,
    bound: Duration,
) -> [u64; 2] {
    let deadline = tokio::time::Instant::now() + bound;
    let mut last = [None, None];
    loop {
        let snapshot = lane.snapshot(service).await;
        assert_eq!(
            snapshot.rows.iter().filter(|row| row.alloc_id == alloc.to_string()).count(),
            1,
            "probe state stays on the same peer allocation"
        );
        let tcp = snapshot.probes.iter().find(|probe| {
            probe.role == ProbeRole::Readiness
                && probe.idx.get() == 0
                && matches!(probe.mechanic, ProbeMechanic::Tcp { port: V6_TCP_PROBE_PORT, .. })
        });
        let http = snapshot.probes.iter().find(|probe| {
            probe.role == ProbeRole::Readiness
                && probe.idx.get() == 1
                && matches!(probe.mechanic, ProbeMechanic::Http { port: V6_HTTP_PROBE_PORT, ref path, .. } if path == "/healthz")
        });
        assert!(tcp.is_some() && http.is_some(), "the exact TCP and HTTP descriptors are public");
        let mut times = [None, None];
        for result in snapshot
            .probe_results
            .iter()
            .filter(|row| row.alloc_id == alloc.to_string() && row.role == "readiness")
        {
            let expected = matches!(result.status, ProbeStatusJson::Pass) == pass;
            if expected && result.probe_idx < 2 {
                times[result.probe_idx as usize] = Some(result.last_observed_at_unix_ms);
            }
        }
        if let [Some(tcp), Some(http)] = times {
            return [tcp, http];
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "both probe results reach pass={pass}; probes={:#?}; results={:#?}; last={last:#?}; \
             peer console:\n{}",
            snapshot.probes,
            snapshot.probe_results,
            console_of(alloc)
        );
        last = times;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
