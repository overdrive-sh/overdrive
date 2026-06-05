//! Slice 01 / US-WP-3 AC4 — the workflow-lifecycle reconciler re-hydrates
//! a running instance from `Action::StartWorkflow` on restart.
//!
//! Scenario S-WP-01-08. O2 (single-node). An instance that is `running`
//! in intent but has no live engine task after a process restart: the
//! pure-sync `reconcile` re-emits `Action::StartWorkflow { spec,
//! correlation }` for it (the engine's `load_journal` then RESUMES rather
//! than cold-starts), and the `reconcile` body performs no `.await` — the
//! `ReconcilerIsPure` DST invariant continues to hold with the
//! workflow-lifecycle reconciler registered alongside the existing
//! reconcilers. ADR-0064 §5. The shim-dispatch boundary (engine driven
//! off the shim, not run as a reconciler) is the sibling scenario
//! S-WP-01-11.
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! The workflow-lifecycle reconciler, the concrete `WorkflowSpec`, and
//! the engine handoff do not exist yet (DELIVER slice 01).
//! `#[should_panic(expected = "RED scaffold")]` keeps this RED-not-BROKEN
//! and compiling without those unbuilt types.

#[test]
#[should_panic(expected = "RED scaffold")]
fn lifecycle_reconciler_re_emits_start_workflow_for_a_running_instance_with_no_live_task() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-01-08 / workflow-lifecycle reconcile re-emits Action::StartWorkflow for a running-in-intent instance with no live engine task; reconcile stays pure-sync with no .await and ReconcilerIsPure still holds with the workflow-lifecycle reconciler registered)"
    );
}
