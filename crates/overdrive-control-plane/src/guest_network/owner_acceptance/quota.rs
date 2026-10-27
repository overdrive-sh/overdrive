//! The per-allocation flow quota, 4,096, half-closed flows included
//! (ADR-0208), released at the flow's end record (EVMOD-AB, ADR-0209)
//! — R5-50, R5-20 (quota exhaustion is the allocation's alone), D-UP-17
//! (`held_flows` = guest flows + datagrams + `Relay` dials).
//!
//! Design: `feature-delta.md` § *Owner and provisioner*
//! (`GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION`: every flow counts from its
//! registration until the host releases it — at its end record, or for a
//! dial at its stream's drop or the owner's abort; "Exposure": a guest that
//! withholds credit, or an application that does not read, holds at most its
//! own quota). Quint oracle: `flow_quota` (`quota_conf`), replayed by
//! `quint_flow_quota`.
//!
//! N ∈ {0, 1, 4,095, 4,096, 4,097}: the counts at the boundary are exercised
//! through the owner's admission with real flows, never a written counter;
//! every end is a cause scripted on the flow's sockets.

use overdrive_core::guest_transport::dial::DialPurpose;
use overdrive_core::guest_transport::wire::{ControlMessage, FlowRefusal};

use super::support::{Composition, Lease, remote, sent_to};
use crate::guest_network::{GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION, GuestFlowPhase};

/// Opens `count` TcpConnect flows for `lease`, flow ids from `first`; each
/// pairs.
async fn fill(c: &Composition, lease: &Lease, first: u32, count: u32) {
    for seq in first..first + count {
        let _opened = c.open_tcp_flow(lease, seq, remote(443), 10_000 + seq);
    }
    c.settle().await;
}

/// R5-50 / quota boundary: an allocation is admitted up to its quota; the
/// next flow is refused `Capacity` with no other change; another
/// allocation is still admitted (the quota is per allocation, the bulkhead).
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn admission_stops_at_the_allocation_quota_and_spares_other_allocations() {
    let c = Composition::booted(100).await;
    let (lease, _session) = c.active_lease("quota-a", &[]).await;
    assert_eq!(c.observe().await.cids[&lease.cid].held_flows, 0);
    fill(&c, &lease, 1, 1).await;
    assert_eq!(c.observe().await.cids[&lease.cid].held_flows, 1);
    fill(&c, &lease, 2, GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION - 2).await;
    assert_eq!(
        c.observe().await.cids[&lease.cid].held_flows,
        GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION - 1
    );
    fill(&c, &lease, GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION, 1).await;
    let full = c.observe().await;
    assert_eq!(full.cids[&lease.cid].held_flows, GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION);

    let (refused_key, refused) =
        c.open_tcp_flow(&lease, GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION + 1, remote(443), 9_000);
    let after = c.observe().await;
    assert!(
        sent_to(&c, lease.cid)
            .contains(&ControlMessage::Refused { flow: refused, refusal: FlowRefusal::Capacity })
    );
    assert!(c.io.socket_observation().closed_by_owner.contains(&refused_key));
    assert_eq!(after.cids[&lease.cid], full.cids[&lease.cid], "the refusal changes nothing");

    let (other, _other_session) = c.active_lease("quota-b", &[]).await;
    let (_key, admitted) = c.open_tcp_flow(&other, 1, remote(443), 5_000);
    c.settle().await;
    assert!(sent_to(&c, other.cid).contains(&ControlMessage::Paired { flow: admitted }));
}

/// R5-50 (ADR-0208): a half-closed flow whose guest application never reads
/// still counts against the quota — a guest that withholds credit holds at
/// most its own quota; when its pair completes the owner releases the place
/// at the `Completed` record and a new flow is admitted.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_half_closed_flow_counts_against_the_quota_until_its_end_record() {
    let c = Composition::booted(101).await;
    let (lease, _session) = c.active_lease("quota-half", &[]).await;
    let (half_key, half, _pair) = c.paired_tcp(&lease, 1, remote(443), 5_000).await;
    c.io.destination_sends(half, &[9_u8; 1024]);
    c.io.destination_fins(half);
    assert!(matches!(
        c.observe().await.cids[&lease.cid].guest_flows.get(&half_key),
        Some(GuestFlowPhase::Paired { .. })
    ));
    fill(&c, &lease, 2, GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION - 1).await;
    assert_eq!(c.observe().await.cids[&lease.cid].held_flows, GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION);

    let (_key, refused) =
        c.open_tcp_flow(&lease, GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION + 1, remote(443), 9_000);
    c.settle().await;
    assert!(
        sent_to(&c, lease.cid)
            .contains(&ControlMessage::Refused { flow: refused, refusal: FlowRefusal::Capacity })
    );

    c.io.guest_half_closes(half);
    let released = c.observe().await;
    assert!(!released.cids[&lease.cid].guest_flows.contains_key(&half_key));
    assert_eq!(released.cids[&lease.cid].held_flows, GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION - 1);
    let (_key, admitted) =
        c.open_tcp_flow(&lease, GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION + 2, remote(443), 9_001);
    c.settle().await;
    assert!(sent_to(&c, lease.cid).contains(&ControlMessage::Paired { flow: admitted }));
}

/// R5-50 / EVMOD-AB: aborting a half-closed flow releases its place at its
/// abort record, at once.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_abort_of_a_half_closed_flow_releases_its_place_at_its_record() {
    let c = Composition::booted(102).await;
    let (lease, _session) = c.active_lease("quota-abort", &[]).await;
    let (key, flow, _pair) = c.paired_tcp(&lease, 1, remote(443), 5_000).await;
    c.io.destination_fins(flow);
    assert_eq!(c.observe().await.cids[&lease.cid].held_flows, 1);
    c.io.guest_closes(flow);
    let aborted = c.observe().await;
    assert!(!aborted.cids[&lease.cid].guest_flows.contains_key(&key));
    assert_eq!(aborted.cids[&lease.cid].held_flows, 0);
    assert!(c.io.pair_of(flow).is_none(), "the pair's record was read");
    assert_eq!(c.io.forwarder_inventory().live_pairs, 0);
}

/// D-UP-17 (QuotaBound): `held_flows` counts guest-opened flows,
/// datagram associations and `Relay` dials, and not `Probe` dials.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn held_flows_counts_flows_associations_and_relay_dials_only() {
    let c = Composition::booted(103).await;
    let (lease, session) = c.active_lease("quota-mix", &[8080]).await;
    let (_tcp_key, _tcp, _tcp_pair) = c.paired_tcp(&lease, 1, remote(443), 5_000).await;
    let (_dg_key, _dg) = c.open_datagram(&lease, 2, remote(53), 5_001);
    c.settle().await;
    let (_relay, _relay_flow) =
        c.paired_dial(session, lease.target(8080), DialPurpose::Relay, 0).await;
    let (_probe, _probe_flow) =
        c.paired_dial(session, lease.target(8080), DialPurpose::Probe, 1).await;
    let observed = c.observe().await;
    let cid = &observed.cids[&lease.cid];
    assert_eq!(cid.dials.len(), 2);
    assert_eq!(
        cid.held_flows, 3,
        "one TcpConnect, one association, one Relay dial; the probe is exempt"
    );
}

/// R5-20 / D15-R3: 4,096 connects to unresponsive non-mesh destinations
/// exhaust only that VM's quota — another VM's flows still pair.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn unresponsive_connects_exhaust_only_their_own_vms_quota() {
    let c = Composition::booted(104).await;
    let (stuck, _stuck_session) = c.active_lease("quota-stuck", &[]).await;
    c.io.hold(crate::guest_network::flow_io::sim::SimEffectKind::ConnectPeer);
    fill(&c, &stuck, 1, GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION).await;
    assert_eq!(c.observe().await.cids[&stuck.cid].held_flows, GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION);
    let (_k, refused) =
        c.open_tcp_flow(&stuck, GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION + 1, remote(443), 9_000);
    c.settle().await;
    assert!(
        sent_to(&c, stuck.cid)
            .contains(&ControlMessage::Refused { flow: refused, refusal: FlowRefusal::Capacity })
    );

    c.io.auto_release(crate::guest_network::flow_io::sim::SimEffectKind::ConnectPeer);
    let (healthy, _healthy_session) = c.active_lease("quota-healthy", &[]).await;
    let (_key, flow) = c.open_tcp_flow(&healthy, 1, remote(443), 5_000);
    c.settle().await;
    assert!(sent_to(&c, healthy.cid).contains(&ControlMessage::Paired { flow }));
}
