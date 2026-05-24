//! Tier 1 acceptance — liveness probe → `RestartAllocation`.
//!
//! Slice 05 (US-05). RED scaffolds.
//!
//! KPI K3: liveness probe N consecutive fails (N =
//! failure_threshold) → `Action::RestartAllocation { reason:
//! LivenessExhausted { probe_idx, consecutive_failures, threshold }
//! }` emitted within 1 reconciler tick.

#![allow(clippy::expect_used, clippy::unwrap_used)]
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

/// S-SHCP-RECON-09 (US-05 / K3) — liveness probe fails 3
/// consecutively → `Action::RestartAllocation` emitted with
/// `reason: LivenessExhausted { ... }` within 1 tick.
#[test]
#[should_panic(expected = "RED scaffold")]
fn given_liveness_three_consecutive_fails_when_reconcile_then_emits_restart_allocation_liveness_exhausted()
 {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-RECON-09 / 3 consecutive liveness fails → RestartAllocation ( LivenessExhausted ))"
    );
}

/// S-SHCP-RECON-10 (US-05 — recovery resets counter) — liveness
/// fails twice then passes → consecutive_failures resets to 0; no
/// restart action emitted.
#[test]
#[should_panic(expected = "RED scaffold")]
fn given_liveness_two_fails_then_pass_when_reconcile_then_resets_counter_and_no_restart_emitted() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-RECON-10 / liveness fail/fail/pass resets counter)"
    );
}

/// S-SHCP-RECON-11 (US-05 — restart budget exhaustion) — after
/// `RESTART_BACKOFF_CEILING` (5) liveness-driven restarts, next
/// liveness trigger emits `Failed { reason: BackoffExhausted
/// { attempts: 5 } }` (composes with existing JobLifecycle
/// pathway).
#[test]
#[should_panic(expected = "RED scaffold")]
fn given_restart_count_at_ceiling_when_liveness_fires_then_emits_failed_backoff_exhausted() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-RECON-11 / liveness restart exhausts budget → BackoffExhausted)"
    );
}
