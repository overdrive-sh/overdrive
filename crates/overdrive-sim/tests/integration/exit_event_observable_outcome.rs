//! fix-exit-observer-running-gate step 01-05 (Solution 4) — DST
//! integration test for the `ExitEventObservableOutcome` invariant.
//!
//! Drives the invariant evaluator (which itself drives the live
//! `action_shim + exit_observer + SimDriver + SimObservationStore`
//! wiring through two scenarios — happy path + May-2 degraded
//! escalation) and asserts the invariant holds. See
//! `crates/overdrive-sim/src/invariants/exit_event_observable_outcome.rs`
//! for the scenario bodies.
//!
//! With Solution 1' (oneshot-gated watcher emission) landed in steps
//! 01-02 / 01-03, this test passes naturally — every `ExitEvent`
//! consumed by the observer produces at least one of the three
//! visible outcomes named by AC1. The test's load-bearing role is
//! preventing future regressions through any emission path that
//! bypasses the gate.
//!
//! Closes the gap predecessor RCA
//! `fix-exit-observer-write-retry/deliver/rca.md:107-109` named and
//! `docs/evolution/2026-05-02-fix-exit-observer-write-retry.md:64`
//! left open.
//!
//! See:
//! - `docs/feature/fix-exit-observer-running-gate/deliver/rca.md`
//! - `docs/feature/fix-exit-observer-write-retry/deliver/rca.md`
//!   (predecessor; symmetric write-failure leg)

use overdrive_sim::invariants::exit_event_observable_outcome::evaluate_exit_event_observable_outcome;
use overdrive_sim::{Invariant, InvariantStatus};

/// AC1 + AC3 + AC4 + AC6 — the invariant holds across both scenarios
/// (positive happy-path AND May-2 degraded-escalation negative
/// scenario) with Solution 1' landed.
#[tokio::test]
async fn invariant_holds_across_happy_path_and_degraded_escalation() {
    let result = evaluate_exit_event_observable_outcome().await;

    assert_eq!(
        result.name,
        Invariant::ExitEventObservableOutcome.as_canonical(),
        "invariant name MUST round-trip with the catalogue's canonical \
         kebab-case spelling so `cargo dst --only \
         exit-event-observable-outcome` resolves correctly",
    );

    assert!(
        matches!(result.status, InvariantStatus::Pass),
        "ExitEventObservableOutcome invariant FAILED: cause = {:?}. With \
         Solution 1' landed in steps 01-02 / 01-03, every ExitEvent \
         consumed by the worker exit_observer MUST produce at least one \
         of (a) AllocStatusRow with state ∈ {{Failed, Terminated}}, (b) \
         degraded LifecycleEvent carrying \
         TransitionReason::DriverInternalError, or (c) structured \
         tracing::error! naming the alloc_id. A failure here means a \
         future emission path bypassed the gate or violated the May-2 \
         loud-failure-semantics contract.",
        result.cause,
    );
}

/// AC2 + the canonical-name round-trip property — the new variant is
/// reachable from `Invariant::ALL` and resolves through `FromStr`. If
/// a future refactor drops the variant from the catalogue or breaks
/// the round-trip, this test catches it.
#[tokio::test]
async fn variant_is_in_catalogue_and_round_trips() {
    let canonical = Invariant::ExitEventObservableOutcome.as_canonical();
    assert_eq!(canonical, "exit-event-observable-outcome");

    let parsed: Invariant = canonical.parse().expect("canonical name MUST parse via FromStr");
    assert_eq!(parsed, Invariant::ExitEventObservableOutcome);

    assert!(
        Invariant::ALL.contains(&Invariant::ExitEventObservableOutcome),
        "ExitEventObservableOutcome MUST appear in `Invariant::ALL` so \
         the harness's default catalogue iterates it under `cargo dst` \
         without requiring `--only`",
    );
}
