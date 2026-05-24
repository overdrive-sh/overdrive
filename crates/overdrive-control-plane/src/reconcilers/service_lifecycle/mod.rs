//! `ServiceLifecycleReconciler` — Service-kind lifecycle reconciler
//! per ADR-0055.
//!
//! Pure sync `reconcile(desired, actual, view, tick) → (Vec<Action>,
//! View)` per `.claude/rules/development.md` § "Reconciler I/O".
//! No `.await`. No port dependencies. No wall-clock outside
//! `tick.now`.
//!
//! Per ADR-0055 §3 / DDD-5: `View` persists INPUTS only (counters,
//! sets). `Stable` predicate, readiness `healthy` gate, liveness
//! restart trigger, deadline computations — ALL recomputed every
//! tick against the live spec policy (per `.claude/rules/
//! development.md` § "Persist inputs, not derived state").
//!
//! ESR pairs (DST invariants land in DELIVER):
//! - `ServiceLifecycleStableIsDeduplicated` — once Stable announced
//!   for an alloc, no further Stable action is emitted.
//! - `ServiceLifecycleReadinessFlipsConverges` — readiness Pass →
//!   Fail flips `Backend.healthy` within 1 tick (K2).
//! - `ServiceLifecycleLivenessRestartTriggers` — N consecutive
//!   liveness Fail (N = `failure_threshold`) emits exactly one
//!   `RestartAllocation` (K3).
//!
//! RED scaffold — module declaration only. `reconcile` body lands
//! across slices 01 (Stable / EarlyExit / StartupProbeFailed),
//! 04 (readiness → Backend.healthy), 05 (liveness → restart),
//! 08 (EarlyExit hardening).
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

pub use overdrive_core::service_lifecycle::{
    DEFAULT_STARTUP_DEADLINE, ProbeWitness, ServiceFailureReason, ServiceLifecycleState,
    ServiceLifecycleView,
};

/// `ReconcilerName` constant for this reconciler. Wired into the
/// runtime registry per ADR-0035 / ADR-0036 by DELIVER.
pub const NAME: &str = "service-lifecycle";
