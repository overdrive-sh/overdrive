//! Launches that end before running (ADR-0355): the one predicate
//! `is_launch_abandoned` and the two launch-ended reasons it names.
//!
//! Contract: a row is an abandoned launch iff it is `Terminated` and its reason is
//! `LaunchSuperseded` or `LaunchLostWithControlPlane`. Every other reason and every
//! other state (a `Failed` row in particular) is not. The two reasons are progress
//! markers, never failures, and carry their operator wording.
//!
//! CONTRACT_SHAPE: pure-function (a value or a string for a value; no state, no I/O).

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::doc_markdown)]

use std::net::Ipv4Addr;

use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{
    CancelledBy, StoppedBy, TransitionReason, is_launch_abandoned,
};
use proptest::prelude::*;

const ALL_STATES: [AllocState; 6] = [
    AllocState::Pending,
    AllocState::Running,
    AllocState::Draining,
    AllocState::Suspended,
    AllocState::Terminated,
    AllocState::Failed,
];

fn row(state: AllocState, reason: Option<TransitionReason>) -> AllocStatusRow {
    let node = NodeId::new("node-001").expect("node id");
    AllocStatusRow {
        alloc_id: AllocationId::new("alloc-web-0").expect("alloc id"),
        workload_id: WorkloadId::new("web").expect("workload id"),
        node_id: node.clone(),
        state,
        updated_at: LogicalTimestamp { counter: 1, writer: node },
        reason,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: None,
        workload_addr: Some(Ipv4Addr::new(10, 99, 0, 6)),
        last_terminated: None,
        restart_count: 0,
    }
}

const fn is_abandon_reason(reason: &TransitionReason) -> bool {
    matches!(
        reason,
        TransitionReason::LaunchSuperseded | TransitionReason::LaunchLostWithControlPlane
    )
}

fn arb_reason() -> impl Strategy<Value = Option<TransitionReason>> {
    prop_oneof![
        Just(None),
        Just(Some(TransitionReason::Scheduling)),
        Just(Some(TransitionReason::Starting)),
        Just(Some(TransitionReason::Started)),
        Just(Some(TransitionReason::Stopped { by: StoppedBy::Operator })),
        Just(Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed })),
        Just(Some(TransitionReason::Cancelled { by: CancelledBy::Operator })),
        Just(Some(TransitionReason::LaunchSuperseded)),
        Just(Some(TransitionReason::LaunchLostWithControlPlane)),
    ]
}

/// A `Terminated` row is an abandoned launch for exactly the two reasons.
#[test]
fn a_terminated_row_with_either_launch_ended_reason_is_an_abandoned_launch() {
    for reason in [TransitionReason::LaunchSuperseded, TransitionReason::LaunchLostWithControlPlane]
    {
        assert!(
            is_launch_abandoned(&row(AllocState::Terminated, Some(reason.clone()))),
            "a Terminated row with {reason:?} must be an abandoned launch"
        );
    }
}

/// A `Failed` row is never an abandoned launch, even with a launch-ended reason.
#[test]
fn a_failed_row_is_never_an_abandoned_launch() {
    for reason in [TransitionReason::LaunchSuperseded, TransitionReason::LaunchLostWithControlPlane]
    {
        assert!(
            !is_launch_abandoned(&row(AllocState::Failed, Some(reason.clone()))),
            "a Failed row with {reason:?} must not be an abandoned launch"
        );
    }
}

/// A `Terminated` row without a reason, or with an ordinary one, is not abandoned.
#[test]
fn a_terminated_row_without_a_launch_ended_reason_is_not_an_abandoned_launch() {
    assert!(!is_launch_abandoned(&row(AllocState::Terminated, None)));
    assert!(!is_launch_abandoned(&row(
        AllocState::Terminated,
        Some(TransitionReason::Stopped { by: StoppedBy::Operator })
    )));
}

proptest! {
    /// Over every (state, reason) pair, the predicate is exactly
    /// `state == Terminated && reason in {LaunchSuperseded, LaunchLostWithControlPlane}`.
    #[test]
    fn the_predicate_is_exactly_terminated_with_a_launch_ended_reason(
        state_idx in 0..ALL_STATES.len(),
        reason in arb_reason(),
    ) {
        let state = ALL_STATES[state_idx];
        let expected = state == AllocState::Terminated
            && reason.as_ref().is_some_and(is_abandon_reason);
        prop_assert_eq!(
            is_launch_abandoned(&row(state, reason.clone())),
            expected,
            "state {:?} reason {:?}", state, reason
        );
    }
}

/// The two reasons are progress markers: an abandoned launch is not a start failure.
#[test]
fn the_launch_ended_reasons_are_not_failures() {
    assert!(!TransitionReason::LaunchSuperseded.is_failure());
    assert!(!TransitionReason::LaunchLostWithControlPlane.is_failure());
}

/// Operator-facing wording of the two reasons.
#[test]
fn the_launch_ended_reasons_carry_their_wording() {
    assert_eq!(
        TransitionReason::LaunchSuperseded.human_readable(),
        "launch abandoned: superseded by a stop, a restart or node shutdown"
    );
    assert_eq!(
        TransitionReason::LaunchLostWithControlPlane.human_readable(),
        "launch abandoned: lost with the control plane"
    );
}
