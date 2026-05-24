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

use crate::id::AllocationId;
use crate::observation::{ProbeIdx, ProbeStatus};
use crate::traits::observation_store::AllocState;

// Re-exports — see file-header docstring for the cycle-breaking
// rationale.
pub use crate::transition_reason::{ProbeWitness, ServiceFailureReason};

/// Per-alloc fact bundle the reconciler consults when deciding
/// `Stable` / `Failed` / no-op for a single Service-kind allocation.
///
/// Sourced by the runtime's hydrate-actual / hydrate-desired pass:
/// `state` + `started_at_unix_ms` + `exit_code` come from the
/// alloc-status row; `latest_startup_probe` is the LWW projection
/// of the per-`(alloc, probe_idx)` `ProbeResultRow`s for the
/// startup role.
///
/// `max_attempts` + `startup_deadline` + `mechanic_summary` come
/// from the live `ServiceSpec` (intent side) — re-evaluated every
/// tick per `.claude/rules/development.md` § "Persist inputs, not
/// derived state".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceAllocFact {
    /// Allocation identifier.
    pub alloc_id: AllocationId,
    /// Lifecycle state observed on the alloc-status row.
    pub state: AllocState,
    /// Wall-clock (UNIX-epoch ms) at which the alloc transitioned
    /// to Running. Required even for `Failed` allocs — used by
    /// EarlyExit's `elapsed_since_started_at < startup_deadline`
    /// gate per US-08.
    pub started_at_unix_ms: u64,
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

use crate::reconcilers::{Action, Reconciler, ReconcilerName, TickContext};
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

        for (alloc_id, fact) in &actual.allocs {
            if next_view.stable_announced.contains(alloc_id) {
                // S-SHCP-RECON-02: dedup — Stable already announced
                // for this alloc; emit nothing further. Falls
                // through to no-action.
                continue;
            }

            // Branch (a): Stable — Running + any startup probe Pass.
            if fact.state == AllocState::Running
                && matches!(fact.latest_startup_probe, Some(ProbeStatus::Pass))
            {
                let settled_in_ms = settled_in_ms_from(tick.now_unix, fact.started_at_unix_ms);
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
                continue;
            }

            // Branch (c): EarlyExit — alloc Failed within startup_deadline,
            // no Pass observed. Closes RCA-A per US-08.
            if fact.state == AllocState::Failed {
                let elapsed_ms = u64::try_from(tick.now_unix.as_unix_duration().as_millis())
                    .unwrap_or(u64::MAX)
                    .saturating_sub(fact.started_at_unix_ms);
                let deadline_ms =
                    u64::try_from(fact.startup_deadline.as_millis()).unwrap_or(u64::MAX);
                let within_deadline = elapsed_ms < deadline_ms;
                let no_pass = !matches!(fact.latest_startup_probe, Some(ProbeStatus::Pass));
                if within_deadline && no_pass {
                    actions.push(Action::FinalizeFailed {
                        alloc_id: alloc_id.clone(),
                        terminal: Some(TerminalCondition::ServiceFailed {
                            reason: ServiceFailureReason::EarlyExit {
                                exit_code: fact.exit_code.unwrap_or(0),
                            },
                        }),
                    });
                    continue;
                } // fall-through to StartupProbeFailed branch
            }

            // Branch (b): StartupProbeFailed — attempts exhausted AND
            // deadline elapsed AND no Pass observed.
            let attempts = next_view.startup_attempts_per_alloc.get(alloc_id).copied().unwrap_or(0);
            let elapsed_ms = u64::try_from(tick.now_unix.as_unix_duration().as_millis())
                .unwrap_or(u64::MAX)
                .saturating_sub(fact.started_at_unix_ms);
            let deadline_ms = u64::try_from(fact.startup_deadline.as_millis()).unwrap_or(u64::MAX);
            let no_pass = !matches!(fact.latest_startup_probe, Some(ProbeStatus::Pass));
            if attempts >= fact.max_attempts && elapsed_ms >= deadline_ms && no_pass {
                let last_fail = match &fact.latest_startup_probe {
                    Some(ProbeStatus::Fail { last_fail_reason }) => last_fail_reason.clone(),
                    _ => String::new(),
                };
                actions.push(Action::FinalizeFailed {
                    alloc_id: alloc_id.clone(),
                    terminal: Some(TerminalCondition::ServiceFailed {
                        reason: ServiceFailureReason::StartupProbeFailed {
                            probe_idx: 0,
                            last_fail,
                            attempts,
                        },
                    }),
                });
            }
        }

        (actions, next_view)
    }
}

#[inline]
#[must_use]
fn settled_in_ms_from(now: UnixInstant, started_at_unix_ms: u64) -> u64 {
    let now_ms = u64::try_from(now.as_unix_duration().as_millis()).unwrap_or(u64::MAX);
    now_ms.saturating_sub(started_at_unix_ms)
}
