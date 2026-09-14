//! Step 02-03 / Slice 3A.3 scenario 3.7 — walking-skeleton:
//! `killed_workload_is_restarted_with_fresh_alloc_id`.
//!
//! Submits a 1-replica VM job; waits until the allocation is Running; injects
//! a guest-authoritative crash event through the simulated VM driver; and
//! asserts the accepted predecessor remains Failed while recovery reaches
//! Running under a distinct successor `AllocationId`.
//!
//! # The contract (ADR-0078 § D6)
//!
//! Phase 3 asserts the predecessor's durable crash facts remain immutable and
//! the recovered successor starts its per-allocation history at zero/None.
//!
#![allow(
    clippy::doc_markdown,
    reason = "the required CONTRACT_SHAPE line is an exact repository token"
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::worker::exit_observer;
use overdrive_control_plane::{AppState, noop_heartbeat, workload_lifecycle};
use overdrive_core::aggregate::{DriverInput, IntentKey, Job, JobSpecInput, ResourcesInput};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::reconcilers::TargetResource;
use overdrive_core::traits::driver::{Driver, DriverType, ExitKind};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_core::transition_reason::TransitionReason;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn killed_workload_is_restarted_with_fresh_alloc_id() {
    let tmp = TempDir::new().expect("tempdir");
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    runtime.register(noop_heartbeat()).await.expect("register noop");
    runtime.register(workload_lifecycle()).await.expect("register workload-lifecycle");

    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("open store"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    // Share the simulated VM driver's logical clock with the test so the
    // injected guest exit can be delivered deterministically.
    let sim_clock = Arc::new(SimClock::new());
    let driver_concrete = Arc::new(SimDriver::with_clock(DriverType::Vm, sim_clock.clone()));
    let driver: Arc<dyn Driver> = driver_concrete.clone();

    let allocator = overdrive_control_plane::test_default_allocator(
        Arc::clone(&store) as Arc<dyn overdrive_core::traits::intent_store::IntentStore>
    );
    let state = AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        Arc::new(overdrive_host::SystemClock),
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        overdrive_core::id::NodeId::new("writer-1").unwrap(),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    );

    // Spawn the exit-observer subsystem. In production this is wired
    // by `run_server_with_obs_and_driver`; tests construct it directly
    // so the watcher's `ExitEvent`s are consumed and classified into
    // `AllocStatusRow`s on the obs store. Without this spawn, the
    // production VM driver's per-allocation watcher task fires (sending
    // events on its mpsc channel) but nothing reads the receiver, so
    // no `Failed`/`Terminated` row ever appears in obs.
    let _exit_observer = exit_observer::spawn(
        state.obs.clone(),
        state.drivers.get(DriverType::Vm).cloned().expect("registry has a VM entry"),
        state.lifecycle_events.clone(),
        state.clock.clone(),
    );

    // Use a distinct workload ID so this fixture's allocation remains
    // distinguishable from the other lifecycle fixtures.
    let job = Job::from_submit(JobSpecInput {
        id: "recovery".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Vm(overdrive_core::aggregate::VmInput {
            command: "/bin/sleep".to_string(),
            args: vec!["3600".to_string()],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
    })
    .expect("valid job spec");
    let archived = overdrive_core::aggregate::WorkloadIntent::Job(job.clone())
        .archive_for_store()
        .expect("rkyv archive");
    let key = IntentKey::for_workload(&job.id);
    state.store.put(key.as_bytes(), archived.as_ref()).await.expect("put job");

    let target = TargetResource::new("workload/recovery").expect("valid target");
    let workload_lifecycle_name =
        overdrive_core::reconcilers::ReconcilerName::new("workload-lifecycle")
            .expect("workload-lifecycle reconciler name");
    let start = Instant::now();
    let deadline = start + Duration::from_secs(120);

    // Phase 1: drive to first Running.
    let mut tick_n = 0_u64;
    let mut first_running = false;
    while tick_n < 30 && !first_running {
        run_convergence_tick(
            &state,
            &workload_lifecycle_name,
            &target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        let rows = state.obs.alloc_status_rows().await.expect("read rows");
        first_running = rows.iter().any(|r| r.state == AllocState::Running);
        tick_n += 1;
    }
    assert!(first_running, "alloc must reach Running before crash");

    // Phase 2: inject a guest-authoritative crash through the simulated VM
    // driver. The production VM exit observer consumes the same
    // `ExitEvent` port, so no synthetic observation row bypasses the owner.
    let rows = state.obs.alloc_status_rows().await.expect("read rows");
    let prior = rows.into_iter().find(|r| r.state == AllocState::Running).expect("running row");
    driver_concrete.inject_exit_after(
        &prior.alloc_id,
        Duration::ZERO,
        ExitKind::Crashed { exit_code: None, signal: Some(9) },
    );
    tokio::task::yield_now().await;
    sim_clock.tick(Duration::ZERO);
    tokio::task::yield_now().await;

    // Capture the accepted predecessor terminal before asking the production
    // owner to allocate its successor.
    let crash_deadline = Instant::now() + Duration::from_secs(20);
    let crash_row = loop {
        if let Some(row) =
            state.obs.alloc_status_row(&prior.alloc_id).await.expect("read predecessor row")
            && row.state == AllocState::Failed
        {
            break row;
        }
        assert!(
            Instant::now() < crash_deadline,
            "exit observer did not publish predecessor Failed"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let successor = AllocationId::new("alloc-recovery-1").expect("valid successor allocation ID");

    // Phase 3: drive convergence until a distinct successor reaches Running.
    let mut recovered: Option<AllocStatusRow> = None;
    while tick_n < 150 && recovered.is_none() {
        run_convergence_tick(
            &state,
            &workload_lifecycle_name,
            &target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        tokio::time::sleep(Duration::from_millis(20)).await;
        let rows = state.obs.alloc_status_rows().await.expect("read rows");
        recovered =
            rows.into_iter().find(|r| r.state == AllocState::Running && r.alloc_id == successor);
        tick_n += 1;
    }
    let row = recovered.expect(
        "alloc must converge to a distinct Running successor after the injected crash \
         within the Phase-3 tick budget",
    );

    assert_eq!(row.alloc_id, successor);
    assert_eq!(row.restart_count, 0, "fresh successor starts per-key history at zero");
    assert!(row.last_terminated.is_none(), "fresh successor inherits no predecessor snapshot");
    assert!(
        matches!(crash_row.reason, Some(TransitionReason::WorkloadCrashedImmediately { .. })),
        "the injected exit must be classified as a crash, not an intentional stop: {:?}",
        crash_row.reason,
    );
    assert_eq!(
        state.obs.alloc_status_row(&prior.alloc_id).await.expect("predecessor row re-read"),
        Some(crash_row),
        "successor publication must not rewrite predecessor crash history",
    );
}
