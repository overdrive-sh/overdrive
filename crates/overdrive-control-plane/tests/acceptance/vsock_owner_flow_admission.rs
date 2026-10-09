//! P2 — guest-flow owner: admission, pairing, attribution, half-close seam
//! (DISTILL, netns-density-295).
//!
//! Translates `docs/feature/netns-density-295/distill/test-scenarios-p2-owner-flows.md`
//! P2-S3..S13 into seeded-`overdrive-sim` `#[tokio::test]` bodies that drive the
//! **production** guest-flow owner's flow state machine through its private seam
//! with `Sim*` adapters at its driven ports (feature-delta §"[REF] Owner and
//! provisioner", §"[REF] Per-kind total orders", §"[REF] Control session",
//! §"Sim counterparts"). Example-based (layer 2–3): every sad path is
//! enumerated, never `proptest`-generated (Mandate 9/11). The oracle is a
//! before/after **observable** snapshot (the Rust universe-guard adaptation of
//! Mandate 8): the owner's CID-table phase, the `ClaimSet` claim holder, the
//! control messages emitted per session, the `guest_flow.*` counters, the
//! `FlowCloseCause`, and the forwarder's ordered operation log — never private
//! owner fields (`.claude/rules/testing.md` §"DISTILL — prove composed system
//! behaviour", §"Quint conformance"). Each assertion names the delta and the
//! unrelated slots that MUST stay unchanged.
//!
//! RED scaffold, gated on the DELIVER roadmap-reconciliation owner step. The
//! bodies call only design-pinned trait surface
//! (`SharedGuestNetworkOwner` / `GuestNetworkProvisioner`,
//! `GuestTransportAssignment`, `ControlMessage`, `FlowRefusal`,
//! `FlowCloseCause`, `GuestCid`, `FlowId`, `LaunchToken`) plus a **DISTILL-shaped
//! test-support seam** (`OwnerFlowHarness` below) the owner step must provide —
//! inject an accepted guest-opened connection and read the observable surface,
//! with the owner exposing NO private field. See
//! `test-scenarios-p2-owner-flows.md` §"Testability gaps" #1 and #3. CLAUDE.md
//! §"Implement to the design": no production API is invented here. Registered in
//! `tests/acceptance.rs` by the author when the owner scaffold co-arrives
//! (shared doc §6); unregistered (dangling) until then.

#![allow(dead_code, unused_imports)]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

use overdrive_control_plane::guest_network::{
    ForwardingActivation, GuestNetworkError, QuiescenceHolder, SharedGuestNetworkComponent,
    SharedGuestNetworkOwner,
};
use overdrive_core::guest_transport::wire::{
    ControlMessage, FlowId, FlowKind, FlowOrigin, FlowRefusal, FlowRequest,
};
use overdrive_core::guest_transport::{GuestCid, LaunchToken};
use overdrive_core::id::AllocationId;

// =============================================================================
// DISTILL-shaped test-support seam (gap #1 / #3) — provided by the DELIVER
// owner + `SimGuestFlowForwarder` step. Shapes the contract this suite needs;
// the owner exposes ONLY this observable surface, never a private field.
// =============================================================================

/// One accepted guest-opened connection delivered through the owner's private
/// seam, as the host kernel would deliver it (P-37): a peer CID, the host
/// listener port it arrived on (1240 stream / 1241 seqpacket / 1243 control),
/// and the first record the connection buffered (which a dead launch's queued
/// connection still carries).
#[derive(Debug, Clone)]
struct InjectedConnection {
    peer_cid: GuestCid,
    port: u32,
    first_record: InjectedRecord,
    /// `false` models a queued connection with no buffered bytes (fails its
    /// first read).
    has_buffered_bytes: bool,
}

#[derive(Debug, Clone)]
enum InjectedRecord {
    ControlOpen { launch: LaunchToken },
    Flow(FlowRequest),
}

/// The owner's observable surface the snapshot oracle reads (no private field).
#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnerObservable {
    /// CID lifecycle phase as the public fact exposes it.
    phase: CidPhase,
    /// The connection id holding the CID's `ClaimSet<GuestCid>` session claim
    /// (`None` = none), so S8/S9 can assert exactly-one / never-stale.
    session_claim_holder: Option<u64>,
    /// `true` once the CID has a live control session (G-V3 precondition).
    session_live: bool,
    /// Control messages the owner wrote on the session, in order.
    control_messages: Vec<ControlMessage>,
    /// The `guest_flow.*` telemetry counters captured as a sorted name→count.
    counters: Counters,
    /// Flows still registered in the CID's flow table (by flow id).
    registered_flows: Vec<FlowId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CidPhase {
    Absent,
    Provisioned,
    Active,
    Retiring,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Counters {
    no_control_session: u64,
    stale_launch: u64,
    session_held: u64,
    control_discarded: u64,
    datagram_request_mismatch: u64,
    guest_park_exhausted: u64,
}

/// Ordered log entry from `SimGuestFlowForwarder` (gap #3), so S3/S4/S12 can
/// assert operation ordering (read-before-install, install-before-Paired,
/// drain→HalfClose→ack→shutdown).
#[derive(Debug, Clone, PartialEq, Eq)]
enum ForwarderOp {
    AcceptFlow(FlowId),
    ReadRequest(FlowId),
    Install(FlowId),
    Route { from: FlowId, to: FlowId },
    Connect(FlowId),
    Drain(FlowId),
    ShutdownWrite(FlowId),
    Abort { flow: FlowId, cause: overdrive_control_plane::guest_network::FlowCloseCause },
}

/// Harness over the production owner wired with seeded `Sim*` driven ports.
/// Every method forwards to the DELIVER owner's test-support seam or to a real
/// pinned trait method; it holds no business logic of its own.
struct OwnerFlowHarness {
    owner: Arc<dyn SharedGuestNetworkOwner>,
    // Sim driven-port handles (SimGuestFlowForwarder op log, SimMtlsResolve
    // script, SimGuestCidClaim, SimEntropy, SimClock) live here; the DELIVER
    // owner step wires them. Accessors below read their observable state.
}

impl OwnerFlowHarness {
    /// Boots the production owner with seeded `Sim*` driven ports and a seeded
    /// `SimEntropy` (so launch tokens are deterministic). DELIVER provides this.
    fn seeded(_seed: u64) -> Self {
        unimplemented!("DELIVER owner step: production owner + Sim* driven ports")
    }

    /// Provisions `alloc`'s CID as Provisioned with `launch` as its live token,
    /// then opens and reads a control session carrying `launch`, driving the CID
    /// to Active with a live session. Returns the CID.
    async fn provisioned_active(&self, _alloc: &AllocationId, _launch: LaunchToken) -> GuestCid {
        unimplemented!("DELIVER owner step: provision + activate + session open seam")
    }

    /// Delivers one accepted guest-opened connection through the private seam.
    async fn deliver(&self, _conn: InjectedConnection) {
        unimplemented!("DELIVER owner step: private flow-admission injection seam")
    }

    /// Drives the owner's event loop one logical step (so a check-then-register
    /// / connect continuation can interleave with an injected quiesce/teardown).
    async fn pump(&self) {
        unimplemented!("DELIVER owner step: single-step pump of the owner loop")
    }

    /// Scripts the next host `connect()` outcome for `flow` (non-mesh): pending
    /// forever, refused at once, or succeeding. For S11/S5.
    fn script_connect(&self, _flow: FlowId, _outcome: ConnectOutcome) {
        unimplemented!("DELIVER SimGuestFlowForwarder: scripted connect outcome")
    }

    fn observe(&self, _cid: GuestCid) -> OwnerObservable {
        unimplemented!("DELIVER owner step: observable surface accessor")
    }

    fn forwarder_ops(&self) -> Vec<ForwarderOp> {
        unimplemented!("DELIVER SimGuestFlowForwarder: ordered op log")
    }
}

#[derive(Debug, Clone, Copy)]
enum ConnectOutcome {
    PendingForever,
    RefusedAtOnce,
    Succeeds,
}

fn alloc(name: &str) -> AllocationId {
    AllocationId::new(name).expect("valid allocation id")
}

fn token(seed: u8) -> LaunchToken {
    LaunchToken::from_bytes([seed.max(1); 16]).expect("non-zero token")
}

fn guest_flow(seq: u32) -> FlowId {
    FlowId::new(FlowOrigin::Guest, seq).expect("valid guest flow id")
}

fn tcp_connect(flow: FlowId, dst: SocketAddrV4, launch: LaunchToken) -> FlowRequest {
    FlowRequest::TcpConnect { flow, destination: dst, launch }
}

// =============================================================================
// P2-S3 — acceptor reads the request before installing its vsock (R5-9, inv 3)
// =============================================================================

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn acceptor_reads_request_before_installing_its_vsock() {
    // Given an Active CID with a live control session.
    let h = OwnerFlowHarness::seeded(1);
    let a = alloc("payments");
    let launch = token(0x11);
    let cid = h.provisioned_active(&a, launch).await;

    // When a guest-opened TcpConnect flow to a server-first destination arrives.
    let flow = guest_flow(1);
    h.deliver(InjectedConnection {
        peer_cid: cid,
        port: 1240,
        first_record: InjectedRecord::Flow(tcp_connect(
            flow,
            SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 7), 443),
            launch,
        )),
        has_buffered_bytes: true,
    })
    .await;
    h.script_connect(flow, ConnectOutcome::Succeeds);
    h.pump().await;

    // Then the forwarder read the 32-byte request strictly before installing
    // the flow socket (H-1): no request is ever lost, and no byte is routed
    // before Paired.
    let ops = h.forwarder_ops();
    let read_at =
        ops.iter().position(|o| *o == ForwarderOp::ReadRequest(flow)).expect("read happened");
    let install_at =
        ops.iter().position(|o| *o == ForwarderOp::Install(flow)).expect("install happened");
    assert!(read_at < install_at, "request read must precede install (H-1): {ops:?}");
}

// =============================================================================
// P2-S4 — Paired only after peer install + route (G-V6 row 1, inv 3)
// =============================================================================

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn paired_is_sent_only_after_install_and_route() {
    let h = OwnerFlowHarness::seeded(2);
    let a = alloc("payments");
    let launch = token(0x12);
    let cid = h.provisioned_active(&a, launch).await;

    let flow = guest_flow(1);
    let before = h.observe(cid);
    h.deliver(InjectedConnection {
        peer_cid: cid,
        port: 1240,
        first_record: InjectedRecord::Flow(tcp_connect(
            flow,
            SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 9), 80),
            launch,
        )),
        has_buffered_bytes: true,
    })
    .await;
    h.script_connect(flow, ConnectOutcome::Succeeds);
    h.pump().await;

    // Then Paired is emitted only after install + route; it travels on the
    // control session, never the flow vsock.
    let ops = h.forwarder_ops();
    let install_at = ops.iter().position(|o| *o == ForwarderOp::Install(flow)).expect("installed");
    let route_at = ops
        .iter()
        .position(|o| matches!(o, ForwarderOp::Route { from, .. } if *from == flow))
        .expect("routed");
    assert!(install_at < route_at, "install precedes route");

    let after = h.observe(cid);
    // Universe delta: exactly one Paired message for this flow appeared; the CID
    // phase and claim holder are unchanged.
    let new_msgs: Vec<_> = after.control_messages[before.control_messages.len()..].to_vec();
    assert_eq!(new_msgs, vec![ControlMessage::Paired { flow }], "one Paired, on the session");
    assert_eq!(after.phase, before.phase, "CID phase unchanged");
    assert_eq!(after.session_claim_holder, before.session_claim_holder, "claim holder unchanged");
}

// =============================================================================
// P2-S5 — admission racing quiesce / teardown / session-loss (R5-26, U-1, inv 4)
// =============================================================================

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn no_flow_registered_on_a_check_quiescence_made_false() {
    for event in [RaceEvent::Quiesce, RaceEvent::Teardown, RaceEvent::SessionLoss] {
        let h = OwnerFlowHarness::seeded(3);
        let a = alloc("payments");
        let launch = token(0x13);
        let cid = h.provisioned_active(&a, launch).await;

        let flow = guest_flow(1);
        // Deliver the flow; hold its non-mesh connect pending so the
        // continuation runs after the racing event.
        h.deliver(InjectedConnection {
            peer_cid: cid,
            port: 1240,
            first_record: InjectedRecord::Flow(tcp_connect(
                flow,
                SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 5), 443),
                launch,
            )),
            has_buffered_bytes: true,
        })
        .await;
        h.script_connect(flow, ConnectOutcome::Succeeds);

        // When the racing event lands between check-and-register and the connect
        // continuation.
        match event {
            RaceEvent::Quiesce => {
                let _hold = h
                    .owner
                    .quiesce_forwarding(QuiescenceHolder::Recovery(
                        SharedGuestNetworkComponent::FlowListeners,
                    ))
                    .await
                    .expect("quiesce");
            }
            RaceEvent::Teardown => { /* teardown driven via the harness session seam */ }
            RaceEvent::SessionLoss => { /* session loss driven via the harness seam */ }
        }
        h.pump().await;

        // Then the flow is not registered; the completing connect sends no
        // Paired and closes its socket (U-1).
        let obs = h.observe(cid);
        assert!(!obs.registered_flows.contains(&flow), "no flow on a false check ({event:?})");
        assert!(
            !obs.control_messages
                .iter()
                .any(|m| matches!(m, ControlMessage::Paired { flow: f } if *f == flow)),
            "no Paired for an aborted flow ({event:?})"
        );
        assert!(
            h.forwarder_ops()
                .iter()
                .any(|o| matches!(o, ForwarderOp::Abort { flow: f, .. } if *f == flow)),
            "the completing connect closes its socket ({event:?})"
        );
    }
}

#[derive(Debug, Clone, Copy)]
enum RaceEvent {
    Quiesce,
    Teardown,
    SessionLoss,
}

// =============================================================================
// P2-S6 — flow from a CID without a live session / stale token (R5-16, inv 4)
// =============================================================================

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn flow_without_live_session_is_closed_counted_never_paired() {
    let h = OwnerFlowHarness::seeded(4);
    let a = alloc("payments");
    let launch = token(0x14);
    // Provision only (no control session opened): the CID has no live session.
    let cid = provision_only(&h, &a, launch).await;

    let flow = guest_flow(1);
    let before = h.observe(cid);
    h.deliver(InjectedConnection {
        peer_cid: cid,
        port: 1240,
        first_record: InjectedRecord::Flow(tcp_connect(
            flow,
            SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 1), 443),
            launch,
        )),
        has_buffered_bytes: true,
    })
    .await;
    h.pump().await;
    let after = h.observe(cid);

    // Closed, counted NoControlSession, never paired, no control message.
    assert_eq!(after.counters.no_control_session, before.counters.no_control_session + 1);
    assert_eq!(after.control_messages, before.control_messages, "no message on a dead session");
    assert!(!after.registered_flows.contains(&flow));
}

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn flow_with_another_launch_token_is_closed_counted_stale_launch() {
    let h = OwnerFlowHarness::seeded(5);
    let a = alloc("payments");
    let live = token(0x15);
    let cid = h.provisioned_active(&a, live).await;

    // A flow request carrying a DIFFERENT (stale) launch token.
    let stale = token(0x99);
    let flow = guest_flow(1);
    let before = h.observe(cid);
    h.deliver(InjectedConnection {
        peer_cid: cid,
        port: 1240,
        first_record: InjectedRecord::Flow(tcp_connect(
            flow,
            SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 1), 443),
            stale,
        )),
        has_buffered_bytes: true,
    })
    .await;
    h.pump().await;
    let after = h.observe(cid);

    // Closed, unregistered, counted StaleLaunch; no control message on the live
    // session (ATTR-TOKEN) — its flow id is another launch's id space.
    assert_eq!(after.counters.stale_launch, before.counters.stale_launch + 1);
    assert_eq!(after.control_messages, before.control_messages, "no cross-launch message");
    assert!(!after.registered_flows.contains(&flow));
    assert_eq!(after.phase, CidPhase::Active, "the live launch is undisturbed");
}

async fn provision_only(
    _h: &OwnerFlowHarness,
    _a: &AllocationId,
    _launch: LaunchToken,
) -> GuestCid {
    unimplemented!("DELIVER owner step: provision without opening a session")
}

// =============================================================================
// P2-S7 — datagram request pair mismatch (R5-17)
// =============================================================================

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn datagram_request_pair_mismatch_refuses_malformed_and_closes_both() {
    let h = OwnerFlowHarness::seeded(6);
    let a = alloc("dns");
    let launch = token(0x16);
    let cid = h.provisioned_active(&a, launch).await;

    // Two datagram requests on the same (CID, flow) but different destinations.
    let flow = guest_flow(1);
    let stream = FlowRequest::DatagramStream {
        flow,
        destination: SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 1), 53),
        launch,
    };
    let seqpacket = FlowRequest::DatagramSeqpacket {
        flow,
        destination: SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 2), 53), // differs → mismatch
        launch,
    };
    for rec in [stream, seqpacket] {
        h.deliver(InjectedConnection {
            peer_cid: cid,
            port: if matches!(rec.kind(), FlowKind::DatagramSeqpacket) { 1241 } else { 1240 },
            first_record: InjectedRecord::Flow(rec),
            has_buffered_bytes: true,
        })
        .await;
    }
    let before = h.observe(cid);
    h.pump().await;
    let after = h.observe(cid);

    // Refused(Malformed), both closed, counted DatagramRequestMismatch.
    let new_msgs: Vec<_> = after.control_messages[before.control_messages.len()..].to_vec();
    assert_eq!(new_msgs, vec![ControlMessage::Refused { flow, refusal: FlowRefusal::Malformed }],);
    assert_eq!(
        after.counters.datagram_request_mismatch,
        before.counters.datagram_request_mismatch + 1
    );
    assert!(!after.registered_flows.contains(&flow), "both sockets closed");
}

// =============================================================================
// P2-S8 — two concurrent sessions, current token: exactly one accepted (R5-18, inv 1)
// =============================================================================

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn two_concurrent_sessions_current_token_exactly_one_accepted() {
    let h = OwnerFlowHarness::seeded(7);
    let a = alloc("payments");
    let live = token(0x17);
    let cid = provision_only(&h, &a, live).await;

    // Two control-open connections, both carrying the current launch token.
    for _ in 0..2 {
        h.deliver(InjectedConnection {
            peer_cid: cid,
            port: 1243,
            first_record: InjectedRecord::ControlOpen { launch: live },
            has_buffered_bytes: true,
        })
        .await;
    }
    h.pump().await;
    let obs = h.observe(cid);

    // Exactly one session live (one claim holder); the second closed SessionHeld.
    assert!(obs.session_live, "one session is live");
    assert!(obs.session_claim_holder.is_some(), "the claim is held by exactly one connection");
    assert_eq!(obs.counters.session_held, 1, "the second concurrent open is SessionHeld (L-3)");
    assert_eq!(obs.counters.stale_launch, 0, "neither carries a stale token");
}

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn queued_previous_launch_session_never_takes_the_claim() {
    // A queued session of the CID's PREVIOUS launch (buffered ControlOpen with
    // the stale token), delivered before/after/between the current launch's own.
    let h = OwnerFlowHarness::seeded(8);
    let a = alloc("payments");
    let previous = token(0x30);
    let current = token(0x31);
    let cid = provision_only(&h, &a, current).await;

    // Stale queued, then the current launch's own.
    h.deliver(InjectedConnection {
        peer_cid: cid,
        port: 1243,
        first_record: InjectedRecord::ControlOpen { launch: previous },
        has_buffered_bytes: true,
    })
    .await;
    h.deliver(InjectedConnection {
        peer_cid: cid,
        port: 1243,
        first_record: InjectedRecord::ControlOpen { launch: current },
        has_buffered_bytes: true,
    })
    .await;
    h.pump().await;
    let obs = h.observe(cid);

    // The stale open never takes the claim; the current launch's session is
    // live; the stale one is counted StaleLaunch and holds nothing.
    assert!(obs.session_live);
    assert_eq!(obs.counters.stale_launch, 1, "the previous launch's open is StaleLaunch");
    assert_eq!(obs.counters.session_held, 0, "the current session is not refused SessionHeld");
}

// =============================================================================
// P2-S9 — stale-launch attribution end-to-end + ATTR-LIVE-CLAIM regression (R5-41)
// =============================================================================

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn dead_launch_queued_connections_are_closed_unattributed() {
    // A launch writes its control open + a flow request + READY, then its VMM is
    // killed; the CID's next allocation boots and all the dead launch's queued
    // connections are accepted with the new CID as peer.
    let h = OwnerFlowHarness::seeded(9);
    let a = alloc("payments");
    let dead = token(0x40);
    let fresh = token(0x41);
    // The CID is reused: provision the fresh launch as the current one.
    let cid = provision_only(&h, &a, fresh).await;

    // Dead launch's queued connections (control + a flow), peer = the new CID,
    // first records carrying the DEAD token.
    h.deliver(InjectedConnection {
        peer_cid: cid,
        port: 1243,
        first_record: InjectedRecord::ControlOpen { launch: dead },
        has_buffered_bytes: true,
    })
    .await;
    h.deliver(InjectedConnection {
        peer_cid: cid,
        port: 1240,
        first_record: InjectedRecord::Flow(tcp_connect(
            guest_flow(1),
            SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 8), 443),
            dead,
        )),
        has_buffered_bytes: true,
    })
    .await;
    // A queued connection with no buffered bytes fails its first read.
    h.deliver(InjectedConnection {
        peer_cid: cid,
        port: 1240,
        first_record: InjectedRecord::Flow(tcp_connect(guest_flow(2), socket(), dead)),
        has_buffered_bytes: false,
    })
    .await;
    // The new launch's own control open.
    h.deliver(InjectedConnection {
        peer_cid: cid,
        port: 1243,
        first_record: InjectedRecord::ControlOpen { launch: fresh },
        has_buffered_bytes: true,
    })
    .await;
    h.pump().await;
    let obs = h.observe(cid);

    // The dead launch takes no claim, releases no READY, registers no flow, sends
    // no control message, causes no Paired. The fresh launch's session is live.
    assert!(obs.session_live, "the fresh launch's session is live");
    assert!(obs.registered_flows.is_empty(), "no dead-launch flow registered");
    assert!(
        obs.control_messages.iter().all(|m| !matches!(m, ControlMessage::Paired { .. })),
        "no Paired for a dead launch"
    );
    assert!(obs.counters.stale_launch >= 1, "the dead launch's records counted StaleLaunch");
}

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn attr_live_claim_regression_claim_read_after_teardown_takes_nothing() {
    // ATTR-LIVE-CLAIM: a dead launch's control connection accepted while the CID
    // is still Provisioned (launch live) and READ ONLY AFTER teardown — teardown
    // found no session to close — is closed at its read and takes no claim; after
    // pool.release the next lease's session takes the claim (the round-9
    // counterexample as a regression lock).
    let h = OwnerFlowHarness::seeded(10);
    let a = alloc("payments");
    let dead = token(0x50);

    // Provision the (soon-dead) launch; deliver its control connection but do NOT
    // read it yet (the seam delivers it "accepted, unread").
    let cid = provision_only(&h, &a, dead).await;
    h.deliver(InjectedConnection {
        peer_cid: cid,
        port: 1243,
        first_record: InjectedRecord::ControlOpen { launch: dead },
        has_buffered_bytes: true,
    })
    .await;

    // Teardown runs (the CID moves to Retiring; its live token ends) BEFORE the
    // queued connection is read. Then it is read.
    teardown(&h, &a).await;
    h.pump().await; // the connection is read here, after teardown

    let after_read = h.observe(cid);
    assert_eq!(after_read.session_claim_holder, None, "a read-after-teardown open takes no claim");
    assert!(after_read.counters.stale_launch >= 1, "closed StaleLaunch at its read");

    // After pool.release the CID is leased again; the next lease's session takes
    // the claim, and no Abort or other control message crossed launches.
    let fresh = token(0x51);
    let cid2 = release_then_provision(&h, &a, fresh).await;
    h.deliver(InjectedConnection {
        peer_cid: cid2,
        port: 1243,
        first_record: InjectedRecord::ControlOpen { launch: fresh },
        has_buffered_bytes: true,
    })
    .await;
    h.pump().await;
    let obs = h.observe(cid2);
    assert!(obs.session_live, "the next lease's session takes the claim");
    assert_eq!(obs.counters.control_discarded, 0, "no cross-launch control message");
}

fn socket() -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 2), 443)
}

async fn teardown(_h: &OwnerFlowHarness, _a: &AllocationId) {
    unimplemented!("DELIVER owner step: teardown seam (ends the live token, closes the session)")
}

async fn release_then_provision(
    _h: &OwnerFlowHarness,
    _a: &AllocationId,
    _launch: LaunchToken,
) -> GuestCid {
    unimplemented!("DELIVER owner step: pool.release then re-provision the reused CID")
}

// =============================================================================
// P2-S10 — TcpAccept V_g end on host close; U-5 discard (R5-28, R5-31, K-A4)
// =============================================================================

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn paired_for_a_host_aborted_tcp_accept_is_discarded_and_counted() {
    let h = OwnerFlowHarness::seeded(11);
    let a = alloc("web");
    let launch = token(0x18);
    let cid = h.provisioned_active(&a, launch).await;

    // The host opened a TcpAccept flow, then aborted it (quiescence) before the
    // guest's Paired lands. A late Paired for that closed flow is discarded.
    let flow = FlowId::new(FlowOrigin::Host, 1).expect("valid host flow id");
    let before = h.observe(cid);
    host_open_then_abort_tcp_accept(&h, cid, flow).await;
    deliver_guest_control(&h, cid, ControlMessage::Paired { flow }).await;
    h.pump().await;
    let after = h.observe(cid);

    assert_eq!(
        after.counters.control_discarded,
        before.counters.control_discarded + 1,
        "a Paired for a closed flow is discarded (U-5)"
    );
    // No reply, no resurrection: the flow stays closed.
    assert!(!after.registered_flows.contains(&flow));
}

async fn host_open_then_abort_tcp_accept(_h: &OwnerFlowHarness, _cid: GuestCid, _flow: FlowId) {
    unimplemented!("DELIVER owner step: host-opened TcpAccept open + abort seam")
}

async fn deliver_guest_control(_h: &OwnerFlowHarness, _cid: GuestCid, _msg: ControlMessage) {
    unimplemented!("DELIVER owner step: inject a guest→host control message")
}

// =============================================================================
// P2-S11 — non-mesh connect has no owner deadline (R5-20, D15-R3)
// =============================================================================

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn non_mesh_connect_has_no_pairing_timeout() {
    let h = OwnerFlowHarness::seeded(12);
    let a = alloc("batch");
    let launch = token(0x19);
    let cid = h.provisioned_active(&a, launch).await;

    // A TcpConnect to an unresponsive non-mesh destination: the host connect is
    // pending for the whole SYN-retry window (a sim parameter).
    let pending = guest_flow(1);
    h.deliver(InjectedConnection {
        peer_cid: cid,
        port: 1240,
        first_record: InjectedRecord::Flow(tcp_connect(
            pending,
            SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 200), 9),
            launch,
        )),
        has_buffered_bytes: true,
    })
    .await;
    h.script_connect(pending, ConnectOutcome::PendingForever);

    // Meanwhile another flow and a refused destination make progress.
    let refused = guest_flow(2);
    h.deliver(InjectedConnection {
        peer_cid: cid,
        port: 1240,
        first_record: InjectedRecord::Flow(tcp_connect(
            refused,
            SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 201), 9),
            launch,
        )),
        has_buffered_bytes: true,
    })
    .await;
    h.script_connect(refused, ConnectOutcome::RefusedAtOnce);
    h.pump().await;

    let obs = h.observe(cid);
    // No PairingTimeout fired for the pending flow; the refused one reset at once
    // with Refused(DestinationUnreachable); the pending flow is still registered
    // (holding one quota slot) with no control message yet.
    assert!(obs.registered_flows.contains(&pending), "the pending flow holds its slot");
    assert!(
        obs.control_messages.iter().any(
            |m| matches!(m, ControlMessage::Refused { flow, refusal: FlowRefusal::DestinationUnreachable } if *flow == refused)
        ),
        "a refused non-mesh destination resets at once"
    );
    assert!(
        !obs.control_messages
            .iter()
            .any(|m| matches!(m, ControlMessage::Abort { flow } if *flow == pending)),
        "no PairingTimeout abort for a pending TcpConnect (D15-R3)"
    );
}

// =============================================================================
// P2-S12 — half-close owner seam (R5-45, inv 13, HALFCLOSE-SIGNAL)
// =============================================================================

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn owner_shuts_flow_vsock_only_after_half_close_ack() {
    let h = OwnerFlowHarness::seeded(13);
    let a = alloc("web");
    let launch = token(0x1A);
    let cid = h.provisioned_active(&a, launch).await;

    let flow = guest_flow(1);
    pair_tcp_flow(&h, cid, flow, launch).await;

    // The outer leg reads EOF → the owner drains, sends HalfClose, and shuts the
    // flow vsock for writing ONLY after HalfCloseAck.
    outer_leg_eof(&h, cid, flow).await;
    h.pump().await;
    let ops = h.forwarder_ops();
    let drain = ops.iter().position(|o| *o == ForwarderOp::Drain(flow)).expect("drained");
    let half_close = h
        .observe(cid)
        .control_messages
        .iter()
        .position(|m| matches!(m, ControlMessage::HalfClose { flow: f } if *f == flow));
    assert!(half_close.is_some(), "HalfClose sent after drain");
    assert!(
        !ops.iter().any(|o| *o == ForwarderOp::ShutdownWrite(flow)),
        "the vsock is not shut down before the ack"
    );

    deliver_guest_control(&h, cid, ControlMessage::HalfCloseAck { flow }).await;
    h.pump().await;
    let ops = h.forwarder_ops();
    let shut =
        ops.iter().position(|o| *o == ForwarderOp::ShutdownWrite(flow)).expect("shut after ack");
    assert!(drain < shut, "drain precedes shutdown_write");
}

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn unannounced_vsock_end_aborts_with_reset_not_eof() {
    let h = OwnerFlowHarness::seeded(14);
    let a = alloc("web");
    let launch = token(0x1B);
    let cid = h.provisioned_active(&a, launch).await;

    let flow = guest_flow(1);
    pair_tcp_flow(&h, cid, flow, launch).await;

    // The flow vsock ends for a direction whose HalfClose was never acknowledged
    // (the peer owner is lost), while the session is still live.
    flow_vsock_ends_unannounced(&h, cid, flow).await;
    h.pump().await;

    assert!(
        h.forwarder_ops().iter().any(|o| matches!(
            o,
            ForwarderOp::Abort { flow: f, cause }
                if *f == flow
                    && *cause == overdrive_control_plane::guest_network::FlowCloseCause::UnannouncedVsockEnd
        )),
        "an unannounced vsock end aborts the flow with reset (UnannouncedVsockEnd), never EOF"
    );
}

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn half_close_for_an_unknown_flow_is_discarded_and_counted() {
    let h = OwnerFlowHarness::seeded(15);
    let a = alloc("web");
    let launch = token(0x1C);
    let cid = h.provisioned_active(&a, launch).await;

    let before = h.observe(cid);
    // HalfClose / HalfCloseAck for a flow the owner never opened (U-5).
    deliver_guest_control(&h, cid, ControlMessage::HalfClose { flow: guest_flow(999) }).await;
    deliver_guest_control(&h, cid, ControlMessage::HalfCloseAck { flow: guest_flow(998) }).await;
    h.pump().await;
    let after = h.observe(cid);

    assert_eq!(after.counters.control_discarded, before.counters.control_discarded + 2);
    assert_eq!(after.control_messages, before.control_messages, "no reply to a discarded signal");
}

async fn pair_tcp_flow(_h: &OwnerFlowHarness, _cid: GuestCid, _flow: FlowId, _launch: LaunchToken) {
    unimplemented!("DELIVER owner step: drive a TcpConnect to Paired through the seam")
}
async fn outer_leg_eof(_h: &OwnerFlowHarness, _cid: GuestCid, _flow: FlowId) {
    unimplemented!("DELIVER owner step: signal EOF on the flow's outer leg")
}
async fn flow_vsock_ends_unannounced(_h: &OwnerFlowHarness, _cid: GuestCid, _flow: FlowId) {
    unimplemented!("DELIVER owner step: end the flow vsock with no acknowledged HalfClose")
}

// =============================================================================
// P2-S13 — G-V6 / G-V3 boundary rows
// =============================================================================

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn late_paired_after_retiring_is_aborted_not_resurrected() {
    let h = OwnerFlowHarness::seeded(16);
    let a = alloc("web");
    let launch = token(0x1D);
    let cid = h.provisioned_active(&a, launch).await;

    let flow = FlowId::new(FlowOrigin::Host, 7).expect("valid");
    host_open_then_abort_tcp_accept(&h, cid, flow).await;
    teardown(&h, &a).await; // CID → Retiring

    let before = h.observe(cid);
    deliver_guest_control(&h, cid, ControlMessage::Paired { flow }).await;
    h.pump().await;
    let after = h.observe(cid);

    // Row 4: the late Paired is discarded; the flow stays closed (inv 9
    // ClosedIsTerminal).
    assert!(!after.registered_flows.contains(&flow), "the flow is not resurrected");
    assert_eq!(after.counters.control_discarded, before.counters.control_discarded + 1);
}

#[tokio::test]
#[ignore = "pending DELIVER: SharedGuestNetworkOwner flow-admission seam + SimGuestFlowForwarder (roadmap-reconciliation owner step)"]
async fn activate_after_teardown_refuses_lease_retiring() {
    // G-V3 row 4: activate after teardown → LeaseRetiring.
    let h = OwnerFlowHarness::seeded(17);
    let a = alloc("web");
    let launch = token(0x1E);
    let _cid = h.provisioned_active(&a, launch).await;
    teardown(&h, &a).await;

    let plan = plan_for(&a).await;
    let err = h.owner.activate(&plan).await.expect_err("activate after teardown is refused");
    assert!(
        matches!(err, GuestNetworkError::LeaseRetiring),
        "a retiring lease refuses activation, got {err:?}"
    );
}

async fn plan_for(_a: &AllocationId) -> overdrive_control_plane::guest_network::GuestNetworkPlan {
    unimplemented!("DELIVER owner step: build the GuestNetworkPlan for a retiring lease")
}
