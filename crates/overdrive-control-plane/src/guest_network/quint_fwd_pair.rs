//! Quint conformance driver — `fwd_pair`, conformance main `pair_conf`, host
//! side `h`: one guest flow end to end through the in-kernel forwarder
//! (EVMOD; § *Per-module projection and action map* "`fwd_pair`"; QPROJ-8,
//! its second- and third-iteration variables; D-UP-1).
//!
//! Replays `specs/quint/guest-flow-owner/fwd_pair.qnt` (referenced in place)
//! on its recording main `pair_conf` against the production owner
//! (`HostSharedGuestNetworkOwner`) over `SimGuestFlowIo`, whose
//! `SimFlowForwarder` and Sim sockets are the named arbiters of the
//! forwarder's kernel facts (Tier-3 pairs: V-EV-END, V-EV-CTL, V-EV-PAIR,
//! V-EV-DGRAM and the module's suite, P-69 to P-72). The guest side `g` is
//! not under test here (its owner, `overdrive-init`, has its own driver): its
//! actions are stimulus. The driver keeps, as stimulus only, what the guest
//! side has put on the vsock toward `V_h` and not yet delivered (the model's
//! `ctl(gh)`), what the host has put toward the guest (`ctl(hg)`), and the
//! guest pair's phase, which decides which packets the guest sends; none of
//! it is projected. The step record is read through
//! `Config::nondet = &["pair_conf::fwd_pair::conf_step"]`; the state is
//! projected by qualified names ([`Qualified`]).
//!
//! | Action | Production call or fault |
//! |---|---|
//! | `InitWith("stream")` | a fresh composition boots; one lease activated; the guest opens a TcpConnect flow; the owner admits it, reads it and opens D; D's connect is held |
//! | `InitWith("watch")` | the same lease; the driver dials it through the composition's one `OwnerDial` — the inbound enforcement's leg-S `guest_dial` (`dial`, `Relay`; D-13 under LEGS-DIAL, its binding to the owner checked with `OwnerDial::owner()`): `V_h` connects, the watch is installed and the request written |
//! | `InitWith("dgram")` | the guest opens a datagram association; the owner reads it, completes route lookup, opens H and is held at `ConnectUdp` |
//! | `Attach("h")` | stream: release `ConnectPeer`; datagram: release `ConnectUdp`, validate the connected peer, then synchronously attach; the owner sends `Paired` |
//! | `Attach("g")` | the guest takes `Paired` (watch: the guest's `Paired` reaches the owner and the dial returns its `GuestStream`) |
//! | `Abort("h")` | quiescence's first hold (`quiesce`): stream / dgram abort; watch: pending-dial cancellation before handoff, held-stream abort after handoff, and `Abort { flow }` |
//! | `CtxRelease("h")` | one mapping chosen by the state (D-5): with no host pair or watch live (S: the current context's inventory), the `Forwarder` component's repair after a context `Io` failure (`audit_shared` reports it) — `converge_shared` closes the old context and its `open()` is held; with one live, the host owner's death — the seam's process exit (`exit_process`: every handle and held effect dies, the context's release aborts every pair) and the end of the runtime the owner's tasks run on; no host owner step follows (the model makes the death final) |
//! | `Reopen("h")` | the held `open()` is answered: the repair opens and probes the next context |
//! | `TakeEnd("h")` | the owner's dispatch reads the pair's end record (held ends: one released) and, outside watch mode, closes the flow's descriptors in the same dispatch (the model's `TakeEnd(h)` carries `OwnerClose(h)`'s effect) |
//! | `Hop0("hg")`, `HopEnd0("hg")` / `Hop1("gh")`, `HopEnd1("gh")` | one `SimFlowForwarder` run of the host pair, source D/H (side B) / source `V_h` (side A) |
//! | `FwdErr("h")` | the second step of the orphan sweep (`sk_error_report`, after `VmmExit`); else H's error report (after `UdpErr`); else the run that finds the send error |
//! | `VCtl("gh")` | the guest's next packet reaches `V_h`: `0x2` (`SHUT_WR`), `0x3` (close) or `RST` |
//! | `VmmExit` | the orphan sweep's first step: `sk_err` and state on every host vsock to the guest, no callback |
//! | `Write`, `Fin`, `Read`, `ReadEof`, `ShutRd` at the host far end | D's peer sends, FINs, stops reading (stream); the dialer's calls on its production `GuestStream` (watch) |
//! | `TcpRst("h", _)` | D's peer resets (the half's `sk_error_report` runs in the step) |
//! | `HolderDrop` | the driver cancels its pending dial while the watch is live, or drops its actual `GuestStream` after handoff |
//! | `SendDg("hg", sz)`, `UdpBlock("gh")`, `UdpUnblock("gh")`, `UdpErr("h")`, `DgHalfClose("h")` | H's peer sends a datagram of `sz` bytes, stops / resumes reading, H's `sk_err` (`EPERM`), `V_h`'s send shut by a peer outside the design |
//! | guest-side actions (`g`) | stimulus: the guest's pair and the packets it puts on the vsock |
//! | `OwnerClose("h")` | never enabled (the host closes in its `TakeEnd` dispatch): an error |
//! | `OwnerClose("g")` | stimulus: the guest owner's close after its record puts `{S, R}` on the vsock |
//! | `HolderEnd(k)` | the driver's own read of its stream's `V_h` on the half's Sim vsock socket (`SimSocket::local_read`), which reports the scripted end fact: `Eof` for `eof`, `Err` for `err`; any other result fails the step. Not an owner port |
//! | `DRead(d)`, `TrailingRst("g")` | stimulus: the far receiver's read (the Sim socket delivers to its far reader as it accepts, so the read is folded into the forwarder's run) and the guest-side trailing reset |
//! | `DropRecord`, `AttachRead`, `AttachHook`, `UnclassifiedEof`, `WatchRecord` | hazard-only: an error |
//!
//! Projection (side `h`; QPROJ-8, "Host-side projection pins (D-3 to
//! D-5)"; D-11). S — always projected: `ctx(h)`, `oldCtx(h)` ←
//! `SimFlowForwarder`'s context table (the host owner's death releases its
//! context through the seam's process exit, so the table alone shows it);
//! `tcpErr(h)` ← D's `sk_err`; `vs(h)` in full (`.ps` `received_shutdown`,
//! `.sent` `sent_shutdown`, `.err` `sk_err`, `.cls` `recorded_class`, `.st`
//! `vsock_state`, `.sshut` `own_send_shut`) ← the half's shared `SimSocket`
//! (`SimGuestFlowIo::vsock_half`). C — always: `hres` ← what the production
//! `GuestStream` returned at `ReadEof("gh")`. O — while the host owner lives
//! (after its death no owner exists and these are not compared): `gen(h)` ←
//! `forwarder_generation` (model 0 ↔ `FIRST`); `attRes(h)` / `attEnded(h)` ←
//! the cumulative `forwarder_attached` / `forwarder_attach_ended`, readable
//! after the flow left the table; `pr(h)` `.ph` ← the flow's phase
//! (`guest_flows` / `datagrams` / `dials`) with S the pair's or watch's
//! liveness and its queued record; `.cgen` while live ← the phase's
//! `generation`; `.rec` ← `queued` ⇔ S the flow's id in `queued_ends` of the
//! context row at its generation (D-4), else `read` ⇔ the owner's
//! `last_consumed_end` is the flow's pair's or watch's record (D-11), else
//! `none`; `.nrec` ← the queued count plus one if consumed; `.out` /
//! `.cause` / `.errno` ← that consumed record's outcome, compared once `rec =
//! read` (never a record the driver took from `SimGuestFlowIo`); `.closed` ⇔
//! the flow left the table after it was attached and its `V_h` sent `{S,
//! R}` (outside watch mode); `.byRel` ⇔ the flow left the table with no
//! record consumed and none queued (outside watch mode); `strmAbort` ⇔ the
//! dial's entry is absent while the driver holds its stream (D-3);
//! `gh.unclassified` ← `inventory().unclassified_eof`. Not projected: the
//! queues, the far ends' facts, and the other side's and the model-only
//! variables. The flow's `(generation, ForwardId)` is a relabelling the
//! driver fixes when it first sees the attached pair or the dial's watch,
//! never a source of state.
//!
//! **`fwd_safe` has no driver (model-only; testing.md checklist item 6).**
//! `fwd_safe.qnt`'s named-run main `safe_runs` is not a conformance target
//! (feature-delta § *Formal protocol model*, "`fwd_safe`"). Its variables are
//! orderings inside `ovd_fwd`: RCU readers, the work item's PENDING bit, the
//! free's phase, socket-lock and wait-queue-lock holds, a store not yet
//! visible, the module's reference count and whether its text may run. No
//! owner accessor exposes them, and its actions are interleavings no driving
//! port can schedule. No production owner unloads the module either: the
//! appliance kernel has no module unloading, and `kmod-load` is development
//! tooling. Each run is checked by the model only (`fwd-safe-run-*`,
//! `expect = "holds"`), and each rule's teeth by its `fwd_safe` hazard. The
//! run's observable consequence is checked where an owner or the module
//! exposes it:
//!
//! | `safe_runs` run | Observable consequence, and where it is checked |
//! |---|---|
//! | `TargetStallResumesOnWriteSpace` | the stalled direction resumes byte-exact: this driver's `TargetStallBlocksSenderThenResumes`; module suite `stream_battery::stall_a_reader_that_stops_blocks_the_sender_and_resumes_byte_exact` (TCP target); V-EV-SAFE's full-target resume cases (vsock and UDP targets) |
//! | `WakeupDuringRunIsNotLost` | every unit arrives: `CleanBothWaysCompletes`, `EarlyBytesBeforeAttachDelivered`; `stream_battery::burst_ten_thousand_small_writes_arrive_in_order` |
//! | `CleanEndProducesOneRecordAndFrees` | one record per pair: `pr(h).nrec` in `CleanBothWaysCompletes`; `stream_battery::cycles_a_thousand_flows_each_end_with_one_record_and_leave_nothing_live` |
//! | `LateCallbackAfterEndCallsOriginal` | a socket event after the end changes nothing: `TrailingRstAfterCompletionIsInert`; `stream_end::end_idempotent_a_trailing_reset_after_completion_adds_nothing`; the free order itself by V-EV-SAFE |
//! | `CallbackHoldingPairDelaysFree` | none outside the kernel (a callback in flight across the grace period): V-EV-SAFE (A-45) |
//! | `ControlCallAfterIdRemovalIsEnoent` | `FlowForwarder::abort` of an ended id answers `AbortResult::AlreadyEnded`: `watch::watch_abort_records_unhooks_and_never_touches_the_socket` (its stale-id abort); V-EV-SAFE's control calls on ending ids |
//! | `AbortRacingEndIsCancelledByFree` | one record per id: `context_release::a_thousand_pair_close_storm_yields_exactly_one_record_per_id`; V-EV-SAFE |
//! | `SweepReportedEndAbortsSocketError` | `VmmExitAbortsHostHalf`; `guest_boundary::a_vmm_exit_aborts_a_live_host_pair_with_the_device_releases_reset` |
//! | `SweepStoresBeforeErrReadRechecksUnderTableLock`, `SweepErrVisibleFirstRechecksUnderTableLock` | no unclassified end: `gh.unclassified` ← `inventory().unclassified_eof`, compared at every step of this driver's `VmmExit` runs; the sweep's store order cannot be forced from outside the kernel, so the window itself has no measurement (§ *End classification*) |
//! | `ReleaseEndsPairWithoutRecord` | `ContextReleaseResetsEveryPair`; `context_release::a_released_context_aborts_every_pair_and_watch_and_resets_every_far_end`; `watch::a_context_release_shuts_every_watched_socket_and_queues_no_record` |
//! | `WatchOverrideEndThenOwnerAbortIsEnoent` | the same `AlreadyEnded` answer as `ControlCallAfterIdRemovalIsEnoent`; the module suite's stale-id abort removes the id by the owner's abort, not by the watch's own end, so the own-end case rests on V-EV-SAFE's control calls on ending ids |
//! | `WatchOwnerAbortRacingSweepOneRecord` | `VmmExitWithWatchRecordInFlightResolvesFromRecord`; `guest_boundary::a_watched_half_records_the_sweep_before_its_holder_observes_the_reset` |
//! | `UnloadRefusedWhileContextOpen` | V-EV-UNLOAD's context half, `unload::an_open_context_refuses_the_unload_and_its_close_lets_it_succeed` (`kvm-tests`, metal) |
//! | `UnloadRefusedWhileSocketHooked` | V-EV-UNLOAD's socket half, `unload::hooked_sockets_hold_the_module_until_their_free_work_and_a_reload_then_serves` |
//! | `UnloadSucceedsAfterReleaseAndFree` | both V-EV-UNLOAD cells above (the unload succeeds once the context closes and the free work returned) |
//! | `LateLockedCallbackFinishesBeforeRemoval` | none outside the kernel (a lock-owning caller inside an override at the removal): V-EV-UNLOAD's unload stress and the DELIVER source review of rule 5 |
//! | `UlpCopyKeepsModuleLoaded` | the ULP is an environment fault the owner rules out (ADR-0200): attach refuses a ULP half, `attach_watch::a_tcp_half_with_an_upper_layer_protocol_is_refused`; a ULP installed after attach is V-EV-UNLOAD's ULP control |
//! | `WfreeWakeFinishesBeforeRemoveWait` | none outside the kernel (a `sock_wfree` waker inside the wait-queue entry): V-EV-SAFE's vsock and UDP resume cases, V-EV-UNLOAD's stress with senders on every vsock and UDP half, and the DELIVER source review of `sk_write_space` |

#![allow(
    clippy::needless_pass_by_ref_mut,
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
    clippy::struct_excessive_bools,
    reason = "test support and conformance drivers: fixture preconditions are asserted with expect, and handler signatures follow the action map"
)]

use std::collections::{BTreeSet, VecDeque};
use std::net::SocketAddrV4;
use std::sync::Arc;

use itf::Value;
use overdrive_core::guest_transport::dial::{
    DialPurpose, GuestDialError, GuestStream, GuestStreamDial as _,
};
use overdrive_core::guest_transport::forwarder::{
    AbortCause, ForwardEnd, ForwardId, ForwardOutcome, ForwarderGeneration, ForwarderOp, PairSide,
    SourceEnd,
};
use overdrive_core::guest_transport::wire::{
    ControlMessage, DatagramFrame, FlowId, FlowRefusal, KernelErrno,
};
use overdrive_sim::adapters::{
    SimContextState, SimLocalRead, SimShutdown, SimSocket, SimVsockState,
};
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use tokio::task::JoinHandle;

use super::flow_io::sim::{OwnerShutdownRead, SimEffectKind, SimFailure, SimForwarderFailure};
use super::owner_acceptance::support::{Composition, Lease, paused_runtime, remote};
use super::quint_itf::{self, ItfError, Qualified};
use super::{
    AssociationPhase, DialPhase, ForwardingQuiescence, GuestConnKey, GuestFlowPhase,
    HostSharedGuestNetworkOwner, QuiescenceHold, SharedGuestNetworkOwner,
};

const MAIN: &str = "pair_conf";
const MODULE: &str = "fwd_pair";
const SEED: u64 = 0x0F7A;
/// The declared listen port the dial targets (watch mode).
const PORT: u16 = 8_080;

// ----- the compared state ---------------------------------------------------

/// `vs(h)`: the host vsock half's facts (S, `SimGuestFlowIo::vsock_half`).
#[derive(Debug, PartialEq, Eq)]
struct VsH {
    ps: BTreeSet<String>,
    sent: BTreeSet<String>,
    err: bool,
    cls: String,
    st: String,
    sshut: bool,
}

impl VsH {
    /// `vs0`: a half the seam has not made yet.
    fn none() -> Self {
        Self {
            ps: BTreeSet::new(),
            sent: BTreeSet::new(),
            err: false,
            cls: "none".to_owned(),
            st: "est".to_owned(),
            sshut: false,
        }
    }
}

/// The end record the host owner consumed (`pr(h)` `.out`, `.cause`,
/// `.errno`), compared once `rec = read` (D-11).
#[derive(Debug, PartialEq, Eq)]
struct EndH {
    out: String,
    cause: String,
    errno: String,
}

/// The host owner's facts (O), compared only while the host owner lives.
#[derive(Debug, PartialEq, Eq)]
struct OwnerH {
    generation: i64,
    ph: String,
    live_cgen: Option<i64>,
    rec: String,
    end: Option<EndH>,
    nrec: i64,
    closed: bool,
    by_rel: bool,
    att_res: String,
    att_ended: i64,
    strm_abort: bool,
    unclassified: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct PairStateH {
    ctx: String,
    old_ctx: String,
    tcp_err: String,
    vs: VsH,
    hres: String,
    /// The actual move-only GuestStream is currently held by this driver.
    strm_held: bool,
    /// `None` after the host owner's death (`gh.hostDead`): no owner exists.
    owner: Option<OwnerH>,
}

fn at_h(value: &Value) -> std::result::Result<&Value, ItfError> {
    quint_itf::map_entries(value)?
        .into_iter()
        .find(|(key, _)| quint_itf::string(key).is_ok_and(|side| side == "h"))
        .map(|(_, v)| v)
        .ok_or(ItfError::MissingField("h".to_owned()))
}

impl PairStateH {
    fn from_value(value: &Value) -> std::result::Result<Self, ItfError> {
        let q = Qualified::of(value, MAIN, MODULE)?;
        let gh = q.var("gh")?;
        let host_dead = quint_itf::boolean(quint_itf::field(gh, "hostDead")?)?;
        let vs = at_h(q.var("vs")?)?;
        let vs = VsH {
            ps: quint_itf::str_set(quint_itf::field(vs, "ps")?)?,
            sent: quint_itf::str_set(quint_itf::field(vs, "sent")?)?,
            err: quint_itf::boolean(quint_itf::field(vs, "err")?)?,
            cls: quint_itf::string(quint_itf::field(vs, "cls")?)?,
            st: quint_itf::string(quint_itf::field(vs, "st")?)?,
            sshut: quint_itf::boolean(quint_itf::field(vs, "sshut")?)?,
        };
        let owner = if host_dead {
            None
        } else {
            let pr = at_h(q.var("pr")?)?;
            let ph = quint_itf::string(quint_itf::field(pr, "ph")?)?;
            let cgen = quint_itf::int(quint_itf::field(pr, "cgen")?)?;
            let rec = quint_itf::string(quint_itf::field(pr, "rec")?)?;
            let end = if rec == "read" {
                Some(EndH {
                    out: quint_itf::string(quint_itf::field(pr, "out")?)?,
                    cause: quint_itf::string(quint_itf::field(pr, "cause")?)?,
                    errno: quint_itf::string(quint_itf::field(pr, "errno")?)?,
                })
            } else {
                None
            };
            Some(OwnerH {
                generation: quint_itf::int(at_h(q.var("gen")?)?)?,
                live_cgen: (ph == "live").then_some(cgen),
                ph,
                rec,
                end,
                nrec: quint_itf::int(quint_itf::field(pr, "nrec")?)?,
                closed: quint_itf::boolean(quint_itf::field(pr, "closed")?)?,
                by_rel: quint_itf::boolean(quint_itf::field(pr, "byRel")?)?,
                att_res: quint_itf::string(at_h(q.var("attRes")?)?)?,
                att_ended: quint_itf::int(at_h(q.var("attEnded")?)?)?,
                strm_abort: quint_itf::boolean(q.var("strmAbort")?)?,
                unclassified: quint_itf::boolean(quint_itf::field(gh, "unclassified")?)?,
            })
        };
        Ok(Self {
            ctx: quint_itf::string(at_h(q.var("ctx")?)?)?,
            old_ctx: quint_itf::string(at_h(q.var("oldCtx")?)?)?,
            tcp_err: quint_itf::string(at_h(q.var("tcpErr")?)?)?,
            vs,
            hres: quint_itf::string(q.var("hres")?)?,
            strm_held: quint_itf::boolean(q.var("strmHeld")?)?,
            owner,
        })
    }
}

impl<'de> Deserialize<'de> for PairStateH {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

impl State<FwdPairDriver> for PairStateH {
    /// The raw ITF state, decoded by the shared ITF helpers (the serde
    /// path renders `#bigint` as a string and `#map` as a record, QPROJ-1).
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &FwdPairDriver) -> Result<Self> {
        driver.project()
    }
}

#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("action `{0}` is hazard-only: it is never enabled in a design instance")]
    HazardOnly(&'static str),
    #[error("action `{action}` cannot be played: {why}")]
    NotPlayable { action: &'static str, why: String },
}

fn not_playable(action: &'static str, why: impl Into<String>) -> anyhow::Error {
    DriverError::NotPlayable { action, why: why.into() }.into()
}

// ----- stimulus: the guest side and the vsock packets ----------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Stream,
    Watch,
    Dgram,
}

/// A SHUTDOWN / RST packet on the vsock (`{S}`, `{S, R}` from a close or a
/// shutdown of both directions, `{S, R}` from a kernel's reset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Packet {
    Shut,
    Close,
    Rst,
}

impl Packet {
    fn flags(self) -> BTreeSet<char> {
        match self {
            Self::Shut => BTreeSet::from(['S']),
            Self::Close | Self::Rst => BTreeSet::from(['S', 'R']),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuestPhase {
    Unattached,
    Live,
    Ended,
    Refused,
    Gone,
}

/// The guest side, as stimulus: the facts that decide what it puts on the
/// vsock toward `V_h` (EVMOD-END's table, read at its own half).
#[derive(Debug)]
struct GuestSide {
    phase: GuestPhase,
    /// `peer_shutdown` of `V_g`: the host's packets processed.
    ps: BTreeSet<char>,
    /// The flags `V_g` sent.
    sent: BTreeSet<char>,
    tcp_err: bool,
    udp_err: bool,
    in_clean: bool,
    out_clean: bool,
    /// Units (datagram mode: frame items) the host put on the vsock and the
    /// guest forwarder has not consumed.
    inflight: u32,
}

impl GuestSide {
    fn new() -> Self {
        Self {
            phase: GuestPhase::Unattached,
            ps: BTreeSet::new(),
            sent: BTreeSet::new(),
            tcp_err: false,
            udp_err: false,
            in_clean: false,
            out_clean: false,
            inflight: 0,
        }
    }
}

pub(super) struct FwdPairDriver {
    rt: tokio::runtime::Runtime,
    /// The production owner under test (QPROJ-3), the composition's own.
    owner: Arc<HostSharedGuestNetworkOwner>,
    /// C: the driver ended the host owner (a `CtxRelease("h")` with a pair or
    /// watch live): its seam exited and its runtime ended; it takes no step
    /// and is not projected.
    owner_dead: bool,
    /// The owner's last actual serving generation, captured before its
    /// process/seam exits so post-mortem projection performs no owner I/O.
    dead_forwarder_generation: Option<ForwarderGeneration>,
    /// The flow's pair or watch: relabelling fixed when first seen.
    ids: Option<(ForwarderGeneration, ForwardId)>,
    /// The composition the owner stands on (its seam sim and pool).
    c: Composition,
    lease: Option<Lease>,
    session: Option<GuestConnKey>,
    mode: Mode,
    flow: Option<FlowId>,
    key: Option<GuestConnKey>,
    /// The exact host peer socket a pre-attach stimulus reached. The Sim seam
    /// learns its flow only at attach, so retain the same socket for QPROJ-8
    /// after owner cleanup removes its live handle.
    host_peer: Option<SimSocket>,
    destination: SocketAddrV4,
    dial: Option<JoinHandle<std::result::Result<GuestStream, GuestDialError>>>,
    stream: Option<GuestStream>,
    quiesce: Option<JoinHandle<(QuiescenceHold, ForwardingQuiescence)>>,
    repair: Option<JoinHandle<super::Result<()>>>,
    /// The counted hold the real supervisor acquires before a Forwarder
    /// recovery attempt and restores only after the repaired owner audits
    /// clean (ADR-0124 / ADR-0169).
    recovery_hold: Option<QuiescenceHold>,
    /// C: the outcome the production `GuestStream` returned at `ReadEof`.
    hres: &'static str,
    guest: GuestSide,
    /// The guest's packets toward `V_h` not yet delivered (`ctl(gh)`).
    to_host: VecDeque<Packet>,
    /// The host's packets toward the guest not yet processed (`ctl(hg)`).
    to_guest: VecDeque<Packet>,
    /// The host vsock's sent flags already turned into `to_guest` packets.
    host_sent_seen: Option<SimShutdown>,
    vmm_exited: bool,
    swept: bool,
    udp_err_h: bool,
    unit_gh: u8,
    unit_hg: u8,
    /// Guest datagrams (sizes) in its source queue (WU = 1) and the items of
    /// the head frame already sent.
    gh_dgrams: VecDeque<usize>,
    gh_foff: usize,
    /// Sizes of the destination's datagrams, for the guest's consumption.
    hg_dgrams: VecDeque<usize>,
}

impl FwdPairDriver {
    fn new() -> Self {
        let rt = paused_runtime();
        let c = {
            let _runtime = rt.enter();
            Composition::non_mesh(SEED)
        };
        Self {
            rt,
            owner: Arc::clone(&c.owner),
            owner_dead: false,
            dead_forwarder_generation: None,
            ids: None,
            c,
            lease: None,
            session: None,
            mode: Mode::Stream,
            flow: None,
            key: None,
            host_peer: None,
            destination: remote(PORT),
            dial: None,
            stream: None,
            quiesce: None,
            repair: None,
            recovery_hold: None,
            hres: "none",
            guest: GuestSide::new(),
            to_host: VecDeque::new(),
            to_guest: VecDeque::new(),
            host_sent_seen: None,
            vmm_exited: false,
            swept: false,
            udp_err_h: false,
            unit_gh: 0,
            unit_hg: 0,
            gh_dgrams: VecDeque::new(),
            gh_foff: 0,
            hg_dgrams: VecDeque::new(),
        }
    }

    fn composition(&self) -> &Composition {
        &self.c
    }

    fn lease(&self) -> &Lease {
        self.lease.as_ref().expect("InitWith provisioned the lease")
    }

    fn flow(&self, action: &'static str) -> Result<FlowId> {
        self.flow.ok_or_else(|| not_playable(action, "no flow"))
    }

    fn settle(&mut self) {
        self.rt.block_on(self.composition().settle());
        self.capture_host_packets();
        self.note_ids();
    }

    /// Fixes the relabelling of the flow's pair (stream, dgram: its `Paired`
    /// phase) or the dial's watch (the seam's record of the watch the owner
    /// made, in the context it held then) the first time it is seen.
    fn note_ids(&mut self) {
        if self.ids.is_some() || self.owner_dead {
            return;
        }
        let (Some(flow), Some(lease)) = (self.flow, self.lease.as_ref()) else { return };
        let observed = self.rt.block_on(self.owner.flow_observation());
        let Some(cid) = observed.cids.get(&lease.cid) else { return };
        self.ids = match self.mode {
            Mode::Stream => self.key.and_then(|key| match cid.guest_flows.get(&key) {
                Some(GuestFlowPhase::Paired { pair, generation }) => Some((*generation, *pair)),
                _ => None,
            }),
            Mode::Dgram => match cid.datagrams.get(&flow).map(|assoc| assoc.phase) {
                Some(AssociationPhase::Paired { pair, generation }) => Some((generation, pair)),
                _ => None,
            },
            Mode::Watch => self
                .c
                .io
                .dials()
                .first()
                .and_then(|dial| dial.watch)
                .map(|watch| (observed.forwarder_generation, watch)),
        };
    }

    /// The host's own shutdowns of `V_h` become packets toward the guest.
    fn capture_host_packets(&mut self) {
        let Some(vsock) = self.flow.and_then(|flow| self.composition().io.vsock_half(flow)) else {
            return;
        };
        let now = vsock.sent_shutdown();
        match (self.host_sent_seen, now) {
            (None, Some(SimShutdown::Send)) => self.to_guest.push_back(Packet::Shut),
            (None | Some(SimShutdown::Send), Some(SimShutdown::Both)) => {
                self.to_guest.push_back(Packet::Close);
            }
            _ => {}
        }
        if now.is_some() {
            self.host_sent_seen = now;
        }
    }

    // ----- init -----

    fn init_with(&mut self, m: &str) -> Result {
        *self = Self::new();
        self.mode = match m {
            "stream" => Mode::Stream,
            "watch" => Mode::Watch,
            "dgram" => Mode::Dgram,
            other => return Err(not_playable("InitWith", format!("unknown mode {other}"))),
        };
        self.rt.block_on(self.c.boot());
        let c = &self.c;
        eprintln!("fwd_pair host driver: composition seed {SEED:#x}");
        // Forwarder runs and end-record reads are the trace's steps.
        c.io.set_auto_run(false);
        c.io.set_auto_ends(false);
        let (lease, session) = self.rt.block_on(c.active_lease("fwd-pair", &[PORT]));
        match self.mode {
            Mode::Stream => {
                c.io.hold(SimEffectKind::ConnectPeer);
                let (key, flow) = c.open_tcp_flow(&lease, 1, self.destination, 2_000);
                self.key = Some(key);
                self.flow = Some(flow);
            }
            Mode::Dgram => {
                c.io.hold(SimEffectKind::ConnectUdp);
                let (key, flow) = c.open_datagram(&lease, 1, self.destination, 2_000);
                self.key = Some(key);
                self.flow = Some(flow);
            }
            Mode::Watch => {
                // D-13 under LEGS-DIAL: a leg-S relay dial reaches the owner
                // through the boot's one `OwnerDial` — the `guest_dial` the
                // composition's enforcement was built with. Its binding is
                // the composed owner (`OwnerDial::owner()`, the pinned
                // accessor); its use is the `Relay` entry in that owner's
                // `dials`, which `note_ids` and the projection read.
                let bound = c
                    .dial
                    .owner()
                    .expect("compose_guest_network binds the boot's OwnerDial (D-13)");
                assert!(
                    Arc::ptr_eq(&bound, &self.owner),
                    "the boot's OwnerDial is bound to the composed owner (D-13)"
                );
                let dial = Arc::clone(&c.dial);
                let target = lease.target(PORT);
                let _runtime = self.rt.enter();
                self.dial =
                    Some(tokio::spawn(async move { dial.dial(target, DialPurpose::Relay).await }));
            }
        }
        self.lease = Some(lease);
        self.session = Some(session);
        self.settle();
        if self.mode == Mode::Watch {
            self.flow = Some(self.composition().dial_flow(0));
            self.capture_host_packets();
            self.note_ids();
        }
        Ok(())
    }

    // ----- host owner steps -----

    fn attach_h(&mut self) -> Result {
        let kind = match self.mode {
            Mode::Stream => SimEffectKind::ConnectPeer,
            Mode::Dgram => SimEffectKind::ConnectUdp,
            Mode::Watch => {
                return Err(not_playable("Attach", "a dial's watch exists from its start"));
            }
        };
        let c = self.composition();
        let held =
            c.io.held_of(kind)
                .ok_or_else(|| not_playable("Attach", "no held step before the host attach"))?;
        c.io.release(held);
        self.settle();
        Ok(())
    }

    /// A host owner step after the host owner's death is not a production
    /// state (the model makes the death final).
    fn alive(&self, action: &'static str) -> Result {
        if self.owner_dead {
            return Err(not_playable(action, "the host owner is dead; no host owner step follows"));
        }
        Ok(())
    }

    fn abort_h(&mut self) -> Result {
        self.alive("Abort")?;
        let owner = Arc::clone(&self.owner);
        let _runtime = self.rt.enter();
        self.quiesce = Some(tokio::spawn(async move { owner.quiesce().await }));
        self.settle();
        Ok(())
    }

    /// `CtxRelease("h")`: one mapping chosen by the state, never by what
    /// follows (D-5). S: the current context's inventory says whether the
    /// host pair or watch is live.
    fn ctx_release_h(&mut self) -> Result {
        self.alive("CtxRelease")?;
        // conformance-lint: sim-arbiter-ok QPROJ-8 the current context's live pairs and watches through SimFlowForwarder, Tier-3 pair V-EV-CTL and the module suite
        let inventory = self.c.io.forwarder_inventory();
        if inventory.live_pairs + inventory.live_watches > 0 {
            self.owner_death();
            return Ok(());
        }
        self.forwarder_recovery_close()
    }

    /// The host owner's death with its context: the `serve` process ends.
    /// The seam's process exit drops every handle and held effect and
    /// releases the context (every pair aborted, far ends reset, no record);
    /// the runtime the owner's tasks run on ends with them. The driver keeps
    /// only the sockets' facts (S) and its stream's results (C).
    fn owner_death(&mut self) {
        self.dead_forwarder_generation =
            Some(self.rt.block_on(self.owner.flow_observation()).forwarder_generation);
        self.c.io.exit_process();
        self.dial = None;
        self.quiesce = None;
        self.repair = None;
        let ended = std::mem::replace(&mut self.rt, paused_runtime());
        drop(ended);
        self.owner_dead = true;
        self.capture_host_packets();
    }

    /// The `Forwarder` recovery's close of the old context, with nothing
    /// live: a scripted context `Io` failure damages `Forwarder`; the repair
    /// (`converge_shared`) ends every flow in the owner's table, closes the
    /// old context and is held at its `open()` until `Reopen("h")`.
    fn forwarder_recovery_close(&mut self) -> Result {
        self.composition().io.script_failure(
            SimEffectKind::Inventory,
            SimFailure::Forwarder(SimForwarderFailure::ContextIo(ForwarderOp::Inventory)),
        );
        let audit = self.rt.block_on(self.owner.audit_shared());
        if audit.is_ok() {
            return Err(not_playable(
                "CtxRelease",
                "the context's Io failure did not damage Forwarder",
            ));
        }
        if self.recovery_hold.is_some() {
            return Err(not_playable("CtxRelease", "a Forwarder recovery hold is already active"));
        }
        let (hold, _unconfirmed) = self.rt.block_on(self.owner.quiesce());
        self.recovery_hold = Some(hold);
        self.composition().io.hold(SimEffectKind::OpenForwarder);
        let owner = Arc::clone(&self.owner);
        let _runtime = self.rt.enter();
        self.repair = Some(tokio::spawn(async move { owner.converge_shared().await }));
        self.settle();
        Ok(())
    }

    fn reopen_h(&mut self) -> Result {
        self.alive("Reopen")?;
        let c = self.composition();
        let open_effect =
            c.io.held_of(SimEffectKind::OpenForwarder)
                .ok_or_else(|| not_playable("Reopen", "no repair is waiting on its open()"))?;
        c.io.release(open_effect);
        self.settle();
        let repair = self.repair.take().ok_or_else(|| not_playable("Reopen", "no repair"))?;
        self.rt
            .block_on(repair)
            .map_err(|error| not_playable("Reopen", error.to_string()))?
            .map_err(|error| not_playable("Reopen", error.to_string()))?;
        self.rt
            .block_on(self.owner.audit_shared())
            .map_err(|error| not_playable("Reopen", error.to_string()))?;
        let quiescence_hold = self
            .recovery_hold
            .take()
            .ok_or_else(|| not_playable("Reopen", "the Forwarder recovery holds no quiescence"))?;
        self.rt
            .block_on(self.owner.restore(quiescence_hold))
            .map_err(|error| not_playable("Reopen", error.to_string()))?;
        self.settle();
        Ok(())
    }

    fn take_end_h(&mut self) -> Result {
        self.alive("TakeEnd")?;
        self.composition().io.release_end();
        self.settle();
        Ok(())
    }

    /// `HolderEnd(k)`: the driver's own read of its stream's `V_h` on the
    /// half's Sim vsock socket (QPROJ-8: not an owner port), which reports
    /// the scripted end fact.
    fn holder_end(&mut self, k: &str) -> Result {
        if self.mode == Mode::Watch && self.stream.is_none() {
            return Err(not_playable("HolderEnd", "the dial has no held stream"));
        }
        let flow = self.flow("HolderEnd")?;
        let vsock = self
            .composition()
            .io
            .vsock_half(flow)
            .ok_or_else(|| not_playable("HolderEnd", "the dial has no V_h"))?;
        let selected = match k {
            "eof" => OwnerShutdownRead::Eof,
            "err" => OwnerShutdownRead::NotConnected,
            other => return Err(not_playable("HolderEnd", format!("unknown end result {other}"))),
        };
        let read = self
            .composition()
            .io
            .owner_shutdown_read(flow, usize::MAX, selected)
            .unwrap_or_else(|| vsock.local_read(usize::MAX));
        let observed = match read {
            SimLocalRead::Eof => "eof",
            SimLocalRead::Err(_) => "err",
            other => {
                return Err(not_playable(
                    "HolderEnd",
                    format!("the holder's read of V_h returned {other:?}, not an end"),
                ));
            }
        };
        if observed != k {
            return Err(not_playable(
                "HolderEnd",
                format!("the holder's read of V_h observed `{observed}`, the trace `{k}`"),
            ));
        }
        self.settle();
        Ok(())
    }

    /// The pair identity the production owner currently exposes. An
    /// unattached or already-ended flow has no pair to run; after owner death
    /// there is no owner observation to make.
    fn observed_pair(
        &self,
        action: &'static str,
    ) -> Result<Option<(ForwarderGeneration, ForwardId)>> {
        if self.owner_dead {
            return Ok(None);
        }
        let flow = self.flow(action)?;
        self.rt.block_on(self.composition().settle());
        let observed = self.rt.block_on(self.owner.flow_observation());
        let Some(cid) = observed.cids.get(&self.lease().cid) else { return Ok(None) };
        let pair = match self.mode {
            Mode::Stream => self.key.and_then(|key| match cid.guest_flows.get(&key) {
                Some(GuestFlowPhase::Paired { pair, generation }) => Some((*generation, *pair)),
                _ => None,
            }),
            Mode::Dgram => match cid.datagrams.get(&flow).map(|assoc| assoc.phase) {
                Some(AssociationPhase::Paired { pair, generation }) => Some((generation, pair)),
                _ => None,
            },
            Mode::Watch => None,
        };
        Ok(pair.filter(|(generation, pair)| {
            *generation == observed.forwarder_generation
                && self.composition().io.forwarder_pair_state(*pair).is_some()
        }))
    }

    fn pair_id(&self, action: &'static str) -> Result<ForwardId> {
        self.alive(action)?;
        self.observed_pair(action)?
            .map(|(_, pair)| pair)
            .ok_or_else(|| not_playable(action, "the host pair is not attached"))
    }

    fn run_h(&mut self, source: PairSide, action: &'static str) -> Result {
        let id = self.pair_id(action)?;
        let _ran = self.composition().io.run_forwarder_once(id, source);
        self.settle();
        Ok(())
    }

    fn fwd_err_h(&mut self) -> Result {
        let flow = self.flow("FwdErr")?;
        let sweep_callback = self.vmm_exited && !self.swept;
        let udp_callback = !sweep_callback && self.udp_err_h;
        if !sweep_callback && !udp_callback {
            return self.run_h(PairSide::B, "FwdErr");
        }
        let pair_to_run =
            if self.mode == Mode::Watch { None } else { self.observed_pair("FwdErr")? };
        if sweep_callback {
            self.swept = true;
            self.composition().io.finish_sweep_of(flow);
        } else {
            self.udp_err_h = false;
            self.peer_socket("FwdErr")?.report_error();
            self.composition().io.caused();
        }
        if let Some((generation, pair)) = pair_to_run
            && !self.composition().io.run_forwarder_once(pair, PairSide::A)
        {
            return Err(not_playable(
                "FwdErr",
                format!(
                    "error callback queued no run for attached pair {pair:?} in generation {generation}"
                ),
            ));
        }
        self.settle();
        Ok(())
    }

    // ----- the host far end -----

    /// D / H: the attached socket, or the one the owner holds before attach.
    fn peer_socket(&mut self, action: &'static str) -> Result<SimSocket> {
        let socket = {
            let c = self.composition();
            self.flow
                .and_then(|flow| c.io.peer_of_opt(flow))
                .or_else(|| self.host_peer.clone())
                .or_else(|| c.io.latest_peer_socket())
        }
        .ok_or_else(|| not_playable(action, "the owner holds no peer socket"))?;
        self.host_peer = Some(socket.clone());
        Ok(socket)
    }

    fn on_peer(&mut self, action: &'static str, cause: impl FnOnce(&SimSocket)) -> Result {
        let socket = self.peer_socket(action)?;
        cause(&socket);
        self.composition().io.caused();
        self.settle();
        Ok(())
    }

    fn write_hg(&mut self) -> Result {
        self.unit_hg = self.unit_hg.wrapping_add(1);
        let unit = [self.unit_hg];
        match self.mode {
            Mode::Watch => {
                let mut stream =
                    self.stream.take().ok_or_else(|| not_playable("Write", "no stream"))?;
                let wrote = self.rt.block_on(stream.write(&unit));
                self.stream = Some(stream);
                wrote.map_err(|error| not_playable("Write", error.to_string()))?;
                self.guest.inflight += 1;
                self.settle();
                Ok(())
            }
            _ => self.on_peer("Write", |socket| socket.peer_send(&unit)),
        }
    }

    fn fin_hg(&mut self) -> Result {
        match self.mode {
            Mode::Watch => {
                let mut stream =
                    self.stream.take().ok_or_else(|| not_playable("Fin", "no stream"))?;
                let shut = self.rt.block_on(stream.shutdown_write());
                self.stream = Some(stream);
                shut.map_err(|error| not_playable("Fin", error.to_string()))?;
                self.settle();
                Ok(())
            }
            _ => self.on_peer("Fin", |socket| socket.peer_shutdown(SimShutdown::Send)),
        }
    }

    fn read_gh(&mut self) -> Result {
        if self.mode != Mode::Watch {
            self.settle();
            return Ok(());
        }
        let mut stream = self.stream.take().ok_or_else(|| not_playable("Read", "no stream"))?;
        let mut buf = [0_u8; 1];
        let read = self.rt.block_on(stream.read(&mut buf));
        self.stream = Some(stream);
        match read {
            Ok(1) => {}
            other => {
                return Err(not_playable(
                    "Read",
                    format!("the stream returned {other:?}, not one unit"),
                ));
            }
        }
        self.settle();
        Ok(())
    }

    fn read_eof_gh(&mut self) -> Result {
        if self.mode != Mode::Watch {
            self.settle();
            return Ok(());
        }
        let mut stream = self.stream.take().ok_or_else(|| not_playable("ReadEof", "no stream"))?;
        let mut buf = [0_u8; 8];
        let read = self.rt.block_on(stream.read(&mut buf));
        self.stream = Some(stream);
        self.hres = match read {
            Ok(0) => "HalfClose",
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => "GuestAbort",
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionAborted => "OwnerAbort",
            other => return Err(not_playable("ReadEof", format!("the stream returned {other:?}"))),
        };
        self.settle();
        Ok(())
    }

    fn holder_drop(&mut self) {
        if let Some(stream) = self.stream.take() {
            drop(stream);
        } else if let Some(dial) = self.dial.take() {
            dial.abort();
            drop(self.rt.block_on(dial));
        }
        self.settle();
    }

    fn vmm_exit(&mut self) {
        let cid = self.lease().cid;
        let launch = self.lease().launch;
        self.composition().io.vm_stop_before_callbacks(cid, launch);
        self.vmm_exited = true;
        if !matches!(self.guest.phase, GuestPhase::Ended | GuestPhase::Refused) {
            self.guest.phase = GuestPhase::Gone;
        }
        self.settle();
    }

    // ----- the guest side (stimulus) -----

    fn guest_shut_all(&mut self) {
        if self.guest.sent != Packet::Close.flags() {
            self.to_host.push_back(Packet::Close);
            self.guest.sent = Packet::Close.flags();
        }
    }

    /// The guest pair (or a flow it has not attached, when `unattached`)
    /// aborts: SHUT_RDWR of `V_g`.
    fn guest_abort(&mut self, unattached: bool) {
        let applies = self.guest.phase == GuestPhase::Live
            || (unattached && self.guest.phase == GuestPhase::Unattached);
        if applies {
            self.guest.phase = GuestPhase::Ended;
            self.guest_shut_all();
        }
        self.settle();
    }

    fn guest_complete_if_done(&mut self) {
        if self.guest.phase == GuestPhase::Live && self.guest.in_clean && self.guest.out_clean {
            self.guest.phase = GuestPhase::Ended;
        }
    }

    fn guest_mark_clean(&mut self) {
        if self.mode == Mode::Dgram && self.guest.inflight == 0 {
            self.guest.phase = GuestPhase::Ended;
        }
    }

    fn attach_g(&mut self) -> Result {
        if self.guest.phase != GuestPhase::Unattached {
            return Err(not_playable("Attach", "the guest pair is not unattached"));
        }
        let closing = self.guest.ps == Packet::Close.flags() && self.guest.inflight == 0;
        let attach_refused = self.guest.tcp_err || closing;
        match self.mode {
            Mode::Watch => {
                let flow = self.flow("Attach")?;
                let watch_live = !self.owner_dead
                    && self.composition().io.forwarder_inventory().live_watches > 0;
                let expect_stream = watch_live && !attach_refused;
                let expect_refused = watch_live && attach_refused;
                // Guest actions are stimulus in the host driver. After the
                // host owner/process died there is no host control connection
                // to receive this locally produced reply; the guest attach
                // still occurs, but it must not write through the dead host
                // seam. While the owner lives, drive the real dial unchanged.
                if !self.owner_dead {
                    let session =
                        self.session.ok_or_else(|| not_playable("Attach", "no session"))?;
                    let reply = if attach_refused {
                        ControlMessage::Refused {
                            flow,
                            refusal: FlowRefusal::ApplicationRefused {
                                errno: KernelErrno::ECONNREFUSED,
                            },
                        }
                    } else {
                        ControlMessage::Paired { flow }
                    };
                    self.composition().guest_sends(session, reply);
                    self.settle();
                }
                if let Some(dial) = self.dial.take() {
                    let result = self
                        .rt
                        .block_on(dial)
                        .map_err(|error| not_playable("Attach", error.to_string()))?;
                    match (expect_stream, expect_refused, result) {
                        (true, false, Ok(stream)) => self.stream = Some(stream),
                        (
                            false,
                            true,
                            Err(GuestDialError::Refused { errno: KernelErrno::ECONNREFUSED }),
                        ) => {}
                        (false, false, Err(_fixed_error)) => {}
                        (true, false, Err(error)) => {
                            return Err(not_playable(
                                "Attach",
                                format!("the live successful handoff returned {error}"),
                            ));
                        }
                        (false, _, Ok(_stream)) => {
                            return Err(not_playable(
                                "Attach",
                                "a refused or late Paired handed out a stream",
                            ));
                        }
                        (false, true, Err(error)) => {
                            return Err(not_playable(
                                "Attach",
                                format!(
                                    "the live refused handoff returned {error}, not Refused(ECONNREFUSED)"
                                ),
                            ));
                        }
                        (true, true, _) => {
                            return Err(not_playable(
                                "Attach",
                                "a handoff cannot expect both a stream and refusal",
                            ));
                        }
                    }
                } else if expect_stream || expect_refused {
                    return Err(not_playable("Attach", "the live handoff has no dial future"));
                }
            }
            Mode::Stream | Mode::Dgram => {
                let cid = self.lease().cid;
                let _paired = self.composition().io.take_outbox(cid);
            }
        }
        if attach_refused {
            self.guest.phase = GuestPhase::Refused;
            self.guest_shut_all();
        } else {
            self.guest.phase = GuestPhase::Live;
            if self.guest.ps.contains(&'R') {
                self.guest.phase = GuestPhase::Ended;
                self.guest_shut_all();
            } else if self.guest.ps.contains(&'S') {
                self.guest_mark_clean();
            }
        }
        self.settle();
        Ok(())
    }

    /// `VCtl("hg")`: the host's next packet reaches `V_g` (EVMOD-END's table).
    fn vctl_hg(&mut self) -> Result {
        let packet = self
            .to_guest
            .pop_front()
            .ok_or_else(|| not_playable("VCtl", "no host packet in flight"))?;
        let flags = packet.flags();
        let new: BTreeSet<char> = flags.difference(&self.guest.ps).copied().collect();
        let sends = !self.guest.sent.contains(&'S');
        let class = if new.contains(&'S') && !new.contains(&'R') && !self.guest.ps.contains(&'R') {
            "clean"
        } else if new.contains(&'S') || (new.contains(&'R') && sends) {
            "abort"
        } else {
            "none"
        };
        self.guest.ps.extend(flags);
        if self.guest.phase == GuestPhase::Live {
            match class {
                "clean" => self.guest_mark_clean(),
                "abort" => {
                    self.guest.phase = GuestPhase::Ended;
                    self.guest_shut_all();
                }
                _ => {}
            }
        }
        self.settle();
        Ok(())
    }

    /// `VCtl("gh")`: the guest's next packet reaches `V_h`.
    fn vctl_gh(&mut self) -> Result {
        let packet = self
            .to_host
            .pop_front()
            .ok_or_else(|| not_playable("VCtl", "no guest packet in flight"))?;
        let pair_to_run = if matches!(packet, Packet::Close | Packet::Rst) {
            self.observed_pair("VCtl")?
        } else {
            None
        };
        let flow = self.flow("VCtl")?;
        let io = &self.composition().io;
        match packet {
            Packet::Shut => io.guest_half_closes(flow),
            Packet::Close => io.guest_closes(flow),
            Packet::Rst => io.guest_resets(flow),
        }
        if let Some((generation, pair)) = pair_to_run
            && !io.run_forwarder_once(pair, PairSide::A)
        {
            return Err(not_playable(
                "VCtl",
                format!(
                    "abort callback queued no run for attached pair {pair:?} in generation {generation}"
                ),
            ));
        }
        self.settle();
        Ok(())
    }

    fn hop0_gh(&mut self) -> Result {
        let flow = self.flow("Hop0")?;
        if self.mode == Mode::Dgram {
            let size = *self
                .gh_dgrams
                .front()
                .ok_or_else(|| not_playable("Hop0", "no guest datagram queued"))?;
            let payload = vec![b'd'; size];
            let frame = DatagramFrame::new(&payload)
                .map_err(|error| not_playable("Hop0", error.to_string()))?
                .encode();
            let header = frame.len() - size;
            let bytes = if self.gh_foff == 0 {
                &frame[..header]
            } else {
                &frame[header + self.gh_foff - 1..header + self.gh_foff]
            };
            self.composition().io.guest_sends(flow, bytes);
            if self.gh_foff == size {
                self.gh_dgrams.pop_front();
                self.gh_foff = 0;
            } else {
                self.gh_foff += 1;
            }
        } else {
            self.unit_gh = self.unit_gh.wrapping_add(1);
            self.composition().io.guest_sends(flow, &[self.unit_gh]);
        }
        self.settle();
        Ok(())
    }

    fn hop1_hg_at_guest(&mut self) {
        if self.mode == Mode::Dgram {
            let items = self
                .hg_dgrams
                .pop_front()
                .map_or(1, |size| u32::try_from(size + 1).unwrap_or(u32::MAX));
            self.guest.inflight = self.guest.inflight.saturating_sub(items);
        } else {
            self.guest.inflight = self.guest.inflight.saturating_sub(1);
        }
        self.settle();
    }

    fn hop_end0_gh(&mut self) {
        self.to_host.push_back(Packet::Shut);
        self.guest.sent.insert('S');
        self.guest.out_clean = true;
        self.guest_complete_if_done();
        self.settle();
    }

    fn hop_end1_hg(&mut self) {
        self.guest.in_clean = true;
        self.guest_complete_if_done();
        self.settle();
    }

    fn send_dg(&mut self, d: &str, sz: i64) -> Result {
        let size = usize::try_from(sz).map_err(|_| not_playable("SendDg", "a negative size"))?;
        if d == "gh" {
            // The guest's source queue holds one datagram (WU = 1); a full
            // queue drops (F8).
            if self.gh_dgrams.is_empty() {
                self.gh_dgrams.push_back(size);
            }
            self.settle();
            Ok(())
        } else {
            self.hg_dgrams.push_back(size);
            let payload = vec![b'h'; size];
            self.on_peer("SendDg", |socket| socket.peer_send(&payload))
        }
    }

    fn hop0_hg(&mut self) -> Result {
        self.guest.inflight += 1;
        self.run_h(PairSide::B, "Hop0")
    }

    fn hazard(name: &'static str) -> Result {
        Err(DriverError::HazardOnly(name).into())
    }

    // ----- projection -----

    fn project(&self) -> Result<PairStateH> {
        let c = self.composition();
        // conformance-lint: sim-arbiter-ok QPROJ-8 oldCtx/ctx: SimFlowForwarder's context table, Tier-3 pair P-70 § 3.2 and § 3.8
        let contexts = c.io.forwarder_contexts();
        // The device table also retains phase 1's released probe context.
        // `forwarder_generation` counts only contexts this serving owner
        // opened, so it supplies the exact suffix modeled by `ctx`/`oldCtx`.
        // Keep the phase row in the arbiter and validate its full complement:
        // a second prefix row or any retained phase-1 resource is a fixture
        // divergence, never something the projection silently filters away.
        let forwarder_generation = if self.owner_dead {
            self.dead_forwarder_generation.ok_or_else(|| {
                anyhow::anyhow!("the dead owner has no pre-exit forwarder generation snapshot")
            })?
        } else {
            self.rt.block_on(self.owner.flow_observation()).forwarder_generation
        };
        let serving_count = usize::try_from(forwarder_generation.get())?;
        let phase_count = contexts.len().checked_sub(serving_count).ok_or_else(|| {
            anyhow::anyhow!(
                "owner generation {serving_count} exceeds the device's {} context rows",
                contexts.len()
            )
        })?;
        if phase_count != 1 {
            anyhow::bail!(
                "the device has {phase_count} non-serving context rows; phase 1 opens exactly one"
            );
        }
        let phase = &contexts[0];
        if phase.open
            || phase.live_pairs != 0
            || phase.live_watches != 0
            || !phase.queued_ends.is_empty()
        {
            anyhow::bail!("phase 1's released probe context retained resources: {phase:?}");
        }
        let serving_contexts = &contexts[phase_count..];
        // The host owner's death releases its context (the seam's process
        // exit), so the table alone says whether the current context is open.
        let current_open = serving_contexts.last().is_some_and(|row| row.open);
        let old_ctx = match serving_contexts.len() {
            0 | 1 => "none",
            n if serving_contexts[n - 2].open => "open",
            _ => "released",
        };
        let flow = self.flow;
        // conformance-lint: sim-arbiter-ok QPROJ-8 tcpErr: D's sk_err in SimFlowForwarder's socket model, Tier-3 pair P-69 hazard-16 shapes
        let peer = flow
            .and_then(|flow| c.io.peer_of_opt(flow))
            // conformance-lint: sim-arbiter-ok QPROJ-8 tcpErr: retained clone of the same actual host-peer SimSocket selected by the pre-attach stimulus, Tier-3 P-69 reset classification and P-71 destination-reset-before-attach
            .or_else(|| self.host_peer.clone())
            .or_else(|| c.io.latest_peer_socket());
        let tcp_err = match (self.mode, peer.and_then(|socket| socket.sk_err())) {
            (Mode::Stream, Some(errno)) => errno_label(errno),
            _ => "0".to_owned(),
        };
        // conformance-lint: sim-arbiter-ok QPROJ-7 / QPROJ-8 vs(h): the host vsock half's shared SimSocket (vsock_half, D-5), Tier-3 pairs P-69, P-70 § 3.6 and P-71 h_lateshut
        let vsock = flow.and_then(|flow| c.io.vsock_half(flow));
        let vs = vsock.as_ref().map_or_else(VsH::none, |socket| VsH {
            ps: shutdown_flags(socket.received_shutdown()),
            sent: shutdown_flags(socket.sent_shutdown()),
            err: socket.sk_err().is_some(),
            cls: match socket.recorded_class() {
                Some(SourceEnd::Clean) => "clean",
                Some(SourceEnd::Abort) | None => "none",
            }
            .to_owned(),
            st: vsock_state_label(socket),
            sshut: socket.own_send_shut(),
        });
        let owner =
            if self.owner_dead { None } else { Some(self.project_owner(serving_contexts)?) };
        Ok(PairStateH {
            ctx: if current_open { "open" } else { "released" }.to_owned(),
            old_ctx: old_ctx.to_owned(),
            tcp_err,
            vs,
            hres: self.hres.to_owned(),
            strm_held: self.stream.is_some(),
            owner,
        })
    }

    /// The host owner's facts (O), with the forwarder facts (S) they are
    /// read against; only while the host owner lives.
    fn project_owner(&self, contexts: &[SimContextState]) -> Result<OwnerH> {
        let c = self.composition();
        let observed = self.rt.block_on(self.owner.flow_observation());
        // conformance-lint: sim-arbiter-ok QPROJ-8 the current context's inventory through SimFlowForwarder, Tier-3 pair V-EV-CTL and the module suite
        let inventory = c.io.forwarder_inventory();
        let current = observed.forwarder_generation;
        let generation = i64::try_from(current.get())? - 1;
        let attached = observed.forwarder_attached > 0;
        let att_ended = i64::try_from(observed.forwarder_attach_ended)?;
        let att_res = if att_ended > 0 {
            "halfEnded"
        } else if attached {
            "ok"
        } else {
            "none"
        };
        let cid = observed.cids.get(&self.lease().cid);
        let flow = self.flow;
        // The flow's entry in the owner's table, with the pair or watch it
        // names once attached.
        let (held, paired) = match self.mode {
            Mode::Stream => {
                let phase = cid.and_then(|cid| self.key.and_then(|key| cid.guest_flows.get(&key)));
                (
                    phase.is_some(),
                    phase.and_then(|phase| match phase {
                        GuestFlowPhase::Paired { pair, generation } => Some((*generation, *pair)),
                        _ => None,
                    }),
                )
            }
            Mode::Dgram => {
                let phase = cid.and_then(|cid| {
                    flow.and_then(|flow| cid.datagrams.get(&flow)).map(|assoc| assoc.phase)
                });
                (
                    phase.is_some(),
                    phase.and_then(|phase| match phase {
                        AssociationPhase::Paired { pair, generation } => Some((generation, pair)),
                        AssociationPhase::AwaitingPaired => None,
                    }),
                )
            }
            Mode::Watch => {
                let dial = cid.and_then(|cid| flow.and_then(|flow| cid.dials.get(&flow)));
                (dial.is_some(), self.ids.filter(|_| dial.is_some()))
            }
        };
        let guest_end_read = match self.mode {
            Mode::Watch => {
                cid.and_then(|cid| flow.and_then(|flow| cid.dials.get(&flow))).is_some_and(|dial| {
                    matches!(dial.phase, DialPhase::Paired { guest_end: Some(_), .. })
                })
            }
            Mode::Stream | Mode::Dgram => false,
        };
        // D-4: the flow's record in the end queue of the context that
        // attached it, released contexts included.
        let ids = paired.or(self.ids);
        let queued = ids.map_or(0, |(generation, id)| {
            usize::try_from(generation.get())
                .ok()
                .and_then(|g| g.checked_sub(1))
                .and_then(|row| contexts.get(row))
                .map_or(0, |row| row.queued_ends.iter().filter(|queued| **queued == id).count())
        });
        let live = held
            && queued == 0
            && paired.is_some_and(|(generation, id)| {
                generation == current
                    && contexts.last().is_some_and(|row| row.open)
                    && match self.mode {
                        // conformance-lint: sim-arbiter-ok QPROJ-8 SimFlowForwarder pair_state (a live pair), Tier-3 pair V-EV-PAIR and the module suite
                        Mode::Stream | Mode::Dgram => c.io.forwarder_pair_state(id).is_some(),
                        Mode::Watch => inventory.live_watches > 0 && !guest_end_read,
                    }
            });
        let ph = if att_ended > 0 {
            "refused"
        } else if live {
            "live"
        } else if held && paired.is_none() {
            "unattached"
        } else {
            "ended"
        };
        // D-11: the end record the owner consumed, read through its
        // `last_consumed_end` when its id is this flow's pair or watch.
        let consumed =
            observed.last_consumed_end.filter(|end| ids.is_some_and(|(_, id)| id == end.id));
        let rec = if queued > 0 {
            "queued"
        } else if consumed.is_some() {
            "read"
        } else {
            "none"
        };
        let end = consumed.map(|end| self.end_h(&end));
        let nrec = i64::try_from(queued)? + i64::from(rec == "read");
        let closed = self.mode != Mode::Watch
            && attached
            && !held
            && flow
                .and_then(|flow| c.io.vsock_half(flow))
                // conformance-lint: sim-arbiter-ok QPROJ-8 closed: the host vsock half sent {S, R} at the owner's close (vsock_half, D-5), Tier-3 pair P-71
                .is_some_and(|socket| socket.sent_shutdown() == Some(SimShutdown::Both));
        // byRel: the flow left the table and no record was produced for it —
        // the owner consumed none and the row at its generation holds none.
        let by_rel = self.mode != Mode::Watch
            && flow.is_some()
            && !held
            && consumed.is_none()
            && queued == 0;
        let live_cgen = if live {
            paired
                .map(|(generation, _)| i64::try_from(generation.get()))
                .transpose()?
                .map(|g| g - 1)
        } else {
            None
        };
        let strm_abort = self.mode == Mode::Watch
            && self.stream.is_some()
            && flow.is_some_and(|flow| cid.is_some_and(|cid| !cid.dials.contains_key(&flow)));
        Ok(OwnerH {
            generation,
            ph: ph.to_owned(),
            live_cgen,
            rec: rec.to_owned(),
            end,
            nrec,
            closed,
            by_rel,
            att_res: att_res.to_owned(),
            att_ended,
            strm_abort,
            unclassified: inventory.unclassified_eof > 0,
        })
    }
}

impl FwdPairDriver {
    /// A consumed end record's outcome in the model's vocabulary; `errno` is
    /// the `SocketError` errno of the TCP half D's reset (stream mode, side
    /// B; T-1), `none` otherwise.
    fn end_h(&self, end: &ForwardEnd) -> EndH {
        let (out, cause, errno) = match end.outcome {
            ForwardOutcome::Completed => ("completed", "none", "none".to_owned()),
            ForwardOutcome::GuestHalfClose => ("halfclose", "none", "none".to_owned()),
            ForwardOutcome::Aborted { cause } => match cause {
                AbortCause::PeerAbort { .. } => ("aborted", "PeerAbort", "none".to_owned()),
                AbortCause::SocketError { side, errno } => (
                    "aborted",
                    "SocketError",
                    if side == PairSide::B && self.mode == Mode::Stream {
                        errno_label(errno)
                    } else {
                        "none".to_owned()
                    },
                ),
                AbortCause::FrameInvalid { .. } => ("aborted", "FrameInvalid", "none".to_owned()),
                AbortCause::Detached => ("aborted", "Detached", "none".to_owned()),
                AbortCause::Unclassified { .. } => ("aborted", "Unclassified", "none".to_owned()),
            },
        };
        EndH { out: out.to_owned(), cause: cause.to_owned(), errno }
    }
}

/// `vs(h).st`: a vsock half's state in the model's vocabulary.
fn vsock_state_label(socket: &SimSocket) -> String {
    if socket.vsock_state() == SimVsockState::Closing { "closing" } else { "est" }.to_owned()
}

/// A vsock half's SHUTDOWN flags as the model's set (`{S}`, `{S, R}`).
fn shutdown_flags(flags: Option<SimShutdown>) -> BTreeSet<String> {
    match flags {
        None => BTreeSet::new(),
        Some(SimShutdown::Send) => BTreeSet::from(["S".to_owned()]),
        Some(SimShutdown::Both) => BTreeSet::from(["S".to_owned(), "R".to_owned()]),
    }
}

fn errno_label(errno: KernelErrno) -> String {
    match errno.get() {
        104 => "ECONNRESET".to_owned(),
        32 => "EPIPE".to_owned(),
        other => format!("errno {other}"),
    }
}

impl Driver for FwdPairDriver {
    type State = PairStateH;

    fn config() -> Config {
        Config { state: &[], nondet: &["pair_conf::fwd_pair::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            InitWith(m: String) => self.init_with(&m)?,
            Attach(s: String) => if s == "h" { self.attach_h()? } else { self.attach_g()? },
            Abort(s: String) => if s == "h" { self.abort_h()? } else { self.guest_abort(false) },
            GuestAbort(s: String) => {
                let _ = s;
                let cid = self.lease().cid;
                let _abort = self.composition().io.take_outbox(cid);
                self.guest_abort(self.mode != Mode::Watch);
            }
            CloseUnattached(s: String) => {
                let _ = s;
                self.guest_abort(true);
            }
            CtxRelease(s: String) => if s == "h" { self.ctx_release_h()? } else { self.guest_abort(true) },
            Reopen(s: String) => {
                let _ = s;
                self.reopen_h()?;
            }
            TakeEnd(s: String) => if s == "h" { self.take_end_h()? } else { self.settle() },
            OwnerClose(s: String) => if s == "h" {
                return Err(not_playable(
                    "OwnerClose",
                    "the host owner closes in the dispatch that reads the record (TakeEnd)",
                ));
            } else {
                self.guest_shut_all();
                self.settle();
            },
            Hop0(d: String) => if d == "hg" { self.hop0_hg()? } else { self.hop0_gh()? },
            Hop1(d: String) => if d == "gh" { self.run_h(PairSide::A, "Hop1")? } else { self.hop1_hg_at_guest() },
            HopEnd0(d: String) => if d == "hg" { self.run_h(PairSide::B, "HopEnd0")? } else { self.hop_end0_gh() },
            HopEnd1(d: String) => if d == "gh" { self.run_h(PairSide::A, "HopEnd1")? } else { self.hop_end1_hg() },
            VCtl(d: String) => if d == "gh" { self.vctl_gh()? } else { self.vctl_hg()? },
            FwdErr(s: String) => if s == "h" { self.fwd_err_h()? } else if self.guest.udp_err || self.guest.tcp_err || self.guest.phase == GuestPhase::Live {
                self.guest_abort(false);
            } else {
                self.settle();
            },
            Write(d: String) => if d == "hg" { self.write_hg()? } else { self.settle() },
            Fin(d: String) => if d == "hg" { self.fin_hg()? } else { self.settle() },
            Read(d: String) => if d == "gh" { self.read_gh()? } else { self.settle() },
            ReadEof(d: String) => if d == "gh" { self.read_eof_gh()? } else { self.settle() },
            ShutRd(d: String) => if d == "gh" {
                self.on_peer("ShutRd", SimSocket::peer_stop_reading)?;
            } else {
                self.settle();
            },
            TcpRst(s: String, after_fin: bool) => {
                let _ = after_fin;
                if s == "h" {
                    let pair_to_run = self.observed_pair("TcpRst")?;
                    self.peer_socket("TcpRst")?.peer_reset();
                    self.composition().io.caused();
                    if let Some((generation, pair)) = pair_to_run
                        && !self.composition().io.run_forwarder_once(pair, PairSide::B)
                    {
                        return Err(not_playable(
                            "TcpRst",
                            format!(
                                "reset callback queued no side-B run for attached pair {pair:?} in generation {generation}"
                            ),
                        ));
                    }
                    self.settle();
                } else {
                    self.guest.tcp_err = true;
                    self.guest_abort(false);
                }
            }
            HolderDrop => self.holder_drop(),
            VmmExit => self.vmm_exit(),
            TrailingRst(s: String) => {
                if s == "h" {
                    self.to_host.push_back(Packet::Rst);
                } else {
                    self.to_guest.push_back(Packet::Rst);
                }
                self.settle();
            }
            SendDg(d: String, sz: i64) => self.send_dg(&d, sz)?,
            DRead(d: String) => {
                let _ = d;
                self.settle();
            }
            UdpBlock(d: String) => if d == "gh" {
                self.on_peer("UdpBlock", SimSocket::peer_stop_reading)?;
            } else {
                self.settle();
            },
            UdpUnblock(d: String) => if d == "gh" {
                self.on_peer("UdpUnblock", SimSocket::peer_resume_reading)?;
            } else {
                self.settle();
            },
            UdpErr(s: String) => {
                if s == "h" {
                    let eperm = KernelErrno::new(1).map_err(|error| not_playable("UdpErr", error.to_string()))?;
                    self.peer_socket("UdpErr")?.set_sk_err(eperm);
                    self.udp_err_h = true;
                } else {
                    self.guest.udp_err = true;
                }
                self.settle();
            }
            DgHalfClose(s: String) => {
                if s == "h" {
                    let flow = self.flow("DgHalfClose")?;
                    let vsock = self
                        .composition()
                        .io
                        .vsock_half(flow)
                        .ok_or_else(|| not_playable("DgHalfClose", "no V_h"))?;
                    vsock.shutdown_send();
                    self.composition().io.caused();
                } else {
                    self.to_host.push_back(Packet::Shut);
                    self.guest.sent.insert('S');
                }
                self.settle();
            }
            HolderEnd(k: String) => self.holder_end(&k)?,
            DropRecord => return Self::hazard("DropRecord"),
            AttachRead => return Self::hazard("AttachRead"),
            AttachHook => return Self::hazard("AttachHook"),
            UnclassifiedEof => return Self::hazard("UnclassifiedEof"),
            WatchRecord => return Self::hazard("WatchRecord"),
        })
    }
}

// ----- named runs (`checks.toml`, `fwd_pair` on `pair_conf`) ---------------

/// Named run `Paired` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "Paired"
)]
fn quint_fwd_pair_paired() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `CleanBothWaysCompletes` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "CleanBothWaysCompletes"
)]
fn quint_fwd_pair_clean_both_ways_completes() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `TargetStallBlocksSenderThenResumes` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "TargetStallBlocksSenderThenResumes"
)]
fn quint_fwd_pair_target_stall_blocks_sender_then_resumes() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `EarlyBytesBeforeAttachDelivered` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "EarlyBytesBeforeAttachDelivered"
)]
fn quint_fwd_pair_early_bytes_before_attach_delivered() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `EarlyFinBeforeAttachCarriedOut` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "EarlyFinBeforeAttachCarriedOut"
)]
fn quint_fwd_pair_early_fin_before_attach_carried_out() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `GuestAppResetReachesRemoteAsReset` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "GuestAppResetReachesRemoteAsReset"
)]
fn quint_fwd_pair_guest_app_reset_reaches_remote_as_reset() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `RemoteResetReachesGuestAppAsReset` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "RemoteResetReachesGuestAppAsReset"
)]
fn quint_fwd_pair_remote_reset_reaches_guest_app_as_reset() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `HalfCloseThenPeerCloseIsClean` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "HalfCloseThenPeerCloseIsClean"
)]
fn quint_fwd_pair_half_close_then_peer_close_is_clean() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `PeerCloseWithDirectionOpenAborts` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "PeerCloseWithDirectionOpenAborts"
)]
fn quint_fwd_pair_peer_close_with_direction_open_aborts() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `ReadShutdownPeerStallsLikeSlowReader` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "ReadShutdownPeerStallsLikeSlowReader"
)]
fn quint_fwd_pair_read_shutdown_peer_stalls_like_slow_reader() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `OwnerAbortResetsBothFarEnds` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "OwnerAbortResetsBothFarEnds"
)]
fn quint_fwd_pair_owner_abort_resets_both_far_ends() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `ContextReleaseResetsEveryPair` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "ContextReleaseResetsEveryPair"
)]
fn quint_fwd_pair_context_release_resets_every_pair() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `VmmExitAbortsHostHalf` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "VmmExitAbortsHostHalf"
)]
fn quint_fwd_pair_vmm_exit_aborts_host_half() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `TrailingRstAfterCompletionIsInert` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "TrailingRstAfterCompletionIsInert"
)]
fn quint_fwd_pair_trailing_rst_after_completion_is_inert() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `WatchedStreamSeesGuestResetAsAbort` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "WatchedStreamSeesGuestResetAsAbort"
)]
fn quint_fwd_pair_watched_stream_sees_guest_reset_as_abort() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `DgramPaired` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "DgramPaired"
)]
fn quint_fwd_pair_dgram_paired() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `DatagramsInOrderWithEmpty` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "DatagramsInOrderWithEmpty"
)]
fn quint_fwd_pair_datagrams_in_order_with_empty() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `PartialFrameResumedNotDuplicated` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "PartialFrameResumedNotDuplicated"
)]
fn quint_fwd_pair_partial_frame_resumed_not_duplicated() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `StalledUdpTargetDropsAtSource` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "StalledUdpTargetDropsAtSource"
)]
fn quint_fwd_pair_stalled_udp_target_drops_at_source() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `HostHalfCloseThenAbortBeforeGuestAttachIsAbort` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "HostHalfCloseThenAbortBeforeGuestAttachIsAbort"
)]
fn quint_fwd_pair_host_half_close_then_abort_before_guest_attach_is_abort() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `GuestCloseBeforeHostAttachAborts` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "GuestCloseBeforeHostAttachAborts"
)]
fn quint_fwd_pair_guest_close_before_host_attach_aborts() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `RemoteFinThenResetAbortsPair` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "RemoteFinThenResetAbortsPair"
)]
fn quint_fwd_pair_remote_fin_then_reset_aborts_pair() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `ForwarderRecoveryAbortsThenReopens` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "ForwarderRecoveryAbortsThenReopens"
)]
fn quint_fwd_pair_forwarder_recovery_aborts_then_reopens() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `ReleasedWatchWithNoRecordReadsAsAbort` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "ReleasedWatchWithNoRecordReadsAsAbort"
)]
fn quint_fwd_pair_released_watch_with_no_record_reads_as_abort() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `VmmExitWithWatchRecordInFlightResolvesFromRecord` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "VmmExitWithWatchRecordInFlightResolvesFromRecord"
)]
fn quint_fwd_pair_vmm_exit_with_watch_record_in_flight_resolves_from_record() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `TruncatedFrameAtPeerEndAbortsFrameInvalid` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "TruncatedFrameAtPeerEndAbortsFrameInvalid"
)]
fn quint_fwd_pair_truncated_frame_at_peer_end_aborts_frame_invalid() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `GuestAppFinThenResetAbortsPairWithEpipe` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "GuestAppFinThenResetAbortsPairWithEpipe"
)]
fn quint_fwd_pair_guest_app_fin_then_reset_aborts_pair_with_epipe() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `EarlyVsockHalfCloseBeforeGuestAttachCompletes` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "EarlyVsockHalfCloseBeforeGuestAttachCompletes"
)]
fn quint_fwd_pair_early_vsock_half_close_before_guest_attach_completes() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `AppWriteAndFinBeforePairedAttachesInCloseWaitAndCompletes` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "AppWriteAndFinBeforePairedAttachesInCloseWaitAndCompletes"
)]
fn quint_fwd_pair_app_write_and_fin_before_paired_attaches_in_close_wait_and_completes()
-> impl Driver {
    FwdPairDriver::new()
}

/// Named run `IntakeChildResetBeforePairedIsRefusedAndFlowAborts` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "IntakeChildResetBeforePairedIsRefusedAndFlowAborts"
)]
fn quint_fwd_pair_intake_child_reset_before_paired_is_refused_and_flow_aborts() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `DestinationResetBeforeHostAttachIsRefusedAndFlowAborts` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "DestinationResetBeforeHostAttachIsRefusedAndFlowAborts"
)]
fn quint_fwd_pair_destination_reset_before_host_attach_is_refused_and_flow_aborts() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `GuestCloseWithEmptyQueueBeforeHostAttachIsRefusedAndFlowAborts` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "GuestCloseWithEmptyQueueBeforeHostAttachIsRefusedAndFlowAborts"
)]
fn quint_fwd_pair_guest_close_with_empty_queue_before_host_attach_is_refused_and_flow_aborts()
-> impl Driver {
    FwdPairDriver::new()
}

/// Named run `OwnerAbortOfWatchLeavesVhAndStreamReportsAborted` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "OwnerAbortOfWatchLeavesVhAndStreamReportsAborted"
)]
fn quint_fwd_pair_owner_abort_of_watch_leaves_vh_and_stream_reports_aborted() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `OwnerAbortSendsGuestAbortThenHolderDropIsInert` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "OwnerAbortSendsGuestAbortThenHolderDropIsInert"
)]
fn quint_fwd_pair_owner_abort_sends_guest_abort_then_holder_drop_is_inert() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `ContextReleaseShutsWatchedVhAndHolderReadsNotConnected` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "ContextReleaseShutsWatchedVhAndHolderReadsNotConnected"
)]
fn quint_fwd_pair_context_release_shuts_watched_vh_and_holder_reads_not_connected() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `DroppedStreamAbortsWatchThenGuestSeesAbort` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "DroppedStreamAbortsWatchThenGuestSeesAbort"
)]
fn quint_fwd_pair_dropped_stream_aborts_watch_then_guest_sees_abort() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `PeerCloseRacingOwnHalfCloseIsClean` (`pair_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "PeerCloseRacingOwnHalfCloseIsClean"
)]
fn quint_fwd_pair_peer_close_racing_own_half_close_is_clean() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `HostOwnerDeathIsFinal` (`pair_conf`), host side: a release
/// that finds the host pair live is the host owner's death (D-5).
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "HostOwnerDeathIsFinal"
)]
fn quint_fwd_pair_host_owner_death_is_final() -> impl Driver {
    FwdPairDriver::new()
}

/// Named run `RecoveryEndsHeldStreamWhoseWatchEnded` (`pair_conf`), host
/// side: the `Forwarder` recovery ends a held stream whose watch already
/// ended (U-R1).
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt",
    main = "pair_conf",
    max_samples = 1,
    test = "RecoveryEndsHeldStreamWhoseWatchEnded"
)]
fn quint_fwd_pair_recovery_ends_held_stream_whose_watch_ended() -> impl Driver {
    FwdPairDriver::new()
}

/// Randomized `pair_conf` traces; the seed is printed on failure.
#[quint_run(spec = "../../specs/quint/guest-flow-owner/fwd_pair.qnt", main = "pair_conf")]
fn pair_conf_randomized_traces() -> impl Driver {
    FwdPairDriver::new()
}
