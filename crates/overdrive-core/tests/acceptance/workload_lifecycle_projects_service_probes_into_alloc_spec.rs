//! GAP-8 close-out (Phase 01 structural audit) — `WorkloadLifecycle::reconcile`
//! projects `desired.probe_descriptors` into both `Action::StartAllocation`'s
//! and `Action::RestartAllocation`'s `AllocationSpec.probe_descriptors`.
//!
//! Pre-patch the reconciler hardcoded `probe_descriptors: Vec::new()` at
//! both action arms with a comment justifying it for Job-kind; Service-
//! kind silently inherited the empty vec even after GAP-6 (admission
//! probe persistence) and GAP-7 (per-descriptor probe-task spawn loop)
//! landed. The runtime now populates
//! `WorkloadLifecycleState.probe_descriptors` from the live intent at
//! hydrate-desired time via `project_probe_descriptors`, and the
//! reconciler clones it into both action arms.
//!
//! These ATs pin the contract from the reconciler's perspective:
//!
//! * **AT-01** — Service intent with startup probes projects into
//!   `StartAllocation.spec.probe_descriptors` byte-equal.
//! * **AT-02** — Job intent yields empty `probe_descriptors` (regression
//!   guard — Job-kind has no probe surface per ADR-0054 §3).
//! * **AT-03** — `RestartAllocation` arm projects identically to
//!   `StartAllocation`.
//! * **AT-04** — Canonical role order `startup → readiness → liveness`
//!   is preserved through `project_probe_descriptors` + the
//!   reconciler's clone into the action arm.

#![allow(clippy::expect_used)]
#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{DriverInput, ExecInput, ResourcesInput, ServiceV1};
use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver, WorkloadIntent, WorkloadKind};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::id::{AllocationId, NodeId, Region, WorkloadId};
use overdrive_core::observation::ProbeRole;
use overdrive_core::reconcilers::{
    Action, Reconciler, TickContext, WorkloadLifecycle, WorkloadLifecycleState,
    WorkloadLifecycleView, project_probe_descriptors,
};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::TransitionReason;

// -------------------------------------------------------------------
// Fixtures (mirror `workload_lifecycle_natural_exit.rs`)
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
        driver: WorkloadDriver::Exec(Exec { command: "/bin/serve".to_string(), args: vec![] }),
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

fn tcp_descriptor(role: ProbeRole, port: u16) -> ProbeDescriptor {
    ProbeDescriptor {
        role,
        mechanic: ProbeMechanic::Tcp { host: "127.0.0.1".to_string(), port },
        timeout_seconds: 1,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: if role == ProbeRole::Liveness { Some(3) } else { None },
        success_threshold: if role == ProbeRole::Readiness { Some(1) } else { None },
        inferred: false,
    }
}

/// Construct a Failed-state row for `alloc_id` — restart-budget-eligible
/// so the reconciler's Run branch emits `RestartAllocation` rather than
/// scheduling a fresh placement.
fn alloc_failed_with_budget(alloc_id: &str, workload_id: &str, node_id: &str) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(alloc_id),
        workload_id: jid(workload_id),
        node_id: nid(node_id),
        state: AllocState::Failed,
        updated_at: LogicalTimestamp { counter: 2, writer: nid(node_id) },
        reason: Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(1),
            signal: None,
            stderr_tail: None,
        }),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        // Host-netns acceptance fixture — no canonical workload address (AllocStatusRowV2 additive field, GH #241).
        workload_addr: None,
    }
}

// -------------------------------------------------------------------
// AT-01 — Service intent with startup probes projects into
//         `StartAllocation.spec.probe_descriptors`.
// -------------------------------------------------------------------

#[test]
fn at_01_service_kind_projects_startup_probes_into_start_allocation_spec() {
    let nodes = one_node_map("local");

    // Two TCP startup descriptors — the operator's declared probe set.
    let descriptors =
        vec![tcp_descriptor(ProbeRole::Startup, 8080), tcp_descriptor(ProbeRole::Startup, 9090)];

    let desired = WorkloadLifecycleState {
        workload_id: jid("svc"),
        job: Some(make_job("svc")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: descriptors.clone(),
        service_ports: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("svc"),
        job: Some(make_job("svc")),
        desired_to_stop: false,
        nodes,
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    // Run branch with no Running / no Failed → fresh schedule emits
    // `StartAllocation`. UI-06 appends one bridge `EnqueueEvaluation`.
    let start = actions
        .iter()
        .find_map(|a| match a {
            Action::StartAllocation { spec, .. } => Some(spec),
            _ => None,
        })
        .expect("Service-kind fresh schedule must emit StartAllocation");

    assert_eq!(
        spec_descriptors_len(start),
        2,
        "spec.probe_descriptors must carry both startup descriptors (got {:?})",
        start.probe_descriptors,
    );
    assert_eq!(
        start.probe_descriptors, descriptors,
        "spec.probe_descriptors must byte-equal desired.probe_descriptors",
    );
}

const fn spec_descriptors_len(spec: &overdrive_core::traits::driver::AllocationSpec) -> usize {
    spec.probe_descriptors.len()
}

// -------------------------------------------------------------------
// AT-02 — Job intent yields empty `probe_descriptors` (regression
//         guard).
// -------------------------------------------------------------------

#[test]
fn at_02_job_kind_yields_empty_probe_descriptors_in_start_allocation_spec() {
    let nodes = one_node_map("local");

    let desired = WorkloadLifecycleState {
        workload_id: jid("job"),
        job: Some(make_job("job")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Job,
        service_spec_digest: None,
        // Per ADR-0054 §3, Job-kind has no probe surface. The hydrate
        // path projects empty for Job; the regression guard asserts the
        // reconciler doesn't synthesise anything else.
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("job"),
        job: Some(make_job("job")),
        desired_to_stop: false,
        nodes,
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Job,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    let start = actions
        .iter()
        .find_map(|a| match a {
            Action::StartAllocation { spec, .. } => Some(spec),
            _ => None,
        })
        .expect("Job-kind fresh schedule must emit StartAllocation");

    assert!(
        start.probe_descriptors.is_empty(),
        "Job-kind spec.probe_descriptors MUST be empty per ADR-0054 §3; got {:?}",
        start.probe_descriptors,
    );
}

// -------------------------------------------------------------------
// AT-03 — `RestartAllocation` arm projects identically.
// -------------------------------------------------------------------

#[test]
fn at_03_restart_allocation_arm_projects_probe_descriptors_identically() {
    let nodes = one_node_map("local");

    // Three descriptors spanning the canonical roles so byte-equality
    // and ordering are both exercised at the restart arm.
    let descriptors = vec![
        tcp_descriptor(ProbeRole::Startup, 8080),
        tcp_descriptor(ProbeRole::Readiness, 8081),
        tcp_descriptor(ProbeRole::Liveness, 8082),
    ];

    // A Failed-state alloc with restart budget intact (attempts == 0)
    // routes the reconciler through the `RestartAllocation` arm.
    let mut allocations = BTreeMap::new();
    allocations.insert(aid("alloc-svc-0"), alloc_failed_with_budget("alloc-svc-0", "svc", "local"));

    let desired = WorkloadLifecycleState {
        workload_id: jid("svc"),
        job: Some(make_job("svc")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: descriptors.clone(),
        service_ports: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("svc"),
        job: Some(make_job("svc")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    // Budget remaining + no prior failure timestamp → restart fires
    // immediately on this tick.
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    let restart_spec = actions
        .iter()
        .find_map(|a| match a {
            Action::RestartAllocation { spec, .. } => Some(spec),
            _ => None,
        })
        .expect("Failed-with-budget Service must emit RestartAllocation");

    assert_eq!(
        restart_spec.probe_descriptors, descriptors,
        "RestartAllocation.spec.probe_descriptors must byte-equal desired.probe_descriptors",
    );
}

// -------------------------------------------------------------------
// AT-04 — Canonical role order `startup → readiness → liveness` is
//         preserved through `project_probe_descriptors` + the
//         reconciler's clone into the StartAllocation arm.
// -------------------------------------------------------------------

#[test]
fn at_04_canonical_role_order_startup_readiness_liveness_is_preserved() {
    let nodes = one_node_map("local");

    // Build a Service intent end-to-end via the parser-side
    // `ServiceSpecInput` → `ServiceV1::from_submit` path so the
    // projection helper is exercised against the same shape the
    // runtime hydrate path uses. Each role bucket carries a single
    // descriptor with a port that uniquely identifies the role —
    // makes order-checking trivial.
    let startup = vec![tcp_descriptor(ProbeRole::Startup, 8001)];
    let readiness = vec![tcp_descriptor(ProbeRole::Readiness, 8002)];
    let liveness = vec![tcp_descriptor(ProbeRole::Liveness, 8003)];

    let input = ServiceSpecInput {
        id: "svc".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/serve".to_string(), args: vec![] }),
        listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_string() }],
        startup_probes: startup.clone(),
        readiness_probes: readiness.clone(),
        liveness_probes: liveness.clone(),
    };
    let svc = ServiceV1::from_submit(input).expect("canonical ServiceSpecInput is valid");
    let intent = WorkloadIntent::Service(svc);

    // 1. Helper-level invariant: the projection produces the canonical
    //    concatenation `startup ++ readiness ++ liveness`.
    let projected = project_probe_descriptors(&intent);
    let expected: Vec<ProbeDescriptor> =
        startup.iter().chain(readiness.iter()).chain(liveness.iter()).cloned().collect();
    assert_eq!(
        projected, expected,
        "project_probe_descriptors must concatenate startup → readiness → liveness in canonical order",
    );

    // 2. End-to-end invariant: the reconciler's `StartAllocation` arm
    //    clones the projected vec into `spec.probe_descriptors` in the
    //    SAME canonical order.
    let desired = WorkloadLifecycleState {
        workload_id: jid("svc"),
        job: Some(make_job("svc")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: projected,
        service_ports: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: jid("svc"),
        job: Some(make_job("svc")),
        desired_to_stop: false,
        nodes,
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = fresh_tick(Instant::now(), UnixInstant::from_unix_duration(Duration::from_secs(0)));

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    let start = actions
        .iter()
        .find_map(|a| match a {
            Action::StartAllocation { spec, .. } => Some(spec),
            _ => None,
        })
        .expect("Service-kind fresh schedule must emit StartAllocation");

    assert_eq!(
        start.probe_descriptors, expected,
        "StartAllocation.spec.probe_descriptors must preserve startup → readiness → liveness order; got {:?}",
        start.probe_descriptors,
    );
}
