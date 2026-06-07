//! Acceptance — a `ctx.run` step that resolves to `Err(StepError::Terminal)`
//! fails the workflow TERMINALLY: `ctx.run(...).await` yields
//! `Err(TerminalError)` (no transient-slot record, no body-park, no
//! re-drive), the body returns that terminal, and the engine projects
//! `WorkflowStatus::Failed { terminal: kind == Explicit }`. The capability
//! is NEW under ADR-0065 Gap 1: before Gap 1 the `ctx.run` closure error was
//! retryable-ONLY (`RetryableStepError`), so a step could not deliberately
//! fail terminally — every step error was absorbed and re-driven. The
//! `retryable | terminal` step-error union (`StepError`) adds the terminal
//! arm; this test pins it.
//!
//! # Why NEW
//!
//! The step-error union's `Terminal` arm is the genuinely-new behaviour Gap 1
//! introduces — a step CAN now short-circuit the whole workflow with a
//! permanent failure, never re-driven. No existing test exercises it: the
//! `workflow_budget_exhaustion_mints_terminal` sibling exercises the
//! `Retryable` arm (engine-absorbed + re-driven to budget); this exercises the
//! `Terminal` arm (propagated, NOT re-driven). The contrast between the two is
//! the whole point of the union.
//!
//! # Layer / paradigm
//!
//! Layer 1-2 (the engine drives over `Sim*` ports; the forced-terminal
//! workflow is deterministic under `SimClock`). Per Mandate 11 this
//! engine-boundary sad path is EXAMPLE-based (a single forced-terminal
//! fixture), NOT PBT-generated — there is one observable behaviour (terminal
//! short-circuit), not an equivalence class of inputs.
//!
//! # Port-to-port
//!
//! The driving port is `WorkflowEngine::start`. The observable outcomes are
//! asserted at the driven-port boundaries:
//!   (a) the `ObservationStore` `WorkflowTerminal { status }` row carries
//!       `WorkflowStatus::Failed { terminal: kind == Explicit }` — the
//!       step's terminal, projected verbatim;
//!   (b) the journal carries ZERO `RetryAttempted` commands — the engine did
//!       NOT re-drive (a terminal is never re-driven, contrast the budget
//!       sibling's `budget`-many `RetryAttempted`);
//!   (c) a `ctx.run` step AFTER the terminal one NEVER runs — a shared
//!       `AtomicUsize` stays `0`, proving the terminal short-circuited the
//!       body at the failing step rather than continuing.
//!
//! Scenario traces to: ADR-0065 Amendment (2026-06-07) Gap 1, DST invariant
//! `WorkflowStepTerminalShortCircuits`. Tags: `@in-memory` `@error` `@gap-1`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;

use overdrive_control_plane::journal::{JournalCommand, JournalStore, LoadedEntry, WorkflowId};
use overdrive_control_plane::workflow_runtime::{WorkflowEngine, WorkflowRegistry};

use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{
    StepError, TerminalError, TerminalErrorKind, Workflow, WorkflowCtx, WorkflowName,
    WorkflowStart, WorkflowStatus,
};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

/// CBOR-encode the unit `Input` the fixture takes.
fn cbor_unit() -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::new();
    ciborium::into_writer(&(), &mut bytes).expect("CBOR-encode unit");
    bytes
}

/// The detail the failing step's terminal carries — asserted on the projected
/// terminal row so the test pins the step-authored terminal, not an
/// engine-minted one.
const TERMINAL_DETAIL: &str = "permanent: provision rejected (Gap 1 step-terminal)";

/// A workflow whose FIRST `ctx.run` step resolves to
/// `Err(StepError::Terminal)` — a permanent step failure. A SECOND `ctx.run`
/// step follows it and bumps `second_step_runs`; the second step must NEVER
/// run, because the terminal from the first `ctx.run(...).await` propagates
/// via `?` and short-circuits the body. The body's `Result<(), TerminalError>`
/// return carries the step's terminal directly (no engine re-drive).
struct StepFailsTerminallyWorkflow {
    /// Bumped iff the SECOND step's closure runs. Stays `0` on the
    /// terminal-short-circuit path (the first step's terminal short-circuits
    /// the body before the second `ctx.run` is reached).
    second_step_runs: Arc<AtomicUsize>,
}

impl StepFailsTerminallyWorkflow {
    const WORKFLOW_NAME: &'static str = "step-fails-terminally-wf";

    fn spec() -> WorkflowStart {
        WorkflowStart {
            name: WorkflowName::new(Self::WORKFLOW_NAME).expect("valid kebab name"),
            input: cbor_unit(),
        }
    }
}

#[async_trait]
impl Workflow for StepFailsTerminallyWorkflow {
    type Output = ();
    type Input = ();

    async fn run(&self, ctx: &WorkflowCtx, _input: ()) -> Result<(), TerminalError> {
        // The first step resolves to `Err(StepError::Terminal(..))` — a
        // PERMANENT step failure (Gap 1). `ctx.run(...).await` returns
        // `Err(TerminalError)` directly (no transient-slot record, no body
        // park, no re-drive); the `?` propagates it and the body returns the
        // terminal. The terminal's detail is asserted on the projected
        // terminal row.
        ctx.run("provision", async move {
            Err::<(), StepError>(StepError::terminal(TerminalError::explicit(TERMINAL_DETAIL)))
        })
        .await?;

        // The second step must NEVER run: the first `ctx.run` above
        // short-circuited the body with the terminal. If the engine had
        // (incorrectly) re-driven or swallowed the terminal, this closure
        // would bump the counter — the assertion that it stays `0` is the
        // observable proof the terminal short-circuited.
        let second_step_runs = Arc::clone(&self.second_step_runs);
        ctx.run("after-terminal", async move {
            second_step_runs.fetch_add(1, Ordering::SeqCst);
            Ok::<(), StepError>(())
        })
        .await?;
        Ok(())
    }
}

/// Count `RetryAttempted` commands in a loaded run.
fn retry_attempted_count(loaded: &[LoadedEntry]) -> usize {
    loaded
        .iter()
        .filter(|e| matches!(e, LoadedEntry::Command(JournalCommand::RetryAttempted { .. })))
        .count()
}

/// `@in-memory` `@error` `@gap-1` — a `ctx.run` step that resolves to
/// `Err(StepError::Terminal)` fails the workflow terminally: the engine
/// projects `WorkflowStatus::Failed { terminal: Explicit }`, records ZERO
/// `RetryAttempted` commands (no re-drive), and a step after the terminal one
/// never runs.
#[tokio::test]
async fn step_terminal_short_circuits_body_with_no_redrive() {
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    let second_step_runs = Arc::new(AtomicUsize::new(0));
    let second_step_runs_for_factory = Arc::clone(&second_step_runs);

    let mut registry = WorkflowRegistry::new();
    registry.register(StepFailsTerminallyWorkflow::spec().name, move || {
        StepFailsTerminallyWorkflow { second_step_runs: Arc::clone(&second_step_runs_for_factory) }
    });

    let engine = WorkflowEngine::new(
        Arc::clone(&journal),
        Arc::clone(&clock),
        Arc::clone(&transport),
        Arc::clone(&entropy),
        registry,
        Arc::clone(&obs),
    );

    let spec: WorkflowStart = StepFailsTerminallyWorkflow::spec();
    let correlation = CorrelationKey::derive(
        "wf-step-terminal-0001",
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-step-terminal-0001").expect("valid instance id");

    // No clock ticker needed: a terminal short-circuits on the FIRST drive
    // with no backoff park (contrast the budget sibling, which parks on
    // `clock.sleep(backoff)` between re-drives).
    engine.start(&spec, &correlation, &workflow_id).await.expect("engine start succeeds");
    engine.join_all().await;

    // Observable outcome (c): the second step NEVER ran — the first step's
    // terminal short-circuited the body before the second `ctx.run`.
    assert_eq!(
        second_step_runs.load(Ordering::SeqCst),
        0,
        "a step after the terminal one must never run — the terminal short-circuits the body"
    );

    // Observable outcome (b): ZERO RetryAttempted commands — the engine did
    // NOT re-drive (a terminal is never re-driven). This is the structural
    // contrast with the budget sibling (which records `budget`-many).
    let loaded = journal.load_journal(&workflow_id).await.expect("load journal");
    assert_eq!(
        retry_attempted_count(&loaded),
        0,
        "a terminal step is never re-driven, so the journal records no RetryAttempted"
    );

    // Observable outcome (a): the WorkflowTerminal row carries Failed with the
    // step-authored Explicit kind and its detail (NOT engine-minted).
    let terminals = obs.workflow_terminal_rows().await.expect("read terminal rows");
    let (_, status) = terminals
        .iter()
        .find(|(corr, _)| *corr == correlation)
        .expect("a terminal step must write a WorkflowTerminal row");
    assert!(
        matches!(
            status,
            WorkflowStatus::Failed { terminal }
                if terminal.kind() == TerminalErrorKind::Explicit
                    && terminal.detail() == TERMINAL_DETAIL
        ),
        "a step terminal projects Failed{{Explicit}} carrying the step's detail, got {status:?}"
    );
}
