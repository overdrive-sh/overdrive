//! R5-54 (seeded-sim half) — LEGS-DIAL (D-14, approved by the user
//! 2026-10-19; amends D-UP-5): the inbound enforcement, not the mTLS worker,
//! dials leg S to a VM backend through the boot's one `OwnerDial`, and only
//! after the client authenticated.
//!
//! Design: `feature-delta.md` § *leg-S over vsock (temporary bridge until
//! #303)*, "Who dials leg S and relays it (D-14)"; § *Owner and mTLS-worker
//! composition* (`compose_guest_network(ports, io, dial)`, `OwnerDial`);
//! obligation R5-54.
//!
//! **Composition.** The production owner and worker built by
//! `compose_guest_network(ports, io, dial)` over `SimGuestFlowIo`, where
//! `ports.enforcement` is a `SimMtlsEnforcement` constructed with the same
//! unbound `OwnerDial` as its `guest_dial` — the construction `run_server`
//! performs at boot step 5 — so the enforcement reaches the composed owner
//! only through the binding `compose_guest_network` makes. The body drives
//! the enforcement's driving port (`MtlsEnforcement::enforce` with
//! `Routed::Inbound { orig_dst }`, then `teardown`) as the worker's leg-C
//! accept would, and plays the guest at the seam (the `Paired` reply on the
//! control session).
//!
//! **What the sim enforcement models.** No TLS (`SimMtlsEnforcement`):
//! the handshake outcome is a function of the held identity and of the
//! script. A client the peer verification refuses is a bad client against a
//! node that *does* hold a trust bundle: the sim's scripted
//! `ScriptedTrip::PeerVerification` refuses the next `enforce` of the
//! allocation `PeerVerificationFailed`, as the host adapter refuses a client
//! certificate that does not chain (a node with no bundle, `AbsentBundle`,
//! is a different state and not this oracle). The relay's end table is
//! modelled on leg C's real loopback socket: the client's half-close is the
//! guest's `0x2`, the client's reset (`SO_LINGER{1,0}`) is the guest's
//! `0x3`. TLS framing and the 0.5-RTT-first order are the real
//! `HostMtlsEnforcement`'s, in R5-54's Tier-3 half
//! (`overdrive-dataplane` `tests/integration/mtls_inbound_guest_relay.rs`).
//!
//! Oracles (port-observable): `enforce`'s result, the owner's
//! `flow_observation()` (`dials` with their `DialPurpose`, `held_flows`),
//! the seam's dial records (`SimGuestFlowIo::dials`, the QPROJ-7 arbiter),
//! the vsock shutdowns the guest received, and the far end of leg C.
//! Layer 3, example-based (Mandate 11). Pending the attachment-cut step (9)
//! (DG-2 row 19: leg S's VM-backend dial).

use std::collections::BTreeMap;
use std::io::Read as _;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::guest_transport::dial::{DialPurpose, GuestDialError, GuestStreamDial};
use overdrive_core::guest_transport::wire::{ControlMessage, FlowId};
use overdrive_core::guest_transport::{GUEST_NODE_GATEWAY, GUEST_NODE_PREFIX, VsockListenPort};
use overdrive_core::id::AllocationId;
use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::mtls_enforcement::{
    EnforcedConnection, InterceptedConnection, MtlsEnforcement, MtlsEnforcementError, MtlsLimits,
    Routed,
};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{CertSerial, SpiffeId, WorkloadId};
use overdrive_dataplane::allocators::VipRange;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::mtls_enforcement::{ScriptedTrip, SimMtlsEnforcement};
use overdrive_sim::adapters::{SimIdentityRead, SimMtlsIntercept, SimMtlsResolve};
use tokio::task::JoinHandle;

use super::support::{Composition, Lease, vip_range};
use crate::guest_network::flow_io::GuestFlowIo;
use crate::guest_network::flow_io::sim::{SimEffectKind, SimGuestFlowIo, VsockShutdownOp};
use crate::guest_network::{DialObservation, GuestAddressPool, GuestConnKey};
use crate::guest_network_composition::OwnerDial;

/// The VM backend's declared service port.
const PORT: u16 = 8443;
/// The server allocation the inbound connection is for (its SVID is held).
const SERVER: &str = "legs-server";

/// A placeholder held SVID for `alloc` (the sim performs no handshake; the
/// held set's presence is its outcome).
fn held_svid(alloc: &AllocationId) -> SvidMaterial {
    let workload = WorkloadId::new("legs-dial").expect("valid workload id");
    SvidMaterial::new(
        CaCertPem::new("-----BEGIN CERTIFICATE-----\nLEGS\n-----END CERTIFICATE-----\n".into()),
        CaCertDer::new(vec![0x30, 0x00]),
        CertSerial::new("0a0b0c0d").expect("valid serial"),
        SpiffeId::for_allocation(&workload, alloc),
        CaKeyPem::new("-----BEGIN PRIVATE KEY-----\nLEGS\n-----END PRIVATE KEY-----\n".into()),
        UnixInstant::from_unix_duration(Duration::from_secs(4_102_444_800)),
    )
}

/// The node holds a trust anchor in every cell; whether a given client
/// authenticates is the client's (scripted) property.
#[derive(Debug, Clone, Copy)]
enum Verification {
    /// A trust anchor is held: the client authenticates.
    Anchored,
}

/// The production composition of boot step 5 under LEGS-DIAL: one unbound
/// `OwnerDial`, given to the enforcement as its `guest_dial` and bound by
/// `compose_guest_network`; and the enforcement handle the body drives.
fn compose(seed: u64, verification: Verification) -> (Composition, Arc<SimMtlsEnforcement>) {
    eprintln!("legs-dial composition seed = {seed}");
    let io = SimGuestFlowIo::new(seed);
    io.auto_release(SimEffectKind::ProbeForwarder);
    let clock = SimClock::new();
    let resolve = Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh));
    let server = AllocationId::new(SERVER).expect("valid allocation id");
    let Verification::Anchored = verification;
    let bundle = Some(TrustBundle::new(
        CaCertPem::new("-----BEGIN CERTIFICATE-----\nROOT\n-----END CERTIFICATE-----\n".into()),
        None,
    ));
    let identity = Arc::new(SimIdentityRead::new(
        BTreeMap::from([(server.clone(), held_svid(&server))]),
        bundle,
    ));
    let dial = Arc::new(OwnerDial::unbound());
    let enforcement = Arc::new(SimMtlsEnforcement::new(
        identity,
        MtlsLimits::default(),
        Arc::clone(&dial) as Arc<dyn GuestStreamDial>,
    ));
    let ports = crate::GuestNetworkCompositionPorts {
        clock: Arc::new(clock.clone()) as Arc<dyn Clock>,
        resolve: Arc::clone(&resolve) as Arc<dyn MtlsResolve>,
        enforcement: Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
        intercept: Arc::new(SimMtlsIntercept::new()),
        vip_range: VipRange::new(vec![vip_range()], std::collections::BTreeSet::new())
            .expect("valid VIP range"),
        node_prefix: GUEST_NODE_PREFIX,
        gateway: GUEST_NODE_GATEWAY,
        stream_port: VsockListenPort::KernelAssigned,
        control_port: VsockListenPort::KernelAssigned,
    };
    let (owner, mtls) = crate::compose_guest_network(
        ports,
        Arc::clone(&io) as Arc<dyn GuestFlowIo>,
        Arc::clone(&dial),
    );
    let bound = dial.owner().expect("compose_guest_network binds the boot's one OwnerDial");
    assert!(Arc::ptr_eq(&bound, &owner), "the enforcement's dial reaches the composed owner");
    let pool = Arc::new(GuestAddressPool::new(
        GUEST_NODE_PREFIX,
        GUEST_NODE_GATEWAY,
        Arc::new(SimEntropy::new(seed)),
    ));
    (Composition { seed, io, owner, pool, clock, resolve, mtls, dial }, enforcement)
}

/// A booted composition with the server's lease Active (control session
/// live), declaring `PORT`.
async fn serving(
    seed: u64,
    verification: Verification,
) -> (Composition, Arc<SimMtlsEnforcement>, Lease, GuestConnKey) {
    let (c, enforcement) = compose(seed, verification);
    c.boot().await;
    let (lease, session) = c.active_lease(SERVER, &[PORT]).await;
    (c, enforcement, lease, session)
}

/// Leg C as the worker's accept hands it over: one end of a connected pair;
/// the other end plays the TLS client's socket.
fn leg_c() -> (OwnedFd, UnixStream) {
    let (leg, client) = UnixStream::pair().expect("a connected pair for leg C");
    (OwnedFd::from(leg), client)
}

/// Leg C over a real loopback TCP connection, so the client can end it
/// either way the end table distinguishes: a half-close (FIN) or a reset
/// (`SO_LINGER{1,0}` then close, an RST).
fn tcp_leg_c() -> (OwnedFd, std::net::TcpStream) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let client = std::net::TcpStream::connect(listener.local_addr().expect("addr"))
        .expect("connect loopback");
    let (leg, _) = listener.accept().expect("accept leg C");
    (OwnedFd::from(leg), client)
}

/// The TLS client resets leg C: `SO_LINGER{1,0}` then close (an RST).
fn reset(client: std::net::TcpStream) {
    let linger = libc::linger { l_onoff: 1, l_linger: 0 };
    nix::sys::socket::setsockopt(&client, nix::sys::socket::sockopt::Linger, &linger)
        .expect("SO_LINGER{1,0} on the client's open socket");
    drop(client);
}

/// `enforce` of an inbound connection to `orig_dst` for the server, on a
/// task (it awaits the leg-S dial).
fn spawn_inbound(
    enforcement: &Arc<SimMtlsEnforcement>,
    leg: OwnedFd,
    orig_dst: SocketAddrV4,
) -> JoinHandle<Result<EnforcedConnection, MtlsEnforcementError>> {
    let enforcement = Arc::clone(enforcement);
    let conn = InterceptedConnection {
        leg,
        routed: Routed::Inbound { orig_dst },
        alloc: AllocationId::new(SERVER).expect("valid allocation id"),
        expected_peer: None,
    };
    tokio::spawn(async move { enforcement.enforce(conn).await })
}

fn relay_dials(observed: &crate::guest_network::GuestOwnerObservation, lease: &Lease) -> usize {
    observed.cids.get(&lease.cid).map_or(0, |cid| {
        cid.dials
            .values()
            .filter(|dial| matches!(dial, DialObservation { purpose: DialPurpose::Relay, .. }))
            .count()
    })
}

/// R5-54 / LEGS-DIAL — an authenticated client's connection to a VM
/// backend: the enforcement dials leg S through the owner (one
/// `DialPurpose::Relay` dial, counted in `held_flows`), `enforce` returns
/// once the guest pairs it, and the connection's teardown ends the relay:
/// the `GuestStream`'s drop delivers `0x3` and releases the flow's quota
/// place (`held_flows` back to its prior value, no watch left).
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_authenticated_client_dials_leg_s_once_and_its_teardown_releases_the_flow() {
    let (c, enforcement, lease, session) = serving(54_001, Verification::Anchored).await;
    let prior = c.observe().await.cids[&lease.cid].held_flows;
    assert!(c.io.dials().is_empty(), "precondition: no dial before the connection");
    let (leg, _client) = leg_c();

    let enforced = spawn_inbound(&enforcement, leg, lease.target(PORT));
    c.settle().await;
    assert_eq!(c.io.dials().len(), 1, "the enforcement dialled leg S once, after authentication");
    let flow = c.dial_flow(0);
    c.guest_sends(session, ControlMessage::Paired { flow });
    c.settle().await;
    let handle = enforced
        .await
        .expect("the enforce task does not panic")
        .expect("an authenticated client to a served VM port is established over leg S");

    let paired = c.observe().await;
    assert_eq!(relay_dials(&paired, &lease), 1, "one Relay dial held by the relay's stream");
    assert_eq!(paired.cids[&lease.cid].held_flows, prior + 1, "the relay counts against the quota");

    enforcement.teardown(handle).await.expect("teardown of an established connection");
    let ended = c.observe().await;
    assert_eq!(relay_dials(&ended, &lease), 0, "the relay's stream is gone");
    assert_eq!(ended.cids[&lease.cid].held_flows, prior, "the flow's quota place is released");
    assert_eq!(
        c.io.sent_to_guest(flow).last(),
        Some(&VsockShutdownOp::Close),
        "the stream's drop closed V_h (0x3 toward the guest)"
    );
    assert_eq!(c.io.forwarder_inventory().live_watches, 0, "no watch outlives the stream");
}

/// R5-54 / LEGS-DIAL — a client the peer verification refuses (a bad
/// client certificate against the node's held bundle) causes no leg-S dial:
/// `enforce` fails closed `PeerVerificationFailed` and no
/// `DialPurpose::Relay` entry ever appears; the seam records no dial and the
/// quota is untouched.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_client_refused_by_peer_verification_causes_no_leg_s_dial() {
    let (c, enforcement, lease, _session) = serving(54_002, Verification::Anchored).await;
    let prior = c.observe().await.cids[&lease.cid].held_flows;
    enforcement.script_trip(
        &AllocationId::new(SERVER).expect("valid allocation id"),
        ScriptedTrip::PeerVerification,
    );
    let (leg, _client) = leg_c();

    let refused = spawn_inbound(&enforcement, leg, lease.target(PORT));
    c.settle().await;
    let refused = refused.await.expect("the enforce task does not panic");
    assert!(
        matches!(refused, Err(MtlsEnforcementError::PeerVerificationFailed { .. })),
        "peer verification refuses the bad client, got {refused:?}"
    );
    let observed = c.observe().await;
    assert!(c.io.dials().is_empty(), "no leg-S dial was made");
    assert_eq!(relay_dials(&observed, &lease), 0, "no Relay entry ever appears");
    assert_eq!(observed.cids[&lease.cid].held_flows, prior, "the quota is untouched");
}

/// Establishes a relayed connection over a TCP leg C: the dial paired by
/// the guest. Returns the client's end of leg C and the dial's flow.
async fn relayed(
    c: &Composition,
    enforcement: &Arc<SimMtlsEnforcement>,
    lease: &Lease,
    session: GuestConnKey,
) -> (std::net::TcpStream, EnforcedConnection, FlowId) {
    let (leg, client) = tcp_leg_c();
    let enforced = spawn_inbound(enforcement, leg, lease.target(PORT));
    c.settle().await;
    let flow = c.dial_flow(0);
    c.guest_sends(session, ControlMessage::Paired { flow });
    c.settle().await;
    let handle = enforced
        .await
        .expect("the enforce task does not panic")
        .expect("an authenticated client to a served VM port is established");
    (client, handle, flow)
}

/// R5-54 end table — the TLS client's half-close of leg C (FIN) is the
/// guest's `0x2`: the relay half-closes leg S with `shutdown_write`, the
/// guest receives a SHUTDOWN with flags 0x2 and no 0x3, and the relay's
/// stream stays held (the guest's direction is still open).
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_client_half_close_of_leg_c_is_the_guests_0x2() {
    let (c, enforcement, lease, session) = serving(54_005, Verification::Anchored).await;
    let (client, _handle, flow) = relayed(&c, &enforcement, &lease, session).await;
    assert!(c.io.sent_to_guest(flow).is_empty(), "precondition: no shutdown sent yet");

    client.shutdown(std::net::Shutdown::Write).expect("the client half-closes leg C");
    for _ in 0..50 {
        c.settle().await;
        if !c.io.sent_to_guest(flow).is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        c.io.sent_to_guest(flow),
        vec![VsockShutdownOp::HalfClose],
        "the client's FIN reaches the guest as 0x2, and nothing else"
    );
    assert_eq!(relay_dials(&c.observe().await, &lease), 1, "the relay still holds its stream");
}

/// R5-54 end table — the TLS client resets leg C: the relay ends and drops
/// leg S with the host→guest direction open, so the guest receives 0x3 and
/// no 0x2, and the flow's quota place is released.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_client_reset_of_leg_c_is_the_guests_0x3_and_releases_the_flow() {
    let (c, enforcement, lease, session) = serving(54_006, Verification::Anchored).await;
    let prior = c.observe().await.cids[&lease.cid].held_flows;
    let (client, _handle, flow) = relayed(&c, &enforcement, &lease, session).await;

    reset(client);
    for _ in 0..50 {
        c.settle().await;
        if !c.io.sent_to_guest(flow).is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        c.io.sent_to_guest(flow),
        vec![VsockShutdownOp::Close],
        "a client reset reaches the guest as 0x3 (the stream's drop), never 0x2"
    );
    let ended = c.observe().await;
    assert_eq!(relay_dials(&ended, &lease), 0, "the relay's stream is gone");
    assert_eq!(ended.cids[&lease.cid].held_flows, prior, "the flow's quota place is released");
}

/// R5-54 / LEGS-DIAL — a leg-S dial error resets leg C and returns
/// `MtlsEnforcementError::GuestDial` naming the dial outcome: the target is
/// a VM whose lease is provisioned and not yet Active, which the owner
/// answers `NotServing`. Leg C is closed (its client end reads its end) and
/// nothing is held.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_leg_s_dial_error_closes_leg_c_and_returns_guest_dial() {
    let (c, enforcement) = compose(54_003, Verification::Anchored);
    c.boot().await;
    let lease = c.lease(SERVER, &[PORT]).await;
    let (leg, mut client) = leg_c();

    let failed = spawn_inbound(&enforcement, leg, lease.target(PORT));
    c.settle().await;
    let failed = failed.await.expect("the enforce task does not panic");
    assert!(
        matches!(
            failed,
            Err(MtlsEnforcementError::GuestDial { source: GuestDialError::NotServing })
        ),
        "a dial error returns GuestDial naming the outcome, got {failed:?}"
    );
    client.set_read_timeout(Some(Duration::from_secs(1))).expect("a bounded read");
    let mut buf = [0_u8; 1];
    let end = client.read(&mut buf);
    assert!(
        matches!(end, Ok(0)) || end.is_err_and(|e| e.kind() != std::io::ErrorKind::WouldBlock),
        "leg C is closed: its client end reads its end"
    );
    let observed = c.observe().await;
    assert_eq!(relay_dials(&observed, &lease), 0, "no Relay entry is held");
}

/// R5-54 / LEGS-DIAL — a target outside the guest prefix keeps today's TCP
/// leg S: the enforcement makes no dial through the owner.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_target_outside_the_guest_prefix_makes_no_owner_dial() {
    let (c, enforcement, _lease, _session) = serving(54_004, Verification::Anchored).await;
    let outside = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 5), PORT);
    assert!(!GUEST_NODE_PREFIX.contains(outside.ip()), "precondition: outside the prefix");
    let (leg, _client) = leg_c();

    let enforced = spawn_inbound(&enforcement, leg, outside);
    c.settle().await;
    let outcome = enforced.await.expect("the enforce task does not panic");
    eprintln!("outside-prefix enforce: {:?}", outcome.as_ref().err());
    assert!(c.io.dials().is_empty(), "no dial through the guest-flow owner");
}
