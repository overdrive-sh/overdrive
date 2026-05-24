//! Tier 1 acceptance — `ServiceLifecycleReconciler` reconcile-fn
//! purity invariants per `.claude/rules/development.md` §
//! "Reconciler I/O".
//!
//! Cross-cutting all slices. RED scaffolds.

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

/// S-SHCP-PURITY-01 (cross-cutting) — `reconcile` signature is
/// pure sync. Compile-time witness: importing
/// `ServiceLifecycleReconciler` and calling its `reconcile` without
/// `.await` AND without an async runtime succeeds at compile time.
///
/// (This test is a structural witness; the body is a structural
/// no-op when GREEN.)
#[test]
#[should_panic(expected = "RED scaffold")]
fn reconcile_signature_is_pure_sync() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-PURITY-01 / ServiceLifecycleReconciler::reconcile is pure sync)"
    );
}

/// S-SHCP-PURITY-02 — `ServiceLifecycleView` carries inputs only.
/// Compile-time witness: there is NO `is_stable: bool` field on
/// the View. (Per DDD-5 + `.claude/rules/development.md` § "Persist
/// inputs, not derived state".)
#[test]
#[should_panic(expected = "RED scaffold")]
fn view_carries_inputs_only_no_derived_is_stable_field() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-PURITY-02 / View has no derived is_stable field)"
    );
}

/// S-SHCP-PURITY-03 (cross-cutting / ADR-0037 §3 byte-equality) —
/// `AllocStatusRow.terminal` and `LifecycleEvent.terminal` populated
/// by the same action-shim call carry byte-identical
/// `TerminalCondition` values for the same deciding tick.
///
/// This test is the load-bearing byte-equality witness per ADR-0037
/// — the same property test that previously pinned Job-kind values
/// extends to Service-kind Stable/Failed values.
#[test]
#[should_panic(expected = "RED scaffold")]
fn alloc_status_row_terminal_and_lifecycle_event_terminal_are_byte_equal() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-PURITY-03 / Stable/Failed byte-equality across snapshot + streaming surfaces)"
    );
}
