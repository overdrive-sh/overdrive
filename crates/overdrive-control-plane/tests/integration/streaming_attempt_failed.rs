//! Regression tests for `workload_event_from_lifecycle` populating
//! `attempt_index`, `will_restart`, and `next_attempt_delay` from the
//! reconciler's view cache rather than hardcoding `(1, false, None)`.
//!
//! These tests open a real `RedbViewStore` via `TempDir` and are gated
//! behind `integration-tests` per `.claude/rules/testing.md`
//! § "Integration vs unit gating".

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;

use overdrive_core::TransitionReason;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconciler::{
    AnyReconciler, RESTART_BACKOFF_CEILING, RESTART_BACKOFF_DURATION, TargetResource,
    WorkloadLifecycle, WorkloadLifecycleView,
};
use overdrive_core::traits::driver::DriverType;
use overdrive_core::traits::observation_store::ObservationStore;
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
        source: TransitionSource::Driver(DriverType::Exec),
        at: "3@node-a".to_string(),
        terminal: None,
    }
}

#[tokio::test]
async fn attempt_failed_first_attempt_will_restart() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let node = NodeId::from_str("node-a").expect("node id");
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(node, 0));
    let alloc_id = AllocationId::from_str("alloc-pay-0").expect("alloc id");
    let wl_id = WorkloadId::from_str("payments").expect("wl id");
    let target = TargetResource::new("job/payments").expect("target");

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
            assert!(will_restart, "budget not exhausted — will_restart must be true");
            assert_eq!(exit_code, 42);
            let expected_delay = format!("{}ms", RESTART_BACKOFF_DURATION.as_millis());
            assert_eq!(
                next_attempt_delay.as_deref(),
                Some(expected_delay.as_str()),
                "will_restart=true must populate delay from backoff_for_attempt"
            );
        }
        other => panic!("expected AttemptFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn attempt_failed_mid_budget_reports_correct_index() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let node = NodeId::from_str("node-a").expect("node id");
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(node, 0));
    let alloc_id = AllocationId::from_str("alloc-pay-0").expect("alloc id");
    let wl_id = WorkloadId::from_str("payments").expect("wl id");
    let target = TargetResource::new("job/payments").expect("target");

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
            assert!(will_restart, "4 < CEILING(5) — will_restart must be true");
            assert!(
                next_attempt_delay.is_some(),
                "will_restart=true must populate next_attempt_delay"
            );
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
    let target = TargetResource::new("job/payments").expect("target");

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

#[tokio::test]
async fn attempt_failed_empty_view_defaults_conservative() {
    let tmp = tempfile::tempdir().expect("tmpdir");
    let node = NodeId::from_str("node-a").expect("node id");
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(node, 0));
    let alloc_id = AllocationId::from_str("alloc-pay-0").expect("alloc id");
    let wl_id = WorkloadId::from_str("payments").expect("wl id");
    let target = TargetResource::new("job/payments").expect("target");

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
            assert!(will_restart, "empty view → conservative will_restart=true");
            assert!(
                next_attempt_delay.is_some(),
                "empty view defaults to will_restart=true → delay must be populated"
            );
        }
        other => panic!("expected AttemptFailed, got {other:?}"),
    }
}
