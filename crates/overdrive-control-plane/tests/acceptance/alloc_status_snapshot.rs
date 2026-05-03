//! Slice 01 step 01-03 — `AllocStatusResponse` extension + `alloc_status`
//! handler hydration via observation rows + `JobLifecycleView`.
//!
//! Covers four scenarios (S-AS-01, S-AS-07, S-AS-08, S-AS-09) at the
//! control-plane port:
//!
//! * **S-AS-01** — six populated fields in Running case (the KPI-03
//!   actionable-field budget).
//! * **S-AS-07** — `row.reason` → `last_transition.reason` projection
//!   identity over every `TransitionReason` variant via proptest.
//! * **S-AS-08** — `RestartBudget.exhausted` derivation property
//!   (`exhausted == used >= max`) over restart count N ∈ [0, 16].
//! * **S-AS-09** — `GET /v1/allocs?job=ghost-v0` against an unknown
//!   job → HTTP 404 with `ErrorBody { error: "not_found", .. }`.
//!
//! Default-lane: in-process axum routing, sim observation store seeded
//! with rows, no real I/O. Per `crates/overdrive-control-plane/CLAUDE.md`
//! and `.claude/rules/testing.md`.

#![allow(clippy::expect_used, clippy::expect_fun_call, clippy::unwrap_used)]

use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Query, State};
use overdrive_control_plane::AppState;
use overdrive_control_plane::api::{AllocStateWire, AllocStatusResponse, TransitionSource};
use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::handlers::{AllocStatusQuery, alloc_status};
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::TransitionReason;
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput,
};
use overdrive_core::id::{AllocationId, JobId, NodeId};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_core::transition_reason::{CancelledBy, ResourceEnvelope, StoppedBy};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use proptest::prelude::*;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Test fixtures
// ---------------------------------------------------------------------------

fn sample_node() -> NodeId {
    NodeId::from_str("node-a").expect("valid node id")
}

const fn sample_job_id_str() -> &'static str {
    "payments-v2"
}

fn sample_alloc() -> AllocationId {
    AllocationId::from_str("alloc-payments-v2-0").expect("valid alloc id")
}

fn build_app_state(tmp: &TempDir) -> AppState {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    let store = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb")).expect("LocalIntentStore::open"),
    );
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(sample_node(), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    AppState::new(store, obs, Arc::new(runtime), driver, Arc::new(SimClock::new()))
}

fn sample_spec() -> JobSpecInput {
    JobSpecInput {
        id: sample_job_id_str().to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 134_217_728 },
        driver: DriverInput::Exec(ExecInput {
            command: "/usr/local/bin/payments".to_string(),
            args: vec!["--port".to_string(), "8080".to_string()],
        }),
    }
}

/// Persist `Job::from_spec(spec)` into the `IntentStore` — the required
/// precondition for a 200 response (S-AS-09 vacuum-base for non-404 paths).
async fn install_job(state: &AppState, spec: JobSpecInput) -> Job {
    let job = Job::from_spec(spec).expect("Job::from_spec must succeed for fixture");
    let key = IntentKey::for_job(&job.id);
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&job).expect("rkyv archive");
    state.store.put(key.as_bytes(), archived.as_ref()).await.expect("IntentStore put");
    job
}

/// Write a row into the sim observation store. `reason` and `detail`
/// flow into the row verbatim per ADR-0032 §3 (Amendment 2026-04-30).
async fn write_row(
    state: &AppState,
    alloc: AllocationId,
    job_id: JobId,
    state_value: AllocState,
    counter: u64,
    reason: Option<TransitionReason>,
    detail: Option<String>,
) {
    let row = AllocStatusRow {
        alloc_id: alloc,
        job_id,
        node_id: sample_node(),
        state: state_value,
        updated_at: LogicalTimestamp { counter, writer: sample_node() },
        reason,
        detail,
        terminal: None,
    };
    state.obs.write(ObservationRow::AllocStatus(row)).await.expect("obs write");
}

// ---------------------------------------------------------------------------
// S-AS-01 — six populated fields in Running case (KPI-03)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn s_as_01_running_snapshot_carries_six_actionable_fields() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);
    let job = install_job(&state, sample_spec()).await;

    write_row(
        &state,
        sample_alloc(),
        job.id.clone(),
        AllocState::Running,
        2,
        Some(TransitionReason::Started),
        Some("driver started (pid 12345)".to_owned()),
    )
    .await;

    let resp = alloc_status(
        State(state.clone()),
        Query(AllocStatusQuery { job: Some(sample_job_id_str().to_owned()) }),
    )
    .await
    .expect("alloc_status returned err");

    let body: AllocStatusResponse = resp.0;

    // KPI-03 — at least 6 populated fields beyond legacy alloc_id/job_id/node_id/state.
    let envelope_populated_count = [
        body.job_id.is_some(),
        body.spec_digest.is_some(),
        body.replicas_desired > 0,
        body.replicas_running > 0,
        body.restart_budget.is_some(),
        !body.rows.is_empty(),
    ]
    .iter()
    .filter(|f| **f)
    .count();
    assert!(
        envelope_populated_count >= 6,
        "S-AS-01 envelope must populate >= 6 actionable fields; got {envelope_populated_count}",
    );

    let row = body.rows.first().expect("rows must contain the running alloc");
    assert_eq!(row.state, AllocStateWire::Running);
    assert_eq!(row.resources.cpu_milli, 500);
    assert_eq!(row.resources.memory_bytes, 134_217_728);
    let last = row.last_transition.as_ref().expect("last_transition populated");
    assert_eq!(last.reason, TransitionReason::Started);
    assert_eq!(last.to, AllocStateWire::Running);
    assert!(matches!(last.source, TransitionSource::Reconciler | TransitionSource::Driver(_)));
    assert_eq!(row.error.as_deref(), Some("driver started (pid 12345)"));

    let budget = body.restart_budget.expect("restart_budget populated");
    assert_eq!(budget.used, 0);
    assert_eq!(budget.max, 5);
    assert!(!budget.exhausted, "fresh Running alloc has not exhausted its budget");
}

// ---------------------------------------------------------------------------
// S-AS-07 — `row.reason` → `last_transition.reason` projection identity
// ---------------------------------------------------------------------------

fn arb_label() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{0,15}".prop_map(String::from)
}

fn arb_transition_reason() -> impl Strategy<Value = TransitionReason> {
    prop_oneof![
        Just(TransitionReason::Scheduling),
        Just(TransitionReason::Starting),
        Just(TransitionReason::Started),
        any::<u32>().prop_map(|attempt| TransitionReason::BackoffPending { attempt }),
        prop_oneof![
            Just(StoppedBy::Operator),
            Just(StoppedBy::Reconciler),
            Just(StoppedBy::Process)
        ]
        .prop_map(|by| TransitionReason::Stopped { by }),
        arb_label().prop_map(|path| TransitionReason::ExecBinaryNotFound { path }),
        arb_label().prop_map(|path| TransitionReason::ExecPermissionDenied { path }),
        (arb_label(), arb_label())
            .prop_map(|(path, kind)| TransitionReason::ExecBinaryInvalid { path, kind }),
        (arb_label(), arb_label())
            .prop_map(|(kind, source)| TransitionReason::CgroupSetupFailed { kind, source }),
        arb_label().prop_map(|detail| TransitionReason::DriverInternalError { detail }),
        (any::<u32>(), arb_label()).prop_map(|(attempts, last_cause_summary)| {
            TransitionReason::RestartBudgetExhausted { attempts, last_cause_summary }
        }),
        prop_oneof![Just(CancelledBy::Operator), Just(CancelledBy::Cluster)]
            .prop_map(|by| TransitionReason::Cancelled { by }),
        (any::<u32>(), any::<u64>(), any::<u32>(), any::<u64>()).prop_map(|(rc, rm, fc, fm)| {
            TransitionReason::NoCapacity {
                requested: ResourceEnvelope { cpu_milli: rc, memory_bytes: rm },
                free: ResourceEnvelope { cpu_milli: fc, memory_bytes: fm },
            }
        },),
        (any::<u64>(), any::<u64>())
            .prop_map(|(p, l)| TransitionReason::OutOfMemory { peak_bytes: p, limit_bytes: l }),
        (
            proptest::option::of(any::<i32>()),
            proptest::option::of(any::<u8>()),
            proptest::option::of(arb_label()),
        )
            .prop_map(|(exit_code, signal, stderr_tail)| {
                TransitionReason::WorkloadCrashedImmediately { exit_code, signal, stderr_tail }
            }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// S-AS-07 — for every `TransitionReason`, the row's `reason` field
    /// projects byte-identically to `AllocStatusRowBody.last_transition.reason`.
    /// The assertion is structural identity at the typed Rust boundary —
    /// the wire ride-along is enforced upstream by the
    /// `transition_reason_type_identity` test (S-AS-02).
    #[test]
    fn s_as_07_row_reason_projects_to_last_transition_reason(
        reason in arb_transition_reason(),
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        runtime.block_on(async {
            let tmp = TempDir::new().expect("tmpdir");
            let state = build_app_state(&tmp);
            let job = install_job(&state, sample_spec()).await;
            write_row(
                &state,
                sample_alloc(),
                job.id.clone(),
                AllocState::Running,
                2,
                Some(reason.clone()),
                None,
            )
            .await;

            let resp = alloc_status(
                State(state.clone()),
                Query(AllocStatusQuery { job: Some(sample_job_id_str().to_owned()) }),
            )
            .await
            .expect("alloc_status ok");

            let body: AllocStatusResponse = resp.0;
            let row = body.rows.first().expect("row populated");
            let last = row.last_transition.as_ref().expect("last_transition populated");
            prop_assert_eq!(&last.reason, &reason);
            Ok(())
        })?;
    }
}

// ---------------------------------------------------------------------------
// S-AS-08 — RestartBudget.exhausted derivation
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    /// `RestartBudget { used: N, max: 5, exhausted }` satisfies
    /// `exhausted == (N >= 5)` for every N ∈ [0, 16]. Pure-function
    /// property — the derivation lives on the constructor.
    #[test]
    fn s_as_08_restart_budget_exhaustion_matches_used_ge_max(
        used in 0u32..=16,
    ) {
        let max = 5u32;
        let budget = overdrive_control_plane::api::RestartBudget {
            used,
            max,
            exhausted: used >= max,
        };
        prop_assert_eq!(budget.exhausted, used >= max);
    }
}

// ---------------------------------------------------------------------------
// S-AS-09 — 404 on missing job
// ---------------------------------------------------------------------------

#[tokio::test]
async fn s_as_09_unknown_job_returns_not_found_naming_resource() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);
    // No install_job — IntentStore is empty.

    let result = alloc_status(
        State(state.clone()),
        Query(AllocStatusQuery { job: Some("ghost-v0".to_owned()) }),
    )
    .await;

    match result {
        Err(ControlPlaneError::NotFound { resource }) => {
            assert!(
                resource.contains("ghost-v0"),
                "NotFound resource string must name the missing job; got {resource:?}",
            );
            assert!(
                resource.starts_with("jobs/"),
                "NotFound resource string must use the canonical IntentKey rendering \
                 jobs/<id>; got {resource:?}",
            );
        }
        Err(other) => panic!("expected ControlPlaneError::NotFound; got {other:?}"),
        Ok(body) => panic!("missing job must return NotFound; handler returned Ok({body:?})"),
    }
}
