//! Slice 01 / US-WP-1 AC1 — author writes one ordinary `async fn run`
//! and the platform drives it to a terminal `WorkflowResult`.
//!
//! Scenario S-WP-01-01 (`docs/feature/workflow-primitive/distill/test-scenarios.md`).
//! ADR-0064 §1 (`Workflow` trait + `WorkflowCtx` in `overdrive-core`),
//! §4 (`ctx.call` is the slice-01 surface). K6 / O3.
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! The `Workflow` trait, `WorkflowCtx`, `WorkflowResult` and a concrete
//! `ProvisionRecord` do not exist yet — they land in DELIVER slice 01.
//! Per the project RED-scaffold convention this body is a `panic!`
//! naming the scenario, gated by `#[should_panic(expected = "RED
//! scaffold")]`, so the test COMPILES and PASSES at the bar (nextest
//! PASS, clippy happy) without importing the unbuilt production types.
//! DELIVER replaces the panic body with the real drive-to-terminal
//! assertion when it unskips this scenario.

#[test]
#[should_panic(expected = "RED scaffold")]
fn provision_record_drives_to_terminal_workflow_result() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-01-01 / ProvisionRecord async fn run drives to a terminal WorkflowResult::Success)"
    );
}
