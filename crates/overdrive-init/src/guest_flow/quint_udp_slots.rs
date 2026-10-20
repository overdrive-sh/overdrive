//! Quint conformance driver — `udp_slots`, guest side (QPROJ-1, QPROJ-5,
//! QPROJ-7, DG-5-A, DG-5-B, EVMOD-DGRAM).
//!
//! Replays `specs/quint/guest-flow-owner/udp_slots.qnt`, conformance main
//! `slots_conf` (`REC = true`, `MAXFID` above any trace's reach, DG-5-B),
//! against the production guest flow owner [`GuestFlowOwner`] built over the
//! guest-owner seam sim [`SimGuestOwnerIo`] (its forwarder context a
//! `SimFlowForwarder`) and its clock [`SimGuestClock`]. The spec is referenced
//! in place; the driver plays the host owner and the applications (stimulus)
//! and re-implements no rule of the guest's slot pool.
//!
//! **Step record (D-UP-1).** `Config::nondet` is the single element
//! `slots_conf::udp_slots::conf_step`; every `#[quint_test]` and the
//! `#[quint_run]` name `slots_conf`; the projection matches the qualified
//! names `slots_conf::udp_slots::<var>` ([`quint_itf::Qualified`]).
//!
//! Action → driving port / seam stimulus (feature-delta § *Conformance
//! observation surface and action map*, "`udp_slots`, guest side"). Every
//! handler ends by dispatching the owner until it is quiescent (DG-5-Q):
//!
//! | Action | Driver |
//! |---|---|
//! | `init` | a fresh guest (`GuestFlowOwner::new` over a fresh seam) whose control session the host accepts |
//! | `AppClaim(k, d, s)`, `AppSendOnSlot(k, d, s)` | seam: application socket `k` sends one datagram to `d` on slot `s` (the model's nondeterministic claim is the driver's choice of `s`); the forwarder's runs are stepped so an attached slot forwards it |
//! | `GuestReady(s)` | seam releases slot `s`'s readiness; the host accepts the V_g the owner opens for it |
//! | `HostPair(i)`, `HostRefuse(i)`, `HostAbortAssoc(i)` | stimulus: the host's `Paired` / `Refused` / `Abort` for `i` is in flight |
//! | `GuestRecvPaired(i)` | seam releases the host's `Paired`; the owner attaches (S, V_g) and the attach-time run is stepped |
//! | `GuestRecvEnd(i)` | seam releases the host's `Refused` / `Abort` and the host's close of V_h (whichever the owner handles second finds no association) |
//! | `GuestPairingTimeout(s)` | `SimGuestClock` advanced to the slot's association start + `FLOW_PAIRING_DEADLINE` |
//! | `HostRecvEnd(i)` | the host consumes the guest's `Abort` for `i` and / or observes V_g's close; a reply the guest did not write fails the step |
//! | `SockClose(k)`, `SockCloseReportLost(k)` | seam: socket `k` is released with / without a `sock_release` report |
//! | `GuestReleaseSock(k)` | seam releases the report |
//! | `GuestAuditIdle(idle)` | `SimGuestClock` to the owner's next audit tick (`GUEST_OWNER_AUDIT_PERIOD`, fixed rate from the first dispatch, DG-5-A); with a non-empty `idle`, tick by tick until those slots have been idle `GUEST_DATAGRAM_SLOT_IDLE`, the host keeping every other attached slot busy |
//! | `AppPick`, `AppTake` | hazard-only — an error, never a no-op |
//!
//! Projection (`State::from_driver`): `slots(s).owned / .sock / .dest / .fid
//! / .attached` ← O QPROJ-5 `guest_observation().slots` (`owner`,
//! `association`, `attached.is_some()`); `slots(s).parked` ← S `parked` (the
//! slot socket's receive queue, by origin); `releasePending` ← S
//! `release_reports` (S = `SimGuestOwnerIo::guest_socket_observation`, the
//! QPROJ-7 arbiter, Tier-3 pairs R5-14 and V-EV-DGRAM). `sockAlive`, `host`
//! → stimulus; `toHost*`, `toGuest*` → model-only; `nextFid`, `guestEnded`,
//! `slots(s).holders`, `endedNoNew` → ghost / model-internal; `misdelivered`,
//! `lateIgnored`, `staleReassoc` → ghost; `picks` → hazard-only; `conf_step` →
//! model-only.
//!
//! Identity relabelling (a bijection fixed at first use, never a state
//! source): socket `k` ↔ cookie `k`; destination `d` ↔ `93.0.d.1:53`; slot
//! `s` ↔ `workload_addr:GUEST_UDP_SLOT_BASE + s`; association id `i` ↔ the
//! `i`-th guest-origin `FlowId` the owner opened for an association. A
//! production identity with no pre-image fails the comparison.
//!
//! **Abstract time.** The spec's `GuestAuditIdle` releases idle slots in one
//! step with no time passing for anything else; the production idle release
//! needs `GUEST_DATAGRAM_SLOT_IDLE` (120 s) of the owner's clock, which passes
//! every `FLOW_PAIRING_DEADLINE` (5 s) armed meanwhile. A trace whose idle
//! release would cross a pending association's pairing deadline is refused
//! with [`DriverError::AbstractTime`] rather than replayed wrongly.
//!
//! **The stimulus model is verified against the spec.** `State::from_spec`
//! stashes each step's `toGuestPaired`, `toGuestRefused`, `toGuestAbort` and
//! the instance's domains; the next step compares them with the driver's.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
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
use overdrive_core::guest_transport::{
    GUEST_DATAGRAM_SLOT_IDLE, GUEST_OWNER_AUDIT_PERIOD, GUEST_UDP_SLOT_BASE, LaunchToken,
};
use overdrive_core::vm::config::VsockPort;
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;

use super::quint_itf::{self, ItfError, Qualified};
use super::sim::{SimConnectTarget, SimGuestClock, SimGuestOwnerIo, sim_cookie};
use super::test_support::{GuestProcess, settle, start};
use super::{GuestClock, GuestFlowOwner, HostChannel, SocketCookie};
use crate::transport::{GuestHostPorts, GuestTransportToken};

/// The host listener ports the launch's `overdrive.transport=` token names
/// (GUEST-PORTS, ADR-0217: `beacon=`, `stream=`, `control=`, read by the
/// guest from its command line; it holds no host-port constant). The seam's
/// sim keys connections by the guest's own socket, so fixture values in the
/// valid range, three distinct and none the production default.
const fn host_ports() -> GuestHostPorts {
    GuestHostPorts {
        beacon: VsockPort::new(50_234),
        stream: VsockPort::new(50_240),
        control: VsockPort::new(50_243),
    }
}

const MAIN: &str = "slots_conf";
const MODULE: &str = "udp_slots";
const WL: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 7);
const GW: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
const LAUNCH_HEX: &str = "0123456789abcdef0123456789abcdef";
const SLOTS: [i64; 2] = [0, 1];
const SOCKS: [i64; 2] = [1, 2];
const DESTS: [i64; 2] = [1, 2];
/// `MAXFID` of `slots_conf` (`CONF_STEP_BOUND + 1`).
const CONF_MAX_FID: usize = 33;

/// Why a driver step cannot proceed.
#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("action `{action}` is not enabled in the production state: {why}")]
    NotEnabled { action: &'static str, why: String },
    #[error("action `{0}` is hazard-only: it is never enabled in a design instance")]
    HazardOnly(&'static str),
    #[error(
        "action `GuestReady` reached the model's association-id bound (MAXFID = {0}); the production \
         owner has no such bound"
    )]
    ModelBound(usize),
    #[error("action `{action}` cannot be replayed in production time: {why}")]
    AbstractTime { action: &'static str, why: String },
    #[error("the driver has no guest yet (no `init`)")]
    NoGuest,
    #[error(
        "peer stimulus model diverged from the spec's host variables: spec {spec}, driver {driver}"
    )]
    PeerDiverged { spec: String, driver: String },
    #[error("the spec instance's domains differ from the driver's: {0}")]
    DomainMismatch(String),
    #[error("{0} has no pre-image in the model")]
    NoPreImage(String),
}

// ----- stimulus verification ----------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct HostVars {
    paired: BTreeSet<i64>,
    refused: BTreeSet<i64>,
    abort: BTreeSet<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Domains {
    slots: BTreeSet<i64>,
    socks: BTreeSet<i64>,
    fids: BTreeSet<i64>,
}

struct SpecStash {
    host: HostVars,
    domains: Domains,
}

thread_local! {
    /// Written by `State::from_spec`, taken by the driver at its next step.
    /// Verifies the stimulus model; never a projection source.
    static SPEC_HOST: RefCell<Option<SpecStash>> = const { RefCell::new(None) };
}

impl HostVars {
    fn from_vars(vars: &Qualified) -> std::result::Result<Self, ItfError> {
        Ok(Self {
            paired: quint_itf::int_set(vars.var("toGuestPaired")?)?,
            refused: quint_itf::int_set(vars.var("toGuestRefused")?)?,
            abort: quint_itf::int_set(vars.var("toGuestAbort")?)?,
        })
    }
}

impl Domains {
    fn from_vars(vars: &Qualified) -> std::result::Result<Self, ItfError> {
        let keys = |name: &str| -> std::result::Result<BTreeSet<i64>, ItfError> {
            quint_itf::map_entries(vars.var(name)?)?
                .into_iter()
                .map(|(k, _)| quint_itf::int(k))
                .collect()
        };
        Ok(Self { slots: keys("slots")?, socks: keys("sockAlive")?, fids: keys("host")? })
    }
}

// ----- the driver ----------------------------------------------------------------

/// The host's message for an association, in flight toward the guest.
#[derive(Debug, Clone, Copy)]
enum HostMessage {
    Paired,
    Refused,
    Abort,
}

struct UdpSlotsDriver {
    /// The production guest flow owner under test, built at construction
    /// (S-7, the "Owner field" ruling): the only path to the owner.
    owner: GuestFlowOwner,
    /// The guest process's lifecycle (test support beside the owner field).
    process: GuestProcess,
    io: SimGuestOwnerIo,
    clock: Arc<SimGuestClock>,
    /// The owner's first dispatch: its audit ticks fall at `started + n · period`.
    started: Instant,
    to_guest_paired: BTreeSet<i64>,
    to_guest_refused: BTreeSet<i64>,
    to_guest_abort: BTreeSet<i64>,
    /// When each slot's current association started (its pairing deadline).
    assoc_started: BTreeMap<i64, Instant>,
    /// The production flow of association id `i` is `fids[i - 1]`.
    fids: Vec<FlowId>,
}

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

fn sock_index(c: SocketCookie) -> Option<i64> {
    SOCKS.into_iter().find(|k| cookie(*k) == c)
}

fn dest_index(addr: SocketAddrV4) -> Option<i64> {
    DESTS.into_iter().find(|d| dest(*d) == addr)
}

impl UdpSlotsDriver {
    fn new() -> Self {
        SPEC_HOST.with(|stash| stash.borrow_mut().take());
        let clock = Arc::new(SimGuestClock::new());
        let io = SimGuestOwnerIo::new(WL, Arc::clone(&clock));
        let started = clock.now();
        let launch = LaunchToken::from_str(LAUNCH_HEX).expect("fixture launch token");
        let token =
            GuestTransportToken { workload_addr: WL, gateway: GW, launch, ports: host_ports() };
        let owner_clock: Arc<dyn GuestClock> = clock.clone();
        let owner = GuestFlowOwner::new(token, Box::new(io.clone()), owner_clock);
        Self {
            owner,
            process: GuestProcess::NotStarted,
            io,
            clock,
            started,
            to_guest_paired: BTreeSet::new(),
            to_guest_refused: BTreeSet::new(),
            to_guest_abort: BTreeSet::new(),
            assoc_started: BTreeMap::new(),
            fids: Vec::new(),
        }
    }

    fn domains() -> Domains {
        let max_fid = i64::try_from(CONF_MAX_FID).expect("MAXFID is small");
        Domains {
            slots: SLOTS.into_iter().collect(),
            socks: SOCKS.into_iter().collect(),
            fids: (1..=max_fid).collect(),
        }
    }

    fn verify_stimulus(&self) -> std::result::Result<(), DriverError> {
        let Some(spec) = SPEC_HOST.with(|stash| stash.borrow_mut().take()) else {
            return Ok(());
        };
        let domains = Self::domains();
        if spec.domains != domains {
            return Err(DriverError::DomainMismatch(format!(
                "spec {:?}, driver {domains:?}",
                spec.domains
            )));
        }
        let model = HostVars {
            paired: self.to_guest_paired.clone(),
            refused: self.to_guest_refused.clone(),
            abort: self.to_guest_abort.clone(),
        };
        if model != spec.host {
            return Err(DriverError::PeerDiverged {
                spec: format!("{:?}", spec.host),
                driver: format!("{model:?}"),
            });
        }
        Ok(())
    }

    /// Dispatches the owner until quiescent (DG-5-Q): the owner's driving
    /// port, called by every handler.
    fn settle(&mut self) -> std::result::Result<(), DriverError> {
        if !self.process.is_alive() {
            return Err(DriverError::NoGuest);
        }
        let io = self.io.clone();
        let clock = Arc::clone(&self.clock);
        settle(&mut self.owner, &io, &clock);
        Ok(())
    }

    fn fid(&self, i: i64) -> std::result::Result<FlowId, DriverError> {
        usize::try_from(i)
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| self.fids.get(i).copied())
            .ok_or_else(|| DriverError::NotEnabled {
                action: "association",
                why: format!("association id {i} has no production flow"),
            })
    }

    fn note_new_flows(&mut self) {
        if !self.process.is_alive() {
            return;
        }
        let observation = self.owner.guest_observation();
        for s in SLOTS {
            if let Some(flow) = observation.slots.get(&slot(s)).and_then(|o| o.association)
                && !self.fids.contains(&flow)
            {
                self.fids.push(flow);
            }
        }
    }

    // ----- actions ------------------------------------------------------------------

    fn init(&mut self) -> std::result::Result<(), DriverError> {
        *self = Self::new();
        self.started = self.clock.now();
        start(&mut self.owner);
        self.process = GuestProcess::Alive;
        self.settle()?;
        self.io.complete_host_connect(HostChannel::Control, Ok(()));
        self.settle()
    }

    fn app_send(&mut self, k: i64, d: i64, s: i64) -> std::result::Result<(), DriverError> {
        if !DESTS.contains(&d) {
            return Err(DriverError::DomainMismatch(format!(
                "the trace picks destination {d}; the driver's bijection covers {DESTS:?}"
            )));
        }
        self.io.app_send_on(cookie(k), dest(d), slot(s));
        // an attached slot forwards the datagram: the forwarder's run (the
        // kernel's work item) is stepped
        self.io.run_forwarder();
        self.settle()
    }

    fn guest_ready(&mut self, s: i64) -> std::result::Result<(), DriverError> {
        if !self.io.guest_socket_observation().readiness.contains(&slot(s)) {
            return Err(DriverError::NotEnabled {
                action: "GuestReady",
                why: format!("slot {s} has no readiness event pending"),
            });
        }
        if self.fids.len() >= CONF_MAX_FID {
            return Err(DriverError::ModelBound(CONF_MAX_FID));
        }
        self.io.release_slot_ready(slot(s));
        self.settle()?;
        let stream = self
            .io
            .pending_connects()
            .iter()
            .any(|(_, t)| *t == SimConnectTarget::Host(HostChannel::FlowStream));
        if stream {
            self.assoc_started.insert(s, self.clock.now());
            self.io.complete_host_connect(HostChannel::FlowStream, Ok(()));
            self.settle()?;
        }
        self.note_new_flows();
        Ok(())
    }

    /// A host message for `i` is in flight (stimulus); the guest runs whatever
    /// is due (DG-5-Q).
    fn host_message(
        &mut self,
        i: i64,
        message: HostMessage,
    ) -> std::result::Result<(), DriverError> {
        let in_flight = match message {
            HostMessage::Paired => &mut self.to_guest_paired,
            HostMessage::Refused => &mut self.to_guest_refused,
            HostMessage::Abort => &mut self.to_guest_abort,
        };
        in_flight.insert(i);
        self.settle()
    }

    fn guest_recv_paired(&mut self, i: i64) -> std::result::Result<(), DriverError> {
        if !self.to_guest_paired.remove(&i) {
            return Err(DriverError::NotEnabled {
                action: "GuestRecvPaired",
                why: format!("no Paired for {i}"),
            });
        }
        let flow = self.fid(i)?;
        self.io.host_control(ControlMessage::Paired { flow });
        self.settle()?;
        // the attach-time run forwards the datagrams queued in S (EVMOD-DGRAM)
        self.io.run_forwarder();
        self.settle()
    }

    fn guest_recv_end(&mut self, i: i64) -> std::result::Result<(), DriverError> {
        let flow = self.fid(i)?;
        let message = if self.to_guest_refused.remove(&i) {
            ControlMessage::Refused { flow, refusal: FlowRefusal::Policy }
        } else if self.to_guest_abort.remove(&i) {
            ControlMessage::Abort { flow }
        } else {
            return Err(DriverError::NotEnabled {
                action: "GuestRecvEnd",
                why: format!("no Refused / Abort for {i}"),
            });
        };
        self.io.host_control(message);
        self.settle()?;
        // the host closes V_h with its refusal or abort: an attached guest
        // pair sees the 0x3 (its record is handled after the message, and
        // finds no association)
        self.io.host_vsock_end(flow);
        self.io.run_forwarder();
        self.io.release_forward_ends();
        self.settle()
    }

    fn guest_pairing_timeout(&mut self, s: i64) -> std::result::Result<(), DriverError> {
        let started =
            self.assoc_started.get(&s).copied().ok_or_else(|| DriverError::NotEnabled {
                action: "GuestPairingTimeout",
                why: format!("slot {s} has no association the driver started"),
            })?;
        self.clock.advance_to(started + FLOW_PAIRING_DEADLINE);
        self.settle()
    }

    fn host_recv_end(&mut self, i: i64) -> std::result::Result<(), DriverError> {
        let flow = self.fid(i)?;
        let took = self.io.host_take_first(|m| *m == ControlMessage::Abort { flow }).is_some();
        let ended = self.io.vsock_ended().contains(&flow);
        if took || ended {
            self.settle()
        } else {
            Err(DriverError::NotEnabled {
                action: "HostRecvEnd",
                why: format!("nothing from the guest for {i}"),
            })
        }
    }

    fn sock_close(&mut self, k: i64, report: bool) -> std::result::Result<(), DriverError> {
        self.io.app_close_socket(cookie(k), report);
        self.settle()
    }

    fn guest_release_sock(&mut self, k: i64) -> std::result::Result<(), DriverError> {
        self.io.release_socket_report(cookie(k));
        self.settle()?;
        self.io.run_forwarder();
        self.io.release_forward_ends();
        self.settle()
    }

    /// The owner's next audit tick after now (DG-5-A: fixed rate from the
    /// first dispatch).
    fn next_audit_tick(&self) -> Instant {
        let now = self.clock.now();
        let elapsed = now.saturating_duration_since(self.started);
        let period = GUEST_OWNER_AUDIT_PERIOD.as_nanos();
        let n = elapsed.as_nanos() / period + 1;
        self.started + GUEST_OWNER_AUDIT_PERIOD * u32::try_from(n).expect("traces are short")
    }

    /// The owned slots with a pending (unpaired) association.
    fn pending_associations(&self) -> BTreeSet<i64> {
        SLOTS
            .into_iter()
            .filter(|s| {
                let st = self.io.slot_state(slot(*s));
                st.flow.is_some() && st.attached.is_none()
            })
            .collect()
    }

    fn guest_audit(&mut self, idle: &Value) -> std::result::Result<(), DriverError> {
        let idle =
            quint_itf::int_set(idle).map_err(|e| DriverError::DomainMismatch(e.to_string()))?;
        let tick = self.next_audit_tick();
        let pending = self.pending_associations();
        if idle.is_empty() {
            let crosses = pending.iter().any(|s| {
                self.assoc_started.get(s).is_some_and(|t| *t + FLOW_PAIRING_DEADLINE <= tick)
            });
            if crosses {
                return Err(DriverError::AbstractTime {
                    action: "GuestAuditIdle",
                    why: "the next audit tick falls after a pending association's pairing deadline"
                        .to_owned(),
                });
            }
            self.clock.advance_to(tick);
            self.settle()?;
            self.io.run_forwarder();
            self.io.release_forward_ends();
            return self.settle();
        }
        if !pending.is_empty() {
            return Err(DriverError::AbstractTime {
                action: "GuestAuditIdle",
                why: format!(
                    "releasing idle slots {idle:?} takes GUEST_DATAGRAM_SLOT_IDLE, past the pairing deadline \
                     of the pending association(s) on {pending:?}"
                ),
            });
        }
        let bound =
            GUEST_DATAGRAM_SLOT_IDLE.as_secs() / GUEST_OWNER_AUDIT_PERIOD.as_secs().max(1) + 3;
        for _ in 0..bound {
            // the host keeps every other attached slot busy: one datagram
            // toward the guest per tick moves its counters
            for s in SLOTS {
                let st = self.io.slot_state(slot(s));
                if st.attached.is_some() && !idle.contains(&s) {
                    self.io.host_datagram(slot(s), &[0x4f, 0x56, 0x44, 0x31, 0, 0, 0, 0]);
                    self.io.run_forwarder();
                }
            }
            let tick = self.next_audit_tick();
            self.clock.advance_to(tick);
            self.settle()?;
            self.io.run_forwarder();
            self.io.release_forward_ends();
            self.settle()?;
            if idle.iter().all(|s| self.io.slot_state(slot(*s)).owner.is_none()) {
                return Ok(());
            }
        }
        Err(DriverError::NotEnabled {
            action: "GuestAuditIdle",
            why: format!("slots {idle:?} were not released after GUEST_DATAGRAM_SLOT_IDLE"),
        })
    }
}

impl Driver for UdpSlotsDriver {
    type State = UdpSlotsState;

    fn config() -> Config {
        Config { state: &[], nondet: &["slots_conf::udp_slots::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        self.verify_stimulus()?;
        switch!(step {
            init => self.init()?,
            AppClaim(k: i64, d: i64, s: i64) => self.app_send(k, d, s)?,
            AppSendOnSlot(k: i64, d: i64, s: i64) => self.app_send(k, d, s)?,
            AppPick => return Err(DriverError::HazardOnly("AppPick").into()),
            AppTake => return Err(DriverError::HazardOnly("AppTake").into()),
            GuestReady(s: i64) => self.guest_ready(s)?,
            HostPair(i: i64) => self.host_message(i, HostMessage::Paired)?,
            HostRefuse(i: i64) => self.host_message(i, HostMessage::Refused)?,
            HostAbortAssoc(i: i64) => self.host_message(i, HostMessage::Abort)?,
            GuestRecvPaired(i: i64) => self.guest_recv_paired(i)?,
            GuestRecvEnd(i: i64) => self.guest_recv_end(i)?,
            GuestPairingTimeout(s: i64) => self.guest_pairing_timeout(s)?,
            HostRecvEnd(i: i64) => self.host_recv_end(i)?,
            SockClose(k: i64) => self.sock_close(k, true)?,
            SockCloseReportLost(k: i64) => self.sock_close(k, false)?,
            GuestReleaseSock(k: i64) => self.guest_release_sock(k)?,
            GuestAuditIdle(idle: Value) => self.guest_audit(&idle)?,
        })
    }
}

// ----- the compared state ------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct SlotView {
    owned: bool,
    sock: i64,
    dest: i64,
    fid: i64,
    attached: bool,
    parked: BTreeSet<(i64, i64)>,
}

/// The compared `udp_slots` variables, guest side.
#[derive(Debug, PartialEq, Eq)]
struct UdpSlotsState {
    slots: BTreeMap<i64, SlotView>,
    release_pending: BTreeMap<i64, bool>,
}

impl UdpSlotsState {
    fn from_vars(vars: &Qualified) -> std::result::Result<Self, ItfError> {
        let slots = quint_itf::int_map(vars.var("slots")?, |x| {
            let parked = quint_itf::set_elems(quint_itf::field(x, "parked")?)?
                .into_iter()
                .map(|t| {
                    let parts = quint_itf::tuple_elems(t)?;
                    match parts.as_slice() {
                        [k, d] => Ok((quint_itf::int(k)?, quint_itf::int(d)?)),
                        _ => Err(ItfError::Shape {
                            expected: "a (socket, destination) tuple",
                            found: t.clone(),
                        }),
                    }
                })
                .collect::<std::result::Result<BTreeSet<_>, _>>()?;
            Ok(SlotView {
                owned: quint_itf::boolean(quint_itf::field(x, "owned")?)?,
                sock: quint_itf::int(quint_itf::field(x, "sock")?)?,
                dest: quint_itf::int(quint_itf::field(x, "dest")?)?,
                fid: quint_itf::int(quint_itf::field(x, "fid")?)?,
                attached: quint_itf::boolean(quint_itf::field(x, "attached")?)?,
                parked,
            })
        })?;
        let release_pending = quint_itf::int_map(vars.var("releasePending")?, quint_itf::boolean)?;
        Ok(Self { slots, release_pending })
    }
}

impl<'de> Deserialize<'de> for UdpSlotsState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let vars = Qualified::of(&value, MAIN, MODULE).map_err(serde::de::Error::custom)?;
        Self::from_vars(&vars).map_err(serde::de::Error::custom)
    }
}

impl State<UdpSlotsDriver> for UdpSlotsState {
    /// Decodes the compared variables (by qualified name), and stashes the
    /// host messages in flight and the domains for the next step's
    /// verification.
    fn from_spec(value: Value) -> Result<Self> {
        let vars = Qualified::of(&value, MAIN, MODULE)?;
        let state = Self::from_vars(&vars)?;
        let host = HostVars::from_vars(&vars)?;
        let domains = Domains::from_vars(&vars)?;
        SPEC_HOST.with(|stash| *stash.borrow_mut() = Some(SpecStash { host, domains }));
        Ok(state)
    }

    fn from_driver(driver: &UdpSlotsDriver) -> Result<Self> {
        if !driver.process.is_alive() {
            return Err(DriverError::NoGuest.into());
        }
        let observation = driver.owner.guest_observation();
        // conformance-lint: sim-arbiter-ok QPROJ-7 guest-owner seam arbiter (slot socket queues by origin, release reports), Tier-3 R5-14 / V-EV-DGRAM
        let sim = driver.io.guest_socket_observation();
        let pre_tag =
            |(c, d): (SocketCookie, SocketAddrV4)| -> std::result::Result<(i64, i64), DriverError> {
                Ok((
                    sock_index(c)
                        .ok_or_else(|| DriverError::NoPreImage(format!("cookie {c:?}")))?,
                    dest_index(d)
                        .ok_or_else(|| DriverError::NoPreImage(format!("destination {d}")))?,
                ))
            };
        let mut slots = BTreeMap::new();
        for s in SLOTS {
            let o = observation
                .slots
                .get(&slot(s))
                .ok_or_else(|| DriverError::NoPreImage(format!("slot {}", slot(s))))?;
            let (sock, dest) = match o.owner {
                Some(tag) => pre_tag(tag)?,
                None => (0, 0),
            };
            let fid = match o.association {
                Some(flow) => driver
                    .fids
                    .iter()
                    .position(|f| *f == flow)
                    .map(|i| i64::try_from(i + 1).expect("few associations"))
                    .ok_or_else(|| DriverError::NoPreImage(format!("association {flow:?}")))?,
                None => 0,
            };
            let parked = sim
                .parked
                .get(&slot(s))
                .map(|tags| {
                    tags.iter()
                        .map(|t| pre_tag(*t))
                        .collect::<std::result::Result<BTreeSet<_>, _>>()
                })
                .transpose()?
                .unwrap_or_default();
            slots.insert(
                s,
                SlotView {
                    owned: o.owner.is_some(),
                    sock,
                    dest,
                    fid,
                    attached: o.attached.is_some(),
                    parked,
                },
            );
        }
        let release_pending =
            SOCKS.into_iter().map(|k| (k, sim.release_reports.contains(&cookie(k)))).collect();
        Ok(Self { slots, release_pending })
    }
}

// ----- the named runs (critical scenarios, `slots_conf`) ---------------------------

/// Named run `DatagramAssociationClaimsPairsAndRoutes` (`slots_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "DatagramAssociationClaimsPairsAndRoutes"
)]
#[ignore = "pending DELIVER step 5"]
fn quint_udp_slots_datagram_association_claims_pairs_and_routes() -> impl Driver {
    UdpSlotsDriver::new()
}

/// Named run `TwoSocketsClaimDistinctSlots` (`slots_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "TwoSocketsClaimDistinctSlots"
)]
#[ignore = "pending DELIVER step 5"]
fn quint_udp_slots_two_sockets_claim_distinct_slots() -> impl Driver {
    UdpSlotsDriver::new()
}

/// Named run `RefusedDiscardsParkedFramesAndOnlyNewDatagramReassociates` (`slots_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "RefusedDiscardsParkedFramesAndOnlyNewDatagramReassociates"
)]
#[ignore = "pending DELIVER step 5"]
fn quint_udp_slots_refused_discards_parked_frames_and_only_new_datagram_reassociates() -> impl Driver
{
    UdpSlotsDriver::new()
}

/// Named run `HostAbortOfLiveAssociationEndsItAndKeepsSlotOwned` (`slots_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "HostAbortOfLiveAssociationEndsItAndKeepsSlotOwned"
)]
#[ignore = "pending DELIVER step 5"]
fn quint_udp_slots_host_abort_of_live_association_ends_it_and_keeps_slot_owned() -> impl Driver {
    UdpSlotsDriver::new()
}

/// Named run `DatagramQueuedBeforeAnEndIsDiscardedWithTheQueue` (`slots_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "DatagramQueuedBeforeAnEndIsDiscardedWithTheQueue"
)]
#[ignore = "pending DELIVER step 5"]
fn quint_udp_slots_datagram_queued_before_an_end_is_discarded_with_the_queue() -> impl Driver {
    UdpSlotsDriver::new()
}

/// Named run `LatePairedForReleasedSlotIsAnsweredWithAbort` (`slots_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "LatePairedForReleasedSlotIsAnsweredWithAbort"
)]
#[ignore = "pending DELIVER step 5"]
fn quint_udp_slots_late_paired_for_released_slot_is_answered_with_abort() -> impl Driver {
    UdpSlotsDriver::new()
}

/// Named run `LostReleaseReportIsFoundByAudit` (`slots_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "LostReleaseReportIsFoundByAudit"
)]
#[ignore = "pending DELIVER step 5"]
fn quint_udp_slots_lost_release_report_is_found_by_audit() -> impl Driver {
    UdpSlotsDriver::new()
}

/// Named run `SocketCloseReleasesEverySlotItOwns` (`slots_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "SocketCloseReleasesEverySlotItOwns"
)]
#[ignore = "pending DELIVER step 5"]
fn quint_udp_slots_socket_close_releases_every_slot_it_owns() -> impl Driver {
    UdpSlotsDriver::new()
}

/// Named run `IdleSlotReleaseEndsAssociation` (`slots_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "IdleSlotReleaseEndsAssociation"
)]
#[ignore = "pending DELIVER step 5"]
fn quint_udp_slots_idle_slot_release_ends_association() -> impl Driver {
    UdpSlotsDriver::new()
}

/// Named run `AuditTickReleasesOrphansAndIdleSlotsInOneStep` (`slots_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "AuditTickReleasesOrphansAndIdleSlotsInOneStep"
)]
#[ignore = "pending DELIVER step 5"]
fn quint_udp_slots_audit_tick_releases_orphans_and_idle_slots_in_one_step() -> impl Driver {
    UdpSlotsDriver::new()
}

/// Named run `PairingDeadlineAbortsAndLatePairedIsDiscarded` (`slots_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "PairingDeadlineAbortsAndLatePairedIsDiscarded"
)]
#[ignore = "pending DELIVER step 5"]
fn quint_udp_slots_pairing_deadline_aborts_and_late_paired_is_discarded() -> impl Driver {
    UdpSlotsDriver::new()
}

/// Named run `RecycledSlotNeverDeliversPreviousOwnersFrames` (`slots_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "RecycledSlotNeverDeliversPreviousOwnersFrames"
)]
#[ignore = "pending DELIVER step 5"]
fn quint_udp_slots_recycled_slot_never_delivers_previous_owners_frames() -> impl Driver {
    UdpSlotsDriver::new()
}

// ----- randomized traces -------------------------------------------------------------

/// Randomized traces of the conformance main (seed printed on failure).
/// CONTRACT_SHAPE: bounded-change.
#[quint_run(spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt", main = "slots_conf")]
#[ignore = "pending DELIVER step 5"]
fn quint_udp_slots_slots_conf_guest() -> impl Driver {
    UdpSlotsDriver::new()
}
