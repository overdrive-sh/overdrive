//! Host datagram associations (EVMOD-DGRAM, ADR-0201; D10, #310, D24,
//! D26-PINS F-4): R5-7 owner projection, R5-27 host half, the `gateway:53`
//! exemption, the VIP rewrite by `connect4`, framing errors and the U-5
//! discard of unknown flow ids.
//!
//! Design: `feature-delta.md` § *Per-kind total orders* → "Datagram
//! association" (host steps 3–4: one 1240 stream with its `DatagramStream`
//! request, UDP socket H connected, the kernel-peer check, then attach
//! `V_h`–H `Datagram { udp: B }` and `Paired`); § *[REF] vsock data path* →
//! "Datagram mode" and "Failure projection" (any `SocketError` on H's half
//! ends the association `DestinationUnreachable`; a bad frame
//! `FrameInvalid`); § *Owner and provisioner* → "Per-flow policy and the
//! host-internal deny set" and "Kernel enforcement" (F-4). Quint oracles:
//! `udp_slots` host side (`slots_conf`, replayed by `quint_udp_slots`) and
//! `fwd_pair`'s datagram runs (`pair_conf`).
//!
//! There is no empty-datagram notice, ring or compensation: every datagram,
//! an empty one included, is a frame the forwarder carries in order
//! (EVMOD-DGRAM; the `udp_empty` module is deleted).

use std::net::{Ipv4Addr, SocketAddrV4};

use overdrive_core::guest_transport::wire::{
    ControlMessage, DatagramFrame, FlowId, FlowRefusal, KernelErrno,
};
use overdrive_core::guest_transport::{GUEST_NODE_GATEWAY, GUEST_NODE_PREFIX};
use overdrive_netlink::FibRouteType;

use super::support::{Composition, EventCapture, Lease, guest_flow, remote, sent_to};
use crate::guest_network::flow_io::PeerProtocol;
use crate::guest_network::flow_io::sim::{SimEffectKind, SimEffectTarget};
use crate::guest_network::{AssociationPhase, FlowCloseCause, GuestConnKey};

/// The D26 rule's verdict on a forwarded datagram's transmit (`EPERM`).
fn eperm() -> KernelErrno {
    KernelErrno::new(1).expect("EPERM is non-zero")
}

struct Association {
    key: GuestConnKey,
    flow: FlowId,
}

async fn associate(
    c: &Composition,
    lease: &Lease,
    seq: u32,
    destination: SocketAddrV4,
) -> Association {
    let (key, flow) = c.open_datagram(lease, seq, destination, 6000 + seq);
    c.settle().await;
    assert!(
        sent_to(c, lease.cid).contains(&ControlMessage::Paired { flow }),
        "the association pairs"
    );
    Association { key, flow }
}

fn closed_with(capture: &EventCapture, cause: FlowCloseCause) -> usize {
    capture
        .named("guest_flow.closed")
        .iter()
        .filter(|event| event.field("cause") == Some(cause.as_str()))
        .count()
}

/// EVMOD-DGRAM host order: the association's request is read, H is made,
/// connected and attached in datagram mode (`udp` = B) before `Paired`; two
/// associations of one VM pair independently, each counted against the
/// quota.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn concurrent_associations_pair_independently_and_count_against_the_quota() {
    let c = Composition::booted(80).await;
    let (lease, _session) = c.active_lease("dgram-pair", &[]).await;
    let first = associate(&c, &lease, 1, remote(53)).await;
    let second = associate(&c, &lease, 2, remote(123)).await;
    let observed = c.observe().await;
    let cid = &observed.cids[&lease.cid];
    let (a, b) = (cid.datagrams[&first.flow].phase, cid.datagrams[&second.flow].phase);
    let (
        AssociationPhase::Paired { pair: pa, generation: ga },
        AssociationPhase::Paired { pair: pb, generation: gb },
    ) = (a, b)
    else {
        panic!("both associations are Paired: {a:?} {b:?}");
    };
    assert_ne!(pa, pb);
    assert_eq!((ga, gb), (observed.forwarder_generation, observed.forwarder_generation));
    assert_eq!(cid.held_flows, 2);
    let modes: Vec<_> =
        c.io.issued()
            .into_iter()
            .filter(|effect| effect.kind == SimEffectKind::OpenPeer)
            .filter_map(|effect| match effect.target {
                SimEffectTarget::Peer(spec) => Some(spec.protocol),
                _ => None,
            })
            .collect();
    assert_eq!(modes, vec![PeerProtocol::Udp, PeerProtocol::Udp]);
}

/// EVMOD-DGRAM order and empties: datagrams both ways cross the
/// association whole and in order, an empty one included.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn datagrams_cross_whole_and_in_order_with_an_empty_one() {
    let c = Composition::booted(88).await;
    let (lease, _session) = c.active_lease("dgram-order", &[]).await;
    let association = associate(&c, &lease, 1, remote(9)).await;
    c.io.destination_sends(association.flow, b"one");
    c.io.destination_sends(association.flow, b"");
    c.io.destination_sends(association.flow, b"three");
    let frames = c.io.vsock_of(association.flow).peer_received();
    assert!(!frames.is_empty(), "the frames reach the guest's vsock");
    // The guest's own frames reach the destination as whole datagrams.
    let guest_frames: Vec<u8> = [b"ab".as_slice(), b"".as_slice()]
        .iter()
        .flat_map(|payload| DatagramFrame::new(payload).expect("within the bound").encode())
        .collect();
    c.io.guest_sends(association.flow, &guest_frames);
    assert_eq!(
        c.io.peer_of(association.flow).peer_datagrams(),
        vec![b"ab".to_vec(), Vec::new()],
        "every datagram once, whole and in order, the empty one included"
    );
}

/// D10 / #310 (host half): `gateway:53` is the one guest-prefix UDP
/// destination not refused `Policy` — the DNS association pairs although the
/// gateway is host-local (GW-LOCAL); another guest-prefix address is refused
/// `Policy`, its `V_h` closed, no UDP socket made.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn dns_to_the_gateway_pairs_and_other_prefix_addresses_are_policy() {
    let c = Composition::booted(81).await;
    let (lease, _session) = c.active_lease("dgram-dns", &[]).await;
    c.io.set_route_type(GUEST_NODE_GATEWAY, FibRouteType::Local);
    let dns = associate(&c, &lease, 1, SocketAddrV4::new(GUEST_NODE_GATEWAY, 53)).await;
    assert!(c.observe().await.cids[&lease.cid].datagrams.contains_key(&dns.flow));

    let udp_sockets_before = c.io.issued_count(SimEffectKind::OpenPeer);
    let other = SocketAddrV4::new(Ipv4Addr::from(u32::from(GUEST_NODE_PREFIX.network()) + 9), 53);
    let (key, flow) = c.open_datagram(&lease, 2, other, 7000);
    c.settle().await;
    assert!(
        sent_to(&c, lease.cid)
            .contains(&ControlMessage::Refused { flow, refusal: FlowRefusal::Policy })
    );
    assert!(c.io.socket_observation().closed_by_owner.contains(&key));
    assert_eq!(c.io.issued_count(SimEffectKind::OpenPeer), udp_sockets_before);
}

/// D24 (host step 4): a UDP association to a service VIP is connected
/// unmarked and rewritten by `connect4` to a backend; the kernel peer is
/// outside the guest prefix, so the association pairs. A VIP rewritten to an
/// address inside the guest prefix (other than `gateway:53`) is refused
/// `Policy` by the kernel-peer check (#310).
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER step 11"]
async fn a_vip_association_is_rewritten_by_connect4_and_checked_on_its_kernel_peer() {
    let c = Composition::booted(89).await;
    let (lease, _session) = c.active_lease("dgram-vip", &[]).await;
    let vip = SocketAddrV4::new(Ipv4Addr::new(10, 96, 0, 20), 53);
    c.io.set_connect4_rewrite(vip, SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 20), 5353));
    let routed = associate(&c, &lease, 1, vip).await;
    assert!(c.observe().await.cids[&lease.cid].datagrams.contains_key(&routed.flow));
    let marks: Vec<_> =
        c.io.issued()
            .into_iter()
            .filter_map(|effect| match effect.target {
                SimEffectTarget::Peer(spec) if effect.kind == SimEffectKind::OpenPeer => {
                    Some(spec.owner_egress_mark)
                }
                _ => None,
            })
            .collect();
    assert_eq!(marks, vec![false], "a datagram to a service VIP carries no owner egress mark");

    let into_prefix = SocketAddrV4::new(Ipv4Addr::new(10, 96, 0, 21), 53);
    let workload =
        SocketAddrV4::new(Ipv4Addr::from(u32::from(GUEST_NODE_PREFIX.network()) + 30), 53);
    c.io.set_connect4_rewrite(into_prefix, workload);
    let (key, flow) = c.open_datagram(&lease, 2, into_prefix, 7002);
    c.settle().await;
    assert!(
        sent_to(&c, lease.cid)
            .contains(&ControlMessage::Refused { flow, refusal: FlowRefusal::Policy })
    );
    assert!(c.io.socket_observation().closed_by_owner.contains(&key));
}

/// R5-7 owner projection (D26-PINS F-4): the forwarder's send of a forwarded
/// datagram on H is rejected `EPERM` by the host-internal rule (an address
/// made local after the owner's check); the pair aborts `SocketError` on its
/// UDP half; the owner ends the association with close cause
/// `DestinationUnreachable`; the guest's next association to that address
/// is refused `HostInternal` by the owner's check.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_transmit_rejected_on_h_ends_the_association_destination_unreachable() {
    let (capture, _guard) = EventCapture::install();
    let c = Composition::booted(92).await;
    let (lease, _session) = c.active_lease("r5-7", &[]).await;
    let late_local = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 77), 5353);
    let association = associate(&c, &lease, 1, late_local).await;
    c.io.set_route_type(*late_local.ip(), FibRouteType::Local);
    c.io.destination_socket_error(association.flow, eperm());
    let ended = c.observe().await;
    assert!(!ended.cids[&lease.cid].datagrams.contains_key(&association.flow));
    assert_eq!(ended.cids[&lease.cid].held_flows, 0);
    assert_eq!(closed_with(&capture, FlowCloseCause::DestinationUnreachable), 1);
    assert!(
        c.io.peer_of(association.flow).peer_datagrams().is_empty(),
        "nothing reached the host service"
    );

    let (_key, next) = c.open_datagram(&lease, 2, late_local, 7004);
    c.settle().await;
    assert!(
        sent_to(&c, lease.cid)
            .contains(&ControlMessage::Refused { flow: next, refusal: FlowRefusal::HostInternal })
    );
}

/// R5-27 (host half, U-2): an association to a destination refused `Policy`
/// or `HostInternal` holds nothing at the host; each new request is a new
/// attempt refused the same way — the host keeps no association, no socket
/// and no quota place between attempts.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_refused_association_holds_nothing_between_attempts() {
    let c = Composition::booted(93).await;
    let (lease, _session) = c.active_lease("r5-27", &[]).await;
    let refused = SocketAddrV4::new(Ipv4Addr::new(169, 254, 169, 254), 53);
    for seq in 1..=3 {
        let (key, flow) = c.open_datagram(&lease, seq, refused, 7100 + seq);
        let after = c.observe().await;
        assert!(
            sent_to(&c, lease.cid)
                .contains(&ControlMessage::Refused { flow, refusal: FlowRefusal::HostInternal })
        );
        assert!(after.cids[&lease.cid].datagrams.is_empty());
        assert_eq!(after.cids[&lease.cid].held_flows, 0);
        assert!(c.io.socket_observation().closed_by_owner.contains(&key));
    }
    assert_eq!(
        c.io.issued_count(SimEffectKind::OpenPeer),
        0,
        "a refused destination gets no UDP socket"
    );
}

/// EVMOD-DGRAM (the guest's slot release): the guest aborts its pair — the
/// association's vsock is shut both ways — and the host ends the
/// association at its `PeerAbort` record (`PeerReset`), releasing its
/// place.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_guests_slot_release_ends_the_association() {
    let (capture, _guard) = EventCapture::install();
    let c = Composition::booted(94).await;
    let (lease, _session) = c.active_lease("dgram-release", &[]).await;
    let association = associate(&c, &lease, 1, remote(9)).await;
    c.io.guest_closes(association.flow);
    let ended = c.observe().await;
    assert!(!ended.cids[&lease.cid].datagrams.contains_key(&association.flow));
    assert_eq!(ended.cids[&lease.cid].held_flows, 0);
    assert!(c.io.socket_observation().closed_by_owner.contains(&association.key));
    assert_eq!(closed_with(&capture, FlowCloseCause::PeerReset), 1);
}

/// R5-51 (framing): a frame with a bad magic from the guest aborts the pair
/// `FrameInvalid`; the owner ends the association with that close cause.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_bad_frame_aborts_the_association_frame_invalid() {
    let (capture, _guard) = EventCapture::install();
    let c = Composition::booted(95).await;
    let (lease, _session) = c.active_lease("dgram-frame", &[]).await;
    let association = associate(&c, &lease, 1, remote(9)).await;
    c.io.guest_sends(association.flow, b"XXXX\0\0\0\x02ab");
    let ended = c.observe().await;
    assert!(!ended.cids[&lease.cid].datagrams.contains_key(&association.flow));
    assert_eq!(closed_with(&capture, FlowCloseCause::FrameInvalid), 1);
    assert!(c.io.peer_of(association.flow).peer_datagrams().is_empty());
}

/// U-5 (host half): an `Abort` or `Paired` from the guest for a flow id the
/// owner does not hold is discarded with no reply and no state change
/// (`guest_flow.control_discarded`); the guest's `Abort` for a live
/// association ends it.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn control_messages_for_unknown_flows_are_discarded_and_an_abort_ends_its_association() {
    let (capture, _guard) = EventCapture::install();
    let c = Composition::booted(82).await;
    let (lease, session) = c.active_lease("u-5", &[]).await;
    let live = associate(&c, &lease, 1, remote(53)).await;
    let before = c.observe().await;
    let replies = sent_to(&c, lease.cid);
    let unknown = guest_flow(77);
    c.guest_sends(session, ControlMessage::Abort { flow: unknown });
    c.guest_sends(session, ControlMessage::Paired { flow: unknown });
    assert_eq!(c.observe().await, before);
    assert_eq!(sent_to(&c, lease.cid), replies, "no reply to a discarded message");
    assert_eq!(capture.named("guest_flow.control_discarded").len(), 2);

    c.guest_sends(session, ControlMessage::Abort { flow: live.flow });
    let ended = c.observe().await;
    assert!(!ended.cids[&lease.cid].datagrams.contains_key(&live.flow));
    assert_eq!(ended.cids[&lease.cid].held_flows, 0);
    assert!(c.io.pair_of(live.flow).is_none());
}
