//! T-F (ADR-0078 § D6) — TWO crash-restart cycles through the REAL exit
//! observer and the REAL action shim. **Mandatory; nothing else covers
//! the forward-carry.**
//!
//! Drives `Running → Failed → Running → Failed → Running` with a real
//! `ExecDriver`, a real `exit_observer::spawn` watcher task, and real
//! `action_shim::dispatch` restarts, then asserts the final row carries
//! `restart_count == 2` and a `last_terminated` describing the **second**
//! terminal — not the first.
//!
//! # Why this test is load-bearing (§ D6)
//!
//! This is the ONLY test that fails when a writer forward-carries the
//! two crash-observability fields wrongly. A hand-typed
//! `restart_count: 0` at any forward-carry site:
//!
//! - passes T-A (`crash_facts_advance.rs`), which tests the pure
//!   function, not the call sites;
//! - passes T-C (`action_shim_crash_observability.rs`), which asserts
//!   `== 1` after exactly one restart;
//! - passes the rewritten `crash_recovery.rs`, which also asserts `== 1`.
//!
//! Only a SECOND cycle observes the difference: a resetting writer
//! yields `1` again where the contract requires `2`. The specific hazard
//! is § D2 site 7 — the exit observer, which before ADR-0078 built its
//! row from a raw `AllocStatusRow { .. }` literal and would have had to
//! type both fields by hand. Routing it through
//! `build_alloc_status_row` puts it inside the required-parameter net;
//! this test is what proves the routing actually forwards.
//!
//! # Determinism
//!
//! The restarts are dispatched EXPLICITLY through `action_shim::dispatch`
//! rather than driven by the `WorkloadLifecycle` reconciler's backoff, so
//! there is no restart-budget timing to race. The only waits are for the
//! kernel to reap the workload and for the observer task to drain its
//! channel — both polled on the durable row, never on a transient state.
//!
//! Linux-only — `ExecDriver` requires a real cgroup v2 root. Routed
//! through Lima per `.claude/rules/testing.md` § "Running tests — Lima
//! VM"; gated behind the `integration-tests` feature per § "Integration
//! vs unit gating".

#![cfg(target_os = "linux")]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::CgroupFs;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{AllocationSpec, Driver, Resources};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_core::transition_reason::TransitionReason;
use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::ExecDriver;
use overdrive_worker::cgroup_manager::CgroupManager;
use serial_test::serial;
use tempfile::TempDir;
use tokio::sync::broadcast;

use overdrive_control_plane::action_shim::{LifecycleEvent, dispatch};
use overdrive_control_plane::veth_provisioner::NetSlotAllocator;
use overdrive_control_plane::worker::exit_observer;

use super::cleanup::AllocCleanup;

const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// A workload that exits non-zero after a short delay. The delay gives
/// the action shim's `Running` write and the observer's Running-gate
/// release time to land before the process is reaped, so the exit event
/// always finds a prior row (the shape production guarantees via the
/// gate).
fn crashing_spec(alloc: &AllocationId, exit_code: u8) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/workload/crashobs2/alloc/0")
            .expect("valid spiffe id"),
        driver: overdrive_core::traits::driver::DriverPayload::Exec(
            overdrive_core::traits::driver::ExecPayload {
                command: "/bin/sh".to_owned(),
                args: vec!["-c".to_owned(), format!("sleep 0.3; exit {exit_code}")],
            },
        ),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
    }
}

/// A long-lived workload — used for the FINAL restart so the terminal
/// row under assertion stays `Running` for the duration of the checks.
/// The cleanup guard reaps it.
fn long_lived_spec(alloc: &AllocationId) -> AllocationSpec {
    let mut spec = crashing_spec(alloc, 0);
    spec.driver = overdrive_core::traits::driver::DriverPayload::Exec(
        overdrive_core::traits::driver::ExecPayload {
            command: spec.driver.command().to_owned(),
            args: vec!["-c".to_owned(), "sleep 3600".to_owned()],
        },
    );
    spec
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

#[tokio::test]
#[serial(cgroup)]
#[allow(clippy::too_many_lines)]
async fn two_crash_cycles_count_two_restarts_and_describe_the_second_terminal() {
    let cgroup_root = Path::new(CGROUP_ROOT);
    let fs: Arc<dyn CgroupFs> = Arc::new(overdrive_host::RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let driver_concrete = Arc::new(ExecDriver::new(cgroup_root.to_path_buf(), clock.clone(), fs));
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
    let workload = WorkloadId::new("crashobs2").expect("valid workload id");
    let _cleanup =
        AllocCleanup { obs: obs.clone(), cgroup_root: std::path::PathBuf::from(CGROUP_ROOT) };

    // The REAL exit-observer subsystem — the § D2 site-7 writer under
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
            spec: crashing_spec(&alloc, 3),
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

    // ---- Cycle 1 recovery: RestartAllocation -> Running (restarts 1). --
    dispatch_one(
        obs.as_ref(),
        drivers.as_ref(),
        &alloc_drivers,
        &store,
        &events,
        Action::RestartAllocation {
            alloc_id: alloc.clone(),
            spec: crashing_spec(&alloc, 4),
            kind: WorkloadKind::Service,
            reason: None,
        },
        1,
    )
    .await;

    let recovery_one = await_row(
        obs.as_ref(),
        &alloc,
        Duration::from_secs(5),
        "the first recovered Running row",
        |r| r.state == AllocState::Running && r.restart_count >= 1,
    )
    .await;
    assert_eq!(recovery_one.restart_count, 1, "exactly one observed restart after cycle 1");
    let lt_one =
        recovery_one.last_terminated.as_ref().expect("recovery 1 must snapshot the first crash");
    assert!(
        matches!(
            lt_one.reason,
            Some(TransitionReason::WorkloadCrashedImmediately { exit_code: Some(3), .. })
        ),
        "the snapshot must describe crash 1 (exit 3): {:?}",
        lt_one.reason,
    );

    // ---- Cycle 2: the SECOND crash -> Failed (forwards both fields). ---
    let crash_two = await_row(
        obs.as_ref(),
        &alloc,
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

    // THE assertion this whole test exists for. A writer that hand-types
    // `restart_count: 0` here — the pre-ADR-0078 raw-literal shape — makes
    // this line fail while every other crash-observability test still
    // passes.
    assert_eq!(
        crash_two.restart_count, 1,
        "§ D2 site 7: the exit observer must FORWARD restart_count across a crash, \
         not reset it (this is the forward-carry regression T-F exists to catch)",
    );
    assert_eq!(
        crash_two.last_terminated.as_ref().and_then(|lt| lt.reason.clone()),
        lt_one.reason.clone(),
        "and it must forward the snapshot of crash 1 verbatim, not re-mint one",
    );

    // ---- Cycle 2 recovery: RestartAllocation -> Running (restarts 2). --
    dispatch_one(
        obs.as_ref(),
        drivers.as_ref(),
        &alloc_drivers,
        &store,
        &events,
        Action::RestartAllocation {
            alloc_id: alloc.clone(),
            // A long-lived command so the final Running row is stable for
            // the assertions; the cleanup guard reaps it.
            spec: long_lived_spec(&alloc),
            kind: WorkloadKind::Service,
            reason: None,
        },
        2,
    )
    .await;

    let recovery_two = await_row(
        obs.as_ref(),
        &alloc,
        Duration::from_secs(5),
        "the second recovered Running row",
        |r| r.state == AllocState::Running && r.restart_count >= 2,
    )
    .await;

    assert_eq!(recovery_two.restart_count, 2, "TWO crashes survived, TWO restarts counted");
    let lt_two =
        recovery_two.last_terminated.as_ref().expect("recovery 2 must snapshot the second crash");
    assert!(
        matches!(
            lt_two.reason,
            Some(TransitionReason::WorkloadCrashedImmediately { exit_code: Some(4), .. })
        ),
        "the depth-1 snapshot must describe the SECOND terminal (exit 4), not the first: {:?}",
        lt_two.reason,
    );
    assert_eq!(
        lt_two.terminated_at, crash_two.updated_at,
        "and it must identify exactly WHICH durable observation it summarises",
    );
    assert!(
        recovery_two.updated_at.dominates(&lt_two.terminated_at),
        "the recovered row must strictly dominate the terminal it snapshots",
    );

    // Reap the long-lived final workload through the PRODUCTION stop path
    // rather than leaving it for the `Drop` guard: an outliving
    // `sleep 3600` is what nextest flags as LEAK, and the guard only fires
    // on unwind. This also exercises § D2 site 6's forward-carry against a
    // row that genuinely carries crash history.
    dispatch_one(
        obs.as_ref(),
        drivers.as_ref(),
        &alloc_drivers,
        &store,
        &events,
        Action::StopAllocation { alloc_id: alloc.clone(), terminal: None },
        3,
    )
    .await;
    let stopped = await_row(
        obs.as_ref(),
        &alloc,
        Duration::from_secs(10),
        "the Terminated row after the operator stop",
        |r| r.state == AllocState::Terminated,
    )
    .await;
    assert_eq!(stopped.restart_count, 2, "§ D2 site 6: a stop FORWARDS the monotone counter");
    assert_eq!(
        stopped.last_terminated.as_ref().map(|lt| &lt.terminated_at),
        Some(&crash_two.updated_at),
        "and it keeps describing the crash the alloc survived",
    );

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

    dispatch(
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
        &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
    )
    .await
    .expect("dispatch must succeed");
}
