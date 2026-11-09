//! Feature `service-allocation-readiness-and-ending`, decisions D6 and D7, obligation
//! S-27 (wire half) — the restart budget the `GET /v1/allocs` snapshot reports for
//! each terminal a workload's rows can carry.
//!
//! `restart_budget_from_rows` is private; it is exercised through its only caller,
//! the `alloc_status` handler, over rows written to the observation store. A budget
//! is exhausted with `used == RESTART_BACKOFF_CEILING` for `BackoffExhausted` (its own
//! attempts), `ServiceFailed { EarlyExit }` (D6) and `ServiceFailed {
//! StartupProbeFailed }` (D7); `ServiceFailed { LivenessProbeFailed }` is unchanged
//! (tracked as F-2, GH #336); no terminal means an untouched budget.
//!
//! CONTRACT_SHAPE: pure-function over the handler's wire output (a `RestartBudget`
//! for a set of rows; the store is only read).

#![allow(clippy::expect_used, clippy::expect_fun_call, clippy::unwrap_used)]
#![allow(clippy::doc_markdown)]

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use overdrive_control_plane::AppState;
use overdrive_control_plane::api::RestartBudget;
use overdrive_control_plane::handlers::{AllocStatusQuery, alloc_status};
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::TransitionReason;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{
    DriverInput, IntentKey, Job, JobSpecInput, ResourcesInput, WorkloadKind,
};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore as _;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore,
};
use overdrive_core::transition_reason::{ServiceFailureReason, StoppedBy, TerminalCondition};
use overdrive_reconcilers::RESTART_BACKOFF_CEILING;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

const WORKLOAD: &str = "payments-v2";

fn node() -> NodeId {
    NodeId::from_str("node-a").expect("node id")
}

fn app_state(tmp: &TempDir) -> AppState {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("intent store"));
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(node(), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
    let allocator = overdrive_control_plane::test_default_allocator(
        Arc::clone(&store) as Arc<dyn overdrive_core::traits::intent_store::IntentStore>
    );
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        Arc::new(SimClock::new()),
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        NodeId::new("writer-1").expect("node id"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
        crate::acceptance::serve_ports::worker(),
        crate::acceptance::serve_ports::owner(),
        crate::acceptance::serve_ports::exec_gate(),
        crate::acceptance::serve_ports::pool(),
    )
}

async fn install_workload(state: &AppState) -> WorkloadId {
    let job = Job::from_submit(JobSpecInput {
        id: WORKLOAD.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 134_217_728 },
        driver: DriverInput::Vm(overdrive_core::aggregate::VmInput {
            command: "/usr/local/bin/payments".to_owned(),
            args: vec![],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
    })
    .expect("a valid spec");
    let key = IntentKey::for_workload(&job.id);
    let archived = overdrive_core::aggregate::WorkloadIntent::Job(job.clone())
        .archive_for_store()
        .expect("archive");
    state.store.put(key.as_bytes(), archived.as_ref()).await.expect("intent put");
    job.id
}

async fn write_row(
    state: &AppState,
    workload: &WorkloadId,
    attempt: u32,
    row_state: AllocState,
    terminal: Option<TerminalCondition>,
) {
    let row = AllocStatusRow {
        alloc_id: AllocationId::new(&format!("alloc-{WORKLOAD}-{attempt}")).expect("alloc id"),
        workload_id: workload.clone(),
        node_id: node(),
        state: row_state,
        updated_at: LogicalTimestamp { counter: u64::from(attempt) + 1, writer: node() },
        reason: Some(TransitionReason::Started),
        detail: None,
        terminal,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    };
    state
        .obs
        .write_alloc_lifecycle(
            row,
            overdrive_core::traits::observation_store::TransitionSource::Reconciler,
        )
        .await
        .expect("obs write");
}

async fn budget_for(terminal: Option<TerminalCondition>, row_state: AllocState) -> RestartBudget {
    let tmp = TempDir::new().expect("tmpdir");
    let state = app_state(&tmp);
    let workload = install_workload(&state).await;
    write_row(&state, &workload, 0, row_state, terminal).await;
    let response =
        alloc_status(State(state), Query(AllocStatusQuery { job: Some(WORKLOAD.to_owned()) }))
            .await
            .expect("alloc_status");
    response.0.restart_budget.expect("a restart budget on a read with rows")
}

/// S-27 — a give-up by attempt budget is exhausted with its own attempts.
#[tokio::test]
async fn a_backoff_exhausted_terminal_is_an_exhausted_budget_with_its_attempts() {
    let budget = budget_for(
        Some(TerminalCondition::BackoffExhausted { attempts: RESTART_BACKOFF_CEILING }),
        AllocState::Failed,
    )
    .await;
    assert_eq!(
        budget,
        RestartBudget {
            used: RESTART_BACKOFF_CEILING,
            max: RESTART_BACKOFF_CEILING,
            exhausted: true
        },
    );
}

/// S-27 / D6 — a pre-startup crash loop given up as `EarlyExit` is exhausted with
/// `used == RESTART_BACKOFF_CEILING`.
#[tokio::test]
/// CONTRACT_SHAPE: pure-function.
#[ignore = "pending DELIVER: D6 an EarlyExit give-up is an exhausted restart budget"]
async fn an_early_exit_give_up_is_an_exhausted_budget_at_the_ceiling() {
    let budget = budget_for(
        Some(TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::EarlyExit { exit_code: Some(1) },
        }),
        AllocState::Failed,
    )
    .await;
    assert_eq!(
        budget,
        RestartBudget {
            used: RESTART_BACKOFF_CEILING,
            max: RESTART_BACKOFF_CEILING,
            exhausted: true
        },
    );
}

/// S-27 / D7 — a never-ready loop given up as `StartupProbeFailed` is exhausted with
/// `used == RESTART_BACKOFF_CEILING`.
#[tokio::test]
/// CONTRACT_SHAPE: pure-function.
#[ignore = "pending DELIVER: D7 a StartupProbeFailed give-up is an exhausted restart budget"]
async fn a_startup_probe_failed_give_up_is_an_exhausted_budget_at_the_ceiling() {
    let budget = budget_for(
        Some(TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::StartupProbeFailed {
                probe_idx: 0,
                last_fail: "connection refused".to_owned(),
                attempts: RESTART_BACKOFF_CEILING,
            },
        }),
        AllocState::Failed,
    )
    .await;
    assert_eq!(
        budget,
        RestartBudget {
            used: RESTART_BACKOFF_CEILING,
            max: RESTART_BACKOFF_CEILING,
            exhausted: true
        },
    );
}

/// S-27 unaffected control — a liveness give-up is unchanged (tracked as F-2, GH
/// #336): not reported as exhausted; and a row with no terminal, or a restartable
/// probe stop, leaves the budget untouched.
#[tokio::test]
async fn a_liveness_give_up_no_terminal_and_a_probe_stop_leave_the_budget_untouched() {
    let untouched = RestartBudget { used: 0, max: RESTART_BACKOFF_CEILING, exhausted: false };
    assert_eq!(
        budget_for(
            Some(TerminalCondition::ServiceFailed {
                reason: ServiceFailureReason::LivenessProbeFailed {
                    probe_idx: 0,
                    attempts: RESTART_BACKOFF_CEILING,
                },
            }),
            AllocState::Failed,
        )
        .await,
        untouched,
        "LivenessProbeFailed is unchanged (F-2)",
    );
    assert_eq!(budget_for(None, AllocState::Running).await, untouched, "no terminal");
    assert_eq!(
        budget_for(
            Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }),
            AllocState::Terminated,
        )
        .await,
        untouched,
        "a restartable startup stop is not a give-up",
    );
}
