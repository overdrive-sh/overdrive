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
//! [`crate::transition_reason`] (so they can be carried inside
//! [`crate::TerminalCondition::ServiceFailed`] / `::Stable` without
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

use crate::dataplane::fingerprint::{BackendSetFingerprint, fingerprint};
use crate::id::{AllocationId, ServiceId, ServiceVip, SpiffeId};
use crate::observation::{ProbeIdx, ProbeStatus};
use crate::traits::observation_store::AllocState;

// Re-exports — see file-header docstring for the cycle-breaking
// rationale.
pub use crate::transition_reason::{ProbeWitness, ServiceFailureReason};

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
    /// [`crate::traits::clock::Clock`] port. Sourced verbatim from
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
    /// construct the [`crate::traits::dataplane::Backend`] this alloc
    /// contributes to the service's backend set.
    pub backend_spiffe: SpiffeId,
    /// Socket address this alloc serves on as a dataplane backend.
    pub backend_addr: std::net::SocketAddr,

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
    /// Running alloc trigger `RestartAllocation`); configurable.
    /// Sourced from the live `ServiceSpec` — re-evaluated every tick,
    /// never persisted.
    pub liveness_failure_threshold: u32,
    /// How many times the SHARED `WorkloadLifecycle` restart budget has
    /// already restarted this alloc (`WorkloadLifecycleView::
    /// restart_counts`). The liveness branch composes with — does NOT
    /// duplicate — the shared `RESTART_BACKOFF_CEILING` budget per
    /// ADR-0055 §7: once `restart_count >= RESTART_BACKOFF_CEILING` the
    /// liveness branch stops emitting `RestartAllocation` and finalises
    /// the alloc with `TerminalCondition::ServiceFailed {
    /// LivenessProbeFailed }` so operators can distinguish from crash-
    /// loop `BackoffExhausted`. Sourced from the runtime's
    /// per-alloc restart-status projection (intent/observation join) —
    /// an INPUT, recomputed each tick, never a cached verdict.
    pub restart_count: u32,
    /// Fully-populated `AllocationSpec` the liveness branch clones into
    /// the emitted `Action::RestartAllocation { spec, .. }` so the
    /// action_shim's stop+start replays the workload with its live
    /// command / args / identity / resources / probe descriptors.
    /// Hydrated from the live `Job` (intent side) — the SAME source the
    /// `WorkloadLifecycle` crash-loop restart pathway uses
    /// (`workload_lifecycle.rs` Run branch). Carrying it on the fact
    /// keeps the reconciler pure: the spec is an input projected by the
    /// runtime's hydrate pass, not re-derived inside `reconcile`.
    pub restart_spec: crate::traits::driver::AllocationSpec,
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

    /// Service-level dataplane identity used by the Slice 04 readiness
    /// branch to compose the [`crate::traits::observation_store::ServiceBackendRow`]
    /// it writes when backend health changes. `None` for Services that
    /// have no VIP yet (no readiness write is possible — the branch
    /// is a no-op) or for the pre-Slice-04 no-alloc case.
    ///
    /// Sourced from the service's `ServiceVipAllocator` assignment +
    /// `ServiceSpec` identity (intent side); projected by the runtime's
    /// hydrate pass. Carries no derived state.
    pub service_dataplane: Option<ServiceDataplaneIdentity>,
}

/// Service-level dataplane identity for the readiness branch's
/// `ServiceBackendRow` composition. Separated from the per-alloc
/// [`ServiceAllocFact`] because it is one-per-Service, not
/// one-per-alloc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDataplaneIdentity {
    /// Identity of the service (LWW primary key for the backend row).
    pub service_id: ServiceId,
    /// Virtual IP the service's backends serve behind.
    pub vip: ServiceVip,
    /// Owner-writer node id stamped on the LWW `ServiceBackendRow`.
    /// Sourced from the local node identity (the runtime composes it
    /// at hydrate time, same as `BackendDiscoveryBridge`'s mandatory
    /// `writer_node_id`).
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

    /// Per-service fingerprint of the last `ServiceBackendRow` the
    /// readiness branch emitted. Compared against the freshly-computed
    /// fingerprint each tick; the branch emits
    /// `Action::WriteServiceBackendRow` only on drift (same dedup
    /// pattern as `BackendDiscoveryBridgeView::last_written_fingerprint`).
    #[serde(default)]
    pub last_emitted_backend_fingerprint: BTreeMap<ServiceId, BackendSetFingerprint>,
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

use crate::id::{ContentHash, CorrelationKey, NodeId};
use crate::reconcilers::workload_lifecycle::RESTART_BACKOFF_CEILING;
use crate::reconcilers::{Action, Reconciler, ReconcilerName, RestartReason, TickContext};
use crate::traits::dataplane::Backend;
use crate::traits::observation_store::{LogicalTimestamp, ServiceBackendRow};
use crate::transition_reason::TerminalCondition;
use crate::wall_clock::UnixInstant;

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

impl Reconciler for ServiceLifecycleReconciler {
    const NAME: &'static str = "service-lifecycle";
    type State = ServiceLifecycleState;
    type View = ServiceLifecycleView;

    fn name(&self) -> &ReconcilerName {
        &self.name
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
                alloc_id,
                fact.latest_startup_probe.as_ref(),
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
                // GAP-9 — record the non-Stable terminal (see EarlyExit
                // branch for the dedup + predicate-falseness rationale).
                next_view.terminal_announced.insert(alloc_id.clone());
            }
        }

        // ---- Step 03-01 / Slice 04 — readiness → Backend.healthy ----
        //
        // For every alloc that contributes to the service's backend
        // set, recompute `Backend.healthy` THIS TICK from the OBSERVED
        // readiness input + the live `success_threshold` + the
        // consecutive-Pass counter (the View INPUT). Never reads a
        // cached `healthy: bool` — there is none, per persist-inputs.
        //
        // The branch flips `healthy = false` when readiness fails
        // (drains the backend) — it NEVER emits `RestartAllocation`.
        // Restart is liveness (step 03-02); a readiness Fail only
        // removes the backend from rotation. The K3 no-restart-under-
        // readiness-flapping invariant rides on this branch emitting
        // nothing but `WriteServiceBackendRow`.
        if let Some(action) = readiness_backend_row_action(actual, &mut next_view, tick) {
            actions.push(action);
        }

        // ---- Step 03-02 / Slice 05 — liveness → RestartAllocation ----
        collect_liveness_actions(actual, &stable_this_tick, &mut next_view, &mut actions);

        (actions, next_view)
    }
}

/// Step 03-02 / Slice 05 — walk every alloc declaring a liveness probe,
/// maintain its consecutive-failure counter in the View, and emit
/// `RestartAllocation` or `FinalizeFailed { BackoffExhausted }` when the
/// trigger predicate holds. Extracted from `reconcile` to stay under the
/// clippy `too_many_lines` limit.
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
        if let Some(action) = liveness_restart_action(alloc_id, fact, next_view) {
            if matches!(action, Action::FinalizeFailed { .. }) {
                next_view.terminal_announced.insert(alloc_id.clone());
            }
            actions.push(action);
        }
    }
}

/// Step 03-02 / Slice 05 — maintain the per-(alloc, probe_idx)
/// liveness consecutive-failure counter (the View INPUT) and, when the
/// recomputed restart-trigger predicate holds this tick, emit the
/// matching terminal action.
///
/// Counter maintenance (mirrors the readiness consecutive-Pass shape,
/// inverted for failures):
/// - `Some(Fail)` → streak grows by one (saturating at `u32::MAX`).
/// - `Some(Pass)` → recovery: streak resets to 0 (entry removed;
///   absence == 0). Per S-SHCP-RECON-10 a Pass below threshold clears
///   the counter and emits NO restart.
/// - `None` → no liveness observation this tick; leave the counter
///   untouched.
///
/// Trigger predicate (recomputed every tick from the post-update
/// counter + the live `failure_threshold`, never persisted):
/// `state == Running AND consecutive_failures >= failure_threshold`.
/// When it holds, compose with the shared `RESTART_BACKOFF_CEILING`
/// budget:
/// - `restart_count < RESTART_BACKOFF_CEILING` → emit ONE
///   `RestartAllocation { reason: LivenessExhausted { .. } }`
///   (S-SHCP-RECON-09).
/// - `restart_count >= RESTART_BACKOFF_CEILING` → emit
///   `FinalizeFailed { ServiceFailed { LivenessProbeFailed {
///   probe_idx: 0, attempts: consecutive_failures } } }` so operators
///   can distinguish liveness-driven backoff from crash-loop backoff
///   (S-SHCP-RECON-11).
///
/// Returns `None` when the alloc declares no liveness probe, or when
/// the predicate does not hold (Running-but-below-threshold, recovery,
/// non-Running state).
fn liveness_restart_action(
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

    // Compose with the shared restart budget. Once the budget is spent
    // the liveness branch finalises with ServiceFailed { LivenessProbeFailed }
    // so operators can distinguish from crash-loop BackoffExhausted.
    if fact.restart_count >= RESTART_BACKOFF_CEILING {
        return Some(Action::FinalizeFailed {
            alloc_id: alloc_id.clone(),
            terminal: Some(TerminalCondition::ServiceFailed {
                reason: ServiceFailureReason::LivenessProbeFailed {
                    probe_idx: 0,
                    attempts: consecutive_failures,
                },
            }),
        });
    }

    // Reset the consecutive-failure counter so the post-restart alloc
    // starts with a clean slate. Without this, the `None` arm above
    // reads the stale threshold-exceeding value on the first Running
    // tick after restart (probes haven't fired yet), immediately
    // re-triggering RestartAllocation — one restart per tick until
    // BackoffExhausted.
    next_view.liveness_consecutive_failures.remove(&key);

    Some(Action::RestartAllocation {
        alloc_id: alloc_id.clone(),
        spec: fact.restart_spec.clone(),
        kind: crate::aggregate::WorkloadKind::Service,
        reason: Some(RestartReason::LivenessExhausted {
            probe_idx: 0,
            consecutive_failures,
            threshold: fact.liveness_failure_threshold,
        }),
    })
}

/// Step 03-01 / Slice 04 — recompute every backend's `healthy` flag
/// for the service THIS TICK and, when the service has a dataplane
/// identity AND at least one backend, emit a single
/// [`Action::WriteServiceBackendRow`] carrying the full backend set
/// **only when the backend set changed since the last emission**.
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
/// persisted INPUT). Returns `None` when the service has no dataplane
/// identity (no VIP → no row can be written), no allocs, or the
/// backend set is unchanged since the last emission (fingerprint
/// dedup — avoids unnecessary LWW gossip propagation every tick).
fn readiness_backend_row_action(
    actual: &ServiceLifecycleState,
    next_view: &mut ServiceLifecycleView,
    tick: &TickContext,
) -> Option<Action> {
    let dataplane = actual.service_dataplane.as_ref()?;
    if actual.allocs.is_empty() {
        return None;
    }

    let mut backends: Vec<Backend> = Vec::with_capacity(actual.allocs.len());
    for (alloc_id, fact) in &actual.allocs {
        if fact.state != AllocState::Running {
            continue;
        }
        let healthy = compute_backend_healthy(alloc_id, fact, next_view);
        backends.push(Backend {
            alloc: fact.backend_spiffe.clone(),
            addr: fact.backend_addr,
            weight: 1,
            healthy,
        });
    }

    if backends.is_empty() {
        return None;
    }

    let current_fp = fingerprint(&dataplane.vip, &backends);
    let prev_fp = next_view.last_emitted_backend_fingerprint.get(&dataplane.service_id).copied();
    if prev_fp == Some(current_fp) {
        return None;
    }
    next_view.last_emitted_backend_fingerprint.insert(dataplane.service_id, current_fp);

    let target = format!("service-lifecycle/readiness/{}", dataplane.service_id);
    let spec_hash = ContentHash::of(target.as_bytes());
    let correlation = CorrelationKey::derive(&target, &spec_hash, "readiness-backend-row");
    let vip = dataplane.vip.try_as_ipv4()?;

    Some(Action::WriteServiceBackendRow {
        row: ServiceBackendRow {
            service_id: dataplane.service_id,
            vip,
            backends,
            updated_at: LogicalTimestamp {
                counter: tick.tick.saturating_add(1),
                writer: dataplane.writer.clone(),
            },
        },
        correlation,
    })
}

/// Recompute one backend's `healthy` flag for the current tick and
/// update the persisted consecutive-Pass counter INPUT in
/// `next_view`. See [`readiness_backend_row_action`] for the contract.
fn compute_backend_healthy(
    alloc_id: &AllocationId,
    fact: &ServiceAllocFact,
    next_view: &mut ServiceLifecycleView,
) -> bool {
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
/// - `Some(Fail)` → increment by exactly 1 (saturating at `u32::MAX`).
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
    alloc_id: &AllocationId,
    latest_startup_probe: Option<&ProbeStatus>,
) {
    match latest_startup_probe {
        Some(ProbeStatus::Fail { .. }) => {
            let counter = counters.entry(alloc_id.clone()).or_insert(0);
            *counter = counter.saturating_add(1);
        }
        Some(ProbeStatus::Pass) => {
            counters.remove(alloc_id);
        }
        None => {}
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
