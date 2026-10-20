//! The host owner's end of a guest-opened TCP flow, from the forwarder's
//! end records (EVMOD-REC, EVMOD-END, EVMOD-AB, EVMOD-ATTACH-END; R2-END
//! ADR-0180, R2-DRAIN ADR-0183; R5-10 and R5-45 sim parts, R5-49 owner
//! rules, R5-62 sim part; G-V9 rows 1–5).
//!
//! Design: `feature-delta.md` § *Owner and provisioner* → "Flow end",
//! "Owner flow rules (T1–T6)", "Remote-side linger (H-5)", "Telemetry";
//! § *[REF] vsock data path* → "End classification", "Pair end", "Failure
//! projection" and "Owner response to an ended half at attach";
//! § *Lifecycle Gate Ownership*, G-V9 and "Forwarder backpressure". Quint
//! oracles: `fwd_pair` (`pair_conf`, replayed by `quint_fwd_pair`) and
//! `flow_quota` (`quota_conf`).
//!
//! Every end is a cause scripted on a socket — the guest's `0x2`, `0x3` or
//! `RST` on the flow vsock, the destination's FIN or reset, a stalled
//! reader, the device release's `sk_err` — that the `SimFlowForwarder`
//! arbiter classifies and turns into the pair's one end record. The owner
//! runs no drain step: it acts only on the record (release, linger, close,
//! close cause) or on its own abort. Oracles: QPROJ-3 `flow_observation()`
//! (the flow leaves its table and `held_flows` drops in the dispatch that
//! reads the record), QPROJ-7 `socket_observation().far_end`, the Sim
//! sockets' far-reader facts (QPROJ-8 arbiter), the control messages on the
//! session, the effects the owner issued and its `guest_flow.closed` /
//! `guest_flow.unclassified_end` events. Layer 3, example-based.

use overdrive_core::guest_transport::forwarder::{ForwardId, SourceEnd};
use overdrive_core::guest_transport::wire::{ControlMessage, FlowId, KernelErrno};
use overdrive_sim::adapters::SimShutdown;

use super::support::{Composition, EventCapture, Lease, remote, sent_to};
use crate::guest_network::flow_io::sim::{FarEndEvent, SimEffectKind, SimEffectTarget, SimPeerEnd};
use crate::guest_network::{
    FlowCloseCause, GuestConnKey, GuestFlowPhase, SharedGuestNetworkOwner as _,
};

struct Paired {
    c: Composition,
    lease: Lease,
    key: GuestConnKey,
    flow: FlowId,
    /// The pair's id in the owner's context, from its `Paired` phase.
    pair: ForwardId,
}

async fn paired(seed: u64) -> Paired {
    let c = Composition::booted(seed).await;
    let (lease, _session) = c.active_lease("flow-end", &[]).await;
    let (key, flow, pair) = c.paired_tcp(&lease, 1, remote(443), 5000).await;
    Paired { c, lease, key, flow, pair }
}

impl Paired {
    async fn held(&self) -> bool {
        self.c.observe().await.cids[&self.lease.cid].guest_flows.contains_key(&self.key)
    }

    async fn held_flows(&self) -> u32 {
        self.c.observe().await.cids[&self.lease.cid].held_flows
    }

    fn far_end(&self) -> Option<FarEndEvent> {
        self.c.io.socket_observation().far_end.get(&self.flow).copied()
    }

    /// The vsock SHUTDOWN the host's forwarder sent toward the guest.
    fn to_guest(&self) -> Option<SimShutdown> {
        self.c.io.vsock_of(self.flow).sent_shutdown()
    }
}

/// `guest_flow.closed` events for `cause`.
fn closed_with(capture: &EventCapture, cause: FlowCloseCause) -> usize {
    capture
        .named("guest_flow.closed")
        .iter()
        .filter(|event| event.field("cause") == Some(cause.as_str()))
        .count()
}

/// R2-DRAIN happy path, both ways (G-V9 row 1, CLEAN-FULL-END): the
/// destination writes and FINs, the guest writes and half-closes; every byte
/// reaches each far reader before its end, the pair completes, and the owner
/// — on the one `Completed` record — clears the TCP half's linger, closes,
/// releases the flow and records `PeerEof`; no `Abort` is sent.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn a_flow_ended_cleanly_both_ways_completes_and_closes_peer_eof() {
    let (capture, _guard) = EventCapture::install();
    let p = paired(60).await;
    p.c.io.destination_sends(p.flow, b"reply");
    p.c.io.destination_fins(p.flow);
    assert!(p.held().await, "a half-closed flow is still held (ADR-0208)");
    let state = p.c.io.pair_state_of(p.flow).expect("the pair is live");
    assert!(
        state.a_to_b.source_end.is_none() ^ state.b_to_a.source_end.is_none(),
        "exactly one direction has ended"
    );
    assert_eq!(p.c.io.vsock_of(p.flow).peer_received(), b"reply".to_vec());
    assert_eq!(
        p.to_guest(),
        Some(SimShutdown::Send),
        "the destination's FIN reaches the guest as 0x2"
    );

    p.c.io.guest_sends(p.flow, b"request");
    p.c.io.guest_half_closes(p.flow);
    let closed = p.c.observe().await;
    assert!(
        !closed.cids[&p.lease.cid].guest_flows.contains_key(&p.key),
        "released at its end record"
    );
    assert_eq!(closed.cids[&p.lease.cid].held_flows, 0);
    assert_eq!(p.c.io.peer_of(p.flow).peer_received(), b"request".to_vec());
    assert_eq!(p.far_end(), Some(FarEndEvent::Eof), "the destination read a clean end");
    let kinds: Vec<_> = p.c.io.issued().into_iter().map(|effect| effect.kind).collect();
    let linger =
        kinds.iter().position(|kind| *kind == SimEffectKind::ClearLinger).expect("linger cleared");
    let close = kinds.iter().rposition(|kind| *kind == SimEffectKind::Close).expect("closed");
    assert!(linger < close, "linger is cleared before the close of a completed TCP half");
    assert!(!sent_to(&p.c, p.lease.cid).contains(&ControlMessage::Abort { flow: p.flow }));
    assert_eq!(closed_with(&capture, FlowCloseCause::PeerEof), 1);
}

/// R2-DRAIN / R5-49 (G-V9 rows 1 and 3): the guest's `0x2` is a clean
/// half-close of its direction — carried to the destination as a FIN after
/// the last byte it accepted, with no owner step; the other direction stays
/// open and the flow keeps its quota place until its end record.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn the_guests_half_close_reaches_the_destination_after_its_bytes() {
    let p = paired(61).await;
    p.c.io.guest_sends(p.flow, b"abc");
    p.c.io.guest_half_closes(p.flow);
    let peer = p.c.io.peer_of(p.flow);
    assert_eq!(peer.peer_received(), b"abc".to_vec());
    assert_eq!(peer.peer_end(), None, "a half-close is not a terminal event of the far end");
    let state = p.c.io.pair_state_of(p.flow).expect("still live");
    let ended: Vec<_> = [state.a_to_b, state.b_to_a]
        .into_iter()
        .filter(|direction| direction.source_end == Some(SourceEnd::Clean))
        .collect();
    assert_eq!(ended.len(), 1);
    assert!(ended[0].target_shut, "the forwarder shut the target after the last accepted byte");
    assert!(p.held().await);
    assert_eq!(p.held_flows().await, 1);
    let owner_effects_after_pairing = p
        .c
        .io
        .issued()
        .into_iter()
        .filter(|effect| matches!(effect.kind, SimEffectKind::ClearLinger | SimEffectKind::Close))
        .count();
    assert_eq!(owner_effects_after_pairing, 0, "no owner step for a half-close (no drain step)");
}

/// R5-45(b) (G-V9 row 2): the guest's `0x3` with its direction open is an
/// abort — the forwarder resets the destination; on the `PeerAbort` record
/// the owner closes and releases the flow with `PeerReset`; the destination
/// never reads a clean end; no `Abort` message (the guest learns of it from
/// its own forwarder).
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn the_guests_close_with_a_direction_open_aborts_the_flow() {
    let (capture, _guard) = EventCapture::install();
    let p = paired(62).await;
    p.c.io.guest_closes(p.flow);
    assert!(!p.held().await);
    assert_eq!(p.held_flows().await, 0);
    assert_eq!(p.far_end(), Some(FarEndEvent::Reset));
    assert!(!sent_to(&p.c, p.lease.cid).contains(&ControlMessage::Abort { flow: p.flow }));
    assert_eq!(closed_with(&capture, FlowCloseCause::PeerReset), 1);
}

/// R5-45(b): a guest `RST` aborts the flow the same way.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn a_guest_reset_aborts_the_flow() {
    let p = paired(63).await;
    p.c.io.guest_resets(p.flow);
    assert!(!p.held().await);
    assert_eq!(p.far_end(), Some(FarEndEvent::Reset));
}

/// R5-45(c): the destination's reset is a `SocketError` on the TCP half —
/// also after its FIN (the reset reaches no `recvmsg`, only
/// `sk_error_report`, hazard 16) — and aborts the pair: the guest receives
/// `0x3`, never a clean end, and the owner releases the flow `PeerReset`.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn a_destination_reset_after_its_fin_aborts_the_flow() {
    let (capture, _guard) = EventCapture::install();
    let p = paired(64).await;
    p.c.io.destination_fins(p.flow);
    assert!(p.held().await, "the FIN alone half-closes");
    p.c.io.destination_resets(p.flow);
    assert!(!p.held().await);
    assert_eq!(p.to_guest(), Some(SimShutdown::Both), "the guest's vsock is shut both ways");
    assert_eq!(closed_with(&capture, FlowCloseCause::PeerReset), 1);
}

/// EVMOD-END (U-EV-7): an end-of-stream on the vsock half whose class was
/// never recorded — a broken invariant — is aborted `Unclassified`; the
/// owner treats it as an abort (`PeerReset`) and raises the alarm
/// `guest_flow.unclassified_end` at `warn`, naming the CID, the flow, the
/// side and the context generation.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn an_unclassified_end_aborts_with_peer_reset_and_raises_the_alarm() {
    let (capture, _guard) = EventCapture::install();
    let p = paired(65).await;
    assert!(p.c.io.pairs().contains_key(&p.pair));
    p.c.io.vsock_ends_unclassified(p.flow);
    assert!(!p.held().await);
    assert_eq!(p.far_end(), Some(FarEndEvent::Reset));
    assert_eq!(closed_with(&capture, FlowCloseCause::PeerReset), 1);
    let alarms = capture.named("guest_flow.unclassified_end");
    assert_eq!(alarms.len(), 1);
    assert_eq!(alarms[0].level, tracing::Level::WARN);
    for field in ["cid", "flow", "side", "generation"] {
        assert!(alarms[0].field(field).is_some(), "the alarm names its {field}");
    }
    assert_eq!(p.c.io.forwarder_inventory().unclassified_eof, 1);
}

/// EVMOD-ATTACH-END (host TcpConnect, before `Paired`): the destination
/// resets after its connect completed and before the attach; the attach is
/// refused `HalfEnded`; in that dispatch the owner closes `V_h` and `D`
/// (`D` with `SO_LINGER{1,0}`), sends `Abort { flow }` on the live session,
/// releases the flow, counts `forwarder_attach_ended` and records
/// `PeerReset`; no `Paired` is sent.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn a_destination_reset_before_the_attach_ends_the_flow_with_abort() {
    let (capture, _guard) = EventCapture::install();
    let c = Composition::booted(66).await;
    let (lease, _session) = c.active_lease("attach-end-d", &[]).await;
    let before = c.observe().await.forwarder_attach_ended;
    c.io.hold(SimEffectKind::ConnectPeer);
    let (key, flow) = c.open_tcp_flow(&lease, 1, remote(443), 5000);
    c.settle().await;
    let connect = c.io.held_of(SimEffectKind::ConnectPeer).expect("the connect is held");
    // The connect completes; the destination's reset reaches D first.
    c.io.release(connect);
    c.io.destination_resets(flow);
    let after = c.observe().await;
    assert_eq!(after.forwarder_attach_ended, before + 1);
    assert!(!after.cids[&lease.cid].guest_flows.contains_key(&key));
    assert_eq!(after.cids[&lease.cid].held_flows, 0);
    let sent = sent_to(&c, lease.cid);
    assert!(sent.contains(&ControlMessage::Abort { flow }));
    assert!(!sent.contains(&ControlMessage::Paired { flow }));
    assert!(c.io.socket_observation().closed_by_owner.contains(&key));
    assert_eq!(closed_with(&capture, FlowCloseCause::PeerReset), 1);
}

/// EVMOD-ATTACH-END (U-EV3-1, the guest's close of `V_g` in flight at the
/// host's attach): the guest closes `V_g` with an empty queue before the
/// host attaches (`CLOSING`); the attach is refused `HalfEnded`; the
/// destination is reset by `D`'s linger, never ended with a FIN.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn a_guest_close_before_the_attach_is_refused_and_resets_the_destination() {
    let c = Composition::booted(67).await;
    let (lease, _session) = c.active_lease("attach-end-v", &[]).await;
    let before = c.observe().await.forwarder_attach_ended;
    c.io.hold(SimEffectKind::ConnectPeer);
    let (key, flow) = c.open_tcp_flow(&lease, 1, remote(443), 5000);
    c.settle().await;
    c.io.end_connection(key, SimPeerEnd::Eof);
    let connect = c.io.held_of(SimEffectKind::ConnectPeer).expect("the connect is held");
    c.io.release(connect);
    let after = c.observe().await;
    assert_eq!(after.forwarder_attach_ended, before + 1);
    assert!(!after.cids[&lease.cid].guest_flows.contains_key(&key));
    assert_eq!(
        c.io.socket_observation().far_end.get(&flow),
        Some(&FarEndEvent::Reset),
        "D is closed with SO_LINGER{{1,0}}"
    );
}

/// R5-45(e): a destination that writes and FINs before `Paired` (a
/// server-first destination that ends at once) has its bytes and its end
/// carried out by the attach-time run — classified once, at attach — and
/// is never aborted.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn an_end_before_the_attach_is_carried_out_by_the_attach_time_run() {
    let c = Composition::booted(68).await;
    let (lease, _session) = c.active_lease("attach-time", &[]).await;
    c.io.hold(SimEffectKind::ConnectPeer);
    let (key, flow) = c.open_tcp_flow(&lease, 1, remote(443), 5000);
    c.settle().await;
    let connect = c.io.held_of(SimEffectKind::ConnectPeer).expect("the connect is held");
    c.io.release(connect);
    c.io.destination_sends(flow, b"hello");
    c.io.destination_fins(flow);
    let after = c.observe().await;
    assert!(matches!(
        after.cids[&lease.cid].guest_flows.get(&key),
        Some(GuestFlowPhase::Paired { .. })
    ));
    let vsock = c.io.vsock_of(flow);
    assert_eq!(vsock.peer_received(), b"hello".to_vec());
    assert_eq!(vsock.sent_shutdown(), Some(SimShutdown::Send), "the FIN is carried out as 0x2");
}

/// END-IDEMPOTENT (G-V9 row 4): after a completed flow, the guest's `0x3`
/// and a trailing `RST` change nothing — no second release, no record, and a
/// later flow of the allocation is unaffected.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn a_trailing_reset_after_completion_changes_nothing() {
    let p = paired(69).await;
    let (next_key, _next, _next_pair) = p.c.paired_tcp(&p.lease, 2, remote(8443), 5001).await;
    p.c.io.destination_fins(p.flow);
    p.c.io.guest_half_closes(p.flow);
    assert!(!p.held().await, "completed");
    let completed = p.c.observe().await;
    p.c.io.guest_closes(p.flow);
    p.c.io.guest_resets(p.flow);
    let after = p.c.observe().await;
    assert_eq!(after, completed, "a trailing end releases nothing a second time");
    assert!(matches!(
        after.cids[&p.lease.cid].guest_flows.get(&next_key),
        Some(GuestFlowPhase::Paired { .. })
    ));
    assert_eq!(p.c.io.forwarder_inventory().queued_ends, 0);
}

/// G-V9 / "Forwarder backpressure" rows 2–4 (R5-62 sim part): a destination
/// that stops reading holds the guest's bytes in the flow's sockets — the
/// flow stays `Paired`, nothing aborts, no deadline fires, another flow of
/// the allocation pairs and moves bytes meanwhile; when the reader resumes
/// every byte arrives in order, then the clean end.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn a_stalled_reader_is_backpressure_not_an_abort() {
    let p = paired(70).await;
    p.c.io.destination_stops_reading(p.flow);
    p.c.io.guest_sends(p.flow, &[7_u8; 4096]);
    p.c.io.guest_half_closes(p.flow);
    p.c.advance(std::time::Duration::from_secs(600)).await;
    assert!(p.held().await, "a stalled flow is never aborted");
    let (_other_key, other, _other_pair) = p.c.paired_tcp(&p.lease, 2, remote(8443), 5001).await;
    p.c.io.guest_sends(other, b"unrelated");
    assert_eq!(p.c.io.peer_of(other).peer_received(), b"unrelated".to_vec());
    p.c.io.destination_resumes_reading(p.flow);
    let peer = p.c.io.peer_of(p.flow);
    assert_eq!(peer.peer_received(), vec![7_u8; 4096]);
    assert!(p.c.io.pair_state_of(p.flow).is_some_and(|state| {
        [state.a_to_b, state.b_to_a].iter().any(|direction| direction.target_shut)
    }));
}

/// "Forwarder backpressure" row 5 / R5-10 (sim part): quiescence while a
/// flow is stalled aborts it at once with resets at both far ends.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn quiescence_aborts_a_stalled_flow_with_resets() {
    let (capture, _guard) = EventCapture::install();
    let p = paired(71).await;
    p.c.io.destination_stops_reading(p.flow);
    p.c.io.guest_sends(p.flow, &[1_u8; 1024]);
    let (_hold, _) = p.c.owner.quiesce().await;
    assert!(!p.held().await);
    assert_eq!(p.far_end(), Some(FarEndEvent::Reset));
    assert_eq!(p.to_guest(), Some(SimShutdown::Both));
    assert_eq!(closed_with(&capture, FlowCloseCause::Quiesced), 1);
    assert_eq!(p.c.io.forwarder_inventory().live_pairs, 0);
}

/// R5-10 (sim part) / H-5 / EVMOD-CTL: the host owner's death (the `serve`
/// process exits) releases its context, which aborts every pair: the
/// non-mesh destination — whose peer socket carries the owner egress mark
/// and `SO_LINGER{1,0}` — observes a reset, never EOF, and the guest's
/// vsock is shut both ways.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn the_owners_death_resets_its_non_mesh_far_end() {
    let p = paired(72).await;
    p.c.io.guest_sends(p.flow, b"mid-transfer");
    p.c.io.destination_sends(p.flow, b"mid-transfer");
    let spec =
        p.c.io
            .issued()
            .into_iter()
            .find_map(|effect| match effect.target {
                SimEffectTarget::Peer(spec) if effect.kind == SimEffectKind::OpenPeer => Some(spec),
                _ => None,
            })
            .expect("the owner opened the non-mesh peer");
    assert!(spec.abortive_linger && spec.owner_egress_mark);
    p.c.io.exit_process();
    assert_eq!(p.far_end(), Some(FarEndEvent::Reset));
    assert_eq!(p.to_guest(), Some(SimShutdown::Both));
    let contexts = p.c.io.forwarder_contexts();
    assert!(contexts.iter().all(|context| !context.open && context.live_pairs == 0));
}

/// RESET-EXIT / R5-10 (sim part): the VM ends (a guest PID-1 crash): the
/// device release's sweep sets `sk_err` on every host vsock of the CID and
/// the forwarder aborts the pair (`SocketError { ECONNRESET }`); the owner
/// releases the flow `PeerReset` and the destination is reset.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn the_vms_end_aborts_its_flows_through_the_device_release() {
    let (capture, _guard) = EventCapture::install();
    let p = paired(73).await;
    p.c.io.vm_stop(p.lease.cid);
    assert!(!p.held().await);
    assert_eq!(p.far_end(), Some(FarEndEvent::Reset));
    assert_eq!(p.c.io.vsock_of(p.flow).sk_err(), Some(KernelErrno::ECONNRESET));
    assert_eq!(closed_with(&capture, FlowCloseCause::PeerReset), 1);
}

/// T1–T6 (EVMOD-PAIR; R5-49 owner rules): once attached, the owner never
/// reads, writes or shuts the pair's sockets before the end record: across a
/// flow's whole life the only owner effects on its halves after `Paired` are
/// the closes (and the linger clear) that follow its record.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn the_owner_touches_an_attached_pair_only_after_its_record() {
    let p = paired(74).await;
    let at_pairing = p.c.io.issued().len();
    p.c.io.guest_sends(p.flow, b"x");
    p.c.io.destination_sends(p.flow, b"y");
    p.c.io.guest_half_closes(p.flow);
    let while_live: Vec<_> = p.c.io.issued().into_iter().skip(at_pairing).collect();
    assert!(
        while_live.iter().all(|effect| !matches!(
            effect.kind,
            SimEffectKind::ClearLinger | SimEffectKind::Close | SimEffectKind::Abort
        )),
        "no owner effect on a live pair: {while_live:?}"
    );
    p.c.io.destination_fins(p.flow);
    assert!(!p.held().await, "the record ends the flow, then the owner closes it");
}
