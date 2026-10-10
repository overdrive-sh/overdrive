//! Quint conformance driver — `owner_flows` `flows_half_guest` (S-4;
//! HALFCLOSE-SIGNAL; QPROJ-1, QPROJ-5, QPROJ-7, DG-15).
//!
//! Replays `specs/quint/guest-flow-owner/owner_flows.qnt`, instance
//! `flows_half_guest` (one paired TCP flow; the side under test is the guest,
//! `UNDER = "g"`), against the production guest flow owner [`GuestFlowOwner`]
//! over the guest-owner seam sim [`SimGuestOwnerIo`] and [`SimGuestClock`].
//! The spec is referenced in place. The driver plays the host owner `h`, both
//! far ends and the control session (stimulus, QPROJ-1: "the other side's
//! actions are stimulus taken from the trace, and its outputs are consumed by
//! the driver as the trace's next step dictates"); it keeps a stimulus model
//! of the host's side of the flow so that the host's reactions (an
//! acknowledgement, an abort, the end of its vsock) are produced as seam
//! inputs when the trace plays them. It re-implements no rule of the guest.
//!
//! Action → driving port / seam stimulus (feature-delta § *Conformance
//! observation surface and action map*, "`owner_flows` `flows_half`"; DG-15):
//!
//! | Action at `g` (side under test) | Driver |
//! |---|---|
//! | `init(o)` | a fresh guest with a live control session and the one flow: `o = g` the application's captured `connect()` (TcpConnect, request written), `o = h` the host's TcpAccept (V_g accepted, request read, application connect in flight) |
//! | `HcFarHalfClose(g)` / `HcFarReset(g)` | seam: the application half-closes / resets its leg |
//! | `HcPair` (`g` the acceptor) | seam releases the pairing: the application connect completes |
//! | `HcAnnounce(g)` | seam reports the drain toward the host complete (`drained`) and the owner writes `HalfClose` (clock hops to the owner's own deadlines, never to `FLOW_DRAIN_DEADLINE`) |
//! | `HcRecv(g)` | seam releases the next message of the host's channel |
//! | `VsockEndArrives(g)`, `VsockEndReachesSurvivor(g)` | seam delivers the host's vsock end (V_h ended) |
//! | `HcCleanClose(g)` | seam releases the close (the owner is settled; see the module note) |
//! | `HcLossObserved(g)` | seam delivers the control session's end, deferred since the host ended it |
//! | `HcFlowError(g)` | `SimGuestClock` past `FLOW_DRAIN_DEADLINE` while a drain is pending, else a scripted error on the flow vsock (DG-15) |
//! | `HcOwnerDies(g)` | drop the owner; the seam applies linger (far end reset, vsock end, session end) |
//!
//! | Action at `h` / shared (stimulus) | Driver |
//! |---|---|
//! | `HcFarHalfClose(h)`, `HcAnnounce(h)`, `HcPair` (`h` the acceptor), `HcCleanClose(h)`, `HcLossObserved(h)`, `HcFarReset(h)`, `HcFlowError(h)`, `HcPeerDefectEnd(h)` | the host's side in the stimulus model; a message it sends is queued for `HcRecv(g)`, its vsock end for `VsockEndArrives(g)` |
//! | `HcRecv(h)` | the host consumes the guest's oldest message (seam outbox) and reacts in the stimulus model |
//! | `VsockEndArrives(h)`, `VsockEndReachesSurvivor(h)` | the host observes the guest's vsock end (seam) and reacts |
//! | `HcOwnerDies(h)` | the host dies: the guest's unconsumed messages are lost, the host's vsock end is queued, the session ends (seam, deferred until `HcLossObserved(g)`: K-B4 delivers the dead owner's in-flight messages first) |
//! | `HcSessionDrop` | the session drops: both directions' in-flight messages are lost; its end is deferred until `HcLossObserved(g)` |
//! | `HcLateAck(s)`, `HcPeerDefectEnd(g)` | hazard-only / disabled for the side under test (S-4) — an error, never a no-op |
//!
//! Projection (`State::from_driver`), side `g`: `paired`, `outEof`, `ann`,
//! `acked`, `outShut` ← O5 `guest_observation().flows[F]`
//! (`FlowEndObservation`), or `awaiting_paired[F]` (`outer_eof`) while
//! unpaired; `open` ⇔ `F` is in the owner's table (`accepted`, `flows` or
//! `awaiting_paired`); `loss` ⇔ S `session_ended` ∧ ¬O5 `session`; `ch` ← S
//! `outbox`; `vEnd`, `vEndSent` ← S `vsock_ends_in_flight`, `vsock_ended`;
//! `far` ← S `far_end`; `ctlDown` ← S `session_ended`; `dead` ← C (the
//! driver dropped the owner). Once `open(g)` is false nothing else of `g` is
//! compared (S-4: history). The host's fields, `opener`, `farHc` are
//! stimulus; `held`, `ackDue` hazard-only; `earlyShut`, `faulted` ghost.
//!
//! Identity relabelling: the model's one flow ↔ the `FlowId` of that flow
//! (`o = g`: the guest-origin id of the request the owner wrote, fixed at
//! `init`; `o = h`: the host-origin id the driver opened); side `g` ↔ the
//! owner under test.
//!
//! **The stimulus model is verified against the spec.** The host side's
//! variables the stimulus model tracks — `opener`, and of side `h`: `paired`,
//! `outEof`, `ann`, `acked`, `open`, `ch`, `vEnd`, `vEndSent`, plus `ctlDown` —
//! are stimulus (QPROJ-1), but the driver's own copy of them decides its next
//! seam input, so it is checked: `State::from_spec` decodes them from each
//! step's ITF post-state and stashes them (a thread-local this module owns);
//! at the start of the next step the driver compares its model with the stash
//! and fails with [`DriverError::PeerDiverged`] on any difference. The
//! stimulus is therefore driven from a pre-state equal to the spec's. The
//! stash verifies stimulus; it never feeds `State` — `from_driver` reads only
//! O5 and the marked sim arbiter. The same comparison checks the instance's
//! domain (`SIDES` = {`g`, `h`}) and that a state labelled `step` is the
//! instance's initial state.
//!
//! **`HcCleanClose(g)` and UI-R2-5 (c).** The driver maps `HcCleanClose(g)` to
//! a settle: production closes the flow in the same dispatch that completes
//! its second half-close (§ *Flow end*), so the model's separate clean-close
//! step has no production input of its own. The model, by contrast, keeps the
//! flow open at `g` until `HcCleanClose(g)`, so at `VsockEndArrives(g)` (the
//! step that completes the second half-close) the driver projects the flow
//! closed where the spec still has it open. The acceptance test
//! `half_close::a_clean_close_clears_linger_before_closing` encodes the
//! § *Flow end* text (close in that dispatch) and is not changed; the driver
//! stays ignored until the model decision (UI-R2-5) settles which is right.
//!
//! The spec defines no Quint `run` declarations, so there is no named run for
//! `#[quint_test]`; the instance replays randomized traces through
//! `#[quint_run]` (the seed is printed on failure).

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::struct_excessive_bools,
    reason = "a conformance driver asserts its own preconditions; protocol names are prose"
)]

use std::cell::RefCell;
use std::collections::{BTreeSet, VecDeque};
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use itf::Value;
use overdrive_core::guest_transport::LaunchToken;
use overdrive_core::guest_transport::wire::{
    ControlMessage, FLOW_DRAIN_DEADLINE, FlowId, FlowOrigin, FlowRequest,
};
use overdrive_core::net::ListenPort;
use quint_connect::{Driver, Result, State, Step, quint_run, switch};
use serde::Deserialize;

use super::quint_itf::{self, ItfError};
use super::sim::{
    FarEndEvent, SimAppEnd, SimAppTarget, SimGuestClock, SimGuestEffect, SimGuestOwnerIo,
    SimSessionEnd, sim_cookie,
};
use super::test_support::{GuestProcess, settle};
use super::{
    GuestClock, GuestFlowDirection, GuestFlowOwner, HalfCloseAnnounce, HostChannel, SocketCookie,
};
use crate::transport::GuestTransportToken;

const WL: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 7);
const GW: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
const LAUNCH_HEX: &str = "0123456789abcdef0123456789abcdef";
/// The far end of a TcpConnect flow (`o = g`).
const DESTINATION: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(93, 0, 0, 1), 443);
/// The intake client of a TcpAccept flow (`o = h`).
const CLIENT: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 7), 40_000);
const ACCEPT_PORT: &str = "8080";
const APP_COOKIE: u64 = 1;

/// Why a driver step cannot proceed.
#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("action `{action}` is not enabled in the production state: {why}")]
    NotEnabled { action: &'static str, why: String },
    #[error("action `{0}` is hazard-only or disabled for the side under test (S-4)")]
    HazardOnly(&'static str),
    #[error("the driver has no flow yet (no `init`)")]
    NoInit,
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

// ----- the spec's host variables (stimulus verification, never projection) ----

/// The host-side variables the stimulus model tracks, in the spec's
/// vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HostVars {
    opener: String,
    paired: bool,
    out_eof: bool,
    ann: String,
    acked: bool,
    open: bool,
    ch: Vec<String>,
    v_end: bool,
    v_end_sent: bool,
    ctl_down: bool,
}

/// One step's spec post-state, as the next step's driver checks it.
struct SpecStash {
    host: HostVars,
    /// The keys of the spec's per-side maps (`SIDES`).
    sides: BTreeSet<String>,
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
    fn from_value(value: &Value) -> std::result::Result<(Self, BTreeSet<String>), ItfError> {
        let vars = quint_itf::vars(value)?;
        let hc = quint_itf::var(&vars, "hc")?;
        let h = |name: &str| -> std::result::Result<Value, ItfError> {
            quint_itf::map_entries(quint_itf::field(hc, name)?)?
                .into_iter()
                .find(|(k, _)| matches!(k, Value::String(side) if side == "h"))
                .map(|(_, v)| v.clone())
                .ok_or_else(|| ItfError::MissingField(format!("{name}(h)")))
        };
        let sides = quint_itf::map_entries(quint_itf::field(hc, "open")?)?
            .into_iter()
            .map(|(k, _)| quint_itf::string(k))
            .collect::<std::result::Result<_, _>>()?;
        let host = Self {
            opener: quint_itf::string(quint_itf::field(hc, "opener")?)?,
            paired: quint_itf::boolean(&h("paired")?)?,
            out_eof: quint_itf::boolean(&h("outEof")?)?,
            ann: quint_itf::string(&h("ann")?)?,
            acked: quint_itf::boolean(&h("acked")?)?,
            open: quint_itf::boolean(&h("open")?)?,
            ch: quint_itf::list_elems(&h("ch")?)?
                .into_iter()
                .map(quint_itf::string)
                .collect::<std::result::Result<_, _>>()?,
            v_end: quint_itf::boolean(&h("vEnd")?)?,
            v_end_sent: quint_itf::boolean(&h("vEndSent")?)?,
            ctl_down: quint_itf::boolean(quint_itf::field(hc, "ctlDown")?)?,
        };
        Ok((host, sides))
    }

    /// The spec's `init` for this opener.
    fn initial(&self) -> Self {
        Self {
            opener: self.opener.clone(),
            paired: false,
            out_eof: false,
            ann: "none".to_owned(),
            acked: false,
            open: true,
            ch: Vec::new(),
            v_end: false,
            v_end_sent: false,
            ctl_down: false,
        }
    }
}

/// A control message of the one flow, by the model's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Msg {
    Paired,
    HalfClose,
    Ack,
    Abort,
}

impl Msg {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Paired => "Paired",
            Self::HalfClose => "HalfClose",
            Self::Ack => "Ack",
            Self::Abort => "Abort",
        }
    }

    const fn wire(self, flow: FlowId) -> ControlMessage {
        match self {
            Self::Paired => ControlMessage::Paired { flow },
            Self::HalfClose => ControlMessage::HalfClose { flow },
            Self::Ack => ControlMessage::HalfCloseAck { flow },
            Self::Abort => ControlMessage::Abort { flow },
        }
    }

    const fn of(message: ControlMessage) -> Option<(FlowId, Self)> {
        match message {
            ControlMessage::Paired { flow } => Some((flow, Self::Paired)),
            ControlMessage::HalfClose { flow } => Some((flow, Self::HalfClose)),
            ControlMessage::HalfCloseAck { flow } => Some((flow, Self::Ack)),
            ControlMessage::Abort { flow } => Some((flow, Self::Abort)),
            _ => None,
        }
    }
}

/// The host's side of the flow, as the driver plays it (stimulus model):
/// what the host's later seam inputs (its messages, its vsock end) depend on.
#[derive(Debug, Clone)]
struct HostSide {
    /// The host considers the flow paired (it may announce).
    paired: bool,
    /// The host's outer leg read EOF (it may announce).
    out_eof: bool,
    /// The host's announcement: `none`, `pending` (`HalfClose` sent, awaiting
    /// the acknowledgement) or `done` (acknowledged; vsock shut down).
    ann: &'static str,
    /// The host acknowledged the guest's `HalfClose`.
    acked: bool,
    /// The flow is open at the host.
    open: bool,
    /// Messages the host sent and the guest has not received (FIFO).
    ch: VecDeque<Msg>,
    /// The host's vsock end, in flight toward the guest.
    v_end: bool,
    v_end_sent: bool,
}

impl HostSide {
    const fn new() -> Self {
        Self {
            paired: false,
            out_eof: false,
            ann: "none",
            acked: false,
            open: true,
            ch: VecDeque::new(),
            v_end: false,
            v_end_sent: false,
        }
    }
}

/// The one flow under replay.
struct Flow {
    /// Who opened it (`g` TcpConnect, `h` TcpAccept).
    opener_guest: bool,
    id: FlowId,
    /// The application leg the far-end stimuli act on.
    app: SimAppTarget,
    /// When the guest's outer leg read EOF (the drain deadline's origin).
    eof_at: Option<Instant>,
}

struct FlowsHalfDriver {
    /// The owner under test (`Dead` once the driver dropped it: `dead(g)`).
    owner: GuestProcess,
    /// The driver's clone of the owner's seam (the QPROJ-7 arbiter).
    io: SimGuestOwnerIo,
    clock: Arc<SimGuestClock>,
    flow: Option<Flow>,
    host: HostSide,
    /// The control session ended (stimulus: the faults the driver injected).
    ctl_down: bool,
    /// A state labelled `step` was taken as the initial state; the next step
    /// checks that it was.
    step_as_init: bool,
}

fn not_enabled(action: &'static str, why: &'static str) -> impl FnOnce() -> DriverError {
    move || DriverError::NotEnabled { action, why: why.to_owned() }
}

impl FlowsHalfDriver {
    fn new() -> Self {
        let clock = Arc::new(SimGuestClock::new());
        Self {
            owner: GuestProcess::NotStarted,
            io: SimGuestOwnerIo::new(WL, Arc::clone(&clock)),
            clock,
            flow: None,
            host: HostSide::new(),
            ctl_down: false,
            step_as_init: false,
        }
    }

    /// A driver for a new run: no stash of an earlier run is compared.
    fn fresh() -> Self {
        SPEC_HOST.with(|stash| stash.borrow_mut().take());
        Self::new()
    }

    // ----- stimulus verification ------------------------------------------------------

    /// The stimulus model in the spec's vocabulary.
    fn host_vars(&self) -> HostVars {
        let opener = match self.flow.as_ref().map(|f| f.opener_guest) {
            Some(true) => "g",
            Some(false) => "h",
            None => "",
        };
        HostVars {
            opener: opener.to_owned(),
            paired: self.host.paired,
            out_eof: self.host.out_eof,
            ann: self.host.ann.to_owned(),
            acked: self.host.acked,
            open: self.host.open,
            ch: self.host.ch.iter().map(|m| m.as_str().to_owned()).collect(),
            v_end: self.host.v_end,
            v_end_sent: self.host.v_end_sent,
            ctl_down: self.ctl_down,
        }
    }

    /// Compares the stimulus model with the spec's post-state of the previous
    /// step (stashed by `State::from_spec`): the instance's domain, the
    /// `step`-labelled initial state, then every host variable the model
    /// tracks. Nothing to compare before the run's first step.
    fn verify_stimulus(&mut self) -> std::result::Result<(), DriverError> {
        let Some(spec) = SPEC_HOST.with(|stash| stash.borrow_mut().take()) else {
            return Ok(());
        };
        let sides: BTreeSet<String> = ["g", "h"].into_iter().map(str::to_owned).collect();
        if spec.sides != sides {
            return Err(DriverError::DomainMismatch(format!(
                "the spec's SIDES is {:?}, the driver assumes {sides:?}",
                spec.sides
            )));
        }
        if std::mem::take(&mut self.step_as_init) && !spec.initial {
            return Err(DriverError::StepNotInitial);
        }
        let model = self.host_vars();
        if model != spec.host {
            return Err(DriverError::PeerDiverged {
                spec: format!("{:?}", spec.host),
                driver: format!("{model:?}"),
            });
        }
        Ok(())
    }

    fn flow(&self) -> std::result::Result<&Flow, DriverError> {
        self.flow.as_ref().ok_or(DriverError::NoInit)
    }

    /// Settles the owner under test, if it is alive.
    fn settle(&mut self) {
        if let Some(owner) = self.owner.owner_mut() {
            settle(owner, &self.io);
        }
    }

    fn live_owner(
        &self,
        action: &'static str,
    ) -> std::result::Result<&GuestFlowOwner, DriverError> {
        self.owner.owner().ok_or_else(not_enabled(action, "the guest owner is dead"))
    }

    // ----- init -------------------------------------------------------------------

    /// `init(o)`: a fresh guest with a live session and the one flow.
    fn init(&mut self, opener: &str) -> std::result::Result<(), DriverError> {
        let clock = Arc::new(SimGuestClock::new());
        let io = SimGuestOwnerIo::new(WL, Arc::clone(&clock));
        let launch = LaunchToken::from_str(LAUNCH_HEX).expect("fixture launch token");
        let token = GuestTransportToken { workload_addr: WL, gateway: GW, launch };
        let owner_clock: Arc<dyn GuestClock> = clock.clone();
        let mut owner = GuestFlowOwner::new(token, Box::new(io.clone()), owner_clock);
        settle(&mut owner, &io);
        io.complete_host_connect(HostChannel::Control, Ok(()));
        settle(&mut owner, &io);
        let flow = if opener == "g" {
            let cookie: SocketCookie = sim_cookie(APP_COOKIE);
            io.app_connect(cookie, DESTINATION, true);
            settle(&mut owner, &io);
            io.complete_host_connect(HostChannel::FlowStream, Ok(()));
            settle(&mut owner, &io);
            let id = io
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
                .ok_or_else(not_enabled("init", "the guest wrote no TcpConnect request"))?;
            Flow { opener_guest: true, id, app: SimAppTarget::Captured(cookie), eof_at: None }
        } else {
            let id = FlowId::new(FlowOrigin::Host, 1).expect("seq < 2^31");
            let listen_port = ListenPort::from_str(ACCEPT_PORT).expect("non-zero listen port");
            io.host_open_accept(FlowRequest::TcpAccept {
                flow: id,
                client: CLIENT,
                listen_port,
                launch,
            });
            io.release_inbound_accept(id);
            settle(&mut owner, &io);
            io.release_inbound_request(id);
            settle(&mut owner, &io);
            Flow { opener_guest: false, id, app: SimAppTarget::Flow(id), eof_at: None }
        };
        // a drain toward the host completes only when the trace says so
        io.set_drained(flow.id, GuestFlowDirection::TowardHost, false);
        io.clear_effects();
        self.owner = GuestProcess::Alive(owner);
        self.io = io;
        self.clock = clock;
        self.flow = Some(flow);
        self.host = HostSide::new();
        self.ctl_down = false;
        self.step_as_init = false;
        Ok(())
    }

    /// A state labelled `step` (quint 0.32 labels the initial state of some
    /// traces so): taken as the initial state, which the next step verifies.
    fn initial_step(&mut self, opener: &str) -> std::result::Result<(), DriverError> {
        self.init(opener)?;
        self.step_as_init = true;
        Ok(())
    }

    // ----- the host's side (stimulus model) ------------------------------------------

    /// The host sends `m` on the session (lost once the session has ended).
    fn host_send(&mut self, m: Msg) {
        if !self.ctl_down {
            self.host.ch.push_back(m);
        }
    }

    /// The host's vsock ends (shutdown, abort or death).
    const fn host_end_vsock(&mut self) {
        if !self.host.v_end_sent {
            self.host.v_end = true;
            self.host.v_end_sent = true;
        }
    }

    /// The host aborts its side with reset semantics; `Abort` when `msg`.
    fn host_abort(&mut self, msg: bool) {
        self.host.open = false;
        if msg {
            self.host_send(Msg::Abort);
        }
        self.host_end_vsock();
    }

    /// The session ends at the seam, its end deferred until
    /// `HcLossObserved(g)`, unless the guest is dead or the session already
    /// ended there.
    fn defer_session_end(&self) {
        let io = &self.io;
        if self.owner.owner().is_some()
            && io.session_socket().is_some()
            && !io.session_end_pending()
        {
            io.host_end_session_deferred(SimSessionEnd::Reset);
        }
    }

    // ----- actions --------------------------------------------------------------------

    /// `HcFarHalfClose(s)`.
    fn far_half_close(&mut self, s: &str) -> std::result::Result<(), DriverError> {
        if s == "h" {
            self.host.out_eof = true;
            return Ok(());
        }
        self.live_owner("HcFarHalfClose")?;
        let now = self.clock.now();
        let app = self.flow()?.app;
        self.io.app_end(app, SimAppEnd::HalfClose);
        self.settle();
        if let Some(flow) = self.flow.as_mut() {
            flow.eof_at = Some(now);
        }
        Ok(())
    }

    /// `HcPair`: the acceptor pairs and sends `Paired`.
    fn pair(&mut self) -> std::result::Result<(), DriverError> {
        let flow = self.flow()?;
        if flow.opener_guest {
            self.host.paired = true;
            self.host_send(Msg::Paired);
            return Ok(());
        }
        self.live_owner("HcPair")?;
        let id = flow.id;
        self.io.complete_app_connect(id, Ok(()));
        self.settle();
        Ok(())
    }

    /// `HcAnnounce(s)`.
    fn announce(&mut self, s: &str) -> std::result::Result<(), DriverError> {
        if s == "h" {
            if !(self.host.paired && self.host.out_eof) {
                return Err(DriverError::NotEnabled {
                    action: "HcAnnounce",
                    why: "the host has no paired flow whose outer leg read EOF".to_owned(),
                });
            }
            self.host.ann = "pending";
            self.host_send(Msg::HalfClose);
            return Ok(());
        }
        self.live_owner("HcAnnounce")?;
        let (id, eof_at) = {
            let flow = self.flow()?;
            (
                flow.id,
                flow.eof_at
                    .ok_or_else(not_enabled("HcAnnounce", "the guest's outer leg read no EOF"))?,
            )
        };
        self.io.set_drained(id, GuestFlowDirection::TowardHost, true);
        self.settle();
        let deadline = eof_at + FLOW_DRAIN_DEADLINE;
        while !self.announced(id) {
            let next = self.io.armed_deadline();
            match next {
                Some(at) if at < deadline => {
                    let now = self.clock.now();
                    if at > now {
                        self.clock.advance(at - now);
                    }
                    self.settle();
                }
                _ => {
                    return Err(DriverError::NotEnabled {
                        action: "HcAnnounce",
                        why: "the owner re-checks the drain at no deadline before FLOW_DRAIN_DEADLINE".to_owned(),
                    });
                }
            }
        }
        Ok(())
    }

    fn announced(&self, id: FlowId) -> bool {
        self.owner
            .owner()
            .and_then(|o| o.guest_observation().flows.get(&id).copied())
            .is_some_and(|f| f.announce != HalfCloseAnnounce::None)
    }

    /// `HcRecv(s)`: the next message from the other side is delivered at `s`.
    fn recv(&mut self, s: &str) -> std::result::Result<(), DriverError> {
        let id = self.flow()?.id;
        if s == "g" {
            self.live_owner("HcRecv")?;
            let m = self
                .host
                .ch
                .pop_front()
                .ok_or_else(not_enabled("HcRecv", "no host message in flight"))?;
            self.io.host_control(m.wire(id));
            self.settle();
            return Ok(());
        }
        let message = self
            .io
            .host_take_next()
            .ok_or_else(not_enabled("HcRecv", "no guest message in flight"))?;
        let (flow, m) = Msg::of(message).ok_or_else(|| DriverError::NotEnabled {
            action: "HcRecv",
            why: format!("the guest wrote {message:?}, which the model has no name for"),
        })?;
        if flow != id {
            return Err(DriverError::NotEnabled {
                action: "HcRecv",
                why: format!("{message:?} names another flow"),
            });
        }
        if !self.host.open {
            return Ok(()); // U-5: a closed flow's message is discarded
        }
        match m {
            Msg::Paired => self.host.paired = true,
            Msg::HalfClose => {
                self.host.acked = true;
                self.host_send(Msg::Ack);
            }
            Msg::Ack => {
                if self.host.ann == "pending" {
                    self.host.ann = "done";
                    self.host_end_vsock();
                }
            }
            Msg::Abort => self.host_abort(false),
        }
        Ok(())
    }

    /// `VsockEndArrives(s)` / `VsockEndReachesSurvivor(s)`.
    fn vsock_end(&mut self, s: &str, action: &'static str) -> std::result::Result<(), DriverError> {
        let id = self.flow()?.id;
        if s == "g" {
            self.live_owner(action)?;
            if !self.host.v_end {
                return Err(DriverError::NotEnabled {
                    action,
                    why: "no host vsock end in flight".to_owned(),
                });
            }
            self.host.v_end = false;
            self.io.host_vsock_end(id);
            self.settle();
            return Ok(());
        }
        let io = &self.io;
        if !io.guest_socket_observation().vsock_ends_in_flight.contains(&id) {
            return Err(DriverError::NotEnabled {
                action,
                why: "no guest vsock end in flight".to_owned(),
            });
        }
        io.host_observe_vsock_end(id);
        // announced: the host shuts its outer leg; otherwise it aborts
        if self.host.open && !self.host.acked {
            self.host_abort(true);
        }
        Ok(())
    }

    /// `HcCleanClose(s)`.
    fn clean_close(&mut self, s: &str) -> std::result::Result<(), DriverError> {
        if s == "h" {
            self.host.open = false;
            return Ok(());
        }
        self.live_owner("HcCleanClose")?;
        self.settle();
        Ok(())
    }

    /// `HcLossObserved(s)`.
    fn loss_observed(&mut self, s: &str) -> std::result::Result<(), DriverError> {
        if s == "h" {
            if self.host.open {
                self.host_abort(false);
            }
            return Ok(());
        }
        self.live_owner("HcLossObserved")?;
        let io = &self.io;
        if !io.session_end_pending() {
            return Err(DriverError::NotEnabled {
                action: "HcLossObserved",
                why: "the seam holds no session end for the guest".to_owned(),
            });
        }
        io.release_session_end();
        self.settle();
        Ok(())
    }

    /// `HcFarReset(s)`.
    fn far_reset(&mut self, s: &str) -> std::result::Result<(), DriverError> {
        if s == "h" {
            self.host_abort(true);
            return Ok(());
        }
        self.live_owner("HcFarReset")?;
        let app = self.flow()?.app;
        self.io.app_end(app, SimAppEnd::Reset);
        self.settle();
        Ok(())
    }

    /// `HcFlowError(s)`: the drain deadline passes while a drain is pending,
    /// else a scripted error on the flow vsock.
    fn flow_error(&mut self, s: &str) -> std::result::Result<(), DriverError> {
        if s == "h" {
            self.host_abort(true);
            return Ok(());
        }
        let (id, eof_at) = {
            let flow = self.flow()?;
            (flow.id, flow.eof_at)
        };
        let drain_pending = self
            .live_owner("HcFlowError")?
            .guest_observation()
            .flows
            .get(&id)
            .is_some_and(|f| f.outer_eof && f.announce == HalfCloseAnnounce::None);
        match eof_at {
            Some(eof_at) if drain_pending => {
                let at = eof_at + FLOW_DRAIN_DEADLINE;
                let now = self.clock.now();
                if at > now {
                    self.clock.advance(at - now);
                }
            }
            _ => self.io.flow_vsock_error(id, ErrorKind::ConnectionReset),
        }
        self.settle();
        Ok(())
    }

    /// `HcOwnerDies(s)`.
    fn owner_dies(&mut self, s: &str) -> std::result::Result<(), DriverError> {
        if s == "g" {
            self.live_owner("HcOwnerDies")?;
            self.io.guest_owner_dies();
            self.owner = GuestProcess::Dead;
            self.ctl_down = true;
            self.host.ch.clear(); // messages toward the dead guest are lost
            return Ok(());
        }
        self.host.open = false;
        self.ctl_down = true;
        // the guest's unconsumed messages never reach the dead host
        let _lost = self.io.take_outbox();
        self.host_end_vsock();
        self.defer_session_end();
        Ok(())
    }

    /// `HcSessionDrop`: the session drops; its in-flight messages are lost.
    fn session_drop(&mut self) {
        self.ctl_down = true;
        self.host.ch.clear();
        let _lost = self.io.take_outbox();
        self.defer_session_end();
    }

    /// `HcPeerDefectEnd(h)`: the host ends its vsock without the handshake.
    fn peer_defect_end(&mut self, s: &str) -> std::result::Result<(), DriverError> {
        if s != "h" {
            return Err(DriverError::HazardOnly("HcPeerDefectEnd(g)"));
        }
        self.host_end_vsock();
        Ok(())
    }
}

impl Driver for FlowsHalfDriver {
    type State = HalfGuestState;

    fn step(&mut self, step: &Step) -> Result {
        self.verify_stimulus()?;
        switch!(step {
            init(o: String) => self.init(&o)?,
            // quint 0.32 `run --n-traces` labels the initial state of some
            // traces `step` instead of `init` (observed at index 0, with the
            // `init` state's values and its `o` pick); it is taken as the
            // initial state, and the next step fails with `StepNotInitial`
            // unless it was.
            step(o: String) => self.initial_step(&o)?,
            HcFarHalfClose(s: String) => self.far_half_close(&s)?,
            HcPair => self.pair()?,
            HcAnnounce(s: String) => self.announce(&s)?,
            HcRecv(s: String) => self.recv(&s)?,
            HcLateAck => return Err(DriverError::HazardOnly("HcLateAck").into()),
            VsockEndArrives(s: String) => self.vsock_end(&s, "VsockEndArrives")?,
            VsockEndReachesSurvivor(s: String) => self.vsock_end(&s, "VsockEndReachesSurvivor")?,
            HcCleanClose(s: String) => self.clean_close(&s)?,
            HcLossObserved(s: String) => self.loss_observed(&s)?,
            HcFarReset(s: String) => self.far_reset(&s)?,
            HcFlowError(s: String) => self.flow_error(&s)?,
            HcOwnerDies(s: String) => self.owner_dies(&s)?,
            HcSessionDrop => self.session_drop(),
            HcPeerDefectEnd(s: String) => self.peer_defect_end(&s)?,
        })
    }
}

// ----- the compared state ------------------------------------------------------

/// Side `g`'s fields compared while its flow is open.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenSide {
    paired: bool,
    out_eof: bool,
    ann: String,
    acked: bool,
    out_shut: bool,
    loss: bool,
    ch: Vec<String>,
    v_end: bool,
    v_end_sent: bool,
    far: String,
}

/// The compared `hc` variables of side `g`, and `ctlDown`.
#[derive(Debug, PartialEq, Eq)]
struct HalfGuestState {
    ctl_down: bool,
    dead: bool,
    open: bool,
    /// `None` once `open(g)` is false (S-4: history, not compared).
    side: Option<OpenSide>,
}

/// Why the production state has no model image.
#[derive(Debug, thiserror::Error)]
enum ProjectionError {
    #[error("{0} has no pre-image in the model")]
    NoPreImage(String),
}

impl HalfGuestState {
    fn from_value(value: &Value) -> std::result::Result<Self, ItfError> {
        let vars = quint_itf::vars(value)?;
        let hc = quint_itf::var(&vars, "hc")?;
        let g = |name: &str| -> std::result::Result<Value, ItfError> {
            quint_itf::map_entries(quint_itf::field(hc, name)?)?
                .into_iter()
                .find(|(k, _)| matches!(k, Value::String(side) if side == "g"))
                .map(|(_, v)| v.clone())
                .ok_or_else(|| ItfError::MissingField(format!("{name}(g)")))
        };
        let open = quint_itf::boolean(&g("open")?)?;
        let side = if open {
            Some(OpenSide {
                paired: quint_itf::boolean(&g("paired")?)?,
                out_eof: quint_itf::boolean(&g("outEof")?)?,
                ann: quint_itf::string(&g("ann")?)?,
                acked: quint_itf::boolean(&g("acked")?)?,
                out_shut: quint_itf::boolean(&g("outShut")?)?,
                loss: quint_itf::boolean(&g("loss")?)?,
                ch: quint_itf::list_elems(&g("ch")?)?
                    .into_iter()
                    .map(quint_itf::string)
                    .collect::<std::result::Result<_, _>>()?,
                v_end: quint_itf::boolean(&g("vEnd")?)?,
                v_end_sent: quint_itf::boolean(&g("vEndSent")?)?,
                far: quint_itf::string(&g("far")?)?,
            })
        } else {
            None
        };
        Ok(Self {
            ctl_down: quint_itf::boolean(quint_itf::field(hc, "ctlDown")?)?,
            dead: quint_itf::boolean(&g("dead")?)?,
            open,
            side,
        })
    }
}

impl<'de> Deserialize<'de> for HalfGuestState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

impl State<FlowsHalfDriver> for HalfGuestState {
    /// Decodes the compared variables, and stashes the host side's variables
    /// for the next step's stimulus verification (never a projection source).
    fn from_spec(value: Value) -> Result<Self> {
        let state = Self::from_value(&value)?;
        let (host, sides) = HostVars::from_value(&value)?;
        let initial_side = OpenSide {
            paired: false,
            out_eof: false,
            ann: "none".to_owned(),
            acked: false,
            out_shut: false,
            loss: false,
            ch: Vec::new(),
            v_end: false,
            v_end_sent: false,
            far: "none".to_owned(),
        };
        let initial = host == host.initial()
            && state
                == (Self { ctl_down: false, dead: false, open: true, side: Some(initial_side) });
        SPEC_HOST.with(|stash| *stash.borrow_mut() = Some(SpecStash { host, sides, initial }));
        Ok(state)
    }

    fn from_driver(driver: &FlowsHalfDriver) -> Result<Self> {
        let id = driver.flow()?.id;
        // conformance-lint: sim-arbiter-ok QPROJ-7 guest-owner seam arbiter (outbox, session end, vsock ends, far ends), Tier-3 R5-10 / R5-45 / R5-28
        let sim = driver.io.guest_socket_observation();
        let ctl_down = sim.session_ended;
        let Some(owner) = driver.owner.owner() else {
            return Ok(Self { ctl_down, dead: driver.owner.is_dead(), open: false, side: None });
        };
        let obs = owner.guest_observation();
        let open = obs.accepted.contains_key(&id)
            || obs.flows.contains_key(&id)
            || obs.awaiting_paired.contains_key(&id);
        if !open {
            return Ok(Self { ctl_down, dead: false, open, side: None });
        }
        let end = obs.flows.get(&id).copied();
        let ann = match end.map(|f| f.announce) {
            None | Some(HalfCloseAnnounce::None) => "none",
            Some(HalfCloseAnnounce::Pending) => "pending",
            Some(HalfCloseAnnounce::Done) => "done",
        };
        let mut ch = Vec::with_capacity(sim.outbox.len());
        for message in &sim.outbox {
            match Msg::of(*message) {
                Some((flow, m)) if flow == id => ch.push(m.as_str().to_owned()),
                _ => {
                    return Err(
                        ProjectionError::NoPreImage(format!("guest message {message:?}")).into()
                    );
                }
            }
        }
        let far = match sim.far_end.get(&id) {
            None => "none",
            Some(FarEndEvent::Eof) => "eof",
            Some(FarEndEvent::Reset) => "reset",
        };
        Ok(Self {
            ctl_down,
            dead: false,
            open,
            side: Some(OpenSide {
                paired: end.is_some(),
                out_eof: end.map_or_else(
                    || obs.awaiting_paired.get(&id).copied().unwrap_or(false),
                    |f| f.outer_eof,
                ),
                ann: ann.to_owned(),
                acked: end.is_some_and(|f| f.peer_announced),
                out_shut: end.is_some_and(|f| f.outer_shut),
                loss: sim.session_ended && !obs.session,
                ch,
                v_end: sim.vsock_ends_in_flight.contains(&id),
                v_end_sent: sim.vsock_ended.contains(&id),
                far: far.to_owned(),
            }),
        })
    }
}

// ----- the run -----------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[quint_run(spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt", main = "flows_half_guest")]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init); model fidelity open, upstream UI-R2-5"]
fn quint_owner_flows_half_guest() -> impl Driver {
    FlowsHalfDriver::fresh()
}
