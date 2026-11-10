//! `WorkloadLifecycle::hydrate_actual` fills `current_startup_probe`
//! (feature `service-allocation-readiness-and-ending`, S-17; D6 item 4, D7 item 3).
//!
//! Pinned meaning: the status of the latest result row for role `Startup`,
//! probe index 0, of the workload's CURRENT attempt only (`current_alloc`), read
//! only when that row's state is `Failed` or `Terminated`; `None` when there is
//! no such row or no result row. A read failure is
//! `HydrateError::ObservationRead` (never `None`, never "failed read means no
//! pass"). Superseded attempts' rows are never read.
//!
//! Port: `WorkloadLifecycle::hydrate_actual` through the runtime's hydration
//! context (`hydrate_actual_for_test`); inputs are rows in `SimObservationStore`;
//! the probe-row reads are counted by a delegating store at the driven port.

#![allow(clippy::expect_used, clippy::doc_markdown, clippy::too_many_lines)]

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use overdrive_control_plane::AppState;
use overdrive_control_plane::reconciler_runtime::{
    ConvergenceError, ReconcilerRuntime, hydrate_actual_for_test,
};
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::observation::{
    ProbeIdx, ProbeResultRow, ProbeRole, ProbeRunSeq, ProbeStatus, SameStatusRuns,
};
use overdrive_core::reconcilers::{HydrateError, TargetResource};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore, TransitionSource,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::{AnyReconciler, AnyState, WorkloadLifecycle};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

use crate::acceptance::issue_fault_store::IssueFaultStore;

const WORKLOAD: &str = "payments";

fn node() -> NodeId {
    NodeId::from_str("writer-1").expect("node id")
}
fn alloc(n: u32) -> AllocationId {
    AllocationId::new(&format!("alloc-{WORKLOAD}-{n}")).expect("alloc id")
}
fn fail(reason: &str) -> ProbeStatus {
    ProbeStatus::Fail { last_fail_reason: reason.to_owned() }
}

struct World {
    state: AppState,
    faulty: Arc<IssueFaultStore>,
    _tmp: TempDir,
}

fn world() -> World {
    let tmp = TempDir::new().expect("tempdir");
    let sim = Arc::new(SimObservationStore::single_peer(node(), 0));
    let faulty = IssueFaultStore::over(Arc::clone(&sim) as Arc<dyn ObservationStore>);
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("intent store"));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    let state = AppState::new(
        store,
        store_path,
        Arc::clone(&faulty) as Arc<dyn ObservationStore>,
        Arc::new(runtime),
        driver,
        Arc::new(SimClock::new()),
        Arc::new(SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        node(),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
        crate::acceptance::serve_ports::worker(),
        crate::acceptance::serve_ports::owner(),
        crate::acceptance::serve_ports::exec_gate(),
        crate::acceptance::serve_ports::pool(),
    );
    World { state, faulty, _tmp: tmp }
}

async fn write_row(world: &World, n: u32, state: AllocState, counter: u64) {
    let row = AllocStatusRow {
        alloc_id: alloc(n),
        workload_id: WorkloadId::new(WORKLOAD).expect("workload id"),
        node_id: node(),
        state,
        updated_at: LogicalTimestamp { counter, writer: node() },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(std::time::Duration::from_secs(1))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    };
    world
        .state
        .obs
        .write_alloc_lifecycle(row, TransitionSource::Reconciler)
        .await
        .expect("write alloc row");
}

async fn write_startup_row(world: &World, n: u32, status: ProbeStatus, run_seq: u64) {
    world
        .state
        .obs
        .write_probe_result(ProbeResultRow {
            alloc_id: alloc(n),
            probe_idx: ProbeIdx::new(0),
            role: ProbeRole::Startup,
            status,
            last_observed_at_unix_ms: run_seq,
            run_seq: ProbeRunSeq::new(run_seq).expect("non-zero"),
            same_status_runs: SameStatusRuns::ONE,
        })
        .await
        .expect("write startup row");
}

async fn hydrate(world: &World) -> Result<Option<ProbeStatus>, ConvergenceError> {
    let target = TargetResource::new(&format!("workload/{WORKLOAD}")).expect("target");
    let reconciler = AnyReconciler::WorkloadLifecycle(WorkloadLifecycle::canonical());
    match hydrate_actual_for_test(&reconciler, &target, &world.state).await? {
        AnyState::WorkloadLifecycle(state) => Ok(state.current_startup_probe),
        other => panic!("expected WorkloadLifecycle state, got {other:?}"),
    }
}

/// S-17: a `Pass` and a `Fail { last_fail_reason }` on the current attempt's
/// startup row hydrate as that status, for a `Failed` and a `Terminated` row;
/// one probe-row read per hydration.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn current_attempts_startup_status_is_hydrated_for_a_terminal_row() {
    for (row_state, status) in [
        (AllocState::Failed, ProbeStatus::Pass),
        (AllocState::Failed, fail("connection refused")),
        (AllocState::Terminated, fail("connection refused")),
        (AllocState::Terminated, ProbeStatus::Pass),
    ] {
        let world = world();
        write_row(&world, 3, row_state, 1).await;
        write_startup_row(&world, 3, status.clone(), 4).await;
        world.faulty.clear_probe_reads();

        let hydrated = hydrate(&world).await.expect("hydration succeeds");

        assert_eq!(hydrated, Some(status), "row state {row_state:?}");
        assert_eq!(
            world.faulty.probe_reads(),
            vec![alloc(3)],
            "exactly one probe-row read, of the current attempt"
        );
    }
}

/// S-17 control: no result row, or a current row that is not terminal, hydrates
/// as `None`; a non-terminal current row is never read for probe rows. Passes
/// at HEAD.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn no_startup_status_is_hydrated_without_a_result_row_or_for_a_live_row() {
    // Terminal row, no result row.
    let w = world();
    write_row(&w, 0, AllocState::Failed, 1).await;
    assert_eq!(hydrate(&w).await.expect("hydration succeeds"), None);

    // Running row with a stored startup row: not read, `None`.
    let w = world();
    write_row(&w, 0, AllocState::Running, 1).await;
    write_startup_row(&w, 0, fail("connection refused"), 2).await;
    w.faulty.clear_probe_reads();
    assert_eq!(hydrate(&w).await.expect("hydration succeeds"), None);
    assert!(
        !w.faulty.probe_reads().contains(&alloc(0)),
        "a live current row's startup rows are never read by this field"
    );
}

/// S-17: a superseded attempt's rows are never read and never affect the field:
/// attempt 0 carries `Fail` rows, the current attempt 1 is `Failed` with no
/// startup row.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn a_superseded_attempts_startup_rows_are_never_read() {
    let w = world();
    write_row(&w, 0, AllocState::Terminated, 1).await;
    write_startup_row(&w, 0, fail("connection refused"), 5).await;
    write_row(&w, 1, AllocState::Failed, 2).await;
    w.faulty.clear_probe_reads();

    let hydrated = hydrate(&w).await.expect("hydration succeeds");

    assert_eq!(hydrated, None, "the superseded attempt's Fail does not leak");
    assert_eq!(w.faulty.probe_reads(), vec![alloc(1)], "only the current attempt is read");
}

/// S-17: a failing probe-row read surfaces as `HydrateError::ObservationRead`,
/// never as `None`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn a_failed_startup_row_read_is_an_observation_read_error() {
    let w = world();
    write_row(&w, 0, AllocState::Failed, 1).await;
    w.faulty.fail_probe_reads.store(true, Ordering::SeqCst);

    let outcome = hydrate(&w).await;

    assert!(
        matches!(outcome, Err(ConvergenceError::Hydrate(HydrateError::ObservationRead(_)))),
        "expected an ObservationRead error, got {outcome:?}"
    );
}
