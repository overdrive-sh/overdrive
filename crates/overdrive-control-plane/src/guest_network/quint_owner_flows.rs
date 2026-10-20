//! Quint conformance driver — `owner_flows`, conformance main `flows_conf`,
//! host side (QPROJ-1, QPROJ-2, QPROJ-3, QPROJ-7; § *Per-module projection
//! and action map* "`owner_flows` `flows_conf`, host side"; D-UP-1).
//!
//! Replays `specs/quint/guest-flow-owner/owner_flows.qnt` (referenced in
//! place) on its recording main `flows_conf` (`CFG = "core2"`, two dial
//! slots, `REC = true`) against the production host guest-flow owner
//! (`HostSharedGuestNetworkOwner::with_flow_io`, also the production
//! `GuestStreamDial`) and the production pool (`GuestAddressPool`), over the
//! seam sim `SimGuestFlowIo` and its `SimFlowForwarder` arbiter. Every
//! accept and record delivery is released by the trace
//! (`set_auto_deliver(false)`) and the owner's peer-socket open and its
//! connect are held, so the host steps between them are the trace's. The
//! guest is stimulus (QPROJ-1). The step record is read through
//! `Config::nondet = &["flows_conf::owner_flows::conf_step"]` and the state
//! is projected by the spec's qualified names (`flows_conf::owner_flows::<var>`,
//! [`Qualified`]).
//!
//! The pool is a `/30` with one usable offset, so every lease reuses one
//! CID — the model's one CID — and a dead launch's queued connection is
//! accepted under the CID's next lease (K-A2′).
//!
//! | Action | Production call or fault |
//! |---|---|
//! | `InitWith(o)` | a fresh composition boots (`probe_startup`, `sweep_stale`, `converge_shared`) |
//! | `Provision` | `pool.assign`, then `owner.provision(&plan)` |
//! | `VmStart` / `VmStop` | seam: the guest of that launch comes up / dies — K-A2′ (accepted connections whose first record is unread fail at that read; a live session's loss waits for `HostDeadSessionLoss`), the device release's sweep (`sk_err` on every host vsock of the CID, then the callback of every in-flight dial's watch, so each returns `Unreachable { ECONNRESET }`) |
//! | `Activate`, `ActivatePending`, `ActivateQuiesced`, `ActivateRetiring` | `owner.activate(&plan)` |
//! | `Teardown`, `Release` | `pool.retire` + `owner.teardown(&plan)`; `pool.release` |
//! | `Quiesce` / `Restore` | `owner.quiesce()` (the driver keeps the hold) / `owner.restore(hold)` |
//! | `GuestOpenSession(c)`, `WorkloadOpenSession(c)`, `GuestOpenFlow(f)` | seam: queue a 1243 / 1240 connection with its `ControlOpen` / request (token of `vmGen`) buffered |
//! | `HostAcceptSession(c)`, `HostAcceptFlow(f)` | seam releases the accept to the owner |
//! | `HostReadOpen(c)`, `HostReadReqN(f, n)` | seam releases the buffered record (a request in 1 or 2 pieces) |
//! | `HostSessionLoss(c)`, `HostDeadSessionLoss(c)` | seam: the session's peer ends / the dead peer's loss is delivered |
//! | `HostStartConnect(f)` | seam releases the owner's held peer-socket open; the owner then issues its non-blocking connect (held) |
//! | `HostConnectDone(f)`, `HostConnectDiscard(f)` | seam completes the held connect; for a live flow the owner attaches the pair on `SimFlowForwarder` and writes `Paired` in that dispatch (EVMOD-PAIR) |
//! | `HostFlowEndsMsg(f, msg)` | by phase: `SimClock` past `FLOW_REQUEST_DEADLINE` (registered), a failed peer-socket open (attributed), a failed connect (connecting), the destination's reset — or, after `VmStop`, the sweep's deferred callback — and the pair's end record (paired) |
//! | `DialOpen(h)` | `GuestStreamDial::dial(target, Relay)` on a task; the seam auto-releases the `V_h` connect, the watch and the request write |
//! | `DialNotServing`, `DialQuiesced`, `DialNoSession`, `DialUnreachable` | `GuestStreamDial::dial(target, Relay)`, returning its typed error at once |
//! | `HostRecvPaired`, `HostRecvRefused`, `HostRecvTimedOut`, `HostRecvAbort` | seam delivers the guest's `Paired` / `Refused { errno }` / `TimedOut { errno }` / `Abort` on the session |
//! | `HostAbortAccept(h)` | seam: a vsock error on the in-flight dial's `V_h` (its watch's record); a paired dial's holder drops its stream |
//! | `HostDialDeadline(h)` | `SimClock` to the dial's `GUEST_DIAL_PAIRING_DEADLINE` |
//! | `DialCancel(h)` | the driver drops the pending `dial` future |
//! | guest actions (`GuestAcceptV`, `GuestPairOrRefuseOk`, `GuestConnectTimesOut`, `GuestNeverReplies`, `GuestRecvAbort`, `GuestSeesHostClose`, `GuestAcceptedError`, `GuestSessionLossObserved`, `GuestQueueReset`) | stimulus: the guest's next reply (if any) is recorded for its host delivery step; the owner is run to quiescence and the step compares its state unchanged |
//! | `HostClaimSession`, `HostCheckFlow`, `HostRegisterFlow` | hazard-only: an error |
//! | beacon actions | never enabled on `flows_conf` (the VM driver's `flows_beacon_conf`): an error |
//!
//! Projection (`State::from_driver`), O = QPROJ-2 / QPROJ-3, S = QPROJ-7
//! `socket_observation()`, C = the dialer's and the activator's own
//! observations: `alloc` ← O `cids[cid].phase`, absent there → O pool lease
//! `Retiring`, else `Absent`; `gen` ← O the lease's launch token, relabelled
//! (compared while a lease exists); `quiesced` ← O `admission`; `claim` ← O
//! `cids[cid].session`, relabelled; `sess(c).st` ← S `queued` (`pending`), O
//! `unread` (`reading`), O `session` with S `peer_gone` (`live` / `dead`), S
//! `closed_by_owner` (`closed`); `flows(f).st`, `.req`, `.inFlight` ← S
//! `queued`, O `guest_flows[key]` (`Registered { Nothing | Partial }`,
//! `Attributed`, `Connecting`, `Paired`), S `closed_by_owner` (`req` and
//! `inFlight` compared for a registered flow); `hf(h).st`, `.gen` ← O
//! `cids[cid].dials[h]` (`AwaitingReply` → `req`, `Paired` → `paired`, absent
//! after it was present → `closed`; `gen` the holding CID's launch,
//! relabelled, compared while open); `dl.res(h)` ← C the dial call's result;
//! `act` ← C the `activate` result of this step. Every other variable is
//! stimulus, model-only, ghost or constant on this main (`conf_step`
//! included).
//!
//! Identity relabelling: lease `gen` ↔ the production lease's launch token;
//! connection `c` / flow `f` ↔ the guest connection key the driver queued
//! for it (a reopened `c` gets a new kernel port); dial slot `h` ↔ the
//! host-origin `FlowId` the owner wrote in its `TcpAccept` request.

#![allow(
    clippy::assigning_clones,
    clippy::async_yields_async,
    clippy::needless_pass_by_ref_mut,
    clippy::semicolon_if_nothing_returned,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::print_stderr,
    reason = "a conformance driver asserts its own preconditions and prints its seed; protocol names are prose"
)]
#![allow(
    clippy::missing_const_for_fn,
    clippy::option_if_let_else,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::unused_self,
    reason = "test support and conformance drivers: fixture preconditions are asserted with expect, and handler signatures follow the action map"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::{Duration, Instant};

use itf::Value;
use overdrive_core::guest_transport::LaunchToken;
use overdrive_core::guest_transport::dial::{
    DialPurpose, GuestDialError, GuestStream, GuestStreamDial,
};
use overdrive_core::guest_transport::wire::{
    ControlMessage, FLOW_REQUEST_DEADLINE, FlowId, FlowRefusal, GUEST_DIAL_PAIRING_DEADLINE,
    KernelErrno,
};
use overdrive_core::traits::GuestAttachmentLease;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::mtls_resolve::MtlsResolution;
use overdrive_sim::adapters::SimMtlsResolve;
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use tokio::task::JoinHandle;

use super::flow_io::sim::{SimEffectId, SimEffectKind, SimEffectTarget, SimFailure, SimPeerEnd};
use super::owner_acceptance::support::{Composition, Lease, paused_runtime, vip_range};
use super::quint_itf::{self, ItfError, Qualified};
use super::{
    CidPhase, DialPhase, ForwardingActivation, GuestAddressPool, GuestAdmission, GuestConnKey,
    GuestFlowPhase, GuestNetworkError, GuestNetworkOperation, GuestNetworkProvisioner,
    GuestOwnerObservation, HostSharedGuestNetworkOwner, QuiescenceHold, RequestRead,
    SharedGuestNetworkOwner,
};

/// The conformance main and the module that declares the variables.
const MAIN: &str = "flows_conf";
const MODULE: &str = "owner_flows";
/// The declared listen port every dial targets.
const PORT: u16 = 8080;
/// A port no lease declares.
const UNDECLARED: u16 = 8081;
/// The model's connections, flows and dial slots on `flows_conf` (`core2`).
const CONNS: [i64; 2] = [1, 2];
const FLOW_IDS: [i64; 2] = [1, 2];
const HFLOWS: [i64; 2] = [1, 2];
/// The seed of every composition this driver builds (printed by it).
const SEED: u64 = 0x0F10;

/// The `/30` of the one-CID pool: one usable offset (2), gateway `.1`.
fn one_cid_prefix() -> ipnet::Ipv4Net {
    "100.95.0.0/30".parse().expect("a valid /30")
}

const fn one_cid_gateway() -> Ipv4Addr {
    Ipv4Addr::new(100, 95, 0, 1)
}

/// Flow `f`'s non-mesh destination (distinct per flow, so each held connect
/// names its flow).
fn destination(f: i64) -> SocketAddrV4 {
    let port = 500 + u16::try_from(f).unwrap_or(0);
    SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 10), port)
}

// ----- the compared state ---------------------------------------------------

/// One guest-opened flow: `(st, req, inFlight)`, `req` / `inFlight` compared
/// while the flow is registered.
type FlowView = (String, Option<i64>, Option<bool>);
/// One dial slot: `(st, gen)`, `gen` compared while the dial is open.
type DialView = (String, Option<i64>);

#[derive(Debug, PartialEq, Eq)]
pub(super) struct OwnerFlowsState {
    alloc: String,
    lease_gen: Option<i64>,
    quiesced: bool,
    claim: i64,
    sess: BTreeMap<i64, String>,
    flows: BTreeMap<i64, FlowView>,
    hf: BTreeMap<i64, DialView>,
    res: BTreeMap<i64, String>,
    act: String,
}

fn registered(st: &str) -> bool {
    matches!(st, "accepted" | "read" | "connecting" | "paired")
}

fn flow_view(st: String, req: i64, in_flight: bool) -> FlowView {
    if registered(&st) { (st, Some(req), Some(in_flight)) } else { (st, None, None) }
}

fn dial_view(st: String, generation: i64) -> DialView {
    let open = matches!(st.as_str(), "req" | "paired");
    (st, open.then_some(generation))
}

impl OwnerFlowsState {
    fn from_value(value: &Value) -> std::result::Result<Self, ItfError> {
        let q = Qualified::of(value, MAIN, MODULE)?;
        let alloc = quint_itf::string(q.var("alloc")?)?;
        let lease_gen = quint_itf::int(q.var("gen")?)?;
        let st = |v: &Value| quint_itf::string(quint_itf::field(v, "st")?);
        let dl = q.var("dl")?;
        Ok(Self {
            lease_gen: (alloc != "Absent").then_some(lease_gen),
            alloc,
            quiesced: quint_itf::boolean(q.var("quiesced")?)?,
            claim: quint_itf::int(q.var("claim")?)?,
            sess: quint_itf::int_map(q.var("sess")?, st)?,
            flows: quint_itf::int_map(q.var("flows")?, |v| {
                Ok(flow_view(
                    quint_itf::string(quint_itf::field(v, "st")?)?,
                    quint_itf::int(quint_itf::field(v, "req")?)?,
                    quint_itf::boolean(quint_itf::field(v, "inFlight")?)?,
                ))
            })?,
            hf: quint_itf::int_map(q.var("hf")?, |v| {
                Ok(dial_view(
                    quint_itf::string(quint_itf::field(v, "st")?)?,
                    quint_itf::int(quint_itf::field(v, "gen")?)?,
                ))
            })?,
            res: quint_itf::int_map(quint_itf::field(dl, "res")?, quint_itf::string)?,
            act: quint_itf::string(q.var("act")?)?,
        })
    }
}

impl<'de> Deserialize<'de> for OwnerFlowsState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

impl State<OwnerFlowsDriver> for OwnerFlowsState {
    /// The raw ITF state, decoded by the shared ITF helpers (the serde
    /// path renders `#bigint` as a string and `#map` as a record, QPROJ-1).
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &OwnerFlowsDriver) -> Result<Self> {
        driver.project()
    }
}

// ----- the driver ----------------------------------------------------------

/// One dial slot `h`.
struct DialSlot {
    handle: Option<JoinHandle<std::result::Result<GuestStream, GuestDialError>>>,
    flow: Option<FlowId>,
    /// The dial's result once its future returned (or `Cancelled`); "none"
    /// before any call.
    result: String,
    /// The stream a paired dial returned; the dialer holds it.
    stream: Option<GuestStream>,
    opened_at: Option<Instant>,
    /// The guest's next reply on the session, recorded at its guest step.
    reply: Option<ControlMessage>,
}

impl DialSlot {
    fn idle() -> Self {
        Self {
            handle: None,
            flow: None,
            result: "none".to_owned(),
            stream: None,
            opened_at: None,
            reply: None,
        }
    }
}

/// One guest-opened connection (`c` or `f`).
struct GuestConn {
    key: GuestConnKey,
    flow: Option<FlowId>,
    /// Request pieces released (`f`): 0, 1, 2.
    req: i64,
    /// The owner's held peer-socket open, then its held connect (`f`).
    held_open: Option<SimEffectId>,
    held_connect: Option<SimEffectId>,
    /// The VM of the flow died with its pair live: the sweep's callback is
    /// deferred to the trace's end of the flow.
    swept: bool,
}

impl GuestConn {
    const fn new(key: GuestConnKey, flow: Option<FlowId>) -> Self {
        Self { key, flow, req: 0, held_open: None, held_connect: None, swept: false }
    }
}

/// Why a driver step cannot proceed.
#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("action `{0}` is hazard-only: it is never enabled in a design instance")]
    HazardOnly(&'static str),
    #[error("action `{0}` is the beacon's (the VM driver's flows_beacon_conf)")]
    Beacon(&'static str),
    #[error("action `{action}` cannot be played: {why}")]
    NotPlayable { action: &'static str, why: String },
}

fn not_playable(action: &'static str, why: impl Into<String>) -> anyhow::Error {
    DriverError::NotPlayable { action, why: why.into() }.into()
}

pub(super) struct OwnerFlowsDriver {
    rt: tokio::runtime::Runtime,
    /// The production owner under test (QPROJ-3), the composition's own.
    owner: Arc<HostSharedGuestNetworkOwner>,
    /// The production pool (QPROJ-2), the composition's own.
    pool: Arc<GuestAddressPool>,
    /// The composition the owner stands on (its seam sim).
    c: Composition,
    leases: BTreeMap<i64, Lease>,
    /// Launch token ↦ model `gen` (`LaunchToken` is not `Ord`).
    launches: Vec<(LaunchToken, i64)>,
    current_gen: i64,
    vm_gen: i64,
    sessions: BTreeMap<i64, GuestConn>,
    flows: BTreeMap<i64, GuestConn>,
    dials: BTreeMap<i64, DialSlot>,
    holds: Vec<QuiescenceHold>,
    act: Option<String>,
    next_port: u32,
    next_seq: u32,
}

impl OwnerFlowsDriver {
    fn new() -> Self {
        let rt = paused_runtime();
        let c = {
            let _runtime = rt.enter();
            Self::composition_for_trace()
        };
        Self {
            rt,
            owner: Arc::clone(&c.owner),
            pool: Arc::clone(&c.pool),
            c,
            leases: BTreeMap::new(),
            launches: Vec::new(),
            current_gen: 0,
            vm_gen: 0,
            sessions: BTreeMap::new(),
            flows: BTreeMap::new(),
            dials: HFLOWS.iter().map(|h| (*h, DialSlot::idle())).collect(),
            holds: Vec::new(),
            act: None,
            next_port: 4_000,
            next_seq: 1,
        }
    }

    /// The composition of one trace (one CID: the `/30` guest prefix), not
    /// booted.
    fn composition_for_trace() -> Composition {
        let resolve = SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh);
        Composition::with_prefix(resolve, vip_range(), one_cid_prefix(), one_cid_gateway(), SEED)
    }

    fn composition(&self) -> &Composition {
        &self.c
    }

    fn settle(&self) {
        self.rt.block_on(self.composition().settle());
    }

    /// QPROJ-3, read on the held owner after it settled.
    fn observe(&self) -> GuestOwnerObservation {
        self.settle();
        self.rt.block_on(self.owner.flow_observation())
    }

    fn lease(&self, lease_gen: i64, action: &'static str) -> Result<&Lease> {
        self.leases
            .get(&lease_gen)
            .ok_or_else(|| not_playable(action, format!("no lease of gen {lease_gen}")))
    }

    fn current_lease(&self, action: &'static str) -> Result<&Lease> {
        self.lease(self.current_gen, action)
    }

    fn held_ids(&self, kind: SimEffectKind) -> BTreeSet<SimEffectId> {
        self.composition()
            .io
            .held()
            .into_iter()
            .filter(|(_, effect)| effect.kind == kind)
            .map(|(id, _)| id)
            .collect()
    }

    // ----- lifecycle -----

    /// Resets the driver for a new trace (the runner reuses one driver for
    /// every trace) and boots a fresh composition.
    fn init_with(&mut self, _origin: &str) {
        *self = Self::new();
        self.rt.block_on(self.c.boot());
        self.c.io.set_auto_deliver(false);
        self.c.io.hold(SimEffectKind::OpenPeer);
        self.c.io.hold(SimEffectKind::ConnectPeer);
    }

    fn provision(&mut self) {
        let lease_gen = self.current_gen + 1;
        let lease = self
            .rt
            .block_on(self.composition().lease(&format!("owner-flows-{lease_gen}"), &[PORT]));
        self.launches.push((lease.launch, lease_gen));
        self.leases.insert(lease_gen, lease);
        self.current_gen = lease_gen;
        self.settle();
    }

    fn vm_start(&mut self) -> Result {
        let cid = self.current_lease("VmStart")?.cid;
        self.vm_gen = self.current_gen;
        self.composition().io.vm_start(cid);
        self.settle();
        Ok(())
    }

    fn vm_stop(&mut self) -> Result {
        let cid = self.lease(self.vm_gen, "VmStop")?.cid;
        let observed = self.observe();
        let c = self.composition();
        c.io.vm_stop_before_callbacks(cid);
        if let Some(table) = observed.cids.get(&cid) {
            // K-A2′: an accepted connection whose first record is unread
            // fails at that read now; a live session's loss waits for
            // `HostDeadSessionLoss`.
            for key in &table.unread {
                c.io.release_peer_gone(*key);
            }
            for (key, phase) in &table.guest_flows {
                if matches!(phase, GuestFlowPhase::Registered { .. }) {
                    c.io.release_peer_gone(*key);
                }
            }
            // The sweep's callback of every in-flight dial's watch: each dial
            // returns `Unreachable { ECONNRESET }` (K-C4).
            for (flow, dial) in &table.dials {
                if dial.phase == DialPhase::AwaitingReply {
                    c.io.finish_sweep_of(*flow);
                }
            }
        }
        for conn in self.flows.values_mut() {
            conn.swept = true;
        }
        self.settle();
        Ok(())
    }

    fn activate(&mut self, action: &'static str) -> Result {
        let plan = self.current_lease(action)?.plan.clone();
        let outcome = self.rt.block_on(self.composition().owner.activate(&plan));
        self.act = Some(match outcome {
            Ok(ForwardingActivation::Activated) => "Activated".to_owned(),
            Ok(ForwardingActivation::ControlSessionPending) => "ControlSessionPending".to_owned(),
            Ok(ForwardingActivation::QuiescenceLatched) => "QuiescenceLatched".to_owned(),
            Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::ForwardingActivate,
                ..
            }) => "LeaseRetiring".to_owned(),
            Err(other) => return Err(not_playable(action, format!("activate failed: {other}"))),
        });
        self.settle();
        Ok(())
    }

    fn teardown(&mut self) -> Result {
        let lease = self.current_lease("Teardown")?.clone();
        let c = self.composition();
        let _retired = c.pool.retire(&lease.alloc);
        self.rt
            .block_on(c.owner.teardown(&lease.plan))
            .map_err(|error| not_playable("Teardown", error.to_string()))?;
        self.settle();
        Ok(())
    }

    fn release(&mut self) -> Result {
        let alloc = self.current_lease("Release")?.alloc.clone();
        self.composition().pool.release(&alloc);
        self.settle();
        Ok(())
    }

    fn quiesce(&mut self) {
        let (hold, _quiescence) = self.rt.block_on(self.composition().owner.quiesce());
        self.holds.push(hold);
        self.settle();
    }

    fn restore(&mut self) -> Result {
        let hold = self.holds.pop().ok_or_else(|| not_playable("Restore", "no hold is held"))?;
        self.rt
            .block_on(self.composition().owner.restore(hold))
            .map_err(|error| not_playable("Restore", error.to_string()))?;
        self.settle();
        Ok(())
    }

    // ----- sessions -----

    fn port(&mut self) -> u32 {
        self.next_port += 1;
        self.next_port
    }

    fn open_session(&mut self, conn: i64, action: &'static str) -> Result {
        let lease = self.lease(self.vm_gen, action)?.clone();
        let port = self.port();
        let key = self.composition().open_session(&lease, port);
        self.sessions.insert(conn, GuestConn::new(key, None));
        self.settle();
        Ok(())
    }

    fn session_key(&self, conn: i64, action: &'static str) -> Result<GuestConnKey> {
        self.sessions
            .get(&conn)
            .map(|session| session.key)
            .ok_or_else(|| not_playable(action, format!("connection {conn} was never opened")))
    }

    fn host_accept_session(&mut self, conn: i64) -> Result {
        let key = self.session_key(conn, "HostAcceptSession")?;
        self.composition().io.release_accept(key);
        self.settle();
        Ok(())
    }

    fn host_read_open(&mut self, conn: i64) -> Result {
        let key = self.session_key(conn, "HostReadOpen")?;
        self.composition().io.release_read(key, None);
        self.settle();
        Ok(())
    }

    fn host_session_loss(&mut self, conn: i64) -> Result {
        let key = self.session_key(conn, "HostSessionLoss")?;
        self.composition().io.end_connection(key, SimPeerEnd::Eof);
        self.settle();
        Ok(())
    }

    fn host_dead_session_loss(&mut self, conn: i64) -> Result {
        let key = self.session_key(conn, "HostDeadSessionLoss")?;
        self.composition().io.release_peer_gone(key);
        self.settle();
        Ok(())
    }

    // ----- guest-opened flows -----

    fn guest_open_flow(&mut self, f: i64) -> Result {
        let lease = self.lease(self.vm_gen, "GuestOpenFlow")?.clone();
        let seq = self.next_seq;
        self.next_seq += 1;
        let port = self.port();
        let (key, flow) = self.composition().open_tcp_flow(&lease, seq, destination(f), port);
        self.flows.insert(f, GuestConn::new(key, Some(flow)));
        self.settle();
        Ok(())
    }

    fn flow(&self, f: i64, action: &'static str) -> Result<&GuestConn> {
        self.flows.get(&f).ok_or_else(|| not_playable(action, format!("flow {f} was never opened")))
    }

    fn host_accept_flow(&mut self, f: i64) -> Result {
        let key = self.flow(f, "HostAcceptFlow")?.key;
        self.composition().io.release_accept(key);
        self.settle();
        Ok(())
    }

    fn host_read_req(&mut self, f: i64, n: i64) -> Result {
        let (key, req) = {
            let flow = self.flow(f, "HostReadReqN")?;
            (flow.key, flow.req)
        };
        let reaches = (req + n).min(2);
        // One piece: half of the 32-byte request; two pieces (or the rest):
        // all that is buffered.
        let bytes = if reaches == 1 { Some(16) } else { None };
        let before = self.held_ids(SimEffectKind::OpenPeer);
        self.composition().io.release_read(key, bytes);
        self.settle();
        let opened = self.held_ids(SimEffectKind::OpenPeer).difference(&before).next().copied();
        if let Some(flow) = self.flows.get_mut(&f) {
            flow.req = reaches;
            flow.held_open = opened.or(flow.held_open);
        }
        Ok(())
    }

    fn host_start_connect(&mut self, f: i64) -> Result {
        let open = self.flow(f, "HostStartConnect")?.held_open.ok_or_else(|| {
            not_playable("HostStartConnect", "the owner opened no peer socket for the flow")
        })?;
        self.composition().io.release(open);
        self.settle();
        let target = SimEffectTarget::Addr(destination(f));
        let connect = self
            .composition()
            .io
            .held()
            .into_iter()
            .find(|(_, effect)| {
                effect.kind == SimEffectKind::ConnectPeer && effect.target == target
            })
            .map(|(id, _)| id);
        if let Some(flow) = self.flows.get_mut(&f) {
            flow.held_open = None;
            flow.held_connect = connect;
        }
        Ok(())
    }

    fn host_connect_done(&mut self, f: i64, action: &'static str) -> Result {
        let connect = self.flow(f, action)?.held_connect;
        // A dropped connect future never applies (the owner closed the flow
        // and dropped its call): nothing is held, the completion is moot.
        if let Some(id) = connect
            && self.composition().io.held().iter().any(|(held, _)| *held == id)
        {
            self.composition().io.release(id);
        }
        if let Some(flow) = self.flows.get_mut(&f) {
            flow.held_connect = None;
        }
        self.settle();
        Ok(())
    }

    fn host_flow_ends(&mut self, f: i64) -> Result {
        let (key, flow, held_open, held_connect, swept) = {
            let conn = self.flow(f, "HostFlowEndsMsg")?;
            (conn.key, conn.flow, conn.held_open, conn.held_connect, conn.swept)
        };
        let phase = self
            .observe()
            .cids
            .values()
            .find_map(|cid| cid.guest_flows.get(&key))
            .copied()
            .ok_or_else(|| {
                not_playable("HostFlowEndsMsg", "the flow is not registered at the owner")
            })?;
        let c = self.composition();
        match phase {
            GuestFlowPhase::Registered { .. } => {
                self.rt.block_on(c.advance(FLOW_REQUEST_DEADLINE + Duration::from_millis(1)));
            }
            GuestFlowPhase::Attributed => {
                let id = held_open
                    .ok_or_else(|| not_playable("HostFlowEndsMsg", "no held peer-socket open"))?;
                c.io.fail(id, SimFailure::Io(std::io::ErrorKind::OutOfMemory));
            }
            GuestFlowPhase::Connecting => {
                let id = held_connect
                    .ok_or_else(|| not_playable("HostFlowEndsMsg", "no held connect"))?;
                c.io.fail(id, SimFailure::Io(std::io::ErrorKind::ConnectionRefused));
            }
            GuestFlowPhase::Paired { .. } => {
                let flow =
                    flow.ok_or_else(|| not_playable("HostFlowEndsMsg", "a paired flow has an id"))?;
                if swept {
                    c.io.finish_sweep_of(flow);
                } else {
                    c.io.destination_resets(flow);
                }
            }
        }
        self.settle();
        Ok(())
    }

    // ----- dials -----

    fn dial_target(&self, port: u16) -> SocketAddrV4 {
        self.leases
            .get(&self.current_gen)
            .map_or_else(|| SocketAddrV4::new(one_cid_gateway(), port), |lease| lease.target(port))
    }

    fn dial_open(&mut self, h: i64) -> Result {
        let target = self.dial_target(PORT);
        let c = self.composition();
        let before = c.io.dials().len();
        let handle = self.rt.block_on(async { c.spawn_dial(target, DialPurpose::Relay) });
        self.settle();
        let flow =
            c.io.dials()
                .get(before)
                .and_then(|dial| dial.request)
                .map(|request| request.flow())
                .ok_or_else(|| not_playable("DialOpen", "the owner wrote no TcpAccept request"))?;
        let opened_at = c.clock.now();
        let slot = self
            .dials
            .get_mut(&h)
            .ok_or_else(|| not_playable("DialOpen", format!("no slot {h}")))?;
        *slot = DialSlot {
            handle: Some(handle),
            flow: Some(flow),
            result: "pending".to_owned(),
            stream: None,
            opened_at: Some(opened_at),
            reply: None,
        };
        Ok(())
    }

    fn dial_refused(&mut self, h: i64, port: u16, action: &'static str) -> Result {
        let target = self.dial_target(port);
        let outcome = self.rt.block_on(self.composition().owner.dial(target, DialPurpose::Relay));
        let result = match outcome {
            Ok(_) => return Err(not_playable(action, "the dial was not refused")),
            Err(error) => dial_result(error),
        };
        let slot =
            self.dials.get_mut(&h).ok_or_else(|| not_playable(action, format!("no slot {h}")))?;
        slot.result = result;
        self.settle();
        Ok(())
    }

    fn slot(&self, h: i64, action: &'static str) -> Result<&DialSlot> {
        self.dials.get(&h).ok_or_else(|| not_playable(action, format!("no dial slot {h}")))
    }

    fn dial_flow(&self, h: i64, action: &'static str) -> Result<FlowId> {
        self.slot(h, action)?
            .flow
            .ok_or_else(|| not_playable(action, format!("dial {h} never opened")))
    }

    /// The live control session (the owner's claim holder) of the dial's
    /// lease, which carries the guest's reply.
    fn live_session(&self, action: &'static str) -> Result<GuestConnKey> {
        let cid = self.current_lease(action)?.cid;
        self.observe()
            .cids
            .get(&cid)
            .and_then(|table| table.session)
            .ok_or_else(|| not_playable(action, "no live session carries the guest's reply"))
    }

    fn guest_reply(&mut self, h: i64, reply: ControlMessage) {
        if let Some(slot) = self.dials.get_mut(&h) {
            slot.reply = Some(reply);
        }
        self.settle();
    }

    fn host_recv(
        &mut self,
        h: i64,
        action: &'static str,
        expect: fn(&ControlMessage) -> bool,
    ) -> Result {
        let reply = self
            .slot(h, action)?
            .reply
            .ok_or_else(|| not_playable(action, "no reply in flight"))?;
        if !expect(&reply) {
            return Err(not_playable(action, format!("the reply in flight is {reply:?}")));
        }
        let session = self.live_session(action)?;
        self.composition().guest_sends(session, reply);
        self.composition().io.release_read(session, None);
        if let Some(slot) = self.dials.get_mut(&h) {
            slot.reply = None;
        }
        self.settle();
        Ok(())
    }

    fn host_abort_accept(&mut self, h: i64) -> Result {
        let flow = self.dial_flow(h, "HostAbortAccept")?;
        let stream = self.dials.get_mut(&h).and_then(|slot| slot.stream.take());
        // The holder of a paired dial's stream drops it on the error.
        if let Some(stream) = stream {
            drop(stream);
        } else {
            let eio = KernelErrno::new(5)
                .map_err(|error| not_playable("HostAbortAccept", error.to_string()))?;
            self.composition().io.dial_vsock_error(flow, eio);
        }
        self.settle();
        Ok(())
    }

    fn host_dial_deadline(&mut self, h: i64) -> Result {
        let opened_at = self
            .slot(h, "HostDialDeadline")?
            .opened_at
            .ok_or_else(|| not_playable("HostDialDeadline", "the dial never opened"))?;
        let c = self.composition();
        let elapsed = c.clock.now().saturating_duration_since(opened_at);
        let remaining =
            GUEST_DIAL_PAIRING_DEADLINE.saturating_sub(elapsed) + Duration::from_millis(1);
        self.rt.block_on(c.advance(remaining));
        Ok(())
    }

    fn dial_cancel(&mut self, h: i64) -> Result {
        let slot = self
            .dials
            .get_mut(&h)
            .ok_or_else(|| not_playable("DialCancel", format!("no slot {h}")))?;
        let handle =
            slot.handle.take().ok_or_else(|| not_playable("DialCancel", "no pending dial"))?;
        handle.abort();
        slot.result = "Cancelled".to_owned();
        self.settle();
        Ok(())
    }

    fn guest_pair_or_refuse(&mut self, h: i64, ok: bool) -> Result {
        let flow = self.dial_flow(h, "GuestPairOrRefuseOk")?;
        let reply = if ok {
            ControlMessage::Paired { flow }
        } else {
            ControlMessage::Refused {
                flow,
                refusal: FlowRefusal::ApplicationRefused { errno: KernelErrno::ECONNREFUSED },
            }
        };
        self.guest_reply(h, reply);
        Ok(())
    }

    fn guest_connect_times_out(&mut self, h: i64) -> Result {
        let flow = self.dial_flow(h, "GuestConnectTimesOut")?;
        self.guest_reply(h, ControlMessage::TimedOut { flow, errno: KernelErrno::ETIMEDOUT });
        Ok(())
    }

    fn guest_accepted_error(&mut self, h: i64) -> Result {
        let flow = self.dial_flow(h, "GuestAcceptedError")?;
        self.guest_reply(h, ControlMessage::Abort { flow });
        Ok(())
    }

    /// A guest step with no host-visible effect: the guest's own state moves;
    /// the owner is run to quiescence and the step compares it unchanged.
    fn guest_stimulus(&mut self) {
        self.settle();
    }

    /// Collects every dial whose future returned (the dialer's C
    /// observation) and keeps a returned stream (the dialer holds it).
    fn harvest_dials(&mut self) -> Result {
        let mut finished = Vec::new();
        for (h, slot) in &mut self.dials {
            if slot.handle.as_ref().is_some_and(JoinHandle::is_finished)
                && let Some(handle) = slot.handle.take()
            {
                finished.push((*h, handle));
            }
        }
        for (h, handle) in finished {
            let outcome = self.rt.block_on(handle);
            let slot = self.dials.get_mut(&h).expect("collected from the map");
            match outcome {
                Ok(Ok(stream)) => {
                    slot.result = "Ok".to_owned();
                    slot.stream = Some(stream);
                }
                Ok(Err(error)) => slot.result = dial_result(error),
                Err(join) if join.is_cancelled() => slot.result = "Cancelled".to_owned(),
                Err(join) => anyhow::bail!("the dial task panicked: {join}"),
            }
        }
        Ok(())
    }

    // ----- projection -----

    fn project(&self) -> Result<OwnerFlowsState> {
        let mut state = OwnerFlowsState {
            alloc: "Absent".to_owned(),
            lease_gen: None,
            quiesced: false,
            claim: 0,
            sess: CONNS.iter().map(|c| (*c, "none".to_owned())).collect(),
            flows: FLOW_IDS.iter().map(|f| (*f, flow_view("none".to_owned(), 0, false))).collect(),
            hf: HFLOWS.iter().map(|h| (*h, dial_view("none".to_owned(), 0))).collect(),
            res: HFLOWS.iter().map(|h| (*h, "none".to_owned())).collect(),
            act: self.act.clone().unwrap_or_else(|| "none".to_owned()),
        };
        let observed = self.rt.block_on(self.owner.flow_observation());
        let leases = self.pool.lease_observation();
        // conformance-lint: sim-arbiter-ok QPROJ-7 owner-seam socket arbiter (queued, closed_by_owner, peer_gone), Tier-3 pair V-22 / R5-16 / R5-41 and R5-59(a)
        let sockets = self.c.io.socket_observation();
        state.quiesced = observed.admission == GuestAdmission::Quiesced;

        if let Some(lease) = self.leases.get(&self.current_gen) {
            match observed.cids.get(&lease.cid) {
                Some(cid) => {
                    state.alloc = match cid.phase {
                        CidPhase::Provisioned => "Provisioned",
                        CidPhase::Active => "Active",
                        CidPhase::Retiring => "Retiring",
                    }
                    .to_owned();
                    state.lease_gen = Some(self.relabel_launch(cid.launch)?);
                    if let Some(holder) = cid.session {
                        state.claim = self.relabel_session(holder)?;
                    }
                }
                None => {
                    if let Some(view) = leases.leases.get(&lease.alloc) {
                        if view.state == GuestAttachmentLease::Retiring {
                            state.alloc = "Retiring".to_owned();
                            state.lease_gen = Some(self.relabel_launch(view.launch)?);
                        } else {
                            anyhow::bail!("an Admitted lease the owner does not report");
                        }
                    }
                }
            }
        }

        for (conn, session) in &self.sessions {
            let table = observed.cids.get(&session.key.peer_cid);
            let st = if sockets.queued.contains_key(&session.key) {
                "pending"
            } else if table.is_some_and(|t| t.unread.contains(&session.key)) {
                "reading"
            } else if table.is_some_and(|t| t.session == Some(session.key)) {
                if sockets.peer_gone.contains(&session.key) { "dead" } else { "live" }
            } else if sockets.closed_by_owner.contains(&session.key) {
                "closed"
            } else {
                anyhow::bail!("session {conn} ({:?}) has no model state", session.key);
            };
            state.sess.insert(*conn, st.to_owned());
        }

        for (f, conn) in &self.flows {
            let phase =
                observed.cids.get(&conn.key.peer_cid).and_then(|t| t.guest_flows.get(&conn.key));
            let view = match phase {
                Some(GuestFlowPhase::Registered { request }) => flow_view(
                    "accepted".to_owned(),
                    match request {
                        RequestRead::Nothing => 0,
                        RequestRead::Partial => 1,
                    },
                    false,
                ),
                Some(GuestFlowPhase::Attributed) => flow_view("read".to_owned(), 2, false),
                Some(GuestFlowPhase::Connecting) => flow_view("connecting".to_owned(), 2, true),
                Some(GuestFlowPhase::Paired { .. }) => flow_view("paired".to_owned(), 2, false),
                None if sockets.queued.contains_key(&conn.key) => {
                    flow_view("queued".to_owned(), 0, false)
                }
                None if sockets.closed_by_owner.contains(&conn.key) => {
                    flow_view("closed".to_owned(), 0, false)
                }
                None => anyhow::bail!("flow {f} ({:?}) has no model state", conn.key),
            };
            state.flows.insert(*f, view);
        }

        for (h, slot) in &self.dials {
            state.res.insert(*h, slot.result.clone());
            let Some(flow) = slot.flow else { continue };
            let held = observed
                .cids
                .values()
                .find_map(|t| t.dials.get(&flow).map(|dial| (t.launch, dial.phase)));
            let view = match held {
                Some((launch, DialPhase::AwaitingReply)) => {
                    dial_view("req".to_owned(), self.relabel_launch(launch)?)
                }
                Some((launch, DialPhase::Paired { .. })) => {
                    dial_view("paired".to_owned(), self.relabel_launch(launch)?)
                }
                None => dial_view("closed".to_owned(), 0),
            };
            state.hf.insert(*h, view);
        }
        Ok(state)
    }

    fn relabel_launch(&self, launch: LaunchToken) -> Result<i64> {
        self.launches
            .iter()
            .find(|(token, _)| *token == launch)
            .map(|(_, lease_gen)| *lease_gen)
            .ok_or_else(|| anyhow::anyhow!("launch token {launch} has no pre-image"))
    }

    fn relabel_session(&self, key: GuestConnKey) -> Result<i64> {
        self.sessions
            .iter()
            .find(|(_, session)| session.key == key)
            .map(|(conn, _)| *conn)
            .ok_or_else(|| anyhow::anyhow!("session {key:?} has no pre-image"))
    }
}

fn dial_result(error: GuestDialError) -> String {
    match error {
        GuestDialError::Refused { .. } => "Refused".to_owned(),
        GuestDialError::TimedOut { .. } => "TimedOut".to_owned(),
        GuestDialError::PairingDeadline => "PairingDeadline".to_owned(),
        GuestDialError::NotServing => "NotServing".to_owned(),
        GuestDialError::NoSession => "NoSession".to_owned(),
        GuestDialError::Quiesced => "Quiesced".to_owned(),
        GuestDialError::Unreachable { errno } if errno == KernelErrno::ENODEV => {
            "Unreachable:ENODEV".to_owned()
        }
        GuestDialError::Unreachable { errno } if errno == KernelErrno::ECONNRESET => {
            "Unreachable:ECONNRESET".to_owned()
        }
        GuestDialError::Unreachable { .. } => "Unreachable:io".to_owned(),
        GuestDialError::Capacity => "Capacity".to_owned(),
    }
}

impl Driver for OwnerFlowsDriver {
    type State = OwnerFlowsState;

    fn config() -> Config {
        Config { state: &[], nondet: &["flows_conf::owner_flows::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        self.act = None;
        let outcome: Result = (|| -> Result {
            switch!(step {
                InitWith(o: String) => self.init_with(&o),
                Provision => self.provision(),
                VmStart => self.vm_start()?,
                VmStop => self.vm_stop()?,
                Activate => self.activate("Activate")?,
                ActivatePending => self.activate("ActivatePending")?,
                ActivateQuiesced => self.activate("ActivateQuiesced")?,
                ActivateRetiring => self.activate("ActivateRetiring")?,
                Teardown => self.teardown()?,
                Release => self.release()?,
                Quiesce => self.quiesce(),
                Restore => self.restore()?,
                GuestOpenSession(c: i64) => self.open_session(c, "GuestOpenSession")?,
                WorkloadOpenSession(c: i64) => self.open_session(c, "WorkloadOpenSession")?,
                HostAcceptSession(c: i64) => self.host_accept_session(c)?,
                HostReadOpen(c: i64) => self.host_read_open(c)?,
                HostClaimSession => return Err(DriverError::HazardOnly("HostClaimSession").into()),
                HostSessionLoss(c: i64) => self.host_session_loss(c)?,
                HostDeadSessionLoss(c: i64) => self.host_dead_session_loss(c)?,
                GuestOpenFlow(f: i64) => self.guest_open_flow(f)?,
                HostAcceptFlow(f: i64) => self.host_accept_flow(f)?,
                HostCheckFlow => return Err(DriverError::HazardOnly("HostCheckFlow").into()),
                HostRegisterFlow => return Err(DriverError::HazardOnly("HostRegisterFlow").into()),
                HostReadReqN(f: i64, n: i64) => self.host_read_req(f, n)?,
                HostStartConnect(f: i64) => self.host_start_connect(f)?,
                HostConnectDone(f: i64) => self.host_connect_done(f, "HostConnectDone")?,
                HostConnectDiscard(f: i64) => self.host_connect_done(f, "HostConnectDiscard")?,
                HostFlowEndsMsg(f: i64, msg: bool) => {
                    let _ = msg;
                    self.host_flow_ends(f)?
                }
                DialOpen(h: i64) => self.dial_open(h)?,
                DialNotServing(h: i64, declared: bool) => {
                    self.dial_refused(h, if declared { PORT } else { UNDECLARED }, "DialNotServing")?
                }
                DialQuiesced(h: i64) => self.dial_refused(h, PORT, "DialQuiesced")?,
                DialNoSession(h: i64) => self.dial_refused(h, PORT, "DialNoSession")?,
                DialUnreachable(h: i64) => self.dial_refused(h, PORT, "DialUnreachable")?,
                GuestAcceptV(h: i64) => {
                    let _ = h;
                    self.guest_stimulus()
                }
                GuestPairOrRefuseOk(h: i64, ok: bool) => self.guest_pair_or_refuse(h, ok)?,
                GuestConnectTimesOut(h: i64) => self.guest_connect_times_out(h)?,
                HostRecvPaired(h: i64) => {
                    self.host_recv(h, "HostRecvPaired", |m| matches!(m, ControlMessage::Paired { .. }))?
                }
                HostRecvRefused(h: i64) => {
                    self.host_recv(h, "HostRecvRefused", |m| matches!(m, ControlMessage::Refused { .. }))?
                }
                HostRecvTimedOut(h: i64) => {
                    self.host_recv(h, "HostRecvTimedOut", |m| matches!(m, ControlMessage::TimedOut { .. }))?
                }
                HostRecvAbort(h: i64) => {
                    self.host_recv(h, "HostRecvAbort", |m| matches!(m, ControlMessage::Abort { .. }))?
                }
                HostAbortAccept(h: i64) => self.host_abort_accept(h)?,
                HostDialDeadline(h: i64) => self.host_dial_deadline(h)?,
                DialCancel(h: i64) => self.dial_cancel(h)?,
                GuestNeverReplies(h: i64) => {
                    let _ = h;
                    self.guest_stimulus()
                }
                GuestRecvAbort(h: i64) => {
                    let _ = h;
                    self.guest_stimulus()
                }
                GuestSeesHostClose(h: i64) => {
                    let _ = h;
                    self.guest_stimulus()
                }
                GuestAcceptedError(h: i64) => self.guest_accepted_error(h)?,
                GuestSessionLossObserved(c: i64) => {
                    let _ = c;
                    self.guest_stimulus()
                }
                GuestQueueReset(h: i64) => {
                    let _ = h;
                    self.guest_stimulus()
                }
                GuestOpenBeacon => return Err(DriverError::Beacon("GuestOpenBeacon").into()),
                HostAcceptBeacon => return Err(DriverError::Beacon("HostAcceptBeacon").into()),
                HostReadReady => return Err(DriverError::Beacon("HostReadReady").into()),
                DriverStop => return Err(DriverError::Beacon("DriverStop").into()),
            })
        })();
        outcome?;
        self.harvest_dials()
    }
}

// ----- named runs (flows_conf, max_samples = 1) and randomized traces -------

/// Named run `ControlSessionOpensAndCidActivates` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "ControlSessionOpensAndCidActivates"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_control_session_opens_and_cid_activates() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `GuestOpenedFlowIsAdmittedReadAttachedAndPaired` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "GuestOpenedFlowIsAdmittedReadAttachedAndPaired"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_guest_opened_flow_is_admitted_read_attached_and_paired() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `RequestReadInPiecesAttachesOnlyAfterFullRead` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "RequestReadInPiecesAttachesOnlyAfterFullRead"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_request_read_in_pieces_attaches_only_after_full_read() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `RacingFirstConnectsLeaveOneLiveSession` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "RacingFirstConnectsLeaveOneLiveSession"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_racing_first_connects_leave_one_live_session() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `SessionLossAbortsFlowsAndReconnectRestores` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "SessionLossAbortsFlowsAndReconnectRestores"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_session_loss_aborts_flows_and_reconnect_restores() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `GuestReopensSessionOnlyAfterObservingItsLoss` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "GuestReopensSessionOnlyAfterObservingItsLoss"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_guest_reopens_session_only_after_observing_its_loss() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `GuestObservesSessionLossThenReopens` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "GuestObservesSessionLossThenReopens"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_guest_observes_session_loss_then_reopens() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `VmStopClearsTheGuestSession` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "VmStopClearsTheGuestSession"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_vm_stop_clears_the_guest_session() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `QuiesceAbortsFlowsAndRefusesAdmissionUntilRestore` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "QuiesceAbortsFlowsAndRefusesAdmissionUntilRestore"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_quiesce_aborts_flows_and_refuses_admission_until_restore() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `ActivationWaitsForQuiescenceToEnd` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "ActivationWaitsForQuiescenceToEnd"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_activation_waits_for_quiescence_to_end() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `InFlightConnectOfAbortedFlowIsDiscarded` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "InFlightConnectOfAbortedFlowIsDiscarded"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_in_flight_connect_of_aborted_flow_is_discarded() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `AttributedFlowEndsAndCidKeepsServing` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "AttributedFlowEndsAndCidKeepsServing"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_attributed_flow_ends_and_cid_keeps_serving() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `TeardownAbortsEverythingThenCidIsLeasedAgain` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "TeardownAbortsEverythingThenCidIsLeasedAgain"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_teardown_aborts_everything_then_cid_is_leased_again() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `StaleLaunchSessionIsClosedAtReadAndDoesNotPoisonClaim` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "StaleLaunchSessionIsClosedAtReadAndDoesNotPoisonClaim"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_stale_launch_session_is_closed_at_read_and_does_not_poison_claim()
-> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `TeardownBetweenAcceptAndReadClosesWithoutClaim` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "TeardownBetweenAcceptAndReadClosesWithoutClaim"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_teardown_between_accept_and_read_closes_without_claim() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `StaleLaunchFlowIsClosedAtRequestReadWithNoMessage` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "StaleLaunchFlowIsClosedAtRequestReadWithNoMessage"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_stale_launch_flow_is_closed_at_request_read_with_no_message() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `VmDeathMakesLiveSessionDeadThenLossIsProcessed` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "VmDeathMakesLiveSessionDeadThenLossIsProcessed"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_vm_death_makes_live_session_dead_then_loss_is_processed() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `DialPairsAndReturnsStream` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialPairsAndReturnsStream"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_dial_pairs_and_returns_stream() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `DialRefusedReturnsTypedError` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialRefusedReturnsTypedError"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_dial_refused_returns_typed_error() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `DialTimedOutReturnsTypedError` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialTimedOutReturnsTypedError"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_dial_timed_out_returns_typed_error() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `DialNotServingIsATypedResultWithNoStateChange` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialNotServingIsATypedResultWithNoStateChange"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_dial_not_serving_is_a_typed_result_with_no_state_change() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `DialWhileQuiescedIsATypedResultWithNoStateChange` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialWhileQuiescedIsATypedResultWithNoStateChange"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_dial_while_quiesced_is_a_typed_result_with_no_state_change() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `DialWithoutLiveSessionIsATypedResultWithNoStateChange` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialWithoutLiveSessionIsATypedResultWithNoStateChange"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_dial_without_live_session_is_a_typed_result_with_no_state_change()
-> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `DialToAGoneVmIsUnreachable` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialToAGoneVmIsUnreachable"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_dial_to_a_gone_vm_is_unreachable() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `DialAfterObservedSessionLossWithVmGoneReturnsNoSession` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialAfterObservedSessionLossWithVmGoneReturnsNoSession"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_dial_after_observed_session_loss_with_vm_gone_returns_no_session()
-> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `PairingDeadlineWaitsForTheGuestAnswer` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "PairingDeadlineWaitsForTheGuestAnswer"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_pairing_deadline_waits_for_the_guest_answer() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `PairingDeadlineFiresOnlyAfterGuestTimeoutAndLateReplyIsDiscarded` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "PairingDeadlineFiresOnlyAfterGuestTimeoutAndLateReplyIsDiscarded"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_pairing_deadline_fires_only_after_guest_timeout_and_late_reply_is_discarded()
-> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `PairingDeadlineEndsDialToALiveVmWhoseGuestNeverReplies` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "PairingDeadlineEndsDialToALiveVmWhoseGuestNeverReplies"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_pairing_deadline_ends_dial_to_a_live_vm_whose_guest_never_replies()
-> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `VmExitDuringDialReturnsUnreachableReset` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "VmExitDuringDialReturnsUnreachableReset"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_vm_exit_during_dial_returns_unreachable_reset() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `VmExitDuringDialLeavesNoPairingDeadline` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "VmExitDuringDialLeavesNoPairingDeadline"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_vm_exit_during_dial_leaves_no_pairing_deadline() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `DroppedDialFutureEndsLikeThePairingDeadline` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DroppedDialFutureEndsLikeThePairingDeadline"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_dropped_dial_future_ends_like_the_pairing_deadline() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `TeardownEndsPendingDialWithNotServing` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "TeardownEndsPendingDialWithNotServing"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_teardown_ends_pending_dial_with_not_serving() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `ActivationIsImpossibleWhileTheSessionIsPending` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "ActivationIsImpossibleWhileTheSessionIsPending"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_activation_is_impossible_while_the_session_is_pending() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `ActivationWithSessionPendingReturnsPendingThenActivates` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "ActivationWithSessionPendingReturnsPendingThenActivates"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_activation_with_session_pending_returns_pending_then_activates() -> impl Driver
{
    OwnerFlowsDriver::new()
}

/// Named run `ActivateAfterTeardownIsRefusedAndChangesNothing` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "ActivateAfterTeardownIsRefusedAndChangesNothing"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_activate_after_teardown_is_refused_and_changes_nothing() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `ActivateAfterTeardownNeverActivates` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "ActivateAfterTeardownNeverActivates"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_activate_after_teardown_never_activates() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `DialAbortedBeforeGuestAcceptsEndsByHostClose` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialAbortedBeforeGuestAcceptsEndsByHostClose"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_dial_aborted_before_guest_accepts_ends_by_host_close() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `DialQueuedVgIsResetByHostClose` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialQueuedVgIsResetByHostClose"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_dial_queued_vg_is_reset_by_host_close() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `LatePairedForClosedHostFlowIsDiscarded` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "LatePairedForClosedHostFlowIsDiscarded"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_late_paired_for_closed_host_flow_is_discarded() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `SessionLossAbortsDialOnBothSides` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "SessionLossAbortsDialOnBothSides"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_session_loss_aborts_dial_on_both_sides() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `QuiesceAbortsDialAndSendsAbort` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "QuiesceAbortsDialAndSendsAbort"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_quiesce_aborts_dial_and_sends_abort() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `AcceptedVgHostCloseEndsFlowSilently` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "AcceptedVgHostCloseEndsFlowSilently"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_accepted_vg_host_close_ends_flow_silently() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `AcceptedVgErrorAbortsWithAbort` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "AcceptedVgErrorAbortsWithAbort"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_accepted_vg_error_aborts_with_abort() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `SessionLossAbortsAllFlowsBeforeReconnect` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "SessionLossAbortsAllFlowsBeforeReconnect"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_session_loss_aborts_all_flows_before_reconnect() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Named run `DialAfterReconnectPairs` (`flows_conf`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt",
    main = "flows_conf",
    max_samples = 1,
    test = "DialAfterReconnectPairs"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_owner_flows_dial_after_reconnect_pairs() -> impl Driver {
    OwnerFlowsDriver::new()
}

/// Randomized `flows_conf` traces; the seed is printed on failure.
#[quint_run(spec = "../../specs/quint/guest-flow-owner/owner_flows.qnt", main = "flows_conf")]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn flows_conf_randomized_traces() -> impl Driver {
    OwnerFlowsDriver::new()
}
