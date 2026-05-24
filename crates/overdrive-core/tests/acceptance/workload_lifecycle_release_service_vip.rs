//! service-vip-allocator step 03-01 — `WorkloadLifecycle::reconcile`
//! emits `Action::ReleaseServiceVip` exactly once when a Service-kind
//! workload has reached a terminal-state observation row.
//!
//! Per ADR-0049 (amended 2026-05-15): the `ServiceVipAllocator` holds a
//! content-addressed memo keyed by `spec_digest`. When the reconciler
//! observes that every allocation belonging to a Service workload has
//! reached a terminal claim, it emits `Action::ReleaseServiceVip`
//! exactly once so the action shim (step 03-02) can reclaim the VIP.
//!
//! Per `.claude/rules/development.md` § "Persist inputs, not derived
//! state": the `released_for_terminal: BTreeSet<ContentHash>` field on
//! the View records the *input* "we already emitted release for this
//! spec_digest" — never a derived "needs release now" cache. The
//! gate is recomputed every tick.
//!
//! Per-layer scope: this step exercises the reconciler's emission
//! contract only. It does NOT instantiate an action shim or an
//! allocator. The end-to-end S-VIP-06 flow (submit → terminal →
//! tick → release → reallocate) is owned by step 03-03.

#![allow(clippy::expect_used)]
#![allow(clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver, WorkloadKind};
use overdrive_core::id::{AllocationId, ContentHash, NodeId, Region, WorkloadId};
use overdrive_core::reconcilers::{
    Action, Reconciler, TickContext, WorkloadLifecycle, WorkloadLifecycleState,
    WorkloadLifecycleView,
};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};

// -------------------------------------------------------------------
// Fixtures
// -------------------------------------------------------------------

fn nid(s: &str) -> NodeId {
    NodeId::new(s).expect("valid NodeId")
}

fn jid(s: &str) -> WorkloadId {
    WorkloadId::new(s).expect("valid WorkloadId")
}

fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}

fn local_region() -> Region {
    Region::new("local").expect("valid Region")
}

fn make_node(id: &str) -> Node {
    Node {
        id: nid(id),
        region: local_region(),
        capacity: Resources { cpu_milli: 4_000, memory_bytes: 8 * 1024 * 1024 * 1024 },
    }
}

fn make_job(id: &str) -> Job {
    Job {
        id: jid(id),
        replicas: NonZeroU32::new(1).expect("1 is non-zero"),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: WorkloadDriver::Exec(Exec { command: "/bin/true".to_string(), args: vec![] }),
    }
}

fn one_node_map(node_id: &str) -> BTreeMap<NodeId, Node> {
    let n = make_node(node_id);
    let mut m = BTreeMap::new();
    m.insert(n.id.clone(), n);
    m
}

fn fresh_tick(now: Instant, now_unix: UnixInstant) -> TickContext {
    TickContext { now, now_unix, tick: 0, deadline: now + Duration::from_secs(1) }
}

/// Construct an alloc row in a terminal state — `state: Terminated`
/// with a `terminal: Some(Stopped { by: Operator })` claim — matching
/// the shape the action shim writes after a `StopAllocation` for a
/// service workload.
fn alloc_terminal_operator_stopped(
    alloc_id: &str,
    workload_id: &str,
    node_id: &str,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(alloc_id),
        workload_id: jid(workload_id),
        node_id: nid(node_id),
        state: AllocState::Terminated,
        updated_at: LogicalTimestamp { counter: 2, writer: nid(node_id) },
        reason: Some(TransitionReason::Stopped { by: StoppedBy::Operator }),
        detail: None,
        terminal: Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
    }
}

fn fake_spec_digest() -> ContentHash {
    ContentHash::of(b"service-vip-03-01-fixture-digest")
}

fn service_state_with_terminal_alloc(
    workload_id: &str,
    spec_digest: Option<ContentHash>,
) -> (WorkloadLifecycleState, WorkloadLifecycleState) {
    let nodes = one_node_map("local");
    let mut allocations = BTreeMap::new();
    allocations.insert(
        aid("alloc-payments-0"),
        alloc_terminal_operator_stopped("alloc-payments-0", workload_id, "local"),
    );

    let desired = WorkloadLifecycleState {
        workload_id: jid(workload_id),
        job: Some(make_job(workload_id)),
        desired_to_stop: true,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: spec_digest,
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid(workload_id),
        job: Some(make_job(workload_id)),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: spec_digest,
    };
    (desired, actual)
}

// -------------------------------------------------------------------
// S-VIP-06 (reconciler-emission layer) — scenarios
// -------------------------------------------------------------------

/// Scenario 1: terminal-state observation triggers a single
/// `Action::ReleaseServiceVip` and records the digest in
/// `next_view.released_for_terminal`.
#[test]
fn terminal_state_emits_release_action_once() {
    let digest = fake_spec_digest();
    let (desired, actual) = service_state_with_terminal_alloc("payments", Some(digest));
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
    let (actions, next_view) = r.reconcile(&desired, &actual, &view, &tick);

    let release_actions: Vec<&Action> =
        actions.iter().filter(|a| matches!(a, Action::ReleaseServiceVip { .. })).collect();
    assert_eq!(
        release_actions.len(),
        1,
        "expected exactly one Action::ReleaseServiceVip on terminal-state observation; got {actions:?}",
    );
    match release_actions[0] {
        Action::ReleaseServiceVip { spec_digest, .. } => {
            assert_eq!(
                *spec_digest, digest,
                "release action must carry the workload's spec_digest"
            );
        }
        _ => unreachable!("filtered to ReleaseServiceVip above"),
    }

    assert!(
        next_view.released_for_terminal.contains(&digest),
        "next_view.released_for_terminal must record the emitted digest; got {:?}",
        next_view.released_for_terminal,
    );
}

/// Scenario 2: re-ticking with the digest already recorded in
/// `view.released_for_terminal` does NOT re-emit a release action.
#[test]
fn terminal_state_release_action_idempotent_on_reemit() {
    let digest = fake_spec_digest();
    let (desired, actual) = service_state_with_terminal_alloc("payments", Some(digest));
    let mut released = BTreeSet::new();
    released.insert(digest);
    let view = WorkloadLifecycleView {
        restart_counts: BTreeMap::new(),
        last_failure_seen_at: BTreeMap::new(),
        released_for_terminal: released,
    };
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
    let (actions, next_view) = r.reconcile(&desired, &actual, &view, &tick);

    let release_count =
        actions.iter().filter(|a| matches!(a, Action::ReleaseServiceVip { .. })).count();
    assert_eq!(
        release_count, 0,
        "re-tick must NOT re-emit ReleaseServiceVip once digest is in released_for_terminal; got {actions:?}",
    );
    assert!(
        next_view.released_for_terminal.contains(&digest),
        "next_view.released_for_terminal must still contain the digest after idempotent tick",
    );
}

/// Regression: Service workloads have `job: None` (read_job returns
/// `(None, Some(digest))` for `WorkloadIntent::Service`). The
/// correlation key must embed the real workload ID from
/// `desired.workload_id`, not fall back to `"unknown"`.
#[test]
fn service_release_correlation_uses_workload_id_not_unknown() {
    use overdrive_core::id::CorrelationKey;

    let digest = fake_spec_digest();
    let nodes = one_node_map("local");
    let mut allocations = BTreeMap::new();
    allocations.insert(
        aid("alloc-web-api-0"),
        alloc_terminal_operator_stopped("alloc-web-api-0", "web-api", "local"),
    );

    let desired = WorkloadLifecycleState {
        workload_id: jid("web-api"),
        job: None,
        desired_to_stop: true,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: Some(digest),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("web-api"),
        job: None,
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: Some(digest),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
    let (actions, _) = r.reconcile(&desired, &actual, &view, &tick);

    let release = actions
        .iter()
        .find(|a| matches!(a, Action::ReleaseServiceVip { .. }))
        .expect("Service with terminal alloc must emit ReleaseServiceVip");

    let expected_correlation =
        CorrelationKey::derive("job-lifecycle/web-api", &digest, "release-service-vip");
    let wrong_correlation =
        CorrelationKey::derive("job-lifecycle/unknown", &digest, "release-service-vip");

    match release {
        Action::ReleaseServiceVip { correlation, .. } => {
            assert_eq!(
                *correlation, expected_correlation,
                "correlation must embed the real workload ID 'web-api'"
            );
            assert_ne!(
                *correlation, wrong_correlation,
                "correlation must NOT embed 'unknown' for Service workloads"
            );
        }
        _ => unreachable!(),
    }
}
