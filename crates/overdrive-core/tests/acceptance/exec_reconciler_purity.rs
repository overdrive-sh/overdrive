//! Acceptance scenarios for `wire-exec-spec-end-to-end` — the
//! `JobLifecycle::reconcile` projection of `job.command` / `job.args`
//! into `Action::StartAllocation { spec }` and
//! `Action::RestartAllocation { alloc_id, spec }`.
//!
//! Covers `docs/feature/wire-exec-spec-end-to-end/distill/test-scenarios.md`
//! §5 *Reconciler purity*.
//!
//! The kill targets:
//!   - The literal `/bin/sleep` / `["60"]` at `reconciler.rs:1194-1195`
//!     (production code carrying test-fixture intent).
//!   - The new `spec: AllocationSpec` field on
//!     `Action::RestartAllocation` (per ADR-0031 §5).
//!   - Reconciler purity — `reconcile()` is byte-equal across two
//!     invocations with the same input (twin-invocation invariant per
//!     ADR-0013).
//!
//! Tests enter through the driving port (`JobLifecycle::reconcile`
//! via the `Reconciler` trait) and assert observable outcomes (returned
//! `Vec<Action>` shape with exact `spec` field equality). No internal
//! state is peeked.

#![allow(clippy::expect_used)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver};
use overdrive_core::id::{AllocationId, JobId, NodeId, Region};
use overdrive_core::reconciler::{
    Action, JobLifecycle, JobLifecycleState, JobLifecycleView, Reconciler, TickContext,
};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};

// ---------------------------------------------------------------------------
// fixtures
// ---------------------------------------------------------------------------

fn nid(s: &str) -> NodeId {
    NodeId::new(s).expect("valid NodeId")
}

fn jid(s: &str) -> JobId {
    JobId::new(s).expect("valid JobId")
}

fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}

fn local_region() -> Region {
    Region::new("local").expect("valid Region")
}

fn make_node(id: &str, capacity: Resources) -> Node {
    Node { id: nid(id), region: local_region(), capacity }
}

/// Construct a `Job` aggregate directly (NOT through `Job::from_spec`)
/// so the test fixture wires explicit `command` / `args` values without
/// depending on the new wire-shape input twin. This keeps the focused
/// scenario isolated from the input shape — a regression in the input
/// reshape would not silently flip the meaning of these tests.
///
/// Per ADR-0031 Amendment 1 the `Job` carries a tagged-enum
/// `driver: WorkloadDriver` field; the fixture wraps the supplied
/// command + args in `WorkloadDriver::Exec(Exec { ... })`.
fn make_job_with_command_args(
    id: &str,
    command: &str,
    args: Vec<String>,
    resources: Resources,
) -> Job {
    Job {
        id: jid(id),
        replicas: NonZeroU32::new(1).expect("1 is non-zero"),
        resources,
        driver: WorkloadDriver::Exec(Exec { command: command.to_string(), args }),
    }
}

fn alloc_with_state(
    alloc_id: &str,
    job_id: &str,
    node_id: &str,
    state: AllocState,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(alloc_id),
        job_id: jid(job_id),
        node_id: nid(node_id),
        state,
        updated_at: LogicalTimestamp { counter: 1, writer: nid(node_id) },
        reason: None,
        detail: None,
        terminal: None,
    }
}

fn one_node_map(node_id: &str) -> BTreeMap<NodeId, Node> {
    let n =
        make_node(node_id, Resources { cpu_milli: 4_000, memory_bytes: 8 * 1024 * 1024 * 1024 });
    let mut m = BTreeMap::new();
    m.insert(n.id.clone(), n);
    m
}

fn one_alloc_map(alloc_id: &str, row: AllocStatusRow) -> BTreeMap<AllocationId, AllocStatusRow> {
    let mut m = BTreeMap::new();
    m.insert(aid(alloc_id), row);
    m
}

const fn empty_alloc_map() -> BTreeMap<AllocationId, AllocStatusRow> {
    BTreeMap::new()
}

/// Canonical `fresh_tick` signature (uniform across every acceptance
/// suite per step 03-01): callers pass both `now` (monotonic) and
/// `now_unix` (wall-clock) explicitly. Tests that do not exercise the
/// wall-clock domain pass
/// `UnixInstant::from_unix_duration(Duration::from_secs(0))`.
fn fresh_tick(now: Instant, now_unix: UnixInstant) -> TickContext {
    TickContext { now, now_unix, tick: 0, deadline: now + Duration::from_secs(1) }
}

// ---------------------------------------------------------------------------
// §5 — Start carries operator-declared command + args (no /bin/sleep)
// ---------------------------------------------------------------------------

#[test]
fn start_action_carries_full_alloc_spec_from_live_job_command_and_args() {
    // Given a Job whose operator-declared command is /opt/payments/bin/server
    // and args are ["--port", "8080"].
    let job_resources = Resources { cpu_milli: 500, memory_bytes: 256 * 1024 * 1024 };
    let job = make_job_with_command_args(
        "payments",
        "/opt/payments/bin/server",
        vec!["--port".to_string(), "8080".to_string()],
        job_resources,
    );

    // No allocations present — the reconciler enters the fresh-start
    // branch and emits StartAllocation.
    let nodes = one_node_map("local");
    let desired = JobLifecycleState {
        job: Some(job.clone()),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
    };
    let actual = JobLifecycleState {
        job: Some(job),
        desired_to_stop: false,
        nodes,
        allocations: empty_alloc_map(),
    };
    let view = JobLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = JobLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    // Then exactly one StartAllocation, and the spec carries the
    // operator's declared command + args (NOT /bin/sleep + ["60"]).
    assert_eq!(actions.len(), 1, "must emit one StartAllocation; got {actions:?}");
    match &actions[0] {
        Action::StartAllocation { spec, .. } => {
            assert_eq!(
                spec.command, "/opt/payments/bin/server",
                "spec.command must equal the operator's declared command, NOT the deleted /bin/sleep literal",
            );
            assert_eq!(
                spec.args,
                vec!["--port".to_string(), "8080".to_string()],
                "spec.args must equal the operator's declared args, NOT the deleted [\"60\"] literal",
            );
            assert_eq!(
                spec.resources, job_resources,
                "spec.resources must equal the operator's declared resources",
            );
        }
        other => panic!("expected StartAllocation, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// §5 — Restart carries operator-declared command + args from live Job
// ---------------------------------------------------------------------------

#[test]
fn restart_action_carries_full_alloc_spec_from_live_job() {
    // Given a Job whose command is /opt/x/y and args are ["--mode=fast"]
    let job_resources = Resources { cpu_milli: 200, memory_bytes: 128 * 1024 * 1024 };
    let job = make_job_with_command_args(
        "payments",
        "/opt/x/y",
        vec!["--mode=fast".to_string()],
        job_resources,
    );

    // And one Terminated alloc for the job (eligible for restart).
    let nodes = one_node_map("local");
    let allocations = one_alloc_map(
        "alloc-payments-0",
        alloc_with_state("alloc-payments-0", "payments", "local", AllocState::Terminated),
    );
    let desired = JobLifecycleState {
        job: Some(job.clone()),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
    };
    let actual = JobLifecycleState { job: Some(job), desired_to_stop: false, nodes, allocations };
    // attempts=0, no deadline → restart fires immediately.
    let view = JobLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = JobLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    // Then exactly one RestartAllocation, and the spec carries the
    // operator's declared command + args + resources.
    assert_eq!(actions.len(), 1, "must emit one RestartAllocation; got {actions:?}");
    match &actions[0] {
        Action::RestartAllocation { alloc_id, spec } => {
            assert_eq!(alloc_id.as_str(), "alloc-payments-0");
            assert_eq!(
                spec.command, "/opt/x/y",
                "Restart spec.command must equal the live Job.command \
                 (NOT the deleted action_shim::default fabrication)",
            );
            assert_eq!(
                spec.args,
                vec!["--mode=fast".to_string()],
                "Restart spec.args must equal the live Job.args",
            );
            assert_eq!(
                spec.resources, job_resources,
                "Restart spec.resources must equal the live Job.resources \
                 (NOT the deleted default_restart_resources fabrication)",
            );
            assert_eq!(spec.alloc, *alloc_id);
        }
        other => panic!("expected RestartAllocation, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Twin-invocation invariant — reconcile() is deterministic
// ---------------------------------------------------------------------------

#[test]
fn reconcile_with_exec_spec_is_deterministic_across_twin_invocations() {
    // Given a Job with a non-trivial command + args spec.
    let job_resources = Resources { cpu_milli: 500, memory_bytes: 256 * 1024 * 1024 };
    let job = make_job_with_command_args(
        "payments",
        "/opt/payments/bin/server",
        vec!["--port".to_string(), "8080".to_string()],
        job_resources,
    );

    let nodes = one_node_map("local");
    let desired = JobLifecycleState {
        job: Some(job.clone()),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
    };
    let actual = JobLifecycleState {
        job: Some(job),
        desired_to_stop: false,
        nodes,
        allocations: empty_alloc_map(),
    };
    let view = JobLifecycleView::default();

    // Pin a fixed `tick.now` — purity says reconcile is a pure
    // function over its inputs; with the SAME tick the SAME output
    // must come out.
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = JobLifecycle::canonical();
    let (actions_a, view_a) = r.reconcile(&desired, &actual, &view, &tick);
    let (actions_b, view_b) = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions_a, actions_b,
        "reconcile() must be deterministic across two invocations with identical inputs \
         (ReconcilerIsPure invariant per ADR-0013); a non-determinism would indicate \
         an Instant::now() snuck into the spec-construction path",
    );
    assert_eq!(view_a, view_b, "next_view must also be deterministic across twin invocations");

    // Also pin that the produced action carries the expected shape on
    // both invocations — guards against a pathological case where the
    // function is deterministic but produces wrong output.
    assert_eq!(actions_a.len(), 1);
    match &actions_a[0] {
        Action::StartAllocation { spec, .. } => {
            assert_eq!(spec.command, "/opt/payments/bin/server");
            assert_eq!(spec.args, vec!["--port".to_string(), "8080".to_string()]);
        }
        other => panic!("expected StartAllocation, got {other:?}"),
    }
}
