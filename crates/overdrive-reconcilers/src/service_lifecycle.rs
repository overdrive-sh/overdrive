//! `ServiceFailureReason` + `ProbeWitness` + `ServiceLifecycleState`
//! + `ServiceLifecycleView` — Service-kind reconciler types.
//!
//! Per ADR-0055 §4: `ServiceFailureReason` is a single per-kind
//! `#[non_exhaustive]` enum (NOT per-condition sub-enums; that would
//! fragment the operator-facing "why did my Service fail?" surface).
//! Additive variants per ADR-0037 §5.
//!
//! Per ADR-0055 §3 / DDD-5: `ServiceLifecycleView` carries
//! **inputs only** (sets and the liveness act-once row) — the `Stable` predicate, the
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
use overdrive_core::observation::{ProbeIdx, ProbeRunSeq, ProbeStatus};
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationRowKind};

// Re-exports — see file-header docstring for the cycle-breaking
// rationale.
pub use overdrive_core::transition_reason::{ProbeWitness, ServiceFailureReason};

/// A Service backend's identity: its SPIFFE id and the address it serves on.
/// The listener port stays with the enclosing dataplane identity when the
/// complete backend row is composed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceBackendIdentity {
    /// SPIFFE identity of the allocation as a dataplane backend.
    pub spiffe: SpiffeId,
    /// IPv4 workload address the allocation serves on.
    pub ip: std::net::Ipv4Addr,
}

/// An [`AllocState`] other than `Running`. The private field makes a
/// `Running` value unrepresentable: construct through [`NotRunningState::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotRunningState(AllocState);

impl NotRunningState {
    /// `None` iff `state` is `Running`.
    #[must_use]
    pub const fn new(state: AllocState) -> Option<Self> {
        match state {
            AllocState::Running => None,
            other => Some(Self(other)),
        }
    }

    /// The wrapped non-`Running` state.
    #[must_use]
    pub const fn get(self) -> AllocState {
        self.0
    }
}

/// A Running allocation's observation row carries no workload address, so no
/// backend address can be derived for it (ADR-0276).
#[derive(Debug, thiserror::Error)]
#[error(
    "Running Service allocation {alloc_id} carries no workload address; \
     a Running Service allocation must carry its workload address"
)]
pub struct RunningAllocationWithoutWorkloadAddr {
    /// The Running allocation whose row carries no workload address.
    pub alloc_id: AllocationId,
}

/// What a Service allocation is as a backend candidate: a `Running` allocation
/// always carries its [`ServiceBackendIdentity`]; any other state carries none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceAllocPhase {
    /// Running, with the identity the dataplane backend is built from.
    Running(ServiceBackendIdentity),
    /// Any state other than Running.
    NotRunning(NotRunningState),
}

impl ServiceAllocPhase {
    /// Project an observation row. Total over `AllocState x Option<Ipv4Addr>`:
    /// `Running` with an address is `Running(identity)`; `Running` without one
    /// is refused; every other state is `NotRunning` whatever the address.
    ///
    /// # Errors
    ///
    /// [`RunningAllocationWithoutWorkloadAddr`] when the row is `Running` and
    /// carries no `workload_addr`.
    pub fn from_row(
        row: &AllocStatusRow,
        spiffe: SpiffeId,
    ) -> Result<Self, RunningAllocationWithoutWorkloadAddr> {
        match (NotRunningState::new(row.state), row.workload_addr) {
            (Some(not_running), _) => Ok(Self::NotRunning(not_running)),
            (None, Some(ip)) => Ok(Self::Running(ServiceBackendIdentity { spiffe, ip })),
            (None, None) => {
                Err(RunningAllocationWithoutWorkloadAddr { alloc_id: row.alloc_id.clone() })
            }
        }
    }

    /// The lifecycle state this phase projects.
    #[must_use]
    pub const fn state(&self) -> AllocState {
        match self {
            Self::Running(_) => AllocState::Running,
            Self::NotRunning(not_running) => not_running.get(),
        }
    }
}

/// Per-alloc fact bundle the reconciler consults when deciding
/// `Stable` / `Failed` / no-op for a single Service-kind allocation.
///
/// Sourced by the runtime's hydrate-actual / hydrate-desired pass:
/// `state` + `started_at` come from the alloc-status row;
/// `latest_startup_probe` is the LWW projection
/// of the per-`(alloc, probe_idx)` `ProbeResultRow`s for the
/// startup role.
///
/// `startup_attempts_required` + `startup_deadline` + `mechanic_summary` come
/// from the live `ServiceSpec` (intent side) — re-evaluated every
/// tick per `.claude/rules/development.md` § "Persist inputs, not
/// derived state".
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "hydrate-boundary fact-bundle projection (one row's observed + spec-derived inputs), \
              not a domain entity — Object Calisthenics applies to the hexagonal core, not to \
              the runtime's per-alloc observation projection. Each bool names an independent \
              observed/spec fact (startup_probes_empty / has_readiness_probe / \
              has_liveness_probe); collapsing them into enums would obscure the projection."
)]
pub struct ServiceAllocFact {
    /// Allocation identifier.
    pub alloc_id: AllocationId,
    /// Lifecycle phase observed on the alloc-status row; `Running` carries the
    /// backend identity (ADR-0276).
    pub phase: ServiceAllocPhase,
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
    ///   that need a started-at timestamp (Stable and
    ///   StartupProbeFailed-elapsed) handle `None` explicitly per
    ///   the per-branch contract:
    ///   - Stable / opt-out Stable: `unreachable!()` — hydrate
    ///     invariant says a `Running` alloc carries
    ///     `Some(started_at)`.
    ///   - StartupProbeFailed: skip the branch — the
    ///     alloc never reached Running, so the elapsed-vs-deadline
    ///     classification doesn't apply. The row's typed `terminal`
    ///     flows through other projections (e.g., Custom → Other).
    /// - `Some(ts)`: the alloc reached Running at wall-clock `ts`;
    ///   used by Stable's `settled_in_ms = tick.now_unix - ts`
    ///   arithmetic and the startup-probe failure deadline.
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
    /// Latest-observed startup probe outcome at index 0. `None` if
    /// no probe result has yet been written for this alloc.
    pub latest_startup_probe: Option<ProbeStatus>,
    /// Legacy hydration stamp retained for source compatibility with existing
    /// fact fixtures. Probe-run counting reads the producer's typed
    /// `same_status_runs` from the hydrated row instead of this timestamp.
    pub latest_startup_probe_observed_at: Option<UnixInstant>,
    /// Startup-row streak required before `StartupProbeFailed` fires.
    /// TCP and HTTP use the descriptor's `max_attempts`; guest listeners
    /// use 1 because their first failure row is written at the deadline.
    pub startup_attempts_required: u32,
    /// Operator-spec-declared startup deadline window. Default per
    /// ADR-0057 §2 = 60s.
    pub startup_deadline: Duration,
    /// Operator-facing mechanic summary for the witnessing probe
    /// (e.g. `"tcp 0.0.0.0:8080"`). Reconciler composes
    /// `ProbeWitness.mechanic_summary` from this field at the
    /// deciding tick.
    pub mechanic_summary: String,
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
    /// is RECOMPUTED every tick from this status, the producer's
    /// `same_status_runs`, and the live `success_threshold`. It is never a
    /// cached `healthy: bool`.
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

    // ---- Step 03-02 / Slice 05 — liveness facts ----
    /// Latest-observed liveness probe outcome at index 0. `None` when
    /// no liveness `ProbeResultRow` has yet been written for this alloc
    /// (no liveness observation this tick — neither a failure nor a
    /// recovery; the latest stored row remains unchanged).
    ///
    /// Per `.claude/rules/development.md` § "Persist inputs, not
    /// derived state": this is the OBSERVED INPUT. The restart-trigger
    /// predicate is RECOMPUTED every tick from this status, the producer's
    /// `same_status_runs`, the live `liveness_failure_threshold`, and the
    /// act-once row sequence. It is never a cached `should_restart: bool`.
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

    /// Latest raw probe rows for this allocation, keyed by role and
    /// per-role probe index. Hydration builds this transient input from the
    /// same allocation-scoped observation snapshot as the retained status
    /// projections (PROBE-RUN-COUNT).
    #[doc(hidden)]
    pub probe_rows: BTreeMap<(ProbeRole, ProbeIdx), ProbeResultRow>,
    /// The allocation's status row carries a committed `Stable` claim.
    pub stable_committed: bool,
}

impl ServiceAllocFact {
    /// Lifecycle state of the allocation (delegates to the phase).
    #[must_use]
    pub const fn state(&self) -> AllocState {
        self.phase.state()
    }

    /// The backend identity; `Some` iff the allocation is Running.
    #[must_use]
    pub const fn backend(&self) -> Option<&ServiceBackendIdentity> {
        match &self.phase {
            ServiceAllocPhase::Running(identity) => Some(identity),
            ServiceAllocPhase::NotRunning(_) => None,
        }
    }
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
    /// GAP-9 — per-alloc set of allocs the reconciler has OBSERVED in
    /// a non-terminal state (i.e. it has begun watching the alloc's
    /// startup window but has not yet observed a committed Stable or
    /// startup-stop terminal).
    ///
    /// This is the load-bearing input for the reconciler's pure
    /// `next_evaluation_at` hook: during the active startup window the
    /// reconciler emits ZERO actions (no Pass yet, not failed, deadline not
    /// elapsed), so the hook uses this membership to retain the pending key
    /// until the window boundary or an external observation wakes it.
    ///
    /// Per `.claude/rules/development.md` § "Persist inputs, not
    /// derived state": this records an OBSERVED FACT ("the reconciler
    /// is watching alloc X"), not a derived "needs re-enqueue now"
    /// boolean — the predicate recomputes that from the set
    /// difference against `terminal_announced` and the hydrated committed
    /// `Stable` claim every read.
    pub observed: BTreeSet<AllocationId>,

    /// Per-alloc set of allocations this reconciler has finished watching:
    /// every observed `Failed` or `Terminated` allocation, and every live
    /// allocation for which it emitted a startup stop. A committed Stable
    /// is read from the allocation status row instead.
    ///
    /// Two jointly load-bearing roles:
    ///
    /// 1. **Hook falseness at terminal** — the
    ///    `next_evaluation_at` hook filters [`Self::observed`] against this
    ///    set and the hydrated committed `Stable` claim; once an allocation
    ///    lands in either terminal state the hook returns no boundary.
    /// 2. **Startup-stop/liveness gating** — a live allocation stopped for a
    ///    startup failure remains excluded from liveness until the stop lands
    ///    or a later startup `Pass` announces `Stable`.
    ///
    /// Records observed lifecycle membership, never a derived flag. A terminal
    /// row joins this set at the head of its per-allocation reconcile step,
    /// whatever its `started_at` value; the reconciler emits no action for it.
    pub terminal_announced: BTreeSet<AllocationId>,

    /// Per liveness probe key, the `run_seq` of the probe row on which this
    /// reconciler last emitted a liveness terminate (PROBE-RUN-COUNT,
    /// ADR-0191, act once per crossing: a terminate is emitted only on a
    /// newer row). An input, not a derived value; written only when a
    /// terminate is emitted. Absent ⇒ never terminated on this key.
    #[serde(default)]
    pub liveness_last_acted_run: BTreeMap<(AllocationId, ProbeIdx), ProbeRunSeq>,

    /// The `run_seq` of the startup probe row on which this reconciler last
    /// emitted a startup stop, keyed `(alloc, ProbeIdx::new(0))`. An input, not a
    /// derived value; written only when a stop is emitted; absent means never
    /// acted.
    #[serde(default)]
    pub startup_last_acted_run: BTreeMap<(AllocationId, ProbeIdx), ProbeRunSeq>,
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
use overdrive_core::aggregate::{IntentKey, Listener, Service, WorkloadIntent};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{ContentHash, CorrelationKey, NodeId, WorkloadId};
use overdrive_core::observation::{ProbeResultRow, ProbeRole};
use overdrive_core::reconcilers::{
    Action, HydrateError, HydrationContext, Reconciler, ReconcilerName, TargetResource, TickContext,
};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{LogicalTimestamp, ServiceBackendRow};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
use overdrive_core::wall_clock::UnixInstant;

/// Service-kind lifecycle reconciler per ADR-0055.
///
/// Pure-sync `reconcile(desired, actual, view, tick) → (Vec<Action>,
/// View)` per `.claude/rules/development.md` § "Reconciler I/O" —
/// no `.await`, no port dependencies, no wall-clock outside
/// `tick.now_unix`.
///
/// The reconcile body covers Stable and startup-probe failure handling.
/// Slice 04 (readiness → Backend.healthy) and
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

    /// Row-backed: `service-lifecycle` converges `Stable`, startup-probe
    /// handling and terminal membership against the `AllocStatusRow` set, so
    /// an accepted `alloc_status` transition must wake it (ADR-0084 §5
    /// single-cut migration — the declarative
    /// replacement for the deleted `exit_observer` producer-push). An accepted
    /// `ProbeResult` also wakes it so readiness health converges immediately;
    /// the router validates the current Service allocation before deriving the
    /// workload target (ADR-0101 amendment E11). It authors no `alloc_status`
    /// rows (it writes `healthy` on `service_backends`), so the interest is
    /// loop-free (ADR-0079).
    fn interests(&self) -> &'static [ObservationRowKind] {
        &[ObservationRowKind::AllocStatus, ObservationRowKind::ProbeResult]
    }

    // ServiceLifecycle's reconcile is one cohesive transition pipeline: it
    // updates per-allocation startup-stop memory, emits the backend projection,
    // then collects liveness actions.
    // Splitting those transitions solely for the line-count lint would thread
    // mutable view/action state through helpers and obscure that ordering.
    #[allow(clippy::too_many_lines)]
    fn reconcile(
        &self,
        _desired: &Self::State,
        actual: &Self::State,
        view: &Self::View,
        tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        let mut actions: Vec<Action> = Vec::new();
        let mut next_view = view.clone();

        for (alloc_id, fact) in &actual.allocs {
            // A terminal row ends this reconciler's watch. This is before
            // every other per-allocation branch, including committed Stable.
            if matches!(fact.state(), AllocState::Failed | AllocState::Terminated) {
                next_view.terminal_announced.insert(alloc_id.clone());
                continue;
            }

            if fact.stable_committed {
                // B1: the committed allocation row is authoritative for
                // Stable. No View marker is needed to deduplicate emission.
                continue;
            }

            // A live allocation in `terminal_announced` still needs the D8
            // startup-stop rule evaluated against each new probe row. Keep the
            // existing dedup for non-Running facts; terminal rows were handled
            // at the head of this loop above.
            if next_view.terminal_announced.contains(alloc_id)
                && fact.state() != AllocState::Running
            {
                continue;
            }

            // GAP-9 Shape B — record that the reconciler is watching
            // this alloc's startup window. During
            // the active window the branches below emit no action, so this
            // membership supplies the retained scheduling boundary. The
            // hook reads the observed membership together with the hydrated
            // committed Stable claim and terminal startup-stop set, so a
            // completed alloc does NOT keep the runtime spinning.
            next_view.observed.insert(alloc_id.clone());

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
            if fact.startup_probes_empty && fact.state() == AllocState::Running {
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
                continue;
            }

            // Branch (a): Stable — Running + any startup probe Pass.
            // Same hydrate invariant as branch (a'): a Running alloc
            // carries `Some(started_at)`.
            if fact.state() == AllocState::Running
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
                    inferred: false,
                };
                actions.push(Action::FinalizeFailed {
                    alloc_id: alloc_id.clone(),
                    terminal: Some(TerminalCondition::Stable { settled_in_ms, witness }),
                });
                next_view.terminal_announced.remove(alloc_id);
                continue;
            }

            // Branch (b): stop this startup attempt when its producer-recorded
            // streak is exhausted, the deadline elapsed, and no Pass was
            // observed. A missing row consumes no attempt.
            let startup_row_key = (alloc_id.clone(), ProbeIdx::new(0));
            let latest_startup_row = fact.probe_rows.get(&(ProbeRole::Startup, ProbeIdx::new(0)));
            let newer_than_last_startup_stop = latest_startup_row.is_some_and(|row| {
                next_view
                    .startup_last_acted_run
                    .get(&startup_row_key)
                    .is_none_or(|last| row.run_seq > *last)
            });
            let attempts = latest_startup_row
                .filter(|row| matches!(&row.status, ProbeStatus::Fail { .. }))
                .map_or(0, |row| row.same_status_runs.get());
            if newer_than_last_startup_stop
                && let Some(action) =
                    startup_probe_failed_action(alloc_id, fact, attempts, tick.now_unix)
            {
                actions.push(action);
                if let Some(row) = latest_startup_row {
                    next_view.startup_last_acted_run.insert(startup_row_key, row.run_seq);
                }
                // Record that this reconciler has finished watching the
                // allocation after emitting its startup stop.
                next_view.terminal_announced.insert(alloc_id.clone());
            }
        }

        // ---- ADR-0101 — authoritative ServiceBackendRow projection ----
        //
        // ServiceLifecycle owns the complete row: current Running
        // membership, listener address materialization, the committed Stable
        // startup gate, and the readiness rule are composed before comparing
        // with the observed row. A startup stop remains before this projection.
        actions.extend(service_backend_row_actions(actual, tick));

        // ---- Step 03-02 / Slice 05 — liveness → RestartAllocation ----
        collect_liveness_actions(actual, &mut next_view, &mut actions);

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

    /// Return the earliest future startup-window boundary still owned by this
    /// hydrated ServiceLifecycle evaluation.
    fn next_evaluation_at(
        &self,
        _desired: &Self::State,
        actual: &Self::State,
        next_view: &Self::View,
        tick: &TickContext,
    ) -> Option<UnixInstant> {
        actual
            .allocs
            .values()
            .filter_map(|fact| {
                if !next_view.observed.contains(&fact.alloc_id)
                    || fact.stable_committed
                    || next_view.terminal_announced.contains(&fact.alloc_id)
                    || fact.started_at.is_none()
                    || fact.state() == AllocState::Terminated
                    || matches!(fact.latest_startup_probe, Some(ProbeStatus::Pass))
                {
                    return None;
                }
                let startup_deadline = Duration::from_millis(
                    u64::try_from(fact.startup_deadline.as_millis()).unwrap_or(u64::MAX),
                );
                let end = fact.started_at? + startup_deadline;
                (end > tick.now_unix).then_some(end)
            })
            .min()
    }
}

// ---------------------------------------------------------------------------
// Hydration bodies — moved off the central `reconciler_runtime` free fns
// (ADR-0086 S3).
// ---------------------------------------------------------------------------

/// Liveness probe `failure_threshold` default per ADR-0057 §2 / DDD-14.
const LIVENESS_FAILURE_THRESHOLD_DEFAULT: u32 = 3;

/// Read `WorkloadIntent::Service(Service)` for `workload_id`; `Ok(None)` when
/// absent or a `Job` / `Schedule` variant.
async fn service_spec_from_intent(
    ctx: &HydrationContext<'_>,
    workload_id: &WorkloadId,
) -> Result<Option<Service>, HydrateError> {
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
///
/// `listeners` is the Service's declared listener set in declaration order (D14
/// item 4, DG-11); only the `GuestListeners` arm uses it.
fn format_mechanic_summary(mechanic: &ProbeMechanic, listeners: &[Listener]) -> String {
    match mechanic {
        ProbeMechanic::Tcp { host, port } => format!("tcp {host}:{port}"),
        ProbeMechanic::Http { path, port, host } => host
            .as_ref()
            .map_or_else(|| format!("http {path}"), |h| format!("http {h}:{port}{path}")),
        ProbeMechanic::GuestListeners => {
            let names = listeners
                .iter()
                .map(|listener| {
                    let protocol = match listener.protocol {
                        Proto::Tcp => "tcp",
                        Proto::Udp => "udp",
                    };
                    format!("{protocol}:{}", listener.port)
                })
                .collect::<Vec<_>>()
                .join(" ");
            if names.is_empty() {
                "guest listeners".to_owned()
            } else {
                format!("guest listeners {names}")
            }
        }
    }
}

/// Project the spec-derived startup facts uniform across every alloc.
fn spec_facts_for_service(svc: &Service) -> (u32, Duration, String, bool) {
    let startup_probes_empty = svc.startup_probes.is_empty();
    if startup_probes_empty {
        return (30, startup_deadline(&svc.startup_probes), String::new(), true);
    }
    let probe = &svc.startup_probes[0];
    let startup_attempts_required = match &probe.mechanic {
        ProbeMechanic::GuestListeners => 1,
        ProbeMechanic::Tcp { .. } | ProbeMechanic::Http { .. } => probe.max_attempts,
    };
    let startup_deadline = startup_deadline(&svc.startup_probes);
    let mechanic_summary = format_mechanic_summary(&probe.mechanic, &svc.listeners);
    (startup_attempts_required, startup_deadline, mechanic_summary, false)
}

/// The startup deadline of a Service's startup probes: `interval x max_attempts`
/// of startup probe index 0, or [`DEFAULT_STARTUP_DEADLINE`] for an empty list or
/// on overflow (D10 item 5: extracted from `spec_facts_for_service`).
// SCAFFOLD: true — pinned shape (feature service-allocation-readiness-and-ending, D10 item 5)
#[must_use]
pub fn startup_deadline(
    startup_probes: &[overdrive_core::aggregate::probe_descriptor::ProbeDescriptor],
) -> Duration {
    let Some(probe) = startup_probes.first() else {
        return DEFAULT_STARTUP_DEADLINE;
    };
    Duration::from_secs(u64::from(probe.interval_seconds))
        .checked_mul(probe.max_attempts)
        .unwrap_or(DEFAULT_STARTUP_DEADLINE)
}

/// Project the readiness facts uniform across every alloc.
fn readiness_facts_for_service(svc: &Service) -> (bool, u32) {
    let has_readiness_probe = !svc.readiness_probes.is_empty();
    let success_threshold =
        svc.readiness_probes.first().and_then(|p| p.success_threshold).unwrap_or(1);
    (has_readiness_probe, success_threshold)
}

/// Project the liveness facts uniform across every alloc.
fn liveness_facts_for_service(svc: &Service) -> (bool, u32) {
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
    svc: &Service,
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
                vip: assigned_vip,
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
    rows.iter().filter(|p| p.role == role && p.probe_idx == probe_idx).max_by_key(|p| p.run_seq)
}

/// Per-workload projection of every `AllocStatusRow` into `ServiceAllocFact`,
/// joining each row with its LWW probe projections and the spec-derived facts.
async fn hydrate_service_alloc_facts(
    ctx: &HydrationContext<'_>,
    workload_id: &WorkloadId,
    spec_facts: &(u32, Duration, String, bool),
    readiness_facts: &(bool, u32),
    liveness_facts: &(bool, u32),
) -> Result<BTreeMap<AllocationId, ServiceAllocFact>, HydrateError> {
    let (startup_attempts_required, startup_deadline, mechanic_summary, startup_probes_empty) =
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
        let probe_rows_by_key = probe_rows
            .iter()
            .map(|probe_row| ((probe_row.role, probe_row.probe_idx), probe_row.clone()))
            .collect();
        let latest_startup_probe =
            latest_probe_row(&probe_rows, ProbeRole::Startup, ProbeIdx::new(0));
        let latest_readiness_probe =
            latest_probe_row(&probe_rows, ProbeRole::Readiness, ProbeIdx::new(0));
        let latest_liveness_probe =
            latest_probe_row(&probe_rows, ProbeRole::Liveness, ProbeIdx::new(0));

        let phase =
            ServiceAllocPhase::from_row(&row, SpiffeId::for_allocation(workload_id, &row.alloc_id))
                .map_err(|refused| HydrateError::RunningAllocationWithoutWorkloadAddr {
                    alloc_id: refused.alloc_id,
                })?;

        let fact = ServiceAllocFact {
            stable_committed: matches!(
                row.terminal.as_ref(),
                Some(TerminalCondition::Stable { .. })
            ),
            alloc_id: row.alloc_id.clone(),
            phase,
            started_at: row.started_at,
            latest_startup_probe: latest_startup_probe.map(|row| row.status.clone()),
            latest_startup_probe_observed_at: latest_startup_probe.map(|row| {
                UnixInstant::from_unix_duration(Duration::from_millis(row.last_observed_at_unix_ms))
            }),
            startup_attempts_required: *startup_attempts_required,
            startup_deadline: *startup_deadline,
            mechanic_summary: mechanic_summary.clone(),
            startup_probes_empty: *startup_probes_empty,
            latest_readiness_probe: latest_readiness_probe.map(|row| row.status.clone()),
            has_readiness_probe,
            readiness_success_threshold,
            latest_liveness_probe: latest_liveness_probe.map(|row| row.status.clone()),
            has_liveness_probe,
            liveness_failure_threshold,
            probe_rows: probe_rows_by_key,
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

/// ADR-0087 D2 / ADR-0191 — walk every alloc declaring a liveness probe,
/// apply the producer-recorded streak, and emit a liveness
/// TERMINATE (`StopAllocation { terminal: Stopped { by: LivenessProbe } }`)
/// when the trigger predicate holds. `ServiceLifecycle` makes NO
/// restart-vs-finalize decision and reads NO budget — the termination IS
/// the signal, and `WorkloadLifecycle` (the sole restart authority)
/// restarts the liveness-terminated row under its single budget.
/// Extracted from `reconcile` to stay under the clippy `too_many_lines`
/// limit.
fn collect_liveness_actions(
    actual: &ServiceLifecycleState,
    next_view: &mut ServiceLifecycleView,
    actions: &mut Vec<Action>,
) {
    for (alloc_id, fact) in &actual.allocs {
        if next_view.terminal_announced.contains(alloc_id) {
            continue;
        }
        let latest_row = fact.probe_rows.get(&(ProbeRole::Liveness, ProbeIdx::new(0)));
        if let Some(action) = liveness_terminate_action(alloc_id, fact, latest_row, next_view) {
            actions.push(action);
        }
    }
}

/// ADR-0087 D2 / ADR-0191 — apply the producer-recorded liveness streak at
/// read time. A crossed row emits at most once, keyed by its `run_seq`; a
/// failed stop is retried on a newer crossed row while the allocation remains
/// Running. `ServiceLifecycle` remains the detector and `WorkloadLifecycle`
/// remains the restart authority.
fn liveness_terminate_action(
    alloc_id: &AllocationId,
    fact: &ServiceAllocFact,
    latest_row: Option<&ProbeResultRow>,
    next_view: &mut ServiceLifecycleView,
) -> Option<Action> {
    if !fact.has_liveness_probe {
        return None;
    }
    let row = latest_row?;
    let crossed = matches!(&row.status, ProbeStatus::Fail { .. })
        && row.same_status_runs.get() >= fact.liveness_failure_threshold;
    let key = (alloc_id.clone(), ProbeIdx::new(0));
    let newer_than_last_action =
        next_view.liveness_last_acted_run.get(&key).is_none_or(|last| row.run_seq > *last);
    if fact.state() != AllocState::Running || !crossed || !newer_than_last_action {
        return None;
    }

    next_view.liveness_last_acted_run.insert(key, row.run_seq);

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
/// - A startup-probed allocation without its own committed Stable is
///   ineligible.
/// - Otherwise, no readiness probe → `healthy = true`; a declared readiness
///   probe uses its latest result and producer streak against the threshold.
///   Initial state (no Pass row yet) → `healthy = false` (S-SHCP-RECON-08c —
///   avoids the inverse race). The explicit startup opt-out is eligible at
///   Running, as its first Running row is its Stable.
///
/// The threshold is read from the latest producer row on every reconcile.
fn service_backend_row_actions(actual: &ServiceLifecycleState, tick: &TickContext) -> Vec<Action> {
    if actual.service_dataplane.is_empty() {
        return Vec::new();
    }

    // Readiness is an allocation-level policy. Compute it once per current
    // Running allocation, then reuse the result for every listener so adding
    // listeners never multiplies the producer-row read.
    let mut healthy_by_alloc: BTreeMap<AllocationId, bool> = BTreeMap::new();
    for (alloc_id, fact) in &actual.allocs {
        if fact.state() != AllocState::Running {
            continue;
        }
        let healthy = compute_backend_healthy(fact);
        healthy_by_alloc.insert(alloc_id.clone(), healthy);
    }

    let mut actions = Vec::new();
    for (service_id, identity) in &actual.service_dataplane {
        let backends: Vec<Backend> = actual
            .allocs
            .iter()
            .filter_map(|(alloc_id, fact)| {
                fact.backend().map(|backend| Backend {
                    alloc: backend.spiffe.clone(),
                    addr: SocketAddr::new(IpAddr::V4(backend.ip), identity.port.get()),
                    weight: 1,
                    healthy: healthy_by_alloc.get(alloc_id).copied().unwrap_or(false),
                })
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

/// Recompute one backend's `healthy` flag from the hydrated allocation fact.
/// See [`service_backend_row_actions`] for the row projection contract.
fn compute_backend_healthy(fact: &ServiceAllocFact) -> bool {
    if !fact.startup_probes_empty && !fact.stable_committed {
        return false;
    }
    if !fact.has_readiness_probe {
        // Backward-compat default: no readiness gate → always healthy.
        return true;
    }

    let Some(row) = fact.probe_rows.get(&(ProbeRole::Readiness, ProbeIdx::new(0))) else {
        return false;
    };
    matches!(&row.status, ProbeStatus::Pass)
        && row.same_status_runs.get() >= fact.readiness_success_threshold
}

/// Branch (b) — startup-probe stop action, or `None` when the three-gate
/// condition is not met.
///
/// The terminal CONDITION is mechanic-specific: `attempts >= startup_attempts_required && elapsed_ms >=
/// deadline_ms && no_pass`. The `attempts` argument is the producer-recorded
/// consecutive-fail count from the latest startup row.
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
    // D5: a row that already carries an ending cannot receive a startup
    // verdict. `is_terminal` excludes both `Failed` and `Terminated`; Pending
    // remains eligible because the existing branch contract exercises the
    // three startup gates before the allocation reaches Running.
    if fact.state().is_terminal() {
        return None;
    }
    let started = fact.started_at?;
    let elapsed_ms = elapsed_ms_from(now_unix, started);
    let deadline_ms = u64::try_from(fact.startup_deadline.as_millis()).unwrap_or(u64::MAX);
    let no_pass = !matches!(fact.latest_startup_probe, Some(ProbeStatus::Pass));
    if attempts >= fact.startup_attempts_required && elapsed_ms >= deadline_ms && no_pass {
        Some(Action::StopAllocation {
            alloc_id: alloc_id.clone(),
            terminal: Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }),
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
/// [`settled_in_ms_from`] but named for the startup-probe failure deadline
/// that reads it as `elapsed_ms`.
#[inline]
#[must_use]
fn elapsed_ms_from(now: UnixInstant, started_at: UnixInstant) -> u64 {
    u64::try_from((now - started_at).as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use overdrive_core::observation::SameStatusRuns;
    use proptest::prelude::*;
    use std::num::NonZeroU16;
    use std::time::Instant;

    /// D14 item 4 (DG-11): the `GuestListeners` summary names the declared listeners in
    /// declaration order, `tcp:<port>` or `udp:<port>` joined by single spaces.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn guest_listeners_summary_names_the_declared_listeners_in_declaration_order() {
        let declared = [
            Listener { port: NonZeroU16::new(8080).expect("non-zero"), protocol: Proto::Tcp },
            Listener { port: NonZeroU16::new(5353).expect("non-zero"), protocol: Proto::Udp },
        ];
        assert_eq!(
            format_mechanic_summary(&ProbeMechanic::GuestListeners, &declared),
            "guest listeners tcp:8080 udp:5353"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 128,
            rng_seed: proptest::test_runner::RngSeed::Fixed(257_222),
            ..ProptestConfig::default()
        })]

        /// CONTRACT_SHAPE: pure-function.
        /// D12/R2-1 backend policy through the public reconciler: a startup-
        /// probed allocation is eligible only after its own committed Stable;
        /// opt-out keeps the prior readiness rule. Startup row and terminal-
        /// membership inputs do not change that policy, and unrelated view
        /// membership survives the projection.
        #[test]
        fn backend_policy_uses_committed_stable_and_readiness_without_a_veto(
            same_status_runs in prop_oneof![Just(1), Just(u32::MAX), 1_u32..=u32::MAX],
            threshold in prop_oneof![Just(1), Just(u32::MAX), 1_u32..=u32::MAX],
        ) {
            let alloc = AllocationId::new("policy-property").unwrap();
            let unrelated = AllocationId::new("unrelated-policy").unwrap();
            let service_id = ServiceId::new(42).unwrap();
            let identity = ServiceDataplaneIdentity {
                port: NonZeroU16::new(8080).unwrap(),
                protocol: Proto::Tcp,
                vip: ServiceVip::new("10.0.0.42".parse().unwrap()).unwrap(),
                writer: NodeId::new("policy-writer").unwrap(),
            };
            let tick = TickContext {
                now: Instant::now(),
                now_unix: UnixInstant::from_unix_duration(Duration::from_secs(2)),
                tick: 2,
                deadline: Instant::now() + Duration::from_secs(1),
            };
            let startup_rows = [
                None,
                Some(ProbeStatus::Pass),
                Some(ProbeStatus::Fail { last_fail_reason: "startup refused".into() }),
            ];
            let readiness_rows = [
                None,
                Some(ProbeStatus::Pass),
                Some(ProbeStatus::Fail { last_fail_reason: "readiness refused".into() }),
            ];
            for opt_out in [false, true] {
                for committed in [false, true] {
                    for enabled in [false, true] {
                        for own_announced in [false, true] {
                            for unrelated_announced in [false, true] {
                                for startup in &startup_rows {
                                    for readiness in &readiness_rows {
                                        let mut fact = ServiceAllocFact {
                                            stable_committed: committed,
                                            alloc_id: alloc.clone(),
                                            phase: ServiceAllocPhase::Running(ServiceBackendIdentity {
                                                spiffe: SpiffeId::new("spiffe://overdrive.local/workload/policy/alloc/0").unwrap(),
                                                ip: "192.0.2.10".parse().unwrap(),
                                            }),
                                            started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
                                            latest_startup_probe: startup.clone(),
                                            latest_startup_probe_observed_at: None,
                                            startup_attempts_required: 3,
                                            startup_deadline: Duration::from_secs(30),
                                            mechanic_summary: "tcp 0.0.0.0:8080".into(),
                                            startup_probes_empty: opt_out,
                                            latest_readiness_probe: readiness.clone(),
                                            has_readiness_probe: enabled,
                                            readiness_success_threshold: threshold,
                                            latest_liveness_probe: None,
                                            has_liveness_probe: false,
                                            liveness_failure_threshold: 3,
                                            probe_rows: BTreeMap::new(),
                                        };
                                        let probe_idx = ProbeIdx::new(0);
                                        if let Some(status) = readiness.clone() {
                                            fact.probe_rows.insert(
                                                (ProbeRole::Readiness, probe_idx),
                                                ProbeResultRow {
                                                    alloc_id: alloc.clone(),
                                                    probe_idx,
                                                    role: ProbeRole::Readiness,
                                                    status,
                                                    last_observed_at_unix_ms: 1_000,
                                                    inferred: false,
                                                    run_seq: ProbeRunSeq::FIRST,
                                                    same_status_runs: SameStatusRuns::new(same_status_runs)
                                                        .expect("generated streak is non-zero"),
                                                },
                                            );
                                        }
                                        let state = ServiceLifecycleState {
                                            allocs: BTreeMap::from([(alloc.clone(), fact)]),
                                            service_dataplane: BTreeMap::from([(service_id, identity.clone())]),
                                            observed_backend_rows: BTreeMap::new(),
                                        };
                                        let mut view = ServiceLifecycleView::default();
                                        view.observed.insert(alloc.clone());
                                        if own_announced {
                                            view.terminal_announced.insert(alloc.clone());
                                        }
                                        if unrelated_announced {
                                            view.terminal_announced.insert(unrelated.clone());
                                        }
                                        let (actions, next_view) = ServiceLifecycleReconciler::new()
                                            .reconcile(&state, &state, &view, &tick);
                                        let row = actions.iter().find_map(|action| match action {
                                            Action::WriteServiceBackendRow { row, .. } => Some(row),
                                            _ => None,
                                        }).expect("backend projection is written from an empty observation");
                                        let readiness_passed = matches!(readiness, Some(ProbeStatus::Pass));
                                        let expected_healthy = (opt_out || committed)
                                            && (!enabled || (readiness_passed && same_status_runs >= threshold));
                                        prop_assert_eq!(row.backends.len(), 1);
                                        prop_assert_eq!(row.backends[0].healthy, expected_healthy);
                                        prop_assert_eq!(next_view.observed, view.observed);
                                        prop_assert_eq!(next_view.liveness_last_acted_run, view.liveness_last_acted_run);
                                        prop_assert_eq!(next_view.startup_last_acted_run, view.startup_last_acted_run);
                                        prop_assert_eq!(
                                            next_view.terminal_announced.contains(&unrelated),
                                            unrelated_announced,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn startup_window_hook_returns_the_earliest_future_window_end() {
        let alloc_id = AllocationId::new("startup-window-hook").expect("valid alloc id");
        let fact = ServiceAllocFact {
            stable_committed: false,
            alloc_id: alloc_id.clone(),
            phase: ServiceAllocPhase::Running(ServiceBackendIdentity {
                spiffe: SpiffeId::new(
                    "spiffe://overdrive.local/workload/startup-hook/alloc/startup-window-hook",
                )
                .expect("valid spiffe id"),
                ip: "192.0.2.10".parse().expect("valid IPv4"),
            }),
            started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(100))),
            latest_startup_probe: None,
            latest_startup_probe_observed_at: None,
            startup_attempts_required: 3,
            startup_deadline: Duration::from_secs(60),
            mechanic_summary: "tcp 0.0.0.0:8080".to_owned(),
            startup_probes_empty: false,
            latest_readiness_probe: None,
            has_readiness_probe: false,
            readiness_success_threshold: 1,
            latest_liveness_probe: None,
            has_liveness_probe: false,
            liveness_failure_threshold: 3,
            probe_rows: BTreeMap::new(),
        };
        let actual = ServiceLifecycleState {
            allocs: BTreeMap::from([(alloc_id.clone(), fact)]),
            ..ServiceLifecycleState::default()
        };
        let view =
            ServiceLifecycleView { observed: BTreeSet::from([alloc_id]), ..Default::default() };
        let now_unix = UnixInstant::from_unix_duration(Duration::from_secs(100));
        let tick = TickContext {
            now: Instant::now(),
            now_unix,
            tick: 0,
            deadline: Instant::now() + Duration::from_secs(1),
        };

        assert_eq!(
            ServiceLifecycleReconciler::new().next_evaluation_at(
                &ServiceLifecycleState::default(),
                &actual,
                &view,
                &tick,
            ),
            Some(UnixInstant::from_unix_duration(Duration::from_secs(160))),
        );
    }
}
