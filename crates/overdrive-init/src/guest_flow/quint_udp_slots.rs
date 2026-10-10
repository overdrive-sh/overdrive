//! Quint conformance driver — `udp_slots`, guest side (QPROJ-1, QPROJ-5,
//! QPROJ-7, DG-15).
//!
//! Replays `specs/quint/guest-flow-owner/udp_slots.qnt` (instances `slots_ok`,
//! `slots_live`) against the production guest flow owner
//! [`GuestFlowOwner`] built over the guest-owner seam sim [`SimGuestOwnerIo`]
//! and its clock [`SimGuestClock`]. The spec is referenced in place; the
//! driver plays the host owner and the applications (stimulus) and
//! re-implements no rule of the guest's slot pool.
//!
//! Action → driving port / seam stimulus (feature-delta § *Conformance
//! observation surface and action map*, "`udp_slots`, guest side"; DG-15):
//!
//! | Action | Driver |
//! |---|---|
//! | `init` | a fresh guest (`GuestFlowOwner::new` over a fresh seam) whose control session the host accepts |
//! | `AppClaim(k, d, s)`, `AppSendOnSlot(k, d, s)` | seam: application socket `k` sends to `d`, on slot `s` (`app_send_on`; the model's nondeterministic slot is the driver's choice) |
//! | `GuestReady(s, cur)` | seam releases that readiness event (`cur`: the slot's current cell generation; `¬cur`: every queued event of an earlier generation — the model keeps one `(s, false)` for them); if the owner starts an association, the host accepts both legs |
//! | `HostPair(i)`, `HostRefuse(i)`, `HostAbortAssoc(i)` | stimulus: the host's message for `i` is in flight; recorded in the stimulus model that produces `GuestRecvPaired` / `GuestRecvEnd` |
//! | `GuestRecvPaired(i)` | seam releases the host's `Paired` |
//! | `GuestRecvEnd(i)` | seam releases the host's `Refused` / `Abort`; the host, whose side is closed, observes the legs' end |
//! | `GuestPairingTimeout(s)` | `SimGuestClock` past `FLOW_PAIRING_DEADLINE` for the slot's association; the host observes the legs' end (the model carries a U-2 end to the host as its `Abort` only) |
//! | `HostRecvEnd(i)` | the host consumes the guest's `Abort` for `i` and / or observes the end of its legs |
//! | `SockClose(k)`, `SockCloseReportLost(k)` | seam: socket `k` is released with / without a `sock_release` report |
//! | `GuestReleaseSock(k)` | seam releases the report |
//! | `GuestAudit` | `SimGuestClock` to the owner's next deadline (the audit tick; its period is not pinned) |
//! | `IdleRelease(s)` | `SimGuestClock` past `GUEST_DATAGRAM_SLOT_IDLE` since the slot's last datagram, then to the owner's next deadlines until it acts |
//! | `AppPick`, `AppTake`, `Refill` | hazard-only (`claimSplit`, `cellsEmptiedOnly`): a design trace never reaches them — an error, never a no-op |
//!
//! Projection (`State::from_driver`): `slots(s).owned / .sock / .dest / .fid /
//! .routed` ← O5 `GuestFlowOwner::guest_observation().slots` (`owner`,
//! `association`, `routed`); `slots(s).parked` ← S `parked`; `ready` ← S
//! `readiness`, current iff the event's generation equals O5's for the slot;
//! `releasePending` ← S `release_reports`; `toHostAbort` ← S `outbox`;
//! `toHostEnd` ← S `legs_closed_unobserved` (S =
//! `SimGuestOwnerIo::guest_socket_observation`, QPROJ-7 arbiter). `sockAlive`,
//! `host`, `toGuest*` are stimulus; `nextFid`, `guestEnded`,
//! `slots(s).holders`, `endedNoNew`, `misdelivered`, `lateIgnored`,
//! `staleReassoc`, `staleGenStart` ghost / model-internal; `picks`, `residue`
//! hazard-only — not compared.
//!
//! Identity relabelling (a bijection fixed at first use, never a state
//! source): socket `k` ↔ cookie `k`; destination `d` ↔ `93.0.d.1:53`; slot
//! `s` ↔ `workload_addr:GUEST_UDP_SLOT_BASE + s`; association id `i` ↔ the
//! `i`-th guest-origin `FlowId` the owner opened (the model allocates ids
//! monotonically from 1, the owner allocates one per association it starts).
//! A production identity with no pre-image fails the comparison.
//!
//! **The stimulus model is verified against the spec.** The host's messages
//! in flight — `toGuestPaired`, `toGuestRefused`, `toGuestAbort` — are
//! stimulus (QPROJ-1), but the driver's own copy of them decides which message
//! `GuestRecvPaired` / `GuestRecvEnd` releases, so it is checked:
//! `State::from_spec` decodes them from each step's ITF post-state and stashes
//! them (a thread-local this module owns); at the start of the next step the
//! driver compares its model with the stash and fails with
//! [`DriverError::PeerDiverged`] on any difference. The stash verifies
//! stimulus; it never feeds `State` — `from_driver` reads only O5 and the
//! marked sim arbiter. The same comparison checks the instance's domains
//! against the bijection's (`SLOTS` = {0, 1}, `SOCKS` = {1, 2}, `FIDS` =
//! 1..=`MAXFID` of the instance constructor; every destination pick lies in
//! {1, 2}) and that a state labelled `step` is the instance's initial state.
//!
//! The spec defines no Quint `run` declarations, so there is no named run for
//! `#[quint_test]`; both instances replay randomized traces through
//! `#[quint_run]` (the seed is printed on failure).

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    reason = "a conformance driver asserts its own preconditions; protocol names are prose"
)]

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use itf::Value;
use overdrive_core::guest_transport::wire::{
    ControlMessage, FLOW_PAIRING_DEADLINE, FlowId, FlowRefusal,
};
use overdrive_core::guest_transport::{GUEST_DATAGRAM_SLOT_IDLE, GUEST_UDP_SLOT_BASE, LaunchToken};
use quint_connect::{Driver, Result, State, Step, quint_run, switch};
use serde::Deserialize;

use super::quint_itf::{self, ItfError};
use super::sim::{SimGuestClock, SimGuestOwnerIo, sim_cookie};
use super::test_support::{GuestProcess, settle};
use super::{GuestClock, GuestFlowOwner, HostChannel, SocketCookie};
use crate::transport::GuestTransportToken;

const WL: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 7);
const GW: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
const LAUNCH_HEX: &str = "0123456789abcdef0123456789abcdef";

/// Why a driver step cannot proceed.
#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("action `{action}` is not enabled in the production state: {why}")]
    NotEnabled { action: &'static str, why: String },
    #[error("action `{0}` is hazard-only: it is never enabled in a design instance")]
    HazardOnly(&'static str),
    #[error(
        "action `GuestReady` reached the model's association-id bound (MAXFID = {0}); the production \
         owner has no such bound, so the trace cannot be replayed past it"
    )]
    ModelBound(usize),
    #[error("the driver has no guest yet (no `init`)")]
    NoGuest,
    #[error(
        "peer stimulus model diverged from the spec's host variables: spec {spec}, driver {driver}"
    )]
    PeerDiverged { spec: String, driver: String },
    #[error("the spec instance's domains differ from the driver's assumptions: {0}")]
    DomainMismatch(String),
    #[error(
        "action `step` labelled a state that is not the instance's initial state (only a trace's \
         initial state may carry quint's `step` label)"
    )]
    StepNotInitial,
}

/// The bijection's domains (`SLOTS`, `SOCKS`, `DESTS` ⊆ these).
const SLOTS: [i64; 2] = [0, 1];
const SOCKS: [i64; 2] = [1, 2];
const DESTS: [i64; 2] = [1, 2];

// ----- the spec's host variables (stimulus verification, never projection) ----

/// The host's messages in flight to the guest (`toGuestPaired`,
/// `toGuestRefused`, `toGuestAbort`), in the spec's vocabulary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct HostVars {
    paired: BTreeSet<i64>,
    refused: BTreeSet<i64>,
    abort: BTreeSet<i64>,
}

/// The instance's domains as the spec's state carries them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Domains {
    slots: BTreeSet<i64>,
    socks: BTreeSet<i64>,
    fids: BTreeSet<i64>,
}

/// One step's spec post-state, as the next step's driver checks it.
struct SpecStash {
    host: HostVars,
    domains: Domains,
    /// The state is the instance's initial state.
    initial: bool,
}

thread_local! {
    /// Written by `State::from_spec` (each step's post-state), taken by the
    /// driver at its next step. Verifies the stimulus model; never a
    /// projection source.
    static SPEC_HOST: RefCell<Option<SpecStash>> = const { RefCell::new(None) };
}

impl HostVars {
    fn from_vars(vars: &BTreeMap<String, Value>) -> std::result::Result<Self, ItfError> {
        Ok(Self {
            paired: quint_itf::int_set(quint_itf::var(vars, "toGuestPaired")?)?,
            refused: quint_itf::int_set(quint_itf::var(vars, "toGuestRefused")?)?,
            abort: quint_itf::int_set(quint_itf::var(vars, "toGuestAbort")?)?,
        })
    }
}

impl Domains {
    fn from_vars(vars: &BTreeMap<String, Value>) -> std::result::Result<Self, ItfError> {
        let keys = |name: &str| -> std::result::Result<BTreeSet<i64>, ItfError> {
            quint_itf::map_entries(quint_itf::var(vars, name)?)?
                .into_iter()
                .map(|(k, _)| quint_itf::int(k))
                .collect()
        };
        Ok(Self { slots: keys("slots")?, socks: keys("sockAlive")?, fids: keys("host")? })
    }
}

/// A handler's view of the guest under test: the production owner, the
/// driver's clone of its seam, and the shared clock.
struct Guest<'a> {
    owner: &'a mut GuestFlowOwner,
    io: &'a SimGuestOwnerIo,
    clock: &'a SimGuestClock,
}

/// What the driver plays (QPROJ-1: the host owner and the applications), and
/// the instants of the stimuli it issued (its own observation of the calls it
/// made, used to place the clock for the time actions).
#[derive(Default)]
struct Stimulus {
    /// The host's `Paired` in flight to the guest, by association id.
    to_guest_paired: BTreeSet<i64>,
    /// The host's `Refused` in flight, by association id.
    to_guest_refused: BTreeSet<i64>,
    /// The host's `Abort` in flight, by association id.
    to_guest_abort: BTreeSet<i64>,
    /// When each slot's current association was started (GuestReady).
    assoc_started: BTreeMap<i64, Instant>,
    /// When the application last sent on each slot.
    last_datagram: BTreeMap<i64, Instant>,
}

struct UdpSlotsDriver {
    /// The model's association-id bound of the instance under replay.
    max_fid: usize,
    /// The production owner under test.
    owner: GuestProcess,
    /// The driver's clone of the owner's seam (the QPROJ-7 arbiter).
    io: SimGuestOwnerIo,
    /// The clock the owner and its seam read.
    clock: Arc<SimGuestClock>,
    stimulus: Stimulus,
    /// Association id `i` ↔ `fids[i - 1]` (order of first sight).
    fids: Vec<FlowId>,
    /// A state labelled `step` was taken as the initial state; the next step
    /// checks that it was.
    step_as_init: bool,
}

// ----- the bijection ---------------------------------------------------------

fn cookie(k: i64) -> SocketCookie {
    sim_cookie(u64::try_from(k).expect("model sockets are positive"))
}

fn dest(d: i64) -> SocketAddrV4 {
    SocketAddrV4::new(
        Ipv4Addr::new(93, 0, u8::try_from(d).expect("model destinations are small"), 1),
        53,
    )
}

fn slot(s: i64) -> SocketAddrV4 {
    SocketAddrV4::new(WL, GUEST_UDP_SLOT_BASE + u16::try_from(s).expect("model slots are small"))
}

fn slot_index(addr: SocketAddrV4) -> Option<i64> {
    SLOTS.into_iter().find(|s| slot(*s) == addr)
}

fn sock_index(c: SocketCookie) -> Option<i64> {
    SOCKS.into_iter().find(|k| cookie(*k) == c)
}

fn dest_index(addr: SocketAddrV4) -> Option<i64> {
    DESTS.into_iter().find(|d| dest(*d) == addr)
}

impl UdpSlotsDriver {
    fn new(max_fid: usize) -> Self {
        let clock = Arc::new(SimGuestClock::new());
        let io = SimGuestOwnerIo::new(WL, Arc::clone(&clock));
        Self {
            max_fid,
            owner: GuestProcess::NotStarted,
            io,
            clock,
            stimulus: Stimulus::default(),
            fids: Vec::new(),
            step_as_init: false,
        }
    }

    /// A driver for a new run: no stash of an earlier run is compared.
    fn fresh(max_fid: usize) -> Self {
        SPEC_HOST.with(|stash| stash.borrow_mut().take());
        Self::new(max_fid)
    }

    // ----- stimulus verification ------------------------------------------------------

    /// The bijection's domains for this instance.
    fn domains(&self) -> Domains {
        let max_fid = i64::try_from(self.max_fid).expect("MAXFID is small");
        Domains {
            slots: SLOTS.into_iter().collect(),
            socks: SOCKS.into_iter().collect(),
            fids: (1..=max_fid).collect(),
        }
    }

    /// Compares the stimulus model with the spec's post-state of the previous
    /// step (stashed by `State::from_spec`): the instance's domains, the
    /// `step`-labelled initial state, then the host's messages in flight.
    /// Nothing to compare before the run's first step.
    fn verify_stimulus(&mut self) -> std::result::Result<(), DriverError> {
        let Some(spec) = SPEC_HOST.with(|stash| stash.borrow_mut().take()) else {
            return Ok(());
        };
        let domains = self.domains();
        if spec.domains != domains {
            return Err(DriverError::DomainMismatch(format!(
                "spec {:?}, driver {domains:?}",
                spec.domains
            )));
        }
        if std::mem::take(&mut self.step_as_init) && !spec.initial {
            return Err(DriverError::StepNotInitial);
        }
        let model = HostVars {
            paired: self.stimulus.to_guest_paired.clone(),
            refused: self.stimulus.to_guest_refused.clone(),
            abort: self.stimulus.to_guest_abort.clone(),
        };
        if model != spec.host {
            return Err(DriverError::PeerDiverged {
                spec: format!("{:?}", spec.host),
                driver: format!("{model:?}"),
            });
        }
        Ok(())
    }

    fn guest(&mut self) -> std::result::Result<Guest<'_>, DriverError> {
        let owner = self.owner.owner_mut().ok_or(DriverError::NoGuest)?;
        Ok(Guest { owner, io: &self.io, clock: &self.clock })
    }

    fn fid(&self, i: i64) -> std::result::Result<FlowId, DriverError> {
        usize::try_from(i)
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| self.fids.get(i).copied())
            .ok_or_else(|| DriverError::NotEnabled {
                action: "association",
                why: format!("association id {i} has no production flow yet"),
            })
    }

    /// Fixes the bijection for every association the owner opened since the
    /// last step (order of first sight).
    fn note_new_flows(&mut self) {
        let Some(owner) = self.owner.owner() else { return };
        for slot_obs in owner.guest_observation().slots.values() {
            if let Some(flow) = slot_obs.association
                && !self.fids.contains(&flow)
            {
                self.fids.push(flow);
            }
        }
    }

    // ----- actions -----------------------------------------------------------

    /// `init`: a fresh guest whose control session the host accepted.
    fn init(&mut self) {
        let clock = Arc::new(SimGuestClock::new());
        let io = SimGuestOwnerIo::new(WL, Arc::clone(&clock));
        let launch = LaunchToken::from_str(LAUNCH_HEX).expect("fixture launch token");
        let token = GuestTransportToken { workload_addr: WL, gateway: GW, launch };
        let owner_clock: Arc<dyn GuestClock> = clock.clone();
        let owner = GuestFlowOwner::new(token, Box::new(io.clone()), owner_clock);
        self.owner = GuestProcess::Alive(owner);
        self.io = io;
        self.clock = clock;
        let guest = self.guest().expect("the owner was just started");
        settle(guest.owner, guest.io);
        guest.io.complete_host_connect(HostChannel::Control, Ok(()));
        settle(guest.owner, guest.io);
        self.stimulus = Stimulus::default();
        self.fids.clear();
        self.step_as_init = false;
    }

    /// A state labelled `step` (quint 0.32 labels the initial state of some
    /// traces so): taken as the initial state, which the next step verifies.
    fn initial_step(&mut self) {
        self.init();
        self.step_as_init = true;
    }

    /// `AppClaim(k, d, s)` / `AppSendOnSlot(k, d, s)`: the application sends.
    fn app_send(&mut self, k: i64, d: i64, s: i64) -> std::result::Result<(), DriverError> {
        if !DESTS.contains(&d) {
            return Err(DriverError::DomainMismatch(format!(
                "the trace picks destination {d}; the driver's bijection covers {DESTS:?}"
            )));
        }
        let guest = self.guest()?;
        guest.io.app_send_on(cookie(k), dest(d), slot(s));
        let now = guest.clock.now();
        settle(guest.owner, guest.io);
        self.stimulus.last_datagram.insert(s, now);
        Ok(())
    }

    /// `GuestReady(s, cur)`: the seam releases the readiness event(s).
    fn guest_ready(&mut self, s: i64, cur: bool) -> std::result::Result<(), DriverError> {
        let max_fid = self.max_fid;
        let opened = self.fids.len();
        let guest = self.guest()?;
        let state = guest.io.slot_state(slot(s));
        let events: Vec<_> = guest
            .io
            .guest_socket_observation()
            .readiness
            .into_iter()
            .filter(|(a, generation)| *a == slot(s) && (*generation == state.generation) == cur)
            .collect();
        if events.is_empty() {
            return Err(DriverError::NotEnabled {
                action: "GuestReady",
                why: format!("slot {s} has no queued readiness event (current = {cur})"),
            });
        }
        if cur
            && opened >= max_fid
            && state.owner.is_some()
            && state.flow.is_none()
            && !state.routed
        {
            return Err(DriverError::ModelBound(max_fid));
        }
        for (addr, generation) in events {
            guest.io.release_slot_ready(addr, generation);
        }
        settle(guest.owner, guest.io);
        let pending: Vec<_> = guest.io.pending_connects();
        let stream = pending
            .iter()
            .any(|(_, t)| *t == super::sim::SimConnectTarget::Host(HostChannel::FlowStream));
        let seqpacket = pending
            .iter()
            .any(|(_, t)| *t == super::sim::SimConnectTarget::Host(HostChannel::FlowSeqpacket));
        if stream && seqpacket {
            // the owner started an association: the host accepts both legs
            guest.io.complete_host_connect(HostChannel::FlowStream, Ok(()));
            guest.io.complete_host_connect(HostChannel::FlowSeqpacket, Ok(()));
            let now = guest.clock.now();
            settle(guest.owner, guest.io);
            self.stimulus.assoc_started.insert(s, now);
        }
        self.note_new_flows();
        Ok(())
    }

    /// `HostPair(i)`: the host's `Paired` for `i` is in flight.
    fn host_pair(&mut self, i: i64) {
        self.stimulus.to_guest_paired.insert(i);
    }

    /// `HostRefuse(i)`: the host's `Refused` for `i` is in flight.
    fn host_refuse(&mut self, i: i64) {
        self.stimulus.to_guest_refused.insert(i);
    }

    /// `HostAbortAssoc(i)`: the host's `Abort` for `i` is in flight.
    fn host_abort_assoc(&mut self, i: i64) {
        self.stimulus.to_guest_abort.insert(i);
    }

    /// `GuestRecvPaired(i)`: the seam releases the host's `Paired`.
    fn guest_recv_paired(&mut self, i: i64) -> std::result::Result<(), DriverError> {
        if !self.stimulus.to_guest_paired.remove(&i) {
            return Err(DriverError::NotEnabled {
                action: "GuestRecvPaired",
                why: format!("no Paired for {i}"),
            });
        }
        let flow = self.fid(i)?;
        let guest = self.guest()?;
        guest.io.host_control(ControlMessage::Paired { flow });
        settle(guest.owner, guest.io);
        Ok(())
    }

    /// `GuestRecvEnd(i)`: the seam releases the host's `Refused` / `Abort`.
    fn guest_recv_end(&mut self, i: i64) -> std::result::Result<(), DriverError> {
        let flow = self.fid(i)?;
        // the spec removes `i` from both sets; the stimulus is the one present
        let refused = self.stimulus.to_guest_refused.remove(&i);
        let aborted = self.stimulus.to_guest_abort.remove(&i);
        let message = if refused {
            ControlMessage::Refused { flow, refusal: FlowRefusal::Policy }
        } else if aborted {
            ControlMessage::Abort { flow }
        } else {
            return Err(DriverError::NotEnabled {
                action: "GuestRecvEnd",
                why: format!("no Refused / Abort for {i}"),
            });
        };
        let guest = self.guest()?;
        guest.io.host_control(message);
        settle(guest.owner, guest.io);
        // the host's side of `i` is closed: it observes the legs' close at once
        guest.io.host_observe_legs_end(flow);
        Ok(())
    }

    /// `GuestPairingTimeout(s)`: the clock passes the association's pairing
    /// deadline.
    fn guest_pairing_timeout(&mut self, s: i64) -> std::result::Result<(), DriverError> {
        let started = self.stimulus.assoc_started.get(&s).copied().ok_or_else(|| {
            DriverError::NotEnabled {
                action: "GuestPairingTimeout",
                why: format!("slot {s} has no association the driver started"),
            }
        })?;
        let mut guest = self.guest()?;
        let Some(flow) = guest.io.slot_state(slot(s)).flow else {
            return Err(DriverError::NotEnabled {
                action: "GuestPairingTimeout",
                why: format!("slot {s} has no pending association"),
            });
        };
        advance_to(&mut guest, started + FLOW_PAIRING_DEADLINE);
        // the model carries a U-2 end to the host as its Abort only
        guest.io.host_observe_legs_end(flow);
        Ok(())
    }

    /// `HostRecvEnd(i)`: the host consumes the guest's `Abort` and / or
    /// observes the end of `i`'s legs.
    fn host_recv_end(&mut self, i: i64) -> std::result::Result<(), DriverError> {
        let flow = self.fid(i)?;
        let guest = self.guest()?;
        let took = guest.io.host_take_message(ControlMessage::Abort { flow });
        let ended = guest.io.guest_socket_observation().legs_closed_unobserved.contains(&flow);
        if ended {
            guest.io.host_observe_legs_end(flow);
        }
        if took || ended {
            Ok(())
        } else {
            Err(DriverError::NotEnabled {
                action: "HostRecvEnd",
                why: format!("nothing from the guest for {i}"),
            })
        }
    }

    /// `SockClose(k)` / `SockCloseReportLost(k)`: the application socket is
    /// released, with / without a `sock_release` report.
    fn sock_close(&mut self, k: i64, report: bool) -> std::result::Result<(), DriverError> {
        let guest = self.guest()?;
        guest.io.app_close_socket(cookie(k), report);
        settle(guest.owner, guest.io);
        Ok(())
    }

    /// `GuestReleaseSock(k)`: the seam releases the report.
    fn guest_release_sock(&mut self, k: i64) -> std::result::Result<(), DriverError> {
        let guest = self.guest()?;
        guest.io.release_socket_report(cookie(k));
        settle(guest.owner, guest.io);
        Ok(())
    }

    /// `GuestAudit`: the clock to the owner's next deadline (the audit tick).
    fn guest_audit(&mut self) -> std::result::Result<(), DriverError> {
        let mut guest = self.guest()?;
        let deadline = next_deadline(&guest).ok_or_else(|| DriverError::NotEnabled {
            action: "GuestAudit",
            why: "the owner waits for no deadline (no audit tick)".to_owned(),
        })?;
        advance_to(&mut guest, deadline);
        Ok(())
    }

    /// `IdleRelease(s)`: the clock past `GUEST_DATAGRAM_SLOT_IDLE` since the
    /// slot's last datagram, then to the owner's next deadlines (the audits
    /// that measure the slot's counters) until the slot is released or a
    /// second idle period has passed.
    fn idle_release(&mut self, s: i64) -> std::result::Result<(), DriverError> {
        let last = self.stimulus.last_datagram.get(&s).copied().ok_or_else(|| {
            DriverError::NotEnabled {
                action: "IdleRelease",
                why: format!("slot {s} never carried a datagram"),
            }
        })?;
        let mut guest = self.guest()?;
        let idle = last + GUEST_DATAGRAM_SLOT_IDLE;
        advance_to(&mut guest, idle);
        let bound = idle + GUEST_DATAGRAM_SLOT_IDLE;
        while guest.io.slot_state(slot(s)).owner.is_some() {
            match next_deadline(&guest) {
                Some(deadline) if deadline <= bound => advance_to(&mut guest, deadline),
                _ => break,
            }
        }
        Ok(())
    }
}

/// The owner's earliest armed deadline as its blocking wait passed it
/// (`SimGuestOwnerIo::armed_deadline`).
fn next_deadline(guest: &Guest<'_>) -> Option<Instant> {
    guest.io.armed_deadline()
}

/// Moves the clock to `at` (never backwards) and settles the owner.
fn advance_to(guest: &mut Guest<'_>, at: Instant) {
    let now = guest.clock.now();
    if at > now {
        guest.clock.advance(at - now);
    }
    settle(guest.owner, guest.io);
}

impl Driver for UdpSlotsDriver {
    type State = UdpSlotsState;

    fn step(&mut self, step: &Step) -> Result {
        self.verify_stimulus()?;
        switch!(step {
            init => self.init(),
            // quint 0.32 `run --n-traces` labels the initial state of some
            // traces `step` instead of `init` (observed at index 0, with the
            // `init` state's values); it is taken as the initial state, and
            // the next step fails with `StepNotInitial` unless it was.
            step => self.initial_step(),
            AppClaim(k: i64, d: i64, s: i64) => self.app_send(k, d, s)?,
            AppSendOnSlot(k: i64, d: i64, s: i64) => self.app_send(k, d, s)?,
            GuestReady(s: i64, cur: bool) => self.guest_ready(s, cur)?,
            HostPair(i: i64) => self.host_pair(i),
            HostRefuse(i: i64) => self.host_refuse(i),
            HostAbortAssoc(i: i64) => self.host_abort_assoc(i),
            GuestRecvPaired(i: i64) => self.guest_recv_paired(i)?,
            GuestRecvEnd(i: i64) => self.guest_recv_end(i)?,
            GuestPairingTimeout(s: i64) => self.guest_pairing_timeout(s)?,
            HostRecvEnd(i: i64) => self.host_recv_end(i)?,
            SockClose(k: i64) => self.sock_close(k, true)?,
            SockCloseReportLost(k: i64) => self.sock_close(k, false)?,
            GuestReleaseSock(k: i64) => self.guest_release_sock(k)?,
            GuestAudit => self.guest_audit()?,
            IdleRelease(s: i64) => self.idle_release(s)?,
            AppPick => return Err(DriverError::HazardOnly("AppPick").into()),
            AppTake => return Err(DriverError::HazardOnly("AppTake").into()),
            Refill => return Err(DriverError::HazardOnly("Refill").into()),
        })
    }
}

// ----- the compared state ------------------------------------------------------

/// One slot's compared fields (`holders` is ghost).
#[derive(Debug, Clone, PartialEq, Eq)]
struct SlotView {
    owned: bool,
    sock: i64,
    dest: i64,
    fid: i64,
    routed: bool,
    parked: BTreeSet<(i64, i64)>,
}

/// The compared `udp_slots` variables, guest side.
#[derive(Debug, PartialEq, Eq)]
struct UdpSlotsState {
    slots: BTreeMap<i64, SlotView>,
    ready: BTreeSet<(i64, bool)>,
    release_pending: BTreeMap<i64, bool>,
    to_host_abort: BTreeSet<i64>,
    to_host_end: BTreeSet<i64>,
}

/// Why the production state has no model image.
#[derive(Debug, thiserror::Error)]
enum ProjectionError {
    #[error("{0} has no pre-image in the model (the bijection is fixed by the driver)")]
    NoPreImage(String),
}

impl UdpSlotsState {
    fn from_value(value: &Value) -> std::result::Result<Self, ItfError> {
        let vars = quint_itf::vars(value)?;
        let slots = quint_itf::int_map(quint_itf::var(&vars, "slots")?, |slot| {
            Ok(SlotView {
                owned: quint_itf::boolean(quint_itf::field(slot, "owned")?)?,
                sock: quint_itf::int(quint_itf::field(slot, "sock")?)?,
                dest: quint_itf::int(quint_itf::field(slot, "dest")?)?,
                fid: quint_itf::int(quint_itf::field(slot, "fid")?)?,
                routed: quint_itf::boolean(quint_itf::field(slot, "routed")?)?,
                parked: quint_itf::set_elems(quint_itf::field(slot, "parked")?)?
                    .into_iter()
                    .map(|tag| {
                        let parts = quint_itf::tuple_elems(tag)?;
                        let [k, d] = parts.as_slice() else {
                            return Err(ItfError::Shape {
                                expected: "a (socket, destination) tag",
                                found: tag.clone(),
                            });
                        };
                        Ok((quint_itf::int(k)?, quint_itf::int(d)?))
                    })
                    .collect::<std::result::Result<_, _>>()?,
            })
        })?;
        let ready = quint_itf::set_elems(quint_itf::var(&vars, "ready")?)?
            .into_iter()
            .map(|event| {
                let parts = quint_itf::tuple_elems(event)?;
                let [s, cur] = parts.as_slice() else {
                    return Err(ItfError::Shape {
                        expected: "a (slot, current) event",
                        found: event.clone(),
                    });
                };
                Ok((quint_itf::int(s)?, quint_itf::boolean(cur)?))
            })
            .collect::<std::result::Result<_, _>>()?;
        Ok(Self {
            slots,
            ready,
            release_pending: quint_itf::int_map(
                quint_itf::var(&vars, "releasePending")?,
                quint_itf::boolean,
            )?,
            to_host_abort: quint_itf::int_set(quint_itf::var(&vars, "toHostAbort")?)?,
            to_host_end: quint_itf::int_set(quint_itf::var(&vars, "toHostEnd")?)?,
        })
    }
}

impl<'de> Deserialize<'de> for UdpSlotsState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

impl State<UdpSlotsDriver> for UdpSlotsState {
    /// Decodes the compared variables, and stashes the host's messages in
    /// flight and the instance's domains for the next step's stimulus
    /// verification (never a projection source).
    fn from_spec(value: Value) -> Result<Self> {
        let state = Self::from_value(&value)?;
        let vars = quint_itf::vars(&value)?;
        let host = HostVars::from_vars(&vars)?;
        let domains = Domains::from_vars(&vars)?;
        let free = SlotView {
            owned: false,
            sock: 0,
            dest: 0,
            fid: 0,
            routed: false,
            parked: BTreeSet::new(),
        };
        let initial = host == HostVars::default()
            && state.slots.values().all(|slot| *slot == free)
            && state.ready.is_empty()
            && state.release_pending.values().all(|pending| !pending)
            && state.to_host_abort.is_empty()
            && state.to_host_end.is_empty();
        SPEC_HOST.with(|stash| *stash.borrow_mut() = Some(SpecStash { host, domains, initial }));
        Ok(state)
    }

    fn from_driver(driver: &UdpSlotsDriver) -> Result<Self> {
        let owner = driver.owner.owner().ok_or(DriverError::NoGuest)?.guest_observation();
        // conformance-lint: sim-arbiter-ok QPROJ-7 guest-owner seam arbiter (parked, readiness, release reports, outbox, legs), Tier-3 R5-14 / R5-27
        let sim = driver.io.guest_socket_observation();
        let fid_of = |flow: FlowId| -> std::result::Result<i64, ProjectionError> {
            driver
                .fids
                .iter()
                .position(|f| *f == flow)
                .and_then(|i| i64::try_from(i + 1).ok())
                .ok_or_else(|| ProjectionError::NoPreImage(format!("flow {flow:?}")))
        };
        let tag_of = |(c, d): (SocketCookie, SocketAddrV4)| -> std::result::Result<(i64, i64), ProjectionError> {
            Ok((
                sock_index(c).ok_or_else(|| ProjectionError::NoPreImage(format!("socket {c:?}")))?,
                dest_index(d).ok_or_else(|| ProjectionError::NoPreImage(format!("destination {d}")))?,
            ))
        };
        let mut slots = BTreeMap::new();
        for (addr, obs) in &owner.slots {
            let parked = sim.parked.get(addr).cloned().unwrap_or_default();
            let Some(s) = slot_index(*addr) else {
                if obs.owner.is_some()
                    || obs.association.is_some()
                    || obs.routed
                    || !parked.is_empty()
                {
                    return Err(ProjectionError::NoPreImage(format!("slot {addr} in use")).into());
                }
                continue;
            };
            let (is_owned, owner_sock, owner_dest) = match obs.owner {
                Some(tag) => {
                    let (k, d) = tag_of(tag)?;
                    (true, k, d)
                }
                None => (false, 0, 0),
            };
            let fid = obs.association.map(fid_of).transpose()?.unwrap_or(0);
            let parked = parked.into_iter().map(tag_of).collect::<std::result::Result<_, _>>()?;
            slots.insert(
                s,
                SlotView {
                    owned: is_owned,
                    sock: owner_sock,
                    dest: owner_dest,
                    fid,
                    routed: obs.routed,
                    parked,
                },
            );
        }
        let mut ready = BTreeSet::new();
        for (addr, generation) in &sim.readiness {
            let s = slot_index(*addr)
                .ok_or_else(|| ProjectionError::NoPreImage(format!("readiness of {addr}")))?;
            let current = owner.slots.get(addr).is_some_and(|obs| obs.generation == *generation);
            ready.insert((s, current));
        }
        let mut release_pending: BTreeMap<i64, bool> =
            SOCKS.into_iter().map(|k| (k, false)).collect();
        for c in &sim.release_reports {
            let k = sock_index(*c)
                .ok_or_else(|| ProjectionError::NoPreImage(format!("report for {c:?}")))?;
            release_pending.insert(k, true);
        }
        let mut to_host_abort = BTreeSet::new();
        for message in &sim.outbox {
            match message {
                ControlMessage::Abort { flow } => {
                    to_host_abort.insert(fid_of(*flow)?);
                }
                other => {
                    return Err(
                        ProjectionError::NoPreImage(format!("guest message {other:?}")).into()
                    );
                }
            }
        }
        let to_host_end = sim
            .legs_closed_unobserved
            .iter()
            .map(|f| fid_of(*f))
            .collect::<std::result::Result<_, _>>()?;
        Ok(Self { slots, ready, release_pending, to_host_abort, to_host_end })
    }
}

// ----- the runs ----------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[quint_run(spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt", main = "slots_ok")]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init); model fidelity open, upstream UI-R2-5"]
fn quint_udp_slots_guest_slots_ok() -> impl Driver {
    UdpSlotsDriver::fresh(3)
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_run(spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt", main = "slots_live")]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init); model fidelity open, upstream UI-R2-5"]
fn quint_udp_slots_guest_slots_live() -> impl Driver {
    UdpSlotsDriver::fresh(2)
}
