//! Per-flow policy and the host-internal deny set at the owner (R5-6 owner
//! order, R5-7 TCP projection, R5-8 owner branches; D26, D26-PINS F-4/F-6,
//! D14, D9, D24a, ADR-0162, ADR-0167).
//!
//! Design: `feature-delta.md` § *Owner and provisioner* → "Per-flow policy
//! and the host-internal deny set (owner)" (first match decides: static
//! `Policy`, static `HostInternal`, mesh resolution, local-delivery
//! `HostInternal`, the datagram kernel-peer check) and "Kernel enforcement"
//! (F-4: a TCP connect the rules reject → `Refused(DestinationUnreachable)`);
//! § *Per-kind total orders* → TcpConnect steps 4–5 (`Mesh(b)` → bind D on
//! loopback, register the forwarded outbound flow, connect D to leg-F,
//! unmarked and with no linger; `NonMesh` → D marked with `SO_LINGER{1,0}`
//! to the destination; `MeshUnreachable` / a resolve error → refused).
//!
//! The owner's decisions are driven through flows over the seam sim, with a
//! scripted route type per destination and scripted resolutions at the
//! `MtlsResolve` port; each refusal is flow-local: the CID's phase, session
//! and other flows are unchanged. The kernel-rule half is Tier-3 (R5-48,
//! V-21); the adapter's VIP branch is `mtls_resolve_adapter` (step 11).

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddrV4};

use overdrive_core::guest_transport::GUEST_NODE_GATEWAY;
use overdrive_core::guest_transport::wire::{ControlMessage, FlowRefusal};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, ResolvedBackend};
use overdrive_netlink::FibRouteType;
use overdrive_sim::adapters::SimMtlsResolve;

use super::support::{Composition, remote, sent_to};
use crate::guest_network::GuestFlowPhase;
use crate::guest_network::flow_io::sim::{SimEffectKind, SimEffectTarget, SimFailure};
use crate::guest_network::flow_io::{PeerProtocol, PeerSocketSpec};

fn addr(a: u8, b: u8, c: u8, d: u8, port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), port)
}

fn peer_specs(c: &Composition) -> Vec<PeerSocketSpec> {
    c.io.issued()
        .into_iter()
        .filter_map(|effect| match effect.target {
            SimEffectTarget::Peer(spec) if effect.kind == SimEffectKind::OpenPeer => Some(spec),
            _ => None,
        })
        .collect()
}

/// R5-6 (D26): guest TCP flows to host-internal or excluded destinations are
/// refused with the first matching class — the host's own addresses and
/// `gateway:22` by their `local` route type, broadcast and anycast by
/// theirs — and each refusal is flow-local; no refused destination is
/// connected.
///
/// CONTRACT_SHAPE: pure-function.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn tcp_flows_to_host_internal_or_excluded_destinations_are_refused_by_class() {
    let c = Composition::booted(90).await;
    let (lease, session) = c.active_lease("r5-6", &[]).await;
    let host_own = addr(192, 0, 2, 10, 8080);
    c.io.set_route_type(*host_own.ip(), FibRouteType::Local);
    c.io.set_route_type(GUEST_NODE_GATEWAY, FibRouteType::Local);
    let broadcast = addr(198, 51, 100, 255, 9);
    c.io.set_route_type(*broadcast.ip(), FibRouteType::Broadcast);
    let anycast = addr(198, 51, 100, 1, 9);
    c.io.set_route_type(*anycast.ip(), FibRouteType::Anycast);
    let cases = [
        (addr(0, 0, 0, 1, 80), FlowRefusal::Policy),
        (addr(224, 0, 0, 9, 80), FlowRefusal::Policy),
        (addr(255, 255, 255, 255, 80), FlowRefusal::Policy),
        (addr(240, 0, 0, 1, 80), FlowRefusal::Policy),
        (addr(127, 0, 0, 1, 22), FlowRefusal::HostInternal),
        (addr(169, 254, 169, 254, 80), FlowRefusal::HostInternal),
        (host_own, FlowRefusal::HostInternal),
        (SocketAddrV4::new(GUEST_NODE_GATEWAY, 22), FlowRefusal::HostInternal),
        (broadcast, FlowRefusal::HostInternal),
        (anycast, FlowRefusal::HostInternal),
    ];
    let before = c.observe().await;
    for (seq, (destination, refusal)) in (1_u32..).zip(cases) {
        let (key, flow) = c.open_tcp_flow(&lease, seq, destination, 5000 + seq);
        c.settle().await;
        assert!(
            sent_to(&c, lease.cid).contains(&ControlMessage::Refused { flow, refusal }),
            "{destination} is refused {refusal:?}"
        );
        assert!(c.io.socket_observation().closed_by_owner.contains(&key));
    }
    let after = c.observe().await;
    assert_eq!(after, before, "every refusal is flow-local");
    assert_eq!(after.cids[&lease.cid].session, Some(session));
    assert_eq!(
        c.io.issued_count(SimEffectKind::ConnectPeer),
        0,
        "no refused destination is connected"
    );
}

/// R5-6 (policy order, the first match decides): a static class decides
/// before the resolution — `127.0.0.1` is `HostInternal` even when the
/// resolution would say `Mesh` (it is never resolved); a mesh-resolved
/// destination is not subject to the local-delivery check (the owner never
/// connects it); `gateway:53` over TCP passes the local-delivery check.
///
/// CONTRACT_SHAPE: pure-function.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (8)"]
async fn the_first_matching_class_decides_in_the_pinned_order() {
    let loopback = addr(127, 0, 0, 1, 443);
    let local_mesh = addr(192, 0, 2, 50, 443);
    let backend = ResolvedBackend { addr: addr(10, 0, 0, 9, 8443), expected_svid: None };
    let resolve = SimMtlsResolve::new(
        BTreeMap::from([
            (loopback, MtlsResolution::Mesh(backend.clone())),
            (local_mesh, MtlsResolution::Mesh(backend)),
        ]),
        MtlsResolution::NonMesh,
    );
    let c = Composition::new(resolve, 96);
    c.boot().await;
    let (lease, _session) = c.active_lease("r5-6-order", &[]).await;
    c.intercept_live(&lease).await;
    c.io.set_route_type(*local_mesh.ip(), FibRouteType::Local);
    c.io.set_route_type(GUEST_NODE_GATEWAY, FibRouteType::Local);

    let (_k1, refused) = c.open_tcp_flow(&lease, 1, loopback, 5001);
    let (k2, _meshed) = c.open_tcp_flow(&lease, 2, local_mesh, 5002);
    let (k3, _dns) = c.open_tcp_flow(&lease, 3, SocketAddrV4::new(GUEST_NODE_GATEWAY, 53), 5003);
    let after = c.observe().await;
    assert!(
        sent_to(&c, lease.cid).contains(&ControlMessage::Refused {
            flow: refused,
            refusal: FlowRefusal::HostInternal
        })
    );
    assert!(matches!(
        after.cids[&lease.cid].guest_flows.get(&k2),
        Some(GuestFlowPhase::Paired { .. })
    ));
    assert!(matches!(
        after.cids[&lease.cid].guest_flows.get(&k3),
        Some(GuestFlowPhase::Paired { .. })
    ));
}

/// R5-7 TCP half (D26-PINS F-4): a non-mesh TCP connect the kernel rejects
/// (`ECONNREFUSED`, the rules' plain `reject`) is refused
/// `DestinationUnreachable`, the flow released.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_rejected_non_mesh_connect_is_destination_unreachable() {
    let c = Composition::booted(93).await;
    let (lease, _session) = c.active_lease("r5-7t", &[]).await;
    c.io.script_failure(
        SimEffectKind::ConnectPeer,
        SimFailure::Io(std::io::ErrorKind::ConnectionRefused),
    );
    let (key, flow) = c.open_tcp_flow(&lease, 1, remote(8443), 5000);
    let after = c.observe().await;
    assert!(
        sent_to(&c, lease.cid).contains(&ControlMessage::Refused {
            flow,
            refusal: FlowRefusal::DestinationUnreachable
        })
    );
    assert!(!after.cids[&lease.cid].guest_flows.contains_key(&key));
    assert_eq!(after.cids[&lease.cid].held_flows, 0);
    assert_eq!(c.io.issued_count(SimEffectKind::Attach), 0);
}

/// D14 / D26-PINS F-6 / H-5: a non-mesh guest TCP flow is connected by the
/// owner directly to its destination on a socket carrying the owner egress
/// mark and `SO_LINGER{1,0}`, unbound (no leg-F loopback hop).
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_non_mesh_flow_is_connected_directly_on_a_marked_lingering_socket() {
    let c = Composition::booted(94).await;
    let (lease, _session) = c.active_lease("d14", &[]).await;
    let destination = remote(443);
    let (_key, _flow, _pair) = c.paired_tcp(&lease, 1, destination, 5000).await;
    assert_eq!(
        peer_specs(&c),
        vec![PeerSocketSpec {
            protocol: PeerProtocol::Tcp,
            owner_egress_mark: true,
            abortive_linger: true,
            bind: None,
        }]
    );
    assert_eq!(c.io.peer_connects(), vec![destination], "straight to the destination");
}

/// R5-8 owner branch `Mesh(b)` (D9): the owner binds D on `127.0.0.1:0`
/// before the connect, registers the forwarded outbound flow, and connects
/// D to leg-F — unmarked and with no linger (both ends in one process);
/// the flow pairs. It never connects the original destination.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (8)"]
async fn a_mesh_destination_is_connected_to_leg_f_on_an_unmarked_loopback_socket() {
    let vip = addr(10, 96, 0, 10, 8080);
    let backend = ResolvedBackend { addr: addr(10, 0, 0, 4, 8080), expected_svid: None };
    let resolve = SimMtlsResolve::new(
        BTreeMap::from([(vip, MtlsResolution::Mesh(backend))]),
        MtlsResolution::NonMesh,
    );
    let c = Composition::new(resolve, 97);
    c.boot().await;
    let (lease, _session) = c.active_lease("r5-8-mesh", &[]).await;
    c.intercept_live(&lease).await;
    let (_key, _flow, _pair) = c.paired_tcp(&lease, 1, vip, 5000).await;
    let specs = peer_specs(&c);
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].protocol, PeerProtocol::Tcp);
    assert!(!specs[0].owner_egress_mark, "the connect to leg-F is unmarked");
    assert!(!specs[0].abortive_linger, "both ends are in one process");
    assert_eq!(specs[0].bind, Some(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)));
    assert!(!c.io.peer_connects().contains(&vip), "the owner never connects a mesh destination");
}

/// R5-8 owner branches: `MeshUnreachable` → `Refused(MeshUnreachable)`, a
/// resolve error → `Refused(ResolveFailed)` — fail closed, never cleartext:
/// no peer socket is opened.
///
/// CONTRACT_SHAPE: pure-function.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_unreachable_mesh_destination_or_a_resolve_error_fails_closed() {
    let unreachable = addr(10, 96, 0, 99, 8080);
    let resolve = SimMtlsResolve::new(
        BTreeMap::from([(unreachable, MtlsResolution::MeshUnreachable)]),
        MtlsResolution::NonMesh,
    );
    let c = Composition::new(resolve, 98);
    c.boot().await;
    let (lease, _session) = c.active_lease("r5-8-closed", &[]).await;
    let (_k1, first) = c.open_tcp_flow(&lease, 1, unreachable, 5000);
    c.settle().await;
    c.resolve.script_resolve_fault("store unreadable");
    let (_k2, second) = c.open_tcp_flow(&lease, 2, remote(443), 5001);
    c.settle().await;
    let sent = sent_to(&c, lease.cid);
    assert!(
        sent.contains(&ControlMessage::Refused {
            flow: first,
            refusal: FlowRefusal::MeshUnreachable
        })
    );
    assert!(
        sent.contains(&ControlMessage::Refused {
            flow: second,
            refusal: FlowRefusal::ResolveFailed
        })
    );
    assert!(peer_specs(&c).is_empty(), "no socket toward a fail-closed destination");
}

/// D9 precondition: a mesh flow of an allocation that is not intercept-live
/// is refused `IntakeUnavailable` (`register_forwarded_outbound` returns
/// `NotInterceptLive`), its D closed, nothing connected.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (8)"]
async fn a_mesh_flow_of_an_allocation_not_intercept_live_is_intake_unavailable() {
    let vip = addr(10, 96, 0, 11, 8080);
    let backend = ResolvedBackend { addr: addr(10, 0, 0, 5, 8080), expected_svid: None };
    let resolve = SimMtlsResolve::new(
        BTreeMap::from([(vip, MtlsResolution::Mesh(backend))]),
        MtlsResolution::NonMesh,
    );
    let c = Composition::new(resolve, 99);
    c.boot().await;
    let (lease, _session) = c.active_lease("d9-not-live", &[]).await;
    let (key, flow) = c.open_tcp_flow(&lease, 1, vip, 5000);
    c.settle().await;
    assert!(
        sent_to(&c, lease.cid)
            .contains(&ControlMessage::Refused { flow, refusal: FlowRefusal::IntakeUnavailable })
    );
    assert!(c.io.socket_observation().closed_by_owner.contains(&key));
    assert!(c.io.peer_connects().is_empty());
}
