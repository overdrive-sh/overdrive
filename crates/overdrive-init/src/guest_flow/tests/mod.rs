//! Acceptance tests of the guest flow owner (`GuestFlowOwner`) over the
//! guest-owner seam sim (`SimGuestOwnerIo` + `SimGuestClock`, the guest
//! forwarder context a `SimFlowForwarder` of `overdrive-sim`), source-local
//! because `overdrive-init` has no library target.
//!
//! Driving port: `GuestFlowOwner::dispatch_next` (DG-5), dispatched under the
//! DG-5-Q quiescence rule (`test_support::settle`). Oracles read the owner's
//! observation (QPROJ-5 `guest_observation`), the sim's QPROJ-7 surface
//! (`guest_socket_observation`), the forwarder arbiter (QPROJ-8
//! `forwarder_pair_state` / `forwarder_inventory`), the far sides of the
//! pair's halves (the `SimSocket`s' `peer_*` read-back: what the guest
//! application or the host received, and the first terminal event each saw)
//! and — for ordering obligations and the host's consumption of the guest's
//! messages only — the sim's test-support effect log and outbox.
//!
//! Scenario sources: feature-delta § *Per-kind total orders*, § *Control
//! session*, § *Guest adaptation contract* (TcpAccept guest side, INB-1, the
//! intake model's slot release), § *[REF] vsock data path* ("Sides and pairs",
//! "Owners' use of the forwarder", "Failure projection", "Owner response to an
//! ended half at attach", EVMOD-PAIR, EVMOD-REC, EVMOD-DGRAM,
//! EVMOD-ATTACH-END) and DG-5 (failure classes, DG-5-A, DG-5-D, DG-5-E,
//! DG-5-F). The guest kernel's own behaviour (a reserved intake port refusing a
//! real application bind, R5-15; the module and setup on the real guest
//! kernel) is `transport::real_kernel`; the real-kernel accept-path
//! equivalence (R5-59 (b)) is `guest_flow::real_kernel`.
//!
//! Every owner body is `pending DELIVER step 5` (the owner, the
//! `SocketCookie` codec) — and the guest forwarder sim it runs over is DELIVER
//! step 4's (`SimFlowForwarder`), which step 5 follows.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::missing_panics_doc,
    clippy::items_after_statements,
    clippy::too_many_lines
)]

mod flow_end;
mod newtypes;
mod session;
mod tcp_accept;
mod tcp_connect;
mod udp_slots;

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::guest_transport::LaunchToken;
use overdrive_core::guest_transport::forwarder::ForwardId;
use overdrive_core::guest_transport::wire::{
    ControlMessage, FlowId, FlowOrigin, FlowRequest, KernelErrno,
};
use overdrive_core::net::ListenPort;
use overdrive_core::vm::config::VsockPort;
use overdrive_sim::adapters::SimSocket;

use super::sim::{SimConnectTarget, SimGuestClock, SimGuestEffect, SimGuestOwnerIo, SimHalf};
use super::{
    GuestAcceptPhase, GuestClock, GuestFlowOwner, GuestSideObservation, HostChannel, SocketCookie,
    test_support,
};
use crate::transport::{GuestHostPorts, GuestTransportToken};

pub(super) const WL: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 7);
/// The guest gateway: every TcpAccept request's `peer` (ADR-0174).
pub(super) const GW: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
pub(super) const LAUNCH_HEX: &str = "0123456789abcdef0123456789abcdef";
pub(super) const OTHER_LAUNCH_HEX: &str = "fedcba9876543210fedcba9876543210";

pub(super) fn launch() -> LaunchToken {
    LaunchToken::from_str(LAUNCH_HEX).expect("fixture launch token")
}

pub(super) fn other_launch() -> LaunchToken {
    LaunchToken::from_str(OTHER_LAUNCH_HEX).expect("fixture launch token")
}

pub(super) fn listen_port(port: u16) -> ListenPort {
    ListenPort::from_str(&port.to_string()).expect("non-zero listen port")
}

/// A Linux errno as the kernel reports it.
pub(super) fn errno(raw: i32) -> KernelErrno {
    KernelErrno::new(u16::try_from(raw).expect("errno fits u16")).expect("non-zero errno")
}

/// A host-origin flow id (TcpAccept).
pub(super) fn host_flow(seq: u32) -> FlowId {
    FlowId::new(FlowOrigin::Host, seq).expect("seq < 2^31")
}

/// A guest-origin flow id the guest never opened (U-5 discards).
pub(super) fn unknown_guest_flow() -> FlowId {
    FlowId::new(FlowOrigin::Guest, 0x7fff_fff0).expect("seq < 2^31")
}

/// The TcpAccept request a dial writes for `flow` to `port`: `peer` = the
/// guest gateway (port 0 on the wire), carrying `launch`.
pub(super) fn accept_request(flow: FlowId, port: u16, launch: LaunchToken) -> FlowRequest {
    FlowRequest::TcpAccept { flow, peer: GW, listen_port: listen_port(port), launch }
}

/// The address C is bound to for a TcpAccept: the request's peer with port 0.
pub(super) const fn accept_client() -> SocketAddrV4 {
    SocketAddrV4::new(GW, 0)
}

/// A UDP slot address.
pub(super) fn slot(n: u16) -> SocketAddrV4 {
    SocketAddrV4::new(WL, overdrive_core::guest_transport::GUEST_UDP_SLOT_BASE + n)
}

/// A distinct external destination per `n`.
pub(super) fn dest(n: u16) -> SocketAddrV4 {
    let [hi, lo] = n.to_be_bytes();
    SocketAddrV4::new(Ipv4Addr::new(93, hi, lo, 1), 53)
}

/// A test cookie (the owner's `SocketCookie::new` is a DELIVER scaffold).
pub(super) fn cookie(raw: u64) -> SocketCookie {
    super::sim::sim_cookie(raw)
}

/// One guest: the owner under test, the driver's clone of its seam, and the
/// shared clock.
pub(super) struct Guest {
    pub(super) owner: GuestFlowOwner,
    pub(super) io: SimGuestOwnerIo,
    pub(super) clock: Arc<SimGuestClock>,
}

/// GUEST-PORTS (ADR-0217): the host listener ports the token carries; the
/// sim seam addresses host channels by kind, so the values are stimulus.
fn host_ports() -> GuestHostPorts {
    GuestHostPorts {
        beacon: VsockPort::new(50_234),
        stream: VsockPort::new(50_240),
        control: VsockPort::new(50_243),
    }
}

impl Guest {
    /// A constructed owner (nothing dispatched yet).
    pub(super) fn new() -> Self {
        let clock = Arc::new(SimGuestClock::new());
        let io = SimGuestOwnerIo::new(WL, Arc::clone(&clock));
        let token = GuestTransportToken {
            workload_addr: WL,
            gateway: GW,
            launch: launch(),
            ports: host_ports(),
        };
        let owner_clock: Arc<dyn GuestClock> = clock.clone();
        let owner = GuestFlowOwner::new(token, Box::new(io.clone()), owner_clock);
        Self { owner, io, clock }
    }

    pub(super) fn obs(&self) -> GuestSideObservation {
        self.owner.guest_observation()
    }

    /// The owner's first dispatch (it starts the control-session connect).
    pub(super) fn start(&mut self) {
        test_support::start(&mut self.owner);
    }

    /// Exactly one owner step; a per-flow or session failure is never fatal.
    pub(super) fn step(&mut self) {
        self.owner.dispatch_next().expect("no GuestOwnerFatal on this path");
    }

    /// Dispatches every released event and every due deadline (DG-5-Q).
    pub(super) fn settle(&mut self) {
        test_support::settle(&mut self.owner, &self.io, &self.clock);
    }

    /// Runs the guest forwarder's queued work (the kernel's work items), then
    /// releases every end record the guest context holds and settles, until
    /// nothing is left (the context's readiness, EVMOD-REC).
    pub(super) fn run_and_settle(&mut self) {
        loop {
            self.io.run_forwarder();
            let released = self.io.release_forward_ends();
            self.settle();
            if released.is_empty() && self.io.forwarder_pending_runs().is_empty() {
                return;
            }
        }
    }

    /// Advances the clock by `by`, then settles.
    pub(super) fn advance(&mut self, by: Duration) {
        self.clock.advance(by);
        self.settle();
    }

    /// Advances the clock to exactly `at` (DG-5-A), then settles.
    pub(super) fn advance_to(&mut self, at: Instant) {
        test_support::advance_to(&mut self.owner, &self.io, &self.clock, at);
    }

    /// The current instant on the owner's clock.
    pub(super) fn now(&self) -> Instant {
        self.clock.now()
    }

    /// The first dispatch starts the control connect; the host accepts it.
    pub(super) fn open_session(&mut self) {
        self.start();
        self.settle();
        self.io.complete_host_connect(HostChannel::Control, Ok(()));
        self.settle();
        assert!(self.obs().session, "the control session is held");
    }

    /// The guest accepts `flow`'s V_g and reads its request: C is connecting.
    pub(super) fn accept_until_connecting(&mut self, flow: FlowId, port: u16) {
        self.io.host_open_accept(accept_request(flow, port, launch()));
        self.io.release_inbound_accept(flow);
        self.settle();
        self.io.release_inbound_request(flow);
        self.settle();
    }

    /// Host-opened TcpAccept `flow` to `port` up to `Paired` (attached, and
    /// `Paired` written). Returns the pair.
    pub(super) fn accept_flow(&mut self, flow: FlowId, port: u16) -> ForwardId {
        self.accept_until_connecting(flow, port);
        self.io.complete_app_connect(flow, Ok(()));
        self.settle();
        match self.obs().accepted.get(&flow) {
            Some(GuestAcceptPhase::Paired { pair }) => *pair,
            other => panic!("expected {flow:?} Paired, got {other:?}"),
        }
    }

    /// The application connects to `destination`; the guest opens V_g and
    /// writes its request. Returns the guest-origin flow id it claimed.
    pub(super) fn capture_flow(
        &mut self,
        cookie: SocketCookie,
        destination: SocketAddrV4,
    ) -> FlowId {
        let _child = self.io.app_connect(cookie, destination);
        self.settle();
        self.io.complete_host_connect(HostChannel::FlowStream, Ok(()));
        self.settle();
        self.last_request_flow()
    }

    /// `capture_flow` then the host's `Paired` (the guest attaches the pair).
    pub(super) fn connect_flow(
        &mut self,
        cookie: SocketCookie,
        destination: SocketAddrV4,
    ) -> FlowId {
        let flow = self.capture_flow(cookie, destination);
        self.io.host_control(ControlMessage::Paired { flow });
        self.settle();
        assert!(self.obs().flows.contains_key(&flow), "paired");
        flow
    }

    /// The pair the guest forwarder accepted for `flow`.
    pub(super) fn pair(&self, flow: FlowId) -> ForwardId {
        self.io.pair_of(flow).expect("the flow was attached")
    }

    /// The kernel socket of `flow`'s V_g (the host is its far peer).
    pub(super) fn vsock(&self, flow: FlowId) -> SimSocket {
        self.io.flow_socket(flow, SimHalf::Vsock).expect("the flow has a V_g")
    }

    /// The kernel socket of `flow`'s outer leg I / C (the application is its
    /// far peer).
    pub(super) fn outer(&self, flow: FlowId) -> SimSocket {
        self.io.flow_socket(flow, SimHalf::Outer).expect("the flow has an outer leg")
    }

    /// The flow of the last TcpConnect request the guest wrote.
    pub(super) fn last_request_flow(&self) -> FlowId {
        self.io
            .requests()
            .iter()
            .rev()
            .find_map(|r| match r {
                FlowRequest::TcpConnect { flow, .. } => Some(*flow),
                _ => None,
            })
            .expect("the guest wrote a TcpConnect request")
    }

    /// Starts slot `s`'s association from its readiness event, V_g connected
    /// and its request written; returns the association's flow id.
    pub(super) fn associate(&mut self, s: SocketAddrV4) -> FlowId {
        self.io.release_slot_ready(s);
        self.settle();
        self.io.complete_host_connect(HostChannel::FlowStream, Ok(()));
        self.settle();
        let flow = self.io.slot_flow(s).expect("the association's request is written");
        assert_eq!(self.obs().slots[&s].association, Some(flow));
        flow
    }

    /// The host pairs slot `s`'s association (the guest attaches it).
    pub(super) fn pair_slot(&mut self, flow: FlowId) {
        self.io.host_control(ControlMessage::Paired { flow });
        self.settle();
    }

    /// Number of in-flight connects to `channel`.
    pub(super) fn connects(&self, channel: HostChannel) -> usize {
        self.io
            .pending_connects()
            .iter()
            .filter(|(_, t)| *t == SimConnectTarget::Host(channel))
            .count()
    }
}

/// Index of the first effect matching `pred`.
pub(super) fn position(
    effects: &[SimGuestEffect],
    pred: impl Fn(&SimGuestEffect) -> bool,
) -> Option<usize> {
    effects.iter().position(pred)
}

/// The set of slots owned in an observation.
pub(super) fn owned_slots(obs: &GuestSideObservation) -> BTreeSet<SocketAddrV4> {
    obs.slots.iter().filter(|(_, s)| s.owner.is_some()).map(|(a, _)| *a).collect()
}
