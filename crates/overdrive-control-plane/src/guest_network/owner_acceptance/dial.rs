//! INB-1 — the host guest-flow owner as the production `GuestStreamDial`
//! (ADR-0173, ADR-0174, ADR-0187 PROBE-NOSAMPLE) and the stream it returns
//! (EVMOD-STREAM-END, EVMOD-STREAM-ABORT, WATCH-ABORT, EVMOD-AB): R5-53 (sim
//! part), R5-63 (a)–(f), R5-28 and R5-31 (host halves), R5-57's sim half.
//!
//! Design: `feature-delta.md` § *Driving port — guest stream dial
//! `GuestStreamDial`* (order of decision, opening — the watch of `V_h`
//! before its request is written —, outcome, `Abort` when the owner ends the
//! dial before `Paired`, in-flight aborts, late replies, cancellation, the
//! stream's end resolution, owner abort and drop, observable invariants);
//! § *[REF] vsock data path* → "Owners' use of the forwarder" (`GuestStream`)
//! and "End classification" → "Watch". Quint oracles: `owner_flows`
//! `Dial*`, `PairingDeadline*`, `VmExitDuringDial*`,
//! `DroppedDialFutureEndsLikeThePairingDeadline`,
//! `TeardownEndsPendingDialWithNotServing` (replayed by `quint_owner_flows`)
//! and `fwd_pair`'s watch runs (replayed by `quint_fwd_pair`).
//!
//! One cell per `GuestDialError` variant and per in-flight end, each with
//! its no-state-change assertion for a typed refusal (steps 1–4 change
//! nothing: no slot, no socket, no watch, no quota slot, no message). Every
//! stream end is a cause on `V_h`'s far side (the guest's `0x2`, `0x3` or
//! `RST`, the device release) or an owner abort; the stream's outcome is
//! read from the production `GuestStream`. Layer 3, example-based
//! (Mandate 11). Every cell is ignored pending the DELIVER step that lands
//! the dial (step 9, host platform clients dial through the owner).

#![allow(
    clippy::unchecked_time_subtraction,
    reason = "DISTILL test support and conformance drivers: handler signatures follow the action map and fixtures assert their preconditions"
)]

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use overdrive_core::guest_transport::dial::{
    DialPurpose, GuestDialError, GuestStream, GuestStreamDial, GuestStreamEnd,
};
use overdrive_core::guest_transport::wire::{
    ControlMessage, FlowId, FlowRefusal, FlowRequest, GUEST_DIAL_PAIRING_DEADLINE, KernelErrno,
};
use overdrive_core::guest_transport::{GUEST_NODE_GATEWAY, GUEST_NODE_PREFIX};
use overdrive_core::net::ListenPort;
use overdrive_sim::adapters::SimShutdown;

use super::support::{Composition, EventCapture, Lease, sent_to};
use crate::guest_network::flow_io::sim::{SimGuestStreamEnd, SimPeerEnd, VsockShutdownOp};
use crate::guest_network::{
    DialObservation, DialPhase, ForwardingRestore, GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION,
    GuestConnKey, GuestNetworkProvisioner, SharedGuestNetworkOwner,
};

const PORT: u16 = 8080;

/// A booted owner with one Active lease declaring `PORT`, its control
/// session live.
async fn serving(seed: u64) -> (Composition, Lease, GuestConnKey) {
    let c = Composition::booted(seed).await;
    let (lease, session) = c.active_lease("inb1", &[PORT]).await;
    (c, lease, session)
}

/// A paired `Relay` dial of `lease`'s `PORT`: the stream and its flow id.
async fn paired_stream(
    c: &Composition,
    lease: &Lease,
    session: GuestConnKey,
) -> (GuestStream, FlowId) {
    c.paired_dial(session, lease.target(PORT), DialPurpose::Relay, 0).await
}

fn dial_of(
    c_observed: &crate::guest_network::GuestOwnerObservation,
    lease: &Lease,
    flow: FlowId,
) -> Option<DialObservation> {
    c_observed.cids[&lease.cid].dials.get(&flow).copied()
}

/// Opening (INB-1): the dial connects `V_h`, watches it before writing the
/// request (EVMOD-STREAM-END), and writes a TcpAccept request from the guest
/// gateway (port 0 on the wire) to the declared port with the live launch
/// token; a `Relay` dial is registered `AwaitingReply` and holds a quota
/// place.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_dial_watches_vh_then_writes_a_tcp_accept_request_with_the_live_token() {
    let (c, lease, _session) = serving(40).await;
    let pending = c.spawn_dial(lease.target(PORT), DialPurpose::Relay);
    c.settle().await;
    let dial = c.io.dials().first().copied().expect("one dial opened V_h");
    assert_eq!(dial.cid, lease.cid);
    assert!(dial.watch.is_some(), "V_h is watched");
    let kinds: Vec<_> = c.io.issued().into_iter().map(|effect| effect.kind).collect();
    let watch = kinds
        .iter()
        .position(|kind| *kind == crate::guest_network::flow_io::sim::SimEffectKind::Watch)
        .expect("watched");
    let write = kinds
        .iter()
        .rposition(|kind| *kind == crate::guest_network::flow_io::sim::SimEffectKind::WriteRecord)
        .expect("request written");
    assert!(watch < write, "the watch is installed before the request is written");
    let Some(FlowRequest::TcpAccept { flow, peer, listen_port, launch }) = dial.request else {
        panic!("the dial wrote a TcpAccept request, got {:?}", dial.request);
    };
    assert_eq!(peer, GUEST_NODE_GATEWAY);
    assert_eq!(listen_port, ListenPort::new(PORT).expect("non-zero"));
    assert_eq!(launch, lease.launch);
    let observed = c.observe().await;
    assert_eq!(
        dial_of(&observed, &lease, flow),
        Some(DialObservation { purpose: DialPurpose::Relay, phase: DialPhase::AwaitingReply })
    );
    assert_eq!(observed.cids[&lease.cid].held_flows, 1, "a Relay dial holds a quota place");
    assert_eq!(c.io.forwarder_inventory().live_watches, 1);
    pending.abort();
}

/// R5-53 (`Ok`): `Paired` → `Ok(GuestStream)` for the dialed address; bytes
/// cross both ways; the stream holds its slot until its drop, which aborts
/// the watch, then closes `V_h` (`0x3` toward the guest) and releases the
/// slot and the quota place (EVMOD-AB, EVMOD-STREAM-ABORT, R5-63 (e)).
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn paired_returns_the_stream_and_its_drop_aborts_the_watch_then_closes_vh() {
    let (c, lease, session) = serving(41).await;
    let (mut stream, flow) = paired_stream(&c, &lease, session).await;
    assert_eq!(stream.target(), lease.target(PORT));
    let observed = c.observe().await;
    let Some(DialObservation {
        purpose: DialPurpose::Relay,
        phase: DialPhase::Paired { watch, generation, guest_end: None, host_shut: false },
    }) = dial_of(&observed, &lease, flow)
    else {
        panic!("the dial is Paired");
    };
    assert_eq!(generation, observed.forwarder_generation);
    assert_eq!(Some(watch), c.io.dials()[0].watch);
    assert_eq!(observed.cids[&lease.cid].held_flows, 1);

    assert_eq!(stream.write(b"ping").await.expect("the host direction is open"), 4);
    assert_eq!(c.io.stream_observation(flow).to_guest, b"ping".to_vec());
    c.io.stream_guest_write(flow, b"pong");
    let mut buf = [0_u8; 8];
    let read = stream.read(&mut buf).await.expect("guest bytes");
    assert_eq!(&buf[..read], b"pong");

    drop(stream);
    let dropped = c.observe().await;
    assert!(dropped.cids[&lease.cid].dials.is_empty());
    assert_eq!(dropped.cids[&lease.cid].held_flows, 0);
    assert_eq!(c.io.forwarder_inventory().live_watches, 0, "no watch outlives the drop");
    assert_eq!(c.io.sent_to_guest(flow).last(), Some(&VsockShutdownOp::Close));
}

/// R5-53 (`Refused`): the guest's `Refused { ApplicationRefused { errno } }`
/// → `Refused { errno }`; `V_h` closed; slot, watch and quota place
/// released.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_guests_refusal_is_returned_with_its_errno() {
    let (c, lease, session) = serving(42).await;
    let pending = c.spawn_dial(lease.target(PORT), DialPurpose::Relay);
    c.settle().await;
    let flow = c.dial_flow(0);
    c.guest_sends(
        session,
        ControlMessage::Refused {
            flow,
            refusal: FlowRefusal::ApplicationRefused { errno: KernelErrno::ECONNREFUSED },
        },
    );
    c.settle().await;
    assert_eq!(
        pending.await.expect("no panic").map(|_| ()),
        Err(GuestDialError::Refused { errno: KernelErrno::ECONNREFUSED })
    );
    let observed = c.observe().await;
    assert!(observed.cids[&lease.cid].dials.is_empty());
    assert_eq!(observed.cids[&lease.cid].held_flows, 0);
    assert!(c.io.dials()[0].closed, "every error closes V_h");
    assert_eq!(c.io.forwarder_inventory().live_watches, 0, "and aborts its watch");
}

/// R5-53 (`TimedOut`): the guest's `TimedOut { errno }` → `TimedOut {
/// errno }`.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_guests_connect_timeout_is_returned_with_its_errno() {
    let (c, lease, session) = serving(43).await;
    let pending = c.spawn_dial(lease.target(PORT), DialPurpose::Relay);
    c.settle().await;
    let flow = c.dial_flow(0);
    c.guest_sends(session, ControlMessage::TimedOut { flow, errno: KernelErrno::ETIMEDOUT });
    c.settle().await;
    assert_eq!(
        pending.await.expect("no panic").map(|_| ()),
        Err(GuestDialError::TimedOut { errno: KernelErrno::ETIMEDOUT })
    );
    assert!(c.observe().await.cids[&lease.cid].dials.is_empty());
}

/// R5-53 (`PairingDeadline`) / U-5: no reply by `GUEST_DIAL_PAIRING_DEADLINE`
/// from a live VM → `PairingDeadline`, not a moment earlier; the owner
/// closes `V_h` and sends `Abort { flow }` on the live session; a late
/// `Paired` is discarded with no reply and never yields a stream
/// (`NoPairedAfterDialEnded`, `guest_flow.control_discarded`).
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_pairing_deadline_ends_a_dial_and_a_late_paired_is_discarded() {
    let (capture, _guard) = EventCapture::install();
    let (c, lease, session) = serving(44).await;
    let pending = c.spawn_dial(lease.target(PORT), DialPurpose::Relay);
    c.settle().await;
    let flow = c.dial_flow(0);
    c.advance(GUEST_DIAL_PAIRING_DEADLINE - Duration::from_millis(1)).await;
    assert!(!pending.is_finished(), "the owner waits the whole deadline");
    c.advance(Duration::from_millis(1)).await;
    assert_eq!(pending.await.expect("no panic").map(|_| ()), Err(GuestDialError::PairingDeadline));
    assert_eq!(sent_to(&c, lease.cid), vec![ControlMessage::Abort { flow }]);
    let ended = c.observe().await;
    assert!(ended.cids[&lease.cid].dials.is_empty());

    c.guest_sends(session, ControlMessage::Paired { flow });
    let late = c.observe().await;
    assert_eq!(late, ended, "a late Paired changes nothing");
    assert_eq!(sent_to(&c, lease.cid), vec![ControlMessage::Abort { flow }], "and gets no reply");
    assert_eq!(capture.named("guest_flow.control_discarded").len(), 1);
}

/// R5-53 (`NotServing`, order of decision step 1): an address that is not an
/// Active lease's workload address, or a port the lease does not declare, is
/// `NotServing` at once with no state change — the gateway, port 0, an
/// address outside the guest prefix, a prefix address with no lease, an
/// undeclared port — for both purposes.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_target_no_active_lease_serves_is_not_serving_with_no_state_change() {
    let (c, lease, _session) = serving(45).await;
    let before = c.observe().await;
    let unleased =
        SocketAddrV4::new(Ipv4Addr::from(u32::from(GUEST_NODE_PREFIX.network()) + 4_000), PORT);
    let targets = [
        SocketAddrV4::new(GUEST_NODE_GATEWAY, PORT),
        lease.target(0),
        SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 7), PORT),
        unleased,
        lease.target(PORT + 1),
    ];
    for target in targets {
        for purpose in [DialPurpose::Relay, DialPurpose::Probe] {
            let outcome = c.owner.dial(target, purpose).await.map(|_| ());
            assert_eq!(outcome, Err(GuestDialError::NotServing), "{target} ({purpose:?})");
        }
    }
    assert_eq!(c.observe().await, before, "a typed refusal changes nothing");
    assert!(c.io.dials().is_empty(), "no V_h was opened");
    assert!(sent_to(&c, lease.cid).is_empty(), "no message was sent");
}

/// R5-53: a lease that is Provisioned (not yet Active) does not serve dials.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_provisioned_lease_is_not_serving() {
    let c = Composition::booted(46).await;
    let lease = c.lease("inb1-prov", &[PORT]).await;
    let _session = c.open_session(&lease, 1024);
    let before = c.observe().await;
    assert_eq!(
        c.owner.dial(lease.target(PORT), DialPurpose::Relay).await.map(|_| ()),
        Err(GuestDialError::NotServing)
    );
    assert_eq!(c.observe().await, before);
    assert!(c.io.dials().is_empty());
}

/// R5-53 (`Quiesced`, step 2): while node forwarding is quiesced a dial is
/// `Quiesced` with no state change, for both purposes.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_dial_while_quiesced_is_quiesced_with_no_state_change() {
    let (c, lease, _session) = serving(47).await;
    let (hold, _) = c.owner.quiesce().await;
    let before = c.observe().await;
    for purpose in [DialPurpose::Relay, DialPurpose::Probe] {
        assert_eq!(
            c.owner.dial(lease.target(PORT), purpose).await.map(|_| ()),
            Err(GuestDialError::Quiesced)
        );
    }
    assert_eq!(c.observe().await, before);
    assert!(c.io.dials().is_empty());
    assert_eq!(c.owner.restore(hold).await, Ok(ForwardingRestore::Reopened));
}

/// R5-53 (`NoSession`, step 3): an Active lease whose control session the
/// owner has observed lost is `NoSession` with no state change — also with
/// the VM gone, once the loss is observed.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_dial_after_observed_session_loss_is_no_session() {
    let (c, lease, session) = serving(48).await;
    c.io.end_connection(session, SimPeerEnd::Eof);
    let before = c.observe().await;
    assert_eq!(before.cids[&lease.cid].session, None);
    assert_eq!(
        c.owner.dial(lease.target(PORT), DialPurpose::Relay).await.map(|_| ()),
        Err(GuestDialError::NoSession)
    );
    assert_eq!(c.observe().await, before);
    c.io.vm_stop(lease.cid);
    assert_eq!(
        c.owner.dial(lease.target(PORT), DialPurpose::Relay).await.map(|_| ()),
        Err(GuestDialError::NoSession),
        "a gone VM whose loss is observed is NoSession, not Unreachable"
    );
}

/// R5-53 (`Unreachable { ENODEV }`): a gone VM whose session loss the owner
/// has not observed: the vsock connect fails `ENODEV`; nothing is
/// registered and nothing is sent.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_dial_to_a_gone_vm_whose_loss_is_unobserved_is_unreachable() {
    let (c, lease, session) = serving(49).await;
    c.io.set_auto_deliver(false);
    c.io.vm_stop(lease.cid);
    let before = c.observe().await;
    assert_eq!(before.cids[&lease.cid].session, Some(session), "the loss is not yet observed");
    assert_eq!(
        c.owner.dial(lease.target(PORT), DialPurpose::Relay).await.map(|_| ()),
        Err(GuestDialError::Unreachable { errno: KernelErrno::ENODEV })
    );
    let after = c.observe().await;
    assert!(after.cids[&lease.cid].dials.is_empty());
    assert_eq!(after.cids[&lease.cid].held_flows, 0);
    assert!(sent_to(&c, lease.cid).is_empty(), "a connect failure opens no flow and sends nothing");
}

/// R5-53 (`Unreachable { ECONNRESET }`): a VMM exit with the dial in flight —
/// the device-release sweep resets `V_h`; the dial returns
/// `Unreachable { ECONNRESET }`, never `PairingDeadline`; no `Abort` is sent
/// (the same sweep resets the session).
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_vmm_exit_during_a_dial_is_unreachable_reset() {
    let (c, lease, _session) = serving(50).await;
    let pending = c.spawn_dial(lease.target(PORT), DialPurpose::Relay);
    c.settle().await;
    let flow = c.dial_flow(0);
    c.io.set_auto_deliver(false);
    c.io.vm_stop(lease.cid);
    c.settle().await;
    assert_eq!(
        pending.await.expect("no panic").map(|_| ()),
        Err(GuestDialError::Unreachable { errno: KernelErrno::ECONNRESET })
    );
    c.advance(GUEST_DIAL_PAIRING_DEADLINE * 2).await;
    assert!(!sent_to(&c, lease.cid).contains(&ControlMessage::Abort { flow }));
    assert!(c.observe().await.cids[&lease.cid].dials.is_empty());
}

/// R5-53 (in-flight end, teardown) → `NotServing`.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn teardown_ends_a_pending_dial_with_not_serving() {
    let (c, lease, _session) = serving(51).await;
    let pending = c.spawn_dial(lease.target(PORT), DialPurpose::Relay);
    c.settle().await;
    c.owner.teardown(&lease.plan).await.expect("teardown");
    c.settle().await;
    assert_eq!(pending.await.expect("no panic").map(|_| ()), Err(GuestDialError::NotServing));
    assert!(c.io.dials()[0].closed);
    assert!(!c.observe().await.cids.contains_key(&lease.cid));
}

/// R5-53 (in-flight end, quiescence) → `Quiesced`, `Abort { flow }` on the
/// live session.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn quiescence_ends_a_pending_dial_with_quiesced_and_sends_abort() {
    let (c, lease, _session) = serving(52).await;
    let pending = c.spawn_dial(lease.target(PORT), DialPurpose::Relay);
    c.settle().await;
    let flow = c.dial_flow(0);
    let (hold, _) = c.owner.quiesce().await;
    c.settle().await;
    assert_eq!(pending.await.expect("no panic").map(|_| ()), Err(GuestDialError::Quiesced));
    assert!(sent_to(&c, lease.cid).contains(&ControlMessage::Abort { flow }));
    let observed = c.observe().await;
    assert!(observed.cids[&lease.cid].dials.is_empty());
    assert_eq!(observed.cids[&lease.cid].held_flows, 0);
    assert_eq!(c.owner.restore(hold).await, Ok(ForwardingRestore::Reopened));
}

/// R5-53 (in-flight end, observed session loss) → `NoSession`; no `Abort`
/// (the session is gone).
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn session_loss_ends_a_pending_dial_with_no_session() {
    let (c, lease, session) = serving(53).await;
    let pending = c.spawn_dial(lease.target(PORT), DialPurpose::Relay);
    c.settle().await;
    let flow = c.dial_flow(0);
    c.io.end_connection(session, SimPeerEnd::Eof);
    c.settle().await;
    assert_eq!(pending.await.expect("no panic").map(|_| ()), Err(GuestDialError::NoSession));
    assert!(!sent_to(&c, lease.cid).contains(&ControlMessage::Abort { flow }));
    assert!(c.observe().await.cids[&lease.cid].dials.is_empty());
}

/// R5-53 (cancellation): dropping the dial future ends the dial exactly as
/// the pairing deadline does — `V_h` closed, `Abort { flow }` on the live
/// session, slot, watch and quota place released, a late reply discarded.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_dropped_dial_future_ends_the_dial_like_the_pairing_deadline() {
    let (c, lease, session) = serving(54).await;
    let pending = c.spawn_dial(lease.target(PORT), DialPurpose::Relay);
    c.settle().await;
    let flow = c.dial_flow(0);
    pending.abort();
    c.settle().await;
    assert!(c.io.dials()[0].closed);
    assert_eq!(sent_to(&c, lease.cid), vec![ControlMessage::Abort { flow }]);
    let ended = c.observe().await;
    assert!(ended.cids[&lease.cid].dials.is_empty());
    assert_eq!(ended.cids[&lease.cid].held_flows, 0);
    assert_eq!(c.io.forwarder_inventory().live_watches, 0);
    c.guest_sends(session, ControlMessage::Paired { flow });
    assert_eq!(c.observe().await, ended);
}

/// R5-53 (`Capacity`, step 4) / PROBE-NOSAMPLE: with the allocation's quota
/// (4,096) held by `Relay` dials, another `Relay` dial is `Capacity` with no
/// state change, and a `Probe` dial takes a node slot and no quota place
/// and proceeds.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_quota_refuses_relay_dials_with_capacity_and_exempts_probe_dials() {
    let (c, lease, _session) = serving(55).await;
    let quota = usize::try_from(GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION).expect("fits usize");
    let mut relays = Vec::with_capacity(quota);
    for _ in 0..quota {
        relays.push(c.spawn_dial(lease.target(PORT), DialPurpose::Relay));
    }
    c.settle().await;
    let full = c.observe().await;
    assert_eq!(full.cids[&lease.cid].held_flows, GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION);

    assert_eq!(
        c.owner.dial(lease.target(PORT), DialPurpose::Relay).await.map(|_| ()),
        Err(GuestDialError::Capacity)
    );
    assert_eq!(c.observe().await, full, "the refusal changes nothing");

    let probe = c.spawn_dial(lease.target(PORT), DialPurpose::Probe);
    c.settle().await;
    let with_probe = c.observe().await;
    assert_eq!(with_probe.cids[&lease.cid].dials.len(), quota + 1, "the probe dial is registered");
    assert_eq!(
        with_probe.cids[&lease.cid].held_flows, GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION,
        "a probe dial takes no quota place"
    );
    assert_eq!(c.io.forwarder_inventory().live_watches, u32::try_from(quota + 1).expect("fits"));
    probe.abort();
    for relay in relays {
        relay.abort();
    }
    c.settle().await;
    let released = c.observe().await;
    assert!(released.cids[&lease.cid].dials.is_empty(), "every cancelled dial released its slot");
    assert_eq!(released.cids[&lease.cid].held_flows, 0);
}

/// R5-53 (teardown release): the allocation's teardown ends its open
/// streams and in-flight dials and releases every slot, watch and quota
/// place, whatever the streams' states.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn teardown_releases_every_dial_and_stream_of_the_allocation() {
    let (c, lease, session) = serving(56).await;
    let (mut stream, _flow) = paired_stream(&c, &lease, session).await;
    let _pending = c.spawn_dial(lease.target(PORT), DialPurpose::Probe);
    c.settle().await;
    c.owner.teardown(&lease.plan).await.expect("teardown");
    c.settle().await;
    let mut buf = [0_u8; 4];
    let read = stream.read(&mut buf).await;
    assert!(matches!(read, Err(ref error) if error.kind() == io::ErrorKind::ConnectionAborted));
    assert!(!c.observe().await.cids.contains_key(&lease.cid));
    assert_eq!(c.io.forwarder_inventory().live_watches, 0);
}

/// R5-63 (b): the guest half-closes its direction — reads return the bytes,
/// then `Ok(0)` (resolved from the watch's `GuestHalfClose` record); `end()`
/// is `HalfClose`; the owner reports the class; writes still succeed; the
/// dialer's half-close sends `0x2` and is reported as `host_shut`.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_stream_reports_the_guests_half_close_and_keeps_its_own_direction() {
    let (c, lease, session) = serving(57).await;
    let (mut stream, flow) = paired_stream(&c, &lease, session).await;
    c.io.stream_guest_write(flow, b"tail");
    c.io.guest_ends_stream(flow, SimGuestStreamEnd::HalfClose);
    let mut buf = [0_u8; 8];
    let read = stream.read(&mut buf).await.expect("guest bytes");
    assert_eq!(&buf[..read], b"tail");
    assert_eq!(stream.read(&mut buf).await.expect("a clean end"), 0);
    assert_eq!(stream.end(), Some(GuestStreamEnd::HalfClose));
    let observed = c.observe().await;
    assert!(matches!(
        dial_of(&observed, &lease, flow),
        Some(DialObservation {
            phase: DialPhase::Paired {
                guest_end: Some(GuestStreamEnd::HalfClose),
                host_shut: false,
                ..
            },
            ..
        })
    ));
    assert_eq!(stream.write(b"x").await.expect("the host direction is open"), 1);
    stream.shutdown_write().await.expect("the dialer half-closes");
    assert_eq!(c.io.vsock_of(flow).sent_shutdown(), Some(SimShutdown::Send));
    let shut = c.observe().await;
    assert!(matches!(
        dial_of(&shut, &lease, flow),
        Some(DialObservation {
            phase: DialPhase::Paired {
                guest_end: Some(GuestStreamEnd::HalfClose),
                host_shut: true,
                ..
            },
            ..
        })
    ));
}

/// R5-63 (c): the guest resets — the holder's own read of `V_h` returns 0,
/// yet the stream reports `Err(ConnectionReset)` from the watch's
/// `Aborted` record (a read's return is never the classification).
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_guest_abort_is_reported_as_a_reset_whatever_the_read_returned() {
    let (c, lease, session) = serving(58).await;
    let (mut stream, flow) = paired_stream(&c, &lease, session).await;
    c.io.guest_ends_stream(flow, SimGuestStreamEnd::Abort);
    let mut buf = [0_u8; 8];
    let read = stream.read(&mut buf).await;
    assert!(matches!(read, Err(ref error) if error.kind() == io::ErrorKind::ConnectionReset));
    assert_eq!(stream.end(), Some(GuestStreamEnd::Abort));
    let write = stream.write(b"x").await;
    assert!(matches!(write, Err(ref error) if error.kind() == io::ErrorKind::ConnectionReset));
}

/// R5-63 (a) / EVMOD-STREAM-ABORT / WATCH-ABORT: quiescence's first hold
/// aborts a stream while the holder is blocked in `read` — the call returns
/// `Err(ConnectionAborted)` at once, `V_h` is untouched (no SHUTDOWN sent),
/// the guest receives `Abort { flow }`, and the holder's later drop closes
/// `V_h` with no second effect.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_owner_abort_ends_a_blocked_read_at_once_and_leaves_vh_untouched() {
    let (c, lease, session) = serving(59).await;
    let (mut stream, flow) = paired_stream(&c, &lease, session).await;
    let reader = tokio::spawn(async move {
        let mut buf = [0_u8; 8];
        let outcome = stream.read(&mut buf).await;
        (stream, outcome)
    });
    c.settle().await;
    assert!(!reader.is_finished(), "the holder is blocked in read");
    let (hold, _) = c.owner.quiesce().await;
    c.settle().await;
    let (stream, outcome) = reader.await.expect("no panic");
    assert!(matches!(outcome, Err(ref error) if error.kind() == io::ErrorKind::ConnectionAborted));
    assert_eq!(c.io.vsock_of(flow).sent_shutdown(), None, "the owner's abort sends nothing on V_h");
    assert!(sent_to(&c, lease.cid).contains(&ControlMessage::Abort { flow }));
    assert_eq!(stream.end(), Some(GuestStreamEnd::Abort));
    let observed = c.observe().await;
    assert!(observed.cids[&lease.cid].dials.is_empty());
    assert_eq!(observed.cids[&lease.cid].held_flows, 0, "released at the owner's abort");
    let effects = c.io.issued().len();
    drop(stream);
    c.settle().await;
    assert_eq!(c.io.sent_to_guest(flow), vec![VsockShutdownOp::Close], "the drop only closes V_h");
    assert!(
        !c.io.issued()[effects..]
            .iter()
            .any(|effect| effect.kind
                == crate::guest_network::flow_io::sim::SimEffectKind::AbortForward),
        "no second abort of the watch"
    );
    assert_eq!(c.owner.restore(hold).await, Ok(ForwardingRestore::Reopened));
}

/// R5-63 (d) / U-EV-11: the VMM exits while the holder is not blocked; the
/// holder may observe the end before the sweep's callback ran — the stream
/// waits for the watch's record and then reports `Err(ConnectionReset)`.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_vmm_exit_is_resolved_from_the_record_in_flight() {
    let (c, lease, session) = serving(60).await;
    let (stream, _flow) = paired_stream(&c, &lease, session).await;
    c.io.vm_stop_before_callbacks(lease.cid, lease.launch);
    let reader = tokio::spawn(async move {
        let mut stream = stream;
        let mut buf = [0_u8; 8];
        let outcome = stream.read(&mut buf).await;
        (stream, outcome)
    });
    c.settle().await;
    assert!(!reader.is_finished(), "the stream waits for the record in flight");
    c.io.finish_sweep(lease.cid);
    c.settle().await;
    let (_stream, outcome) = reader.await.expect("no panic");
    assert!(matches!(outcome, Err(ref error) if error.kind() == io::ErrorKind::ConnectionReset));
}

/// R5-63 (f) / EVMOD-CTL (the `Forwarder` recovery in its pinned order,
/// played through the owner's driving ports as the supervisor plays it):
/// a context `Io` damages `Forwarder`; the first hold aborts every flow and
/// stream (`abort_all`), so the open stream reports `ConnectionAborted`;
/// the repair closes the old context before it opens and probes a new one
/// (`forwarder_generation` + 1, the old context released holding nothing);
/// the audit is clean and the restore reopens; a new dial pairs in the new
/// context.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_forwarder_recovery_ends_streams_then_reopens_the_context() {
    let (c, lease, session) = serving(61).await;
    let (mut stream, _flow) = paired_stream(&c, &lease, session).await;
    let generation = c.observe().await.forwarder_generation;
    c.io.script_failure(
        crate::guest_network::flow_io::sim::SimEffectKind::Inventory,
        crate::guest_network::flow_io::sim::SimFailure::Forwarder(
            crate::guest_network::flow_io::sim::SimForwarderFailure::ContextIo(
                overdrive_core::guest_transport::forwarder::ForwarderOp::Inventory,
            ),
        ),
    );
    let damaged = c.owner.audit_shared().await.expect_err("the context's Io damages Forwarder");
    assert_eq!(
        damaged.component,
        overdrive_core::guest_network::SharedGuestNetworkComponent::Forwarder
    );
    let (hold, _) = c.owner.quiesce().await;
    let mut buf = [0_u8; 8];
    let read = stream.read(&mut buf).await;
    assert!(matches!(read, Err(ref error) if error.kind() == io::ErrorKind::ConnectionAborted));
    c.owner.converge_shared().await.expect("the Forwarder repair reopens the context");
    let repaired = c.observe().await;
    assert_eq!(repaired.forwarder_generation, generation.next().expect("a next generation"));
    let contexts = c.io.forwarder_contexts();
    assert_eq!(contexts.len(), 3, "phase-1 probe, old serving, replacement serving");
    assert!(
        !contexts[0].open && contexts[0].live_pairs == 0 && contexts[0].live_watches == 0,
        "the phase-1 probe context is released empty"
    );
    assert!(
        !contexts[1].open && contexts[1].live_pairs == 0 && contexts[1].live_watches == 0,
        "the old serving context is released empty before replacement"
    );
    assert!(
        contexts[2].open && contexts[2].live_pairs == 0 && contexts[2].live_watches == 0,
        "the replacement serving context is open and initially empty"
    );
    c.owner.audit_shared().await.expect("the audit after the repair is clean");
    assert_eq!(c.owner.restore(hold).await, Ok(ForwardingRestore::Reopened));
    drop(stream);
    let (_next, _next_flow) =
        c.paired_dial(session, lease.target(PORT), DialPurpose::Relay, 1).await;
    let reopened = c.observe().await;
    assert!(reopened.cids[&lease.cid].dials.values().all(|dial| matches!(
        dial.phase,
        DialPhase::Paired { generation, .. } if generation == reopened.forwarder_generation
    )));
}

/// R5-63 (f), the release branch: when the first hold's `abort_all` fails on
/// the context (`Io`), the owner still ends every stream; the repair's
/// release of the old context shuts the watched `V_h` both ways, and the
/// holder — whose own read then returns 0 or fails `ENOTCONN` — still
/// reports the owner's abort.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_context_release_is_reported_as_the_owners_abort() {
    let (c, lease, session) = serving(64).await;
    let (mut stream, flow) = paired_stream(&c, &lease, session).await;
    c.io.script_failure(
        crate::guest_network::flow_io::sim::SimEffectKind::AbortAll,
        crate::guest_network::flow_io::sim::SimFailure::Forwarder(
            crate::guest_network::flow_io::sim::SimForwarderFailure::ContextIo(
                overdrive_core::guest_transport::forwarder::ForwarderOp::AbortAll,
            ),
        ),
    );
    let (hold, _) = c.owner.quiesce().await;
    let damaged = c.owner.audit_shared().await.expect_err("the failed abort_all damages Forwarder");
    assert_eq!(
        damaged.component,
        overdrive_core::guest_network::SharedGuestNetworkComponent::Forwarder
    );
    c.owner.converge_shared().await.expect("the Forwarder repair reopens the context");
    assert_eq!(
        c.io.vsock_of(flow).sent_shutdown(),
        Some(SimShutdown::Both),
        "the release shut V_h"
    );
    let mut buf = [0_u8; 8];
    let read = stream.read(&mut buf).await;
    assert!(matches!(read, Err(ref error) if error.kind() == io::ErrorKind::ConnectionAborted));
    assert!(c.observe().await.cids[&lease.cid].dials.is_empty());
    c.owner.audit_shared().await.expect("clean after the repair");
    assert_eq!(c.owner.restore(hold).await, Ok(ForwardingRestore::Reopened));
}

/// R5-28 (host half, U-5): a `Paired` from the guest for a host-opened flow
/// the host aborted by quiescence is discarded and counted, with no reply.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_paired_for_a_quiesced_dial_is_discarded_with_no_reply() {
    let (capture, _guard) = EventCapture::install();
    let (c, lease, session) = serving(62).await;
    let pending = c.spawn_dial(lease.target(PORT), DialPurpose::Relay);
    c.settle().await;
    let flow = c.dial_flow(0);
    let (hold, _) = c.owner.quiesce().await;
    c.settle().await;
    assert_eq!(pending.await.expect("no panic").map(|_| ()), Err(GuestDialError::Quiesced));
    let replies = sent_to(&c, lease.cid);
    let before = c.observe().await;
    c.guest_sends(session, ControlMessage::Paired { flow });
    assert_eq!(c.observe().await, before);
    assert_eq!(sent_to(&c, lease.cid), replies, "no reply");
    assert_eq!(capture.named("guest_flow.control_discarded").len(), 1);
    assert!(c.io.dials()[0].closed, "the host's close of V_h reaches the guest's V_g (K-A4)");
    assert_eq!(c.owner.restore(hold).await, Ok(ForwardingRestore::Reopened));
}

/// R5-31 (host half, H-1): the host aborts a host-opened flow before the
/// guest accepted `V_g` — by control-session loss followed by a reconnect —
/// and wrote nothing on `V_h` after the request; after the reconnect a new
/// dial pairs (`DialAfterReconnectPairs`).
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_dial_aborted_by_session_loss_writes_nothing_more_and_a_new_dial_pairs() {
    let (c, lease, session) = serving(63).await;
    let pending = c.spawn_dial(lease.target(PORT), DialPurpose::Relay);
    c.settle().await;
    c.io.end_connection(session, SimPeerEnd::Eof);
    c.settle().await;
    assert_eq!(pending.await.expect("no panic").map(|_| ()), Err(GuestDialError::NoSession));
    let dial = c.io.dials()[0];
    assert!(dial.closed && !dial.streamed);
    let reconnected = c.open_session(&lease, 1100);
    c.settle().await;
    assert_eq!(c.observe().await.cids[&lease.cid].session, Some(reconnected));
    let (_stream, _flow) =
        c.paired_dial(reconnected, lease.target(PORT), DialPurpose::Relay, 1).await;
}
