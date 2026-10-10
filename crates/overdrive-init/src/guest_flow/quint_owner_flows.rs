//! Quint conformance driver — `owner_flows` `flows_ok`, guest side (the
//! TcpAccept half `gf`; QPROJ-1, QPROJ-5, QPROJ-7, DG-15).
//!
//! Replays `specs/quint/guest-flow-owner/owner_flows.qnt`, instance
//! `flows_ok`, against the production guest flow owner [`GuestFlowOwner`] of
//! each launch, over the guest-owner seam sim [`SimGuestOwnerIo`] and
//! [`SimGuestClock`]. The spec is referenced in place. Every host variable is
//! stimulus (QPROJ-1): the driver plays the host owner, the allocation
//! lifecycle, the VM and the applications. It keeps a stimulus model of the
//! host's side — the CID's lease, the control-session connections, the
//! guest-opened flows, the host-opened flows and the host's in-flight `Abort`s
//! — because the host's next seam input depends on it (whether the host
//! accepts or closes a session, whether a flow is admitted, which flows a
//! teardown closes); it re-implements no rule of the guest.
//!
//! Action → driving port / seam stimulus (feature-delta § *Conformance
//! observation surface and action map*, "`owner_flows` `flows_ok`, guest
//! side", with the host-side map read from the guest's seam; DG-15):
//!
//! | Action | Driver |
//! |---|---|
//! | `init` | no lease, no VM, no guest |
//! | `Provision`, `Activate`, `Release`, `Restore` | stimulus: the host's CID lease / admission latch (stimulus model; no guest seam effect) |
//! | `VmStart` | a new launch: a fresh seam and `GuestFlowOwner::new` with the lease's launch token; the owner issues its control connect |
//! | `VmStop` | the VM dies: the seam ends every socket of the guest (`guest_owner_dies`) and the driver drops the owner |
//! | `Teardown`, `Quiesce`, `HostSessionLoss(c)` | the host closes what the model closes: the host's open flows' V_h (seam: vsock end at the guest), the guest's unconsumed messages discarded, and — for a live session — the session's end, held until the guest's `GuestLossAbort` / `GuestOpenSession` |
//! | `GuestOpenSession(c)` | the guest's control connect is in flight (after its loss is delivered and `CONTROL_RECONNECT_BACKOFF` passes, when it lost its previous session) |
//! | `HostAcceptSession(c)`, `HostReadOpen(c)` | seam completes the guest's control connect; a session the host closes (CID not accepting, stale token, held claim) ends at the guest |
//! | `GuestOpenFlow(f)` | seam: the application's `connect()` is captured; the guest issues its flow connect |
//! | `HostAcceptFlow(f)`, `HostReadReq(f)`, `HostConnectDone(f)`, `HostFlowEnds(f)` | seam completes the guest's flow connect / delivers the host's `Paired`, `Refused` or `Abort` / ends V_h, as the host's step does; `HostInstall`, `HostStartConnect`, `HostConnectDiscard` are host-internal (stimulus model only) |
//! | `HostOpenAccept(h)` | seam queues V_g with its `TcpAccept` request (the lease's launch token) |
//! | `GuestAcceptV(h)` | seam releases the accept and the request |
//! | `GuestPairOrRefuse(h, ok)` | seam completes the application connect (accepted / refused) |
//! | `HostRecvPaired(h)`, `HostRecvRefused(h)` | the host consumes the guest's `Paired` / `Refused` |
//! | `HostAbortAccept(h)` | stimulus: the host closes V_h and, while a session lives, sends `Abort` — both reach the guest later (`GuestRecvAbort`, `GuestSeesHostClose`, `GuestQueueReset`) |
//! | `GuestRecvAbort(h)` | seam releases the host's `Abort` |
//! | `GuestSeesHostClose(h)` / `GuestQueueReset(h)` | seam delivers the host's close of V_h (K-A4) after / before the guest's accept |
//! | `GuestLossAbort(h)` | seam delivers the end of the session the host ended |
//! | `HostClaimSession`, `HostCheckFlow`, `HostRegisterFlow` | hazard-only — an error, never a no-op |
//!
//! Projection (`State::from_driver`): `gf(h).st` ← O5
//! `guest_observation().accepted` (`Accepted` → `acc`, `Routed` → `routed`);
//! `reset` ⇔ S `reset_in_queue` of the live launch; `closed` ⇔ the flow has
//! been held by the guest (seen in `accepted` or `reset_in_queue`) and no
//! longer is; `toHostPaired`, `toHostRefused` ← S `outbox` of every launch
//! (messages the guest wrote and the host has not consumed outlive the VM). S
//! = `SimGuestOwnerIo::guest_socket_observation` (QPROJ-7). The memory "the
//! guest has held this flow" is the relabelling's record of the production
//! identities it has seen, kept so that `closed` (a history fact the owner's
//! table cannot hold once the flow is gone) has a pre-image; it never supplies
//! a phase. Every other variable is stimulus, ghost or constant (README S-1).
//!
//! Identity relabelling: host flow `h` ↔ `FlowId::new(Host, h)`; guest flow
//! `f` ↔ the guest-origin `FlowId` of the request the owner wrote for the
//! application connect the driver issued for `f`; connection `c` ↔ the
//! guest's control connect the driver attributed to it; lease `gen` ↔ launch
//! token `gen`.
//!
//! **The stimulus model is verified against the spec.** The host variables the
//! stimulus model tracks — `alloc`, `gen`, `vmGen`, `vmUp`, `quiesced`,
//! `claim`, `sess`, `flows`, `hf`, `toGuestAbort` — are stimulus (QPROJ-1),
//! but the driver's own copy of them decides its next seam input, so it is
//! checked: `State::from_spec` decodes them from each step's ITF post-state and
//! stashes them (a thread-local this module owns); at the start of the next
//! step the driver compares its model with the stash and fails with
//! [`DriverError::PeerDiverged`] on any difference. The stimulus is therefore
//! driven from a pre-state equal to the spec's. The stash verifies stimulus;
//! it never feeds `State` — `from_driver` reads only O5 and the marked sim
//! arbiter. The same comparison checks the instance's domains (`CONNS`,
//! `FLOW_IDS` are read from the spec; `HFLOWS` must equal the driver's) and
//! that a state labelled `step` is the instance's initial state.
//!
//! **Coverage limit.** The `flows_ok` guest side compares only `gf`,
//! `toHostPaired` and `toHostRefused` (the TcpAccept half). The guest's
//! TcpConnect half (`GuestOpenFlow` … `HostFlowEnds`) is exercised here as
//! stimulus only — its production outcome is not projected; its oracle is the
//! acceptance suite `guest_flow/tests/tcp_connect.rs`.
//!
//! The spec defines no Quint `run` declarations, so there is no named run for
//! `#[quint_test]`; the instance replays randomized traces through
//! `#[quint_run]` (the seed is printed on failure).

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "a conformance driver asserts its own preconditions; protocol names are prose"
)]

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::str::FromStr;
use std::sync::Arc;

use itf::Value;
use overdrive_core::guest_transport::LaunchToken;
use overdrive_core::guest_transport::wire::{
    CONTROL_RECONNECT_BACKOFF, ControlMessage, FlowId, FlowOrigin, FlowRefusal, FlowRequest,
};
use overdrive_core::net::ListenPort;
use quint_connect::{Driver, Result, State, Step, quint_run, switch};
use serde::Deserialize;

use super::quint_itf::{self, ItfError};
use super::sim::{
    SimConnectTarget, SimGuestClock, SimGuestEffect, SimGuestOwnerIo, SimSessionEnd, sim_cookie,
};
use super::test_support::{GuestProcess, settle};
use super::{GuestAcceptPhase, GuestClock, GuestFlowOwner, HostChannel, SocketId};
use crate::transport::GuestTransportToken;

const WL: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 7);
const GW: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
/// The intake client a host-opened flow names.
const CLIENT: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 7), 40_000);
const ACCEPT_PORT: &str = "8080";
/// The model's host-opened flows (`HFLOWS`).
const HFLOWS: [i64; 1] = [1];

/// Why a driver step cannot proceed.
#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("action `{action}` is not enabled in the production state: {why}")]
    NotEnabled { action: &'static str, why: String },
    #[error("action `{0}` is hazard-only: it is never enabled in a design instance")]
    HazardOnly(&'static str),
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

/// The host variables the stimulus model tracks, in the spec's vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HostVars {
    alloc: String,
    lease: i64,
    vm_gen: i64,
    vm_up: bool,
    quiesced: bool,
    claim: i64,
    /// `sess(c)`: `(st, gen)`.
    sess: BTreeMap<i64, (String, i64)>,
    /// `flows(f)`: `(st, gen, req, inFlight)`.
    flows: BTreeMap<i64, (String, i64, i64, bool)>,
    /// `hf(h)`: `(st, gen)`.
    hf: BTreeMap<i64, (String, i64)>,
    to_guest_abort: BTreeSet<i64>,
}

/// One step's spec post-state, as the next step's driver checks it.
struct SpecStash {
    host: HostVars,
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
        let st_gen = |v: &Value| -> std::result::Result<(String, i64), ItfError> {
            Ok((
                quint_itf::string(quint_itf::field(v, "st")?)?,
                quint_itf::int(quint_itf::field(v, "gen")?)?,
            ))
        };
        Ok(Self {
            alloc: quint_itf::string(quint_itf::var(vars, "alloc")?)?,
            lease: quint_itf::int(quint_itf::var(vars, "gen")?)?,
            vm_gen: quint_itf::int(quint_itf::var(vars, "vmGen")?)?,
            vm_up: quint_itf::boolean(quint_itf::var(vars, "vmUp")?)?,
            quiesced: quint_itf::boolean(quint_itf::var(vars, "quiesced")?)?,
            claim: quint_itf::int(quint_itf::var(vars, "claim")?)?,
            sess: quint_itf::int_map(quint_itf::var(vars, "sess")?, st_gen)?,
            flows: quint_itf::int_map(quint_itf::var(vars, "flows")?, |v| {
                Ok((
                    quint_itf::string(quint_itf::field(v, "st")?)?,
                    quint_itf::int(quint_itf::field(v, "gen")?)?,
                    quint_itf::int(quint_itf::field(v, "req")?)?,
                    quint_itf::boolean(quint_itf::field(v, "inFlight")?)?,
                ))
            })?,
            hf: quint_itf::int_map(quint_itf::var(vars, "hf")?, st_gen)?,
            to_guest_abort: quint_itf::int_set(quint_itf::var(vars, "toGuestAbort")?)?,
        })
    }

    /// The spec's `init` over the same domains.
    fn initial(&self) -> Self {
        Self {
            alloc: "Absent".to_owned(),
            lease: 0,
            vm_gen: 0,
            vm_up: false,
            quiesced: false,
            claim: 0,
            sess: self.sess.keys().map(|c| (*c, ("none".to_owned(), 0))).collect(),
            flows: self.flows.keys().map(|f| (*f, ("none".to_owned(), 0, 0, false))).collect(),
            hf: self.hf.keys().map(|h| (*h, ("none".to_owned(), 0))).collect(),
            to_guest_abort: BTreeSet::new(),
        }
    }
}

fn not_enabled(action: &'static str, why: &'static str) -> impl FnOnce() -> DriverError {
    move || DriverError::NotEnabled { action, why: why.to_owned() }
}

// ----- the stimulus model (the host's side) ------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alloc {
    Absent,
    Provisioned,
    Active,
    Retiring,
}

impl Alloc {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Absent => "Absent",
            Self::Provisioned => "Provisioned",
            Self::Active => "Active",
            Self::Retiring => "Retiring",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessSt {
    Pending,
    Reading,
    Live,
    Dead,
    Closed,
}

impl SessSt {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Reading => "reading",
            Self::Live => "live",
            Self::Dead => "dead",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Sess {
    st: SessSt,
    lease: u32,
    /// The guest's control connect the driver attributed to this connection.
    socket: Option<SocketId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowSt {
    Queued,
    Accepted,
    Read,
    Installed,
    Connecting,
    Paired,
    Closed,
}

impl FlowSt {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Accepted => "accepted",
            Self::Read => "read",
            Self::Installed => "installed",
            Self::Connecting => "connecting",
            Self::Paired => "paired",
            Self::Closed => "closed",
        }
    }

    const fn registered(self) -> bool {
        matches!(
            self,
            Self::Accepted | Self::Read | Self::Installed | Self::Connecting | Self::Paired
        )
    }

    const fn attributed(self) -> bool {
        matches!(self, Self::Read | Self::Installed | Self::Connecting | Self::Paired)
    }
}

#[derive(Debug, Clone, Copy)]
struct HostFlow {
    st: FlowSt,
    lease: u32,
    req: i64,
    in_flight: bool,
    /// The guest's flow connect for this flow, on the launch `gen`.
    socket: Option<SocketId>,
    /// The guest-origin flow id the guest wrote in its request.
    id: Option<FlowId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HfSt {
    Req,
    Paired,
    Closed,
}

impl HfSt {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Req => "req",
            Self::Paired => "paired",
            Self::Closed => "closed",
        }
    }
}

struct FlowsOkDriver {
    /// The current launch's owner under test.
    guest: GuestProcess,
    /// Every launch's seam, in launch order (the last is the current VM's).
    sims: Vec<SimGuestOwnerIo>,
    /// The current launch's clock.
    clock: Arc<SimGuestClock>,
    // stimulus model
    alloc: Alloc,
    lease: u32,
    vm_gen: u32,
    vm_up: bool,
    quiesced: bool,
    claim: Option<i64>,
    sess: BTreeMap<i64, Sess>,
    flows: BTreeMap<i64, HostFlow>,
    /// `hf(h)`: state and the lease that opened it.
    hf: BTreeMap<i64, (HfSt, u32)>,
    to_guest_abort: BTreeSet<i64>,
    /// A state labelled `step` was taken as the initial state; the next step
    /// checks that it was.
    step_as_init: bool,
    /// Relabelling record: host-opened flows the guest has held (seen in
    /// `accepted` or `reset_in_queue`).
    held: BTreeSet<FlowId>,
}

fn host_flow(h: i64) -> FlowId {
    FlowId::new(FlowOrigin::Host, u32::try_from(h).expect("model flow ids are small"))
        .expect("seq < 2^31")
}

fn launch_token(lease: u32) -> LaunchToken {
    LaunchToken::from_str(&format!("{lease:032x}")).expect("fixture launch token")
}

fn app_cookie(f: i64) -> super::SocketCookie {
    sim_cookie(100 + u64::try_from(f).expect("model flow ids are positive"))
}

fn app_destination(f: i64) -> SocketAddrV4 {
    SocketAddrV4::new(
        Ipv4Addr::new(93, 0, u8::try_from(f).expect("model flow ids are small"), 1),
        443,
    )
}

impl FlowsOkDriver {
    fn new() -> Self {
        let clock = Arc::new(SimGuestClock::new());
        Self {
            guest: GuestProcess::NotStarted,
            sims: Vec::new(),
            clock,
            alloc: Alloc::Absent,
            lease: 0,
            vm_gen: 0,
            vm_up: false,
            quiesced: false,
            claim: None,
            sess: BTreeMap::new(),
            flows: BTreeMap::new(),
            hf: BTreeMap::new(),
            to_guest_abort: BTreeSet::new(),
            step_as_init: false,
            held: BTreeSet::new(),
        }
    }

    /// A driver for a new run: no stash of an earlier run is compared.
    fn fresh() -> Self {
        SPEC_HOST.with(|stash| stash.borrow_mut().take());
        Self::new()
    }

    // ----- stimulus verification ------------------------------------------------------

    /// The stimulus model in the spec's vocabulary, over the spec's domains
    /// (`conns`, `fids`, `hflows`) and any key the model holds besides.
    fn host_vars(&self, spec: &HostVars) -> HostVars {
        let mut sess: BTreeMap<i64, (String, i64)> =
            spec.sess.keys().map(|c| (*c, ("none".to_owned(), 0))).collect();
        for (c, s) in &self.sess {
            sess.insert(*c, (s.st.as_str().to_owned(), i64::from(s.lease)));
        }
        let mut flows: BTreeMap<i64, (String, i64, i64, bool)> =
            spec.flows.keys().map(|f| (*f, ("none".to_owned(), 0, 0, false))).collect();
        for (f, x) in &self.flows {
            flows.insert(*f, (x.st.as_str().to_owned(), i64::from(x.lease), x.req, x.in_flight));
        }
        let mut hf: BTreeMap<i64, (String, i64)> =
            spec.hf.keys().map(|h| (*h, ("none".to_owned(), 0))).collect();
        for (h, (st, lease)) in &self.hf {
            hf.insert(*h, (st.as_str().to_owned(), i64::from(*lease)));
        }
        HostVars {
            alloc: self.alloc.as_str().to_owned(),
            lease: i64::from(self.lease),
            vm_gen: i64::from(self.vm_gen),
            vm_up: self.vm_up,
            quiesced: self.quiesced,
            claim: self.claim.unwrap_or(0),
            sess,
            flows,
            hf,
            to_guest_abort: self.to_guest_abort.clone(),
        }
    }

    /// Compares the stimulus model with the spec's post-state of the previous
    /// step (stashed by `State::from_spec`): the instance's domains, the
    /// `step`-labelled initial state, then every host variable the model
    /// tracks. Nothing to compare before the run's first step.
    fn verify_stimulus(&mut self) -> std::result::Result<(), DriverError> {
        let Some(spec) = SPEC_HOST.with(|stash| stash.borrow_mut().take()) else {
            return Ok(());
        };
        let hflows: BTreeSet<i64> = spec.host.hf.keys().copied().collect();
        if hflows != HFLOWS.iter().copied().collect::<BTreeSet<_>>() {
            return Err(DriverError::DomainMismatch(format!(
                "the spec's HFLOWS is {hflows:?}, the driver assumes {HFLOWS:?}"
            )));
        }
        if std::mem::take(&mut self.step_as_init) && !spec.initial {
            return Err(DriverError::StepNotInitial);
        }
        let model = self.host_vars(&spec.host);
        if model != spec.host {
            return Err(DriverError::PeerDiverged {
                spec: format!("{:?}", spec.host),
                driver: format!("{model:?}"),
            });
        }
        Ok(())
    }

    // ----- the host's predicates (stimulus model) ---------------------------------

    fn sess_live(&self) -> bool {
        self.sess.values().any(|s| matches!(s.st, SessSt::Live | SessSt::Dead))
    }

    const fn cid_accepts_session(&self) -> bool {
        matches!(self.alloc, Alloc::Provisioned | Alloc::Active)
    }

    const fn token_matches(&self, lease: u32) -> bool {
        self.cid_accepts_session() && lease == self.lease
    }

    fn admissible(&self) -> bool {
        self.alloc == Alloc::Active && self.sess_live() && !self.quiesced
    }

    const fn writer_up(&self, lease: u32) -> bool {
        self.vm_up && self.vm_gen == lease
    }

    // ----- the guest -----------------------------------------------------------------

    /// The live launch's seam: the VM of `gen` is running.
    fn live_sim(&self, lease: u32) -> Option<SimGuestOwnerIo> {
        if self.writer_up(lease) && self.guest.owner().is_some() {
            self.sims.last().cloned()
        } else {
            None
        }
    }

    /// Settles the live owner.
    fn settle(&mut self) {
        if let (Some(owner), Some(io)) = (self.guest.owner_mut(), self.sims.last()) {
            settle(owner, io);
        }
    }

    /// Records the host-opened flows the guest holds now (relabelling memory).
    fn note_held(&mut self) {
        let Some(owner) = self.guest.owner() else { return };
        let accepted: Vec<FlowId> = owner.guest_observation().accepted.keys().copied().collect();
        self.held.extend(accepted);
        if let Some(io) = self.sims.last() {
            self.held.extend(io.guest_socket_observation().reset_in_queue);
        }
    }

    /// The host discards every message the guest wrote and it has not consumed.
    fn discard_guest_messages(&self) {
        for io in &self.sims {
            let _discarded = io.take_outbox();
        }
    }

    /// The host closes `flow`'s V_h: the guest sees the vsock end.
    fn end_guest_flow(&mut self, f: i64) {
        let Some(flow) = self.flows.get(&f).copied() else { return };
        if let (Some(id), Some(io)) = (flow.id, self.live_sim(flow.lease)) {
            io.host_vsock_end(id);
            self.settle();
        }
    }

    /// Aborts every registered guest-opened flow (`abortAll`).
    fn abort_all_flows(&mut self) {
        let aborted: Vec<i64> =
            self.flows.iter().filter(|(_, x)| x.st.registered()).map(|(f, _)| *f).collect();
        for f in aborted {
            if let Some(x) = self.flows.get_mut(&f) {
                x.st = FlowSt::Closed;
            }
            self.end_guest_flow(f);
        }
    }

    /// Closes every open host-opened flow (`abortAllH`); the guest observes
    /// each close later (`GuestSeesHostClose`, `GuestQueueReset`).
    fn abort_all_h(&mut self) -> BTreeSet<i64> {
        let open: BTreeSet<i64> = self
            .hf
            .iter()
            .filter(|(_, (st, _))| matches!(st, HfSt::Req | HfSt::Paired))
            .map(|(h, _)| *h)
            .collect();
        for h in &open {
            self.set_hf(*h, HfSt::Closed);
        }
        open
    }

    /// Sets `hf(h).st`, keeping the lease that opened it.
    fn set_hf(&mut self, h: i64, st: HfSt) {
        if let Some(entry) = self.hf.get_mut(&h) {
            entry.0 = st;
        }
    }

    /// The host ends the live guest's session `c`: held at the seam until the
    /// guest observes it.
    fn end_session_deferred(&self, c: i64) {
        let Some(s) = self.sess.get(&c) else { return };
        if let Some(io) = self.live_sim(s.lease)
            && io.session_socket().is_some()
            && s.socket.is_some()
            && !io.session_end_pending()
        {
            io.host_end_session_deferred(SimSessionEnd::Eof);
        }
    }

    /// The host closes connection `c` before it ever was live: the guest's
    /// session ends at once (no guest flow belongs to it).
    fn close_unlive_session(&mut self, c: i64) {
        let Some(s) = self.sess.get(&c).copied() else { return };
        if s.socket.is_some()
            && let Some(io) = self.live_sim(s.lease)
        {
            io.end_session(SimSessionEnd::Eof);
            self.settle();
        }
    }

    // ----- lifecycle -------------------------------------------------------------------

    fn init(&mut self) {
        *self = Self::new();
    }

    /// A state labelled `step` (quint 0.32 labels the initial state of some
    /// traces so): taken as the initial state, which the next step verifies.
    fn initial_step(&mut self) {
        self.init();
        self.step_as_init = true;
    }

    const fn provision(&mut self) {
        self.alloc = Alloc::Provisioned;
        self.lease += 1;
    }

    fn vm_start(&mut self) {
        self.vm_up = true;
        self.vm_gen = self.lease;
        let clock = Arc::new(SimGuestClock::new());
        let io = SimGuestOwnerIo::new(WL, Arc::clone(&clock));
        let token = GuestTransportToken {
            workload_addr: WL,
            gateway: GW,
            launch: launch_token(self.lease),
        };
        let owner_clock: Arc<dyn GuestClock> = clock.clone();
        let owner = GuestFlowOwner::new(token, Box::new(io.clone()), owner_clock);
        self.guest = GuestProcess::Alive(owner);
        self.sims.push(io);
        self.clock = clock;
        self.settle();
    }

    fn vm_stop(&mut self) {
        let vm_gen = self.vm_gen;
        for s in self.sess.values_mut() {
            if s.lease == vm_gen && s.st == SessSt::Live {
                s.st = SessSt::Dead;
            } else if s.lease == vm_gen && s.st == SessSt::Reading {
                s.st = SessSt::Closed;
            }
        }
        for x in self.flows.values_mut() {
            if x.lease == vm_gen && x.st == FlowSt::Accepted && x.req < 2 {
                x.st = FlowSt::Closed;
            }
        }
        if let Some(io) = self.sims.last() {
            io.guest_owner_dies();
        }
        self.guest = GuestProcess::Dead;
        self.vm_up = false;
    }

    const fn activate(&mut self) {
        self.alloc = Alloc::Active;
    }

    fn teardown(&mut self) {
        self.alloc = Alloc::Retiring;
        let closing: Vec<i64> = self
            .sess
            .iter()
            .filter(|(_, s)| matches!(s.st, SessSt::Live | SessSt::Dead))
            .map(|(c, _)| *c)
            .collect();
        for c in &closing {
            self.end_session_deferred(*c);
            if let Some(s) = self.sess.get_mut(c) {
                s.st = SessSt::Closed;
            }
        }
        self.claim = None;
        self.abort_all_flows();
        let _closed = self.abort_all_h();
        self.discard_guest_messages();
        self.to_guest_abort.clear();
    }

    const fn release(&mut self) {
        self.alloc = Alloc::Absent;
    }

    fn quiesce(&mut self) {
        self.quiesced = true;
        self.abort_all_flows();
        let open = self.abort_all_h();
        if self.sess_live() {
            self.to_guest_abort.extend(open);
        } else {
            self.to_guest_abort.clear();
        }
    }

    const fn restore(&mut self) {
        self.quiesced = false;
    }

    // ----- control session -------------------------------------------------------------

    fn guest_open_session(&mut self, c: i64) -> std::result::Result<(), DriverError> {
        let lease = self.vm_gen;
        let io =
            self.live_sim(lease).ok_or_else(not_enabled("GuestOpenSession", "no live guest"))?;
        if io.session_end_pending() {
            // the guest observes the loss before it reconnects
            io.release_session_end();
            self.settle();
            self.note_held();
        }
        let connecting = |io: &SimGuestOwnerIo| {
            io.pending_connects()
                .into_iter()
                .find(|(_, t)| *t == SimConnectTarget::Host(HostChannel::Control))
        };
        if connecting(&io).is_none() {
            if io.session_socket().is_some() {
                return Err(DriverError::NotEnabled {
                    action: "GuestOpenSession",
                    why: "the guest's session is live at the seam".to_owned(),
                });
            }
            self.clock.advance(CONTROL_RECONNECT_BACKOFF);
            self.settle();
        }
        let (socket, _) = connecting(&io)
            .ok_or_else(not_enabled("GuestOpenSession", "the guest issued no control connect"))?;
        self.sess.insert(c, Sess { st: SessSt::Pending, lease, socket: Some(socket) });
        Ok(())
    }

    fn host_accept_session(&mut self, c: i64) -> std::result::Result<(), DriverError> {
        let s = self
            .sess
            .get(&c)
            .copied()
            .ok_or_else(not_enabled("HostAcceptSession", "no such connection"))?;
        let accepts = self.cid_accepts_session();
        if let (Some(socket), Some(io)) = (s.socket, self.live_sim(s.lease)) {
            io.complete_connect(socket, Ok(()));
            self.settle();
        }
        self.sess
            .insert(c, Sess { st: if accepts { SessSt::Reading } else { SessSt::Closed }, ..s });
        if !accepts {
            self.close_unlive_session(c);
        }
        Ok(())
    }

    fn host_read_open(&mut self, c: i64) -> std::result::Result<(), DriverError> {
        let s = self
            .sess
            .get(&c)
            .copied()
            .ok_or_else(not_enabled("HostReadOpen", "no such connection"))?;
        let st = if !self.token_matches(s.lease) || self.claim.is_some() {
            SessSt::Closed
        } else {
            self.claim = Some(c);
            if self.writer_up(s.lease) { SessSt::Live } else { SessSt::Dead }
        };
        self.sess.insert(c, Sess { st, ..s });
        if st == SessSt::Closed {
            self.close_unlive_session(c);
        }
        Ok(())
    }

    fn host_session_loss(&mut self, c: i64) -> std::result::Result<(), DriverError> {
        let s = self
            .sess
            .get(&c)
            .copied()
            .ok_or_else(not_enabled("HostSessionLoss", "no such connection"))?;
        self.end_session_deferred(c);
        self.sess.insert(c, Sess { st: SessSt::Closed, ..s });
        if self.claim == Some(c) {
            self.claim = None;
        }
        self.abort_all_flows();
        let _closed = self.abort_all_h();
        self.discard_guest_messages();
        self.to_guest_abort.clear();
        Ok(())
    }

    // ----- guest-opened flows (TcpConnect) ----------------------------------------------

    fn guest_open_flow(&mut self, f: i64) -> std::result::Result<(), DriverError> {
        let lease = self.vm_gen;
        let io = self.live_sim(lease).ok_or_else(not_enabled("GuestOpenFlow", "no live guest"))?;
        let before: BTreeSet<SocketId> =
            io.pending_connects().into_iter().map(|(id, _)| id).collect();
        io.app_connect(app_cookie(f), app_destination(f), true);
        self.settle();
        // the guest's flow connect; none if the owner refused the connect
        let socket = io
            .pending_connects()
            .into_iter()
            .find(|(id, t)| {
                *t == SimConnectTarget::Host(HostChannel::FlowStream) && !before.contains(id)
            })
            .map(|(id, _)| id);
        self.flows.insert(
            f,
            HostFlow { st: FlowSt::Queued, lease, req: 0, in_flight: false, socket, id: None },
        );
        Ok(())
    }

    fn host_accept_flow(&mut self, f: i64) -> std::result::Result<(), DriverError> {
        let x = self
            .flows
            .get(&f)
            .copied()
            .ok_or_else(not_enabled("HostAcceptFlow", "no such flow"))?;
        let st = if self.admissible() { FlowSt::Accepted } else { FlowSt::Closed };
        let id = if let (Some(socket), Some(io)) = (x.socket, self.live_sim(x.lease)) {
            io.complete_connect(socket, Ok(()));
            self.settle();
            io.effects().iter().rev().find_map(|e| match e {
                SimGuestEffect::WriteRequest {
                    request: FlowRequest::TcpConnect { flow, .. },
                    ..
                } => Some(*flow),
                _ => None,
            })
        } else {
            None
        };
        self.flows.insert(f, HostFlow { st, id, ..x });
        if st == FlowSt::Closed {
            self.end_guest_flow(f);
        }
        Ok(())
    }

    fn host_read_req(&mut self, f: i64, n: i64) -> std::result::Result<(), DriverError> {
        let x =
            self.flows.get(&f).copied().ok_or_else(not_enabled("HostReadReq", "no such flow"))?;
        let req = (x.req + n).min(2);
        let st = if req < 2 {
            FlowSt::Accepted
        } else if self.token_matches(x.lease) {
            FlowSt::Read
        } else {
            FlowSt::Closed
        };
        self.flows.insert(f, HostFlow { st, req, ..x });
        if st == FlowSt::Closed {
            self.end_guest_flow(f);
        }
        Ok(())
    }

    fn host_flow_step(
        &mut self,
        f: i64,
        action: &'static str,
        st: FlowSt,
        in_flight: bool,
    ) -> std::result::Result<(), DriverError> {
        let x = self.flows.get(&f).copied().ok_or_else(not_enabled(action, "no such flow"))?;
        self.flows.insert(f, HostFlow { st, in_flight, ..x });
        Ok(())
    }

    fn host_connect_done(&mut self, f: i64) -> std::result::Result<(), DriverError> {
        let x = self
            .flows
            .get(&f)
            .copied()
            .ok_or_else(not_enabled("HostConnectDone", "no such flow"))?;
        self.flows.insert(f, HostFlow { st: FlowSt::Paired, in_flight: false, ..x });
        if let (Some(flow), Some(io)) = (x.id, self.live_sim(x.lease))
            && io.session_socket().is_some()
            && !io.session_end_pending()
        {
            io.host_control(ControlMessage::Paired { flow });
            self.settle();
        }
        Ok(())
    }

    fn host_flow_ends(&mut self, f: i64, msg: bool) -> std::result::Result<(), DriverError> {
        let x =
            self.flows.get(&f).copied().ok_or_else(not_enabled("HostFlowEnds", "no such flow"))?;
        self.flows.insert(f, HostFlow { st: FlowSt::Closed, ..x });
        if let (Some(flow), Some(io)) = (x.id, self.live_sim(x.lease))
            && msg
            && x.st.attributed()
            && io.session_socket().is_some()
            && !io.session_end_pending()
        {
            let message = if x.st == FlowSt::Paired {
                ControlMessage::Abort { flow }
            } else {
                ControlMessage::Refused { flow, refusal: FlowRefusal::Policy }
            };
            io.host_control(message);
            self.settle();
        }
        self.end_guest_flow(f);
        Ok(())
    }

    // ----- host-opened flows (TcpAccept) -------------------------------------------------

    fn host_open_accept(&mut self, h: i64) -> std::result::Result<(), DriverError> {
        let lease = self.lease;
        let io = self
            .live_sim(lease)
            .ok_or_else(not_enabled("HostOpenAccept", "no live guest of the lease"))?;
        let listen_port = ListenPort::from_str(ACCEPT_PORT).expect("non-zero listen port");
        io.host_open_accept(FlowRequest::TcpAccept {
            flow: host_flow(h),
            client: CLIENT,
            listen_port,
            launch: launch_token(lease),
        });
        self.hf.insert(h, (HfSt::Req, lease));
        Ok(())
    }

    fn guest_accept_v(&mut self, h: i64) -> std::result::Result<(), DriverError> {
        let io =
            self.live_sim(self.vm_gen).ok_or_else(not_enabled("GuestAcceptV", "no live guest"))?;
        let flow = host_flow(h);
        io.release_inbound_accept(flow);
        self.settle();
        io.release_inbound_request(flow);
        self.settle();
        self.note_held();
        Ok(())
    }

    fn guest_pair_or_refuse(&mut self, h: i64, ok: bool) -> std::result::Result<(), DriverError> {
        let io = self
            .live_sim(self.vm_gen)
            .ok_or_else(not_enabled("GuestPairOrRefuse", "no live guest"))?;
        io.complete_app_connect(
            host_flow(h),
            if ok { Ok(()) } else { Err(ErrorKind::ConnectionRefused) },
        );
        self.settle();
        Ok(())
    }

    fn host_recv_paired(&mut self, h: i64) -> std::result::Result<(), DriverError> {
        let flow = host_flow(h);
        let took = self.sims.iter().any(|io| io.host_take_message(ControlMessage::Paired { flow }));
        if !took {
            return Err(DriverError::NotEnabled {
                action: "HostRecvPaired",
                why: "the guest wrote no Paired".to_owned(),
            });
        }
        if self.hf.get(&h).is_some_and(|(st, _)| *st == HfSt::Req) {
            self.set_hf(h, HfSt::Paired);
        }
        Ok(())
    }

    fn host_recv_refused(&mut self, h: i64) -> std::result::Result<(), DriverError> {
        let flow = host_flow(h);
        let took = self.sims.iter().any(|io| {
            io.host_take_first(
                |m| matches!(m, ControlMessage::Refused { flow: f, .. } if *f == flow),
            )
            .is_some()
        });
        if !took {
            return Err(DriverError::NotEnabled {
                action: "HostRecvRefused",
                why: "the guest wrote no Refused".to_owned(),
            });
        }
        if self.hf.get(&h).is_some_and(|(st, _)| *st == HfSt::Req) {
            self.set_hf(h, HfSt::Closed);
        }
        Ok(())
    }

    fn host_abort_accept(&mut self, h: i64) {
        self.set_hf(h, HfSt::Closed);
        if self.sess_live() {
            self.to_guest_abort.insert(h);
        }
    }

    fn guest_recv_abort(&mut self, h: i64) -> std::result::Result<(), DriverError> {
        if !self.to_guest_abort.remove(&h) {
            return Err(DriverError::NotEnabled {
                action: "GuestRecvAbort",
                why: "no Abort in flight".to_owned(),
            });
        }
        // the Abort reaches a live guest on its live session; else it is lost
        if let Some(io) = self.live_sim(self.vm_gen)
            && io.session_socket().is_some()
            && !io.session_end_pending()
        {
            io.host_control(ControlMessage::Abort { flow: host_flow(h) });
            self.settle();
        }
        Ok(())
    }

    fn guest_sees_host_close(
        &mut self,
        h: i64,
        action: &'static str,
    ) -> std::result::Result<(), DriverError> {
        let io = self.live_sim(self.vm_gen).ok_or_else(not_enabled(action, "no live guest"))?;
        io.host_vsock_end(host_flow(h));
        self.settle();
        self.note_held();
        Ok(())
    }

    fn guest_loss_abort(&mut self, h: i64) -> std::result::Result<(), DriverError> {
        let io = self
            .live_sim(self.vm_gen)
            .ok_or_else(not_enabled("GuestLossAbort", "no live guest"))?;
        let holds = self
            .guest
            .owner()
            .is_some_and(|o| o.guest_observation().accepted.contains_key(&host_flow(h)));
        if !holds {
            return Err(DriverError::NotEnabled {
                action: "GuestLossAbort",
                why: format!("the guest holds no flow {h}"),
            });
        }
        if !io.session_end_pending() {
            return Err(DriverError::NotEnabled {
                action: "GuestLossAbort",
                why: "the seam holds no session end for the guest (its flow belongs to no session the host ended)"
                    .to_owned(),
            });
        }
        io.release_session_end();
        self.settle();
        Ok(())
    }
}

impl Driver for FlowsOkDriver {
    type State = FlowsOkGuestState;

    fn step(&mut self, step: &Step) -> Result {
        self.verify_stimulus()?;
        switch!(step {
            init => self.init(),
            // quint 0.32 `run --n-traces` labels the initial state of some
            // traces `step` instead of `init` (observed at index 0, with the
            // `init` state's values); it is taken as the initial state, and
            // the next step fails with `StepNotInitial` unless it was.
            step => self.initial_step(),
            Provision => self.provision(),
            VmStart => self.vm_start(),
            VmStop => self.vm_stop(),
            Activate => self.activate(),
            Teardown => self.teardown(),
            Release => self.release(),
            Quiesce => self.quiesce(),
            Restore => self.restore(),
            GuestOpenSession(c: i64) => self.guest_open_session(c)?,
            HostAcceptSession(c: i64) => self.host_accept_session(c)?,
            HostReadOpen(c: i64) => self.host_read_open(c)?,
            HostClaimSession => return Err(DriverError::HazardOnly("HostClaimSession").into()),
            HostSessionLoss(c: i64) => self.host_session_loss(c)?,
            HostDeadSessionLoss(c: i64) => self.host_session_loss(c)?,
            GuestOpenFlow(f: i64) => self.guest_open_flow(f)?,
            HostAcceptFlow(f: i64) => self.host_accept_flow(f)?,
            HostCheckFlow => return Err(DriverError::HazardOnly("HostCheckFlow").into()),
            HostRegisterFlow => return Err(DriverError::HazardOnly("HostRegisterFlow").into()),
            HostReadReq(f: i64, n: i64) => self.host_read_req(f, n)?,
            HostInstall(f: i64) => self.host_flow_step(f, "HostInstall", FlowSt::Installed, false)?,
            HostStartConnect(f: i64) => self.host_flow_step(f, "HostStartConnect", FlowSt::Connecting, true)?,
            HostConnectDone(f: i64) => self.host_connect_done(f)?,
            HostConnectDiscard(f: i64) => self.host_flow_step(f, "HostConnectDiscard", FlowSt::Closed, false)?,
            HostFlowEnds(f: i64, msg: bool) => self.host_flow_ends(f, msg)?,
            HostOpenAccept(h: i64) => self.host_open_accept(h)?,
            GuestAcceptV(h: i64) => self.guest_accept_v(h)?,
            GuestPairOrRefuse(h: i64, ok: bool) => self.guest_pair_or_refuse(h, ok)?,
            HostRecvPaired(h: i64) => self.host_recv_paired(h)?,
            HostRecvRefused(h: i64) => self.host_recv_refused(h)?,
            HostAbortAccept(h: i64) => self.host_abort_accept(h),
            GuestRecvAbort(h: i64) => self.guest_recv_abort(h)?,
            GuestSeesHostClose(h: i64) => self.guest_sees_host_close(h, "GuestSeesHostClose")?,
            GuestLossAbort(h: i64) => self.guest_loss_abort(h)?,
            GuestQueueReset(h: i64) => self.guest_sees_host_close(h, "GuestQueueReset")?,
        })
    }
}

// ----- the compared state ------------------------------------------------------

/// The compared `owner_flows` variables, guest side: `gf(h).st`,
/// `toHostPaired`, `toHostRefused` (`gf(h).gen` ghost, `gf(h).c` stimulus).
#[derive(Debug, PartialEq, Eq)]
struct FlowsOkGuestState {
    gf: BTreeMap<i64, String>,
    to_host_paired: BTreeSet<i64>,
    to_host_refused: BTreeSet<i64>,
}

/// Why the production state has no model image.
#[derive(Debug, thiserror::Error)]
enum ProjectionError {
    #[error("{0} has no pre-image in the model")]
    NoPreImage(String),
}

impl FlowsOkGuestState {
    fn from_value(value: &Value) -> std::result::Result<Self, ItfError> {
        let vars = quint_itf::vars(value)?;
        Ok(Self {
            gf: quint_itf::int_map(quint_itf::var(&vars, "gf")?, |g| {
                quint_itf::string(quint_itf::field(g, "st")?)
            })?,
            to_host_paired: quint_itf::int_set(quint_itf::var(&vars, "toHostPaired")?)?,
            to_host_refused: quint_itf::int_set(quint_itf::var(&vars, "toHostRefused")?)?,
        })
    }
}

impl<'de> Deserialize<'de> for FlowsOkGuestState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

impl State<FlowsOkDriver> for FlowsOkGuestState {
    /// Decodes the compared variables, and stashes the host variables for the
    /// next step's stimulus verification (never a projection source).
    fn from_spec(value: Value) -> Result<Self> {
        let state = Self::from_value(&value)?;
        let host = HostVars::from_vars(&quint_itf::vars(&value)?)?;
        let initial = host == host.initial()
            && state.to_host_paired.is_empty()
            && state.to_host_refused.is_empty()
            && state.gf.values().all(|st| st == "none");
        SPEC_HOST.with(|stash| *stash.borrow_mut() = Some(SpecStash { host, initial }));
        Ok(state)
    }

    fn from_driver(driver: &FlowsOkDriver) -> Result<Self> {
        let accepted = driver
            .guest
            .owner()
            .map(|owner| owner.guest_observation().accepted)
            .unwrap_or_default();
        // conformance-lint: sim-arbiter-ok QPROJ-7 guest-owner seam arbiter (V_g torn down in the guest accept queue, K-A4), Tier-3 R5-31 / R5-28
        let live_sim = driver.sims.last().filter(|_| driver.guest.owner().is_some());
        let reset =
            live_sim.map(|io| io.guest_socket_observation().reset_in_queue).unwrap_or_default();
        let mut gf = BTreeMap::new();
        if accepted.is_empty() && reset.is_empty() && driver.held.is_empty() {
            // the guest has never held a host-opened flow
            gf.extend(HFLOWS.iter().map(|h| (*h, "none".to_owned())));
        } else {
            for h in HFLOWS {
                let flow = host_flow(h);
                let st = match accepted.get(&flow) {
                    Some(GuestAcceptPhase::Accepted) => "acc",
                    Some(GuestAcceptPhase::Routed) => "routed",
                    None if reset.contains(&flow) => "reset",
                    None if driver.held.contains(&flow) => "closed",
                    None => "none",
                };
                gf.insert(h, st.to_owned());
            }
            for flow in accepted.keys().chain(reset.iter()) {
                if !HFLOWS.iter().any(|h| host_flow(*h) == *flow) {
                    return Err(ProjectionError::NoPreImage(format!("host flow {flow:?}")).into());
                }
            }
        }
        let h_of = |flow: FlowId| -> std::result::Result<i64, ProjectionError> {
            HFLOWS
                .iter()
                .copied()
                .find(|h| host_flow(*h) == flow)
                .ok_or_else(|| ProjectionError::NoPreImage(format!("flow {flow:?}")))
        };
        let mut to_host_paired = BTreeSet::new();
        let mut to_host_refused = BTreeSet::new();
        // conformance-lint: sim-arbiter-ok QPROJ-7 guest-owner seam arbiter (messages the guest wrote, not consumed), Tier-3 R5-31 / R5-28
        for io in &driver.sims {
            for message in io.guest_socket_observation().outbox {
                match message {
                    ControlMessage::Paired { flow } => {
                        to_host_paired.insert(h_of(flow)?);
                    }
                    ControlMessage::Refused { flow, .. } => {
                        to_host_refused.insert(h_of(flow)?);
                    }
                    _ => {}
                }
            }
        }
        Ok(Self { gf, to_host_paired, to_host_refused })
    }
}

// ----- the run -----------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[quint_run(spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt", main = "flows_ok")]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init); model fidelity open, upstream UI-R2-5"]
fn quint_owner_flows_ok_guest() -> impl Driver {
    FlowsOkDriver::fresh()
}
