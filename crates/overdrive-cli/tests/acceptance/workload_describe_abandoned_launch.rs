//! ADR-0355 operator surface: `render::workload_describe` (the live renderer; the historical
//! `alloc_status` renderers are retired) shows a launch that ended before it ran as the
//! workload's current allocation, `Terminated`, with the operator wording of its reason
//! and nothing that reads as a failure.
//!
//! Contract: the row is rendered through the one cause-detail helper, so the only difference
//! from the same row without a reason is the single `reason:` line carrying
//! `TransitionReason::human_readable()`. Driven through the render function over a wire
//! fixture (the pure-render discipline of `crates/overdrive-cli/CLAUDE.md`); the rows are the
//! shape the server returns for an abandoned launch (state `Terminated`, no terminal claim,
//! never started, the `Pending` row's address carried).
//!
//! Evidence tier: unit (layer 1). CONTRACT_SHAPE: pure-function.

#![allow(clippy::expect_used, clippy::doc_markdown)]

use overdrive_cli::commands::workload::WorkloadDescribeOutput;
use overdrive_cli::render::workload_describe;
use overdrive_control_plane::api::{
    AllocStateWire, AllocStatusResponse, AllocStatusRowBody, ResourcesBody,
};
use overdrive_core::TransitionReason;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::byte_size::ByteSize;
use overdrive_core::cpu_quantity::CpuQuantity;
use overdrive_testing::placement;

placement!(host_pure);

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn abandoned_row(alloc_id: &str, reason: Option<TransitionReason>) -> AllocStatusRowBody {
    AllocStatusRowBody {
        alloc_id: alloc_id.to_owned(),
        workload_id: "payments".to_owned(),
        node_id: "node-a".to_owned(),
        state: AllocStateWire::Terminated,
        reason,
        resources: ResourcesBody {
            cpu: CpuQuantity::from_millis(100),
            memory: ByteSize::new(1024),
        },
        workload_addr: Some("10.99.128.2".parse().expect("valid guest address")),
        started_at: None,
        exit_code: None,
        last_transition: None,
        error: None,
        last_terminated: None,
        terminal: None,
        network_cleanup_pending: false,
        svid_issue_failures: 0,
        last_svid_issue_failure: None,
        restart_count: 0,
    }
}

fn describe(rows: Vec<AllocStatusRowBody>) -> String {
    workload_describe(&WorkloadDescribeOutput {
        workload_id: "payments".to_owned(),
        spec_digest: DIGEST.to_owned(),
        allocations_total: rows.len(),
        empty_state_message: String::new(),
        snapshot: AllocStatusResponse {
            workload_id: Some("payments".to_owned()),
            spec_digest: Some(DIGEST.to_owned()),
            kind: Some(WorkloadKind::Service),
            replicas_desired: 1,
            replicas_running: 0,
            rows,
            ..Default::default()
        },
    })
}

/// The one rendered delta of the reason, exactly once, and the rest of the output byte-equal
/// to the same row without a reason. The state reads `Terminated`, never `Failed`.
fn assert_reason_is_the_only_delta(reason: TransitionReason, wording: &str) {
    let baseline = describe(vec![abandoned_row("alloc-payments-1", None)]);
    let rendered = describe(vec![abandoned_row("alloc-payments-1", Some(reason))]);
    let delta = format!("    reason: {wording}\n");
    assert_eq!(
        rendered.match_indices(&delta).count(),
        1,
        "describe must render the abandoned launch's reason line once; got:\n{rendered}"
    );
    assert_eq!(
        rendered.replacen(&delta, "", 1),
        baseline,
        "the reason line must be the only difference from the same row without a reason"
    );
    assert!(
        rendered.contains("alloc-payments-1") && rendered.contains("Terminated"),
        "the abandoned launch must be listed as a Terminated allocation; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("Failed"),
        "an abandoned launch is not a start failure and must not read as Failed; got:\n{rendered}"
    );
}

/// A launch superseded by a stop, a restart or node shutdown reads with its wording.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn describe_shows_a_superseded_launch_as_terminated_with_its_wording() {
    assert_reason_is_the_only_delta(
        TransitionReason::LaunchSuperseded,
        "launch abandoned: superseded by a stop, a restart or node shutdown",
    );
}

/// A launch lost with the control plane reads with its wording.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn describe_shows_a_launch_lost_with_the_control_plane_as_terminated_with_its_wording() {
    assert_reason_is_the_only_delta(
        TransitionReason::LaunchLostWithControlPlane,
        "launch abandoned: lost with the control plane",
    );
}

/// The abandoned launch is the workload's current allocation: with an earlier crashed row
/// above it in id order, both rows are listed and the newest (the abandoned one) is last,
/// carrying its reason beneath its own row.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn describe_lists_an_abandoned_launch_after_the_row_it_replaced() {
    let mut crashed = abandoned_row("alloc-payments-0", None);
    crashed.state = AllocStateWire::Failed;
    let abandoned = abandoned_row("alloc-payments-1", Some(TransitionReason::LaunchSuperseded));
    let rendered = describe(vec![crashed, abandoned]);
    let first = rendered.find("alloc-payments-0").expect("the crashed row is listed");
    let second = rendered.find("alloc-payments-1").expect("the abandoned row is listed");
    let reason = rendered
        .find("reason: launch abandoned: superseded by a stop, a restart or node shutdown")
        .expect("the reason is rendered");
    assert!(
        first < second && second < reason,
        "the abandoned launch must be listed after the row it replaced, with its reason beneath it; got:\n{rendered}"
    );
}
