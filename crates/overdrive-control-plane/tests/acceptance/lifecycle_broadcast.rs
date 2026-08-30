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
//! # S-CP-05 — the typed cause-conversion table (DWD-24)
//!
//! Five branches over `DriverError::StartRejected`'s TYPED
//! `DriverStartFailure.class`. The shim's own text grammar is retired —
//! the driver authors the class where the cause is still known and this
//! layer applies the total `From<&DriverStartFailure>` conversion.
//!
//! **Every operator-visible outcome below is unchanged from the retired
//! grammar** — same `TransitionReason` payloads, same verbatim `detail`.
//! Only the selection mechanism changed, which is why the diagnostics in
//! each branch are kept byte-identical to the strings the old prefix
//! table matched on:
//!
//! | `DriverStartClass`                                    | variant                                      |
//! |---|---|
//! | `Exec(BinaryNotFound { path: "/no/such" })`           | `ExecBinaryNotFound { path: "/no/such" }`    |
//! | `Exec(PermissionDenied { path })`                     | `ExecPermissionDenied { path: "..." }`       |
//! | `Exec(BinaryInvalid { path, kind })`                  | `ExecBinaryInvalid { path, kind }`           |
//! | `Exec(CgroupSetupFailed { kind, source })`            | `CgroupSetupFailed { kind, source }`         |
//! | `Unclassified { driver }`                             | `DriverInternalError { detail }`             |
//!
//! A sixth scenario pins the property the grammar could not hold at all:
//! the SAME class under DIFFERENT prose still yields the SAME reason.
//!
//! Each branch:
//!   1. Constructs a sim driver returning `StartRejected` with the
//!      tabled typed class and diagnostic.
//!   2. Dispatches a single `Action::StartAllocation`.
//!   3. Asserts the written `AllocStatusRow.reason` matches the typed
//!      cause-class variant; `AllocStatusRow.detail` carries the
//!      verbatim text (audit trail per architecture.md).
//!   4. Asserts the broadcast `LifecycleEvent.reason` is the same
//!      typed variant (byte-equal to the row's reason).
//!   5. Asserts the row's `state` is `Failed` (NOT `Terminated`) —
//!      driver-start failure is now the dedicated terminal-failure
//!      lifecycle bucket per ADR-0032 §5.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use proptest::prelude::*;

use overdrive_control_plane::action_shim::{LifecycleEvent, ShimError, dispatch};
use overdrive_core::SpiffeId;
use overdrive_core::TransitionReason;
use overdrive_core::UnixInstant;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverCleanupFailure,
    DriverCleanupStage, DriverError, DriverStartClass, DriverStartFailure, DriverType,
    ExecStartFailure, Resources, VmStartFailure,
};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
    ObservationStoreError,
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
        DriverType::Exec
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

struct CleanupThenDuplicateDriver {
    alloc: AllocationId,
    calls: AtomicUsize,
    releases: AtomicUsize,
    disposition_retries: AtomicUsize,
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
        DriverType::Exec
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

#[async_trait]
impl Driver for CleanupThenDuplicateDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Exec
    }

    async fn start(&self, _spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 1 {
            return Err(DriverError::StartRejected {
                failure: DriverStartFailure {
                    class: DriverStartClass::Vm(VmStartFailure::AllocationAlreadyOwned {
                        alloc: self.alloc.clone(),
                    }),
                    detail: "duplicate fallback must not replace cleanup authority".to_owned(),
                },
            });
        }

        let primary = Arc::new(DriverError::StartRejected {
            failure: DriverStartFailure {
                class: DriverStartClass::Unclassified { driver: DriverType::Exec },
                detail: "primary start rejection".to_owned(),
            },
        });
        let failures = [
            DriverCleanupStage::VmmTerminate,
            DriverCleanupStage::RootfsCloneRemove,
            DriverCleanupStage::RootfsIndexRemove,
            DriverCleanupStage::CgroupKill,
            DriverCleanupStage::CgroupRemove,
            DriverCleanupStage::RunDirectoryRemove,
        ]
        .into_iter()
        .map(|stage| {
            DriverCleanupFailure::new(
                stage,
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("{stage:?} source"),
                ),
            )
        })
        .collect();
        if call == 0 {
            Err(DriverError::start_cleanup_failed(self.alloc.clone(), primary, failures))
        } else {
            Err(DriverError::start_cleanup_recovered(self.alloc.clone(), primary, failures))
        }
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

    fn release_supervision(&self, alloc: &AllocationId) {
        assert_eq!(alloc, &self.alloc);
        self.releases.fetch_add(1, Ordering::SeqCst);
    }

    fn retry_start_cleanup_disposition(&self, alloc: &AllocationId) {
        assert_eq!(alloc, &self.alloc);
        self.disposition_retries.fetch_add(1, Ordering::SeqCst);
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
        DriverType::Exec
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
        driver: overdrive_core::traits::driver::DriverPayload::Exec(
            overdrive_core::traits::driver::ExecPayload {
                command: "/bin/true".to_owned(),
                args: vec![],
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
            dispatch(actions, drivers.as_ref(), &alloc_drivers, obs.as_ref(), dataplane.as_ref(),
                &overdrive_sim::adapters::ca::SimCa::new(std::sync::Arc::new(overdrive_sim::adapters::entropy::SimEntropy::new(0))),
                &overdrive_sim::adapters::clock::SimClock::new(),
                &overdrive_control_plane::identity_mgr::IdentityMgr::new(None),
                &tx, &tick, &writer_node, allocator, &test_broker, None, None,
        // transparent-mtls-enrollment step 04-01: a fresh per-host slot
        // allocator — this fixture exercises no netns provisioning.
        &overdrive_control_plane::veth_provisioner::NetSlotAllocator::new(),
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
    dispatch(
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
    dispatch(
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
        tx,
        &make_tick(tick),
        &NodeId::new("writer-1").expect("writer node"),
        allocator,
        &broker,
        None,
        None,
        &overdrive_control_plane::veth_provisioner::NetSlotAllocator::new(),
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
    obs.write(ObservationRow::AllocStatus(Box::new(pending.clone())))
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

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
async fn cleanup_composition_remains_authoritative_when_the_next_tick_observes_retained_ownership()
{
    let alloc = AllocationId::new("alloc-cleanup-authority").expect("alloc id");
    let driver = Arc::new(CleanupThenDuplicateDriver {
        alloc: alloc.clone(),
        calls: AtomicUsize::new(0),
        releases: AtomicUsize::new(0),
        disposition_retries: AtomicUsize::new(0),
    });
    let obs = SimObservationStore::single_peer(fresh_node(), 0);
    let (tx, mut rx) = broadcast::channel(16);

    dispatch_cleanup_composition_action(driver.clone(), &obs, &tx, &alloc, 1, false)
        .await
        .expect("initial cleanup disposition commits");
    let authoritative = obs
        .alloc_status_row(&alloc)
        .await
        .expect("read cleanup row")
        .expect("cleanup failure writes an explicit retryable row");
    assert_eq!(authoritative.state, AllocState::Pending);
    let detail = authoritative.detail.as_deref().expect("cleanup detail is persisted");
    for stage in [
        "VmmTerminate",
        "RootfsCloneRemove",
        "RootfsIndexRemove",
        "CgroupKill",
        "CgroupRemove",
        "RunDirectoryRemove",
    ] {
        assert!(detail.contains(stage), "cleanup composition retains {stage}: {detail}");
    }
    assert!(detail.contains("primary start rejection"));
    let first_event = rx.try_recv().expect("cleanup failure emits one Pending event");

    dispatch_cleanup_composition_action(driver.clone(), &obs, &tx, &alloc, 2, false)
        .await
        .expect("retained-owner duplicate is a handled conflict");
    assert_eq!(
        obs.alloc_status_row(&alloc).await.expect("read retry row"),
        Some(authoritative.clone()),
        "the retained-owner fallback cannot replace the primary-plus-cleanup composition",
    );
    assert!(rx.try_recv().is_err(), "the retained-owner fallback emits no second event");
    assert_eq!(first_event.to, overdrive_control_plane::api::AllocStateWire::Pending);
    assert_eq!(driver.releases.load(Ordering::SeqCst), 0, "owned residue is not released");

    dispatch_cleanup_composition_action(driver.clone(), &obs, &tx, &alloc, 3, true)
        .await
        .expect("restart cleanup recovery disposition commits");
    let recovered = obs
        .alloc_status_row(&alloc)
        .await
        .expect("read recovered cleanup row")
        .expect("the restart retry commits its cleanup disposition");
    assert_eq!(recovered.state, AllocState::Failed);
    assert_eq!(recovered.detail, authoritative.detail);
    let recovery_event = rx.try_recv().expect("recovered cleanup emits its durable disposition");
    assert_eq!(recovery_event.to, overdrive_control_plane::api::AllocStateWire::Failed);
    assert!(rx.try_recv().is_err(), "one event follows each committed cleanup disposition");
    assert_eq!(driver.releases.load(Ordering::SeqCst), 1, "release follows durable disposition");
    assert_eq!(driver.calls.load(Ordering::SeqCst), 3, "all production action ticks execute");
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
async fn recovered_cleanup_write_failure_returns_disposition_without_releasing_ownership() {
    let alloc = AllocationId::new("alloc-cleanup-write-retry").expect("alloc id");
    let driver = Arc::new(CleanupThenDuplicateDriver {
        alloc: alloc.clone(),
        calls: AtomicUsize::new(2),
        releases: AtomicUsize::new(0),
        disposition_retries: AtomicUsize::new(0),
    });
    let obs = SimObservationStore::single_peer(fresh_node(), 0);
    obs.inject_write_failure(ObservationStoreError::Io(std::io::Error::from(
        std::io::ErrorKind::PermissionDenied,
    )));
    let (tx, mut rx) = broadcast::channel(16);

    let first =
        dispatch_cleanup_composition_action(driver.clone(), &obs, &tx, &alloc, 1, false).await;
    assert!(first.is_err(), "the rejected disposition write propagates");
    assert_eq!(driver.releases.load(Ordering::SeqCst), 0, "ownership remains held");
    assert_eq!(
        driver.disposition_retries.load(Ordering::SeqCst),
        1,
        "the recovered disposition is returned to the driver",
    );
    assert_eq!(obs.alloc_status_row(&alloc).await.expect("read row"), None);
    assert!(rx.try_recv().is_err());

    dispatch_cleanup_composition_action(driver.clone(), &obs, &tx, &alloc, 2, false)
        .await
        .expect("the returned disposition retries and commits");
    assert_eq!(
        obs.alloc_status_row(&alloc).await.expect("read retried row").map(|row| row.state),
        Some(AllocState::Failed),
    );
    assert_eq!(driver.releases.load(Ordering::SeqCst), 1);
    assert_eq!(
        rx.try_recv().expect("commit emits event").to,
        overdrive_control_plane::api::AllocStateWire::Failed,
    );
}

#[tokio::test]
async fn s_cp_05_classifier_enoent_to_exec_binary_not_found() {
    run_classifier_scenario(
        DriverStartClass::Exec(ExecStartFailure::BinaryNotFound { path: "/no/such".to_owned() }),
        "spawn /no/such: No such file or directory (os error 2)",
        TransitionReason::ExecBinaryNotFound { path: "/no/such".to_owned() },
    )
    .await;
}

#[tokio::test]
async fn s_cp_05_classifier_eacces_to_exec_permission_denied() {
    run_classifier_scenario(
        DriverStartClass::Exec(ExecStartFailure::PermissionDenied {
            path: "/usr/local/bin/payments".to_owned(),
        }),
        "spawn /usr/local/bin/payments: Permission denied (os error 13)",
        TransitionReason::ExecPermissionDenied { path: "/usr/local/bin/payments".to_owned() },
    )
    .await;
}

#[tokio::test]
async fn s_cp_05_classifier_enoexec_to_exec_binary_invalid() {
    run_classifier_scenario(
        DriverStartClass::Exec(ExecStartFailure::BinaryInvalid {
            path: "/tmp/garbage".to_owned(),
            kind: "exec_format_error".to_owned(),
        }),
        "spawn /tmp/garbage: Exec format error (os error 8)",
        TransitionReason::ExecBinaryInvalid {
            path: "/tmp/garbage".to_owned(),
            kind: "exec_format_error".to_owned(),
        },
    )
    .await;
}

#[tokio::test]
async fn s_cp_05_classifier_cgroup_failure_to_cgroup_setup_failed() {
    run_classifier_scenario(
        DriverStartClass::Exec(ExecStartFailure::CgroupSetupFailed {
            kind: "place_pid".to_owned(),
            source: "write cgroup.procs: Permission denied".to_owned(),
        }),
        "cgroup setup failed: place_pid: write cgroup.procs: Permission denied",
        TransitionReason::CgroupSetupFailed {
            kind: "place_pid".to_owned(),
            source: "write cgroup.procs: Permission denied".to_owned(),
        },
    )
    .await;
}

#[tokio::test]
async fn s_cp_05_classifier_unclassified_falls_through_to_driver_internal_error() {
    let raw = "totally unclassifiable driver text from a future driver";
    run_classifier_scenario(
        DriverStartClass::Unclassified { driver: DriverType::Exec },
        raw,
        TransitionReason::DriverInternalError { detail: raw.to_owned() },
    )
    .await;
}

/// DWD-24 regression net: the SAME structured cause under DIFFERENT
/// diagnostic prose must still reach the operator as the SAME reason.
///
/// Under the retired text grammar this was impossible — the prose WAS the
/// classification input, so rewording a driver's message silently changed
/// the operator's diagnosis. This is the property that made the grammar
/// unsafe to keep, so it is asserted at the shim boundary directly.
#[tokio::test]
async fn s_cp_05_cause_survives_a_reworded_diagnostic() {
    let class = DriverStartClass::Exec(ExecStartFailure::BinaryNotFound {
        path: "/usr/local/bin/payments".to_owned(),
    });
    let expected =
        TransitionReason::ExecBinaryNotFound { path: "/usr/local/bin/payments".to_owned() };

    // Prose that the retired prefix table could never have matched.
    run_classifier_scenario(
        class.clone(),
        "the configured binary is simply not there any more",
        expected.clone(),
    )
    .await;
    // ...and prose in an entirely different language.
    run_classifier_scenario(class, "fichier introuvable", expected).await;
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
    obs.write(ObservationRow::AllocStatus(Box::new(prior_row))).await.expect("seed prior row");

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
    dispatch(
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
