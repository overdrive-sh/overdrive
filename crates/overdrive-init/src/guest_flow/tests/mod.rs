//! Acceptance tests of the guest flow owner (`GuestFlowOwner`) over the
//! guest-owner seam sim (`SimGuestOwnerIo` + `SimGuestClock`), source-local
//! because `overdrive-init` has no library target.
//!
//! Re-authored from the orphaned draft at `91f9436ff`
//! (`tests/acceptance/vsock_guest_udp_slots.rs`), whose invented
//! `overdrive_init::slots::SlotTable` surface is replaced by the pinned DG-5
//! driving port `GuestFlowOwner::dispatch_next` and the QPROJ-5 accessor
//! `guest_observation`. Oracles read the owner's observation (QPROJ-5), the
//! sim's QPROJ-7 surface, and — for ordering obligations only — the sim's
//! test-support effect log.
//!
//! Scenario sources: `distill/test-scenarios-p3-udp-slots-framing.md` (P3-S2,
//! S4, S5, S6, S9 guest half, S10), `test-scenarios-p2-owner-flows.md` (the
//! guest half of P2-S10, P2-S12), `test-scenarios-p6-recovery-vmm-ws.md`
//! (DG-5 boundary scenario: host owner restart), feature-delta § *Per-kind
//! total orders*, § *Control session*, § *Flow end*, DG-5 failure classes.
//! P3-S3 (a reserved intake port refuses a real application bind) needs a real
//! guest kernel and is a P6 walking-skeleton Tier-3 cell, not here.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::missing_panics_doc,
    clippy::items_after_statements
)]

mod half_close;
mod newtypes;
mod session;
mod tcp_accept;
mod tcp_connect;
mod udp_slots;

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::guest_transport::LaunchToken;
use overdrive_core::guest_transport::wire::{ControlMessage, FlowId, FlowOrigin, FlowRequest};
use overdrive_core::net::ListenPort;

use super::sim::{SimGuestClock, SimGuestEffect, SimGuestOwnerIo};
use super::{GuestClock, GuestFlowOwner, GuestSideObservation, HostChannel};
use crate::transport::GuestTransportToken;

pub(super) const WL: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 7);
pub(super) const GW: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
pub(super) const LAUNCH_HEX: &str = "0123456789abcdef0123456789abcdef";
pub(super) const OTHER_LAUNCH_HEX: &str = "fedcba9876543210fedcba9876543210";
/// The intake client a TcpAccept request names.
pub(super) const CLIENT: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 7), 40_000);

pub(super) fn launch() -> LaunchToken {
    LaunchToken::from_str(LAUNCH_HEX).expect("fixture launch token")
}

pub(super) fn other_launch() -> LaunchToken {
    LaunchToken::from_str(OTHER_LAUNCH_HEX).expect("fixture launch token")
}

pub(super) fn listen_port(port: u16) -> ListenPort {
    ListenPort::from_str(&port.to_string()).expect("non-zero listen port")
}

/// A host-origin flow id (TcpAccept).
pub(super) fn host_flow(seq: u32) -> FlowId {
    FlowId::new(FlowOrigin::Host, seq).expect("seq < 2^31")
}

/// A guest-origin flow id the guest never opened (U-5 discards).
pub(super) fn unknown_guest_flow() -> FlowId {
    FlowId::new(FlowOrigin::Guest, 0x7fff_fff0).expect("seq < 2^31")
}

/// A TcpAccept request for `flow` to `port`, carrying `launch`.
pub(super) fn accept_request(flow: FlowId, port: u16, launch: LaunchToken) -> FlowRequest {
    FlowRequest::TcpAccept { flow, client: CLIENT, listen_port: listen_port(port), launch }
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

/// One guest: the owner under test, the driver's clone of its seam, and the
/// shared clock.
pub(super) struct Guest {
    pub(super) owner: GuestFlowOwner,
    pub(super) io: SimGuestOwnerIo,
    pub(super) clock: Arc<SimGuestClock>,
}

impl Guest {
    pub(super) fn new() -> Self {
        let clock = Arc::new(SimGuestClock::new());
        let io = SimGuestOwnerIo::new(WL, Arc::clone(&clock));
        let token = GuestTransportToken { workload_addr: WL, gateway: GW, launch: launch() };
        let owner_clock: Arc<dyn GuestClock> = clock.clone();
        let owner = GuestFlowOwner::new(token, Box::new(io.clone()), owner_clock);
        Self { owner, io, clock }
    }

    pub(super) fn obs(&self) -> GuestSideObservation {
        self.owner.guest_observation()
    }

    /// One owner step; a per-flow or session failure is never fatal.
    pub(super) fn step(&mut self) {
        self.owner.dispatch_next().expect("no GuestOwnerFatal on this path");
    }

    /// One step per released event (no clock advance: no timer is due).
    pub(super) fn run_released(&mut self) {
        while self.io.deliverable() > 0 {
            self.step();
        }
    }

    /// Dispatches every released event and every due owner deadline until the
    /// owner reaches a blocking wait with nothing to deliver
    /// (`test_support::settle`, the sim's `SimWouldBlock`). Used after
    /// advancing the clock, where the number of due owner timers (audit tick
    /// period is not pinned) is not a test input.
    pub(super) fn settle(&mut self) {
        super::test_support::settle(&mut self.owner, &self.io);
    }

    /// Advances the clock by `by`, then settles.
    pub(super) fn advance(&mut self, by: Duration) {
        self.clock.advance(by);
        self.settle();
    }

    /// The first dispatch starts the control connect; the host accepts it.
    pub(super) fn open_session(&mut self) {
        self.step();
        self.io.complete_host_connect(HostChannel::Control, Ok(()));
        self.run_released();
        assert!(self.obs().session, "the control session is live");
    }

    /// Host-opened TcpAccept `flow` to `port` up to `Routed` (`Paired` written).
    pub(super) fn accept_flow(&mut self, flow: FlowId, port: u16) {
        self.io.host_open_accept(accept_request(flow, port, launch()));
        self.io.release_inbound_accept(flow);
        self.run_released();
        self.io.release_inbound_request(flow);
        self.run_released();
        self.io.complete_app_connect(flow, Ok(()));
        self.run_released();
        assert_eq!(self.obs().accepted.get(&flow), Some(&super::GuestAcceptPhase::Routed));
    }

    /// The application connects to `destination`; the guest opens V_g and
    /// writes its request. Returns the guest-origin flow id it claimed.
    pub(super) fn capture_flow(
        &mut self,
        cookie: super::SocketCookie,
        destination: SocketAddrV4,
    ) -> FlowId {
        self.io.app_connect(cookie, destination, true);
        self.run_released();
        self.io.complete_host_connect(HostChannel::FlowStream, Ok(()));
        self.run_released();
        self.last_request_flow()
    }

    /// `capture_flow` then the host's `Paired`.
    pub(super) fn connect_flow(
        &mut self,
        cookie: super::SocketCookie,
        destination: SocketAddrV4,
    ) -> FlowId {
        let flow = self.capture_flow(cookie, destination);
        self.io.host_control(ControlMessage::Paired { flow });
        self.run_released();
        assert!(self.obs().flows.contains_key(&flow), "paired");
        flow
    }

    /// The flow of the last TcpConnect request the guest wrote.
    pub(super) fn last_request_flow(&self) -> FlowId {
        self.io
            .effects()
            .iter()
            .rev()
            .find_map(|e| match e {
                SimGuestEffect::WriteRequest {
                    request: FlowRequest::TcpConnect { flow, .. },
                    ..
                } => Some(*flow),
                _ => None,
            })
            .expect("the guest wrote a TcpConnect request")
    }

    /// Starts slot `s`'s association from its current readiness event, both
    /// legs connected; returns the association's flow id.
    pub(super) fn associate(&mut self, s: SocketAddrV4) -> FlowId {
        let generation = self.io.slot_state(s).generation;
        self.io.release_slot_ready(s, generation);
        self.run_released();
        self.io.complete_host_connect(HostChannel::FlowStream, Ok(()));
        self.io.complete_host_connect(HostChannel::FlowSeqpacket, Ok(()));
        self.run_released();
        let flow = self.io.slot_state(s).flow.expect("both requests written");
        assert_eq!(self.obs().slots[&s].association, Some(flow));
        flow
    }

    /// The host pairs slot `s`'s association.
    pub(super) fn pair_slot(&mut self, flow: FlowId) {
        self.io.host_control(ControlMessage::Paired { flow });
        self.run_released();
    }

    /// Number of in-flight connects to `channel`.
    pub(super) fn connects(&self, channel: HostChannel) -> usize {
        self.io
            .pending_connects()
            .iter()
            .filter(|(_, t)| *t == super::sim::SimConnectTarget::Host(channel))
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

/// `got` holds exactly the messages of `want`, in any order.
pub(super) fn assert_same_messages(got: &[ControlMessage], want: &[ControlMessage]) {
    assert_eq!(got.len(), want.len(), "got {got:?}, want {want:?}");
    for m in want {
        assert!(got.contains(m), "missing {m:?} in {got:?}");
    }
}
