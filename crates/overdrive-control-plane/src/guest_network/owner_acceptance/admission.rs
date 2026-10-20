//! Host halves of the control session, guest-opened flow admission, pairing
//! and launch attribution (R5-9, R5-16, R5-17, R5-18, R5-20, R5-26, R5-41
//! sim part; G-V3 rows 1, 2 and 4; G-V6 rows 1–5).
//!
//! Design: `feature-delta.md` § *Control session*, § *Per-kind total orders*
//! (TcpConnect steps 3–6, Datagram association steps 3–4), § *Wire
//! contracts* → Attribution (ATTR-TOKEN, ATTR-LIVE-CLAIM), § *Owner and
//! provisioner* (M-7, U-1, U-5, T1–T6), § *[REF] vsock data path* →
//! EVMOD-PAIR (attach at pairing, never before the request was read in
//! full), Gates G-V3 and G-V6. Quint oracle: `owner_flows` (`flows_conf`),
//! replayed by `quint_owner_flows`.
//!
//! Every body drives the production owner through its driving ports
//! (`provision`, `activate`, `teardown`, `quiesce`, `restore`) and the
//! guest's stimuli at the seam sim, and asserts only port-observable state:
//! QPROJ-3 `flow_observation()`, the sim arbiters (`socket_observation()`,
//! the forwarder's inventory), the control messages the owner wrote on the
//! session, and the effects it issued. Layer 3 (in-process owner over a seam
//! sim): example-based, every sad path enumerated (Mandates 9, 11).

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use overdrive_core::guest_transport::wire::{
    ControlMessage, FLOW_PAIRING_DEADLINE, FLOW_REQUEST_DEADLINE, FlowRefusal, FlowRequest,
};

use super::support::{Composition, foreign_token, guest_flow, remote, sent_to};
use crate::guest_network::flow_io::sim::{SimEffectKind, SimEffectTarget, SimFailure, SimPeerEnd};
use crate::guest_network::{
    CidPhase, FlowCloseCause, ForwardingActivation, ForwardingRestore, GuestAdmission,
    GuestFlowPhase, GuestNetworkError, GuestNetworkOperation, GuestNetworkProvisioner, RequestRead,
    SharedGuestNetworkOwner,
};

fn attaches(c: &Composition) -> usize {
    c.io.issued_count(SimEffectKind::Attach)
}

/// R5-9 / H-1 (G-V6 row 1, EVMOD-PAIR): the owner attaches a flow's pair
/// only after the whole 32-byte request was read in userspace and its token
/// matched, and only after the destination connected; a request read in
/// pieces attaches nothing before the last piece, and nothing of the request
/// reaches the destination as payload.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn the_request_is_read_in_full_before_the_pair_is_attached() {
    let c = Composition::booted(3).await;
    let (lease, _session) = c.active_lease("r5-9", &[]).await;
    c.io.set_auto_deliver(false);
    let (key, flow) = c.open_tcp_flow(&lease, 1, remote(443), 5000);

    c.io.release_accept(key);
    let accepted = c.observe().await;
    assert_eq!(
        accepted.cids[&lease.cid].guest_flows.get(&key),
        Some(&GuestFlowPhase::Registered { request: RequestRead::Nothing }),
        "admission registers the flow at accept (U-1)"
    );
    assert_eq!(attaches(&c), 0);

    c.io.release_read(key, Some(16));
    let partial = c.observe().await;
    assert_eq!(
        partial.cids[&lease.cid].guest_flows.get(&key),
        Some(&GuestFlowPhase::Registered { request: RequestRead::Partial })
    );
    assert_eq!(attaches(&c), 0, "nothing is attached before the request is read in full (H-1)");
    assert!(c.io.peer_connects().is_empty(), "nothing is connected before the token matched");

    c.io.release_read(key, None);
    let paired = c.observe().await;
    assert_eq!(attaches(&c), 1, "the full request attaches the pair once");
    assert!(matches!(
        paired.cids[&lease.cid].guest_flows.get(&key),
        Some(GuestFlowPhase::Paired { generation, .. }) if *generation == paired.forwarder_generation
    ));
    assert_eq!(sent_to(&c, lease.cid), vec![ControlMessage::Paired { flow }]);
    assert!(
        c.io.peer_of(flow).peer_received().is_empty(),
        "no byte of the request reaches the destination as payload (K-A3)"
    );
}

/// G-V6 row 1 (P-23, EVMOD-PAIR): the connect completes, then the pair is
/// attached, then `Paired` travels on the control session — never on the
/// flow vsock — and a server-first destination's banner sent before
/// `Paired` waits in the destination socket's queue and reaches the guest
/// in order once the pair runs.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn paired_follows_the_attach_and_a_server_first_banner_is_delivered() {
    let c = Composition::booted(4).await;
    let (lease, session) = c.active_lease("g-v6-paired", &[]).await;
    c.io.hold(SimEffectKind::ConnectPeer);
    let (key, flow) = c.open_tcp_flow(&lease, 1, remote(443), 5000);
    c.settle().await;
    assert_eq!(attaches(&c), 0, "no attach before the connect completes");
    assert!(sent_to(&c, lease.cid).is_empty(), "no Paired before the attach");

    let connect = c.io.held_of(SimEffectKind::ConnectPeer).expect("the connect is held");
    c.io.release(connect);
    c.settle().await;
    let kinds: Vec<_> = c.io.issued().into_iter().map(|effect| effect.kind).collect();
    let attach = kinds.iter().position(|kind| *kind == SimEffectKind::Attach).expect("attached");
    let paired_write =
        kinds.iter().rposition(|kind| *kind == SimEffectKind::WriteRecord).expect("Paired written");
    assert!(attach < paired_write, "the pair is attached before Paired is sent");
    assert_eq!(sent_to(&c, lease.cid), vec![ControlMessage::Paired { flow }]);
    let writes: Vec<_> =
        c.io.issued()
            .into_iter()
            .filter(|effect| effect.kind == SimEffectKind::WriteRecord)
            .map(|effect| effect.target)
            .collect();
    assert!(
        writes.iter().all(|target| *target == SimEffectTarget::Conn(Some(session))),
        "every record the owner writes goes to the control session, never to the flow vsock {key:?}"
    );

    c.io.destination_sends(flow, b"220 banner");
    assert_eq!(
        c.io.vsock_of(flow).peer_received(),
        b"220 banner".to_vec(),
        "the banner reaches the guest"
    );
}

/// R5-26 / U-1: the first quiescence hold aborts a flow whose non-blocking
/// connect is still in flight; the completion that lands after the abort
/// attaches nothing and sends no `Paired`; admission refuses `Quiesced`
/// until the hold is restored.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn quiescence_aborts_a_connecting_flow_and_its_late_completion_attaches_nothing() {
    let c = Composition::booted(5).await;
    let (lease, _session) = c.active_lease("r5-26q", &[]).await;
    c.io.hold(SimEffectKind::ConnectPeer);
    let (key, flow) = c.open_tcp_flow(&lease, 1, remote(443), 5000);
    let connecting = c.observe().await;
    assert_eq!(
        connecting.cids[&lease.cid].guest_flows.get(&key),
        Some(&GuestFlowPhase::Connecting)
    );

    let (hold, quiescence) = c.owner.quiesce().await;
    assert!(quiescence.unconfirmed.is_empty());
    let quiesced = c.observe().await;
    assert_eq!(quiesced.admission, GuestAdmission::Quiesced);
    assert_eq!(quiesced.quiescence_holds, 1);
    assert!(quiesced.cids[&lease.cid].guest_flows.is_empty(), "the flow is aborted");
    assert_eq!(quiesced.cids[&lease.cid].held_flows, 0);
    assert_eq!(quiesced.cids[&lease.cid].phase, CidPhase::Active, "quiescence keeps per-CID state");
    assert!(c.io.socket_observation().closed_by_owner.contains(&key));

    if let Some(late) = c.io.held_of(SimEffectKind::ConnectPeer) {
        c.io.release(late);
    }
    c.settle().await;
    assert_eq!(attaches(&c), 0, "a completion after the abort attaches nothing (U-1)");
    assert!(!sent_to(&c, lease.cid).contains(&ControlMessage::Paired { flow }));

    let (refused_key, refused_flow) = c.open_tcp_flow(&lease, 2, remote(443), 5001);
    c.settle().await;
    assert!(
        sent_to(&c, lease.cid).contains(&ControlMessage::Refused {
            flow: refused_flow,
            refusal: FlowRefusal::Quiesced
        })
    );
    assert!(c.io.socket_observation().closed_by_owner.contains(&refused_key));

    assert_eq!(c.owner.restore(hold).await, Ok(ForwardingRestore::Reopened));
    c.io.auto_release(SimEffectKind::ConnectPeer);
    let (_reopened_key, reopened_flow) = c.open_tcp_flow(&lease, 3, remote(443), 5002);
    c.settle().await;
    assert!(sent_to(&c, lease.cid).contains(&ControlMessage::Paired { flow: reopened_flow }));
}

/// R5-26: teardown aborts a connecting flow; nothing of the CID remains and
/// the completion attaches nothing.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn teardown_aborts_a_connecting_flow_and_leaves_nothing_of_the_cid() {
    let c = Composition::booted(6).await;
    let (lease, _session) = c.active_lease("r5-26t", &[]).await;
    c.io.hold(SimEffectKind::ConnectPeer);
    let (key, flow) = c.open_tcp_flow(&lease, 1, remote(443), 5000);
    c.settle().await;

    c.owner.teardown(&lease.plan).await.expect("teardown converges on absence");
    let after = c.observe().await;
    assert!(!after.cids.contains_key(&lease.cid), "a torn-down CID is absent from the owner");
    assert!(c.io.socket_observation().closed_by_owner.contains(&key));
    if let Some(late) = c.io.held_of(SimEffectKind::ConnectPeer) {
        c.io.release(late);
    }
    c.settle().await;
    assert_eq!(attaches(&c), 0);
    assert!(!sent_to(&c, lease.cid).contains(&ControlMessage::Paired { flow }));
}

/// R5-26 / DG-5-F: the loss of the control session aborts every flow of the
/// CID, a connecting one included; its completion attaches nothing.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn session_loss_aborts_a_connecting_flow() {
    let c = Composition::booted(7).await;
    let (lease, session) = c.active_lease("r5-26l", &[]).await;
    c.io.hold(SimEffectKind::ConnectPeer);
    let (key, flow) = c.open_tcp_flow(&lease, 1, remote(443), 5000);
    c.settle().await;

    c.io.end_connection(session, SimPeerEnd::Eof);
    let lost = c.observe().await;
    let cid = &lost.cids[&lease.cid];
    assert_eq!(cid.session, None);
    assert!(cid.guest_flows.is_empty());
    assert_eq!(cid.phase, CidPhase::Active, "session loss keeps the CID Active");
    assert!(c.io.socket_observation().closed_by_owner.contains(&key));
    if let Some(late) = c.io.held_of(SimEffectKind::ConnectPeer) {
        c.io.release(late);
    }
    c.settle().await;
    assert_eq!(attaches(&c), 0);
    assert!(!sent_to(&c, lease.cid).contains(&ControlMessage::Paired { flow }));
}

/// R5-26: a quiescence that lands while a flow is admitted but its request
/// is still unread finds the registration (check and register are one step,
/// U-1) and aborts it; the request that arrives afterwards is never read.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn quiescence_finds_an_admitted_flow_whose_request_is_unread() {
    let c = Composition::booted(17).await;
    let (lease, _session) = c.active_lease("r5-26r", &[]).await;
    c.io.set_auto_deliver(false);
    let (key, _flow) = c.open_tcp_flow(&lease, 1, remote(443), 5000);
    c.io.release_accept(key);
    c.settle().await;
    let (_hold, _) = c.owner.quiesce().await;
    let quiesced = c.observe().await;
    assert!(quiesced.cids[&lease.cid].guest_flows.is_empty());
    assert!(c.io.socket_observation().closed_by_owner.contains(&key));
    assert!(c.io.peer_connects().is_empty() && attaches(&c) == 0);
}

/// R5-16 / M-7: a flow from a CID with no live control session is closed
/// unattributed (`NoControlSession`): never registered, no message, no
/// attach, counted by `guest_flow.connection_closed_unattributed`.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn a_flow_from_a_cid_without_a_session_is_closed_unattributed() {
    let (capture, _guard) = super::support::EventCapture::install();
    let c = Composition::booted(8).await;
    let lease = c.lease("r5-16n", &[]).await;
    let before = c.observe().await;
    let (key, _flow) = c.open_tcp_flow(&lease, 1, remote(443), 5000);
    let after = c.observe().await;
    assert_eq!(after, before, "the owner's tables are unchanged");
    assert!(c.io.socket_observation().closed_by_owner.contains(&key));
    assert!(sent_to(&c, lease.cid).is_empty(), "no session: no message");
    assert_eq!(attaches(&c), 0);
    let events = capture.named("guest_flow.connection_closed_unattributed");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].field("cause"), Some(FlowCloseCause::NoControlSession.as_str()));
}

/// R5-16 / ATTR-TOKEN: a flow whose request carries another launch's token
/// is closed at its request read, unregistered, with no control message
/// (`StaleLaunch`), and nothing is resolved, connected or attached.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn a_flow_carrying_another_launchs_token_is_closed_with_no_message() {
    let (capture, _guard) = super::support::EventCapture::install();
    let c = Composition::booted(9).await;
    let (lease, _session) = c.active_lease("r5-16t", &[]).await;
    let before = c.observe().await;
    let (key, _flow) = c.open_tcp_flow_with(&lease, 1, remote(443), 5000, foreign_token());
    let after = c.observe().await;
    assert_eq!(after, before);
    assert!(c.io.socket_observation().closed_by_owner.contains(&key));
    assert!(sent_to(&c, lease.cid).is_empty());
    assert_eq!(c.io.issued_count(SimEffectKind::OpenPeer), 0, "no peer for a stale request");
    assert_eq!(attaches(&c), 0);
    let events = capture.named("guest_flow.connection_closed_unattributed");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].field("cause"), Some(FlowCloseCause::StaleLaunch.as_str()));
}

/// R5-17 / L-4: a datagram association whose `DatagramStream` request fails
/// to decode (destination port 0) is refused `Malformed`, its `V_h` closed;
/// no association is registered and no host UDP socket is made.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn a_malformed_datagram_request_is_refused_malformed() {
    let c = Composition::booted(10).await;
    let (lease, _session) = c.active_lease("r5-17", &[]).await;
    let flow = guest_flow(1);
    let mut record =
        FlowRequest::DatagramStream { flow, destination: remote(53), launch: lease.launch }
            .encode()
            .to_vec();
    // Offset 12, the destination port: never 0 for kinds 1 and 2.
    record[12] = 0;
    record[13] = 0;
    let key = c.open_raw_flow(&lease, record, 6000);
    let after = c.observe().await;
    assert!(after.cids[&lease.cid].datagrams.is_empty());
    assert_eq!(after.cids[&lease.cid].held_flows, 0);
    assert_eq!(
        sent_to(&c, lease.cid),
        vec![ControlMessage::Refused { flow, refusal: FlowRefusal::Malformed }]
    );
    assert!(c.io.socket_observation().closed_by_owner.contains(&key));
    assert_eq!(c.io.issued_count(SimEffectKind::OpenPeer), 0);
}

/// R5-18 / L-3: two control sessions of the current launch race; exactly
/// one takes the claim, the second is closed (`SessionHeld`); a session
/// carrying another token is closed and takes no claim.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn racing_sessions_leave_one_claim_and_a_stale_session_takes_none() {
    let c = Composition::booted(11).await;
    let lease = c.lease("r5-18", &[]).await;
    let first = c.open_session(&lease, 1024);
    let second = c.open_session(&lease, 1025);
    let stale = c.open_session_with(&lease, 1026, foreign_token());
    let after = c.observe().await;
    let cid = &after.cids[&lease.cid];
    let holder = cid.session.expect("one session holds the claim");
    assert!(holder == first || holder == second);
    let closed = c.io.socket_observation().closed_by_owner;
    let loser = if holder == first { second } else { first };
    assert!(closed.contains(&loser), "the second session is closed SessionHeld");
    assert!(closed.contains(&stale), "the stale session is closed StaleLaunch");
    assert!(!closed.contains(&holder));
    assert!(cid.unread.is_empty());
}

/// R5-18 (ATTR-TOKEN): a session the CID's previous launch left queued,
/// delivered before the current launch's own, never takes the claim; the
/// current launch's session is accepted.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn a_previous_launchs_queued_session_never_takes_the_claim() {
    let c = Composition::booted(18).await;
    let old = c.lease("r5-18-old", &[]).await;
    c.io.set_auto_deliver(false);
    let old_session = c.open_session(&old, 1024);
    c.io.vm_stop(old.cid);
    assert!(c.pool.retire(&old.alloc));
    c.owner.teardown(&old.plan).await.expect("teardown of the dead launch");
    c.pool.release(&old.alloc);

    let new = c.lease("r5-18-new", &[]).await;
    let new_session = c.open_session(&new, 2048);
    c.io.release_accept(old_session);
    c.settle().await;
    c.io.release_read(old_session, None);
    c.io.release_accept(new_session);
    c.settle().await;
    c.io.release_read(new_session, None);
    let after = c.observe().await;
    assert_eq!(after.cids[&new.cid].session, Some(new_session));
    assert!(c.io.socket_observation().closed_by_owner.contains(&old_session));
}

/// R5-41 (sim part, P-37, ATTR-LIVE-CLAIM): connections a dead launch left
/// queued — a control session and a flow of each kind, each with its first
/// record buffered — are accepted after the launch's teardown and closed at
/// their read: no claim, no message, no registration, no connect and no
/// attach; a queued connection with no buffered bytes fails its first read
/// and is closed; the next lease's own session takes its claim and
/// activation proceeds on it.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn a_dead_launchs_queued_connections_are_closed_at_their_read() {
    let c = Composition::booted(12).await;
    let old = c.lease("r5-41-old", &[]).await;
    c.io.set_auto_deliver(false);
    let old_session = c.open_session(&old, 1024);
    let (old_flow, _) = c.open_tcp_flow(&old, 1, remote(443), 5000);
    let (old_datagram, _) = c.open_datagram(&old, 2, remote(53), 5001);
    let empty = c.open_raw_flow(&old, Vec::new(), 5002);
    c.io.vm_stop(old.cid);
    assert!(c.pool.retire(&old.alloc));
    c.owner.teardown(&old.plan).await.expect("teardown of the dead launch");
    c.pool.release(&old.alloc);

    let new = c.lease("r5-41-new", &[]).await;
    for key in [old_session, old_flow, old_datagram, empty] {
        c.io.release_accept(key);
        c.settle().await;
        c.io.release_read(key, None);
    }
    let new_session = c.open_session(&new, 2048);
    c.io.release_accept(new_session);
    c.settle().await;
    c.io.release_read(new_session, None);
    let after = c.observe().await;

    let closed = c.io.socket_observation().closed_by_owner;
    for key in [old_session, old_flow, old_datagram, empty] {
        assert!(closed.contains(&key), "{key:?} of the dead launch is closed at its read");
    }
    assert!(!after.cids.contains_key(&old.cid) || old.cid == new.cid);
    assert_eq!(after.cids[&new.cid].session, Some(new_session));
    assert!(
        after.cids[&new.cid].guest_flows.is_empty() && after.cids[&new.cid].datagrams.is_empty()
    );
    assert!(sent_to(&c, new.cid).is_empty(), "no control message crosses launches");
    assert!(c.io.peer_connects().is_empty() && attaches(&c) == 0);
    assert_eq!(
        c.owner.activate(&new.plan).await.expect("activation"),
        ForwardingActivation::Activated
    );
}

/// R5-20 / D15-R3: a TcpConnect to an unresponsive non-mesh destination has
/// no owner deadline — past `FLOW_REQUEST_DEADLINE` and
/// `FLOW_PAIRING_DEADLINE` it is still connecting and holds its quota slot;
/// another flow of the VM pairs meanwhile; when the kernel gives up the flow
/// is refused `DestinationUnreachable` and its slot is released.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn a_non_mesh_connect_has_no_owner_deadline() {
    let c = Composition::booted(13).await;
    let (lease, _session) = c.active_lease("r5-20", &[]).await;
    c.io.hold(SimEffectKind::ConnectPeer);
    let (slow_key, slow) = c.open_tcp_flow(&lease, 1, remote(81), 5000);
    c.advance(FLOW_REQUEST_DEADLINE + FLOW_PAIRING_DEADLINE + Duration::from_secs(60)).await;
    let pending = c.observe().await;
    assert_eq!(
        pending.cids[&lease.cid].guest_flows.get(&slow_key),
        Some(&GuestFlowPhase::Connecting)
    );
    assert_eq!(pending.cids[&lease.cid].held_flows, 1);
    assert!(sent_to(&c, lease.cid).is_empty(), "no PairingTimeout, no refusal");

    let first_connect = c.io.held_of(SimEffectKind::ConnectPeer).expect("the slow connect");
    c.io.auto_release(SimEffectKind::ConnectPeer);
    let (_fast_key, fast) = c.open_tcp_flow(&lease, 2, remote(443), 5001);
    c.settle().await;
    assert!(sent_to(&c, lease.cid).contains(&ControlMessage::Paired { flow: fast }));

    c.io.fail(first_connect, SimFailure::Io(std::io::ErrorKind::TimedOut));
    let refused = c.observe().await;
    assert!(sent_to(&c, lease.cid).contains(&ControlMessage::Refused {
        flow: slow,
        refusal: FlowRefusal::DestinationUnreachable
    }));
    assert!(!refused.cids[&lease.cid].guest_flows.contains_key(&slow_key));
    assert_eq!(refused.cids[&lease.cid].held_flows, 1, "only the paired flow is held");
}

/// G-V3 row 4: `activate` after `teardown` is refused
/// `PostconditionMismatch { ForwardingActivate }` and changes nothing.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn activate_after_teardown_is_refused_and_changes_nothing() {
    let c = Composition::booted(14).await;
    let (lease, _session) = c.active_lease("g-v3-r4", &[]).await;
    c.owner.teardown(&lease.plan).await.expect("teardown");
    let before = c.observe().await;
    let refused = c.owner.activate(&lease.plan).await;
    assert!(matches!(
        refused,
        Err(GuestNetworkError::PostconditionMismatch {
            operation: GuestNetworkOperation::ForwardingActivate,
            ..
        })
    ));
    assert_eq!(c.observe().await, before);
}

/// G-V3 rows 1–2: activation with no live session returns
/// `ControlSessionPending` and changes nothing; once the session is live it
/// activates; while quiesced it returns `QuiescenceLatched`.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn activation_waits_for_the_session_and_for_quiescence_to_end() {
    let c = Composition::booted(15).await;
    let lease = c.lease("g-v3-r12", &[]).await;
    let before = c.observe().await;
    assert_eq!(
        c.owner.activate(&lease.plan).await.expect("no error"),
        ForwardingActivation::ControlSessionPending
    );
    assert_eq!(c.observe().await, before);

    let _session = c.open_session(&lease, 1024);
    c.settle().await;
    let (hold, _) = c.owner.quiesce().await;
    assert_eq!(
        c.owner.activate(&lease.plan).await.expect("no error"),
        ForwardingActivation::QuiescenceLatched
    );
    assert_eq!(c.observe().await.cids[&lease.cid].phase, CidPhase::Provisioned);
    assert_eq!(c.owner.restore(hold).await, Ok(ForwardingRestore::Reopened));
    assert_eq!(
        c.owner.activate(&lease.plan).await.expect("no error"),
        ForwardingActivation::Activated
    );
    assert_eq!(c.observe().await.cids[&lease.cid].phase, CidPhase::Active);
}

/// G-V6 rows 2–3 (the taxonomy row "nobody in the allocation lifecycle
/// branches"): a per-flow refusal is flow-local — the CID stays Active, its
/// session and every other flow unchanged.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
async fn a_per_flow_refusal_leaves_the_cid_and_its_other_flows_unchanged() {
    let c = Composition::booted(16).await;
    let (lease, _session) = c.active_lease("g-v6-r3", &[]).await;
    let (paired_key, _paired, _pair) = c.paired_tcp(&lease, 1, remote(443), 5000).await;
    let before = c.observe().await;
    let multicast = SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 9), 5000);
    let (_refused_key, refused) = c.open_tcp_flow(&lease, 2, multicast, 5001);
    let after = c.observe().await;
    assert!(
        sent_to(&c, lease.cid)
            .contains(&ControlMessage::Refused { flow: refused, refusal: FlowRefusal::Policy })
    );
    assert_eq!(after, before, "the refusal changes nothing else at the owner");
    assert!(matches!(
        after.cids[&lease.cid].guest_flows.get(&paired_key),
        Some(GuestFlowPhase::Paired { .. })
    ));
}
