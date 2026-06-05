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
//! is NOT run as a reconciler. This is the upheld two-primitive doctrine
//! (R3): the reconciler manages WHICH instances should exist; the engine
//! manages HOW each instance's steps execute. ADR-0064 §5.
//!
//! # Port-to-port
//!
//! The driving port is `action_shim::dispatch` (the runtime's async I/O
//! boundary, ADR-0023). The driven ports are the injected `WorkflowEngine`
//! (built over `SimJournalStore` + `SimTransport`) and the
//! `ObservationStore`. The observable outcome is asserted at the engine's
//! driven `JournalStore` boundary: after the `StartWorkflow` action is
//! dispatched, the engine drove the author's `async fn run` to its
//! terminal — proven by the durable journal carrying a `Terminal` entry
//! for the started instance (the engine ran `run` off the shim; a
//! no-dispatch `Ok(())` arm or a reconcile loop would leave the journal
//! empty).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::action_shim::{LifecycleEvent, dispatch};
use overdrive_control_plane::journal::{JournalEntry, JournalStore, WorkflowId};
use overdrive_control_plane::workflow_runtime::{WorkflowEngine, WorkflowRegistry};

use overdrive_core::UnixInstant;
use overdrive_core::eval_broker::EvaluationBroker;
use overdrive_core::id::{ContentHash, CorrelationKey, NodeId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::testing::workflow::ProvisionRecord;
use overdrive_core::traits::driver::DriverType;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::{Clock, Entropy, Transport};
use overdrive_core::workflow::WorkflowSpec;

use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::journal::SimJournalStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;

use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

#[tokio::test]
async fn start_workflow_action_is_dispatched_to_the_engine_off_the_shim_not_run_as_a_reconciler() {
    // --- Engine driven ports, all injected as Sim* adapters ----------
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let transport: Arc<dyn Transport> = Arc::new(SimTransport::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(0x5eed));

    let target: SocketAddr = "127.0.0.1:9000".parse().expect("valid addr");

    // The engine's registry maps a WorkflowName → the author's Workflow.
    // The reconciler emits `StartWorkflow { spec }`; the engine resolves
    // `spec.name` to the registered `ProvisionRecord` and drives `run`.
    let mut registry = WorkflowRegistry::new();
    registry.register(ProvisionRecord::spec().name, move || Box::new(ProvisionRecord::new(target)));

    let engine = WorkflowEngine::new(
        Arc::clone(&journal),
        Arc::clone(&clock),
        Arc::clone(&transport),
        Arc::clone(&entropy),
        registry,
    );

    // --- The action the workflow-lifecycle reconciler would emit ------
    let spec: WorkflowSpec = ProvisionRecord::spec();
    let correlation = CorrelationKey::derive(
        target.to_string().as_str(),
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-provision-0001").expect("valid instance id");

    // --- Remaining action-shim ports (untouched by StartWorkflow) -----
    let driver = SimDriver::new(DriverType::Exec);
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let dataplane = SimDataplane::new();
    let (bus, _rx) = tokio::sync::broadcast::channel::<LifecycleEvent>(16);
    let node = NodeId::new("node-a").expect("valid node id");

    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn IntentStore> =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open store"));
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        store,
    )));
    let broker = parking_lot::Mutex::new(EvaluationBroker::new());

    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    // --- Driving port: the action shim, off which the engine runs -----
    dispatch(
        vec![Action::StartWorkflow { spec, correlation }],
        &driver,
        obs.as_ref(),
        &dataplane,
        &bus,
        &tick,
        &node,
        Arc::clone(&allocator),
        &broker,
        Some((&engine, &workflow_id)),
    )
    .await
    .expect("StartWorkflow dispatch must succeed");

    // The engine drives `run` as a tracked task off the shim; wait for it.
    engine.join_all().await;

    // --- Observable outcome at the driven JournalStore boundary -------
    // The engine ran the author's `async fn run` (off the shim, NOT as a
    // reconcile loop) and journaled its terminal. A no-dispatch `Ok(())`
    // arm would leave the journal empty.
    let entries = journal.load_journal(&workflow_id).await.expect("load journal");
    assert!(
        entries
            .iter()
            .any(|e| matches!(e, JournalEntry::Terminal { result } if result == "Success")),
        "the engine must drive run to a Terminal(Success) journal entry off the shim; got {entries:?}"
    );
}
