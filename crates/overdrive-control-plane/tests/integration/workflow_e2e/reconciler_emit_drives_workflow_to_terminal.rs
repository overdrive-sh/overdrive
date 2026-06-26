//! workflow-primitive slice 01 — step 01-08 — end-to-end production
//! composition proof (ADR-0064 §5 the composition path diagram).
//!
//! Scenario
//! `fixture_reconciler_emit_start_workflow_drives_provision_record_to_terminal_through_production_composition`
//! — a fixture trigger reconciler emits `Action::StartWorkflow` →
//! the FULL production composition (the real `WorkflowEngine` composed
//! into `AppState`, the production `action_shim::dispatch`, the workflow-
//! instance intent persistence, and the `WorkflowLifecycle` reconciler's
//! real `hydrate_desired` / `hydrate_actual`) drives `ProvisionRecord`'s
//! `run` to terminal → the `WorkflowTerminal` `ObservationStore` row
//! appears keyed by the same `CorrelationKey` → the lifecycle reconciler
//! converges the instance to terminated; the `ctx.run` effect fired
//! exactly once.
//!
//! This is the user-requested proof that the FULL pipeline works
//! end-to-end (not just the sim-constructed engine of 01-07): a committed
//! `StartWorkflow` actually drives a workflow to terminal through
//! `AppState`'s engine.
//!
//! # Port-to-port
//!
//! The driving boundary is the production `action_shim::dispatch` (the
//! runtime's async I/O boundary, ADR-0023) carrying the action the fixture
//! trigger reconciler emitted, threaded the real engine from
//! `AppState::workflow_engine`. The observable outcomes asserted at the
//! driven boundaries: (1) the `WorkflowTerminal` `ObservationStore` row
//! keyed by the instance `CorrelationKey`; (2) the engine task completed
//! (`join_all` returns, `live_instances` is empty); (3) the `ctx.run`
//! transport effect fired exactly once (`SimTransport` call count); (4)
//! the `WorkflowLifecycle` reconciler converges the instance to
//! terminated (its pure reconcile emits no `StartWorkflow` once the
//! terminal row is observed).
//!
//! # Why a fixture trigger reconciler (AC5)
//!
//! Phase 1 has no production `StartWorkflow` producer (#206 CLI verb +
//! Phase-3 consumers are the future real triggers). The fixture trigger
//! reconciler is the made-up emitter the step supplies to exercise the
//! now-real production composition end-to-end; it lives here in test
//! scope, NOT production `src`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, hydrate_actual_for_test, hydrate_desired_for_test, run_convergence_tick,
};
use overdrive_control_plane::workflow_runtime::{WorkflowEngine, WorkflowRegistry};
use overdrive_control_plane::{AppState, noop_heartbeat, workflow_lifecycle, workload_lifecycle};

use overdrive_core::UnixInstant;
use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::reconcilers::{Action, AnyState, ReconcilerName, TargetResource, TickContext};
use overdrive_core::testing::workflow::ProvisionRecord;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::{WorkflowStart, WorkflowStatus};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::transport::SimTransport;

use overdrive_store_local::{LocalIntentStore, LocalObservationStore};
use tempfile::TempDir;

/// The fixture trigger reconciler (AC5). NOT an `AnyReconciler` variant —
/// it is a test-scope emitter that produces the `Action::StartWorkflow`
/// a future production producer (#206) would emit. Its single job is to
/// hand the production composition the action that exercises the now-real
/// engine wiring end-to-end.
struct FixtureTriggerReconciler {
    spec: WorkflowStart,
    correlation: CorrelationKey,
}

impl FixtureTriggerReconciler {
    /// Emit the `StartWorkflow` action — the made-up trigger's output.
    fn emit(&self) -> Vec<Action> {
        vec![Action::StartWorkflow {
            start: self.spec.clone(),
            correlation: self.correlation.clone(),
        }]
    }
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "single end-to-end golden walkthrough: real composition setup + the four \
              observable-outcome assertions (terminal row, engine task done, ctx.run once, \
              lifecycle convergence) are one indivisible scenario — splitting would obscure \
              the single committed-StartWorkflow-to-terminal pipeline this test exists to prove."
)]
async fn fixture_reconciler_emit_start_workflow_drives_provision_record_to_terminal_through_production_composition()
 {
    let tmp = TempDir::new().expect("tempdir");
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());

    // --- Real engine ports (real redb journal via AppState boot; Sim*
    //     for clock/transport/entropy under DST). The transport is held
    //     by the test so the exactly-once `ctx.run` effect can be
    //     asserted at the SimTransport call-count boundary.
    let transport = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));

    let target: SocketAddr = "127.0.0.1:9000".parse().expect("valid addr");

    // Bind an inbox at the target so the exactly-once `ctx.run` effect is
    // observable at the transport delivery boundary: ProvisionRecord's
    // single `ctx.run` sends one datagram to `target`.
    let mut inbox = transport.bind_inbox(target).await.expect("bind target inbox");

    // The engine's registry maps the workflow kind → the fixture
    // ProvisionRecord factory. Production registers real first-party
    // workflows here at boot; Phase 1 has none, so the e2e registers the
    // fixture workflow to exercise the real composition.
    let mut registry = WorkflowRegistry::new();
    registry.register(ProvisionRecord::spec().name, move || ProvisionRecord::new(target));

    // --- Real observation store (real redb) shared by the engine (writes
    //     the terminal row) and the runtime (reads it in hydrate_actual).
    let obs: Arc<dyn ObservationStore> =
        Arc::new(LocalObservationStore::open(tmp.path().join("obs.redb")).expect("open obs store"));

    // --- Real journal store (real redb) for the engine's durable run.
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

    // --- Real reconciler runtime composing the production reconcilers,
    //     including the workflow-lifecycle reconciler whose hydrate now
    //     reads real intent + the engine live-task set + terminal rows.
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

    // The real AppState composition, carrying the real engine (AC1 —
    // the engine is reachable in the production AppState; the 01-05/01-06
    // `None` dispatch is gone, replaced by `state.workflow_engine`).
    let state = AppState::new_with_workflow_engine(
        store,
        store_path,
        Arc::clone(&obs),
        Arc::new(runtime),
        driver,
        Arc::clone(&clock),
        Arc::new(SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        NodeId::new("local").expect("node id"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
        Arc::clone(&engine),
        // No transparent-mTLS layer in the workflow e2e (no real
        // dataplane to intercept on) — step 06-03 `Option` field.
        None,
        // dial-by-name-responder step 02-01: a fresh empty per-host
        // frontend-address allocator (the workflow e2e composes no DNS
        // responder; this fixture never exercises dial-by-name).
        overdrive_control_plane::dns_responder::frontend_addr_allocator::FrontendAddrAllocator::new(
        ),
    );

    // --- The fixture trigger reconciler emits StartWorkflow. The
    //     correlation is derived the same shape a real producer would use.
    let spec: WorkflowStart = ProvisionRecord::spec();
    let correlation = CorrelationKey::derive(
        target.to_string().as_str(),
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    );
    let trigger = FixtureTriggerReconciler { spec: spec.clone(), correlation: correlation.clone() };

    // === Commit the action through the production action-shim dispatch,
    //     threaded the REAL engine from AppState (the production commit
    //     point a reconciler's emitted actions flow through). This drives:
    //     emit → dispatch → real engine start → run → ctx.run effect →
    //     Terminal journal entry + WorkflowTerminal observation row +
    //     workflow-instance intent persistence.
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

    // The engine drives `run` as a tracked task off the shim; wait for it.
    state.workflow_engine.join_all().await;

    // --- (3) the ctx.run transport effect fired EXACTLY ONCE. The single
    //     ProvisionRecord `ctx.run` delivers exactly one datagram to the
    //     bound target inbox; a second is never delivered.
    let first = tokio::time::timeout(Duration::from_secs(1), inbox.recv())
        .await
        .expect("ctx.run datagram must be delivered within the budget");
    assert!(first.is_some(), "ProvisionRecord's single ctx.run must fire exactly once");
    let second = tokio::time::timeout(Duration::from_millis(200), inbox.recv()).await;
    assert!(
        second.is_err() || matches!(second, Ok(None)),
        "ctx.run must fire EXACTLY once — no second datagram"
    );

    // --- (1) the WorkflowTerminal observation row appears keyed by the
    //     same CorrelationKey, carrying a Completed status (ProvisionRecord's
    //     `Output = ()` projects to `Completed { output: cbor(()) }`).
    let terminals = obs.workflow_terminal_rows().await.expect("read terminal rows");
    let found = terminals.iter().find(|(corr, _)| *corr == correlation);
    let (_, status) = found.expect("WorkflowTerminal row keyed by the instance correlation");
    assert!(
        matches!(status, WorkflowStatus::Completed { .. }),
        "the terminal row must carry a Completed status, got {status:?}"
    );

    // --- (2) the engine task completed: no live instance remains.
    assert!(
        !state.workflow_engine.live_instances().contains(&correlation),
        "the engine must drop the live-task entry once run reaches terminal"
    );

    // --- (4) the workflow-lifecycle reconciler converges the instance to
    //     terminated. The merged per-instance projection the pure reconcile
    //     body consumes lives entirely in `actual` (the `workflows/` intent
    //     SSOT scan + the engine live-task set + the observed terminal row);
    //     `reconcile` ignores its `desired` parameter, so `hydrate_desired`
    //     returns an EMPTY state and does NOT re-scan the `workflows/` prefix
    //     a second time (regression guard:
    //     `reconciler_runtime::tests::workflow_lifecycle_hydrate`). With the
    //     terminal row observed (actual: terminal=Some), the pure reconcile
    //     emits NO StartWorkflow — the instance is converged. We assert at
    //     the hydrate boundary that desired is empty and actual sees the
    //     instance terminal, then that reconcile emits no re-start.
    let wf_name = ReconcilerName::new("workflow-lifecycle").expect("valid reconciler name");
    let wf_target = TargetResource::new("workflow/all").expect("valid target");
    // A fresh reconciler value for the hydrate-boundary assertions — the
    // hydrate_*_for_test wrappers dispatch on the `AnyReconciler` variant,
    // not on registry identity, so a freshly-constructed value reads the
    // same intent / engine / obs state as the registered one.
    let reconciler = workflow_lifecycle();

    let desired = hydrate_desired_for_test(&reconciler, &wf_target, &state)
        .await
        .expect("hydrate_desired ok");
    let actual =
        hydrate_actual_for_test(&reconciler, &wf_target, &state).await.expect("hydrate_actual ok");

    let AnyState::WorkflowLifecycle(desired_state) = desired else {
        panic!("expected WorkflowLifecycle desired state");
    };
    let AnyState::WorkflowLifecycle(actual_state) = actual else {
        panic!("expected WorkflowLifecycle actual state");
    };
    assert!(
        desired_state.instances.is_empty(),
        "hydrate_desired for the workflow-lifecycle reconciler must NOT re-scan the workflows/ \
         prefix — the merged projection lives in `actual` and `reconcile` ignores `desired`; \
         got {} desired instance(s)",
        desired_state.instances.len()
    );
    let actual_instance =
        actual_state.instances.get(&correlation).expect("hydrate_actual must surface the instance");
    assert!(
        matches!(actual_instance.terminal, Some(WorkflowStatus::Completed { .. })),
        "hydrate_actual must surface the observed terminal status, got {:?}",
        actual_instance.terminal
    );

    // Drive a convergence tick for the workflow-lifecycle reconciler:
    // with the instance terminal-observed, the reconciler emits no
    // StartWorkflow re-emit and the broker drains empty (converged).
    let seed_now = Instant::now();
    run_convergence_tick(
        &state,
        &wf_name,
        &wf_target,
        seed_now,
        1,
        seed_now + Duration::from_secs(1),
    )
    .await
    .expect("convergence tick ok");

    // Converged: a second engine start was NOT triggered — no further
    // ctx.run datagram is delivered (the terminal-observed reconcile
    // emits no StartWorkflow, so the engine never re-ran the workflow
    // body).
    state.workflow_engine.join_all().await;
    let no_more = tokio::time::timeout(Duration::from_millis(200), inbox.recv()).await;
    assert!(
        no_more.is_err() || matches!(no_more, Ok(None)),
        "a terminal-converged instance must NOT trigger a second engine run"
    );
}
