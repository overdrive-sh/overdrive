//! Quint conformance driver — `fwd_pair`, guest side (`s = g`; QPROJ-1,
//! QPROJ-5, QPROJ-7, QPROJ-8; EVMOD-PAIR, EVMOD-END, EVMOD-REC,
//! EVMOD-ATTACH-END, EVMOD-DGRAM, CLEAN-FULL-END, END-IDEMPOTENT, R2-END).
//!
//! Replays `specs/quint/guest-flow-owner/fwd_pair.qnt`, conformance main
//! `pair_conf` (every mode; `REC = true`), against the production guest flow
//! owner [`GuestFlowOwner`] over the guest-owner seam sim [`SimGuestOwnerIo`],
//! whose one forwarder context is an `overdrive-sim` `SimFlowForwarder` — the
//! named arbiter of the guest forwarder's kernel facts (QPROJ-8) — and whose
//! sockets are `overdrive-sim` `SimSocket`s. The spec is referenced in place.
//! The guest side `g` is projected; the host side's actions (`s = h`: the host
//! owner, its forwarder, the dialer, the far peer behind D) are stimulus taken
//! from the trace, reaching the guest only as the facts its V_g's far side
//! shows (bytes, SHUTDOWN / RST packets processed at `VCtl(hg)`, the host
//! consuming what V_g sent).
//!
//! **Step record (D-UP-1).** `Config::nondet` is the single element
//! `pair_conf::fwd_pair::conf_step`; every `#[quint_test]` and the
//! `#[quint_run]` name `pair_conf`; the projection matches the qualified names
//! `pair_conf::fwd_pair::<var>` ([`quint_itf::Qualified`]).
//!
//! **The flow per mode** ("Sides and pairs"): `stream` is a TcpConnect — the
//! guest pair is (I, V_g), I half A; `watch` is a dial — the guest pair is the
//! TcpAccept (V_g, C), V_g half A; `dgram` is a datagram association — (S,
//! V_g), S half A. Direction `gh` is the guest outer half → V_g, `hg` V_g →
//! the guest outer half. One model unit is one byte, numbered: unit `k` is the
//! byte `k`.
//!
//! Action → driving port / seam stimulus (feature-delta § *Conformance
//! observation surface and action map*, "`fwd_pair`"). Handlers that release
//! an owner event dispatch it to quiescence (DG-5-Q); socket/forwarder and
//! other-side stimuli do not consume a distinct pending owner dispatch behind
//! the model action's back:
//!
//! | Action | Driver |
//! |---|---|
//! | `InitWith(m)` | the driver builds the guest owner over the sim with a live session and one flow of mode `m`, unattached: a captured connect whose request is written (stream); a dial accepted, request read, C connecting (watch); a slot association whose request is written (dgram, started by one priming datagram) |
//! | `Attach(g)` | the guest owner's dispatch that attaches the pair: the host's `Paired` (stream, dgram); C's connect completion (watch), after which it writes `Paired` |
//! | `Abort(g)` / `GuestAbort(g)` | the host's `Abort { flow }` released to the guest owner, which aborts its pair (`FlowForwarder::abort`) |
//! | `CloseUnattached(g)` | the guest's control session ends (EOF): the owner closes the flow it has not attached — its TCP half with `SO_LINGER{1,0}`, V_g — with no record (DG-5-F) |
//! | `CtxRelease(g)` | the guest owner's death (the owner is PID 1): the seam's process exit releases the guest context (`SimFlowForwarder::release`) and closes every guest socket; the owner field is kept and never called or projected again (S-7) |
//! | `TakeEnd(g)` | the owner's dispatch on its context's readiness: the seam releases the context's oldest record (`take_end`) as the owner's `ForwardEnded` event and the owner takes exactly one dispatch — the one that consumes it (`last_consumed_end`, D-11) and releases the flow |
//! | `OwnerClose(g)` | the owner's later dispatches: the flow's descriptors closed after its record (linger cleared on `Completed`; dgram: the slot socket's queue discarded) |
//! | `Hop0(gh)`, `HopEnd0(gh)` | one run of the guest pair's `gh` direction (`SimFlowForwarder::run`, source the outer half) |
//! | `Hop1(hg)`, `HopEnd1(hg)` | one run of the guest pair's `hg` direction (source V_g) |
//! | `FwdErr(g)` | the guest pair's next runs, which find the pending error |
//! | `VCtl(hg)` | the next SHUTDOWN / RST packet from the host reaches V_g (`SimSocket::peer_shutdown`, classified by the pair's callback in that call) |
//! | `Write(gh)`, `Fin(gh)`, `TcpRst(g, ·)` | the guest application writes unit `k` / half-closes / resets on the outer half (`peer_send`, `peer_shutdown(Send)`, `peer_reset`) |
//! | `Read(hg)` | the guest application reads one unit (the outer half's far reader resumes, then stops) |
//! | `ShutRd(hg)` | the guest application stops reading for good |
//! | `Hop0(hg)` (stream), `Write(hg)` (watch) | a unit from the host reaches V_g's queue (`peer_send` on V_g) |
//! | `Hop1(gh)` (stream), `Read(gh)` (watch) | the host consumes one unit V_g sent (V_g's far reader resumes, then stops) |
//! | `SendDg(gh, sz)` | the guest application sends datagram `k` of `sz` bytes to S |
//! | `Hop0(hg)` (dgram) | the host forwarder's next frame item reaches V_g |
//! | `DRead(hg)` | the guest application reads one datagram from S |
//! | `UdpBlock(hg)`, `UdpUnblock(hg)` | S's far side stops / resumes accepting |
//! | `UdpErr(g)` | S gets a pending socket error (`set_sk_err`) |
//! | `DgHalfClose(g)` | a peer outside the design shuts V_g's send direction |
//! | `VmmExit` | the VM ends: the guest owner dies with its context (`guest_owner_dies`) |
//! | `Attach(h)`, `Abort(h)`, `CtxRelease(h)`, `Reopen(h)`, `TakeEnd(h)`, `OwnerClose(h)`, `Hop1(gh)` (dgram), `HopEnd0(hg)`, `HopEnd1(gh)`, `VCtl(gh)`, `FwdErr(h)`, `Write(hg)` / `Fin(hg)` / `Read(gh)` / `ReadEof(gh)` / `ShutRd(gh)` (stream), `ReadEof(hg)`, `TcpRst(h, ·)`, `HolderDrop`, `HolderEnd(k)`, `TrailingRst(·)`, `SendDg(hg, ·)`, `DRead(gh)`, `UdpBlock(gh)`, `UdpUnblock(gh)`, `UdpErr(h)`, `DgHalfClose(h)` | stimulus of the other side or of the guest application's own observation: no guest fact changes until a guest action above delivers it |
//! | `AttachRead`, `AttachHook`, `DropRecord`, `UnclassifiedEof`, `WatchRecord` | hazard-only — an error, never a no-op |
//!
//! Projection (`State::from_driver`, side `g` only). The driver holds the
//! production `GuestFlowOwner` directly (S-7, "Owner field") and reads it
//! only through `guest_observation()`; after the owner's death (`CtxRelease(g)`
//! or `VmmExit`) no owner fact is projected.
//!
//! - S, until the driver applies `VmmExit`: `ctx(g)` ← the guest device's
//!   context table (`open` / `released`, the `SimFlowForwarder` arbiter).
//!   `gone` is stimulus-only and never projected (round-13 classification,
//!   the feature-delta `fwd_pair` map, `ctx(s)` row): the model writes it
//!   only in `VmmExit`, in the step that sets the stimulus `vmUp` false, so
//!   `ctx(g) = gone ⇔ ¬vmUp` in every reachable state (the spec's checked
//!   invariant `CtxGoneIffVmDown`). After `VmmExit` no guest kernel or owner
//!   exists to observe, so the driver gates on the `VmmExit` stimulus it
//!   applied itself (round 14): from that step on it compares none of
//!   `ctx(g)`, the `pr(g)` fields or the guest socket view
//!   ([`WhileVmUp::Ended`]). The applied stimulus is the gate, never a
//!   projection source (testing.md checklist item 5); no `gone` flag is
//!   read. S, always: `gen(g)` ← the number of
//!   contexts the guest device opened, less one (model 0 ↔ the one context
//!   the guest owner opens for its life, QPROJ-8); `oldCtx(g)` ← the context
//!   table.
//! - O, while the guest owner runs (the spec's `ctx(g)` is `open`) and
//!   until the driver applies `VmmExit`:
//!   `pr(g).ph` ← QPROJ-5 (the flow in `flows` / `accepted` / `slots` with its
//!   pair, `awaiting_paired`, or the unattached association) with S
//!   `pair_state(pair)` (`live` iff `Some`); `refused` ⇔ `forwarder_attach_ended`
//!   rose and the flow left. `pr(g).rec` ← `read` ⇔ the owner's
//!   `last_consumed_end` is the flow's pair's record (D-11); else `queued` ⇔ S
//!   the pair's id is in `queued_ends` of the guest's one context row (D-4);
//!   else `none`. `pr(g).out` / `.cause` / `.errno` ← that consumed record's
//!   outcome, compared once `rec = read` — never a record the driver took
//!   from `SimGuestOwnerIo` itself. `closed` ⇔ the owner consumed the record,
//!   the flow left its table and S V_g reports `closed_by_owner()`;
//!   `byRel` ⇔ the flow left the table with no record consumed and none
//!   queued. `attRes(g)`, `attEnded(g)` ← the cumulative
//!   `forwarder_attached` / `forwarder_attach_ended` (QPROJ-8 third
//!   iteration). `dir(gh).srcEnd` / `.srcShut`, `dir(hg).dstEnd` / `.dstShut`
//!   ← S `pair_state(pair)` (`source_end`, `target_shut`), compared while the
//!   pair is live.
//! - S, until the driver applies `VmmExit`: stream / watch: `dir(gh).q0` ← the outer half's
//!   `stream_input()`; `dir(hg).q1` ← V_g's `stream_input()`;
//!   `dir(hg).q2` / `.got` ← the outer half's `stream_output()` /
//!   `peer_received()`; `dir(hg).rst` ← its `peer_was_reset()`.
//!   `tcpErr(g)` ← the outer half's `sk_err()`. `vs(g).sent` ←
//!   V_g's `sent_shutdown()`; `.cls` ← `recorded_class()`; `.st` ←
//!   `vsock_state()`; `.sshut` ← `own_send_shut()`; `.err` ←
//!   `sk_err().is_some()`; `.ps` ← the peer's SHUTDOWN flags
//!   (`received_shutdown()`). dgram: `dg(hg).got` ← the payload sizes the
//!   application received (`peer_datagrams()`), as frame lengths.
//!
//! Not compared: every host-side fact (`pr(h)`, `vs(h)`, `dir(gh).q1/q2/got`,
//! `dir(hg).q0`, `ctl`, `strmAbort`, `hres`, `ep(h)`, `tcpErr(h)`, `dg(gh)`
//! beyond the guest's queue); the guest application's own observations and
//! acts, which are the driver's stimulus (`wr`, `wfin`, `eof`, `shutRd`,
//! `ep(g)`; comparing them would compare the driver with itself, as for
//! `gh.hobs`); the model-only variables (`loss`, `lost`, `tcls`, `ucls`,
//! `att`, `clsAt`, `ncls`, `snap`, `misc`, `gh`, `terr`, `dg` drop sets and
//! `foff`, `conf_step`); `pr(g).nrec`; every O fact once the guest owner
//! died, and every S fact but `gen(g)` and `oldCtx(g)` once the VM ended
//! (gated on the `VmmExit` the driver applied, which is stimulus; the spec's
//! `vmUp` is stimulus too and is not read). Host-side variables are not this
//! driver's; the host driver compares them at every step.
//!
//! **Runs pinned on the host driver only.** `HostOwnerDeathIsFinal` and
//! `RecoveryEndsHeldStreamWhoseWatchEnded` are the host owner's death and its
//! `Forwarder` recovery: their prefix (`Paired`,
//! `WatchedStreamSeesGuestResetAsAbort`) is pinned here, and their own steps
//! (`CtxRelease(h)`, `Reopen(h)`) are host stimulus with no guest delivery,
//! asserting host facts (`gh.hostDead`, `strmAbort`, `hres`, `ctx(h)`,
//! `gen(h)`) that this driver does not project. The host `fwd_pair` driver
//! pins both (testing.md checklist item 6 is per spec main).
//!
//! **Pre-state stash.** `State::from_spec` stashes each step's state; a
//! handler reads the pre-state for the stimulus the trace implies (the unit
//! number a writer writes next, the packet `VCtl(hg)` processes). Stimulus
//! only; never a projection source.

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

use itf::Value;
use nix::libc;
use overdrive_core::guest_transport::LaunchToken;
use overdrive_core::guest_transport::forwarder::{
    AbortCause, ForwardEnd, ForwardId, ForwardOutcome, PairSide, PairState, SourceEnd,
};
use overdrive_core::guest_transport::wire::{
    ControlMessage, FlowId, FlowOrigin, FlowRequest, KernelErrno,
};
use overdrive_core::net::ListenPort;
use overdrive_core::vm::config::VsockPort;
use overdrive_sim::adapters::{SimContextState, SimShutdown, SimSocket, SimVsockState};
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;

use super::quint_itf::{self, ItfError, Qualified};
use super::sim::{SimGuestClock, SimGuestOwnerIo, SimHalf, SimSessionEnd, sim_cookie};
use super::test_support::{GuestProcess, settle, start};
use super::{GuestAcceptPhase, GuestClock, GuestFlowOwner, GuestSideObservation, HostChannel};
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

const MAIN: &str = "pair_conf";
const MODULE: &str = "fwd_pair";
const WL: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 7);
const GW: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
const LAUNCH_HEX: &str = "0123456789abcdef0123456789abcdef";
const DIAL_PORT: u16 = 8080;

/// Why a driver step cannot proceed.
#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("action `{action}` is not enabled in the production state: {why}")]
    NotEnabled { action: &'static str, why: String },
    #[error("action `{0}` is hazard-only: it is never enabled in a design instance")]
    HazardOnly(&'static str),
    #[error("the step cannot be represented in production: {0}")]
    NotRepresentable(String),
    #[error("the trace's {0} is not a value the driver decodes")]
    Decode(String),
}

// ----- the pre-state stash (stimulus, never projection) --------------------------

thread_local! {
    /// The previous step's spec state, written by `State::from_spec`; read by
    /// the next step's handlers for the stimulus the trace implies.
    static SPEC_PRE: RefCell<Option<Qualified>> = const { RefCell::new(None) };
}

fn pre() -> std::result::Result<Qualified, DriverError> {
    SPEC_PRE
        .with(|stash| stash.borrow().clone())
        .ok_or_else(|| DriverError::Decode("pre-state (none stashed)".to_owned()))
}

/// The pre-state's `dir(d).<field>` list of unit numbers.
fn pre_dir_list(d: &str, field: &str) -> std::result::Result<Vec<i64>, DriverError> {
    let vars = pre()?;
    let decode = || -> std::result::Result<Vec<i64>, ItfError> {
        let dirs = quint_itf::str_map(vars.var("dir")?, |x| Ok(x.clone()))?;
        let dir = dirs.get(d).ok_or_else(|| ItfError::MissingField(d.to_owned()))?;
        quint_itf::list_elems(quint_itf::field(dir, field)?)?
            .into_iter()
            .map(quint_itf::int)
            .collect()
    };
    decode().map_err(|e| DriverError::Decode(e.to_string()))
}

/// The pre-state's `dir(d).wr`.
fn pre_dir_wr(d: &str) -> std::result::Result<i64, DriverError> {
    let vars = pre()?;
    let decode = || -> std::result::Result<i64, ItfError> {
        let dirs = quint_itf::str_map(vars.var("dir")?, |x| Ok(x.clone()))?;
        let dir = dirs.get(d).ok_or_else(|| ItfError::MissingField(d.to_owned()))?;
        quint_itf::int(quint_itf::field(dir, "wr")?)
    };
    decode().map_err(|e| DriverError::Decode(e.to_string()))
}

/// The head packet of the pre-state's `ctl(d)` (the flags it carries).
fn pre_ctl_head(d: &str) -> std::result::Result<BTreeSet<String>, DriverError> {
    let vars = pre()?;
    let decode = || -> std::result::Result<BTreeSet<String>, ItfError> {
        let ctl = quint_itf::str_map(vars.var("ctl")?, |x| Ok(x.clone()))?;
        let packets = ctl.get(d).ok_or_else(|| ItfError::MissingField(d.to_owned()))?;
        let head = quint_itf::list_elems(packets)?
            .into_iter()
            .next()
            .ok_or_else(|| ItfError::MissingField(format!("ctl({d}) head")))?;
        quint_itf::str_set(head)
    };
    decode().map_err(|e| DriverError::Decode(e.to_string()))
}

/// The pre-state's `dg(d).n` (datagrams sent so far) and the head of
/// `dg(d).dq0`, with the sizes.
fn pre_dg(d: &str) -> std::result::Result<(i64, Vec<i64>, Vec<i64>, i64), DriverError> {
    let vars = pre()?;
    let decode = || -> std::result::Result<(i64, Vec<i64>, Vec<i64>, i64), ItfError> {
        let dgs = quint_itf::str_map(vars.var("dg")?, |x| Ok(x.clone()))?;
        let dg = dgs.get(d).ok_or_else(|| ItfError::MissingField(d.to_owned()))?;
        let n = quint_itf::int(quint_itf::field(dg, "n")?)?;
        let dq0 = quint_itf::list_elems(quint_itf::field(dg, "dq0")?)?
            .into_iter()
            .map(quint_itf::int)
            .collect::<std::result::Result<_, _>>()?;
        let sizes = quint_itf::list_elems(quint_itf::field(dg, "sizes")?)?
            .into_iter()
            .map(quint_itf::int)
            .collect::<std::result::Result<_, _>>()?;
        let foff = quint_itf::int(quint_itf::field(dg, "foff")?)?;
        Ok((n, dq0, sizes, foff))
    };
    decode().map_err(|e| DriverError::Decode(e.to_string()))
}

// ----- the driver ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Stream,
    Watch,
    Dgram,
}

struct FwdPairGuestDriver {
    mode: Mode,
    /// The production guest flow owner under test, built at construction
    /// (S-7, the "Owner field" ruling): the only path to the owner. After the
    /// guest's death it is kept and never called or projected again.
    owner: GuestFlowOwner,
    /// The guest process's lifecycle (test support beside the owner field).
    process: GuestProcess,
    io: SimGuestOwnerIo,
    clock: Arc<SimGuestClock>,
    /// The flow under test (its production identity).
    flow: Option<FlowId>,
    /// The datagram association's slot.
    slot: Option<SocketAddrV4>,
    /// The guest pair, once the forwarder accepted its attach (relabelling,
    /// fixed when first seen; never a source of state).
    pair: Option<ForwardId>,
    /// The application stopped reading for good (`ShutRd(hg)`).
    app_shut_rd: bool,
    /// The driver applied the trace's `VmmExit` (stimulus, round 14): from
    /// that step on no guest fact is compared ([`WhileVmUp::Ended`]). Never
    /// a projection source.
    vmm_exit_applied: bool,
}

fn host_flow() -> FlowId {
    FlowId::new(FlowOrigin::Host, 1).expect("small seq")
}

fn errno_name(errno: KernelErrno) -> String {
    match i32::from(errno.get()) {
        libc::ECONNRESET => "ECONNRESET".to_owned(),
        libc::EPIPE => "EPIPE".to_owned(),
        other => format!("errno {other}"),
    }
}

impl FwdPairGuestDriver {
    fn new() -> Self {
        SPEC_PRE.with(|stash| stash.borrow_mut().take());
        let clock = Arc::new(SimGuestClock::new());
        let io = SimGuestOwnerIo::new(WL, Arc::clone(&clock));
        let launch = LaunchToken::from_str(LAUNCH_HEX).expect("fixture launch token");
        let token =
            GuestTransportToken { workload_addr: WL, gateway: GW, launch, ports: host_ports() };
        let owner_clock: Arc<dyn GuestClock> = clock.clone();
        let owner = GuestFlowOwner::new(token, Box::new(io.clone()), owner_clock);
        Self {
            mode: Mode::Stream,
            owner,
            process: GuestProcess::NotStarted,
            io,
            clock,
            flow: None,
            slot: None,
            pair: None,
            app_shut_rd: false,
            vmm_exit_applied: false,
        }
    }

    /// Dispatches the live guest owner until quiescent (DG-5-Q) after a
    /// handler has released an owner event.
    fn settle(&mut self) {
        if self.process.is_alive() {
            let (io, clock) = (self.io.clone(), Arc::clone(&self.clock));
            settle(&mut self.owner, &io, &clock);
        }
    }

    /// The guest process ends (the owner is PID 1): the seam's process exit
    /// releases its context and closes every guest socket; the owner field is
    /// kept and never called or projected again (S-7, as the host's S-3).
    fn guest_dies(&mut self) {
        self.io.guest_owner_dies();
        self.process = GuestProcess::Dead;
    }

    fn flow(&self, action: &'static str) -> std::result::Result<FlowId, DriverError> {
        self.flow.ok_or_else(|| DriverError::NotEnabled { action, why: "no flow".to_owned() })
    }

    /// The guest outer half's kernel socket (I, C or S).
    fn outer(&self) -> Option<SimSocket> {
        match self.mode {
            // conformance-lint: sim-arbiter-ok QPROJ-8 the guest pair half's SimSocket (the arbiter of its per-half facts), Tier-3 V-EV-END / V-EV-PAIR / V-EV-DGRAM and the module suite
            Mode::Dgram => self.slot.map(|s| self.io.slot_socket(s)),
            Mode::Stream | Mode::Watch => {
                // conformance-lint: sim-arbiter-ok QPROJ-8 the guest pair half's SimSocket (the arbiter of its per-half facts), Tier-3 V-EV-END / V-EV-PAIR / V-EV-DGRAM and the module suite
                self.flow.and_then(|f| self.io.flow_socket(f, SimHalf::Outer))
            }
        }
    }

    /// V_g's kernel socket.
    fn vsock(&self) -> Option<SimSocket> {
        match self.mode {
            // conformance-lint: sim-arbiter-ok QPROJ-8 the guest pair half's SimSocket (the arbiter of its per-half facts), Tier-3 V-EV-END / V-EV-PAIR / V-EV-DGRAM and the module suite
            Mode::Dgram => self.slot.and_then(|s| self.io.slot_vsock(s)),
            Mode::Stream | Mode::Watch => {
                // conformance-lint: sim-arbiter-ok QPROJ-8 the guest pair half's SimSocket (the arbiter of its per-half facts), Tier-3 V-EV-END / V-EV-PAIR / V-EV-DGRAM and the module suite
                self.flow.and_then(|f| self.io.flow_socket(f, SimHalf::Vsock))
            }
        }
    }

    /// The outer half's side in the guest pair ("Sides and pairs").
    const fn outer_side(&self) -> PairSide {
        match self.mode {
            Mode::Stream | Mode::Dgram => PairSide::A,
            Mode::Watch => PairSide::B,
        }
    }

    const fn vsock_side(&self) -> PairSide {
        match self.outer_side() {
            PairSide::A => PairSide::B,
            PairSide::B => PairSide::A,
        }
    }

    /// One run of the guest pair's direction whose source is `source`.
    fn run(&self, action: &'static str, source: PairSide) -> std::result::Result<(), DriverError> {
        let pair = self
            .pair
            .ok_or_else(|| DriverError::NotEnabled { action, why: "no guest pair".to_owned() })?;
        if !self.io.run_forwarder_once(pair, source) {
            return Err(DriverError::NotEnabled {
                action,
                why: format!("no run of {source:?}'s direction is queued"),
            });
        }
        Ok(())
    }

    fn note_pair(&mut self) {
        if self.pair.is_none() {
            self.pair = self.flow.and_then(|f| self.io.pair_of(f));
        }
    }

    // ----- actions ------------------------------------------------------------------

    fn init_with(&mut self, m: &str) -> std::result::Result<(), DriverError> {
        *self = Self::new();
        self.mode = match m {
            "stream" => Mode::Stream,
            "watch" => Mode::Watch,
            "dgram" => Mode::Dgram,
            other => return Err(DriverError::Decode(format!("mode {other:?}"))),
        };
        let launch = LaunchToken::from_str(LAUNCH_HEX).expect("fixture launch token");
        start(&mut self.owner);
        self.process = GuestProcess::Alive;
        self.settle();
        self.io.complete_host_connect(HostChannel::Control, Ok(()));
        self.settle();
        match self.mode {
            Mode::Stream => {
                self.io
                    .app_connect(sim_cookie(1), SocketAddrV4::new(Ipv4Addr::new(93, 0, 0, 1), 443));
                self.settle();
                self.io.complete_host_connect(HostChannel::FlowStream, Ok(()));
                self.settle();
                self.flow = self.io.requests().iter().find_map(|r| match r {
                    FlowRequest::TcpConnect { flow, .. } => Some(*flow),
                    _ => None,
                });
            }
            Mode::Watch => {
                let listen = ListenPort::from_str(&DIAL_PORT.to_string()).expect("non-zero port");
                self.io.host_open_accept(FlowRequest::TcpAccept {
                    flow: host_flow(),
                    peer: GW,
                    listen_port: listen,
                    launch,
                });
                self.io.release_inbound_accept(host_flow());
                self.settle();
                self.io.release_inbound_request(host_flow());
                self.settle();
                self.flow = Some(host_flow());
                // the application accepts C; the guest owner learns of it at Attach(g)
                self.io.accept_app_connect(host_flow()).peer_stop_reading();
            }
            Mode::Dgram => {
                let slot = self
                    .io
                    .app_send(sim_cookie(1), SocketAddrV4::new(Ipv4Addr::new(93, 0, 0, 1), 53))
                    .ok_or_else(|| DriverError::NotRepresentable("no free slot".to_owned()))?;
                self.io.release_slot_ready(slot);
                self.settle();
                self.io.complete_host_connect(HostChannel::FlowStream, Ok(()));
                self.settle();
                self.slot = Some(slot);
                self.flow = self.io.slot_flow(slot);
                // the priming datagram that started the association is not a
                // model datagram: the application's first `SendDg` is k = 1
                let s = self.io.slot_socket(slot);
                let _primer = s.local_read(usize::MAX);
            }
        }
        if let Some(outer) = self.outer() {
            outer.peer_stop_reading();
        }
        if let Some(vsock) = self.vsock() {
            vsock.peer_stop_reading();
        }
        if self.flow.is_none() {
            return Err(DriverError::NotRepresentable(format!(
                "the guest owner opened no {m} flow"
            )));
        }
        Ok(())
    }

    fn attach_g(&mut self) -> std::result::Result<(), DriverError> {
        let flow = self.flow("Attach")?;
        match self.mode {
            Mode::Stream | Mode::Dgram => self.io.host_control(ControlMessage::Paired { flow }),
            Mode::Watch => {
                self.io.complete_app_connect(flow, Ok(()));
            }
        }
        self.settle();
        // the guest's own Paired (watch) is consumed by the host side
        let _paired = self.io.host_take_first(|m| *m == ControlMessage::Paired { flow });
        self.note_pair();
        if let Some(vsock) = self.vsock()
            && vsock.received_shutdown() == Some(SimShutdown::Both)
            && let Some(pair) = self.pair
        {
            let _ = self.io.run_forwarder_once(pair, self.vsock_side());
        }
        if let Some(vsock) = self.vsock() {
            vsock.peer_stop_reading();
        }
        Ok(())
    }

    fn guest_abort(&mut self) -> std::result::Result<(), DriverError> {
        let flow = self.flow("GuestAbort")?;
        self.io.host_control(ControlMessage::Abort { flow });
        self.settle();
        Ok(())
    }

    fn close_unattached(&mut self) {
        self.io.end_session(SimSessionEnd::Eof);
        self.settle();
    }

    /// `CtxRelease(g)`: the guest owner's death with its context (D-3 to
    /// D-5; the guest owner is PID 1, RESET-EXIT).
    fn ctx_release_g(&mut self) {
        self.guest_dies();
    }

    /// `VmmExit`: the VM ends, and the guest owner with it. The model's
    /// `ctx(g) = gone` is not projected (stimulus-only); the driver records
    /// that it applied the stimulus, which gates every guest comparison
    /// from this step on ([`WhileVmUp`]).
    fn vmm_exit(&mut self) {
        self.vmm_exit_applied = true;
        self.guest_dies();
    }

    /// `TakeEnd(g)`: the owner's dispatch on its context's readiness —
    /// `take_end` and the flow's release (§ *Conformance observation surface
    /// and action map*, `fwd_pair`). The seam releases the context's oldest
    /// record as the owner's `ForwardEnded` event and the owner takes exactly
    /// one dispatch, the one that consumes it; its descriptors' close is a
    /// dispatch of its own (`OwnerClose(g)`).
    fn take_end_g(&mut self) -> std::result::Result<(), DriverError> {
        if !self.process.is_alive() {
            return Err(DriverError::NotEnabled {
                action: "TakeEnd",
                why: "the guest owner is not running".to_owned(),
            });
        }
        if self.io.release_forward_end().is_none() {
            return Err(DriverError::NotEnabled {
                action: "TakeEnd",
                why: "the guest context has no queued record".to_owned(),
            });
        }
        self.owner.dispatch_next().map_err(|fatal| {
            DriverError::NotRepresentable(format!(
                "a GuestOwnerFatal on the dispatch that reads the end record: {fatal}"
            ))
        })
    }

    /// `OwnerClose(g)`: the owner's dispatches after the record, which close
    /// the flow's descriptors (datagram: the slot-queue discard).
    fn owner_close_g(&mut self) {
        self.settle();
    }

    fn vctl_hg(&self) -> std::result::Result<(), DriverError> {
        let flags = pre_ctl_head("hg")?;
        let vsock = self
            .vsock()
            .ok_or_else(|| DriverError::NotEnabled { action: "VCtl", why: "no V_g".to_owned() })?;
        let received_before = vsock.received_shutdown();
        let sent_before = vsock.sent_shutdown();
        let callback_abort = flags.contains("R")
            && matches!(
                (received_before, sent_before),
                (None, _) | (Some(SimShutdown::Send), None)
            );
        if flags.contains("R") {
            vsock.peer_shutdown(SimShutdown::Both);
        } else {
            vsock.peer_shutdown(SimShutdown::Send);
        }
        let context_open = self.io.forwarder_contexts().first().is_some_and(|context| context.open);
        if callback_abort
            && context_open
            && let Some(pair) = self.pair
        {
            let _ = self.io.run_forwarder_once(pair, self.vsock_side());
        }
        Ok(())
    }

    fn app_write(&self) -> std::result::Result<(), DriverError> {
        let k = pre_dir_wr("gh")? + 1;
        let outer = self.outer().ok_or_else(|| DriverError::NotEnabled {
            action: "Write",
            why: "no outer half".to_owned(),
        })?;
        outer.peer_send(&[u8::try_from(k).map_err(|_| DriverError::Decode(format!("unit {k}")))?]);
        Ok(())
    }

    fn host_unit_to_guest(&self) -> std::result::Result<(), DriverError> {
        // the unit the host side moves next: its source queue's head (stream)
        // or the dialer's next write (watch)
        let k = match self.mode {
            Mode::Stream => *pre_dir_list("hg", "q0")?
                .first()
                .ok_or_else(|| DriverError::Decode("dir(hg).q0 head".to_owned()))?,
            Mode::Watch => pre_dir_wr("hg")? + 1,
            Mode::Dgram => return self.host_frame_item_to_guest(),
        };
        let vsock = self
            .vsock()
            .ok_or_else(|| DriverError::NotEnabled { action: "Hop0", why: "no V_g".to_owned() })?;
        vsock.peer_send(&[u8::try_from(k).map_err(|_| DriverError::Decode(format!("unit {k}")))?]);
        Ok(())
    }

    /// Datagram mode: the host forwarder sends the next item of the head
    /// datagram's frame to V_g — the 8-byte header, then one payload byte per
    /// item.
    fn host_frame_item_to_guest(&self) -> std::result::Result<(), DriverError> {
        let (_, dq0, sizes, foff) = pre_dg("hg")?;
        let k = *dq0.first().ok_or_else(|| DriverError::Decode("dg(hg).dq0 head".to_owned()))?;
        let sz = *sizes
            .get(usize::try_from(k - 1).unwrap_or(usize::MAX))
            .ok_or_else(|| DriverError::Decode(format!("size of {k}")))?;
        let vsock = self
            .vsock()
            .ok_or_else(|| DriverError::NotEnabled { action: "Hop0", why: "no V_g".to_owned() })?;
        if foff == 0 {
            let len = u32::try_from(sz).map_err(|_| DriverError::Decode(format!("size {sz}")))?;
            let mut header = b"OVD1".to_vec();
            header.extend_from_slice(&len.to_le_bytes());
            vsock.peer_send(&header);
        } else {
            vsock
                .peer_send(&[u8::try_from(10 * k + foff)
                    .map_err(|_| DriverError::Decode("item".to_owned()))?]);
        }
        Ok(())
    }

    fn host_consumes_one(&self) -> std::result::Result<(), DriverError> {
        let vsock = self
            .vsock()
            .ok_or_else(|| DriverError::NotEnabled { action: "Hop1", why: "no V_g".to_owned() })?;
        vsock.peer_resume_reading();
        vsock.peer_stop_reading();
        Ok(())
    }

    fn app_reads_one(&self) -> std::result::Result<(), DriverError> {
        if self.app_shut_rd {
            return Err(DriverError::NotEnabled {
                action: "Read",
                why: "the application shut its read side".to_owned(),
            });
        }
        let outer = self.outer().ok_or_else(|| DriverError::NotEnabled {
            action: "Read",
            why: "no outer half".to_owned(),
        })?;
        outer.peer_resume_reading();
        outer.peer_stop_reading();
        Ok(())
    }

    fn app_send_dg(&self, sz: i64) -> std::result::Result<(), DriverError> {
        let (n, ..) = pre_dg("gh")?;
        let k = n + 1;
        let payload: Vec<u8> = (1..=sz)
            .map(|i| u8::try_from(10 * k + i).map_err(|_| DriverError::Decode("item".to_owned())))
            .collect::<std::result::Result<_, _>>()?;
        let outer = self.outer().ok_or_else(|| DriverError::NotEnabled {
            action: "SendDg",
            why: "no slot socket".to_owned(),
        })?;
        outer.peer_send(&payload);
        Ok(())
    }

    fn fwd_err_g(&self) -> std::result::Result<(), DriverError> {
        let pair = self.pair.ok_or_else(|| DriverError::NotEnabled {
            action: "FwdErr",
            why: "no guest pair".to_owned(),
        })?;
        let ran = self.io.run_forwarder_once(pair, self.outer_side())
            || self.io.run_forwarder_once(pair, self.vsock_side());
        if !ran {
            return Err(DriverError::NotEnabled {
                action: "FwdErr",
                why: "no run is queued to find the error".to_owned(),
            });
        }
        Ok(())
    }

    fn step_action(action: &'static str, s: &str) -> std::result::Result<(), DriverError> {
        if s == "g" {
            return Err(DriverError::NotRepresentable(format!(
                "`{action}(g)` has no guest production path"
            )));
        }
        Ok(())
    }

    // ----- projection helpers --------------------------------------------------------

    /// The guest pair's phase (`pr(g).ph`) from the owner's observation and
    /// the forwarder arbiter; only while the guest owner runs.
    fn phase(&self, obs: &GuestSideObservation) -> String {
        let Some(flow) = self.flow else { return "unattached".to_owned() };
        let held_pair = match self.mode {
            Mode::Stream => obs.flows.get(&flow).copied(),
            Mode::Watch => match obs.accepted.get(&flow) {
                Some(GuestAcceptPhase::Paired { pair }) => Some(*pair),
                _ => None,
            },
            Mode::Dgram => self.slot.and_then(|s| obs.slots.get(&s)).and_then(|x| x.attached),
        };
        if let Some(p) = held_pair {
            // conformance-lint: sim-arbiter-ok QPROJ-8 SimFlowForwarder pair_state (the guest pair is live), Tier-3 V-EV-PAIR / V-EV-END and the module suite through KernelFlowForwarder
            let live = self.io.forwarder_pair_state(p).is_some();
            return if live { "live" } else { "ended" }.to_owned();
        }
        if self.flow_held(obs) {
            return "unattached".to_owned();
        }
        if obs.forwarder_attach_ended > 0 {
            return "refused".to_owned();
        }
        "ended".to_owned()
    }

    /// The flow is in the owner's table (QPROJ-5: `flows`, `accepted`,
    /// `awaiting_paired`, or its slot's association).
    fn flow_held(&self, obs: &GuestSideObservation) -> bool {
        self.flow.is_some_and(|f| {
            obs.flows.contains_key(&f)
                || obs.accepted.contains_key(&f)
                || obs.awaiting_paired.contains(&f)
                || self
                    .slot
                    .and_then(|s| obs.slots.get(&s))
                    .is_some_and(|x| x.association == Some(f))
        })
    }
}

impl Driver for FwdPairGuestDriver {
    type State = FwdPairGuestState;

    fn config() -> Config {
        Config { state: &[], nondet: &["pair_conf::fwd_pair::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            InitWith(m: String) => self.init_with(&m)?,
            Attach(s: String) => {
                if s == "g" { self.attach_g()? }
            },
            CloseUnattached(s: String) => {
                if s == "g" { self.close_unattached() } else { Self::step_action("CloseUnattached", &s)? }
            },
            Abort(s: String) => {
                if s == "g" { self.guest_abort()? }
            },
            GuestAbort(s: String) => {
                if s == "g" { self.guest_abort()? } else { Self::step_action("GuestAbort", &s)? }
            },
            CtxRelease(s: String) => {
                if s == "g" { self.ctx_release_g() }
            },
            Reopen(s: String) => Self::step_action("Reopen", &s)?,
            TakeEnd(s: String) => {
                if s == "g" { self.take_end_g()? }
            },
            OwnerClose(s: String) => {
                if s == "g" { self.owner_close_g() }
            },
            Hop0(d: String) => {
                if d == "gh" { self.run("Hop0", self.outer_side())? } else { self.host_unit_to_guest()? }
            },
            Hop1(d: String) => {
                if d == "hg" { self.run("Hop1", self.vsock_side())? }
                else if self.mode != Mode::Dgram { self.host_consumes_one()? }
            },
            HopEnd0(d: String) => {
                if d == "gh" { self.run("HopEnd0", self.outer_side())? }
            },
            HopEnd1(d: String) => {
                if d == "hg" { self.run("HopEnd1", self.vsock_side())? }
            },
            VCtl(d: String) => {
                if d == "hg" { self.vctl_hg()? }
            },
            FwdErr(s: String) => {
                if s == "g" { self.fwd_err_g()? }
            },
            Write(d: String) => {
                if d == "gh" { self.app_write()? }
                else if self.mode == Mode::Watch { self.host_unit_to_guest()? }
            },
            Fin(d: String) => {
                if d == "gh" {
                    let outer = self.outer().ok_or_else(|| DriverError::NotEnabled { action: "Fin", why: "no outer half".to_owned() })?;
                    outer.peer_shutdown(SimShutdown::Send);
                }
            },
            Read(d: String) => {
                if d == "hg" { self.app_reads_one()? }
                else if self.mode == Mode::Watch { self.host_consumes_one()? }
            },
            ReadEof(d: String) => {
                if self.mode == Mode::Dgram {
                    return Err(DriverError::NotEnabled {
                        action: "ReadEof",
                        why: "datagrams have no stream EOF".to_owned(),
                    }.into());
                }
                // Application/holder observation only: validate the trace's
                // approved pre-state signal without consuming a pending owner
                // dispatch or manufacturing a socket effect.
                pre_dir_list(&d, "q2")?;
            },
            ShutRd(d: String) => {
                if d == "hg" { self.app_shut_rd = true; }
            },
            TcpRst(s: String) => {
                if s == "g" {
                    let outer = self.outer().ok_or_else(|| DriverError::NotEnabled { action: "TcpRst", why: "no outer half".to_owned() })?;
                    outer.peer_reset();
                    if let Some(pair) = self.pair {
                        let _ = self.io.run_forwarder_once(pair, self.outer_side());
                    }
                }
            },
            HolderDrop => {
                if self.mode != Mode::Watch {
                    return Err(DriverError::NotEnabled {
                        action: "HolderDrop",
                        why: "no watch holder".to_owned(),
                    }.into());
                }
                pre_dir_list("gh", "q2")?;
            },
            VmmExit => self.vmm_exit(),
            TrailingRst(s: String) => {
                let direction = match s.as_str() {
                    "g" => "hg",
                    "h" => "gh",
                    other => return Err(DriverError::Decode(format!("side {other:?}")).into()),
                };
                pre_dir_list(direction, "q2")?;
            },
            SendDg(d: String, sz: i64) => {
                if d == "gh" { self.app_send_dg(sz)? }
            },
            DRead(d: String) => {
                if d == "hg" { self.app_reads_one()? }
            },
            UdpBlock(d: String) => {
                if d == "hg" && let Some(outer) = self.outer() { outer.peer_stop_reading(); }
            },
            UdpUnblock(d: String) => {
                if d == "hg" && let Some(outer) = self.outer() { outer.peer_resume_reading(); }
            },
            UdpErr(s: String) => {
                if s == "g" {
                    let outer = self.outer().ok_or_else(|| DriverError::NotEnabled { action: "UdpErr", why: "no slot socket".to_owned() })?;
                    outer.set_sk_err(KernelErrno::new(u16::try_from(libc::ECONNREFUSED).expect("errno")).expect("non-zero"));
                }
            },
            DgHalfClose(s: String) => {
                if s == "g" {
                    let vsock = self.vsock().ok_or_else(|| DriverError::NotEnabled { action: "DgHalfClose", why: "no V_g".to_owned() })?;
                    vsock.shutdown_send();
                }
            },
            HolderEnd(k: String) => {
                if self.mode != Mode::Watch {
                    return Err(DriverError::NotEnabled {
                        action: "HolderEnd",
                        why: "no watch holder".to_owned(),
                    }.into());
                }
                if !matches!(k.as_str(), "eof" | "err") {
                    return Err(DriverError::Decode(format!("holder end {k:?}")).into());
                }
                pre_dir_list("gh", "q2")?;
            },
            AttachRead => return Err(DriverError::HazardOnly("AttachRead").into()),
            AttachHook => return Err(DriverError::HazardOnly("AttachHook").into()),
            DropRecord => return Err(DriverError::HazardOnly("DropRecord").into()),
            UnclassifiedEof => return Err(DriverError::HazardOnly("UnclassifiedEof").into()),
            WatchRecord => return Err(DriverError::HazardOnly("WatchRecord").into()),
        })
    }
}

// ----- the compared state ------------------------------------------------------

/// The guest pair's end, compared once the owner consumed the record.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EndView {
    out: String,
    cause: String,
    errno: String,
}

/// The guest owner's facts (O, with the forwarder facts they are read
/// against), compared only while the guest owner runs — the spec's `ctx(g)`
/// is `open`. After the owner's death no owner is projected (S-7, as the
/// host side's S-3).
#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnerView {
    ph: String,
    rec: String,
    closed: bool,
    by_rel: bool,
    end: Option<EndView>,
    att_res: String,
    att_ended: i64,
    /// While the pair is live: the guest pair's per-direction state.
    dirs: Option<DirsView>,
}

/// The guest sockets' facts (S, the Sim sockets' arbiters), compared until
/// the VM ended.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SockView {
    tcp_err: String,
    vs: VsView,
    /// Stream / watch: the guest halves' queues and the application's reads.
    queues: Option<QueuesView>,
    /// Datagram mode: the payload sizes the application received, as frame
    /// lengths.
    dg_got: Option<Vec<i64>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VsView {
    ps: BTreeSet<String>,
    sent: BTreeSet<String>,
    cls: String,
    st: String,
    sshut: bool,
    err: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirsView {
    src_end_gh: String,
    src_shut_gh: bool,
    dst_end_hg: String,
    dst_shut_hg: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueuesView {
    q0_gh: Vec<i64>,
    q1_hg: Vec<i64>,
    q2_hg: Vec<i64>,
    got_hg: Vec<i64>,
    rst_hg: bool,
}

/// A guest-side fact compared only until the driver applies `VmmExit`.
///
/// The spec side always decodes the spec's value. The driver side is
/// [`WhileVmUp::Ended`] once it applied the trace's `VmmExit` stimulus
/// (round 14; `ctx(g) = gone ⇔ ¬vmUp` in every reachable state, the
/// `fwd_pair` map), and otherwise reads the arbiter or the owner. `Ended`
/// compares equal to any value: the comparison is skipped, not satisfied by
/// a driver-chosen value (testing.md checklist item 5). Before `VmmExit` a
/// spec `ctx(g) = gone` meets a driver-read `open` / `released` and fails.
#[derive(Debug, Clone)]
enum WhileVmUp<T> {
    Read(T),
    Ended,
}

impl<T: PartialEq> PartialEq for WhileVmUp<T> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Read(a), Self::Read(b)) => a == b,
            (Self::Ended, _) | (_, Self::Ended) => true,
        }
    }
}

/// The compared `fwd_pair` variables, guest side.
#[derive(Debug, PartialEq)]
struct FwdPairGuestState {
    /// Not compared once the driver applied `VmmExit` ([`WhileVmUp`]).
    ctx_g: WhileVmUp<String>,
    gen_g: i64,
    old_ctx_g: String,
    /// `None` once the guest owner died (`ctx(g)` is not `open`); not
    /// compared once the driver applied `VmmExit` ([`WhileVmUp`]).
    owner: WhileVmUp<Option<OwnerView>>,
    /// Not compared once the driver applied `VmmExit` ([`WhileVmUp`]).
    sockets: WhileVmUp<SockView>,
}

fn source_end_name(end: Option<SourceEnd>) -> String {
    match end {
        Some(SourceEnd::Clean) => "clean".to_owned(),
        Some(SourceEnd::Abort) | None => "none".to_owned(),
    }
}

/// A SHUTDOWN flag set as the model's set (`{S}`, `{S, R}`).
fn shutdown_set(flags: Option<SimShutdown>) -> BTreeSet<String> {
    match flags {
        Some(SimShutdown::Send) => std::iter::once("S".to_owned()).collect(),
        Some(SimShutdown::Both) => ["S".to_owned(), "R".to_owned()].into_iter().collect(),
        None => BTreeSet::new(),
    }
}

impl FwdPairGuestState {
    fn from_vars(vars: &Qualified) -> std::result::Result<Self, ItfError> {
        let str_of = |name: &str| -> std::result::Result<BTreeMap<String, Value>, ItfError> {
            quint_itf::str_map(vars.var(name)?, |x| Ok(x.clone()))
        };
        let side =
            |map: &BTreeMap<String, Value>, s: &str| -> std::result::Result<Value, ItfError> {
                map.get(s).cloned().ok_or_else(|| ItfError::MissingField(s.to_owned()))
            };
        let mode = quint_itf::string(vars.var("mode")?)?;
        // The spec side decodes every compared variable; the gate after
        // `VmmExit` is the driver's applied stimulus ([`WhileVmUp`]).
        let ctx_g = quint_itf::string(&side(&str_of("ctx")?, "g")?)?;
        let gen_g = quint_itf::int(&side(&str_of("gen")?, "g")?)?;
        let old_ctx_g = quint_itf::string(&side(&str_of("oldCtx")?, "g")?)?;
        let pr = side(&str_of("pr")?, "g")?;
        let field_s = |v: &Value, f: &str| quint_itf::string(quint_itf::field(v, f)?);
        let dirs_map = str_of("dir")?;
        let gh = side(&dirs_map, "gh")?;
        let hg = side(&dirs_map, "hg")?;
        let owner = if ctx_g == "open" {
            let ph = field_s(&pr, "ph")?;
            let rec = field_s(&pr, "rec")?;
            let end = if rec == "read" {
                Some(EndView {
                    out: field_s(&pr, "out")?,
                    cause: field_s(&pr, "cause")?,
                    errno: field_s(&pr, "errno")?,
                })
            } else {
                None
            };
            let dirs = (ph == "live")
                .then(|| -> std::result::Result<DirsView, ItfError> {
                    Ok(DirsView {
                        src_end_gh: field_s(&gh, "srcEnd")?,
                        src_shut_gh: quint_itf::boolean(quint_itf::field(&gh, "srcShut")?)?,
                        dst_end_hg: field_s(&hg, "dstEnd")?,
                        dst_shut_hg: quint_itf::boolean(quint_itf::field(&hg, "dstShut")?)?,
                    })
                })
                .transpose()?;
            Some(OwnerView {
                ph,
                rec,
                closed: quint_itf::boolean(quint_itf::field(&pr, "closed")?)?,
                by_rel: quint_itf::boolean(quint_itf::field(&pr, "byRel")?)?,
                end,
                att_res: quint_itf::string(&side(&str_of("attRes")?, "g")?)?,
                att_ended: quint_itf::int(&side(&str_of("attEnded")?, "g")?)?,
                dirs,
            })
        } else {
            None
        };
        let sockets = {
            let ints = |v: &Value, f: &str| -> std::result::Result<Vec<i64>, ItfError> {
                quint_itf::list_elems(quint_itf::field(v, f)?)?
                    .into_iter()
                    .map(quint_itf::int)
                    .collect()
            };
            let queues = (mode != "dgram")
                .then(|| -> std::result::Result<QueuesView, ItfError> {
                    Ok(QueuesView {
                        q0_gh: ints(&gh, "q0")?,
                        q1_hg: ints(&hg, "q1")?,
                        q2_hg: ints(&hg, "q2")?,
                        got_hg: ints(&hg, "got")?,
                        rst_hg: quint_itf::boolean(quint_itf::field(&hg, "rst")?)?,
                    })
                })
                .transpose()?;
            let dg_got = (mode == "dgram")
                .then(|| -> std::result::Result<Vec<i64>, ItfError> {
                    let dg = side(&str_of("dg")?, "hg")?;
                    quint_itf::list_elems(quint_itf::field(&dg, "got")?)?
                        .into_iter()
                        .map(|f| {
                            quint_itf::list_elems(f)
                                .map(|items| i64::try_from(items.len()).unwrap_or(i64::MAX))
                        })
                        .collect()
                })
                .transpose()?;
            let vs = side(&str_of("vs")?, "g")?;
            WhileVmUp::Read(SockView {
                tcp_err: quint_itf::string(&side(&str_of("tcpErr")?, "g")?)?,
                vs: VsView {
                    ps: quint_itf::str_set(quint_itf::field(&vs, "ps")?)?,
                    sent: quint_itf::str_set(quint_itf::field(&vs, "sent")?)?,
                    cls: field_s(&vs, "cls")?,
                    st: field_s(&vs, "st")?,
                    sshut: quint_itf::boolean(quint_itf::field(&vs, "sshut")?)?,
                    err: quint_itf::boolean(quint_itf::field(&vs, "err")?)?,
                },
                queues,
                dg_got,
            })
        };
        Ok(Self {
            ctx_g: WhileVmUp::Read(ctx_g),
            gen_g,
            old_ctx_g,
            owner: WhileVmUp::Read(owner),
            sockets,
        })
    }
}

impl<'de> Deserialize<'de> for FwdPairGuestState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let vars = Qualified::of(&value, MAIN, MODULE).map_err(serde::de::Error::custom)?;
        Self::from_vars(&vars).map_err(serde::de::Error::custom)
    }
}

impl FwdPairGuestDriver {
    /// The end record's outcome in the model's vocabulary. `errno` is the
    /// `SocketError` errno of the TCP outer half's reset (T-1); `none`
    /// otherwise.
    fn end_view(&self, end: &ForwardEnd) -> EndView {
        let (out, cause, errno) = match end.outcome {
            ForwardOutcome::Completed => ("completed", "none", "none".to_owned()),
            ForwardOutcome::GuestHalfClose => ("halfclose", "none", "none".to_owned()),
            ForwardOutcome::Aborted { cause } => {
                let (name, errno) = match cause {
                    AbortCause::PeerAbort { .. } => ("PeerAbort", "none".to_owned()),
                    AbortCause::SocketError { side, errno } => (
                        "SocketError",
                        if side == self.outer_side() && self.mode != Mode::Dgram {
                            errno_name(errno)
                        } else {
                            "none".to_owned()
                        },
                    ),
                    AbortCause::FrameInvalid { .. } => ("FrameInvalid", "none".to_owned()),
                    AbortCause::Detached => ("Detached", "none".to_owned()),
                    AbortCause::Unclassified { .. } => ("Unclassified", "none".to_owned()),
                };
                ("aborted", name, errno)
            }
        };
        EndView { out: out.to_owned(), cause: cause.to_owned(), errno }
    }

    /// The guest owner's facts (O), read through its pinned accessor
    /// `guest_observation()` — the flow tables, the cumulative attach
    /// counters, and the record it consumed (`last_consumed_end`, D-11) — with
    /// the forwarder facts they are read against (S, `SimFlowForwarder`).
    /// Only while the guest owner runs.
    fn owner_view(&self, contexts: &[SimContextState]) -> OwnerView {
        let obs = self.owner.guest_observation();
        let ph = self.phase(&obs);
        let held = self.flow_held(&obs);
        // D-4: the flow's record in the end queue of the context that
        // attached it — the guest's one context (row 0).
        let queued = self.pair.is_some_and(|pair| {
            contexts.first().is_some_and(|row| row.queued_ends.contains(&pair))
        });
        // D-11: the record the owner consumed, when it is this flow's pair's.
        let consumed = obs.last_consumed_end.filter(|end| Some(end.id) == self.pair);
        let rec = if consumed.is_some() {
            "read"
        } else if queued {
            "queued"
        } else {
            "none"
        };
        // conformance-lint: sim-arbiter-ok QPROJ-8 closed: the retained V_g SimSocket records its userspace-owner close, distinct from the forwarder's earlier SHUT_RDWR, Tier-3 V-EV-END / V-EV-CTL and the module suite
        let vsock_closed = self.vsock().is_some_and(|v| v.closed_by_owner());
        let closed = consumed.is_some() && !held && vsock_closed;
        let by_rel = self.flow.is_some() && !held && consumed.is_none() && !queued;
        let att_ended = i64::try_from(obs.forwarder_attach_ended).unwrap_or(i64::MAX);
        let att_res = if att_ended > 0 {
            "halfEnded"
        } else if obs.forwarder_attached > 0 {
            "ok"
        } else {
            "none"
        };
        // conformance-lint: sim-arbiter-ok QPROJ-8 SimFlowForwarder pair_state (per-direction ends and target shutdowns), Tier-3 V-EV-END / V-EV-PAIR and the module suite
        let state: Option<PairState> = self.pair.and_then(|p| self.io.forwarder_pair_state(p));
        let (gh, hg) = match (state, self.outer_side()) {
            (Some(s), PairSide::A) => (Some(s.a_to_b), Some(s.b_to_a)),
            (Some(s), PairSide::B) => (Some(s.b_to_a), Some(s.a_to_b)),
            (None, _) => (None, None),
        };
        let dirs = (ph == "live").then(|| DirsView {
            src_end_gh: source_end_name(gh.and_then(|d| d.source_end)),
            src_shut_gh: gh.is_some_and(|d| d.target_shut),
            dst_end_hg: source_end_name(hg.and_then(|d| d.source_end)),
            dst_shut_hg: hg.is_some_and(|d| d.target_shut),
        });
        OwnerView {
            ph,
            rec: rec.to_owned(),
            closed,
            by_rel,
            end: consumed.as_ref().map(|end| self.end_view(end)),
            att_res: att_res.to_owned(),
            att_ended,
            dirs,
        }
    }

    /// The guest sockets' facts (S): the Sim sockets of the pair's halves,
    /// readable after the owner closed them (D-5).
    fn sockets_view(&self) -> SockView {
        // conformance-lint: sim-arbiter-ok QPROJ-7 / QPROJ-8 SimSocket per-half facts (sk_err, queues, far reader), Tier-3 V-EV-END / V-EV-PAIR / V-EV-DGRAM and the module suite
        let outer = self.outer();
        // conformance-lint: sim-arbiter-ok QPROJ-8 SimSocket V_g per-half facts, Tier-3 V-EV-END / V-EV-CTL and the module suite
        let vsock = self.vsock();
        // conformance-lint: sim-arbiter-ok QPROJ-7 / QPROJ-8 SimSocket per-half facts, Tier-3 V-EV-END / V-EV-PAIR / V-EV-DGRAM and the module suite
        let tcp_err = match (self.mode, outer.as_ref().and_then(SimSocket::sk_err)) {
            (Mode::Dgram, _) | (_, None) => "0".to_owned(),
            (_, Some(errno)) => errno_name(errno),
        };
        let vs = vsock.as_ref().map_or_else(
            || VsView {
                ps: BTreeSet::new(),
                sent: BTreeSet::new(),
                cls: "none".to_owned(),
                st: "est".to_owned(),
                sshut: false,
                err: false,
            },
            |v| {
                VsView {
                    // conformance-lint: sim-arbiter-ok QPROJ-8 vs(g).ps: V_g's peer_shutdown flags, Tier-3 V-EV-END and the module suite
                    ps: shutdown_set(v.received_shutdown()),
                    // conformance-lint: sim-arbiter-ok QPROJ-7 / QPROJ-8 SimSocket per-half facts, Tier-3 V-EV-END / V-EV-PAIR / V-EV-DGRAM and the module suite
                    sent: shutdown_set(v.sent_shutdown()),
                    cls: if self.pair.is_some() && v.recorded_class() == Some(SourceEnd::Clean) {
                        "clean"
                    } else {
                        "none"
                    }
                    .to_owned(),
                    st: match v.vsock_state() {
                        // conformance-lint: sim-arbiter-ok QPROJ-7 / QPROJ-8 SimSocket per-half facts, Tier-3 V-EV-END / V-EV-PAIR / V-EV-DGRAM and the module suite
                        SimVsockState::Established => "est",
                        // conformance-lint: sim-arbiter-ok QPROJ-7 / QPROJ-8 SimSocket per-half facts, Tier-3 V-EV-END / V-EV-PAIR / V-EV-DGRAM and the module suite
                        SimVsockState::Closing => "closing",
                    }
                    .to_owned(),
                    sshut: v.own_send_shut(),
                    err: v.sk_err().is_some(),
                }
            },
        );
        let units = |bytes: Vec<u8>| bytes.into_iter().map(i64::from).collect();
        let queues = (self.mode != Mode::Dgram).then(|| {
            let got_hg: Vec<i64> = outer
                .as_ref()
                .map(|o| o.peer_received().iter().map(|b| i64::from(*b)).collect())
                .unwrap_or_default();
            QueuesView {
                // conformance-lint: sim-arbiter-ok QPROJ-7 / QPROJ-8 SimSocket per-half facts, Tier-3 V-EV-END / V-EV-PAIR / V-EV-DGRAM and the module suite
                q0_gh: outer.as_ref().map_or_else(Vec::new, |socket| units(socket.stream_input())),
                // conformance-lint: sim-arbiter-ok QPROJ-7 / QPROJ-8 SimSocket per-half facts, Tier-3 V-EV-END / V-EV-PAIR / V-EV-DGRAM and the module suite
                q1_hg: vsock.as_ref().map_or_else(Vec::new, |socket| units(socket.stream_input())),
                q2_hg: outer.as_ref().map_or_else(Vec::new, |socket| units(socket.stream_output())),
                got_hg,
                // conformance-lint: sim-arbiter-ok QPROJ-7 / QPROJ-8 SimSocket per-half facts, Tier-3 V-EV-END / V-EV-PAIR / V-EV-DGRAM and the module suite
                rst_hg: outer.as_ref().is_some_and(SimSocket::peer_was_reset),
            }
        });
        let dg_got = (self.mode == Mode::Dgram).then(|| {
            outer
                .as_ref()
                .map(|o| {
                    o.peer_datagrams()
                        .iter()
                        .map(|d| i64::try_from(d.len()).unwrap_or(i64::MAX) + 1)
                        .collect()
                })
                .unwrap_or_default()
        });
        SockView { tcp_err, vs, queues, dg_got }
    }
}

impl State<FwdPairGuestDriver> for FwdPairGuestState {
    /// Decodes the compared variables by qualified name and stashes the state
    /// for the next step's stimulus (never a projection source).
    fn from_spec(value: Value) -> Result<Self> {
        let vars = Qualified::of(&value, MAIN, MODULE)?;
        let state = Self::from_vars(&vars)?;
        SPEC_PRE.with(|stash| *stash.borrow_mut() = Some(vars));
        Ok(state)
    }

    fn from_driver(driver: &FwdPairGuestDriver) -> Result<Self> {
        // conformance-lint: sim-arbiter-ok QPROJ-8 SimFlowForwarder context table (the guest device's contexts, open or released), Tier-3 P-70 § 3.2 / § 3.8 in the module suite
        let contexts = driver.io.forwarder_contexts();
        // Round 14: from the `VmmExit` the driver applied (stimulus, never a
        // projection source) no guest fact is compared — not `ctx(g)`, not
        // the `pr(g)` fields, not the guest socket view.
        let ended = driver.vmm_exit_applied;
        // `open` / `released` from the arbiter; never `gone` (stimulus-only).
        let ctx_g = if ended {
            WhileVmUp::Ended
        } else {
            WhileVmUp::Read(
                if contexts.last().is_some_and(|c| c.open) { "open" } else { "released" }
                    .to_owned(),
            )
        };
        // QPROJ-8: model 0 ↔ the guest's first context; the guest owner opens
        // one context for its life and is never repaired.
        let gen_g = i64::try_from(contexts.len().saturating_sub(1))?;
        let old_ctx_g = contexts.len().checked_sub(2).and_then(|i| contexts.get(i)).map_or_else(
            || "none".to_owned(),
            |old| if old.open { "open" } else { "released" }.to_owned(),
        );
        let (owner, sockets) = if ended {
            (WhileVmUp::Ended, WhileVmUp::Ended)
        } else {
            (
                WhileVmUp::Read(driver.process.is_alive().then(|| driver.owner_view(&contexts))),
                WhileVmUp::Read(driver.sockets_view()),
            )
        };
        Ok(Self { ctx_g, gen_g, old_ctx_g, owner, sockets })
    }
}

// ----- the named runs (critical scenarios, `pair_conf`) ---------------------------

/// Named run `Paired` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "Paired"
)]
fn quint_fwd_pair_guest_paired() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `CleanBothWaysCompletes` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "CleanBothWaysCompletes"
)]
fn quint_fwd_pair_guest_clean_both_ways_completes() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `TargetStallBlocksSenderThenResumes` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "TargetStallBlocksSenderThenResumes"
)]
fn quint_fwd_pair_guest_target_stall_blocks_sender_then_resumes() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `EarlyBytesBeforeAttachDelivered` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "EarlyBytesBeforeAttachDelivered"
)]
fn quint_fwd_pair_guest_early_bytes_before_attach_delivered() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `EarlyFinBeforeAttachCarriedOut` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "EarlyFinBeforeAttachCarriedOut"
)]
fn quint_fwd_pair_guest_early_fin_before_attach_carried_out() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `GuestAppResetReachesRemoteAsReset` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "GuestAppResetReachesRemoteAsReset"
)]
fn quint_fwd_pair_guest_guest_app_reset_reaches_remote_as_reset() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `RemoteResetReachesGuestAppAsReset` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "RemoteResetReachesGuestAppAsReset"
)]
fn quint_fwd_pair_guest_remote_reset_reaches_guest_app_as_reset() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `HalfCloseThenPeerCloseIsClean` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "HalfCloseThenPeerCloseIsClean"
)]
fn quint_fwd_pair_guest_half_close_then_peer_close_is_clean() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `PeerCloseWithDirectionOpenAborts` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "PeerCloseWithDirectionOpenAborts"
)]
fn quint_fwd_pair_guest_peer_close_with_direction_open_aborts() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `ReadShutdownPeerStallsLikeSlowReader` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "ReadShutdownPeerStallsLikeSlowReader"
)]
fn quint_fwd_pair_guest_read_shutdown_peer_stalls_like_slow_reader() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `OwnerAbortResetsBothFarEnds` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "OwnerAbortResetsBothFarEnds"
)]
fn quint_fwd_pair_guest_owner_abort_resets_both_far_ends() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `ContextReleaseResetsEveryPair` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "ContextReleaseResetsEveryPair"
)]
fn quint_fwd_pair_guest_context_release_resets_every_pair() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `VmmExitAbortsHostHalf` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "VmmExitAbortsHostHalf"
)]
fn quint_fwd_pair_guest_vmm_exit_aborts_host_half() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `TrailingRstAfterCompletionIsInert` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "TrailingRstAfterCompletionIsInert"
)]
fn quint_fwd_pair_guest_trailing_rst_after_completion_is_inert() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `WatchedStreamSeesGuestResetAsAbort` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "WatchedStreamSeesGuestResetAsAbort"
)]
fn quint_fwd_pair_guest_watched_stream_sees_guest_reset_as_abort() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `DgramPaired` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "DgramPaired"
)]
fn quint_fwd_pair_guest_dgram_paired() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `DatagramsInOrderWithEmpty` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "DatagramsInOrderWithEmpty"
)]
fn quint_fwd_pair_guest_datagrams_in_order_with_empty() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `PartialFrameResumedNotDuplicated` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "PartialFrameResumedNotDuplicated"
)]
fn quint_fwd_pair_guest_partial_frame_resumed_not_duplicated() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `StalledUdpTargetDropsAtSource` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "StalledUdpTargetDropsAtSource"
)]
fn quint_fwd_pair_guest_stalled_udp_target_drops_at_source() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `HostHalfCloseThenAbortBeforeGuestAttachIsAbort` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "HostHalfCloseThenAbortBeforeGuestAttachIsAbort"
)]
fn quint_fwd_pair_guest_host_half_close_then_abort_before_guest_attach_is_abort() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `GuestCloseBeforeHostAttachAborts` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "GuestCloseBeforeHostAttachAborts"
)]
fn quint_fwd_pair_guest_guest_close_before_host_attach_aborts() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `RemoteFinThenResetAbortsPair` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "RemoteFinThenResetAbortsPair"
)]
fn quint_fwd_pair_guest_remote_fin_then_reset_aborts_pair() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `ForwarderRecoveryAbortsThenReopens` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "ForwarderRecoveryAbortsThenReopens"
)]
fn quint_fwd_pair_guest_forwarder_recovery_aborts_then_reopens() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `ReleasedWatchWithNoRecordReadsAsAbort` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "ReleasedWatchWithNoRecordReadsAsAbort"
)]
fn quint_fwd_pair_guest_released_watch_with_no_record_reads_as_abort() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `VmmExitWithWatchRecordInFlightResolvesFromRecord` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "VmmExitWithWatchRecordInFlightResolvesFromRecord"
)]
fn quint_fwd_pair_guest_vmm_exit_with_watch_record_in_flight_resolves_from_record() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `TruncatedFrameAtPeerEndAbortsFrameInvalid` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "TruncatedFrameAtPeerEndAbortsFrameInvalid"
)]
fn quint_fwd_pair_guest_truncated_frame_at_peer_end_aborts_frame_invalid() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `GuestAppFinThenResetAbortsPairWithEpipe` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "GuestAppFinThenResetAbortsPairWithEpipe"
)]
fn quint_fwd_pair_guest_guest_app_fin_then_reset_aborts_pair_with_epipe() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `EarlyVsockHalfCloseBeforeGuestAttachCompletes` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "EarlyVsockHalfCloseBeforeGuestAttachCompletes"
)]
fn quint_fwd_pair_guest_early_vsock_half_close_before_guest_attach_completes() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `AppWriteAndFinBeforePairedAttachesInCloseWaitAndCompletes` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "AppWriteAndFinBeforePairedAttachesInCloseWaitAndCompletes"
)]
fn quint_fwd_pair_guest_app_write_and_fin_before_paired_attaches_in_close_wait_and_completes()
-> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `IntakeChildResetBeforePairedIsRefusedAndFlowAborts` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "IntakeChildResetBeforePairedIsRefusedAndFlowAborts"
)]
fn quint_fwd_pair_guest_intake_child_reset_before_paired_is_refused_and_flow_aborts() -> impl Driver
{
    FwdPairGuestDriver::new()
}

/// Named run `DestinationResetBeforeHostAttachIsRefusedAndFlowAborts` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "DestinationResetBeforeHostAttachIsRefusedAndFlowAborts"
)]
fn quint_fwd_pair_guest_destination_reset_before_host_attach_is_refused_and_flow_aborts()
-> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `GuestCloseWithEmptyQueueBeforeHostAttachIsRefusedAndFlowAborts` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "GuestCloseWithEmptyQueueBeforeHostAttachIsRefusedAndFlowAborts"
)]
fn quint_fwd_pair_guest_guest_close_with_empty_queue_before_host_attach_is_refused_and_flow_aborts()
-> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `OwnerAbortOfWatchLeavesVhAndStreamReportsAborted` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "OwnerAbortOfWatchLeavesVhAndStreamReportsAborted"
)]
fn quint_fwd_pair_guest_owner_abort_of_watch_leaves_vh_and_stream_reports_aborted() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `OwnerAbortSendsGuestAbortThenHolderDropIsInert` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "OwnerAbortSendsGuestAbortThenHolderDropIsInert"
)]
fn quint_fwd_pair_guest_owner_abort_sends_guest_abort_then_holder_drop_is_inert() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `ContextReleaseShutsWatchedVhAndHolderReadsNotConnected` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "ContextReleaseShutsWatchedVhAndHolderReadsNotConnected"
)]
fn quint_fwd_pair_guest_context_release_shuts_watched_vh_and_holder_reads_not_connected()
-> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `DroppedStreamAbortsWatchThenGuestSeesAbort` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "DroppedStreamAbortsWatchThenGuestSeesAbort"
)]
fn quint_fwd_pair_guest_dropped_stream_aborts_watch_then_guest_sees_abort() -> impl Driver {
    FwdPairGuestDriver::new()
}

/// Named run `PeerCloseRacingOwnHalfCloseIsClean` (`pair_conf`), guest side.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "PeerCloseRacingOwnHalfCloseIsClean"
)]
fn quint_fwd_pair_guest_peer_close_racing_own_half_close_is_clean() -> impl Driver {
    FwdPairGuestDriver::new()
}

// ----- randomized traces -------------------------------------------------------------

/// Randomized traces of the conformance main (seed printed on failure).
/// CONTRACT_SHAPE: bounded-change.
#[quint_run(spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt", main = "pair_conf")]
fn quint_fwd_pair_pair_conf_guest() -> impl Driver {
    FwdPairGuestDriver::new()
}
