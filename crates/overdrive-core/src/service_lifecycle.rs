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
use crate::observation::ProbeIdx;

// Re-exports — see file-header docstring for the cycle-breaking
// rationale.
pub use crate::transition_reason::{ProbeWitness, ServiceFailureReason};

/// `ServiceLifecycleState` — typed projection of intent +
/// observation for the Service reconciler per ADR-0055 §2 +
/// ADR-0021/0036.
///
/// `desired` is sourced from `ServiceSpec` (intent). `actual` is
/// sourced from `alloc_status` rows + `probe_result` rows per
/// alloc.
///
/// RED scaffold — full field tree lands in slice 01-03+.
#[derive(Debug, Clone, Default)]
pub struct ServiceLifecycleState {
    // Full desired/actual decomposition lands in slice 01-03+; this
    // scaffold preserves the trait surface so AnyReconciler /
    // AnyState match arms compile.
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
