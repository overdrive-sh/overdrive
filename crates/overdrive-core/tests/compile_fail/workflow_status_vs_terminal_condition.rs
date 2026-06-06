//! ADR-0065 §Consequences — `WorkflowStatus` and `TerminalCondition` are
//! NOT type-substitutable.
//!
//! The research's load-bearing finding (ADR-0065 §Context / §3) is that the
//! workflow's engine-owned control-plane status (`WorkflowStatus`) and the
//! reconciler's allocation-terminal claim (`TerminalCondition`, ADR-0037)
//! model genuinely different things — "a reader must not conflate them"
//! (§Consequences → Negative). This compile-fail fixture makes the
//! non-conflation STRUCTURAL: a function expecting one rejects the other at
//! compile time, in both directions, so a future refactor cannot silently
//! blur the two control-plane terminal-status types.
//!
//! Counterpart to the `intent_vs_observation.rs` non-substitutability
//! fixture (the `IntentStore` / `ObservationStore` state-split analogue).
//! Both errors are `E0308` mismatched-types; the diagnostic names both
//! concrete types so the operator can tell which side they conflated.

use overdrive_core::transition_reason::TerminalCondition;
use overdrive_core::workflow::WorkflowStatus;

fn expects_workflow_status(_status: WorkflowStatus) {}

fn expects_terminal_condition(_condition: TerminalCondition) {}

fn passes_terminal_condition_where_workflow_status_expected(condition: TerminalCondition) {
    // A reconciler's allocation-terminal claim is NOT a workflow's
    // engine-owned status — this must not compile.
    expects_workflow_status(condition);
}

fn passes_workflow_status_where_terminal_condition_expected(status: WorkflowStatus) {
    // ...and the reverse: a workflow status is NOT an allocation-terminal
    // claim — this must not compile either.
    expects_terminal_condition(status);
}

fn main() {}
