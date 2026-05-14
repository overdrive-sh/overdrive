// Scenario docstrings reference concept words (`SystemGC`, `Operator`,
// `alloc_id`, `is_operator_stopped`, `mint_alloc_id`, `workload_id`,
// `attempt_count`, sub-invariant names like `gc.converges`,
// `resubmit.places_fresh`) embedded in narrative prose where forcing
// every occurrence into backticks degrades readability of the scenario
// specifications. Scoped expect, not crate-wide allow; lifts when the
// docstrings are restructured.
#![expect(clippy::doc_markdown, reason = "narrative scenario docstrings — see file-header comment")]

//! workload-gc-absent-stale-allocs step 01-03 — DST integration tests
//! for the absent-intent workload GC arm.
//!
//! Two scenarios drive the live `SimIntentStore + SimObservationStore +
//! WorkloadLifecycle` runtime stack end-to-end. Entry through the
//! public `submit` (intent put) / `tick` (run_convergence_tick)
//! harness driving ports; assertions land at the
//! `ObservationStore::alloc_status_rows()` driven port boundary. No
//! reconciler internals exercised directly.
//!
//! See:
//! - `docs/feature/workload-gc-absent-stale-allocs/design/architecture.md`
//!   § 7 *DST invariant shape* — the spec these scenarios implement.
//! - `crates/overdrive-sim/src/invariants/workload_gc_absent_intent.rs`
//!   — the evaluator bodies.
//! - GitHub issue #148 AC §1.3 — *"DST scenario covering the
//!   absent-workload-stale-running shape"*.

use overdrive_sim::invariants::workload_gc_absent_intent::{
    evaluate_orphan_workload_converges_to_terminal_gc,
    evaluate_resubmit_after_gc_creates_fresh_alloc,
};
use overdrive_sim::{Invariant, InvariantStatus};

/// Scenario 1 — submit Job(X), drain to Running,
/// `IntentStore::delete("jobs/X")`, drive ≤ 3 ticks, assert all rows
/// for X reach a terminal state with
/// `Some(Stopped { by: SystemGC })` AND no fresh alloc placed in the
/// post-fault tick window. Closes #148 AC §1.3 (orphan-converges
/// half).
#[tokio::test]
async fn orphan_workload_converges_to_terminal_gc() {
    let result = evaluate_orphan_workload_converges_to_terminal_gc().await;

    assert_eq!(
        result.name,
        Invariant::WorkloadGcOrphanConverges.as_canonical(),
        "invariant name MUST round-trip with the catalogue's canonical \
         kebab-case spelling so `cargo dst --only \
         workload-gc-orphan-converges` resolves correctly",
    );

    assert!(
        matches!(result.status, InvariantStatus::Pass),
        "WorkloadGcOrphanConverges invariant FAILED: cause = {:?}. The GC arm \
         in WorkloadLifecycle::reconcile MUST emit StopAllocation per Running \
         alloc with terminal=Some(Stopped {{ by: SystemGC }}) when the desired \
         Job intent is absent — and no fresh alloc may be placed for the \
         workload while intent stays absent. See architecture.md § 7 for the \
         invariant shape and step 01-02's reconciler implementation for the \
         GC arm body. A failure here usually means: (a) the action shim's \
         StopAllocation handling regressed (broadcast-before-durability), (b) \
         the reconciler's None arm regressed (returns Vec::new() instead of \
         emitting GC stops), or (c) the SystemGC stamp was dropped or replaced \
         with a different StoppedBy variant.",
        result.cause,
    );
}

/// Scenario 2 — continues from scenario 1's quiescent state, resubmits
/// `Job(X)`, drives ≤ 5 ticks, asserts (a) ≥1 alloc with a fresh
/// `alloc_id` (distinct from the original GC'd row) reaches Running
/// AND (b) the original alloc's SystemGC terminal stamp stays durable
/// for every post-resubmit tick. Closes #148 AC §1.3
/// (resubmit-creates-fresh half).
///
/// **RED scaffold — production gap surfaced to user 2026-05-14**.
/// Per `.claude/rules/testing.md` § "Test-side scaffolds —
/// `#[should_panic(expected = "RED scaffold")]`", this is the
/// sanctioned RED-without-blocking-CI shape. The scenario evaluator
/// IS exercised against the live action_shim + reconciler runtime
/// (the failure is the production behavior, not unwired
/// scaffolding). The architecture-faithful invariants
/// `resubmit.places_fresh` (alloc_id distinctness half) and
/// `resubmit.preserves_prior_gc_terminal` (architecture.md § 7) are
/// catastrophically incompatible with today's reconciler shape:
///
///   - `crates/overdrive-core/src/reconciler.rs:1294` —
///     `is_operator_stopped(r)` resurrection-protection check
///     matches `StoppedBy::Operator` ONLY, not `StoppedBy::SystemGC`.
///   - `crates/overdrive-core/src/reconciler.rs:1558` —
///     `mint_alloc_id` is purely deterministic on workload_id
///     (`alloc-{workload_id}-0`); a resubmit reuses the same id and
///     LWW overwrites the prior terminal stamp.
///
/// The architecture-faithful fix is one of:
///   (A) extend the resurrection-protection check at line 1294 to
///       cover `SystemGC` (one-line guard via a sibling
///       `is_system_gc_stopped` predicate or a widened
///       `is_terminal_stopped`), OR
///   (B) derive `mint_alloc_id` from `(workload_id, attempt_count)`
///       so resubmit produces a fresh id by construction.
///
/// Both touch `reconciler.rs` body which is step 01-02 scope and
/// excluded from step 01-03 per BOUNDARY_RULES. Surfaced to the
/// user; awaiting decision on whether to expand step 01-03 scope,
/// add a follow-up step (e.g. 01-04), or amend architecture.md § 7
/// to a weaker invariant. Once the production gap closes, promote
/// this test to GREEN by dropping the `#[should_panic]` attribute
/// and the trailing `panic!`.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn resubmit_after_gc_creates_fresh_alloc() {
    let result = evaluate_resubmit_after_gc_creates_fresh_alloc().await;

    assert_eq!(
        result.name,
        Invariant::WorkloadGcResubmitCreatesFresh.as_canonical(),
        "invariant name MUST round-trip with the catalogue's canonical \
         kebab-case spelling so `cargo dst --only \
         workload-gc-resubmit-creates-fresh` resolves correctly",
    );

    if matches!(result.status, InvariantStatus::Pass) {
        // Production gap closed — the architecture-faithful
        // invariant now holds. Promote this test to GREEN by
        // dropping the `#[should_panic]` attribute and this
        // entire `if`/panic guard.
        return;
    }

    panic!(
        "Not yet implemented -- RED scaffold (workload-gc-absent-stale-allocs \
         step 01-03 / scenario 2 / `resubmit.preserves_prior_gc_terminal` + \
         `resubmit.places_fresh` alloc_id-distinctness half): \
         architecture.md § 7 requires the original alloc's SystemGC terminal \
         stamp be durable across resubmit, but today's \
         WorkloadLifecycle::reconcile Run branch reuses the deterministic \
         `alloc-{{workload_id}}-0` id and the action shim's LWW write of the \
         new `Running` row overwrites the prior terminal stamp. Fix lives at \
         crates/overdrive-core/src/reconciler.rs:1294 (extend resurrection \
         protection to cover StoppedBy::SystemGC) OR :1558 (fresh-id \
         derivation in mint_alloc_id). Out of scope for step 01-03 per \
         BOUNDARY_RULES (step 01-02 owns reconciler.rs body changes). \
         Evaluator cause = {:?}",
        result.cause,
    );
}

/// Catalogue + canonical-name round-trip property for both new
/// variants. Both variants MUST round-trip through `FromStr ↔
/// Display` losslessly (per the proptest in
/// `tests/invariant_roundtrip.rs`'s contract). `WorkloadGcOrphanConverges`
/// is in `Invariant::ALL`; `WorkloadGcResubmitCreatesFresh` is
/// intentionally excluded from ALL while the production gap at
/// `crates/overdrive-core/src/reconciler.rs:1294` remains open
/// (see this scenario's `#[should_panic]` guard for the full
/// rationale). Both remain reachable via `cargo dst --only <NAME>`.
#[tokio::test]
async fn workload_gc_variants_in_catalogue_and_round_trip() {
    // Both variants — name + round-trip.
    for (variant, expected_canonical) in [
        (Invariant::WorkloadGcOrphanConverges, "workload-gc-orphan-converges"),
        (Invariant::WorkloadGcResubmitCreatesFresh, "workload-gc-resubmit-creates-fresh"),
    ] {
        let canonical = variant.as_canonical();
        assert_eq!(
            canonical, expected_canonical,
            "{variant:?} canonical name MUST be `{expected_canonical}`",
        );

        let parsed: Invariant = canonical
            .parse()
            .unwrap_or_else(|_| panic!("canonical name `{canonical}` MUST parse via FromStr"));
        assert_eq!(parsed, variant, "FromStr round-trip MUST yield the same variant");
    }

    // Default-catalogue membership.
    assert!(
        Invariant::ALL.contains(&Invariant::WorkloadGcOrphanConverges),
        "WorkloadGcOrphanConverges MUST appear in `Invariant::ALL` so the \
         harness's default catalogue iterates it under `cargo dst` without \
         requiring `--only`",
    );
    assert!(
        !Invariant::ALL.contains(&Invariant::WorkloadGcResubmitCreatesFresh),
        "WorkloadGcResubmitCreatesFresh MUST NOT appear in `Invariant::ALL` \
         while the production gap at \
         `crates/overdrive-core/src/reconciler.rs:1294` (resurrection \
         protection covers StoppedBy::Operator only, not StoppedBy::SystemGC) \
         remains open — its evaluator returns Fail under today's code and \
         would break the default-catalogue green contract observed by \
         `dst_clean_clone_green` / `summary_names_every_expected_invariant`. \
         Once the gap closes, promote the variant into ALL and flip this \
         assertion to `assert!(contains)`.",
    );
}
