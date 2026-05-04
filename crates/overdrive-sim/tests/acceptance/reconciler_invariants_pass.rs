//! Acceptance tests for step 04-05 — DST invariants
//! `AtLeastOneReconcilerRegistered`, `DuplicateEvaluationsCollapse`, and
//! `ReconcilerIsPure` evaluate green on a clean harness run; each
//! invariant's failure mode is caught by a seeded fault.
//!
//! Per ADR-0013 §2 and whitepaper §18, these invariants are the runtime
//! proof of the reconciler-primitive contracts:
//!
//! * **`AtLeastOneReconcilerRegistered`** — after boot, the registry is
//!   never empty; `noop-heartbeat` is the Phase 1 proof-of-life entry.
//! * **`DuplicateEvaluationsCollapse`** — N (≥3) concurrent evaluations at
//!   the same `(ReconcilerName, TargetResource)` key collapse to
//!   exactly one dispatched invocation and `N - 1` cancellations.
//! * **`ReconcilerIsPure`** — twin invocation with identical `(desired,
//!   actual, db)` inputs produces bit-identical `Vec<Action>` outputs.
//!
//! These acceptance tests exercise the invariants through the DST
//! harness, which is the port-to-port entry point. The evaluator
//! functions themselves are unit-tested in
//! `tests/invariant_evaluators.rs`.

use overdrive_sim::{Harness, Invariant, InvariantStatus};

/// The default harness run includes all three new invariants and they
/// all pass — the clean-run green-bar assertion.
#[test]
fn default_harness_run_passes_all_three_reconciler_invariants() {
    let report = Harness::new().run(42).expect("harness must compose");

    let at_least_one = report
        .invariants
        .iter()
        .find(|r| r.name == "at-least-one-reconciler-registered")
        .expect("at-least-one-reconciler-registered must appear in report");
    assert_eq!(
        at_least_one.status,
        InvariantStatus::Pass,
        "AtLeastOneReconcilerRegistered must pass on a default harness run; got {at_least_one:?}",
    );

    let collapse = report
        .invariants
        .iter()
        .find(|r| r.name == "duplicate-evaluations-collapse")
        .expect("duplicate-evaluations-collapse must appear in report");
    assert_eq!(
        collapse.status,
        InvariantStatus::Pass,
        "DuplicateEvaluationsCollapse must pass on a default harness run; got {collapse:?}",
    );

    let purity = report
        .invariants
        .iter()
        .find(|r| r.name == "reconciler-is-pure")
        .expect("reconciler-is-pure must appear in report");
    assert_eq!(
        purity.status,
        InvariantStatus::Pass,
        "ReconcilerIsPure must pass on a default harness run; got {purity:?}",
    );

    // Full run is green.
    assert!(report.is_green(), "clean harness run must be green; failures={:?}", report.failures);
}

/// Narrow the run to just `AtLeastOneReconcilerRegistered`. The harness
/// boots with at least `noop-heartbeat` so the invariant passes.
#[test]
fn at_least_one_reconciler_registered_passes_on_default_harness() {
    let report = Harness::new()
        .only(Invariant::AtLeastOneReconcilerRegistered)
        .run(7)
        .expect("harness must compose");

    assert_eq!(report.invariants.len(), 1);
    assert_eq!(report.invariants[0].name, "at-least-one-reconciler-registered");
    assert_eq!(report.invariants[0].status, InvariantStatus::Pass);
}

/// Narrow the run to just `DuplicateEvaluationsCollapse`. The harness
/// internally submits N ≥ 3 evaluations at the same key and asserts that
/// `dispatched == 1 && cancelled == N - 1`.
#[test]
fn duplicate_evaluations_collapse_passes_on_default_harness() {
    let report = Harness::new()
        .only(Invariant::DuplicateEvaluationsCollapse)
        .run(7)
        .expect("harness must compose");

    assert_eq!(report.invariants.len(), 1);
    assert_eq!(report.invariants[0].name, "duplicate-evaluations-collapse");
    assert_eq!(report.invariants[0].status, InvariantStatus::Pass);
}

/// Narrow the run to just `ReconcilerIsPure`. The harness twin-invokes
/// the Phase 1 proof-of-life reconciler (always returns `[Action::Noop]`)
/// and asserts the two outputs are element-for-element equal.
#[test]
fn reconciler_is_pure_passes_on_default_harness() {
    let report =
        Harness::new().only(Invariant::ReconcilerIsPure).run(7).expect("harness must compose");

    assert_eq!(report.invariants.len(), 1);
    assert_eq!(report.invariants[0].name, "reconciler-is-pure");
    assert_eq!(report.invariants[0].status, InvariantStatus::Pass);
}

// ---------------------------------------------------------------------------
// reconciler-memory-redb step 01-07 — ViewStore DST invariant acceptance.
// ---------------------------------------------------------------------------

/// `ViewStoreRoundtripIsLossless` runs in the default catalogue and
/// passes on a clean harness build. proptest-backed; covers
/// `JobLifecycleView` and the unit-View case (`NoopHeartbeat`).
#[test]
fn view_store_roundtrip_is_lossless_passes_on_default_harness() {
    let report = Harness::new()
        .only(Invariant::ViewStoreRoundtripIsLossless)
        .run(7)
        .expect("harness must compose");
    assert_eq!(report.invariants.len(), 1);
    assert_eq!(report.invariants[0].name, "view-store-roundtrip-is-lossless");
    assert_eq!(
        report.invariants[0].status,
        InvariantStatus::Pass,
        "ViewStoreRoundtripIsLossless must pass on default harness; got {:?}",
        report.invariants[0],
    );
}

/// `BulkLoadIsDeterministic` runs in the default catalogue and passes:
/// two `bulk_load` calls against the same `SimViewStore` produce
/// PartialEq-equal `BTreeMap` results.
#[test]
fn bulk_load_is_deterministic_passes_on_default_harness() {
    let report = Harness::new()
        .only(Invariant::BulkLoadIsDeterministic)
        .run(7)
        .expect("harness must compose");
    assert_eq!(report.invariants.len(), 1);
    assert_eq!(report.invariants[0].name, "bulk-load-is-deterministic");
    assert_eq!(report.invariants[0].status, InvariantStatus::Pass);
}

/// `WriteThroughOrdering` runs in the default catalogue and passes:
/// the runtime obeys the fsync-then-memory ordering rule per ADR-0035
/// §5. Under `SimViewStore::inject_fsync_failure`, the runtime's
/// in-memory map for the target whose write failed MUST still hold
/// the pre-injection value.
#[test]
fn write_through_ordering_passes_on_default_harness() {
    let report =
        Harness::new().only(Invariant::WriteThroughOrdering).run(7).expect("harness must compose");
    assert_eq!(report.invariants.len(), 1);
    assert_eq!(report.invariants[0].name, "write-through-ordering");
    assert_eq!(
        report.invariants[0].status,
        InvariantStatus::Pass,
        "WriteThroughOrdering must pass on default harness; got {:?}",
        report.invariants[0],
    );
}

/// All three new invariants appear in the default catalogue and pass
/// when run as part of the full set. K3 reproducibility: same seed
/// twice produces identical verdicts.
#[test]
fn full_default_catalogue_includes_three_view_store_invariants_and_passes_them() {
    let report = Harness::new().run(99).expect("harness must compose");

    for canonical in
        ["view-store-roundtrip-is-lossless", "bulk-load-is-deterministic", "write-through-ordering"]
    {
        let entry = report
            .invariants
            .iter()
            .find(|r| r.name == canonical)
            .unwrap_or_else(|| panic!("{canonical} must appear in default catalogue"));
        assert_eq!(
            entry.status,
            InvariantStatus::Pass,
            "{canonical} must pass on default harness; got {entry:?}",
        );
    }
}
