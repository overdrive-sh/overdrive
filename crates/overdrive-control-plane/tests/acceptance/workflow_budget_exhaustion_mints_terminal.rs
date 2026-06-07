//! Acceptance — a transient (retryable) failure is absorbed and re-driven
//! by the engine and NEVER reaches the body's return type; on retry-budget
//! exhaustion the engine MINTS `WorkflowStatus::Failed { terminal:
//! BudgetExhausted }` (DISTILL scaffold, `workflow-result-error-model` /
//! ADR-0065 D4; activated in Slice 04, step 04-01).
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
//! fixture re-driven a fixed number of times), NOT PBT-generated. The
//! property counterpart is the 04-02 DST invariant
//! `WorkflowBudgetExhaustionMintsTerminal`.
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
//! state"). The body NEVER returns an explicit `Err(Explicit)` — it returns
//! a `retryable` the engine absorbs and re-drives, then itself mints the
//! `BudgetExhausted` terminal on exhaustion.
//!
//! Scenario traces to: D4 (ADR-0065 § 4), NEW DST invariant
//! `WorkflowBudgetExhaustionMintsTerminal` (ADR-0065 § "DST invariants"),
//! Slice 04 acceptance intent. Tags: `@in-memory` `@error` `@D4`
//! `@slice-04`.
//!
//! # Concurrency model (the `SimClock` backoff park)
//!
//! The engine parks on `clock.sleep(backoff)` between re-drives via the
//! injected `Clock` (`development.md` § "Production code is not shaped by
//! simulation" — the SAME Clock-driven park production uses; no DST-only
//! `select!` arm). Under `SimClock` a park is a deadline-park that releases
//! only when the harness advances logical time. `engine.join_all()` would
//! block forever against a parked task with nothing advancing the clock, so
//! the test spawns a concurrent TICKER that advances the `SimClock` in small
//! steps until the engine task completes. This is the harness driving
//! logical time — the canonical DST shape — NOT a production concession.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;

use overdrive_control_plane::journal::{JournalCommand, JournalStore, LoadedEntry, WorkflowId};
use overdrive_control_plane::workflow_runtime::{
    WORKFLOW_RETRY_BUDGET, WorkflowEngine, WorkflowRegistry,
};

use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{
    RetryableStepError, TerminalError, TerminalErrorKind, Workflow, WorkflowCtx, WorkflowName,
    WorkflowStart, WorkflowStatus,
};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

/// CBOR-encode the unit `Input` the reference fixtures take.
fn cbor_unit() -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::new();
    ciborium::into_writer(&(), &mut bytes).expect("CBOR-encode unit");
    bytes
}

/// A workflow whose STEP always fails transiently: each drive bumps a
/// shared `AtomicUsize` (so the test can count how many times the engine
/// re-drove the body) and the `ctx.run_retryable` step's closure returns
/// `Err(RetryableStepError)` — the engine-absorbed TRANSIENT channel
/// (ADR-0065 §4), NEVER a terminal the body authors. The body's
/// `Result<(), TerminalError>` return type carries NO failure; the transient
/// lives in the step + the ctx. The engine re-drives up to
/// `WORKFLOW_RETRY_BUDGET`, then mints `BudgetExhausted`.
struct AlwaysTransientWorkflow {
    /// Bumped once per drive (inside the `ctx.run_retryable` step). The engine
    /// re-drives the whole body from the journal on each retry; only `Started`
    /// replays (the transient step is NOT journaled — it re-fires every drive).
    attempts: Arc<AtomicUsize>,
}

impl AlwaysTransientWorkflow {
    const WORKFLOW_NAME: &'static str = "always-transient-wf";

    fn spec() -> WorkflowStart {
        WorkflowStart {
            name: WorkflowName::new(Self::WORKFLOW_NAME).expect("valid kebab name"),
            input: cbor_unit(),
        }
    }
}

#[async_trait]
impl Workflow for AlwaysTransientWorkflow {
    type Output = ();
    type Input = ();

    async fn run(&self, ctx: &WorkflowCtx, _input: ()) -> Result<(), TerminalError> {
        // The transient failure is signalled at the STEP level: the
        // `ctx.run_retryable` closure bumps the drive counter and returns
        // `Err(RetryableStepError)`. The engine ABSORBS it (the step is not
        // journaled, the ctx records a TransientStep, `run_erased` surfaces
        // WorkflowDriveError::Transient) and re-drives. The body authors NO
        // terminal — `_step` is `Err(WorkflowCtxError::TransientStep)`, which
        // cannot become a `TerminalError`; the engine mints `BudgetExhausted`
        // once the budget is consumed.
        let attempts = Arc::clone(&self.attempts);
        let _step: Result<(), _> = ctx
            .run_retryable("provision", async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err::<(), RetryableStepError>(RetryableStepError::new(
                    "transient: provision call failed",
                ))
            })
            .await;
        // Unreachable in practice — the engine re-drives off the recorded
        // transient before the body's return value is consulted (the ctx
        // transient takes precedence in `run_erased`). Returning Ok keeps the
        // body's terminal channel empty, proving the body authored no failure.
        Ok(())
    }
}

/// A workflow whose `ctx.run_retryable` step fails transiently the first
/// `clear_after` drives, then succeeds — proving "the transient never reaches
/// the return type": the same step transient that would exhaust the budget in
/// NEW-5 instead resolves to a `Completed` terminal when the underlying
/// transient clears within budget. The body's return type is
/// `Result<(), TerminalError>` and on the clearing drive it returns `Ok(())`
/// — the retries were invisible to the body's terminal channel (the step
/// transient lived in the ctx, never in the body's return value).
struct ClearsWithinBudgetWorkflow {
    /// Bumped once per drive (inside the `ctx.run_retryable` step).
    attempts: Arc<AtomicUsize>,
    /// The number of leading drives whose step fails transiently; drive
    /// number `clear_after + 1` (1-indexed) succeeds. `clear_after < budget`.
    clear_after: usize,
}

impl ClearsWithinBudgetWorkflow {
    const WORKFLOW_NAME: &'static str = "clears-within-budget-wf";

    fn spec() -> WorkflowStart {
        WorkflowStart {
            name: WorkflowName::new(Self::WORKFLOW_NAME).expect("valid kebab name"),
            input: cbor_unit(),
        }
    }
}

#[async_trait]
impl Workflow for ClearsWithinBudgetWorkflow {
    type Output = ();
    type Input = ();

    async fn run(&self, ctx: &WorkflowCtx, _input: ()) -> Result<(), TerminalError> {
        // The step fails transiently for the first `clear_after` drives, then
        // succeeds. On a transient drive the closure returns
        // `Err(RetryableStepError)` (engine absorbs + re-drives); on the
        // clearing drive it returns `Ok(())` and the step result is journaled.
        let attempts = Arc::clone(&self.attempts);
        let clear_after = self.clear_after;
        let _step: Result<(), _> = ctx
            .run_retryable("provision", async move {
                // 1-indexed drive number after this bump.
                let drive = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                if drive <= clear_after {
                    Err(RetryableStepError::new("transient: not cleared yet"))
                } else {
                    Ok(())
                }
            })
            .await;
        // On a transient drive `run_erased` re-drives off the recorded ctx
        // transient before reaching here; on the clearing drive the step
        // returned Ok and the body returns the terminal success `Ok(())`. The
        // retries were invisible to the body's terminal channel.
        Ok(())
    }
}

/// Spawn a concurrent ticker that advances `clock` in small steps until
/// `stop` is set, so the engine's `clock.sleep(backoff)` re-drive parks
/// release under `SimClock`. The harness — never the SUT — drives logical
/// time (`.claude/rules/testing.md` § "Tier 1"); the production engine
/// parks on the injected `Clock` with no DST-only branch.
fn spawn_clock_ticker(clock: Arc<SimClock>, stop: Arc<std::sync::atomic::AtomicBool>) {
    tokio::spawn(async move {
        while !stop.load(Ordering::SeqCst) {
            // Advance well past the largest backoff so every parked re-drive
            // wakes promptly. `tokio::task::yield_now` hands control to the
            // engine task between ticks on the single-threaded runtime.
            clock.tick(Duration::from_secs(1));
            tokio::task::yield_now().await;
        }
    });
}

/// Count `RetryAttempted` commands in a loaded run.
fn retry_attempted_count(loaded: &[LoadedEntry]) -> usize {
    loaded
        .iter()
        .filter(|e| matches!(e, LoadedEntry::Command(JournalCommand::RetryAttempted { .. })))
        .count()
}

/// `@in-memory` `@error` `@D4` `@slice-04` (NEW-5) — a workflow whose body
/// always fails transiently is re-driven by the engine up to the
/// engine-constant budget, then the engine MINTS `WorkflowStatus::Failed {
/// terminal: BudgetExhausted }`. The body authored NO explicit failure (it
/// returned a `retryable` the engine classified and absorbed).
#[tokio::test]
async fn transient_failures_re_drive_to_budget_then_engine_mints_budget_exhausted() {
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let sim_clock = Arc::new(SimClock::new());
    let clock: Arc<dyn Clock> = Arc::clone(&sim_clock) as Arc<dyn Clock>;
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_factory = Arc::clone(&attempts);

    let mut registry = WorkflowRegistry::new();
    registry.register(AlwaysTransientWorkflow::spec().name, move || AlwaysTransientWorkflow {
        attempts: Arc::clone(&attempts_for_factory),
    });

    let engine = WorkflowEngine::new(
        Arc::clone(&journal),
        Arc::clone(&clock),
        Arc::clone(&transport),
        Arc::clone(&entropy),
        registry,
        Arc::clone(&obs),
    );

    let spec: WorkflowStart = AlwaysTransientWorkflow::spec();
    let correlation = CorrelationKey::derive(
        "wf-transient-0001",
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-transient-0001").expect("valid instance id");

    // Drive the SimClock concurrently so the engine's backoff parks release.
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    spawn_clock_ticker(Arc::clone(&sim_clock), Arc::clone(&stop));

    engine.start(&spec, &correlation, &workflow_id).await.expect("engine start succeeds");
    engine.join_all().await;
    stop.store(true, Ordering::SeqCst);

    // Observable outcome (b): the body ran `budget + 1` times — the INITIAL
    // drive plus `WORKFLOW_RETRY_BUDGET` re-drives (the `exit_observer`
    // "1 initial + N retries" precedent). The budget bounds the number of
    // RE-DRIVES (the durable `RetryAttempted` count below), not the total
    // drive count; the (budget+1)-th drive is the one that observes the
    // exhausted budget and the engine mints `BudgetExhausted`. The body
    // authored no failure of its own — every drive returned `retryable`,
    // which the engine absorbed.
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        WORKFLOW_RETRY_BUDGET as usize + 1,
        "the body runs the initial drive + WORKFLOW_RETRY_BUDGET re-drives before exhaustion"
    );

    // Observable outcome (c): the journal carries `budget`-many
    // `RetryAttempted` commands — the recomputed attempt INPUTS (D4). This is
    // the durable SSOT for the retry count: exactly `WORKFLOW_RETRY_BUDGET`
    // re-drives were recorded before the engine minted `BudgetExhausted`.
    let loaded = journal.load_journal(&workflow_id).await.expect("load journal");
    assert_eq!(
        retry_attempted_count(&loaded),
        WORKFLOW_RETRY_BUDGET as usize,
        "the journal records one RetryAttempted per re-drive (budget-many), the attempt inputs"
    );

    // Observable outcome (a): the WorkflowTerminal row carries Failed with
    // the engine-minted BudgetExhausted kind (the body never authored it).
    let terminals = obs.workflow_terminal_rows().await.expect("read terminal rows");
    let (_, status) = terminals
        .iter()
        .find(|(corr, _)| *corr == correlation)
        .expect("budget exhaustion must write a WorkflowTerminal row");
    assert!(
        matches!(status, WorkflowStatus::Failed { terminal } if terminal.kind() == TerminalErrorKind::BudgetExhausted),
        "budget exhaustion mints Failed{{BudgetExhausted}} (engine-minted, not body-authored), got {status:?}"
    );
}

/// `@in-memory` `@D4` `@slice-04` (NEW-5b) — a transient step failure that
/// SUCCEEDS on a re-drive WITHIN the budget terminates
/// `WorkflowStatus::Completed` (the engine absorbed the transient and the
/// body ran to success). Proves "retryable never reaches the return type":
/// the body ran `clear_after + 1` times (the engine re-drove `clear_after`
/// times, then the success).
#[tokio::test]
async fn transient_failure_that_clears_within_budget_terminates_completed() {
    // `clear_after` must be strictly below the budget so the success drive
    // is reached before exhaustion.
    let clear_after: usize = (WORKFLOW_RETRY_BUDGET as usize).saturating_sub(1).max(1);
    assert!(clear_after < WORKFLOW_RETRY_BUDGET as usize, "the transient must clear within budget");

    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let sim_clock = Arc::new(SimClock::new());
    let clock: Arc<dyn Clock> = Arc::clone(&sim_clock) as Arc<dyn Clock>;
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_factory = Arc::clone(&attempts);

    let mut registry = WorkflowRegistry::new();
    registry.register(ClearsWithinBudgetWorkflow::spec().name, move || {
        ClearsWithinBudgetWorkflow { attempts: Arc::clone(&attempts_for_factory), clear_after }
    });

    let engine = WorkflowEngine::new(
        Arc::clone(&journal),
        Arc::clone(&clock),
        Arc::clone(&transport),
        Arc::clone(&entropy),
        registry,
        Arc::clone(&obs),
    );

    let spec: WorkflowStart = ClearsWithinBudgetWorkflow::spec();
    let correlation = CorrelationKey::derive(
        "wf-clears-0001",
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-clears-0001").expect("valid instance id");

    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    spawn_clock_ticker(Arc::clone(&sim_clock), Arc::clone(&stop));

    engine.start(&spec, &correlation, &workflow_id).await.expect("engine start succeeds");
    engine.join_all().await;
    stop.store(true, Ordering::SeqCst);

    // The body ran `clear_after + 1` times (the engine re-drove `clear_after`
    // times, then the success drive) — proving the retryable never reached
    // the return type and the engine absorbed each transient.
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        clear_after + 1,
        "the body ran clear_after re-drives then one success drive"
    );

    // The journal carries exactly `clear_after` RetryAttempted commands (one
    // per absorbed transient; none for the successful drive).
    let loaded = journal.load_journal(&workflow_id).await.expect("load journal");
    assert_eq!(
        retry_attempted_count(&loaded),
        clear_after,
        "one RetryAttempted per absorbed transient, none for the success"
    );

    // The terminal is Completed — the transient cleared within budget and the
    // body returned Ok(()).
    let terminals = obs.workflow_terminal_rows().await.expect("read terminal rows");
    let (_, status) = terminals
        .iter()
        .find(|(corr, _)| *corr == correlation)
        .expect("a clearing transient must write a WorkflowTerminal row");
    assert!(
        matches!(status, WorkflowStatus::Completed { .. }),
        "a transient that clears within budget terminates Completed, got {status:?}"
    );
}
