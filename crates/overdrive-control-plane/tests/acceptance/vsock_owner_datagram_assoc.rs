//! P3 owner-side datagram-association behaviour, driven through the guest-flow
//! owner's seeded-sim private seam (`.claude/rules/testing.md` §"Tier 1"; no
//! test hand-installs a production effect). Covers P3-S6 (owner refusal of a
//! datagram association), P3-S9a (U-5 discard of a control message for an
//! unknown/closed flow — the owner side of SLOT-ABORT), P3-S9b (D4: the host
//! never writes on the SEQPACKET leg), P3-S9c (D10: DNS as a datagram
//! association to the node DnsResponder), and the G-V6 boundary row 6
//! (`transport: None`). Spec: `test-scenarios-p3-udp-slots-framing.md`;
//! `feature-delta.md` §"Per-kind total orders — Datagram association" (L18310),
//! §"Owner and provisioner" (L19062), §"Wire contracts — control message"
//! (L18086). Layer: seeded-sim acceptance, default lane; example-based (Mandate
//! 11 — sad paths enumerated, never PBT at this layer).
//!
//! GATED (shared-infra §0): the replacement owner surface
//! (`overdrive_core::guest_transport::wire::{FlowRequest, ControlMessage,
//! FlowRefusal}`, the reshaped `SimSharedGuestNetworkOwner`) does not exist yet.
//! TESTABILITY GAP G-3: the owner's flow acceptor is internal (D7); this file
//! drives it through the shared seeded-sim seam — `SimGuestFlowForwarder`
//! (shared-infra §3, shared, first of P2/P3/P6 creates — NOT edited here) and
//! the owner's private flow-request-delivery seam the sim-counterparts section
//! describes (L20043). The `DatagramAssocHarness` below is P3-owned test support
//! (DISTILL's to shape, CLAUDE.md) over those shared pieces; the crafter
//! reconciles the seam's exact shape at activation. NOT registered until the
//! `mod vsock_owner_datagram_assoc;` line + production scaffold co-arrive.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::{Ipv4Addr, SocketAddrV4};

use overdrive_core::LaunchToken;
use overdrive_core::guest_transport::wire::{
    ControlMessage, FlowId, FlowOrigin, FlowRefusal, FlowRequest,
};

// P3-owned seeded-sim harness; built over the shared owner seam (G-3).
use overdrive_control_plane::guest_network::test_support::DatagramAssocHarness;

const GATEWAY: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);

fn launch() -> LaunchToken {
    LaunchToken::from_bytes([0x11; 16]).expect("non-zero token")
}

fn datagram_pair(flow_seq: u32, dst: SocketAddrV4) -> (FlowRequest, FlowRequest) {
    let flow = FlowId::new(FlowOrigin::Guest, flow_seq).expect("seq < 2^31");
    (
        FlowRequest::DatagramStream { flow, destination: dst, launch: launch() },
        FlowRequest::DatagramSeqpacket { flow, destination: dst, launch: launch() },
    )
}

// ---------------------------------------------------------------- P3-S6 (owner)
/// A datagram association whose destination the owner refuses `Policy` /
/// `HostInternal` emits exactly one `Refused(code)` on the control session and
/// installs no forwarder entry; unrelated associations stay paired.
#[tokio::test]
#[ignore = "pending DELIVER: owner datagram refusal (U-2 owner side, R5-27); testability gap G-3"]
async fn owner_refuses_a_policy_blocked_datagram_association() {
    let mut h = DatagramAssocHarness::new().await;
    let cid = h.provision_and_activate_with_live_session(launch()).await;

    // A live association to an allowed destination pairs.
    let allowed = SocketAddrV4::new(Ipv4Addr::new(93, 184, 216, 34), 443);
    let (s0, q0) = datagram_pair(1, allowed);
    h.deliver_flow_request(cid, s0).await;
    h.deliver_flow_request(cid, q0).await;
    assert_eq!(
        h.emitted_control(cid),
        vec![ControlMessage::Paired { flow: FlowId::new(FlowOrigin::Guest, 1).unwrap() }],
    );
    assert_eq!(h.forwarder_install_count(cid), 1);

    // A host-internal destination (D26) is refused; no install.
    let internal = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 9000);
    let (s1, q1) = datagram_pair(2, internal);
    h.deliver_flow_request(cid, s1).await;
    h.deliver_flow_request(cid, q1).await;
    assert_eq!(
        h.emitted_control(cid),
        vec![
            ControlMessage::Paired { flow: FlowId::new(FlowOrigin::Guest, 1).unwrap() },
            ControlMessage::Refused {
                flow: FlowId::new(FlowOrigin::Guest, 2).unwrap(),
                refusal: FlowRefusal::HostInternal,
            },
        ],
    );
    assert_eq!(h.forwarder_install_count(cid), 1, "the refused association installs nothing");
}

// ---------------------------------------------------------------- P3-S9a (U-5)
/// U-5: a control message for an unknown or closed flow id is discarded and
/// counted (`guest_flow.control_discarded`), with no reply — the owner side of
/// SLOT-ABORT (the guest-side `LatePairedAborted` is P3-S8 / the guest file).
#[tokio::test]
#[ignore = "pending DELIVER: owner U-5 discard of unknown/closed flow control (U-5); gap G-3"]
async fn owner_discards_control_for_unknown_or_closed_flow() {
    let mut h = DatagramAssocHarness::new().await;
    let cid = h.provision_and_activate_with_live_session(launch()).await;

    let unknown = FlowId::new(FlowOrigin::Guest, 999).unwrap();
    let before = h.control_discarded_count();
    h.deliver_control(cid, ControlMessage::Abort { flow: unknown }).await;
    assert_eq!(h.control_discarded_count(), before + 1, "discarded and counted");
    assert!(h.emitted_control(cid).is_empty(), "no reply to an unknown flow");
}

// ---------------------------------------------------------------- P3-S9b (D4)
/// D4: a paired datagram association installs the STREAM target `T_h` (host→guest,
/// framed) and the SEQPACKET-fed `Q_h`, and the host never writes on the
/// SEQPACKET leg. Observable through the forwarder install record (routes
/// `H → T_h` framed, `Q_h → H` strip) and the owner holding every pair socket as
/// a write-free `PairedSocket` (no host write path on Q).
#[tokio::test]
#[ignore = "pending DELIVER: owner datagram install direction (D4); gap G-3"]
async fn host_never_writes_on_the_seqpacket_leg() {
    let mut h = DatagramAssocHarness::new().await;
    let cid = h.provision_and_activate_with_live_session(launch()).await;
    let dst = SocketAddrV4::new(Ipv4Addr::new(93, 184, 216, 34), 443);
    let (s, q) = datagram_pair(1, dst);
    h.deliver_flow_request(cid, s).await;
    h.deliver_flow_request(cid, q).await;

    let install = h.last_forwarder_install(cid).expect("a paired association installed");
    // Direction (feature-delta L18334): H → T_h framed; Q_h → H strip.
    assert!(install.routes_host_stream_to_guest(), "STREAM toward the guest");
    assert!(install.routes_guest_seqpacket_to_host(), "SEQPACKET from the guest");
    // D4: the host never writes on the SEQPACKET leg — the owner holds no write
    // path on Q_h (every pair socket is a write-free PairedSocket).
    assert_eq!(install.host_writes_on_seqpacket_leg(), 0);
}

// ---------------------------------------------------------------- P3-S9c (D10)
/// D10: guest DNS keeps the gateway as its nameserver and travels as an ordinary
/// datagram association to the node `DnsResponder` at `gateway:53`; that is the
/// one guest-prefix UDP destination not refused `Policy` (#310); any other
/// guest-prefix UDP destination is refused `Policy`.
#[tokio::test]
#[ignore = "pending DELIVER: owner DNS datagram association (D10); gap G-3"]
async fn guest_dns_is_a_datagram_association_to_the_gateway_responder() {
    let mut h = DatagramAssocHarness::new().await;
    let cid = h.provision_and_activate_with_live_session(launch()).await;

    // gateway:53 pairs (resolves to the node DnsResponder).
    let dns = SocketAddrV4::new(GATEWAY, 53);
    let (s0, q0) = datagram_pair(1, dns);
    h.deliver_flow_request(cid, s0).await;
    h.deliver_flow_request(cid, q0).await;
    assert_eq!(
        h.emitted_control(cid),
        vec![ControlMessage::Paired { flow: FlowId::new(FlowOrigin::Guest, 1).unwrap() }],
    );
    assert!(h.last_forwarder_install(cid).unwrap().routes_to_dns_responder());

    // Any other guest-prefix UDP destination is refused Policy (#310).
    let other = SocketAddrV4::new(GATEWAY, 8053);
    let (s1, q1) = datagram_pair(2, other);
    h.deliver_flow_request(cid, s1).await;
    h.deliver_flow_request(cid, q1).await;
    assert_eq!(
        *h.emitted_control(cid).last().unwrap(),
        ControlMessage::Refused {
            flow: FlowId::new(FlowOrigin::Guest, 2).unwrap(),
            refusal: FlowRefusal::Policy,
        },
    );
}

// ---------------------------------------------------------------- boundary row 6
/// G-V6 row 6 (feature disabled): a sim-driver fixture with `transport: None`
/// composes no guest-flow owner and no slot pool, behaving as today's
/// `network: None` — no datagram association is ever accepted.
#[tokio::test]
#[ignore = "pending DELIVER: transport:None disabled path (G-V6 row 6)"]
async fn transport_none_composes_no_owner_and_accepts_no_association() {
    let h = DatagramAssocHarness::new_transport_none().await;
    assert!(!h.owner_composed(), "no guest-flow owner is composed");
    assert_eq!(h.slot_pool_size(), 0, "no slot pool");
}
