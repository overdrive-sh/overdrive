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
//! RED scaffold — types declared; reconcile body lands in
//! `crates/overdrive-control-plane/src/reconcilers/service_lifecycle/`
//! across slices 01, 04, 05, 08.
// SCAFFOLD: true

#![allow(dead_code)]
#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::struct_field_names,
    reason = "DISTILL RED scaffold; per `.claude/rules/testing.md` § 'RED scaffolds' lints land when DELIVER replaces todo!() bodies + rewrites docs"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::id::AllocationId;
use crate::observation::{ProbeIdx, ProbeRole};

/// Reason a Service alloc transitioned to a terminal Failed state.
///
/// Per ADR-0055 §4 + ADR-0056 (wire projection): single
/// `#[non_exhaustive]` enum; additive variants only.
///
/// **Note: the existing `TerminalCondition::Failed { exit_code: i32 }`
/// variant in `crates/overdrive-core/src/transition_reason.rs`
/// (line 432) is the Job-kind shape from ADR-0037 Amendment
/// 2026-05-10. The Service-kind needs a NEW variant; DESIGN ADR-0055
/// proposes a Service-distinct variant
/// `TerminalCondition::ServiceFailed { reason: ServiceFailureReason }`
/// to avoid collision with the Job-kind shape. Naming the variant
/// `Failed { reason: ServiceFailureReason }` (per the user-stories /
/// slice briefs) would be a rkyv-discriminant collision; the actual
/// landed name in slice 01 will be `ServiceFailed` or similar. This
/// is an in-scope DELIVER-wave naming decision that the
/// `service_lifecycle` reconciler ties into.**
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ServiceFailureReason {
    /// Startup probe exhausted `max_attempts` without a Pass result
    /// within `startup_deadline`. `last_fail` carries the last
    /// observed `ProbeStatus::Fail.last_fail_reason` for direct
    /// operator-renderable surface.
    StartupProbeFailed { probe_idx: ProbeIdx, last_fail: String, attempts: u32 },
    /// Workload exited before any startup probe could pass AND
    /// within `startup_deadline` window. Closes RCA-A coinflip
    /// case per US-08.
    EarlyExit { exit_code: i32 },
    /// Liveness probe consecutive-failure count reached its
    /// threshold AND restart-budget reached
    /// `RESTART_BACKOFF_CEILING`. Composes with the existing
    /// `BackoffExhausted` JobLifecycle pathway for Service-kind
    /// liveness-driven restart attempts.
    BackoffExhausted { attempts: u32 },
}

/// Names which probe's Pass moved the reconciler to Stable.
///
/// Per DDD-7 (multi-probe AND-of-all semantic): when N startup
/// probes are declared, all must Pass; the witness names the
/// **last-to-Pass** probe. Renderer surfaces as `witness:
/// startup probe #<idx> (<mechanic_summary>)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeWitness {
    pub probe_idx: ProbeIdx,
    pub role: ProbeRole,
    /// Operator-facing summary (e.g. `"tcp 0.0.0.0:8080"`,
    /// `"http GET http://0.0.0.0:8080/healthz"`,
    /// `"exec /usr/local/bin/healthcheck.sh"`). Reconciler
    /// composes from `ProbeDescriptor.mechanic` at the deciding
    /// tick.
    pub mechanic_summary: String,
    /// `true` IFF this witness was the platform's inferred default
    /// probe per ADR-0058.
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
/// RED scaffold — full field tree lands in slice 01.
#[derive(Debug, Clone, Default)]
pub struct ServiceLifecycleState {
    // Full desired/actual decomposition lands in slice 01; this
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
///
/// RED scaffold — fields enumerated; serde derives land in
/// slice 01 (per ADR-0035/0036 CBOR ViewStore convention).
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
