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
