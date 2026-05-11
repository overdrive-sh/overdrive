//! Unit tests for pure render helper functions that were flagged as
//! missed mutations: `format_human_duration`, `derive_job_verdict`,
//! and the spec-digest branches of `alloc_status_kind_aware`.

#![allow(clippy::expect_used)]

use std::time::Duration;

use overdrive_cli::render::{
    JobVerdict, alloc_status_kind_aware, derive_job_verdict, format_human_duration,
};
use overdrive_control_plane::api::{
    AllocStateWire, AllocStatusResponse, AllocStatusRowBody, ResourcesBody, RestartBudget,
};
use overdrive_core::aggregate::WorkloadKind;

// ---------------------------------------------------------------------------
// format_human_duration — four-branch coverage
// ---------------------------------------------------------------------------

#[test]
fn human_duration_zero_renders_sub_1ms() {
    assert_eq!(format_human_duration(Duration::ZERO), "<1ms");
}

#[test]
fn human_duration_500ms() {
    assert_eq!(format_human_duration(Duration::from_millis(500)), "500ms");
}

#[test]
fn human_duration_boundary_999ms() {
    assert_eq!(format_human_duration(Duration::from_millis(999)), "999ms");
}

#[test]
fn human_duration_boundary_1000ms_enters_seconds_branch() {
    assert_eq!(format_human_duration(Duration::from_secs(1)), "1.0s");
}

#[test]
fn human_duration_sub_minute_with_tenths() {
    assert_eq!(format_human_duration(Duration::from_millis(5_500)), "5.5s");
}

#[test]
fn human_duration_boundary_59s() {
    assert_eq!(format_human_duration(Duration::from_secs(59)), "59.0s");
}

#[test]
fn human_duration_boundary_60s_enters_minutes_branch() {
    assert_eq!(format_human_duration(Duration::from_secs(60)), "1m0s");
}

#[test]
fn human_duration_minutes_and_seconds() {
    assert_eq!(format_human_duration(Duration::from_secs(90)), "1m30s");
}

#[test]
fn human_duration_tenths_truncation() {
    // 2345ms → 2s with 345ms remainder → tenths = 345/100 = 3
    assert_eq!(format_human_duration(Duration::from_millis(2_345)), "2.3s");
}

// ---------------------------------------------------------------------------
// derive_job_verdict — conjunction coverage
// ---------------------------------------------------------------------------

fn minimal_row(state: AllocStateWire, exit_code: Option<i32>) -> AllocStatusRowBody {
    AllocStatusRowBody {
        alloc_id: "alloc-test-0".to_string(),
        workload_id: "test".to_string(),
        node_id: "node-a".to_string(),
        state,
        reason: None,
        resources: ResourcesBody { cpu_milli: 100, memory_bytes: 1024 },
        started_at: None,
        exit_code,
        last_transition: None,
        error: None,
    }
}

#[test]
fn verdict_terminated_zero_is_succeeded() {
    let rows = vec![minimal_row(AllocStateWire::Terminated, Some(0))];
    assert_eq!(derive_job_verdict(&rows), JobVerdict::Succeeded);
}

#[test]
fn verdict_terminated_nonzero_is_failed() {
    let rows = vec![minimal_row(AllocStateWire::Terminated, Some(1))];
    assert_eq!(derive_job_verdict(&rows), JobVerdict::Failed);
}

#[test]
fn verdict_running_is_in_progress() {
    let rows = vec![minimal_row(AllocStateWire::Running, None)];
    assert_eq!(derive_job_verdict(&rows), JobVerdict::InProgress);
}

#[test]
fn verdict_terminated_nonzero_and_running_sibling_is_succeeded_not_in_progress() {
    // Terminated(0) takes priority over Running.
    let rows = vec![
        minimal_row(AllocStateWire::Terminated, Some(0)),
        minimal_row(AllocStateWire::Running, None),
    ];
    assert_eq!(derive_job_verdict(&rows), JobVerdict::Succeeded);
}

// ---------------------------------------------------------------------------
// alloc_status_kind_aware — spec_digest conditional rendering
// ---------------------------------------------------------------------------

fn status_fixture(kind: WorkloadKind, spec_digest: &str) -> AllocStatusResponse {
    AllocStatusResponse {
        workload_id: Some("test-wl".to_string()),
        spec_digest: Some(spec_digest.to_string()),
        replicas_desired: 1,
        replicas_running: 0,
        rows: vec![minimal_row(AllocStateWire::Pending, None)],
        restart_budget: Some(RestartBudget { used: 0, max: 5, exhausted: false }),
        kind: Some(kind),
    }
}

#[test]
fn service_branch_renders_spec_digest_when_present() {
    let out = status_fixture(WorkloadKind::Service, "abcd1234");
    let rendered = alloc_status_kind_aware(&out);
    assert!(
        rendered.contains("Spec digest: abcd1234"),
        "Service with non-empty spec_digest must render it; got:\n{rendered}",
    );
}

#[test]
fn service_branch_omits_spec_digest_when_empty() {
    let out = status_fixture(WorkloadKind::Service, "");
    let rendered = alloc_status_kind_aware(&out);
    assert!(
        !rendered.contains("Spec digest:"),
        "Service with empty spec_digest must omit the line; got:\n{rendered}",
    );
}

#[test]
fn schedule_branch_renders_spec_digest_when_present() {
    let out = status_fixture(WorkloadKind::Schedule, "abcd1234");
    let rendered = alloc_status_kind_aware(&out);
    assert!(
        rendered.contains("Spec digest: abcd1234"),
        "Schedule with non-empty spec_digest must render it; got:\n{rendered}",
    );
}

#[test]
fn schedule_branch_omits_spec_digest_when_empty() {
    let out = status_fixture(WorkloadKind::Schedule, "");
    let rendered = alloc_status_kind_aware(&out);
    assert!(
        !rendered.contains("Spec digest:"),
        "Schedule with empty spec_digest must omit the line; got:\n{rendered}",
    );
}
