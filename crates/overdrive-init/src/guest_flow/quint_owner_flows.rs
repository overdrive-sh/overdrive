//! Quint conformance driver — `owner_flows`, guest side (`gf`, `gSess`;
//! QPROJ-1, QPROJ-5, QPROJ-7, DG-5-E, DG-5-F, D-UP-8, INB-1).
//!
//! Replays `specs/quint/guest-flow-owner/owner_flows.qnt`, conformance main
//! `flows_conf` (`core2`: two control connections, two guest-opened flows, two
//! dial slots; `REC = true`), against the production guest flow owner
//! [`GuestFlowOwner`] of each launch, over the guest-owner seam sim
//! [`SimGuestOwnerIo`] (its forwarder context a `SimFlowForwarder`) and
//! [`SimGuestClock`]. The spec is referenced in place. The guest side of the
//! module is projected; every host action is stimulus (QPROJ-1): the driver
//! plays the host owner, the allocation lifecycle, the VM, the dialer and the
//! applications, and its outputs reach the guest at the guest actions the trace
//! names (`GuestRecvAbort`, `GuestSeesHostClose`, `GuestQueueReset`,
//! `GuestSessionLossObserved`); it re-implements no rule of the guest.
//!
//! **Step record (D-UP-1).** `Config::nondet` is the single element
//! `flows_conf::owner_flows::conf_step`; every `#[quint_test]` and the
//! `#[quint_run]` name `flows_conf`; the projection matches the qualified
//! names `flows_conf::owner_flows::<var>` ([`quint_itf::Qualified`]).
//!
//! Action → driving port / seam stimulus (feature-delta § *Conformance
//! observation surface and action map*, "`owner_flows` `flows_conf`, guest
//! side"). Every handler ends by dispatching the owner until quiescent
//! (DG-5-Q), the owner's driving port, once the guest has taken its first
//! dispatch:
//!
//! | Action | Driver |
//! |---|---|
//! | `InitWith(o)` | no lease, no VM, no guest |
//! | `Provision` | a new lease: its launch token is the next launch's |
//! | `VmStart` | a new launch: a fresh seam and `GuestFlowOwner::new` with the lease's launch token; no dispatch yet |
//! | `VmStop` | the VM ends: the seam releases the guest context and ends every guest socket (`guest_owner_dies`); the driver drops the owner (the guest's session is gone, D-UP-8) |
//! | `GuestOpenSession(c)` | `dispatch_next` with no session — the first dispatch, or `SimGuestClock` advanced to the loss + `CONTROL_RECONNECT_BACKOFF` — must issue `start_connect_host(Control)`; the host listener completes it and the owner writes its `ControlOpen` (an owner that issues no connect fails the step) |
//! | `WorkloadOpenSession(c)` | stimulus: a guest workload's connection, not the guest owner's |
//! | `GuestOpenFlow(f)` | after the owner's first dispatch, seam: an application `connect()` is captured (`OutboundCaptured`), the host listener completes the owner's V_g connect and the owner writes its request; before that dispatch it is the host-side stale queued-flow stimulus and changes no projected guest-owner fact |
//! | `HostConnectDone(f)` | the host attached its pair and writes `Paired` for `f`'s production flow on the session (EVMOD-PAIR: the guest attaches on it) |
//! | `HostFlowEndsMsg(f, msg)` | the host closes `f`'s V_h, writing `Refused` (before `Paired`) or `Abort` (after) first when it answers (`msg` and the flow attributed) |
//! | `Teardown`, `Quiesce`, `HostSessionLoss(c)`, `HostDeadSessionLoss(c)` | the host closes the V_h of every guest-opened flow it held registered; the session's end and the dials' closes reach the guest at the guest actions below |
//! | `DialOpen(h)` | seam queues V_g with its `TcpAccept` request (peer = gateway:0, the lease's token) |
//! | `GuestAcceptV(h)` | seam releases the accept and the request |
//! | `GuestPairOrRefuseOk(h, ok)` | seam completes the application connect (accepted / refused) |
//! | `GuestConnectTimesOut(h)` | `SimGuestClock` advanced to that flow's `GUEST_INBOUND_CONNECT_TIMEOUT` deadline |
//! | `GuestRecvAbort(h)` | seam releases the host's `Abort` |
//! | `GuestSeesHostClose(h)` / `GuestQueueReset(h)` | seam delivers the host's close of V_h after / before the guest's accept (DG-5-E, K-A4); an attached pair's end record is released |
//! | `GuestAcceptedError(h)` | seam delivers `Hangup { Error }` on the accepted V_g (DG-5-E) |
//! | `GuestSessionLossObserved(c)` | seam delivers the end (EOF, K-A5) of the guest's session connection `c`; the owner aborts every flow of it in that dispatch (DG-5-F) |
//! | `GuestNeverReplies(h)` | stimulus: the guest's dial handling stalls — the seam releases nothing more for `h` |
//! | `HostRecvPaired(h)`, `HostRecvRefused(h)`, `HostRecvTimedOut(h)`, `HostRecvAbort(h)` | the host consumes the guest's reply from the seam; a reply the guest did not write fails the step |
//! | `Activate*`, `Release`, `Restore`, `HostAcceptSession`, `HostReadOpen`, `HostAcceptFlow`, `HostReadReqN`, `HostStartConnect`, `HostConnectDiscard`, `Dial{NotServing,Quiesced,NoSession,Unreachable}`, `HostAbortAccept`, `HostDialDeadline`, `DialCancel` | stimulus (host state; the guest observes its effects at the guest actions above) |
//! | `HostClaimSession` | hazard-only — an error, never a no-op |
//! | `GuestOpenBeacon`, `HostAcceptBeacon`, `HostReadReady`, `DriverStop` | the beacon instance's actions (`flows_beacon_conf`, the VM driver's driver) — never enabled on `flows_conf`; an error |
//!
//! Projection (`State::from_driver`): `gf(h).st` ← O QPROJ-5
//! `guest_observation().accepted` (`Accepted` → `acc`, `Paired` → `attached`);
//! `reset` ⇔ S `reset_in_queue` of the live launch (QPROJ-7, Tier-3 pair V-24);
//! `closed` (and the model-internal `tomb`) ⇔ the flow was held by the guest —
//! accepted, or torn down in its queue — and no longer is; `none` otherwise;
//! `silent` is the scripted stimulus of `GuestNeverReplies` and is not
//! compared. `gSess` ← O QPROJ-5 `session` (`session ⇔ gSess ≠ 0`); while the
//! VM is down, or before the guest's first dispatch, `session = false`. Every
//! other variable is stimulus, ghost or model-only.
//!
//! Identity relabelling: host flow `h` ↔ `FlowId::new(Host, h)`; guest flow
//! `f` ↔ the guest-origin `FlowId` of the request the owner wrote for the
//! application connect the driver issued for `f`; connection `c` ↔ the
//! guest's control connect the driver completed for it; lease `gen` ↔ the
//! launch token `token(gen)`. The memory "the guest has held this host flow"
//! is the relabelling's record of the production identities it has seen.
//!
//! **Pre-state stash.** `State::from_spec` stashes each step's state; at the
//! next step the driver reads the pre-state's host variables where a host
//! output depends on them (which guest-opened flows the host held registered;
//! whether the host answers a flow it ends; which connection is the CID's live
//! session at a guest accept). The stash is stimulus; it never feeds `State`.

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
use std::time::Instant;

use itf::Value;
use overdrive_core::guest_transport::LaunchToken;
use overdrive_core::guest_transport::wire::{
    CONTROL_RECONNECT_BACKOFF, ControlMessage, FlowId, FlowOrigin, FlowRefusal, FlowRequest,
    GUEST_INBOUND_CONNECT_TIMEOUT,
};
use overdrive_core::net::ListenPort;
use overdrive_core::vm::config::VsockPort;
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;

use super::quint_itf::{self, ItfError, Qualified};
use super::sim::{SimConnectTarget, SimGuestClock, SimGuestOwnerIo, SimSessionEnd, sim_cookie};
use super::test_support::{GuestProcess, settle, start};
use super::{GuestAcceptPhase, GuestClock, GuestFlowOwner, HostChannel};
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

const MAIN: &str = "flows_conf";
const MODULE: &str = "owner_flows";
const WL: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 7);
const GW: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
const DIAL_PORT: u16 = 8080;
/// `HFLOWS` of `flows_conf` (`core2`).
const HFLOWS: [i64; 2] = [1, 2];

/// Why a driver step cannot proceed.
#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("action `{action}` is not enabled in the production state: {why}")]
    NotEnabled { action: &'static str, why: String },
    #[error("action `{0}` is hazard-only: it is never enabled in a design instance")]
    HazardOnly(&'static str),
    #[error(
        "action `{0}` belongs to the beacon instance (`flows_beacon_conf`); it is never enabled on `flows_conf`"
    )]
    OtherInstance(&'static str),
    #[error("the step cannot be represented in production: {0}")]
    NotRepresentable(String),
    #[error("the trace's {0} has no production identity")]
    NoIdentity(String),
}

// ----- the pre-state stash (stimulus, never projection) --------------------------

thread_local! {
    /// The previous step's spec state, written by `State::from_spec`. Read by
    /// the next step's handlers where a host output depends on the host's
    /// state; never a projection source.
    static SPEC_PRE: RefCell<Option<Qualified>> = const { RefCell::new(None) };
}

fn pre() -> Option<Qualified> {
    SPEC_PRE.with(|stash| stash.borrow().clone())
}

/// The pre-state's `flows(f).st`.
fn pre_flow_st(f: i64) -> Option<String> {
    let vars = pre()?;
    let flows = quint_itf::int_map(vars.var("flows").ok()?, |v| {
        quint_itf::string(quint_itf::field(v, "st")?)
    })
    .ok()?;
    flows.get(&f).cloned()
}

/// The pre-state's guest-opened flows the host holds registered (U-1).
fn pre_registered_flows() -> BTreeSet<i64> {
    let Some(vars) = pre() else { return BTreeSet::new() };
    let Ok(flows) = vars
        .var("flows")
        .and_then(|v| quint_itf::int_map(v, |x| quint_itf::string(quint_itf::field(x, "st")?)))
    else {
        return BTreeSet::new();
    };
    flows
        .into_iter()
        .filter(|(_, st)| matches!(st.as_str(), "accepted" | "read" | "connecting" | "paired"))
        .map(|(f, _)| f)
        .collect()
}

/// The pre-state's live control connection of the current launch (`liveConn`).
fn pre_live_conn() -> i64 {
    let Some(vars) = pre() else { return 0 };
    let vm_gen = vars.var("vmGen").ok().and_then(|v| quint_itf::int(v).ok()).unwrap_or(0);
    let Ok(sess) = vars.var("sess").and_then(|v| {
        quint_itf::int_map(v, |x| {
            Ok((
                quint_itf::string(quint_itf::field(x, "st")?)?,
                quint_itf::int(quint_itf::field(x, "gen")?)?,
            ))
        })
    }) else {
        return 0;
    };
    sess.into_iter()
        .filter(|(_, (st, g))| st == "live" && *g == vm_gen)
        .map(|(c, _)| c)
        .next_back()
        .unwrap_or(0)
}

// ----- the driver ----------------------------------------------------------------

/// One launch of the guest: its seam, clock and lifecycle. The launch's
/// owner is the driver's own `owner` field (S-7, the "Owner field" ruling),
/// built with the launch and replaced with it at each `VmStart`.
struct Launch {
    /// The launch's guest process lifecycle (test support beside the owner).
    process: GuestProcess,
    io: SimGuestOwnerIo,
    clock: Arc<SimGuestClock>,
    /// The owner has taken its first dispatch.
    started: bool,
    /// The guest's control connection `c` (the guest owner's own), by model id.
    own_conns: BTreeSet<i64>,
    /// The connection the guest holds as its session (model id).
    session_conn: Option<i64>,
    /// When the guest observed the loss of the session it held.
    loss_at: Option<Instant>,
    /// When each dial's C connect started (its `GUEST_INBOUND_CONNECT_TIMEOUT`).
    connect_started: BTreeMap<i64, Instant>,
}

struct OwnerFlowsGuestDriver {
    /// The production guest flow owner of the current launch, held directly
    /// (S-7, the "Owner field" ruling): the only path to the owner. Built
    /// with each launch; after the launch's VM stops it is kept and never
    /// called or projected again.
    owner: GuestFlowOwner,
    /// The current lease (`gen`).
    lease_gen: i64,
    /// The VM's launch (`vmGen`) and whether it is up.
    vm_gen: i64,
    /// The current launch (its owner is `owner`): `NotStarted` before the
    /// first `VmStart` (an owner built, never dispatched or projected),
    /// `Alive` while the VM runs, `Dead` after its `VmStop` (the guest is PID
    /// 1: it dies with its VM).
    launch: Launch,
    /// Guest-opened flow `f` ↔ its production flow, per launch.
    flow_of: BTreeMap<i64, FlowId>,
    /// Dial slots the guest has held (accepted, or torn down in its accept
    /// queue): the relabelling's memory of identities seen.
    held: BTreeSet<i64>,
    /// Dial slots whose V_g the host tore down in the guest's accept queue
    /// in a launch that has since ended.
    reset_then_gone: BTreeSet<i64>,
    /// Dial slots scripted `GuestNeverReplies` (stimulus; not compared).
    silent: BTreeSet<i64>,
}

fn token(lease_gen: i64) -> LaunchToken {
    let byte = 0x10 + u8::try_from(lease_gen).expect("MAXGEN is small");
    LaunchToken::from_bytes([byte; 16]).expect("non-zero launch token")
}

fn host_flow(h: i64) -> FlowId {
    FlowId::new(FlowOrigin::Host, u32::try_from(h).expect("dial slots are small"))
        .expect("small seq")
}

fn guest_dest(f: i64) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(93, 0, u8::try_from(f).expect("flow ids are small"), 1), 443)
}

fn listen_port() -> ListenPort {
    ListenPort::from_str(&DIAL_PORT.to_string()).expect("non-zero port")
}

impl Launch {
    /// A launch of generation `vm_gen` whose VM has not started, with its
    /// owner built over the launch's fresh seam and clock, not dispatched.
    fn of(vm_gen: i64) -> (GuestFlowOwner, Self) {
        let clock = Arc::new(SimGuestClock::new());
        let io = SimGuestOwnerIo::new(WL, Arc::clone(&clock));
        let transport = GuestTransportToken {
            workload_addr: WL,
            gateway: GW,
            launch: token(vm_gen),
            ports: host_ports(),
        };
        let owner_clock: Arc<dyn GuestClock> = clock.clone();
        let owner = GuestFlowOwner::new(transport, Box::new(io.clone()), owner_clock);
        let launch = Self {
            process: GuestProcess::NotStarted,
            io,
            clock,
            started: false,
            own_conns: BTreeSet::new(),
            session_conn: None,
            loss_at: None,
            connect_started: BTreeMap::new(),
        };
        (owner, launch)
    }

    /// The launch's VM runs: its owner has not died. (`Alive` from the VM's
    /// start; the owner's first dispatch is taken at the guest's first
    /// session, `started`.)
    const fn alive(&self) -> bool {
        self.process.is_alive()
    }
}

impl OwnerFlowsGuestDriver {
    fn new() -> Self {
        SPEC_PRE.with(|stash| stash.borrow_mut().take());
        let (owner, launch) = Launch::of(0);
        Self {
            owner,
            lease_gen: 0,
            vm_gen: 0,
            launch,
            flow_of: BTreeMap::new(),
            held: BTreeSet::new(),
            reset_then_gone: BTreeSet::new(),
            silent: BTreeSet::new(),
        }
    }

    fn launch(&mut self, action: &'static str) -> std::result::Result<&mut Launch, DriverError> {
        Some(&mut self.launch).filter(|l| l.alive()).ok_or_else(|| DriverError::NotEnabled {
            action,
            why: "no guest is running".to_owned(),
        })
    }

    /// Dispatches the live guest owner until quiescent (DG-5-Q) once it has
    /// taken its first dispatch: the owner's driving port, called by every
    /// handler.
    fn settle(&mut self) {
        let launch = &self.launch;
        if !launch.started || !launch.alive() {
            return;
        }
        let (io, clock) = (launch.io.clone(), Arc::clone(&launch.clock));
        settle(&mut self.owner, &io, &clock);
    }

    /// Runs the guest forwarder's queued work and releases its end records,
    /// then settles (the context's readiness, EVMOD-REC).
    fn settle_forwarder(&mut self) {
        if let Some(launch) = Some(&self.launch).filter(|l| l.started && l.alive()) {
            launch.io.run_forwarder();
            launch.io.release_forward_ends();
        }
        self.settle();
    }

    /// The host closes the V_h of a guest flow the live guest holds.
    fn host_closes_vsock(&mut self, flow: FlowId) {
        if let Some(launch) = Some(&self.launch).filter(|l| l.alive()) {
            launch.io.host_vsock_end(flow);
        }
        self.settle_forwarder();
    }

    // ----- lifecycle --------------------------------------------------------------

    fn init(&mut self) {
        *self = Self::new();
    }

    fn provision(&mut self) {
        self.lease_gen += 1;
        self.settle();
    }

    fn vm_start(&mut self) {
        self.vm_gen = self.lease_gen;
        self.flow_of.clear();
        let (owner, launch) = Launch::of(self.vm_gen);
        self.owner = owner;
        self.launch = launch;
        self.launch.process = GuestProcess::Alive;
    }

    fn vm_stop(&mut self) {
        if let Some(launch) = Some(&mut self.launch).filter(|l| l.alive()) {
            let queued_reset = launch.io.guest_socket_observation().reset_in_queue;
            for h in HFLOWS {
                if queued_reset.contains(&host_flow(h)) {
                    self.reset_then_gone.insert(h);
                }
            }
            launch.io.guest_owner_dies();
            launch.process = GuestProcess::Dead;
        }
    }

    // ----- the guest's control session -----------------------------------------------

    fn guest_open_session(&mut self, c: i64) -> std::result::Result<(), DriverError> {
        let launch = self.launch("GuestOpenSession")?;
        if launch.started {
            let at = launch.loss_at.ok_or_else(|| DriverError::NotEnabled {
                action: "GuestOpenSession",
                why: "the guest holds a session it has not observed lost (D-UP-8)".to_owned(),
            })?;
            launch.clock.advance_to(at + CONTROL_RECONNECT_BACKOFF);
        } else {
            launch.started = true;
            start(&mut self.owner);
        }
        self.settle();
        let launch = self.launch("GuestOpenSession")?;
        let connecting = launch
            .io
            .pending_connects()
            .iter()
            .any(|(_, t)| *t == SimConnectTarget::Host(HostChannel::Control));
        if !connecting {
            return Err(DriverError::NotEnabled {
                action: "GuestOpenSession",
                why: "the guest owner issued no control connect".to_owned(),
            });
        }
        launch.io.complete_host_connect(HostChannel::Control, Ok(()));
        launch.own_conns.insert(c);
        launch.session_conn = Some(c);
        launch.loss_at = None;
        self.settle();
        Ok(())
    }

    fn guest_session_loss_observed(&mut self, c: i64) -> std::result::Result<(), DriverError> {
        let launch = self.launch("GuestSessionLossObserved")?;
        if launch.session_conn != Some(c) {
            if launch.own_conns.contains(&c) {
                self.settle();
                return Ok(());
            }
            return Err(DriverError::NotRepresentable(format!(
                "the guest's flows are of connection {c}, a workload's: in production the guest owner's flows \
                 belong to its own session (DG-5-F, D-UP-8)"
            )));
        }
        launch.io.end_session(SimSessionEnd::Eof);
        let now = launch.clock.now();
        launch.session_conn = None;
        launch.loss_at = Some(now);
        self.settle_forwarder();
        Ok(())
    }

    // ----- guest-opened flows ---------------------------------------------------------

    fn guest_open_flow(&mut self, f: i64) -> std::result::Result<(), DriverError> {
        let launch = self.launch("GuestOpenFlow")?;
        if !launch.started {
            // `flows(f)` is the host owner's flow. Before this launch's guest
            // owner takes its first dispatch, the stale-flow named run is a
            // raw guest workload connection queued at the host; this guest
            // projection has no owner fact to drive or compare.
            return Ok(());
        }
        launch.io.app_connect(sim_cookie(u64::try_from(f).expect("small")), guest_dest(f));
        self.settle();
        let launch = self.launch("GuestOpenFlow")?;
        if !launch
            .io
            .pending_connects()
            .iter()
            .any(|(_, target)| *target == SimConnectTarget::Host(HostChannel::FlowStream))
        {
            // With no live owner session, production rejects the captured
            // child locally and opens no V_g. `flows(f)` is host-side state,
            // outside this driver's compared guest projection.
            return Ok(());
        }
        launch.io.complete_host_connect(HostChannel::FlowStream, Ok(()));
        self.settle();
        let launch = self.launch("GuestOpenFlow")?;
        let flow = launch
            .io
            .requests()
            .iter()
            .rev()
            .find_map(|r| match r {
                FlowRequest::TcpConnect { flow, destination, .. }
                    if *destination == guest_dest(f) =>
                {
                    Some(*flow)
                }
                _ => None,
            })
            .ok_or_else(|| DriverError::NotEnabled {
                action: "GuestOpenFlow",
                why: "the guest owner wrote no TcpConnect request".to_owned(),
            })?;
        self.flow_of.insert(f, flow);
        Ok(())
    }

    fn host_connect_done(&mut self, f: i64) {
        if let (Some(flow), Some(launch)) =
            (self.flow_of.get(&f).copied(), Some(&self.launch).filter(|l| l.alive()))
            && launch.io.session_socket().is_some()
        {
            launch.io.host_control(ControlMessage::Paired { flow });
        }
        self.settle_forwarder();
    }

    fn host_flow_ends(&mut self, f: i64, msg: bool) {
        let st = pre_flow_st(f).unwrap_or_default();
        let attributed = matches!(st.as_str(), "read" | "connecting" | "paired");
        if let Some(flow) = self.flow_of.get(&f).copied() {
            if msg
                && attributed
                && let Some(launch) = Some(&self.launch).filter(|l| l.alive())
                && launch.io.session_socket().is_some()
            {
                let message = if st == "paired" {
                    ControlMessage::Abort { flow }
                } else {
                    ControlMessage::Refused { flow, refusal: FlowRefusal::Policy }
                };
                launch.io.host_control(message);
                self.settle();
            }
            self.host_closes_vsock(flow);
        } else {
            self.settle();
        }
    }

    /// Teardown, quiescence, session loss: the host closes the V_h of every
    /// guest-opened flow it held registered.
    fn host_aborts_flows(&mut self) {
        for f in pre_registered_flows() {
            if let Some(flow) = self.flow_of.get(&f).copied() {
                self.host_closes_vsock(flow);
            }
        }
        self.settle();
    }

    // ----- dials (host-opened flows) -----------------------------------------------------

    fn dial_open(&mut self, h: i64) -> std::result::Result<(), DriverError> {
        let token = token(self.lease_gen);
        let launch = self.launch("DialOpen")?;
        launch.io.host_open_accept(FlowRequest::TcpAccept {
            flow: host_flow(h),
            peer: GW,
            listen_port: listen_port(),
            launch: token,
        });
        self.settle();
        Ok(())
    }

    fn guest_accept_v(&mut self, h: i64) -> std::result::Result<(), DriverError> {
        let live = pre_live_conn();
        let launch = self.launch("GuestAcceptV")?;
        if live != 0 && !launch.own_conns.contains(&live) {
            return Err(DriverError::NotRepresentable(format!(
                "the CID's live session is connection {live}, a workload's; the model attributes the guest's \
                 flow to it, production to the guest owner's own session (DG-5-F, D-UP-8)"
            )));
        }
        launch.io.release_inbound_accept(host_flow(h));
        self.settle();
        let launch = self.launch("GuestAcceptV")?;
        launch.io.release_inbound_request(host_flow(h));
        let now = launch.clock.now();
        launch.connect_started.insert(h, now);
        self.held.insert(h);
        self.settle();
        Ok(())
    }

    fn guest_pair_or_refuse(&mut self, h: i64, ok: bool) -> std::result::Result<(), DriverError> {
        let launch = self.launch("GuestPairOrRefuseOk")?;
        let result = if ok { Ok(()) } else { Err(ErrorKind::ConnectionRefused) };
        let connecting = launch.io.pending_connects().iter().any(
            |(_, t)| matches!(t, SimConnectTarget::App { flow: Some(f), .. } if *f == host_flow(h)),
        );
        if !connecting {
            return Err(DriverError::NotEnabled {
                action: "GuestPairOrRefuseOk",
                why: format!("the guest has no application connect in flight for dial {h}"),
            });
        }
        launch.io.complete_app_connect(host_flow(h), result);
        self.settle();
        Ok(())
    }

    fn guest_connect_times_out(&mut self, h: i64) -> std::result::Result<(), DriverError> {
        let launch = self.launch("GuestConnectTimesOut")?;
        let started =
            launch.connect_started.get(&h).copied().ok_or_else(|| DriverError::NotEnabled {
                action: "GuestConnectTimesOut",
                why: format!("dial {h} has no application connect the guest started"),
            })?;
        launch.clock.advance_to(started + GUEST_INBOUND_CONNECT_TIMEOUT);
        self.settle();
        Ok(())
    }

    fn guest_recv_abort(&mut self, h: i64) -> std::result::Result<(), DriverError> {
        let launch = self.launch("GuestRecvAbort")?;
        launch.io.host_control(ControlMessage::Abort { flow: host_flow(h) });
        self.settle_forwarder();
        Ok(())
    }

    fn guest_sees_host_close(&mut self, h: i64) {
        self.host_closes_vsock(host_flow(h));
        if self.launch.io.guest_socket_observation().reset_in_queue.contains(&host_flow(h)) {
            self.held.insert(h);
        }
    }

    fn guest_accepted_error(&mut self, h: i64) -> std::result::Result<(), DriverError> {
        let launch = self.launch("GuestAcceptedError")?;
        launch.io.flow_vsock_error(host_flow(h), ErrorKind::ConnectionReset);
        self.settle();
        Ok(())
    }

    /// The host consumes the guest's reply for dial `h`; a reply the guest did
    /// not write fails the step.
    fn host_recv(
        &mut self,
        action: &'static str,
        h: i64,
        matches: impl Fn(&ControlMessage) -> bool,
    ) -> std::result::Result<(), DriverError> {
        let taken = self.launch.io.host_take_first(|m| matches(m));
        if taken.is_none() {
            return Err(DriverError::NotEnabled {
                action,
                why: format!("the guest wrote no such reply for dial {h}"),
            });
        }
        self.settle();
        Ok(())
    }
}

impl Driver for OwnerFlowsGuestDriver {
    type State = OwnerFlowsGuestState;

    fn config() -> Config {
        Config { state: &[], nondet: &["flows_conf::owner_flows::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            InitWith => self.init(),
            Provision => self.provision(),
            VmStart => self.vm_start(),
            VmStop => self.vm_stop(),
            Activate => self.settle(),
            ActivatePending => self.settle(),
            ActivateQuiesced => self.settle(),
            ActivateRetiring => self.settle(),
            Teardown => self.host_aborts_flows(),
            Release => self.settle(),
            Quiesce => self.host_aborts_flows(),
            Restore => self.settle(),
            GuestOpenSession(c: i64) => self.guest_open_session(c)?,
            WorkloadOpenSession => self.settle(),
            HostAcceptSession => self.settle(),
            HostReadOpen => self.settle(),
            HostClaimSession => return Err(DriverError::HazardOnly("HostClaimSession").into()),
            HostSessionLoss => self.host_aborts_flows(),
            HostDeadSessionLoss => self.host_aborts_flows(),
            GuestOpenFlow(f: i64) => self.guest_open_flow(f)?,
            HostAcceptFlow => self.settle(),
            HostReadReqN => self.settle(),
            HostStartConnect => self.settle(),
            HostConnectDone(f: i64) => self.host_connect_done(f),
            HostConnectDiscard => self.settle(),
            HostFlowEndsMsg(f: i64, msg: bool) => self.host_flow_ends(f, msg),
            DialOpen(h: i64) => self.dial_open(h)?,
            DialNotServing => self.settle(),
            DialQuiesced => self.settle(),
            DialNoSession => self.settle(),
            DialUnreachable => self.settle(),
            GuestAcceptV(h: i64) => self.guest_accept_v(h)?,
            GuestPairOrRefuseOk(h: i64, ok: bool) => self.guest_pair_or_refuse(h, ok)?,
            GuestConnectTimesOut(h: i64) => self.guest_connect_times_out(h)?,
            HostRecvPaired(h: i64) => {
                let flow = host_flow(h);
                self.host_recv("HostRecvPaired", h, |m| *m == ControlMessage::Paired { flow })?;
            },
            HostRecvRefused(h: i64) => {
                let flow = host_flow(h);
                self.host_recv("HostRecvRefused", h, |m| matches!(m, ControlMessage::Refused { flow: x, .. } if *x == flow))?;
            },
            HostRecvTimedOut(h: i64) => {
                let flow = host_flow(h);
                self.host_recv("HostRecvTimedOut", h, |m| matches!(m, ControlMessage::TimedOut { flow: x, .. } if *x == flow))?;
            },
            HostAbortAccept => self.settle(),
            HostDialDeadline => self.settle(),
            DialCancel => self.settle(),
            GuestNeverReplies(h: i64) => {
                self.silent.insert(h);
                self.settle();
            },
            GuestRecvAbort(h: i64) => self.guest_recv_abort(h)?,
            GuestSeesHostClose(h: i64) => self.guest_sees_host_close(h),
            GuestAcceptedError(h: i64) => self.guest_accepted_error(h)?,
            HostRecvAbort(h: i64) => {
                let flow = host_flow(h);
                self.host_recv("HostRecvAbort", h, |m| *m == ControlMessage::Abort { flow })?;
            },
            GuestSessionLossObserved(c: i64) => self.guest_session_loss_observed(c)?,
            GuestQueueReset(h: i64) => self.guest_sees_host_close(h),
            GuestOpenBeacon => return Err(DriverError::OtherInstance("GuestOpenBeacon").into()),
            HostAcceptBeacon => return Err(DriverError::OtherInstance("HostAcceptBeacon").into()),
            HostReadReady => return Err(DriverError::OtherInstance("HostReadReady").into()),
            DriverStop => return Err(DriverError::OtherInstance("DriverStop").into()),
        })
    }
}

// ----- the compared state ------------------------------------------------------

/// The compared `owner_flows` variables, guest side: each dial slot's guest
/// phase (`None`: the scripted `silent` stimulus, not compared) and whether
/// the guest owner holds a session (`gSess ≠ 0`).
#[derive(Debug, PartialEq, Eq)]
struct OwnerFlowsGuestState {
    gf: BTreeMap<i64, Option<String>>,
    session: bool,
}

impl OwnerFlowsGuestState {
    fn from_vars(vars: &Qualified) -> std::result::Result<Self, ItfError> {
        let gf = quint_itf::int_map(vars.var("gf")?, |x| {
            let st = quint_itf::string(quint_itf::field(x, "st")?)?;
            Ok(match st.as_str() {
                "silent" => None,
                "tomb" => Some("closed".to_owned()),
                _ => Some(st),
            })
        })?;
        let session = quint_itf::int(vars.var("gSess")?)? != 0;
        Ok(Self { gf, session })
    }
}

impl<'de> Deserialize<'de> for OwnerFlowsGuestState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let vars = Qualified::of(&value, MAIN, MODULE).map_err(serde::de::Error::custom)?;
        Self::from_vars(&vars).map_err(serde::de::Error::custom)
    }
}

impl State<OwnerFlowsGuestDriver> for OwnerFlowsGuestState {
    /// Decodes the compared variables by qualified name and stashes the state
    /// for the next step's stimulus (never a projection source).
    fn from_spec(value: Value) -> Result<Self> {
        let vars = Qualified::of(&value, MAIN, MODULE)?;
        let state = Self::from_vars(&vars)?;
        SPEC_PRE.with(|stash| *stash.borrow_mut() = Some(vars));
        Ok(state)
    }

    fn from_driver(driver: &OwnerFlowsGuestDriver) -> Result<Self> {
        // QPROJ-5: the live launch's owner accessor (none while no VM runs).
        let observation = driver.launch.alive().then(|| driver.owner.guest_observation());
        let alive = observation.is_some();
        // conformance-lint: sim-arbiter-ok QPROJ-7 guest-owner seam arbiter (V_g torn down in the guest accept queue, K-A4), Tier-3 V-24 (P-38)
        let sockets = alive.then(|| driver.launch.io.guest_socket_observation());
        let mut gf = BTreeMap::new();
        for h in HFLOWS {
            if driver.silent.contains(&h) {
                gf.insert(h, None);
                continue;
            }
            let flow = host_flow(h);
            let phase = match observation.as_ref().and_then(|o| o.accepted.get(&flow)) {
                Some(GuestAcceptPhase::Accepted) => "acc",
                Some(GuestAcceptPhase::Paired { .. }) => "attached",
                None if sockets.as_ref().is_some_and(|s| s.reset_in_queue.contains(&flow)) => {
                    "reset"
                }
                None if driver.held.contains(&h) || driver.reset_then_gone.contains(&h) => "closed",
                None => "none",
            };
            gf.insert(h, Some(phase.to_owned()));
        }
        let session =
            alive && driver.launch.started && observation.as_ref().is_some_and(|o| o.session);
        Ok(Self { gf, session })
    }
}

// ----- the named runs (every run of the conformance main encodes a critical
// ----- scenario of `owner_flows`, feature-delta § *Named runs pinned by each driver*)

/// Named run `ControlSessionOpensAndCidActivates` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "ControlSessionOpensAndCidActivates"
)]
fn quint_owner_flows_guest_control_session_opens_and_cid_activates() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `GuestOpenedFlowIsAdmittedReadAttachedAndPaired` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "GuestOpenedFlowIsAdmittedReadAttachedAndPaired"
)]
fn quint_owner_flows_guest_guest_opened_flow_is_admitted_read_attached_and_paired() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `RequestReadInPiecesAttachesOnlyAfterFullRead` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "RequestReadInPiecesAttachesOnlyAfterFullRead"
)]
fn quint_owner_flows_guest_request_read_in_pieces_attaches_only_after_full_read() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `RacingFirstConnectsLeaveOneLiveSession` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "RacingFirstConnectsLeaveOneLiveSession"
)]
fn quint_owner_flows_guest_racing_first_connects_leave_one_live_session() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `SessionLossAbortsFlowsAndReconnectRestores` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "SessionLossAbortsFlowsAndReconnectRestores"
)]
fn quint_owner_flows_guest_session_loss_aborts_flows_and_reconnect_restores() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `GuestReopensSessionOnlyAfterObservingItsLoss` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "GuestReopensSessionOnlyAfterObservingItsLoss"
)]
fn quint_owner_flows_guest_guest_reopens_session_only_after_observing_its_loss() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `GuestObservesSessionLossThenReopens` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "GuestObservesSessionLossThenReopens"
)]
fn quint_owner_flows_guest_guest_observes_session_loss_then_reopens() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `VmStopClearsTheGuestSession` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "VmStopClearsTheGuestSession"
)]
fn quint_owner_flows_guest_vm_stop_clears_the_guest_session() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `QuiesceAbortsFlowsAndRefusesAdmissionUntilRestore` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "QuiesceAbortsFlowsAndRefusesAdmissionUntilRestore"
)]
fn quint_owner_flows_guest_quiesce_aborts_flows_and_refuses_admission_until_restore() -> impl Driver
{
    OwnerFlowsGuestDriver::new()
}

/// Named run `ActivationWaitsForQuiescenceToEnd` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "ActivationWaitsForQuiescenceToEnd"
)]
fn quint_owner_flows_guest_activation_waits_for_quiescence_to_end() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `InFlightConnectOfAbortedFlowIsDiscarded` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "InFlightConnectOfAbortedFlowIsDiscarded"
)]
fn quint_owner_flows_guest_in_flight_connect_of_aborted_flow_is_discarded() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `AttributedFlowEndsAndCidKeepsServing` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "AttributedFlowEndsAndCidKeepsServing"
)]
fn quint_owner_flows_guest_attributed_flow_ends_and_cid_keeps_serving() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `TeardownAbortsEverythingThenCidIsLeasedAgain` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "TeardownAbortsEverythingThenCidIsLeasedAgain"
)]
fn quint_owner_flows_guest_teardown_aborts_everything_then_cid_is_leased_again() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `StaleLaunchSessionIsClosedAtReadAndDoesNotPoisonClaim` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "StaleLaunchSessionIsClosedAtReadAndDoesNotPoisonClaim"
)]
fn quint_owner_flows_guest_stale_launch_session_is_closed_at_read_and_does_not_poison_claim()
-> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `TeardownBetweenAcceptAndReadClosesWithoutClaim` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "TeardownBetweenAcceptAndReadClosesWithoutClaim"
)]
fn quint_owner_flows_guest_teardown_between_accept_and_read_closes_without_claim() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `StaleLaunchFlowIsClosedAtRequestReadWithNoMessage` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "StaleLaunchFlowIsClosedAtRequestReadWithNoMessage"
)]
fn quint_owner_flows_guest_stale_launch_flow_is_closed_at_request_read_with_no_message()
-> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `VmDeathMakesLiveSessionDeadThenLossIsProcessed` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "VmDeathMakesLiveSessionDeadThenLossIsProcessed"
)]
fn quint_owner_flows_guest_vm_death_makes_live_session_dead_then_loss_is_processed() -> impl Driver
{
    OwnerFlowsGuestDriver::new()
}

/// Named run `DialPairsAndReturnsStream` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialPairsAndReturnsStream"
)]
fn quint_owner_flows_guest_dial_pairs_and_returns_stream() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `DialRefusedReturnsTypedError` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialRefusedReturnsTypedError"
)]
fn quint_owner_flows_guest_dial_refused_returns_typed_error() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `DialTimedOutReturnsTypedError` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialTimedOutReturnsTypedError"
)]
fn quint_owner_flows_guest_dial_timed_out_returns_typed_error() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `DialNotServingIsATypedResultWithNoStateChange` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialNotServingIsATypedResultWithNoStateChange"
)]
fn quint_owner_flows_guest_dial_not_serving_is_a_typed_result_with_no_state_change() -> impl Driver
{
    OwnerFlowsGuestDriver::new()
}

/// Named run `DialWhileQuiescedIsATypedResultWithNoStateChange` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialWhileQuiescedIsATypedResultWithNoStateChange"
)]
fn quint_owner_flows_guest_dial_while_quiesced_is_a_typed_result_with_no_state_change()
-> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `DialWithoutLiveSessionIsATypedResultWithNoStateChange` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialWithoutLiveSessionIsATypedResultWithNoStateChange"
)]
fn quint_owner_flows_guest_dial_without_live_session_is_a_typed_result_with_no_state_change()
-> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `DialToAGoneVmIsUnreachable` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialToAGoneVmIsUnreachable"
)]
fn quint_owner_flows_guest_dial_to_a_gone_vm_is_unreachable() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `DialAfterObservedSessionLossWithVmGoneReturnsNoSession` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialAfterObservedSessionLossWithVmGoneReturnsNoSession"
)]
fn quint_owner_flows_guest_dial_after_observed_session_loss_with_vm_gone_returns_no_session()
-> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `PairingDeadlineWaitsForTheGuestAnswer` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "PairingDeadlineWaitsForTheGuestAnswer"
)]
fn quint_owner_flows_guest_pairing_deadline_waits_for_the_guest_answer() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `PairingDeadlineFiresOnlyAfterGuestTimeoutAndLateReplyIsDiscarded` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "PairingDeadlineFiresOnlyAfterGuestTimeoutAndLateReplyIsDiscarded"
)]
fn quint_owner_flows_guest_pairing_deadline_fires_only_after_guest_timeout_and_late_reply_is_discarded()
-> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `PairingDeadlineEndsDialToALiveVmWhoseGuestNeverReplies` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "PairingDeadlineEndsDialToALiveVmWhoseGuestNeverReplies"
)]
fn quint_owner_flows_guest_pairing_deadline_ends_dial_to_a_live_vm_whose_guest_never_replies()
-> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `VmExitDuringDialReturnsUnreachableReset` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "VmExitDuringDialReturnsUnreachableReset"
)]
fn quint_owner_flows_guest_vm_exit_during_dial_returns_unreachable_reset() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `VmExitDuringDialLeavesNoPairingDeadline` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "VmExitDuringDialLeavesNoPairingDeadline"
)]
fn quint_owner_flows_guest_vm_exit_during_dial_leaves_no_pairing_deadline() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `DroppedDialFutureEndsLikeThePairingDeadline` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DroppedDialFutureEndsLikeThePairingDeadline"
)]
fn quint_owner_flows_guest_dropped_dial_future_ends_like_the_pairing_deadline() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `TeardownEndsPendingDialWithNotServing` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "TeardownEndsPendingDialWithNotServing"
)]
fn quint_owner_flows_guest_teardown_ends_pending_dial_with_not_serving() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `ActivationIsImpossibleWhileTheSessionIsPending` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "ActivationIsImpossibleWhileTheSessionIsPending"
)]
fn quint_owner_flows_guest_activation_is_impossible_while_the_session_is_pending() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `ActivationWithSessionPendingReturnsPendingThenActivates` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "ActivationWithSessionPendingReturnsPendingThenActivates"
)]
fn quint_owner_flows_guest_activation_with_session_pending_returns_pending_then_activates()
-> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `ActivateAfterTeardownIsRefusedAndChangesNothing` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "ActivateAfterTeardownIsRefusedAndChangesNothing"
)]
fn quint_owner_flows_guest_activate_after_teardown_is_refused_and_changes_nothing() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `ActivateAfterTeardownNeverActivates` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "ActivateAfterTeardownNeverActivates"
)]
fn quint_owner_flows_guest_activate_after_teardown_never_activates() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `DialAbortedBeforeGuestAcceptsEndsByHostClose` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialAbortedBeforeGuestAcceptsEndsByHostClose"
)]
fn quint_owner_flows_guest_dial_aborted_before_guest_accepts_ends_by_host_close() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `DialQueuedVgIsResetByHostClose` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialQueuedVgIsResetByHostClose"
)]
fn quint_owner_flows_guest_dial_queued_vg_is_reset_by_host_close() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `LatePairedForClosedHostFlowIsDiscarded` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "LatePairedForClosedHostFlowIsDiscarded"
)]
fn quint_owner_flows_guest_late_paired_for_closed_host_flow_is_discarded() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `SessionLossAbortsDialOnBothSides` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "SessionLossAbortsDialOnBothSides"
)]
fn quint_owner_flows_guest_session_loss_aborts_dial_on_both_sides() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `QuiesceAbortsDialAndSendsAbort` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "QuiesceAbortsDialAndSendsAbort"
)]
fn quint_owner_flows_guest_quiesce_aborts_dial_and_sends_abort() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `AcceptedVgHostCloseEndsFlowSilently` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "AcceptedVgHostCloseEndsFlowSilently"
)]
fn quint_owner_flows_guest_accepted_vg_host_close_ends_flow_silently() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `AcceptedVgErrorAbortsWithAbort` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "AcceptedVgErrorAbortsWithAbort"
)]
fn quint_owner_flows_guest_accepted_vg_error_aborts_with_abort() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `SessionLossAbortsAllFlowsBeforeReconnect` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "SessionLossAbortsAllFlowsBeforeReconnect"
)]
fn quint_owner_flows_guest_session_loss_aborts_all_flows_before_reconnect() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

/// Named run `DialAfterReconnectPairs` (`flows_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialAfterReconnectPairs"
)]
fn quint_owner_flows_guest_dial_after_reconnect_pairs() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}

// ----- randomized traces -------------------------------------------------------------

/// Randomized traces of the conformance main (seed printed on failure).
/// CONTRACT_SHAPE: bounded-change.
#[quint_run(spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt", main = "flows_conf")]
fn quint_owner_flows_flows_conf_guest() -> impl Driver {
    OwnerFlowsGuestDriver::new()
}
