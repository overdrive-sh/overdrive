//! Acceptance — a per-`ctx.run` [`RunRetryPolicy`] (ADR-0065 Amendment
//! 2026-06-07, Gap 2) governs the engine's whole-workflow re-drive count: a
//! step that sets `max_attempts = 1` exhausts after exactly ONE re-drive
//! (NOT the engine-global `WORKFLOW_RETRY_BUDGET` of 3), minting
//! `WorkflowStatus::Failed { BudgetExhausted }`. The PER-STEP policy, not the
//! global constant, gates the decision.
//!
//! # Why NEW
//!
//! Gap 2 is genuinely-new behaviour: pre-Gap-2 the engine re-drove against a
//! single engine-wide `WORKFLOW_RETRY_BUDGET` constant with no per-step
//! override. The `RunStep` builder's `.retry_policy(..)` and the failing
//! step's policy riding the transient signal into the engine's re-drive
//! decision have no existing test. The companion default-reproduces-today
//! behaviour is covered UNCHANGED by
//! `workflow_budget_exhaustion_mints_terminal.rs` (which sets no policy and
//! still exhausts at `WORKFLOW_RETRY_BUDGET`); this test pins the override.
//!
//! # Layer / paradigm
//!
//! Layer 1-2 (engine drives over `Sim*` ports; the forced-transient workflow
//! and the per-step policy are deterministic under `SimClock`). Per Mandate 11
//! this engine-boundary sad path is EXAMPLE-based (a forced-transient fixture
//! whose step carries `max_attempts = 1`, re-driven a fixed number of times),
//! NOT PBT-generated. The property counterpart is the Gap-2 DST invariant
//! `WorkflowPerStepRetryPolicyGovernsRedrive`.
//!
//! # Port-to-port
//!
//! The driving port is `WorkflowEngine::start`. The observable outcomes are
//! asserted at the driven-port boundaries: (a) the `ObservationStore`
//! `WorkflowTerminal { status }` row carries `WorkflowStatus::Failed {
//! terminal: kind == BudgetExhausted }`; (b) the body's transient step ran
//! exactly `policy.max_attempts + 1` (== 2) times across the re-drives — the
//! INITIAL drive + ONE re-drive — proving the per-step policy (1), not the
//! global constant (3), gated the loop; (c) the journal carries exactly
//! `policy.max_attempts` (== 1) `RetryAttempted` commands. The body NEVER
//! returns an explicit `Err` — it returns a `retryable` the engine absorbs
//! and re-drives per the FAILING step's policy, then mints `BudgetExhausted`.
//!
//! Scenario traces to: ADR-0065 Gap 2 ("per-`ctx.run` retry policy"), NEW DST
//! invariant `WorkflowPerStepRetryPolicyGovernsRedrive`. Tags: `@in-memory`
//! `@error` `@gap-2`.
//!
//! # Concurrency model (the `SimClock` backoff park)
//!
//! Identical to `workflow_budget_exhaustion_mints_terminal.rs`: the engine
//! parks on `clock.sleep(backoff)` between re-drives via the injected `Clock`
//! (no DST-only branch), and the test spawns a concurrent ticker that advances
//! the `SimClock` until the engine task completes — the harness driving
//! logical time, the canonical DST shape.

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
    RunRetryPolicy, StepError, TerminalError, TerminalErrorKind, Workflow, WorkflowCtx,
    WorkflowName, WorkflowStart, WorkflowStatus,
};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

/// The per-step `max_attempts` this fixture sets — deliberately `1`, distinct
/// from the engine-global `WORKFLOW_RETRY_BUDGET` (3), so the test proves the
/// PER-STEP policy governs.
const STEP_MAX_ATTEMPTS: u32 = 1;

/// CBOR-encode the unit `Input` the fixture takes.
fn cbor_unit() -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::new();
    ciborium::into_writer(&(), &mut bytes).expect("CBOR-encode unit");
    bytes
}

/// A workflow whose `ctx.run` STEP always fails transiently AND carries an
/// explicit per-step [`RunRetryPolicy`] of `max_attempts = STEP_MAX_ATTEMPTS`
/// (1) via the [`RunStep`] builder's `.retry_policy(..)`. Each drive bumps a
/// shared `AtomicUsize` and the step's closure returns
/// `Err(StepError::Retryable)` — the engine-absorbed transient channel. The
/// engine re-drives per THIS step's policy (1 re-drive), NOT the global
/// `WORKFLOW_RETRY_BUDGET` (3), then mints `BudgetExhausted`.
struct PerStepPolicyTransientWorkflow {
    /// Bumped once per drive (inside the `ctx.run` step).
    attempts: Arc<AtomicUsize>,
}

impl PerStepPolicyTransientWorkflow {
    const WORKFLOW_NAME: &'static str = "per-step-policy-transient-wf";

    fn spec() -> WorkflowStart {
        WorkflowStart {
            name: WorkflowName::new(Self::WORKFLOW_NAME).expect("valid kebab name"),
            input: cbor_unit(),
        }
    }
}

#[async_trait]
impl Workflow for PerStepPolicyTransientWorkflow {
    type Output = ();
    type Input = ();

    async fn run(&self, ctx: &WorkflowCtx, _input: ()) -> Result<(), TerminalError> {
        let attempts = Arc::clone(&self.attempts);
        // The `.retry_policy(..)` builder sets the FAILING step's policy
        // (max_attempts = 1). The transient PARKS the body; the engine reads
        // the recorded transient's policy and re-drives per IT — exactly once,
        // not WORKFLOW_RETRY_BUDGET times.
        ctx.run("provision", async move {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err::<(), StepError>(StepError::retryable("transient: provision call failed"))
        })
        .retry_policy(RunRetryPolicy { max_attempts: STEP_MAX_ATTEMPTS, ..Default::default() })
        .await?;
        // Unreachable on the always-transient path (the engine cancels the
        // parked body before this returns); present so the body type-checks.
        Ok(())
    }
}

/// Spawn a concurrent ticker that advances `clock` until `stop` is set, so the
/// engine's `clock.sleep(backoff)` re-drive parks release under `SimClock`.
fn spawn_clock_ticker(clock: Arc<SimClock>, stop: Arc<std::sync::atomic::AtomicBool>) {
    tokio::spawn(async move {
        while !stop.load(Ordering::SeqCst) {
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

/// `@in-memory` `@error` `@gap-2` — a workflow whose step sets
/// `.retry_policy(RunRetryPolicy { max_attempts: 1, .. })` exhausts after
/// exactly ONE re-drive (the PER-STEP policy), NOT the engine-global
/// `WORKFLOW_RETRY_BUDGET` of 3, then the engine mints `Failed {
/// BudgetExhausted }`.
#[tokio::test]
async fn per_step_retry_policy_governs_redrive_count_not_the_global_budget() {
    // Guard the premise: the per-step policy must DIFFER from the global budget
    // for this test to prove anything (if they were equal, exhausting at 1
    // would be indistinguishable from exhausting at the global constant).
    assert_ne!(
        STEP_MAX_ATTEMPTS, WORKFLOW_RETRY_BUDGET,
        "the per-step policy must differ from the global budget to prove per-step governance"
    );

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
    registry.register(PerStepPolicyTransientWorkflow::spec().name, move || {
        PerStepPolicyTransientWorkflow { attempts: Arc::clone(&attempts_for_factory) }
    });

    let engine = WorkflowEngine::new(
        Arc::clone(&journal),
        Arc::clone(&clock),
        Arc::clone(&transport),
        Arc::clone(&entropy),
        registry,
        Arc::clone(&obs),
    );

    let spec: WorkflowStart = PerStepPolicyTransientWorkflow::spec();
    let correlation = CorrelationKey::derive(
        "wf-per-step-policy-0001",
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-per-step-policy-0001").expect("valid instance id");

    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    spawn_clock_ticker(Arc::clone(&sim_clock), Arc::clone(&stop));

    engine.start(&spec, &correlation, &workflow_id).await.expect("engine start succeeds");
    engine.join_all().await;
    stop.store(true, Ordering::SeqCst);

    // Observable outcome (b): the body ran `STEP_MAX_ATTEMPTS + 1` (== 2)
    // times — the INITIAL drive + exactly ONE re-drive (the per-step policy),
    // NOT `WORKFLOW_RETRY_BUDGET + 1` (== 4). This is the proof the PER-STEP
    // policy, not the global constant, gated the re-drive count.
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        STEP_MAX_ATTEMPTS as usize + 1,
        "the per-step max_attempts (1) governs: initial drive + 1 re-drive, NOT the global \
         WORKFLOW_RETRY_BUDGET ({WORKFLOW_RETRY_BUDGET}) + 1"
    );

    // Observable outcome (c): the journal carries exactly `STEP_MAX_ATTEMPTS`
    // (== 1) `RetryAttempted` commands — the durable SSOT for the re-drive
    // count under the per-step policy.
    let loaded = journal.load_journal(&workflow_id).await.expect("load journal");
    assert_eq!(
        retry_attempted_count(&loaded),
        STEP_MAX_ATTEMPTS as usize,
        "the journal records exactly the per-step max_attempts (1) RetryAttempted commands"
    );

    // Observable outcome (a): the WorkflowTerminal row carries Failed with the
    // engine-minted BudgetExhausted kind (the body never authored it).
    let terminals = obs.workflow_terminal_rows().await.expect("read terminal rows");
    let (_, status) = terminals
        .iter()
        .find(|(corr, _)| *corr == correlation)
        .expect("per-step-policy exhaustion must write a WorkflowTerminal row");
    assert!(
        matches!(status, WorkflowStatus::Failed { terminal } if terminal.kind() == TerminalErrorKind::BudgetExhausted),
        "per-step-policy exhaustion mints Failed{{BudgetExhausted}} (engine-minted), got {status:?}"
    );
}
