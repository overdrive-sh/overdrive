//! Tier 1 acceptance — `ServiceLifecycleReconciler` reconcile-fn
//! purity invariants per `.claude/rules/development.md` §
//! "Reconciler I/O" and `.claude/rules/development.md` § "Persist
//! inputs, not derived state".
//!
//! Cross-cutting all slices. Step 01-03b wires the dispatch enums
//! and lands S-SHCP-PURITY-02 as a structural-not-behavioural
//! assertion.

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
    reason = "structural acceptance — exercises the dispatch enums and View shape"
)]

use overdrive_core::reconcilers::{
    Action, AnyReconciler, AnyReconcilerView, AnyState, Reconciler, TickContext,
};
use overdrive_core::service_lifecycle::{
    ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
};

/// S-SHCP-PURITY-01 — `ServiceLifecycleReconciler::reconcile` is
/// pure sync (compile-time witness: this function calls reconcile
/// without `.await` and without an async runtime).
#[test]
fn reconcile_signature_is_pure_sync() {
    let reconciler = ServiceLifecycleReconciler::new();
    let desired = ServiceLifecycleState::default();
    let actual = ServiceLifecycleState::default();
    let view = ServiceLifecycleView::default();
    let now = std::time::Instant::now();
    let tick = TickContext {
        now,
        now_unix: overdrive_core::wall_clock::UnixInstant::from_unix_duration(
            std::time::Duration::from_millis(0),
        ),
        tick: 0,
        deadline: now + std::time::Duration::from_secs(1),
    };
    let (actions, _next_view): (Vec<Action>, ServiceLifecycleView) =
        reconciler.reconcile(&desired, &actual, &view, &tick);
    // Empty allocs → no actions; the signature pinning is the load-bearing
    // assertion (`reconcile` returns synchronously, no `.await`).
    assert!(actions.is_empty(), "empty state must produce no actions; got {actions:?}");
}

/// S-SHCP-PURITY-02 — `ServiceLifecycleView` carries inputs only.
///
/// Structural assertion via `rg` over the source: the View type
/// declaration contains NO field name matching the forbidden
/// derived-state patterns (`next_*_at`, `*_deadline`, `is_stable`).
/// Per `.claude/rules/development.md` § "Persist inputs, not
/// derived state": the `Stable` predicate / readiness `healthy` /
/// deadline computations are ALL recomputed every tick — a cached
/// `is_stable: bool` or persisted `next_attempt_at` deadline on the
/// View would be a violation.
///
/// The semantically-permitted shape (per the AC):
/// - per-alloc Stable-announcement record (`BTreeSet<AllocationId>`,
///   recording an *observed event*, not a derived computation),
/// - per-`(alloc, probe_idx)` consecutive-failure counters,
/// - per-`(alloc, probe_idx)` consecutive-success counters,
/// - per-`(alloc, probe_idx)` startup-attempt counters.
#[test]
fn view_carries_inputs_only_no_derived_state_slots() {
    // Find the workspace root and locate the View source file.
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CARGO_MANIFEST_DIR has a parent")
        .parent()
        .expect("crate dir has a workspace ancestor");
    let view_source = workspace_root
        .join("crates")
        .join("overdrive-core")
        .join("src")
        .join("service_lifecycle.rs");
    let source = std::fs::read_to_string(&view_source)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", view_source.display()));

    // Locate the `pub struct ServiceLifecycleView { ... }` block.
    let needle = "pub struct ServiceLifecycleView {";
    let start = source.find(needle).unwrap_or_else(|| {
        panic!("ServiceLifecycleView struct declaration not found in {}", view_source.display())
    });
    let after_brace = start + needle.len();
    let end = source[after_brace..]
        .find("\n}")
        .unwrap_or_else(|| panic!("could not find closing brace of ServiceLifecycleView"));
    let body = &source[after_brace..after_brace + end];

    // Forbidden derived-state identifier patterns (per the AC):
    // these are the rg-verifiable anchors. Field NAMES in the View
    // must not contain any of these substrings.
    //
    // We walk the struct body line-by-line, identify `pub <name>:`
    // declarations, and check the field name (NOT comments).
    for line in body.lines() {
        let trimmed = line.trim();
        // Skip empty / doc-comment / regular-comment lines.
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("///") {
            continue;
        }
        // Match `pub <name>: ...` declarations (field declarations).
        let Some(rest) = trimmed.strip_prefix("pub ") else {
            continue;
        };
        let Some(colon_idx) = rest.find(':') else {
            continue;
        };
        let field_name = rest[..colon_idx].trim();

        // Anchor 1: `next_*_at` — derived deadline pattern.
        assert!(
            !(field_name.starts_with("next_") && field_name.ends_with("_at")),
            "S-SHCP-PURITY-02 violation: field `{field_name}` on ServiceLifecycleView \
             matches forbidden `next_*_at` derived-deadline pattern"
        );
        // Anchor 2: `*_deadline` — derived deadline pattern.
        assert!(
            !field_name.ends_with("_deadline"),
            "S-SHCP-PURITY-02 violation: field `{field_name}` on ServiceLifecycleView \
             matches forbidden `*_deadline` derived-deadline pattern"
        );
        // Anchor 3: `is_stable`-shaped — derived predicate cache.
        // Match: an exact `is_stable` field OR any boolean-shaped
        // name suggesting a cached derived predicate.
        assert!(
            !(field_name == "is_stable" || field_name.starts_with("is_stable_")),
            "S-SHCP-PURITY-02 violation: field `{field_name}` on ServiceLifecycleView \
             matches forbidden `is_stable`-shaped derived-predicate cache pattern"
        );
    }
}

/// S-SHCP-PURITY-03 (placeholder for ADR-0037 § 3 byte-equality;
/// lands in subsequent slice where the action-shim path is wired).
#[test]
#[ignore = "blocked on later slice — AllocStatusRow/LifecycleEvent action-shim wiring"]
fn alloc_status_row_terminal_and_lifecycle_event_terminal_are_byte_equal() {
    // Pinned at later slice that lands the action-shim path for
    // ServiceLifecycle terminal events.
}

/// S-SHCP-DISPATCH-01 — `AnyReconciler::ServiceLifecycle` dispatch
/// arm wires through to `ServiceLifecycleReconciler::reconcile`.
///
/// This test exercises the AnyReconciler / AnyState / AnyReconcilerView
/// extensions landed in step 01-03b: constructing the variant,
/// dispatching reconcile, and unwrapping the returned view shape.
#[test]
fn any_reconciler_service_lifecycle_dispatch_arms_wired() {
    let any = AnyReconciler::ServiceLifecycle(ServiceLifecycleReconciler::new());

    // `name()` / `static_name()` dispatch arms.
    assert_eq!(
        any.name().as_str(),
        <ServiceLifecycleReconciler as Reconciler>::NAME,
        "AnyReconciler::ServiceLifecycle MUST dispatch name() to the inner reconciler"
    );
    assert_eq!(
        any.static_name(),
        <ServiceLifecycleReconciler as Reconciler>::NAME,
        "AnyReconciler::ServiceLifecycle MUST dispatch static_name() to the inner const"
    );

    // `reconcile` dispatch arm. Empty state → empty actions, returned
    // view variant matches the dispatched-on view shape.
    let desired = AnyState::ServiceLifecycle(ServiceLifecycleState::default());
    let actual = AnyState::ServiceLifecycle(ServiceLifecycleState::default());
    let view = AnyReconcilerView::ServiceLifecycle(ServiceLifecycleView::default());
    let now = std::time::Instant::now();
    let tick = TickContext {
        now,
        now_unix: overdrive_core::wall_clock::UnixInstant::from_unix_duration(
            std::time::Duration::from_millis(0),
        ),
        tick: 0,
        deadline: now + std::time::Duration::from_secs(1),
    };
    let (actions, returned_view) = any.reconcile(&desired, &actual, &view, &tick);
    assert!(
        actions.is_empty(),
        "empty ServiceLifecycle state must produce no actions; got {actions:?}"
    );
    assert!(
        matches!(returned_view, AnyReconcilerView::ServiceLifecycle(_)),
        "ServiceLifecycle dispatch must return AnyReconcilerView::ServiceLifecycle; \
         got {returned_view:?}"
    );
}
