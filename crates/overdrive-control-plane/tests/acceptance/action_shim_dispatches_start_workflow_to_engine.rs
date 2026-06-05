//! Slice 01 — engine↔reconciler boundary (DDD-5 / ADR-0064 §5, the
//! RATIFY-flagged "subtlest decision"). Gives the action-shim
//! `StartWorkflow` dispatch arm (a DESIGN EXTEND component,
//! `action_shim/mod.rs:446`) its own acceptance coverage rather than
//! leaving it only implicitly exercised by the walking skeleton.
//!
//! Scenario S-WP-01-11 — when the action-shim dispatches
//! `Action::StartWorkflow { spec, correlation }`, it hands the instance
//! to `WorkflowEngine::start` (the async executor driven off the shim,
//! exactly as `Action::StartAllocation` → `Driver::start`) — the engine
//! is NOT run as a reconciler, and the workflow-lifecycle reconciler that
//! emitted the action stays pure-sync (`ReconcilerIsPure` continues to
//! hold with the workflow-lifecycle reconciler registered). This is the
//! upheld two-primitive doctrine (R3): the reconciler manages WHICH
//! instances should exist; the engine manages HOW each instance's steps
//! execute. ADR-0064 §5.
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! The action-shim `StartWorkflow` arm (today a no-op `Ok(())`), the
//! `WorkflowEngine`, and the workflow-lifecycle reconciler do not exist
//! yet (DELIVER slice 01). `#[should_panic(expected = "RED scaffold")]`
//! keeps this RED-not-BROKEN and compiling without those unbuilt types.

#[test]
#[should_panic(expected = "RED scaffold")]
fn start_workflow_action_is_dispatched_to_the_engine_off_the_shim_not_run_as_a_reconciler() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-01-11 / action-shim StartWorkflow arm hands the instance to WorkflowEngine::start off the shim -- engine is the async executor, NOT a reconcile loop; the emitting workflow-lifecycle reconciler stays pure-sync and ReconcilerIsPure still holds)"
    );
}
