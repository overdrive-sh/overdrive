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
/// Promoted to GREEN in step 01-04: the `is_intentionally_stopped`
/// helper (Operator OR SystemGC) generalised the prior
/// `is_operator_stopped` resurrection-protection check, and the Run
/// branch's `active_allocs_vec` filter now excludes SystemGC-stopped
/// rows from placement-candidacy so a resubmit lands a fresh alloc —
/// making good on architecture.md § 5's promise.
#[tokio::test]
async fn resubmit_after_gc_creates_fresh_alloc() {
    let result = evaluate_resubmit_after_gc_creates_fresh_alloc().await;

    assert_eq!(
        result.name,
        Invariant::WorkloadGcResubmitCreatesFresh.as_canonical(),
        "invariant name MUST round-trip with the catalogue's canonical \
         kebab-case spelling so `cargo dst --only \
         workload-gc-resubmit-creates-fresh` resolves correctly",
    );

    assert!(
        matches!(result.status, InvariantStatus::Pass),
        "WorkloadGcResubmitCreatesFresh invariant FAILED: cause = {:?}. After \
         resubmit, the WorkloadLifecycle reconciler's Run branch MUST place a \
         fresh allocation with a NEW `alloc_id` distinct from the GC'd row's \
         `alloc_id`, AND the original GC'd row's `terminal == \
         Some(Stopped {{ by: SystemGC }})` MUST stay durable for every tick \
         after resubmit. A failure here usually means: (a) the \
         `is_intentionally_stopped` helper at \
         `crates/overdrive-core/src/reconciler.rs` was narrowed back to only \
         match `Operator` (regressing the SystemGC case), (b) the Run-branch \
         `active_allocs_vec` filter regressed (SystemGC-stopped rows now \
         participate in `running_alloc` / restart / natural-exit decisions \
         instead of being filtered out), or (c) `mint_alloc_id` lost its \
         attempt-suffix derivation and started reusing the prior alloc's id.",
        result.cause,
    );
}

/// Catalogue + canonical-name round-trip property for both new
/// variants. Both variants MUST round-trip through `FromStr ↔
/// Display` losslessly (per the proptest in
/// `tests/invariant_roundtrip.rs`'s contract). Both
/// `WorkloadGcOrphanConverges` and `WorkloadGcResubmitCreatesFresh`
/// live in `Invariant::ALL` since step 01-04 closed the
/// resurrection-protection gap (the `is_intentionally_stopped`
/// helper + `active_allocs_vec` Run-branch filter).
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

    // Default-catalogue membership: both variants MUST appear in
    // `Invariant::ALL` so the harness's default catalogue iterates
    // them under `cargo dst` without requiring `--only`. Step 01-04
    // closed the production gap that previously gated
    // `WorkloadGcResubmitCreatesFresh` out of ALL.
    assert!(
        Invariant::ALL.contains(&Invariant::WorkloadGcOrphanConverges),
        "WorkloadGcOrphanConverges MUST appear in `Invariant::ALL`",
    );
    assert!(
        Invariant::ALL.contains(&Invariant::WorkloadGcResubmitCreatesFresh),
        "WorkloadGcResubmitCreatesFresh MUST appear in `Invariant::ALL` since \
         step 01-04 closed the resurrection-protection gap. A regression \
         here means either the variant was reverted out of ALL or the \
         production fix at `crates/overdrive-core/src/reconciler.rs` \
         (`is_intentionally_stopped` helper + `active_allocs_vec` Run-branch \
         filter) was rolled back.",
    );
}
