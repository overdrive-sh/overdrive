//! Regression tests for `workload_event_from_lifecycle` populating
//! `attempt_index`, `will_restart`, and `next_attempt_delay` from the
//! reconciler's view cache rather than hardcoding `(1, false, None)`.
//!
//! These tests open a real `RedbViewStore` via `TempDir` and are gated
//! behind `integration-tests` per `.claude/rules/testing.md`
//! § "Integration vs unit gating".
//!
//! RESTART-SSOT (GH #324, DELIVER step 0): this projection is the Job
//! stream, and the reconciler finalizes a Job's first natural exit, a crash
//! included, so it never places a Job crash again. `will_restart` is the
//! reconciler's decision (`restart_status_for_alloc(target, Job, ..)`):
//! `false` for every Job crash, with no next-attempt delay. The tests that
//! pinned the pre-fix `true` now pin the fixed value and are pending step 0.

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;

use overdrive_core::TransitionReason;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::TargetResource;
use overdrive_core::traits::driver::DriverType;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_reconcilers::{
    AnyReconciler, RESTART_BACKOFF_CEILING, WorkloadLifecycle, WorkloadLifecycleView,
};
use overdrive_sim::adapters::observation_store::SimObservationStore;

use overdrive_control_plane::action_shim::LifecycleEvent;
use overdrive_control_plane::api::{AllocStateWire, TransitionSource};
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_control_plane::streaming::{JobSubmitEvent, workload_event_from_lifecycle};

async fn runtime_with_view(
    tmp: &tempfile::TempDir,
    target: &TargetResource,
    view: Option<WorkloadLifecycleView>,
) -> ReconcilerRuntime {
    let mut rt = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    rt.register(AnyReconciler::WorkloadLifecycle(WorkloadLifecycle::canonical()))
        .await
        .expect("register");
    if let Some(v) = view {
        rt.seed_workload_lifecycle_view_for_test(target, v);
    }
    rt
}

fn failed_lifecycle_event(
    alloc_id: &AllocationId,
    workload_id: &WorkloadId,
    exit_code: Option<i32>,
) -> LifecycleEvent {
    LifecycleEvent {
        alloc_id: alloc_id.clone(),
        workload_id: workload_id.clone(),
        from: AllocStateWire::Running,
        to: AllocStateWire::Failed,
        reason: TransitionReason::WorkloadCrashedImmediately {
            exit_code,
            signal: None,
            stderr_tail: None,
        },
        detail: None,
        source: TransitionSource::Driver(DriverType::Vm),
        at: "3@node-a".to_string(),
        terminal: None,
    }
}

/// Contract shape: pure-function.
#[tokio::test]
#[ignore = "pending DELIVER step 0"]
async fn attempt_failed_first_attempt_of_a_job_will_not_restart() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let node = NodeId::from_str("node-a").expect("node id");
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(node, 0));
    let alloc_id = AllocationId::from_str("alloc-pay-0").expect("alloc id");
    let wl_id = WorkloadId::from_str("payments").expect("wl id");
    let target = TargetResource::new("workload/payments").expect("target");

    let view = WorkloadLifecycleView {
        restart_counts: BTreeMap::from([(alloc_id.clone(), 0)]),
        ..Default::default()
    };
    let rt = runtime_with_view(&tmp, &target, Some(view)).await;

    let event = failed_lifecycle_event(&alloc_id, &wl_id, Some(42));
    let result = workload_event_from_lifecycle(&*obs, &rt, &wl_id, &event).await;

    match result {
        Some(JobSubmitEvent::AttemptFailed {
            attempt_index,
            will_restart,
            exit_code,
            next_attempt_delay,
            ..
        }) => {
            assert_eq!(attempt_index, 1, "first attempt should be index 1");
            assert!(!will_restart, "a Job crash is finalized, never placed again (GH #324)");
            assert_eq!(exit_code, 42);
            assert_eq!(next_attempt_delay, None, "no next attempt → no delay");
        }
        other => panic!("expected AttemptFailed, got {other:?}"),
    }
}

/// Contract shape: pure-function.
#[tokio::test]
#[ignore = "pending DELIVER step 0"]
async fn attempt_failed_mid_budget_reports_correct_index() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let node = NodeId::from_str("node-a").expect("node id");
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(node, 0));
    let alloc_id = AllocationId::from_str("alloc-pay-0").expect("alloc id");
    let wl_id = WorkloadId::from_str("payments").expect("wl id");
    let target = TargetResource::new("workload/payments").expect("target");

    let view = WorkloadLifecycleView {
        restart_counts: BTreeMap::from([(alloc_id.clone(), 3)]),
        ..Default::default()
    };
    let rt = runtime_with_view(&tmp, &target, Some(view)).await;

    let event = failed_lifecycle_event(&alloc_id, &wl_id, Some(1));
    let result = workload_event_from_lifecycle(&*obs, &rt, &wl_id, &event).await;

    match result {
        Some(JobSubmitEvent::AttemptFailed {
            attempt_index,
            will_restart,
            next_attempt_delay,
            ..
        }) => {
            assert_eq!(attempt_index, 4, "restart_counts=3 → attempt_index=4");
            assert!(!will_restart, "a Job crash is never placed again, below the ceiling too");
            assert_eq!(next_attempt_delay, None, "no next attempt → no delay");
        }
        other => panic!("expected AttemptFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn attempt_failed_at_ceiling_will_not_restart() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let node = NodeId::from_str("node-a").expect("node id");
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(node, 0));
    let alloc_id = AllocationId::from_str("alloc-pay-0").expect("alloc id");
    let wl_id = WorkloadId::from_str("payments").expect("wl id");
    let target = TargetResource::new("workload/payments").expect("target");

    let view = WorkloadLifecycleView {
        restart_counts: BTreeMap::from([(alloc_id.clone(), RESTART_BACKOFF_CEILING)]),
        ..Default::default()
    };
    let rt = runtime_with_view(&tmp, &target, Some(view)).await;

    let event = failed_lifecycle_event(&alloc_id, &wl_id, Some(1));
    let result = workload_event_from_lifecycle(&*obs, &rt, &wl_id, &event).await;

    match result {
        Some(JobSubmitEvent::AttemptFailed {
            attempt_index,
            will_restart,
            next_attempt_delay,
            ..
        }) => {
            assert_eq!(
                attempt_index,
                RESTART_BACKOFF_CEILING + 1,
                "at ceiling → attempt_index = CEILING + 1"
            );
            assert!(!will_restart, "at ceiling — will_restart must be false");
            assert_eq!(
                next_attempt_delay, None,
                "will_restart=false → no next attempt → delay must be None"
            );
        }
        other => panic!("expected AttemptFailed, got {other:?}"),
    }
}

/// Contract shape: pure-function.
#[tokio::test]
#[ignore = "pending DELIVER step 0"]
async fn attempt_failed_empty_view_reports_the_first_attempt_that_will_not_restart() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let node = NodeId::from_str("node-a").expect("node id");
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(node, 0));
    let alloc_id = AllocationId::from_str("alloc-pay-0").expect("alloc id");
    let wl_id = WorkloadId::from_str("payments").expect("wl id");
    let target = TargetResource::new("workload/payments").expect("target");

    let rt = runtime_with_view(&tmp, &target, None).await;

    let event = failed_lifecycle_event(&alloc_id, &wl_id, Some(1));
    let result = workload_event_from_lifecycle(&*obs, &rt, &wl_id, &event).await;

    match result {
        Some(JobSubmitEvent::AttemptFailed {
            attempt_index,
            will_restart,
            next_attempt_delay,
            ..
        }) => {
            assert_eq!(attempt_index, 1, "empty view → first attempt");
            assert!(
                !will_restart,
                "the default view decides as the reconciler does: a Job crash is finalized"
            );
            assert_eq!(next_attempt_delay, None, "no next attempt → no delay");
        }
        other => panic!("expected AttemptFailed, got {other:?}"),
    }
}
