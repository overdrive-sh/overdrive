//! workflow-primitive slice 03 — step 03-03 — end-to-end production
//! emit→drain proof (ADR-0064 §4 the `ctx.emit_action` → Action channel
//! → Raft commit path; brief.md §92).
//!
//! Scenario
//! `emitting_workflow_ctx_emit_action_flows_through_production_composition_to_the_action_dispatch_path`
//! — a fixture trigger reconciler emits `Action::StartWorkflow` for an
//! `EmittingWorkflow` → the FULL production composition (the real
//! `WorkflowEngine` composed into `AppState`, the production
//! `action_shim::dispatch_with_workflow_intent`, AND the production
//! emit-drain task `spawn_workflow_emit_drain`) drives the
//! `EmittingWorkflow`'s `run` body (`ctx.run` → `ctx.emit_action(<a
//! second StartWorkflow for ProvisionRecord>)` → terminal) → the
//! production drain task forwards the emitted `StartWorkflow` into the
//! SAME `action_shim` dispatch path → the second workflow (`ProvisionRecord`)
//! runs and writes its own `WorkflowTerminal` `ObservationStore` row.
//!
//! The observable downstream effect that proves the emitted Action flowed
//! through the REAL composition is the SECOND workflow's terminal row: it
//! appears ONLY if the emitted `StartWorkflow` reached
//! `action_shim::dispatch` off the shim via the production drain task. No
//! test-injected drain anywhere — `spawn_workflow_emit_drain` is the
//! genuine production mechanism `run_server` wires at boot.
//!
//! # Why this step exists (the gap 03-01 left)
//!
//! Step 03-01 built `ctx.emit_action` so the engine SENDS the typed Action
//! on its own internal Action channel, but `run_server` never TOOK the
//! receiver — so in PRODUCTION an emitted Action was undrained, never
//! reaching the action-shim/Raft commit path. This step wires the
//! production drain (`spawn_workflow_emit_drain`) and proves it e2e.
//!
//! # Port-to-port
//!
//! The driving boundary is the production
//! `action_shim::dispatch_with_workflow_intent` carrying the action the
//! fixture trigger reconciler emitted, threaded the real engine from
//! `AppState::workflow_engine` AND the production emit-drain task. The
//! observable outcome asserted at the driven boundary: the SECOND
//! workflow's `WorkflowTerminal` `ObservationStore` row keyed by the
//! second instance's `CorrelationKey` — a downstream effect of the REAL
//! dispatch path, never a test-injected capture of the channel.
//!
//! # Why a fixture trigger reconciler + a test-scope `EmittingWorkflow` (AC5)
//!
//! Phase 1 has no production `ctx.emit_action` producer (#206 CLI verb +
//! Phase-3 consumers are the future real producers). The fixture trigger
//! reconciler and the `EmittingWorkflow` are the made-up emitters the step
//! supplies to exercise the now-complete production emit-drain path; they
//! live here in test scope, NOT production `src`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_control_plane::workflow_runtime::{WorkflowEngine, WorkflowRegistry};
use overdrive_control_plane::{
    AppState, noop_heartbeat, spawn_workflow_emit_drain, workflow_lifecycle, workload_lifecycle,
};

use overdrive_core::UnixInstant;
use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::testing::workflow::ProvisionRecord;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{Workflow, WorkflowCtx, WorkflowName, WorkflowResult, WorkflowSpec};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::transport::SimTransport;

use overdrive_store_local::{LocalIntentStore, LocalObservationStore};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// The fixture trigger reconciler (AC5). NOT an `AnyReconciler` variant —
/// a test-scope emitter that produces the `Action::StartWorkflow` a future
/// production producer (#206) would emit. Its single job is to hand the
/// production composition the action that starts the `EmittingWorkflow`.
struct FixtureTriggerReconciler {
    spec: WorkflowSpec,
    correlation: CorrelationKey,
}

impl FixtureTriggerReconciler {
    fn emit(&self) -> Vec<Action> {
        vec![Action::StartWorkflow {
            spec: self.spec.clone(),
            correlation: self.correlation.clone(),
        }]
    }
}

/// The test-scope emitting workflow (AC5): a `ctx.run` durable step
/// followed by a single `ctx.emit_action(<a StartWorkflow for
/// ProvisionRecord>)`, then terminal `Success`. The emitted Action is an
/// OBSERVABLE one — a second `StartWorkflow` whose dispatch off the shim
/// runs `ProvisionRecord` and writes a `WorkflowTerminal` row — so the
/// assertion rides the REAL composition's downstream effect, not a mock.
///
/// The body has no `IntentStore` handle; `ctx.emit_action` is the only
/// mutation it can express, and it routes through the engine's Action
/// channel (→ the production drain → Raft) by construction.
struct EmittingWorkflow {
    /// Where the inner `ctx.run` provision-write effect is addressed.
    run_target: SocketAddr,
    /// The second `StartWorkflow` action this workflow emits.
    emitted: Action,
}

impl EmittingWorkflow {
    const WORKFLOW_NAME: &'static str = "emitting-trigger-workflow";

    fn spec() -> WorkflowSpec {
        WorkflowSpec {
            name: WorkflowName::new(Self::WORKFLOW_NAME).expect("valid kebab name"),
            input: Vec::new(),
        }
    }
}

#[async_trait]
impl Workflow for EmittingWorkflow {
    async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult {
        // One durable step (the `ctx.run` await-point that precedes the
        // emit per the step's `ctx.run → ctx.emit_action → terminal`
        // shape), then the emit. The `ctx.run` effect is a transport send
        // — a real durable step, journaled exactly-once across replay.
        let transport = Arc::clone(ctx.transport());
        let target = self.run_target;
        let sent: Result<usize, String> = ctx
            .run("emit-trigger-run", async move {
                transport
                    .send_datagram(target, bytes::Bytes::from_static(b"emit-trigger"))
                    .await
                    .map_err(|e| e.to_string())
            })
            .await
            .unwrap_or_else(|err| Err(err.to_string()));
        if sent.is_err() {
            return WorkflowResult::Failed { reason: "run step failed".to_string() };
        }
        // The workflow→cluster mutation: emit the second StartWorkflow on
        // the Action channel (→ the production drain → Raft). Exactly-once
        // across a crash (a recorded `ActionEmitted` makes a resumed run
        // NOT re-emit).
        match ctx.emit_action(self.emitted.clone()).await {
            Ok(()) => WorkflowResult::Success,
            Err(_) => WorkflowResult::Failed { reason: "emit failed".to_string() },
        }
    }
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "single end-to-end golden walkthrough: real composition setup + the production \
              emit-drain spawn + the single downstream-effect assertion (the SECOND workflow's \
              terminal row, proving the emitted Action was dispatched off the shim) are one \
              indivisible scenario — splitting would obscure the emit→drain→dispatch pipeline \
              this test exists to prove."
)]
async fn emitting_workflow_ctx_emit_action_flows_through_production_composition_to_the_action_dispatch_path()
 {
    let tmp = TempDir::new().expect("tempdir");
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let transport = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));

    // The SECOND workflow's provision-write target. Binding an inbox here
    // is not required for the assertion (we ride the terminal obs row),
    // but ProvisionRecord's ctx.run send needs a destination.
    let provision_target: SocketAddr = "127.0.0.1:9100".parse().expect("valid addr");
    // The EMITTING workflow's own inner ctx.run target.
    let emit_run_target: SocketAddr = "127.0.0.1:9200".parse().expect("valid addr");

    // Register BOTH workflow kinds in the engine: the emitting trigger
    // workflow AND the ProvisionRecord the emit starts. Production
    // registers real first-party workflows here at boot; Phase 1 has none,
    // so the e2e registers the fixtures to exercise the real composition.
    let provision_spec: WorkflowSpec = ProvisionRecord::spec();
    let second_correlation = CorrelationKey::derive(
        provision_target.to_string().as_str(),
        &ContentHash::of(provision_spec.name.as_str().as_bytes()),
        "emitted-start-workflow",
    );
    let emitted_action = Action::StartWorkflow {
        spec: provision_spec.clone(),
        correlation: second_correlation.clone(),
    };

    let mut registry = WorkflowRegistry::new();
    registry.register(ProvisionRecord::spec().name, move || {
        Box::new(ProvisionRecord::new(provision_target))
    });
    let emitted_for_factory = emitted_action.clone();
    registry.register(EmittingWorkflow::spec().name, move || {
        Box::new(EmittingWorkflow {
            run_target: emit_run_target,
            emitted: emitted_for_factory.clone(),
        })
    });

    // --- Real observation store (real redb) shared by the engine (writes
    //     both workflows' terminal rows) and the runtime.
    let obs: Arc<dyn ObservationStore> =
        Arc::new(LocalObservationStore::open(tmp.path().join("obs.redb")).expect("open obs store"));

    // --- Real journal store (real redb) for the engine's durable runs.
    let journal_db = Arc::new(
        redb::Database::create(tmp.path().join("journal.redb")).expect("create journal redb"),
    );
    let journal = Arc::new(overdrive_control_plane::journal::RedbJournalStore::new(journal_db));

    let transport_dyn: Arc<dyn Transport> = transport.clone();
    let engine = Arc::new(WorkflowEngine::new(
        journal,
        Arc::clone(&clock),
        transport_dyn,
        Arc::clone(&entropy),
        registry,
        Arc::clone(&obs),
    ));

    // --- Real reconciler runtime composing the production reconcilers.
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
    runtime.register(workload_lifecycle()).await.expect("register job-lifecycle");
    runtime.register(workflow_lifecycle()).await.expect("register workflow-lifecycle");

    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator = overdrive_control_plane::test_default_allocator(
        Arc::clone(&store) as Arc<dyn overdrive_core::traits::intent_store::IntentStore>
    );

    // The real AppState composition carrying the real engine.
    let state = AppState::new_with_workflow_engine(
        store,
        store_path,
        Arc::clone(&obs),
        Arc::new(runtime),
        driver,
        Arc::clone(&clock),
        Arc::new(SimDataplane::new()),
        NodeId::new("local").expect("node id"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
        Arc::clone(&engine),
    );

    // === Spawn the PRODUCTION emit-drain task — the genuine mechanism
    //     `run_server` wires at boot. It takes the engine's emit receiver
    //     ONCE and forwards each emitted Action into the SAME
    //     `action_shim` dispatch path (→ Raft). NOT a test-injected drain.
    let drain_shutdown = CancellationToken::new();
    let drain_task =
        spawn_workflow_emit_drain(state.clone(), Arc::clone(&clock), drain_shutdown.clone());

    // --- The fixture trigger reconciler emits StartWorkflow for the
    //     EmittingWorkflow. The correlation is derived the same shape a
    //     real producer would use.
    let emitting_spec: WorkflowSpec = EmittingWorkflow::spec();
    let first_correlation = CorrelationKey::derive(
        emit_run_target.to_string().as_str(),
        &ContentHash::of(emitting_spec.name.as_str().as_bytes()),
        "start-emitting-workflow",
    );
    let trigger =
        FixtureTriggerReconciler { spec: emitting_spec.clone(), correlation: first_correlation };

    // Commit the FIRST action through the production action-shim dispatch
    // (the production commit point a reconciler's emitted actions flow
    // through). This drives: emit → dispatch → engine starts
    // EmittingWorkflow → run → ctx.run + ctx.emit_action(second
    // StartWorkflow) → the emitted Action lands on the engine's channel.
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };
    overdrive_control_plane::action_shim::dispatch_with_workflow_intent(
        trigger.emit(),
        &state,
        &tick,
    )
    .await
    .expect("StartWorkflow dispatch must succeed");

    // === The observable downstream effect: the SECOND workflow
    //     (ProvisionRecord) reaches terminal ONLY because the production
    //     drain task forwarded the emitted StartWorkflow into
    //     action_shim::dispatch. Poll the terminal-row surface until the
    //     second instance's row appears (the engine drives both run bodies
    //     as tracked tasks; the drain → dispatch → start chain is
    //     async). A bounded poll: if the emitted Action were undrained
    //     (the gap this step closes), the second row would NEVER appear.
    let second_result =
        await_terminal_row(obs.as_ref(), &second_correlation, Duration::from_secs(5))
            .await
            .expect("the emitted StartWorkflow must drive ProvisionRecord to a terminal row");
    assert_eq!(
        second_result,
        WorkflowResult::Success,
        "the second (emitted) workflow's terminal row must carry its terminal result — \
         it ran ONLY because the emitted Action flowed through the production drain → dispatch"
    );

    // Exactly-once across replay: the emitted Action is dispatched once,
    // so exactly ONE terminal row exists for the second correlation (a
    // re-drain or re-dispatch would write a second row at the same key).
    let terminals = obs.workflow_terminal_rows().await.expect("read terminal rows");
    let second_rows = terminals.iter().filter(|(corr, _)| *corr == second_correlation).count();
    assert_eq!(
        second_rows, 1,
        "the emitted Action must be dispatched EXACTLY once — a single terminal row \
         for the second instance, never re-dispatched across replay"
    );

    // Shut the drain task down cleanly.
    drain_shutdown.cancel();
    let _ = drain_task.await;
}

/// Poll the `WorkflowTerminal` observation surface until a row keyed by
/// `correlation` appears, or `budget` elapses. Returns the terminal
/// result on success, `None` on timeout. The poll is the honest shape for
/// observing the async drain → dispatch → engine-start → terminal chain:
/// the second workflow runs on a tracked engine task spawned by the drain
/// task, so the row materialises after a bounded async delay, not
/// synchronously.
async fn await_terminal_row(
    obs: &dyn ObservationStore,
    correlation: &CorrelationKey,
    budget: Duration,
) -> Option<WorkflowResult> {
    let deadline = Instant::now() + budget;
    loop {
        let terminals = obs.workflow_terminal_rows().await.expect("read terminal rows");
        if let Some((_, result)) = terminals.iter().find(|(corr, _)| *corr == *correlation) {
            return Some(result.clone());
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
