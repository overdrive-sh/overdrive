//! Acceptance — Slice 02 step 02-01.
//!
//! `S-CP-04` + `S-CP-05` — `LifecycleEvent` broadcast wiring + cause-class
//! classifier. Both scenarios drive through the action shim's
//! `dispatch` — the action shim IS the driving port for the
//! row-write-and-broadcast contract.
//!
//! Per ADR-0032 §4 Amendment 2026-04-30 (cause-class refactor) and
//! design/architecture.md §10 (single writer of `AllocStatusRow` is the
//! action shim, broadcast emit is one more side effect of the same
//! layer).
//!
//! # S-CP-04 — broadcast emits exactly N events for N transitions
//!
//! Property test, 1024 cases, N ∈ [1, 32]: dispatch N successful
//! `StartAllocation` actions through the shim; assert that exactly N
//! `LifecycleEvent` values land on the broadcast channel in submit
//! order. The test subscribes to the channel BEFORE dispatch.
//!
//! # S-CP-05 — typed start-failure fallback
//!
//! The surviving unclassified `DriverStartFailure` path is driven through the
//! action shim and publishes one typed `DriverInternalError` reason together
//! with the verbatim diagnostic detail.
//!   5. Asserts the row's `state` is `Failed` (NOT `Terminated`) —
//!      driver-start failure is now the dedicated terminal-failure
//!      lifecycle bucket per ADR-0032 §5.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use proptest::prelude::*;

use overdrive_control_plane::action_shim::{
    LifecycleEvent, ShimError, WorkloadNetworkProvisioner, dispatch_with_network_provisioner,
};
use overdrive_control_plane::veth_provisioner::{VethProvisionError, VmTapPlan, WorkloadNetnsPlan};
use overdrive_core::SpiffeId;
use overdrive_core::TransitionReason;
use overdrive_core::UnixInstant;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverStartClass,
    DriverStartFailure, DriverType, Resources, VmStartFailure,
};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore,
};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use tokio::sync::{Notify, Semaphore, broadcast};

/// service-vip-allocator step 03-02 — the action shim's dispatch
/// signature carries the allocator for the `ReleaseServiceVip` arm.
/// These lifecycle-broadcast tests do not dispatch `ReleaseServiceVip`,
/// so an ephemeral tempdir-backed allocator is sufficient. The
/// returned tuple keeps the tempdir alive for the test's lifetime.
fn fresh_test_allocator() -> (
    tempfile::TempDir,
    Arc<tokio::sync::Mutex<overdrive_dataplane::allocators::PersistentServiceVipAllocator>>,
) {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let store: Arc<dyn overdrive_core::traits::intent_store::IntentStore> = Arc::new(
        overdrive_store_local::LocalIntentStore::open(tmp.path().join("intent.redb"))
            .expect("open store for allocator"),
    );
    let allocator = overdrive_control_plane::test_default_allocator(store);
    (tmp, allocator)
}

// ---------------------------------------------------------------------------
// Test doubles
// ---------------------------------------------------------------------------

/// Sim driver that always succeeds on `start`. Used by S-CP-04 to
/// drive the success-path broadcast emission.
struct AlwaysOkDriver;

#[async_trait]
impl Driver for AlwaysOkDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Vm
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: None })
    }

    async fn stop(&self, _handle: &AllocationHandle) -> Result<(), DriverError> {
        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        Err(DriverError::NotFound { alloc: handle.alloc.clone() })
    }

    async fn resize(
        &self,
        _handle: &AllocationHandle,
        _resources: Resources,
    ) -> Result<(), DriverError> {
        Ok(())
    }
}

/// Sim driver that returns a TYPED `DriverError::StartRejected` on every
/// `start` call (DWD-24). The driver authors the cause where it is still
/// known; the shim converts rather than parses.
struct FailingDriver {
    failure: DriverStartFailure,
}

struct BarrieredOwnerDriver {
    alloc: AllocationId,
    phase: AtomicUsize,
    entered: Notify,
    release: Semaphore,
}

#[async_trait]
impl Driver for BarrieredOwnerDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Vm
    }

    async fn start(&self, _spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        if self.phase.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            self.entered.notify_one();
            self.release.acquire().await.expect("barrier remains open").forget();
            self.phase.store(2, Ordering::SeqCst);
            Ok(AllocationHandle { alloc: self.alloc.clone(), pid: Some(4242) })
        } else {
            Err(DriverError::StartRejected {
                failure: DriverStartFailure {
                    class: DriverStartClass::Vm(VmStartFailure::AllocationAlreadyOwned {
                        alloc: self.alloc.clone(),
                    }),
                    detail: "allocation already has an active VM start or supervisor".to_owned(),
                },
            })
        }
    }

    async fn stop(&self, _handle: &AllocationHandle) -> Result<(), DriverError> {
        Ok(())
    }

    async fn status(&self, _handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        Ok(if self.phase.load(Ordering::SeqCst) == 2 {
            AllocationState::Running
        } else {
            AllocationState::Pending
        })
    }

    async fn resize(
        &self,
        _handle: &AllocationHandle,
        _resources: Resources,
    ) -> Result<(), DriverError> {
        Ok(())
    }
}

impl FailingDriver {
    fn new(class: DriverStartClass, detail: impl Into<String>) -> Self {
        Self { failure: DriverStartFailure { class, detail: detail.into() } }
    }
}

#[async_trait]
impl Driver for FailingDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Vm
    }

    async fn start(&self, _spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        Err(DriverError::StartRejected { failure: self.failure.clone() })
    }

    async fn stop(&self, _handle: &AllocationHandle) -> Result<(), DriverError> {
        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        Err(DriverError::NotFound { alloc: handle.alloc.clone() })
    }

    async fn resize(
        &self,
        _handle: &AllocationHandle,
        _resources: Resources,
    ) -> Result<(), DriverError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct NoopNetworkProvisioner;

impl WorkloadNetworkProvisioner for NoopNetworkProvisioner {
    fn provision(
        &self,
        _workload: &WorkloadNetnsPlan,
        _vm_tap: &VmTapPlan,
    ) -> Result<(), VethProvisionError> {
        Ok(())
    }

    fn teardown(&self, _workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        Ok(())
    }
}

fn build_spec(alloc_id: &AllocationId, workload_id: &WorkloadId) -> AllocationSpec {
    let identity = SpiffeId::new(&format!(
        "spiffe://overdrive.local/workload/{}/alloc/{}",
        workload_id.as_str(),
        alloc_id.as_str(),
    ))
    .expect("spiffe id");
    AllocationSpec {
        alloc: alloc_id.clone(),
        identity,
        driver: overdrive_core::traits::driver::DriverPayload::Vm(
            overdrive_core::traits::driver::VmPayload {
                command: "/bin/true".to_owned(),
                args: vec![],
                kernel: PathBuf::from("/nonexistent/kernel"),
                rootfs: PathBuf::from("/nonexistent/rootfs"),
            },
        ),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        // transparent-mtls-enrollment step 04-01 (JOIN-4/JOIN-6): off the mTLS-composed boot gate.
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
        guest_tap: None,
        guest_mac: None,
        guest_gateway: None,
        guest_prefix_len: None,
        guest_dns: None,
    }
}

fn fresh_node() -> NodeId {
    NodeId::new("local").expect("node id")
}

fn make_tick(tick_n: u64) -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: tick_n,
        deadline: now + Duration::from_secs(1),
    }
}

// ---------------------------------------------------------------------------
// S-CP-04 — N transitions emit exactly N broadcast events, in order
// ---------------------------------------------------------------------------

/// Drain at most `max` events from `rx` non-blockingly. Returns the
/// drained vec. Used by S-CP-04 to assert that exactly the expected
/// number of events were broadcast.
fn drain_events(rx: &mut broadcast::Receiver<LifecycleEvent>, max: usize) -> Vec<LifecycleEvent> {
    let mut events = Vec::with_capacity(max);
    for _ in 0..max {
        match rx.try_recv() {
            Ok(event) => events.push(event),
            Err(_) => break,
        }
    }
    events
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1024,
        ..ProptestConfig::default()
    })]

    /// S-CP-04: For any N ∈ [1, 32] successful StartAllocation actions
    /// dispatched through the action shim, exactly N `LifecycleEvent`s
    /// land on the broadcast channel, in the order the actions were
    /// dispatched.
    #[test]
    fn s_cp_04_broadcast_emits_exactly_n_events_in_order(n in 1usize..=32) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        runtime.block_on(async move {
            // Broadcast channel — capacity comfortably above N=32 so
            // no slow-receiver lag in this test.
            let (tx, mut rx) = broadcast::channel::<LifecycleEvent>(256);

            let driver: Arc<dyn Driver> = Arc::new(AlwaysOkDriver);
            let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
                let mut r = overdrive_core::traits::driver::DriverRegistry::new();
                r.insert(Arc::clone(&driver));
                Arc::new(r)
            };
            let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
            let obs: Arc<dyn ObservationStore> =
                Arc::new(SimObservationStore::single_peer(fresh_node(), 0));
            let workload_id = WorkloadId::new("payments").expect("job id");
            let node_id = fresh_node();

            // Build N successful StartAllocation actions, each with a
            // distinct alloc id so the obs store sees N distinct rows.
            let mut actions: Vec<Action> = Vec::with_capacity(n);
            let mut expected_alloc_ids: Vec<AllocationId> = Vec::with_capacity(n);
            for i in 0..n {
                let alloc_id = AllocationId::new(&format!("alloc-{i}"))
                    .expect("alloc id");
                expected_alloc_ids.push(alloc_id.clone());
                let spec = build_spec(&alloc_id, &workload_id);
                actions.push(Action::StartAllocation {
                    alloc_id,
                    workload_id: workload_id.clone(),
                    node_id: node_id.clone(),
                    spec,
                    kind: overdrive_core::aggregate::WorkloadKind::Service,
                });
            }

            let tick = make_tick(0);

            // Dispatch — the shim writes N rows AND broadcasts N events.
            let dataplane: std::sync::Arc<dyn overdrive_core::traits::dataplane::Dataplane> = std::sync::Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new());
            let writer_node = overdrive_core::id::NodeId::new("writer-1").expect("NodeId");
            let (_alloc_tmp, allocator) = fresh_test_allocator();
            let test_broker = parking_lot::Mutex::new(
                overdrive_core::eval_broker::EvaluationBroker::new(),
            );
            dispatch_with_network_provisioner(actions, drivers.as_ref(), &alloc_drivers, obs.as_ref(), dataplane.as_ref(),
                &overdrive_sim::adapters::ca::SimCa::new(std::sync::Arc::new(overdrive_sim::adapters::entropy::SimEntropy::new(0))),
                &overdrive_sim::adapters::clock::SimClock::new(),
                &overdrive_control_plane::identity_mgr::IdentityMgr::new(None),
        &overdrive_control_plane::gateway_composition::GatewayIdentityActionComposition::disabled(),
        None,
                &tx, &tick, &writer_node, allocator, &test_broker, None, None,
        // transparent-mtls-enrollment step 04-01: a fresh per-host slot
        // allocator — this fixture exercises no netns provisioning.
        &overdrive_control_plane::veth_provisioner::NetSlotAllocator::new(),
        &NoopNetworkProvisioner,
        &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
    )
                .await
                .expect("dispatch must succeed");

            // Assert exactly N events arrived on the channel, in submit
            // order (broadcast preserves send order to all subscribers).
            let events = drain_events(&mut rx, n + 1);
            prop_assert_eq!(events.len(), n,
                "expected exactly N={} events, got {}", n, events.len());

            for (i, event) in events.iter().enumerate() {
                prop_assert_eq!(&event.alloc_id, &expected_alloc_ids[i],
                    "event {} alloc_id mismatch", i);
                prop_assert_eq!(&event.workload_id, &workload_id);
                prop_assert!(matches!(event.reason, TransitionReason::Started));
            }

            Ok::<(), TestCaseError>(())
        })?;
    }
}

// ---------------------------------------------------------------------------
// S-CP-05 — classifier prefix-match table (5 branches)
// ---------------------------------------------------------------------------

/// Run a single classifier scenario. Sets up the action shim with a
/// `FailingDriver` returning `reason_text`, dispatches one
/// `StartAllocation`, then asserts:
///   - the written `AllocStatusRow.reason` matches `expected_reason`
///   - the written row's `detail` carries `reason_text` verbatim
///   - the written row's `state` is `Failed` (not `Terminated`)
///   - the broadcast event's `reason` matches `expected_reason`
async fn run_classifier_scenario(
    class: DriverStartClass,
    reason_text: &str,
    expected_reason: TransitionReason,
) {
    let (tx, mut rx) = broadcast::channel::<LifecycleEvent>(16);

    let driver: Arc<dyn Driver> = Arc::new(FailingDriver::new(class, reason_text));
    let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
        let mut r = overdrive_core::traits::driver::DriverRegistry::new();
        r.insert(Arc::clone(&driver));
        Arc::new(r)
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(fresh_node(), 0));

    let alloc_id = AllocationId::new("alloc-fail").expect("alloc id");
    let workload_id = WorkloadId::new("payments").expect("job id");
    let node_id = fresh_node();
    let spec = build_spec(&alloc_id, &workload_id);
    let action = Action::StartAllocation {
        alloc_id: alloc_id.clone(),
        workload_id: workload_id.clone(),
        node_id: node_id.clone(),
        spec,
        kind: overdrive_core::aggregate::WorkloadKind::Service,
    };

    let tick = make_tick(0);

    let dataplane: std::sync::Arc<dyn overdrive_core::traits::dataplane::Dataplane> =
        std::sync::Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new());

    let writer_node = overdrive_core::id::NodeId::new("writer-1").expect("NodeId");

    let (_alloc_tmp, allocator) = fresh_test_allocator();
    let test_broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    dispatch_with_network_provisioner(
        vec![action],
        drivers.as_ref(),
        &alloc_drivers,
        obs.as_ref(),
        dataplane.as_ref(),
        &overdrive_sim::adapters::ca::SimCa::new(std::sync::Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        )),
        &overdrive_sim::adapters::clock::SimClock::new(),
        &overdrive_control_plane::identity_mgr::IdentityMgr::new(None),
        &overdrive_control_plane::gateway_composition::GatewayIdentityActionComposition::disabled(),
        None,
        &tx,
        &tick,
        &writer_node,
        allocator,
        &test_broker,
        None,
        None,
        // transparent-mtls-enrollment step 04-01: a fresh per-host slot
        // allocator — this fixture exercises no netns provisioning.
        &overdrive_control_plane::veth_provisioner::NetSlotAllocator::new(),
        &NoopNetworkProvisioner,
        &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
    )
    .await
    .expect("dispatch must succeed even on driver failure (failure is recorded)");

    // Assert the row.
    let rows = obs.alloc_status_rows().await.expect("read rows");
    assert_eq!(rows.len(), 1, "exactly one row written");
    let row = &rows[0];
    assert_eq!(
        row.state,
        AllocState::Failed,
        "StartRejected must write state=Failed (not Terminated) per ADR-0032 §5"
    );
    assert_eq!(
        row.reason,
        Some(expected_reason.clone()),
        "row.reason must be the classified cause-class variant"
    );
    assert_eq!(
        row.detail.as_deref(),
        Some(reason_text),
        "row.detail must carry verbatim driver text for audit"
    );

    // Assert the broadcast event.
    let event = rx.try_recv().expect("broadcast event must arrive");
    assert_eq!(event.alloc_id, alloc_id);
    assert_eq!(
        event.reason, expected_reason,
        "event.reason must match the row's classified reason"
    );
    // Ensure no extra events.
    assert!(rx.try_recv().is_err(), "exactly one broadcast event per row write");
}

async fn dispatch_cleanup_composition_action(
    driver: Arc<dyn Driver>,
    obs: &SimObservationStore,
    tx: &broadcast::Sender<LifecycleEvent>,
    alloc_id: &AllocationId,
    tick: u64,
    restart: bool,
) -> Result<(), ShimError> {
    let drivers = {
        let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
        registry.insert(driver);
        registry
    };
    let workload_id = WorkloadId::new("cleanup-composition").expect("workload id");
    let spec = build_spec(alloc_id, &workload_id);
    let action = if restart {
        Action::RestartAllocation {
            alloc_id: alloc_id.clone(),
            spec,
            kind: overdrive_core::aggregate::WorkloadKind::Service,
        }
    } else {
        Action::StartAllocation {
            alloc_id: alloc_id.clone(),
            workload_id: workload_id.clone(),
            node_id: fresh_node(),
            spec,
            kind: overdrive_core::aggregate::WorkloadKind::Service,
        }
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    let (_tmp, allocator) = fresh_test_allocator();
    let broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    dispatch_with_network_provisioner(
        vec![action],
        &drivers,
        &alloc_drivers,
        obs,
        &overdrive_sim::adapters::dataplane::SimDataplane::new(),
        &overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        )),
        &overdrive_sim::adapters::clock::SimClock::new(),
        &overdrive_control_plane::identity_mgr::IdentityMgr::new(None),
        &overdrive_control_plane::gateway_composition::GatewayIdentityActionComposition::disabled(),
        None,
        tx,
        &make_tick(tick),
        &NodeId::new("writer-1").expect("writer node"),
        allocator,
        &broker,
        None,
        None,
        &overdrive_control_plane::veth_provisioner::NetSlotAllocator::new(),
        &NoopNetworkProvisioner,
        &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
    )
    .await
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
async fn barriered_starting_and_live_duplicate_actions_preserve_the_real_shim_row_and_event() {
    let alloc = AllocationId::new("alloc-barriered-owner").expect("alloc id");
    let workload = WorkloadId::new("barriered-owner").expect("workload id");
    let node = fresh_node();
    let pending = AllocStatusRow {
        alloc_id: alloc.clone(),
        workload_id: workload,
        node_id: node.clone(),
        state: AllocState::Pending,
        updated_at: LogicalTimestamp { counter: 10, writer: node },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: None,
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    };
    let obs = Arc::new(SimObservationStore::single_peer(fresh_node(), 0));
    obs.write_alloc_lifecycle(
        pending.clone(),
        overdrive_core::traits::observation_store::TransitionSource::Reconciler,
    )
    .await
    .expect("seed Pending before the original author enters start");
    let driver = Arc::new(BarrieredOwnerDriver {
        alloc: alloc.clone(),
        phase: AtomicUsize::new(0),
        entered: Notify::new(),
        release: Semaphore::new(0),
    });
    let (tx, mut rx) = broadcast::channel(16);

    let original_obs = Arc::clone(&obs);
    let original_driver = Arc::clone(&driver);
    let original_tx = tx.clone();
    let original_alloc = alloc.clone();
    let original = tokio::spawn(async move {
        dispatch_cleanup_composition_action(
            original_driver,
            original_obs.as_ref(),
            &original_tx,
            &original_alloc,
            11,
            false,
        )
        .await
        .expect("the original start dispatch succeeds");
    });
    driver.entered.notified().await;

    dispatch_cleanup_composition_action(driver.clone(), obs.as_ref(), &tx, &alloc, 12, false)
        .await
        .expect("the Starting duplicate is a handled conflict");
    assert_eq!(
        obs.alloc_status_row(&alloc).await.expect("read Starting owner row"),
        Some(pending),
        "the duplicate action cannot publish Failed while the original start is barriered",
    );
    assert!(rx.try_recv().is_err(), "the Starting conflict emits no lifecycle event");

    driver.release.add_permits(1);
    original.await.expect("original action task completes");
    let running = obs
        .alloc_status_row(&alloc)
        .await
        .expect("read original Running row")
        .expect("original author publishes Running");
    assert_eq!(running.state, AllocState::Running);
    let started = rx.try_recv().expect("the original author emits the sole Running event");
    assert_eq!(started.to, overdrive_control_plane::api::AllocStateWire::Running);
    assert!(rx.try_recv().is_err());

    dispatch_cleanup_composition_action(driver.clone(), obs.as_ref(), &tx, &alloc, 13, false)
        .await
        .expect("the Live duplicate is a handled conflict");
    assert_eq!(
        obs.alloc_status_row(&alloc).await.expect("read Live owner row"),
        Some(running),
        "the Live conflict cannot supersede the healthy owner's row",
    );
    assert!(rx.try_recv().is_err(), "the Live conflict emits no lifecycle event");
    assert_eq!(driver.phase.load(Ordering::SeqCst), 2, "the original owner remains Live");
}

#[tokio::test]
async fn s_cp_05_classifier_unclassified_falls_through_to_driver_internal_error() {
    let raw = "totally unclassifiable driver text from a future driver";
    run_classifier_scenario(
        DriverStartClass::Unclassified { driver: DriverType::Vm },
        raw,
        TransitionReason::DriverInternalError { detail: raw.to_owned() },
    )
    .await;
}

// ---------------------------------------------------------------------------
// Sanity — a Stop action also broadcasts a LifecycleEvent (architectural
// guarantee per architecture.md §10: every obs.write is paired with a
// bus.send).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stop_action_also_broadcasts_lifecycle_event() {
    let (tx, mut rx) = broadcast::channel::<LifecycleEvent>(16);

    let driver: Arc<dyn Driver> = Arc::new(AlwaysOkDriver);
    let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
        let mut r = overdrive_core::traits::driver::DriverRegistry::new();
        r.insert(Arc::clone(&driver));
        Arc::new(r)
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(fresh_node(), 0));

    // Seed a prior alloc row so `find_prior_alloc_row` finds it on the
    // Stop arm.
    let alloc_id = AllocationId::new("alloc-stop").expect("alloc id");
    let workload_id = WorkloadId::new("payments").expect("job id");
    let node_id = fresh_node();
    let prior_row = overdrive_core::traits::observation_store::AllocStatusRow {
        alloc_id: alloc_id.clone(),
        workload_id: workload_id.clone(),
        node_id: node_id.clone(),
        state: AllocState::Running,
        updated_at: overdrive_core::traits::observation_store::LogicalTimestamp {
            counter: 1,
            writer: node_id.clone(),
        },
        reason: Some(TransitionReason::Started),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Service,
        listeners: Vec::new(),
        // GAP-1 subsidiary: Running state carries fixed wall-clock.
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        // Host-netns fixture — no canonical workload address (AllocStatusRowV2 additive field, GH #241).
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    };
    obs.write_alloc_lifecycle(
        prior_row,
        overdrive_core::traits::observation_store::TransitionSource::Reconciler,
    )
    .await
    .expect("seed prior row");

    // Dispatch a Stop action — should write Terminated row AND emit broadcast.
    // ADR-0037 §4: emission sites outside a reconciler tick (here, a
    // direct test-bench dispatch) emit `terminal: None` — the
    // reconciler is the single source of every terminal claim.
    let action = Action::StopAllocation { alloc_id: alloc_id.clone(), terminal: None };
    let tick = make_tick(1);
    let dataplane: std::sync::Arc<dyn overdrive_core::traits::dataplane::Dataplane> =
        std::sync::Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new());
    let writer_node = overdrive_core::id::NodeId::new("writer-1").expect("NodeId");
    let (_alloc_tmp, allocator) = fresh_test_allocator();
    let test_broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    dispatch_with_network_provisioner(
        vec![action],
        drivers.as_ref(),
        &alloc_drivers,
        obs.as_ref(),
        dataplane.as_ref(),
        &overdrive_sim::adapters::ca::SimCa::new(std::sync::Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        )),
        &overdrive_sim::adapters::clock::SimClock::new(),
        &overdrive_control_plane::identity_mgr::IdentityMgr::new(None),
        &overdrive_control_plane::gateway_composition::GatewayIdentityActionComposition::disabled(),
        None,
        &tx,
        &tick,
        &writer_node,
        allocator,
        &test_broker,
        None,
        None,
        // transparent-mtls-enrollment step 04-01: a fresh per-host slot
        // allocator — this fixture exercises no netns provisioning.
        &overdrive_control_plane::veth_provisioner::NetSlotAllocator::new(),
        &NoopNetworkProvisioner,
        &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
    )
    .await
    .expect("dispatch must succeed");

    let event = rx.try_recv().expect("broadcast event must arrive");
    assert_eq!(event.alloc_id, alloc_id);
    assert!(matches!(event.reason, TransitionReason::Stopped { .. }));
}

// ---------------------------------------------------------------------------
// Sanity — recording driver test fixture for ergonomic Mutex usage in
// the always-ok branch. (Anchors the unused-symbol lint.)
// ---------------------------------------------------------------------------
#[allow(dead_code)]
fn _suppress_unused_mutex_import() {
    let _: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
}
