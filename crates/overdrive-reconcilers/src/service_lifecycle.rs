//! `ServiceFailureReason` + `ProbeWitness` + `ServiceLifecycleState`
//! + `ServiceLifecycleView` — Service-kind reconciler types.
//!
//! Per ADR-0055 §4: `ServiceFailureReason` is a single per-kind
//! `#[non_exhaustive]` enum (NOT per-condition sub-enums; that would
//! fragment the operator-facing "why did my Service fail?" surface).
//! Additive variants per ADR-0037 §5.
//!
//! Per ADR-0055 §3 / DDD-5: `ServiceLifecycleView` carries
//! **inputs only** (counters, sets) — the `Stable` predicate, the
//! readiness `healthy` gate, the liveness restart-trigger
//! predicate, the deadline computations — ALL recomputed every
//! tick against the live spec policy per
//! `.claude/rules/development.md` § "Persist inputs, not derived
//! state".
//!
//! `ServiceFailureReason` and `ProbeWitness` live in
//! [`overdrive_core::transition_reason`] (so they can be carried inside
//! [`overdrive_core::TerminalCondition::ServiceFailed`] / `::Stable` without
//! inducing a module-dependency cycle) and are re-exported here
//! for ergonomics — callers under `service_lifecycle::*` get the
//! same surface they had before the cycle-breaking relocation.

#![allow(dead_code)]
#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    clippy::module_name_repetitions,
    clippy::struct_field_names,
    reason = "DISTILL RED scaffold; behavioural expansion in subsequent slices"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use overdrive_core::dataplane::fingerprint::fingerprint;
use overdrive_core::id::{AllocationId, ServiceId, ServiceVip, SpiffeId};
use overdrive_core::observation::{ProbeIdx, ProbeStatus};
use overdrive_core::traits::observation_store::{AllocState, ObservationRowKind};

// Re-exports — see file-header docstring for the cycle-breaking
// rationale.
pub use overdrive_core::transition_reason::{ProbeWitness, ServiceFailureReason};

/// Per-alloc fact bundle the reconciler consults when deciding
/// `Stable` / `Failed` / no-op for a single Service-kind allocation.
///
/// Sourced by the runtime's hydrate-actual / hydrate-desired pass:
/// `state` + `started_at` + `exit_code` come from the
/// alloc-status row; `latest_startup_probe` is the LWW projection
/// of the per-`(alloc, probe_idx)` `ProbeResultRow`s for the
/// startup role.
///
/// `max_attempts` + `startup_deadline` + `mechanic_summary` come
/// from the live `ServiceSpec` (intent side) — re-evaluated every
/// tick per `.claude/rules/development.md` § "Persist inputs, not
/// derived state".
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "hydrate-boundary fact-bundle projection (one row's observed + spec-derived inputs), \
              not a domain entity — Object Calisthenics applies to the hexagonal core, not to \
              the runtime's per-alloc observation projection. Each bool names an independent \
              observed/spec fact (inferred / startup_probes_empty / has_readiness_probe / \
              has_liveness_probe); collapsing them into enums would obscure the projection."
)]
pub struct ServiceAllocFact {
    /// Allocation identifier.
    pub alloc_id: AllocationId,
    /// Lifecycle state observed on the alloc-status row.
    pub state: AllocState,
    /// Wall-clock at which the alloc transitioned Pending → Running,
    /// as observed by the owning node via the injected
    /// [`overdrive_core::traits::clock::Clock`] port. Sourced verbatim from
    /// the alloc-status row's `started_at` field (no translation;
    /// just projection).
    ///
    /// # Semantics
    ///
    /// - `None`: the alloc has not been observed Running yet
    ///   (Pending only, or driver-rejected start). The branches
    ///   that need a started-at timestamp (Stable, EarlyExit-elapsed,
    ///   StartupProbeFailed-elapsed) handle `None` explicitly per
    ///   the per-branch contract:
    ///   - Stable / opt-out Stable: `unreachable!()` — hydrate
    ///     invariant says a `Running` alloc carries
    ///     `Some(started_at)`.
    ///   - EarlyExit / StartupProbeFailed: skip the branch — the
    ///     alloc never reached Running, so the elapsed-vs-deadline
    ///     classification doesn't apply. The row's typed `terminal`
    ///     flows through other projections (e.g., Custom → Other).
    /// - `Some(ts)`: the alloc reached Running at wall-clock `ts`;
    ///   used by EarlyExit's `elapsed < startup_deadline` gate and
    ///   Stable's `settled_in_ms = tick.now_unix - ts` arithmetic.
    ///
    /// Per `.claude/rules/development.md` § "Persist inputs, not
    /// derived state": this is INPUT, not derived. `elapsed_ms` and
    /// `settled_in_ms` are recomputed at reconcile time.
    ///
    /// Per `.claude/rules/development.md` § "Distinct failure modes
    /// get distinct error variants": consumers MUST match on
    /// `Some(ts)` explicitly. Do NOT collapse with
    /// `unwrap_or(Duration::ZERO)` — the `None` and `Some` cases
    /// mean different things.
    pub started_at: Option<UnixInstant>,
    /// Exit code observed on Failed transition. `None` for Running
    /// / Pending allocs.
    pub exit_code: Option<i32>,
    /// Latest-observed startup probe outcome at index 0. `None` if
    /// no probe result has yet been written for this alloc.
    pub latest_startup_probe: Option<ProbeStatus>,
    /// Instant of the same latest Startup/index-0 observation as
    /// [`Self::latest_startup_probe`]. This transient hydration input identifies
    /// one durable LWW result so reconciliation counts it once rather than once
    /// per tick. Hydration converts the observation row's persisted epoch
    /// milliseconds at its adapter boundary.
    pub latest_startup_probe_observed_at: Option<UnixInstant>,
    /// Operator-spec-declared maximum number of startup probe
    /// attempts before `StartupProbeFailed` fires. Default per
    /// ADR-0057 §2 = 30.
    pub max_attempts: u32,
    /// Operator-spec-declared startup deadline window. Default per
    /// ADR-0057 §2 = 60s.
    pub startup_deadline: Duration,
    /// Operator-facing mechanic summary for the witnessing probe
    /// (e.g. `"tcp 0.0.0.0:8080"`). Reconciler composes
    /// `ProbeWitness.mechanic_summary` from this field at the
    /// deciding tick.
    pub mechanic_summary: String,
    /// `true` IFF the startup probe was inferred by the platform
    /// per ADR-0058 default-TCP-startup rule. Surfaces on
    /// `ProbeWitness.inferred`.
    pub inferred: bool,
    /// `true` IFF the operator's `ServiceSpec.startup_probes` is the
    /// empty array (`[[health_check.startup]] = []`) — the
    /// deliberate first-Running-IS-Stable opt-out per ADR-0058 §4 /
    /// ADR-0059 Q5. The reconciler's pre-Stable opt-out branch
    /// fires on this flag + `state == Running`, emitting `Stable`
    /// immediately with `mechanic_summary == "none (opted out)"`.
    pub startup_probes_empty: bool,

    // ---- Step 03-01 / Slice 04 — readiness facts ----
    /// Latest-observed readiness probe outcome at index 0. `None`
    /// when no readiness `ProbeResultRow` has yet been written for
    /// this alloc (the avoid-inverse-race initial state per Slice 04
    /// § Initial state: `Backend.healthy = false` until first Pass).
    ///
    /// Per `.claude/rules/development.md` § "Persist inputs, not
    /// derived state": this is the OBSERVED INPUT; `Backend.healthy`
    /// is RECOMPUTED every tick from this status + the live
    /// `success_threshold` + the consecutive-Pass counter in the
    /// View. It is never a cached `healthy: bool`.
    pub latest_readiness_probe: Option<ProbeStatus>,
    /// `true` IFF this alloc declares at least one readiness probe.
    /// `false` → the backend is unconditionally `healthy = true`
    /// post-Stable (the backward-compat no-readiness default per
    /// S-SHCP-RECON-08b). Sourced from `ServiceSpec.readiness_probes`
    /// non-empty (intent side) — re-evaluated every tick.
    pub has_readiness_probe: bool,
    /// Operator-spec-declared readiness `success_threshold` per
    /// ADR-0055 §6 / ADR-0057 §2 / DDD-8. Default 1 (one consecutive
    /// Pass flips `healthy = true`); configurable upward. Sourced
    /// from the live `ServiceSpec` — re-evaluated every tick, never
    /// persisted.
    pub readiness_success_threshold: u32,
    /// SPIFFE identity of this alloc as a dataplane backend. Used to
    /// construct the [`overdrive_core::traits::dataplane::Backend`] this alloc
    /// contributes to the service's backend set.
    pub backend_spiffe: SpiffeId,
    /// IPv4 address this alloc serves on as a dataplane backend. The
    /// listener-specific port is supplied by the enclosing dataplane
    /// identity when the complete backend row is composed.
    pub backend_ip: std::net::Ipv4Addr,

    // ---- Step 03-02 / Slice 05 — liveness facts ----
    /// Latest-observed liveness probe outcome at index 0. `None` when
    /// no liveness `ProbeResultRow` has yet been written for this alloc
    /// (no liveness observation this tick — neither a failure nor a
    /// recovery; the consecutive-failure counter is left untouched).
    ///
    /// Per `.claude/rules/development.md` § "Persist inputs, not
    /// derived state": this is the OBSERVED INPUT. The restart-trigger
    /// predicate is RECOMPUTED every tick from this status + the live
    /// `liveness_failure_threshold` + the consecutive-failure counter
    /// in the View. It is never a cached `should_restart: bool`.
    pub latest_liveness_probe: Option<ProbeStatus>,
    /// `true` IFF this alloc declares at least one liveness probe.
    /// `false` → the liveness branch is a no-op for this alloc (no
    /// liveness gate → never restart on liveness). Sourced from
    /// `ServiceSpec.liveness_probes` non-empty (intent side) —
    /// re-evaluated every tick.
    pub has_liveness_probe: bool,
    /// Operator-spec-declared liveness `failure_threshold` per
    /// ADR-0057 §2 / DDD-14. Default 3 (three consecutive Fails on a
    /// Running alloc trigger a liveness TERMINATE); configurable.
    /// Sourced from the live `ServiceSpec` — re-evaluated every tick,
    /// never persisted.
    ///
    /// ADR-0087 D2/D7: the fact no longer carries `restart_count` or
    /// `restart_spec`. `ServiceLifecycle` is a liveness DETECTOR — it
    /// reads no restart budget and replays no spec; `WorkloadLifecycle`
    /// (the sole restart authority) owns the budget and rebuilds the
    /// restart spec from live intent.
    pub liveness_failure_threshold: u32,
}

/// `ServiceLifecycleState` — typed projection of intent +
/// observation for the Service reconciler per ADR-0055 §2 +
/// ADR-0021/0036.
///
/// `desired` is sourced from `ServiceSpec` (intent). `actual` is
/// sourced from `alloc_status` rows + `probe_result` rows per
/// alloc.
///
/// Per ADR-0021 the same `State` type is used for both `desired`
/// and `actual` arguments — the runtime constructs both projections
/// from the same hydration pass. `tick.now_unix` provides the
/// reference wall-clock for deadline arithmetic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceLifecycleState {
    /// Per-alloc fact bundle, keyed by `AllocationId` to give the
    /// reconciler a deterministic iteration order. Empty for the
    /// `desired == actual == empty` no-alloc case (e.g. a freshly
    /// submitted Service before any allocation has been scheduled).
    pub allocs: BTreeMap<AllocationId, ServiceAllocFact>,

    /// Current listener identities keyed by their stable ServiceId. These
    /// are transient hydration inputs, not persisted reconciler state.
    pub service_dataplane: BTreeMap<ServiceId, ServiceDataplaneIdentity>,

    /// Complete backend rows observed for the current listener identities.
    /// The projection compares against these rows, excluding only their
    /// LWW stamp, so a dropped write is repaired on a later tick.
    pub observed_backend_rows: BTreeMap<ServiceId, ServiceBackendRow>,
}

/// Service-level dataplane identity for the readiness branch's
/// `ServiceBackendRow` composition. Separated from the per-alloc
/// [`ServiceAllocFact`] because it is one-per-Service, not
/// one-per-alloc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDataplaneIdentity {
    /// Virtual IP the service's backends serve behind.
    pub vip: ServiceVip,
    /// Listener port for this ServiceId.
    pub port: NonZeroU16,
    /// Listener transport protocol for this ServiceId.
    pub protocol: Proto,
    /// Owner-writer node id stamped on the LWW `ServiceBackendRow`.
    /// Sourced from the local node identity (the runtime composes it
    /// at hydrate time from the local node identity.
    pub writer: NodeId,
}

/// `ServiceLifecycleView` — runtime-persisted typed memory per
/// ADR-0055 §3 / DDD-5.
///
/// CARRIES INPUTS ONLY. The `Stable` predicate, readiness
/// `healthy` gate, liveness restart trigger, and deadline
/// computations are ALL recomputed every tick against the live
/// spec policy. Per `.claude/rules/development.md` § "Persist
/// inputs, not derived state" — a `is_stable: bool` field on this
/// view would be a violation.
///
/// Per `.claude/rules/development.md` § "Ordered-collection
/// choice": all maps/sets use `BTreeMap`/`BTreeSet`, NOT
/// `HashMap`/`HashSet` — iteration order is observed by DST
/// invariants AND by the LWW write ordering at the persistence
/// boundary.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceLifecycleView {
    /// Per-alloc count of consecutive startup-probe attempts that
    /// have not yet yielded a Pass.
    pub startup_attempts_per_alloc: BTreeMap<AllocationId, u32>,

    /// Per-`(alloc, probe_idx)` consecutive-failure counter for
    /// liveness probes. Used to gate `RestartAllocation` per
    /// US-05; reset to 0 on the first Pass per the recovery rule.
    pub liveness_consecutive_failures: BTreeMap<(AllocationId, ProbeIdx), u32>,

    /// Per-`(alloc, probe_idx)` consecutive-Pass counter for
    /// readiness probes. Gates `Backend.healthy` per ADR-0055 §6
    /// + P2-Q8: requires `success_threshold` consecutive Pass
    ///   observations before flipping `healthy = true`.
    pub readiness_consecutive_successes: BTreeMap<(AllocationId, ProbeIdx), u32>,

    /// Per-alloc set of allocs that have already had their Stable
    /// terminal condition announced. Used to dedup per-tick
    /// re-emission of Stable (per DDD-6: encoded as `BTreeSet`,
    /// NOT a flag on `TerminalCondition`, per ADR-0037 §5 layering).
    pub stable_announced: BTreeSet<AllocationId>,

    /// Per-alloc wall-clock at which the most recent
    /// startup-probe Fail was observed. Used to compute the
    /// `startup_deadline` deadline at read time (not persisted —
    /// the deadline IS derived state per the rule). Stored as
    /// UNIX-epoch milliseconds.
    pub startup_last_fail_seen_at: BTreeMap<AllocationId, u64>,

    /// GAP-9 — per-alloc set of allocs the reconciler has OBSERVED in
    /// a non-terminal state (i.e. it has begun watching the alloc's
    /// startup window but has not yet announced a terminal verdict).
    ///
    /// This is the load-bearing input for the runtime's
    /// `view_has_backoff_pending` self-re-enqueue predicate (Shape B
    /// of GAP-9): during the active startup window the reconciler
    /// emits ZERO actions (no Pass yet, not failed, deadline not
    /// elapsed), so the §18 *action-emitted* re-enqueue signal is
    /// absent and the broker would drain empty after the FIRST tick,
    /// leaving the reconciler never re-ticked. Recording the
    /// observed-alloc membership lets the predicate keep the
    /// reconciler alive across cadences until it observes the
    /// `ProbeRunner`'s Pass row (→ Stable) or a terminal.
    ///
    /// Per `.claude/rules/development.md` § "Persist inputs, not
    /// derived state": this records an OBSERVED FACT ("the reconciler
    /// is watching alloc X"), not a derived "needs re-enqueue now"
    /// boolean — the predicate recomputes that from the set
    /// difference against the two terminal sets every read.
    pub observed: BTreeSet<AllocationId>,

    /// GAP-9 — per-alloc set of allocs that reached a NON-Stable
    /// terminal verdict (`EarlyExit` / `StartupProbeFailed`). The
    /// Stable terminal continues to use [`Self::stable_announced`];
    /// this set is its non-Stable sibling.
    ///
    /// Two jointly load-bearing roles:
    ///
    /// 1. **Dedup** — without it the `EarlyExit` / `StartupProbeFailed`
    ///    branches re-emit their terminal `FinalizeFailed` action on
    ///    EVERY subsequent tick (a latent re-emission bug independent
    ///    of GAP-9), which would also keep the §18 action-emitted
    ///    re-enqueue alive forever — a busy-loop on a dead alloc.
    /// 2. **Predicate falseness at terminal** — the
    ///    `view_has_backoff_pending` predicate subtracts BOTH terminal
    ///    sets from [`Self::observed`]; once an alloc lands here the
    ///    predicate returns false for it, so a terminal-failed alloc
    ///    stops the runtime re-enqueue (no spinning reconciler).
    ///
    /// Per the same persist-inputs rule: records the observed fact
    /// "this alloc reached a non-Stable terminal," never a derived
    /// flag.
    ///
    /// Also covers pre-Running-Failed allocs (`state == Failed`,
    /// `started_at == None`) the reconciler acknowledges-but-does-not-
    /// classify: it emits no terminal action for them, but still
    /// records membership here so the Shape B predicate flips false
    /// once such a dead alloc is archived (otherwise its stale
    /// `observed` entry would spin the runtime forever).
    pub terminal_announced: BTreeSet<AllocationId>,
}

impl ServiceLifecycleView {
    /// GAP-9 Shape B predicate — does any observed alloc remain
    /// mid-startup-window (observed but not yet terminal)?
    ///
    /// An alloc is mid-startup-window iff the reconciler has recorded
    /// it in [`Self::observed`] AND it has not landed in EITHER
    /// terminal set ([`Self::stable_announced`] or
    /// [`Self::terminal_announced`]). The runtime's
    /// `view_has_backoff_pending` arm delegates here so the
    /// busy-loop-avoidance contract (true during the window, false the
    /// instant ANY terminal is reached) is pinned by a unit-testable
    /// pure predicate co-located with the view it reasons over.
    #[must_use]
    pub fn has_alloc_mid_startup_window(&self) -> bool {
        self.observed.iter().any(|alloc| {
            !self.stable_announced.contains(alloc) && !self.terminal_announced.contains(alloc)
        })
    }
}

/// Default startup deadline used by the reconciler when computing
/// the cut-off for `StartupProbeFailed` emission. Per ADR-0057 §2:
/// `max_attempts × interval_seconds` = 30 × 2s = 60s.
///
/// Recomputed per spec per tick — this constant is the default
/// applied when the spec omits explicit values. Per the rule, NOT
/// persisted.
pub const DEFAULT_STARTUP_DEADLINE: Duration = Duration::from_secs(60);

// ===== ServiceLifecycleReconciler =====
//
// Pure-sync reconciler per ADR-0035 / ADR-0055. Lives in
// `overdrive-core` (NOT `overdrive-control-plane`) because the
// `Reconciler` trait + `Action` / `TickContext` / `ReconcilerName`
// types are all defined in `overdrive-core::reconcilers`; co-locating
// the impl keeps the dispatch surface (`AnyReconciler`,
// `AnyState`, `AnyReconcilerView`) in one place without forcing a
// cyclic `control-plane → core → control-plane` dependency.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;

use overdrive_core::aggregate::probe_descriptor::ProbeMechanic;
use overdrive_core::aggregate::{IntentKey, ServiceV2, WorkloadIntent};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{ContentHash, CorrelationKey, NodeId, WorkloadId};
use overdrive_core::observation::{ProbeResultRow, ProbeRole};
use overdrive_core::reconcilers::{
    Action, HydrateError, HydrationContext, Reconciler, ReconcilerName, TargetResource, TickContext,
};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{LogicalTimestamp, ServiceBackendRow};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};
use overdrive_core::wall_clock::UnixInstant;

/// Service-kind lifecycle reconciler per ADR-0055.
///
/// Pure-sync `reconcile(desired, actual, view, tick) → (Vec<Action>,
/// View)` per `.claude/rules/development.md` § "Reconciler I/O" —
/// no `.await`, no port dependencies, no wall-clock outside
/// `tick.now_unix`.
///
/// The reconcile body covers Slice 01 branches (Stable, EarlyExit,
/// StartupProbeFailed). Slice 04 (readiness → Backend.healthy) and
/// Slice 05 (liveness → restart) extend the body in follow-up
/// slices.
#[derive(Debug, Clone)]
pub struct ServiceLifecycleReconciler {
    name: ReconcilerName,
}

impl Default for ServiceLifecycleReconciler {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceLifecycleReconciler {
    /// Construct a new reconciler with the canonical
    /// `service-lifecycle` name.
    ///
    /// # Panics
    /// Panics if `Self::NAME` fails `ReconcilerName::new`'s
    /// `^[a-z][a-z0-9-]{0,62}$` validation — this is a logic error
    /// caught at construction time, NOT a runtime failure path.
    #[must_use]
    pub fn new() -> Self {
        let name = ReconcilerName::new(<Self as Reconciler>::NAME).unwrap_or_else(|_| {
            unreachable!(
                "ServiceLifecycleReconciler::NAME = {:?} is a static literal that satisfies \
                 ReconcilerName's ^[a-z][a-z0-9-]{{0,62}}$ validator by construction",
                <Self as Reconciler>::NAME
            )
        });
        Self { name }
    }
}

#[async_trait::async_trait]
impl Reconciler for ServiceLifecycleReconciler {
    const NAME: &'static str = "service-lifecycle";
    type State = ServiceLifecycleState;
    type View = ServiceLifecycleView;

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    /// Row-backed: `service-lifecycle` converges `Stable` /
    /// `StartupProbeFailed` / `EarlyExit` terminal conditions against the
    /// running `AllocStatusRow` set, so an accepted `alloc_status` transition
    /// (a Running → Failed is exactly an `EarlyExit` witness for a Service
    /// alloc) must wake it (ADR-0084 §5 single-cut migration — the declarative
    /// replacement for the deleted `exit_observer` producer-push). It authors
    /// no `alloc_status` rows (it writes `healthy` on `service_backends`), so
    /// the interest is loop-free (ADR-0079).
    fn interests(&self) -> &'static [ObservationRowKind] {
        &[ObservationRowKind::AllocStatus]
    }

    fn reconcile(
        &self,
        _desired: &Self::State,
        actual: &Self::State,
        view: &Self::View,
        tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        let mut actions: Vec<Action> = Vec::new();
        let mut next_view = view.clone();
        let mut stable_this_tick: BTreeSet<AllocationId> = BTreeSet::new();
        let mut startup_failed_this_tick: BTreeSet<AllocationId> = BTreeSet::new();

        for (alloc_id, fact) in &actual.allocs {
            if next_view.stable_announced.contains(alloc_id) {
                // S-SHCP-RECON-02: dedup — Stable already announced
                // for this alloc; emit nothing further. Falls
                // through to no-action.
                continue;
            }

            // GAP-9 dedup — a non-Stable terminal verdict (EarlyExit /
            // StartupProbeFailed) was already announced for this alloc.
            // Without this guard those two branches re-emit their
            // terminal `FinalizeFailed` on EVERY tick (latent
            // re-emission bug) AND keep the runtime's §18
            // action-emitted re-enqueue alive forever — a busy-loop on
            // a dead alloc. Mirrors the `stable_announced` dedup above.
            if next_view.terminal_announced.contains(alloc_id) {
                continue;
            }

            // GAP-9 Shape B — record that the reconciler is watching
            // this alloc's startup window. This is the load-bearing
            // input for `ServiceLifecycleView::has_alloc_mid_startup_window`
            // (consulted by the runtime's `view_has_backoff_pending`
            // self-re-enqueue gate). During the active window the
            // branches below emit no action, so without this membership
            // the broker drains empty after the first tick and the
            // reconciler is never re-ticked (the GAP-9 defect). The
            // alloc is removed from the "still mid-flight" set the
            // instant it lands in either terminal set (the predicate
            // subtracts both), so a terminal alloc does NOT keep the
            // runtime spinning.
            next_view.observed.insert(alloc_id.clone());

            // GAP-10 — maintain the consecutive-startup-probe-fail
            // counter that the StartupProbeFailed gate below reads (it was
            // read-but-never-written, making the terminal unreachable and
            // Shape B a failure-path busy-loop). See `update_startup_attempts`
            // for the ADR-0057 §2 semantics; the terminal CONDITION is
            // unchanged — only the `attempts` INPUT now moves.
            update_startup_attempts(
                &mut next_view.startup_attempts_per_alloc,
                &mut next_view.startup_last_fail_seen_at,
                alloc_id,
                fact.latest_startup_probe.as_ref(),
                fact.latest_startup_probe_observed_at,
            );

            // Branch (a'): Empty-probes opt-out — operator declared
            // `[[health_check.startup]] = []` per ADR-0058 §4 /
            // ADR-0059 Q5. Operator's deliberate first-Running-IS-Stable
            // semantics. MUST precede branch (a) so the AND-of-all-
            // probes Pass branch never fires for opt-out specs (which
            // would otherwise hang the stream until the cap timer).
            //
            // `started_at == None` here is a hydrate invariant violation
            // (the alloc IS Running, so hydrate must have copied
            // `Some(ts)` from the row). The `unreachable!()` is the
            // structural defense per `.claude/rules/development.md`
            // § "Logically unreachable None / Err — use `unreachable!()`".
            if fact.startup_probes_empty && fact.state == AllocState::Running {
                let started = fact.started_at.unwrap_or_else(|| {
                    unreachable!("hydrate invariant: AllocStatusRow with state==Running carries Some(started_at)")
                });
                let settled_in_ms = settled_in_ms_from(tick.now_unix, started);
                let witness = ProbeWitness {
                    probe_idx: 0,
                    role: "startup".to_string(),
                    mechanic_summary: "none (opted out)".to_string(),
                    inferred: false,
                };
                actions.push(Action::FinalizeFailed {
                    alloc_id: alloc_id.clone(),
                    terminal: Some(TerminalCondition::Stable { settled_in_ms, witness }),
                });
                next_view.stable_announced.insert(alloc_id.clone());
                stable_this_tick.insert(alloc_id.clone());
                continue;
            }

            // Branch (a): Stable — Running + any startup probe Pass.
            // Same hydrate invariant as branch (a'): a Running alloc
            // carries `Some(started_at)`.
            if fact.state == AllocState::Running
                && matches!(fact.latest_startup_probe, Some(ProbeStatus::Pass))
            {
                let started = fact.started_at.unwrap_or_else(|| {
                    unreachable!("hydrate invariant: AllocStatusRow with state==Running carries Some(started_at)")
                });
                let settled_in_ms = settled_in_ms_from(tick.now_unix, started);
                let witness = ProbeWitness {
                    probe_idx: 0,
                    role: "startup".to_string(),
                    mechanic_summary: fact.mechanic_summary.clone(),
                    inferred: fact.inferred,
                };
                actions.push(Action::FinalizeFailed {
                    alloc_id: alloc_id.clone(),
                    terminal: Some(TerminalCondition::Stable { settled_in_ms, witness }),
                });
                next_view.stable_announced.insert(alloc_id.clone());
                stable_this_tick.insert(alloc_id.clone());
                continue;
            }

            // Branch (c): EarlyExit — alloc Failed within startup_deadline,
            // no Pass observed. Closes RCA-A per US-08.
            //
            // `started_at == None` on a Failed alloc means the alloc
            // never reached Running — the elapsed-vs-deadline
            // classification doesn't apply. Skip the EarlyExit
            // branch; the row's typed `terminal` flows through other
            // projections (Custom → Other) per the audit's branch
            // semantics table.
            if fact.state == AllocState::Failed {
                let Some(started) = fact.started_at else {
                    // Pre-Running Failed: the alloc reached a terminal state but
                    // never started, so the elapsed-vs-deadline EarlyExit
                    // classification does not apply and the reconciler emits no
                    // FinalizeFailed (the row's typed terminal flows through other
                    // projections via WorkloadLifecycle). Still record the terminal
                    // membership so `has_alloc_mid_startup_window` returns false and
                    // the runtime stops self-re-enqueueing this dead alloc after it
                    // is archived (otherwise the stale `observed` entry — inserted
                    // above — keeps the predicate true forever → no-op busy-loop).
                    next_view.terminal_announced.insert(alloc_id.clone());
                    continue;
                };
                let elapsed_ms = elapsed_ms_from(tick.now_unix, started);
                let deadline_ms =
                    u64::try_from(fact.startup_deadline.as_millis()).unwrap_or(u64::MAX);
                let within_deadline = elapsed_ms < deadline_ms;
                let no_pass = !matches!(fact.latest_startup_probe, Some(ProbeStatus::Pass));
                if within_deadline && no_pass {
                    actions.push(Action::FinalizeFailed {
                        alloc_id: alloc_id.clone(),
                        terminal: Some(TerminalCondition::ServiceFailed {
                            reason: ServiceFailureReason::EarlyExit { exit_code: fact.exit_code },
                        }),
                    });
                    // GAP-9 — record the non-Stable terminal so the
                    // dedup guard above skips this alloc next tick and
                    // the Shape B predicate returns false for it.
                    next_view.terminal_announced.insert(alloc_id.clone());
                    continue;
                } // fall-through to StartupProbeFailed branch
            }

            // Branch (b): StartupProbeFailed — attempts exhausted AND
            // deadline elapsed AND no Pass observed. Extracted into
            // `startup_probe_failed_action` (terminal CONDITION verbatim);
            // the `attempts` it reads is the post-`update_startup_attempts`
            // value recorded above.
            let attempts = next_view.startup_attempts_per_alloc.get(alloc_id).copied().unwrap_or(0);
            if let Some(action) =
                startup_probe_failed_action(alloc_id, fact, attempts, tick.now_unix)
            {
                actions.push(action);
                startup_failed_this_tick.insert(alloc_id.clone());
                // GAP-9 — record the non-Stable terminal (see EarlyExit
                // branch for the dedup + predicate-falseness rationale).
                next_view.terminal_announced.insert(alloc_id.clone());
            }
        }

        // ---- ADR-0101 — authoritative ServiceBackendRow projection ----
        //
        // ServiceLifecycle owns the complete row: current Running
        // membership, listener address materialization, and the existing
        // readiness/terminal-veto policy are composed before comparing with
        // the observed row. Place all row/hydrator groups before the first
        // startup-failure publication so a deciding tick withdraws a vetoed
        // backend before its terminal condition is dispatched.
        let backend_actions =
            service_backend_row_actions(actual, &mut next_view, tick, &startup_failed_this_tick);
        if !backend_actions.is_empty() {
            let insert_at = actions
                .iter()
                .position(|action| {
                    matches!(
                        action,
                        Action::FinalizeFailed {
                            terminal: Some(TerminalCondition::ServiceFailed {
                                reason: ServiceFailureReason::StartupProbeFailed { .. },
                            }),
                            ..
                        }
                    )
                })
                .unwrap_or(actions.len());
            actions.splice(insert_at..insert_at, backend_actions);
        }

        // ---- Step 03-02 / Slice 05 — liveness → RestartAllocation ----
        collect_liveness_actions(actual, &stable_this_tick, &mut next_view, &mut actions);

        (actions, next_view)
    }

    /// Hydrate the empty `desired` projection. ServiceLifecycle computes its
    /// complete backend rows from the actual allocation/listener projection;
    /// target validation is retained at this trait boundary.
    async fn hydrate_desired(
        &self,
        _ctx: &HydrationContext<'_>,
        target: &TargetResource,
    ) -> Result<Self::State, HydrateError> {
        let _workload_id = crate::workload_id_from_target(target)?;
        Ok(ServiceLifecycleState::default())
    }

    /// Hydrate the `actual` projection (ADR-0086 D1; moved off the central
    /// `reconciler_runtime::hydrate_service_lifecycle_actual`).
    async fn hydrate_actual(
        &self,
        ctx: &HydrationContext<'_>,
        target: &TargetResource,
    ) -> Result<Self::State, HydrateError> {
        let workload_id = crate::workload_id_from_target(target)?;
        hydrate_service_lifecycle_actual(ctx, &workload_id).await
    }
}

// ---------------------------------------------------------------------------
// Hydration bodies — moved off the central `reconciler_runtime` free fns
// (ADR-0086 S3).
// ---------------------------------------------------------------------------

/// Liveness probe `failure_threshold` default per ADR-0057 §2 / DDD-14.
const LIVENESS_FAILURE_THRESHOLD_DEFAULT: u32 = 3;

/// Read `WorkloadIntent::Service(ServiceV2)` for `workload_id`; `Ok(None)` when
/// absent or a `Job` / `Schedule` variant.
async fn service_spec_from_intent(
    ctx: &HydrationContext<'_>,
    workload_id: &WorkloadId,
) -> Result<Option<ServiceV2>, HydrateError> {
    let key = IntentKey::for_workload(workload_id);
    let Some(bytes) = ctx
        .intent_store
        .get(key.as_bytes())
        .await
        .map_err(|e| HydrateError::IntentRead(e.to_string()))?
    else {
        return Ok(None);
    };
    let intent =
        WorkloadIntent::from_store_bytes(bytes.as_ref(), ctx.intent_redb_path, Some(key.as_str()))
            .map_err(|e| HydrateError::IntentRead(e.to_string()))?;
    match intent {
        WorkloadIntent::Service(svc) => Ok(Some(svc)),
        WorkloadIntent::Job(_) | WorkloadIntent::Schedule(_) => Ok(None),
    }
}

/// Format a `ProbeMechanic` into the operator-facing `mechanic_summary` string.
fn format_mechanic_summary(mechanic: &ProbeMechanic) -> String {
    match mechanic {
        ProbeMechanic::Tcp { host, port } => format!("tcp {host}:{port}"),
        ProbeMechanic::Http { path, port, host } => host
            .as_ref()
            .map_or_else(|| format!("http {path}"), |h| format!("http {h}:{port}{path}")),
        ProbeMechanic::Exec { command } => {
            command.first().map_or_else(|| "exec".to_string(), |c| format!("exec {c}"))
        }
    }
}

/// Project the spec-derived startup facts uniform across every alloc.
fn spec_facts_for_service(svc: &ServiceV2) -> (u32, Duration, String, bool, bool) {
    let startup_probes_empty = svc.startup_probes.is_empty();
    if startup_probes_empty {
        return (30, DEFAULT_STARTUP_DEADLINE, String::new(), false, true);
    }
    let probe = &svc.startup_probes[0];
    let max_attempts = probe.max_attempts;
    let interval = Duration::from_secs(u64::from(probe.interval_seconds));
    let startup_deadline =
        interval.checked_mul(probe.max_attempts).unwrap_or(DEFAULT_STARTUP_DEADLINE);
    let mechanic_summary = format_mechanic_summary(&probe.mechanic);
    (max_attempts, startup_deadline, mechanic_summary, probe.inferred, false)
}

/// Project the readiness facts uniform across every alloc.
fn readiness_facts_for_service(svc: &ServiceV2) -> (bool, u32) {
    let has_readiness_probe = !svc.readiness_probes.is_empty();
    let success_threshold =
        svc.readiness_probes.first().and_then(|p| p.success_threshold).unwrap_or(1);
    (has_readiness_probe, success_threshold)
}

/// Project the liveness facts uniform across every alloc.
fn liveness_facts_for_service(svc: &ServiceV2) -> (bool, u32) {
    let has_liveness_probe = !svc.liveness_probes.is_empty();
    let failure_threshold = svc
        .liveness_probes
        .first()
        .and_then(|p| p.failure_threshold)
        .unwrap_or(LIVENESS_FAILURE_THRESHOLD_DEFAULT);
    (has_liveness_probe, failure_threshold)
}

/// Resolve every current listener's dataplane identity (allocator-issued VIP,
/// listener port/protocol, and local writer node) via the `ServiceVipView`
/// read-port. The enclosing map key supplies each listener's ServiceId.
async fn service_dataplane_identities(
    ctx: &HydrationContext<'_>,
    workload_id: &WorkloadId,
    svc: &ServiceV2,
) -> Result<BTreeMap<ServiceId, ServiceDataplaneIdentity>, HydrateError> {
    if svc.listeners.is_empty() {
        return Ok(BTreeMap::new());
    }
    let key = IntentKey::for_workload(workload_id);
    let Some(bytes) = ctx
        .intent_store
        .get(key.as_bytes())
        .await
        .map_err(|e| HydrateError::IntentRead(e.to_string()))?
    else {
        return Ok(BTreeMap::new());
    };
    let intent =
        WorkloadIntent::from_store_bytes(bytes.as_ref(), ctx.intent_redb_path, Some(key.as_str()))
            .map_err(|e| HydrateError::IntentRead(e.to_string()))?;
    let spec_digest_hash =
        intent.spec_digest().map_err(|e| HydrateError::IntentRead(e.to_string()))?;
    let Some(assigned_vip) = ctx.service_vip_view.assigned_vip(&spec_digest_hash).await else {
        tracing::debug!(
            name: "service_lifecycle.allocator_memo_absent",
            workload_id = %workload_id,
            spec_digest = %spec_digest_hash,
            "VIP allocator memo absent for Service intent; deferring listener projection",
        );
        return Ok(BTreeMap::new());
    };
    let mut identities = BTreeMap::new();
    for listener in &svc.listeners {
        let service_id =
            ServiceId::derive(&assigned_vip, listener.port, listener.protocol, "service-map");
        identities.insert(
            service_id,
            ServiceDataplaneIdentity {
                vip: assigned_vip.clone(),
                port: listener.port,
                protocol: listener.protocol,
                writer: ctx.node_id.clone(),
            },
        );
    }
    Ok(identities)
}

/// LWW-latest projection of one probe's observed status on the full
/// `(role, probe_idx)` identity (ADR-0080 §D3).
fn latest_probe_row(
    rows: &[ProbeResultRow],
    role: ProbeRole,
    probe_idx: ProbeIdx,
) -> Option<&ProbeResultRow> {
    rows.iter()
        .filter(|p| p.role == role && p.probe_idx == probe_idx)
        .max_by_key(|p| p.last_observed_at_unix_ms)
}

/// Per-workload projection of every `AllocStatusRow` into `ServiceAllocFact`,
/// joining each row with its LWW probe projections and the spec-derived facts.
async fn hydrate_service_alloc_facts(
    ctx: &HydrationContext<'_>,
    workload_id: &WorkloadId,
    spec_facts: &(u32, Duration, String, bool, bool),
    readiness_facts: &(bool, u32),
    liveness_facts: &(bool, u32),
) -> Result<BTreeMap<AllocationId, ServiceAllocFact>, HydrateError> {
    let (max_attempts, startup_deadline, mechanic_summary, inferred, startup_probes_empty) =
        spec_facts;
    let (has_readiness_probe, readiness_success_threshold) = *readiness_facts;
    let (has_liveness_probe, liveness_failure_threshold) = *liveness_facts;
    let rows = ctx
        .observation_store
        .alloc_status_rows()
        .await
        .map_err(|e| HydrateError::ObservationRead(e.to_string()))?;
    let mut allocs = BTreeMap::new();
    for row in rows.into_iter().filter(|r| r.workload_id == *workload_id) {
        let probe_rows = ctx
            .observation_store
            .list_probe_results_for_alloc(&row.alloc_id)
            .await
            .map_err(|e| HydrateError::ObservationRead(e.to_string()))?;
        let latest_startup_probe =
            latest_probe_row(&probe_rows, ProbeRole::Startup, ProbeIdx::new(0));
        let latest_readiness_probe =
            latest_probe_row(&probe_rows, ProbeRole::Readiness, ProbeIdx::new(0));
        let latest_liveness_probe =
            latest_probe_row(&probe_rows, ProbeRole::Liveness, ProbeIdx::new(0));

        let backend_spiffe = SpiffeId::for_allocation(workload_id, &row.alloc_id);
        let backend_ip = row.workload_addr.unwrap_or(ctx.host_ipv4);

        let exit_code = match row.reason {
            Some(TransitionReason::WorkloadCrashedImmediately { exit_code, .. }) => exit_code,
            _ => None,
        };
        let fact = ServiceAllocFact {
            alloc_id: row.alloc_id.clone(),
            state: row.state,
            started_at: row.started_at,
            exit_code,
            latest_startup_probe: latest_startup_probe.map(|row| row.status.clone()),
            latest_startup_probe_observed_at: latest_startup_probe.map(|row| {
                UnixInstant::from_unix_duration(Duration::from_millis(row.last_observed_at_unix_ms))
            }),
            max_attempts: *max_attempts,
            startup_deadline: *startup_deadline,
            mechanic_summary: mechanic_summary.clone(),
            inferred: *inferred,
            startup_probes_empty: *startup_probes_empty,
            latest_readiness_probe: latest_readiness_probe.map(|row| row.status.clone()),
            has_readiness_probe,
            readiness_success_threshold,
            backend_spiffe,
            backend_ip,
            latest_liveness_probe: latest_liveness_probe.map(|row| row.status.clone()),
            has_liveness_probe,
            liveness_failure_threshold,
        };
        allocs.insert(row.alloc_id, fact);
    }
    Ok(allocs)
}

/// Actual-side projection: join the per-alloc facts with every current
/// listener identity and its complete observed backend row.
async fn hydrate_service_lifecycle_actual(
    ctx: &HydrationContext<'_>,
    workload_id: &WorkloadId,
) -> Result<ServiceLifecycleState, HydrateError> {
    let Some(spec) = service_spec_from_intent(ctx, workload_id).await? else {
        return Ok(ServiceLifecycleState {
            allocs: BTreeMap::new(),
            service_dataplane: BTreeMap::new(),
            observed_backend_rows: BTreeMap::new(),
        });
    };
    let spec_facts = spec_facts_for_service(&spec);
    let readiness_facts = readiness_facts_for_service(&spec);
    let liveness_facts = liveness_facts_for_service(&spec);
    let service_dataplane = service_dataplane_identities(ctx, workload_id, &spec).await?;
    let mut observed_backend_rows = BTreeMap::new();
    for service_id in service_dataplane.keys() {
        if let Some(row) = ctx
            .observation_store
            .service_backends_rows(service_id)
            .await
            .map_err(|e| HydrateError::ObservationRead(e.to_string()))?
            .into_iter()
            .next()
        {
            observed_backend_rows.insert(*service_id, row);
        }
    }
    let allocs = hydrate_service_alloc_facts(
        ctx,
        workload_id,
        &spec_facts,
        &readiness_facts,
        &liveness_facts,
    )
    .await?;
    Ok(ServiceLifecycleState { allocs, service_dataplane, observed_backend_rows })
}

/// ADR-0087 D2 — walk every alloc declaring a liveness probe, maintain
/// its consecutive-failure counter in the View, and emit a liveness
/// TERMINATE (`StopAllocation { terminal: Stopped { by: LivenessProbe } }`)
/// when the trigger predicate holds. `ServiceLifecycle` makes NO
/// restart-vs-finalize decision and reads NO budget — the termination IS
/// the signal, and `WorkloadLifecycle` (the sole restart authority)
/// restarts the liveness-terminated row under its single budget.
/// Extracted from `reconcile` to stay under the clippy `too_many_lines`
/// limit.
fn collect_liveness_actions(
    actual: &ServiceLifecycleState,
    stable_this_tick: &BTreeSet<AllocationId>,
    next_view: &mut ServiceLifecycleView,
    actions: &mut Vec<Action>,
) {
    for (alloc_id, fact) in &actual.allocs {
        if next_view.terminal_announced.contains(alloc_id) || stable_this_tick.contains(alloc_id) {
            continue;
        }
        // Idempotency (ADR-0087 D2): the counter reset-on-emit below,
        // combined with the `state == Running` guard inside
        // `liveness_terminate_action`, prevents a double-terminate while
        // the shim's stop is in flight — once the row leaves Running the
        // predicate is false, and after restart the fresh Running alloc
        // starts with a clean counter. No `terminal_announced` marker is
        // needed: a `StopAllocation` is not a terminal claim.
        if let Some(action) = liveness_terminate_action(alloc_id, fact, next_view) {
            actions.push(action);
        }
    }
}

/// ADR-0087 D2 — maintain the per-(alloc, probe_idx) liveness
/// consecutive-failure counter (the View INPUT) and, when the
/// recomputed liveness-threshold predicate holds this tick, emit a
/// liveness TERMINATE. `ServiceLifecycle` is demoted to a liveness
/// DETECTOR: it makes NO restart-vs-finalize decision, reads NO restart
/// budget, and carries no `restart_count` / `restart_spec` — the
/// termination IS the signal (kubelet shape), and `WorkloadLifecycle`
/// (the sole restart authority) restarts the liveness-terminated row
/// under its single budget.
///
/// Counter maintenance (mirrors the readiness consecutive-Pass shape,
/// inverted for failures):
/// - `Some(Fail)` → streak grows by one (saturating at `u32::MAX`).
/// - `Some(Pass)` → recovery: streak resets to 0 (entry removed;
///   absence == 0). Per S-SHCP-RECON-10 a Pass below threshold clears
///   the counter and emits NO terminate.
/// - `None` → no liveness observation this tick; leave the counter
///   untouched.
///
/// Trigger predicate (recomputed every tick from the post-update
/// counter + the live `failure_threshold`, never persisted):
/// `state == Running AND consecutive_failures >= failure_threshold`.
/// When it holds, emit exactly one
/// `StopAllocation { terminal: Stopped { by: LivenessProbe } }` — the
/// cause travels on the shared observed `AllocStatusRow.terminal`
/// (ADR-0087 D3), never in a budget read or a restart-cause field.
///
/// Returns `None` when the alloc declares no liveness probe, or when
/// the predicate does not hold (Running-but-below-threshold, recovery,
/// non-Running state).
fn liveness_terminate_action(
    alloc_id: &AllocationId,
    fact: &ServiceAllocFact,
    next_view: &mut ServiceLifecycleView,
) -> Option<Action> {
    if !fact.has_liveness_probe {
        return None;
    }

    let key = (alloc_id.clone(), ProbeIdx::new(0));
    let consecutive_failures = match &fact.latest_liveness_probe {
        Some(ProbeStatus::Fail { .. }) => {
            let entry = next_view.liveness_consecutive_failures.entry(key.clone()).or_insert(0);
            *entry = entry.saturating_add(1);
            *entry
        }
        // Recovery (Pass) OR no observation yet → streak resets to 0.
        // Removing the entry keeps the persisted map minimal
        // (absence == 0) — per S-SHCP-RECON-10.
        Some(ProbeStatus::Pass) => {
            next_view.liveness_consecutive_failures.remove(&key);
            0
        }
        None => next_view.liveness_consecutive_failures.get(&key).copied().unwrap_or(0),
    };

    // Predicate recomputed this tick from the counter INPUT + the live
    // policy threshold. Below threshold OR not Running → no action.
    let triggered = fact.state == AllocState::Running
        && consecutive_failures >= fact.liveness_failure_threshold;
    if !triggered {
        return None;
    }

    // Reset the consecutive-failure counter on emit (ADR-0087 D2,
    // retained load-bearing). Combined with the `state == Running` guard
    // above this prevents a double-terminate while the shim's stop is in
    // flight: once the row leaves Running the predicate is false, and the
    // restarted fresh alloc starts with a clean counter (no stale
    // threshold-exceeding value re-firing a terminate on the first
    // post-restart Running tick before probes have re-fired).
    next_view.liveness_consecutive_failures.remove(&key);

    // The liveness TERMINATE. The cause travels on the observed row's
    // `terminal` (`Stopped { by: LivenessProbe }`); `WorkloadLifecycle`
    // reads that terminal to restart under its single budget and, at
    // exhaustion, to finalise `ServiceFailed { LivenessProbeFailed }`.
    Some(Action::StopAllocation {
        alloc_id: alloc_id.clone(),
        terminal: Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }),
    })
}

/// ADR-0101 — recompute every current listener's complete backend row and
/// return the changed row/hydrator-handoff action pairs.
///
/// `healthy` derivation per backend, in priority order:
/// - alloc has NO readiness probe → `healthy = true` (backward-compat
///   default — the service serves traffic the instant it is Stable,
///   S-SHCP-RECON-08b).
/// - alloc HAS a readiness probe → `healthy = (latest_readiness == Pass
///   AND consecutive_successes >= success_threshold)`. The
///   consecutive-Pass counter is the View INPUT, incremented on Pass
///   and reset to 0 on Fail (or no observation yet). Initial state
///   (no Pass row yet) → counter 0 → `healthy = false`
///   (S-SHCP-RECON-08c — avoids the inverse race).
///
/// Mutates `next_view.readiness_consecutive_successes` in place (the
/// persisted INPUT). Empty membership is still a desired empty row, while
/// no current listener identities produces no managed row keys.
fn service_backend_row_actions(
    actual: &ServiceLifecycleState,
    next_view: &mut ServiceLifecycleView,
    tick: &TickContext,
    startup_failed_this_tick: &BTreeSet<AllocationId>,
) -> Vec<Action> {
    if actual.service_dataplane.is_empty() {
        return Vec::new();
    }

    // Readiness is an allocation-level policy. Compute it once per current
    // Running allocation, then reuse the result for every listener so adding
    // listeners never multiplies the consecutive-Pass counter.
    let mut healthy_by_alloc: BTreeMap<AllocationId, bool> = BTreeMap::new();
    for (alloc_id, fact) in &actual.allocs {
        if fact.state != AllocState::Running {
            continue;
        }
        let terminal_startup_veto = startup_failed_this_tick.contains(alloc_id)
            || next_view.terminal_announced.contains(alloc_id);
        let healthy = compute_backend_healthy(alloc_id, fact, next_view, terminal_startup_veto);
        healthy_by_alloc.insert(alloc_id.clone(), healthy);
    }

    let mut actions = Vec::new();
    for (service_id, identity) in &actual.service_dataplane {
        let backends: Vec<Backend> = actual
            .allocs
            .iter()
            .filter(|(_, fact)| fact.state == AllocState::Running)
            .map(|(alloc_id, fact)| Backend {
                alloc: fact.backend_spiffe.clone(),
                addr: SocketAddr::new(IpAddr::V4(fact.backend_ip), identity.port.get()),
                weight: 1,
                healthy: healthy_by_alloc.get(alloc_id).copied().unwrap_or(false),
            })
            .collect();

        let vip = identity.vip.try_as_ipv4().unwrap_or(Ipv4Addr::UNSPECIFIED);
        let observed = actual.observed_backend_rows.get(service_id);
        let unchanged = observed.is_some_and(|row| {
            row.service_id == *service_id && row.vip == vip && row.backends == backends
        });
        if unchanged {
            continue;
        }

        let fp = fingerprint(&identity.vip, &backends);
        let target = format!("service-lifecycle/backends/{service_id}");
        let spec_hash = ContentHash::of(fp.to_le_bytes().as_slice());
        let correlation = CorrelationKey::derive(&target, &spec_hash, "write-service-backend-row");
        let row = ServiceBackendRow {
            service_id: *service_id,
            vip,
            backends,
            updated_at: LogicalTimestamp::dominating(
                tick.tick,
                identity.writer.clone(),
                observed.map(|row| &row.updated_at),
            ),
        };

        actions.push(Action::WriteServiceBackendRow { row, correlation });
        #[allow(clippy::expect_used)]
        {
            let hydrator_name = ReconcilerName::new(
                <crate::service_map_hydrator::ServiceMapHydrator as Reconciler>::NAME,
            )
            .expect("service-map-hydrator NAME is valid by construction");
            let hydrator_target = TargetResource::new(&format!("service/{service_id}"))
                .expect("service/<service-id> target is valid by construction");
            actions.push(Action::EnqueueEvaluation {
                reconciler: hydrator_name,
                target: hydrator_target,
            });
        }
    }
    actions
}

/// Recompute one backend's `healthy` flag for the current tick and
/// update the persisted consecutive-Pass counter INPUT in
/// `next_view`. See [`service_backend_row_actions`] for the row projection
/// contract.
fn compute_backend_healthy(
    alloc_id: &AllocationId,
    fact: &ServiceAllocFact,
    next_view: &mut ServiceLifecycleView,
    startup_failed_this_tick: bool,
) -> bool {
    if startup_failed_this_tick {
        return false;
    }
    if !fact.has_readiness_probe {
        // Backward-compat default: no readiness gate → always healthy.
        return true;
    }

    let key = (alloc_id.clone(), ProbeIdx::new(0));
    let counter = match &fact.latest_readiness_probe {
        Some(ProbeStatus::Pass) => {
            // Consecutive-Pass streak grows by one this tick.
            let entry = next_view.readiness_consecutive_successes.entry(key).or_insert(0);
            *entry = entry.saturating_add(1);
            *entry
        }
        // Fail OR no observation yet → streak resets to 0. Removing the
        // entry keeps the persisted map minimal (absence == 0).
        Some(ProbeStatus::Fail { .. }) | None => {
            next_view.readiness_consecutive_successes.remove(&key);
            0
        }
    };

    matches!(fact.latest_readiness_probe, Some(ProbeStatus::Pass))
        && counter >= fact.readiness_success_threshold
}

/// GAP-10 — maintain the per-alloc consecutive-startup-probe-fail
/// counter that the `StartupProbeFailed` gate reads.
///
/// Semantics per ADR-0057 §2 (`attempts` = CONSECUTIVE startup-probe
/// failures):
///
/// - A new `Some(Fail)` LWW observation → increment by exactly 1 (saturating
///   at `u32::MAX`); the unchanged observation leaves the count untouched.
///   A legacy counter without its paired observed timestamp is reset to one
///   for the current observation.
/// - `Some(Pass)` → reset to 0 by removing the entry (recovery clears
///   the streak; the alloc proceeds to Stable in branch (a)).
/// - `None` → leave the map untouched (no probe observed this tick:
///   neither a failure nor a recovery).
///
/// Extracted from `reconcile` so the per-alloc body stays under the
/// `too_many_lines` budget and the increment/reset logic is unit-pinned
/// indirectly through the reconcile branch tests.
#[inline]
fn update_startup_attempts(
    counters: &mut BTreeMap<AllocationId, u32>,
    last_fail_seen_at: &mut BTreeMap<AllocationId, u64>,
    alloc_id: &AllocationId,
    latest_startup_probe: Option<&ProbeStatus>,
    latest_startup_probe_observed_at: Option<UnixInstant>,
) {
    match (latest_startup_probe, latest_startup_probe_observed_at) {
        (Some(ProbeStatus::Fail { .. }), Some(observed_at)) => {
            let observed_at_unix_ms =
                u64::try_from(observed_at.as_unix_duration().as_millis()).unwrap_or(u64::MAX);
            if counters.contains_key(alloc_id) && !last_fail_seen_at.contains_key(alloc_id) {
                counters.insert(alloc_id.clone(), 1);
                last_fail_seen_at.insert(alloc_id.clone(), observed_at_unix_ms);
            } else if last_fail_seen_at.get(alloc_id).copied() != Some(observed_at_unix_ms) {
                let counter = counters.entry(alloc_id.clone()).or_insert(0);
                *counter = counter.saturating_add(1);
                last_fail_seen_at.insert(alloc_id.clone(), observed_at_unix_ms);
            }
        }
        (Some(ProbeStatus::Pass), _) => {
            counters.remove(alloc_id);
            last_fail_seen_at.remove(alloc_id);
        }
        _ => {}
    }
}

/// Branch (b) — StartupProbeFailed terminal action, or `None` when the
/// three-gate condition is not met.
///
/// The terminal CONDITION is unchanged from the inline branch it was
/// extracted from: `attempts >= max_attempts && elapsed_ms >=
/// deadline_ms && no_pass`. The `attempts` argument is the
/// post-[`update_startup_attempts`] consecutive-fail count the caller
/// recorded for this tick.
///
/// `started_at == None` means the alloc never reached Running — no
/// probes ran to fail — so the branch is skipped (returns `None`).
#[inline]
fn startup_probe_failed_action(
    alloc_id: &AllocationId,
    fact: &ServiceAllocFact,
    attempts: u32,
    now_unix: UnixInstant,
) -> Option<Action> {
    // `AllocState` gate (brief.md §105a.9/§105a.10, S-VM-29). This branch
    // previously read NO `AllocState` at all — it fired for ANY state
    // reaching it, including `Terminated`. A platform-reclaimed row
    // (`brief.md` §105a.5) is ALWAYS `Terminated` (never `Pending` /
    // `Running` / `Failed`), so an already-reclaimed allocation whose
    // startup probes had been failing raced a fabricated
    // `ServiceFailed { StartupProbeFailed }` claim over an ending already
    // authored elsewhere. Excluding `Terminated` specifically — rather
    // than allow-listing `Running | Failed` — is deliberate: the existing
    // branch-coverage suite (`tests/acceptance/service_lifecycle_reconcile_branches.rs::
    // startup_probe_failed_fires_when_all_three_gates_met` et al.)
    // deliberately drives this branch from `AllocState::Pending` (no
    // Stable/EarlyExit gate applies yet, but the three attempts/deadline/
    // no-pass gates below still must fire) — an allow-list would silently
    // break that pinned, already-covered case. `Terminated` is the one
    // state this branch is never entitled to act on: every reachable
    // `Terminated` row already carries a different ending authored by
    // another path (reclamation, an operator/reconciler stop, …).
    if fact.state == AllocState::Terminated {
        return None;
    }
    let started = fact.started_at?;
    let elapsed_ms = elapsed_ms_from(now_unix, started);
    let deadline_ms = u64::try_from(fact.startup_deadline.as_millis()).unwrap_or(u64::MAX);
    let no_pass = !matches!(fact.latest_startup_probe, Some(ProbeStatus::Pass));
    if attempts >= fact.max_attempts && elapsed_ms >= deadline_ms && no_pass {
        let last_fail = match &fact.latest_startup_probe {
            Some(ProbeStatus::Fail { last_fail_reason }) => last_fail_reason.clone(),
            _ => String::new(),
        };
        Some(Action::FinalizeFailed {
            alloc_id: alloc_id.clone(),
            terminal: Some(TerminalCondition::ServiceFailed {
                reason: ServiceFailureReason::StartupProbeFailed {
                    probe_idx: 0,
                    last_fail,
                    attempts,
                },
            }),
        })
    } else {
        None
    }
}

/// Compute `now - started_at` as milliseconds, saturating to `u64::MAX`
/// at the conversion boundary and to `Duration::ZERO` (= `0u64`) at the
/// underflow boundary per `UnixInstant`'s `Sub` semantics (see
/// `wall_clock.rs` `impl Sub<Self> for UnixInstant`).
///
/// Typed-`Duration` arithmetic: callers pass typed `UnixInstant`s; the
/// `u64` ms cast happens at the function boundary, not at the call site.
#[inline]
#[must_use]
fn settled_in_ms_from(now: UnixInstant, started_at: UnixInstant) -> u64 {
    u64::try_from((now - started_at).as_millis()).unwrap_or(u64::MAX)
}

/// Compute `now - started_at` as milliseconds, mirroring
/// [`settled_in_ms_from`] but named for the EarlyExit /
/// StartupProbeFailed branches that read it as `elapsed_ms`. Inlined
/// so the two call sites read the same shape; the two functions exist
/// to keep call-site intent (settled vs elapsed) legible.
#[inline]
#[must_use]
fn elapsed_ms_from(now: UnixInstant, started_at: UnixInstant) -> u64 {
    u64::try_from((now - started_at).as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 128,
            rng_seed: proptest::test_runner::RngSeed::Fixed(257_222),
            ..ProptestConfig::default()
        })]

        /// CONTRACT_SHAPE: pure-function.
        /// ADR-0101 policy complement: the terminal veto dominates readiness;
        /// otherwise absent/Fail resets, Pass advances exactly once with
        /// saturation, and no-readiness leaves every View input untouched.
        /// This is a pure policy truth table, not a claim that a seeded View is
        /// reachable. The composed Sim tests establish that separate premise.
        #[test]
        fn backend_policy_preserves_veto_threshold_and_unrelated_view_inputs(
            count in prop_oneof![Just(0), Just(u32::MAX), any::<u32>()],
            threshold in prop_oneof![Just(1), Just(u32::MAX), 1_u32..=u32::MAX],
        ) {
            let alloc = AllocationId::new("policy-property").unwrap();
            let unrelated = AllocationId::new("unrelated-policy").unwrap();
            for enabled in [false, true] {
                for veto in [false, true] {
                    for status in [None, Some(ProbeStatus::Pass), Some(ProbeStatus::Fail { last_fail_reason: "refused".into() })] {
                        let fact = ServiceAllocFact {
                            alloc_id: alloc.clone(), state: AllocState::Running,
                            started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
                            exit_code: None, latest_startup_probe: None,
                            latest_startup_probe_observed_at: None, max_attempts: 3,
                            startup_deadline: Duration::from_secs(30), mechanic_summary: "tcp 0.0.0.0:8080".into(),
                            inferred: false, startup_probes_empty: false,
                            latest_readiness_probe: status.clone(), has_readiness_probe: enabled,
                            readiness_success_threshold: threshold,
                            backend_spiffe: SpiffeId::new("spiffe://overdrive.local/workload/policy/alloc/0").unwrap(),
                            backend_ip: "192.0.2.10".parse().unwrap(),
                            latest_liveness_probe: None, has_liveness_probe: false, liveness_failure_threshold: 3,
                        };
                        let key = (alloc.clone(), ProbeIdx::new(0));
                        let other_key = (unrelated.clone(), ProbeIdx::new(0));
                        let mut actual = ServiceLifecycleView {
                            readiness_consecutive_successes: BTreeMap::from([(key.clone(), count), (other_key, 7)]),
                            ..ServiceLifecycleView::default()
                        };
                        actual.terminal_announced.insert(unrelated.clone());
                        actual.stable_announced.insert(unrelated.clone());
                        actual.observed.insert(alloc.clone());
                        let mut expected = actual.clone();
                        let passes = matches!(status, Some(ProbeStatus::Pass));
                        let next = u32::try_from((u64::from(count) + 1).min(u64::from(u32::MAX))).unwrap();
                        if enabled && !veto {
                            if passes { expected.readiness_consecutive_successes.insert(key.clone(), next); }
                            else { expected.readiness_consecutive_successes.remove(&key); }
                        }
                        let expected_healthy = !veto && (!enabled || (passes && next >= threshold));
                        let healthy = compute_backend_healthy(&alloc, &fact, &mut actual, veto);
                        prop_assert_eq!(healthy, expected_healthy);
                        prop_assert_eq!(actual, expected);
                    }
                }
            }
        }
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn startup_attempts_count_each_lww_failure_once_and_normalize_legacy_view() {
        let alloc = AllocationId::new("startup-observation-accounting").expect("valid alloc id");
        let failure = ProbeStatus::Fail { last_fail_reason: "refused".to_owned() };
        let mut counters = BTreeMap::from([(alloc.clone(), 20)]);
        let mut timestamps = BTreeMap::new();

        let instant = |unix_ms| UnixInstant::from_unix_duration(Duration::from_millis(unix_ms));
        update_startup_attempts(
            &mut counters,
            &mut timestamps,
            &alloc,
            Some(&failure),
            Some(instant(100)),
        );
        assert_eq!(counters[&alloc], 1);
        assert_eq!(timestamps[&alloc], 100);

        update_startup_attempts(
            &mut counters,
            &mut timestamps,
            &alloc,
            Some(&failure),
            Some(instant(100)),
        );
        assert_eq!(counters[&alloc], 1);

        update_startup_attempts(
            &mut counters,
            &mut timestamps,
            &alloc,
            Some(&ProbeStatus::Pass),
            Some(instant(101)),
        );
        assert!(counters.is_empty());
        assert!(timestamps.is_empty());

        for observed_at in [102, 103, 104] {
            update_startup_attempts(
                &mut counters,
                &mut timestamps,
                &alloc,
                Some(&failure),
                Some(instant(observed_at)),
            );
        }
        assert_eq!(counters[&alloc], 3);
        assert_eq!(timestamps[&alloc], 104);
    }
}
