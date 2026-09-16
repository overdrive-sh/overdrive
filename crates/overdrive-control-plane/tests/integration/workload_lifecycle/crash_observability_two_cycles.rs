//! T-F (ADR-0078 § D6) — two crash-replacement cycles through the exit
//! observer and action shim.
//!
//! Drives `A0 Running → A0 Failed → A1 Running → A1 Failed → A2 Running`
//! with the simulated VM driver and `exit_observer::spawn`, then proves each
//! predecessor row remains immutable while every successor begins zero/None.
//!
//! # Why this test is load-bearing (§ D6)
//!
//! The two cycles distinguish per-allocation crash history from the stable
//! workload budget: an exit observer may update only its exact allocation key,
//! and replacement may never copy or overwrite a predecessor's history.
//!
//! # Determinism
//!
//! The replacements are dispatched explicitly through `action_shim::dispatch`
//! rather than driven by the `WorkloadLifecycle` reconciler's backoff, so
//! there is no restart-budget timing to race. The simulated driver's
//! injected exit events are driven by its logical clock; the observer still
//! consumes the production `ExitEvent` port and writes durable rows.

#![cfg(target_os = "linux")]
#![allow(
    clippy::doc_markdown,
    reason = "the required CONTRACT_SHAPE line is an exact repository token"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationSpec, Driver, DriverPayload, DriverType, ExitKind, Resources, VmPayload,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_core::transition_reason::TransitionReason;
use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;
use tokio::sync::broadcast;

use overdrive_control_plane::action_shim::{
    LifecycleEvent, WorkloadNetworkProvisioner, dispatch_with_network_provisioner,
};
use overdrive_control_plane::veth_provisioner::NetSlotAllocator;
use overdrive_control_plane::veth_provisioner::{VethProvisionError, VmTapPlan, WorkloadNetnsPlan};
use overdrive_control_plane::worker::exit_observer;

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

/// A workload that exits non-zero after a short delay. The delay gives
/// the action shim's `Running` write and the observer's Running-gate
/// release time to land before the process is reaped, so the exit event
/// always finds a prior row (the shape production guarantees via the
/// gate).
fn crashing_spec(alloc: &AllocationId) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::for_allocation(
            &WorkloadId::new("crashobs2").expect("valid workload id"),
            alloc,
        ),
        driver: DriverPayload::Vm(VmPayload {
            command: "/sbin/init".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/kernel"),
            rootfs: PathBuf::from("/rootfs"),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
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

/// A long-lived workload — used for the FINAL restart so the terminal
/// row under assertion stays `Running` for the duration of the checks.
/// The cleanup guard reaps it.
fn long_lived_spec(alloc: &AllocationId) -> AllocationSpec {
    crashing_spec(alloc)
}

/// Poll the durable LWW-winner row until `pred` holds, or panic with the
/// last-observed row after `budget`.
///
/// Polls on DURABLE state only — never on a transient window — which is
/// the whole point of ADR-0078: the facts survive the merge, so there is
/// no race to lose.
async fn await_row(
    obs: &dyn ObservationStore,
    alloc: &AllocationId,
    budget: Duration,
    what: &str,
    pred: impl Fn(&AllocStatusRow) -> bool,
) -> AllocStatusRow {
    let deadline = Instant::now() + budget;
    let mut last: Option<AllocStatusRow> = None;
    while Instant::now() < deadline {
        if let Some(row) = obs.alloc_status_row(alloc).await.expect("read alloc row") {
            if pred(&row) {
                return row;
            }
            last = Some(row);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}; last observed row: {last:#?}");
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn two_crash_cycles_use_fresh_successors_and_preserve_each_terminal() {
    let sim_clock = Arc::new(SimClock::new());
    let clock: Arc<dyn Clock> = sim_clock.clone();
    let driver_concrete = Arc::new(SimDriver::with_clock(DriverType::Vm, sim_clock.clone()));
    let driver: Arc<dyn Driver> = driver_concrete.clone();
    let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
        let mut r = overdrive_core::traits::driver::DriverRegistry::new();
        r.insert(Arc::clone(&driver));
        Arc::new(r)
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();

    let node_id = NodeId::new("local").expect("valid node id");
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(node_id.clone(), 0));
    let (events_tx, _events_rx) = broadcast::channel::<LifecycleEvent>(256);
    let events = Arc::new(events_tx);

    let alloc = AllocationId::new("alloc-crashobs2-0").expect("valid alloc id");
    let successor_one = AllocationId::new("alloc-crashobs2-1").expect("valid alloc id");
    let successor_two = AllocationId::new("alloc-crashobs2-2").expect("valid alloc id");
    let workload = WorkloadId::new("crashobs2").expect("valid workload id");
    // The production exit-observer subsystem — the § D2 site-7 writer under
    // test. It consumes the driver's `ExitEvent`s and writes the `Failed`
    // rows whose crash-fact FORWARD-CARRY this test exists to pin.
    let observer_handle =
        exit_observer::spawn(obs.clone(), driver.clone(), events.clone(), clock.clone());

    let tmp = TempDir::new().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let store: Arc<dyn IntentStore> =
        Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));

    // ---- Cycle 1: StartAllocation -> Running, crash -> Failed. --------
    dispatch_one(
        obs.as_ref(),
        drivers.as_ref(),
        &alloc_drivers,
        &store,
        &events,
        Action::StartAllocation {
            alloc_id: alloc.clone(),
            workload_id: workload.clone(),
            node_id: node_id.clone(),
            spec: crashing_spec(&alloc),
            kind: WorkloadKind::Service,
        },
        0,
    )
    .await;

    let running_first =
        await_row(obs.as_ref(), &alloc, Duration::from_secs(5), "the first Running row", |r| {
            r.state == AllocState::Running
        })
        .await;
    assert_eq!(running_first.restart_count, 0, "a first start is not a restart");
    assert_eq!(running_first.last_terminated, None, "and it has survived no terminal yet");

    driver_concrete.inject_exit_after(
        &alloc,
        Duration::ZERO,
        ExitKind::Crashed { exit_code: Some(3), signal: None },
    );
    tokio::task::yield_now().await;
    sim_clock.tick(Duration::ZERO);
    tokio::task::yield_now().await;

    let crash_one = await_row(
        obs.as_ref(),
        &alloc,
        Duration::from_secs(20),
        "the FIRST Failed row (exit 3)",
        |r| r.state == AllocState::Failed,
    )
    .await;
    assert_eq!(
        crash_one.restart_count, 0,
        "§ D2 site 7: the exit observer FORWARDS the counter — a crash is not a restart",
    );
    assert_eq!(
        crash_one.last_terminated, None,
        "§ D1: the crash row forwards; it never self-describes",
    );
    assert!(
        matches!(
            crash_one.reason,
            Some(TransitionReason::WorkloadCrashedImmediately { exit_code: Some(3), .. })
        ),
        "crash 1 must be classified as a crash carrying exit code 3: {:?}",
        crash_one.reason,
    );

    // ---- Cycle 1 recovery: predecessor A0 -> fresh Running A1. -----
    dispatch_one(
        obs.as_ref(),
        drivers.as_ref(),
        &alloc_drivers,
        &store,
        &events,
        Action::RestartAllocation {
            alloc_id: alloc.clone(),
            spec: crashing_spec(&successor_one),
            kind: WorkloadKind::Service,
        },
        1,
    )
    .await;

    let recovery_one = await_row(
        obs.as_ref(),
        &successor_one,
        Duration::from_secs(5),
        "the first recovered Running row",
        |r| r.state == AllocState::Running,
    )
    .await;
    assert_eq!(recovery_one.restart_count, 0, "fresh A1 starts per-key history at zero");
    assert!(recovery_one.last_terminated.is_none());
    assert_eq!(
        obs.alloc_status_row(&alloc).await.expect("A0 row re-read"),
        Some(crash_one.clone()),
        "A1 publication must not rewrite A0 crash history",
    );

    driver_concrete.inject_exit_after(
        &successor_one,
        Duration::ZERO,
        ExitKind::Crashed { exit_code: Some(4), signal: None },
    );
    tokio::task::yield_now().await;
    sim_clock.tick(Duration::ZERO);
    tokio::task::yield_now().await;

    // ---- Cycle 2: A1 crashes and keeps its own zero/None history. ---
    let crash_two = await_row(
        obs.as_ref(),
        &successor_one,
        Duration::from_secs(20),
        "the SECOND Failed row (exit 4)",
        |r| {
            r.state == AllocState::Failed
                && matches!(
                    r.reason,
                    Some(TransitionReason::WorkloadCrashedImmediately { exit_code: Some(4), .. })
                )
        },
    )
    .await;

    assert_eq!(crash_two.restart_count, 0);
    assert!(crash_two.last_terminated.is_none());

    // ---- Cycle 2 recovery: predecessor A1 -> fresh Running A2. -----
    dispatch_one(
        obs.as_ref(),
        drivers.as_ref(),
        &alloc_drivers,
        &store,
        &events,
        Action::RestartAllocation {
            alloc_id: successor_one.clone(),
            // A VM payload so the final Running row is stable for the
            // assertions.
            spec: long_lived_spec(&successor_two),
            kind: WorkloadKind::Service,
        },
        2,
    )
    .await;

    let recovery_two = await_row(
        obs.as_ref(),
        &successor_two,
        Duration::from_secs(5),
        "the second recovered Running row",
        |r| r.state == AllocState::Running,
    )
    .await;

    assert_eq!(recovery_two.restart_count, 0, "fresh A2 starts per-key history at zero");
    assert!(recovery_two.last_terminated.is_none());
    assert_eq!(
        obs.alloc_status_row(&successor_one).await.expect("A1 row re-read"),
        Some(crash_two.clone()),
        "A2 publication must not rewrite A1 crash history",
    );

    // Reap the final allocation through the production stop path. This also
    // exercises exact-key stop forward-carry on A2.
    dispatch_one(
        obs.as_ref(),
        drivers.as_ref(),
        &alloc_drivers,
        &store,
        &events,
        Action::StopAllocation { alloc_id: successor_two.clone(), terminal: None },
        3,
    )
    .await;
    let stopped = await_row(
        obs.as_ref(),
        &successor_two,
        Duration::from_secs(10),
        "the Terminated row after the operator stop",
        |r| r.state == AllocState::Terminated,
    )
    .await;
    assert_eq!(stopped.restart_count, 0);
    assert!(stopped.last_terminated.is_none());
    assert_eq!(obs.alloc_status_row(&alloc).await.unwrap(), Some(crash_one));
    assert_eq!(obs.alloc_status_row(&successor_one).await.unwrap(), Some(crash_two));

    drop(driver_concrete);
    drop(driver);
    let _ = tokio::time::timeout(Duration::from_secs(2), observer_handle).await;
}

/// Dispatch exactly one action through the production
/// `action_shim::dispatch` with sim adapters for every orthogonal port.
async fn dispatch_one(
    obs: &dyn ObservationStore,
    drivers: &overdrive_core::traits::driver::DriverRegistry,
    alloc_drivers: &overdrive_control_plane::action_shim::AllocDriverIndex,
    store: &Arc<dyn IntentStore>,
    events: &Arc<broadcast::Sender<LifecycleEvent>>,
    action: Action,
    tick_n: u64,
) {
    let dataplane: Arc<dyn overdrive_core::traits::dataplane::Dataplane> =
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new());
    let writer_node = NodeId::new("local").expect("NodeId");
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        Arc::clone(store),
    )));
    let net_slot_allocator = NetSlotAllocator::new();
    let test_broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());

    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(
            1_700_000_000 + tick_n * 100,
        )),
        tick: tick_n,
        deadline: now + Duration::from_secs(10),
    };

    dispatch_with_network_provisioner(
        vec![action],
        drivers,
        alloc_drivers,
        obs,
        dataplane.as_ref(),
        &overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        )),
        &overdrive_sim::adapters::clock::SimClock::new(),
        &overdrive_control_plane::identity_mgr::IdentityMgr::new(None),
        events,
        &tick,
        &writer_node,
        Arc::clone(&allocator),
        &test_broker,
        None,
        None,
        &net_slot_allocator,
        &NoopNetworkProvisioner,
        &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
    )
    .await
    .expect("dispatch must succeed");
}
