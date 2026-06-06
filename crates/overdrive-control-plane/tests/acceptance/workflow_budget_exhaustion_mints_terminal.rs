//! Acceptance — a transient (retryable) failure is absorbed and re-driven
//! by the engine and NEVER reaches the body's return type; on retry-budget
//! exhaustion the engine MINTS `WorkflowStatus::Failed { terminal:
//! BudgetExhausted }` (DISTILL RED scaffold, `workflow-result-error-model`
//! / ADR-0065).
//!
//! **Slice 04 (follow-on PR; additive).** These scaffolds are authored now
//! so the body contract's retryable-vs-terminal split (D4) is specified
//! from Slice 01, but they STAY `#[should_panic(expected = "RED scaffold")]`
//! until Slice 04 lands the retry-re-drive loop (the
//! `JournalCommand::RetryAttempted` additive command + transient-error
//! classification + backoff parking + `TerminalError::budget_exhausted()`
//! minting). Slices 01-03 land the types + success/explicit-terminal paths;
//! until Slice 04 the engine's behaviour is "explicit `Err(TerminalError)`
//! ends the instance" with no transient re-drive (ADR-0065 § 4, Negative
//! consequence "retry-re-drive loop deferred to Slice 04").
//!
//! # Why NEW
//!
//! The retryable-vs-terminal taxonomy is structurally new: the prior
//! contentless terminal model collapsed retryable and terminal into one
//! body-authored variant (the anti-pattern the four-platform research
//! refutes). The engine-OWNED retry budget (journal-`RetryAttempted`-derived
//! attempts + engine-constant policy, NOT the body, NOT a reconciler `View`
//! — contrast `RetryMemory`) and the engine-MINTED `BudgetExhausted` are new
//! behaviour with no existing test.
//!
//! # Layer / paradigm
//!
//! Layer 1-2 (engine drives over `Sim*` ports; the forced-transient
//! workflow and the budget are deterministic under `SimClock`). Per Mandate
//! 11 these engine-boundary sad paths are EXAMPLE-based (a forced-transient
//! fixture re-driven a fixed number of times), NOT PBT-generated.
//!
//! # Port-to-port
//!
//! The driving port is `WorkflowEngine::start`. The observable outcomes are
//! asserted at the driven-port boundaries: (a) the `ObservationStore`
//! `WorkflowTerminal { status }` row carries `WorkflowStatus::Failed {
//! terminal: kind == BudgetExhausted }`; (b) the author body's
//! transient-failure step ran exactly `budget` times across the re-drives
//! (a shared `AtomicUsize`), proving the engine re-drove from the journal
//! up to the budget rather than the body authoring the failure; (c) the
//! journal carries `budget`-many `RetryAttempted` commands (the recomputed
//! attempt INPUTS, per `development.md` "Persist inputs, not derived
//! state"). The body NEVER returns `Err` — it returns a step `Err` the
//! engine absorbs.
//!
//! Scenario traces to: D4 (ADR-0065 § 4), NEW DST invariant
//! `WorkflowBudgetExhaustionMintsTerminal` (ADR-0065 § "DST invariants"),
//! Slice 04 acceptance intent. Tags: `@in-memory` `@error` `@D4`
//! `@slice-04`.
//!
//! RED-scaffold convention (`.claude/rules/testing.md` § "RED scaffolds"):
//! the bodies below are self-contained `panic!`s importing NO unbuilt
//! production type. They remain RED until Slice 04 — DELIVER Slices 01-03
//! do NOT activate them (the retry loop they assert does not exist until
//! Slice 04). DELIVER Slice 04 replaces the panics with the real
//! re-drive-to-exhaustion bodies.

/// `@in-memory` `@error` `@D4` `@slice-04` (NEW-5) — a workflow whose
/// `ctx.run` step always fails transiently is re-driven by the engine up to
/// the engine-constant budget, then the engine MINTS `WorkflowStatus::Failed
/// { terminal: BudgetExhausted }`. The body authored NO failure (it returned
/// a step `Err` the engine classified as retryable and absorbed).
///
/// DELIVER (Slice 04) body, once the retry loop + `RetryAttempted` +
/// `TerminalError::budget_exhausted()` exist:
///
/// 1. A fixture whose `ctx.run` step closure always resolves `Err(transient)`
///    AND bumps a shared `AtomicUsize` per attempt.
/// 2. `engine.start(..)` then drive the re-drive loop to exhaustion (advance
///    `SimClock` past each backoff window so the parked re-drive fires).
/// 3. Read the `WorkflowTerminal { status }` row; assert
///    `matches!(status, WorkflowStatus::Failed { terminal } if
///    terminal.kind() == TerminalErrorKind::BudgetExhausted)`.
/// 4. `assert_eq!(attempts.load(..), WORKFLOW_RETRY_BUDGET)` — the engine
///    re-drove exactly `budget` times (not the body authoring a failure).
/// 5. The journal carries `budget`-many `RetryAttempted` commands (the
///    recomputed attempt inputs).
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn transient_failures_re_drive_to_budget_then_engine_mints_budget_exhausted() {
    panic!(
        "Not yet implemented -- RED scaffold (NEW-5 / transient failures re-driven to budget, \
         then engine-minted WorkflowStatus::Failed{{BudgetExhausted}}; ADR-0065 D4, Slice 04)"
    );
}

/// `@in-memory` `@D4` `@slice-04` (NEW-5b) — a transient step failure that
/// SUCCEEDS on a re-drive WITHIN the budget terminates
/// `WorkflowStatus::Completed` (the engine absorbed the transient and the
/// body ran to success). Proves "retryable never reaches the return type":
/// the same step `Err` that would exhaust the budget in NEW-5 instead
/// resolves to a successful terminal when the underlying transient clears
/// before exhaustion. The author body's return type is `Result<Output,
/// TerminalError>` and it returned `Ok(Output)` — the retry was invisible
/// to the body.
///
/// DELIVER body: a fixture whose step fails the first `k < budget` attempts
/// then succeeds; assert the terminal is `Completed` and the step ran
/// `k + 1` times (the engine re-drove `k` times, then the success).
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn transient_failure_that_clears_within_budget_terminates_completed() {
    panic!(
        "Not yet implemented -- RED scaffold (NEW-5b / transient that clears within budget ⇒ \
         Completed; retryable never reaches the return type; ADR-0065 D4, Slice 04)"
    );
}
