//! Bug-fix regression — a workflow whose `async fn run` PANICS must
//! converge to a `Failed` terminal, not strand the instance.
//!
//! # The bug
//!
//! `WorkflowEngine::start` spawns the author's `run` future into a tracked
//! `JoinSet<()>`. Before the fix, a panic inside `run` unwound the spawned
//! `async move` block PAST the terminal-write (`JournalEntry::Terminal` +
//! `ObservationRow::WorkflowTerminal`) and PAST the `live_instances` cleanup.
//! `JoinSet` absorbed the panic (production never `join_next`s), so the
//! process stayed healthy but the instance was permanently stranded:
//!
//!   1. `live_instances` kept the correlation → next `hydrate_actual`
//!      derived `has_live_task = true`;
//!   2. no `WorkflowTerminal` row was written → the lifecycle reconciler's
//!      `terminal` stayed `None`.
//!
//! The workflow-lifecycle reconciler then saw "running-in-intent, no
//! terminal, has-live-task" → no convergence, no re-emit → stranded until
//! process restart.
//!
//! # The fix (shape C — `catch_unwind` + RAII drop guard)
//!
//! `start` wraps `run` in `catch_unwind`, mapping a panic to
//! `WorkflowStatus::Failed { terminal: TerminalError::explicit(<deterministic
//! downcast detail>) }` so the EXISTING terminal-write path runs and the
//! reconciler converges; and a `LiveInstanceGuard` whose
//! `Drop` removes the correlation from `live_instances` unconditionally
//! (defense-in-depth against a panic in the terminal-write itself).
//!
//! # Port-to-port
//!
//! The driving port is `WorkflowEngine::start` (the async executor off the
//! shim), driven to completion via `join_all`. The observable outcomes are
//! asserted at TWO driven-port boundaries: (a) the engine's live-instance
//! set (`live_instances()`) no longer contains the correlation — the leak is
//! fixed; (b) the injected `ObservationStore` holds a `WorkflowTerminal`
//! row keyed by the instance correlation carrying `Failed` — the terminal
//! exists, so the reconciler converges. Both are mandatory: (a) alone would
//! pass under a guard-only fix that still loops; (b) is what proves
//! convergence.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use async_trait::async_trait;

use overdrive_control_plane::journal::{JournalStore, WorkflowId};
use overdrive_control_plane::workflow_runtime::{WorkflowEngine, WorkflowRegistry};

use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{
    TerminalError, TerminalErrorKind, Workflow, WorkflowCtx, WorkflowName, WorkflowStart,
    WorkflowStatus,
};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

/// A workflow whose `async fn run` unconditionally panics — the untrusted
/// author-code failure mode the engine must contain. The engine must catch
/// the unwind, converge the instance to `Failed`, and tear down the
/// live-instance entry unconditionally.
struct PanickingWorkflow;

impl PanickingWorkflow {
    const WORKFLOW_NAME: &'static str = "panicking-workflow";

    fn spec() -> WorkflowStart {
        WorkflowStart {
            name: WorkflowName::new(Self::WORKFLOW_NAME).expect("valid kebab name"),
            // `Input = ()`: the opaque `input` MUST be the CBOR of `()` so the
            // adapter decodes it and ENTERS the body (which then panics). An
            // empty `Vec` would fail to decode and short-circuit to a
            // MalformedInput terminal BEFORE the body runs — the panic path
            // would never be exercised (ADR-0065 §1).
            input: cbor_unit(),
        }
    }
}

/// CBOR-encode the unit `Input`.
fn cbor_unit() -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::new();
    ciborium::into_writer(&(), &mut bytes).expect("CBOR-encode unit");
    bytes
}

#[async_trait]
impl Workflow for PanickingWorkflow {
    type Output = ();
    type Input = ();

    async fn run(&self, _ctx: &WorkflowCtx, _input: ()) -> Result<(), TerminalError> {
        panic!("boom");
    }
}

#[tokio::test]
async fn panicking_workflow_converges_to_failed_terminal_and_tears_down_live_instance() {
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    let mut registry = WorkflowRegistry::new();
    registry.register(PanickingWorkflow::spec().name, || PanickingWorkflow);

    let engine = WorkflowEngine::new(
        Arc::clone(&journal),
        Arc::clone(&clock),
        Arc::clone(&transport),
        Arc::clone(&entropy),
        registry,
        Arc::clone(&obs),
    );

    let spec: WorkflowStart = PanickingWorkflow::spec();
    let correlation = CorrelationKey::derive(
        "wf-panic-0001",
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-panic-0001").expect("valid instance id");

    engine.start(&spec, &correlation, &workflow_id).await.expect("engine start succeeds");
    // join_all drains the JoinSet — load-bearing; without it the assertions
    // race the spawned task.
    engine.join_all().await;

    // Observable outcome (a): the live-instance entry is gone. Before the
    // fix the panic skipped the cleanup and this set still contained the
    // correlation → hydrate_actual derived has_live_task = true → no
    // re-emit → permanent strand.
    let live = engine.live_instances();
    assert!(
        !live.contains(&correlation),
        "a panicked workflow must NOT leave a live-instance entry (the leak that stranded the instance)"
    );

    // Observable outcome (b): a WorkflowTerminal row keyed by the instance
    // correlation, carrying Failed. The terminal's existence is what lets
    // the workflow-lifecycle reconciler converge (terminal = Some(Failed)
    // → converged); before the fix the panic skipped the terminal write and
    // terminal stayed None → no convergence.
    let terminals = obs.workflow_terminal_rows().await.expect("read terminal rows");
    let terminal = terminals.iter().find(|(corr, _)| *corr == correlation);
    let (_, status) = terminal.expect(
        "a panicked workflow must write a WorkflowTerminal row so the reconciler converges",
    );
    // The engine maps the contained panic to a `WorkflowStatus::Failed`
    // carrying a `TerminalError::explicit` whose detail is the deterministic
    // downcast of the panic message (ADR-0065 §1/§3) — never the
    // address-bearing raw box, so the durable terminal stays byte-stable.
    assert!(
        matches!(status, WorkflowStatus::Failed { terminal } if terminal.kind() == TerminalErrorKind::Explicit),
        "a panicked workflow must converge to a Failed{{Explicit}} terminal, got {status:?}"
    );
    // The panic message ("boom") survives as the deterministic detail.
    let WorkflowStatus::Failed { terminal } = status else {
        panic!("status must be Failed, got {status:?}");
    };
    assert_eq!(terminal.detail(), "boom", "the panic message is the deterministic terminal detail");
}
