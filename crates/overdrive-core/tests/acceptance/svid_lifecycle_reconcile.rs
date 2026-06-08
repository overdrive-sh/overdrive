//! Acceptance tests for `SvidLifecycle::reconcile` per ADR-0067 D1/D2
//! (workload-identity-manager Slice 01, step 01-04).
//!
//! Layer 1: pure reconciler scenarios. The reconciler converges
//! `desired = running allocs` against `actual = the IdentityMgr held set`
//! and emits the diff (`running ∧ ¬held → IssueSvid`,
//! `¬running ∧ held → DropSvid`). `reconcile` is a pure function — no
//! `.await`, no CA / `ObservationStore` handle, wall-clock only via
//! `tick.now` — so these tests construct the typed `State` directly and
//! assert on the emitted action list (the observable universe) without any
//! infrastructure double.
//!
//! Lives in `tests/acceptance/` rather than `src/` because dst-lint scans
//! `src/**/*.rs` and bans `Instant::now()` there even under `#[cfg(test)]`;
//! the `TickContext.now` snapshot the reconcile signature requires forces
//! the `Instant` read into the test fixture here.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver, WorkloadKind};
use overdrive_core::id::{
    AllocationId, ContentHash, CorrelationKey, NodeId, Region, SpiffeId, WorkloadId,
};
use overdrive_core::reconcilers::svid_lifecycle::{
    RunningAlloc, SvidLifecycle, SvidLifecycleState,
};
use overdrive_core::reconcilers::{
    Action, HeldSvidFacts, RESTART_BACKOFF_CEILING, Reconciler, TargetResource, TickContext,
    WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView,
};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::TerminalCondition;
use overdrive_core::wall_clock::UnixInstant;
use proptest::prelude::*;

const NODE_RAW: &str = "local";

/// Canonical kebab-case name of the `SvidLifecycle` reconciler — sourced
/// from the trait const so a rename is a compile error here, not a silent
/// assertion drift (mirrors the anti-drift const the producer uses).
const SVID_LIFECYCLE_NAME: &str = <SvidLifecycle as Reconciler>::NAME;

/// RED scaffold marker for the Slice-03 / Slice-01-enqueue scenarios
/// (S-WIM-08 retry-memory View, S-WIM-09 emit-gated rotation seam, S-WIM-10
/// lifecycle enqueue) that land in later steps (03-01 / 03-02 / 01-05), not
/// 01-04. Kept as `#[should_panic(expected = "RED scaffold")]` per
/// `.claude/rules/testing.md`.
fn red_scaffold(scenario: &str) -> ! {
    panic!("RED scaffold: workload-identity-manager {scenario}");
}

fn make_tick(now_secs: u64) -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(now_secs)),
        tick: now_secs,
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

fn workload() -> WorkloadId {
    WorkloadId::new("payments").expect("valid WorkloadId")
}

fn node() -> NodeId {
    NodeId::new(NODE_RAW).expect("valid NodeId")
}

fn running_alloc() -> RunningAlloc {
    RunningAlloc { workload_id: workload(), node_id: node() }
}

fn held_facts(alloc: &AllocationId, not_after_secs: u64) -> HeldSvidFacts {
    HeldSvidFacts {
        spiffe_id: SpiffeId::for_allocation(&workload(), alloc),
        not_after: UnixInstant::from_unix_duration(Duration::from_secs(not_after_secs)),
    }
}

/// The deterministic `IssueSvid` correlation the reconciler must derive for a
/// given allocation (ADR-0067 D2): `target = "svid-lifecycle/<alloc>"`,
/// `spec_hash = ContentHash::of(<spiffe-uri bytes>)`, `purpose = "issue-svid"`.
fn expected_issue_correlation(alloc: &AllocationId, spiffe: &SpiffeId) -> CorrelationKey {
    let target = format!("svid-lifecycle/{alloc}");
    let spec_hash = ContentHash::of(spiffe.as_str().as_bytes());
    CorrelationKey::derive(&target, &spec_hash, "issue-svid")
}

/// The deterministic `DropSvid` correlation (purpose `"drop-svid"`); the
/// dropped allocation's `spec_hash` is derived from the held identity.
fn expected_drop_correlation(alloc: &AllocationId, spiffe: &SpiffeId) -> CorrelationKey {
    let target = format!("svid-lifecycle/{alloc}");
    let spec_hash = ContentHash::of(spiffe.as_str().as_bytes());
    CorrelationKey::derive(&target, &spec_hash, "drop-svid")
}

// `@in-memory` `@property` `@S-WIM-01` — for an arbitrary running-set / held-set
// population, EVERY allocation that is Running but NOT held yields exactly one
// `Action::IssueSvid` carrying the four pinned fields (`alloc_id`, the
// `SpiffeId::for_allocation` identity, `node_id`, the derived correlation), and
// the reconcile body performs no CA / ObservationStore I/O (it is a pure
// function over its arguments — there is no handle to call). The observable
// universe is the emitted action list + the returned `next_view`.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn running_alloc_without_held_svid_emits_issue_svid(
        running_suffixes in proptest::collection::btree_set("[a-f0-9]{4,8}", 1..6),
        held_extra_suffixes in proptest::collection::btree_set("[a-f0-9]{4,8}", 0..4),
    ) {
        let reconciler = SvidLifecycle::canonical();

        // desired = the running set.
        let mut desired: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
        for suffix in &running_suffixes {
            let alloc = AllocationId::new(&format!("payments-{suffix}")).expect("valid AllocationId");
            desired.insert(alloc.clone(), running_alloc());
        }

        // actual = the held set: hold SVIDs for a DISJOINT extra set only, so
        // every running alloc is `running ∧ ¬held`. (Suffixes that collide with
        // the running set are filtered out to keep the property's precondition
        // — "running without held" — true for every running alloc.)
        let mut actual: BTreeMap<AllocationId, HeldSvidFacts> = BTreeMap::new();
        for suffix in held_extra_suffixes.iter().filter(|s| !running_suffixes.contains(*s)) {
            let alloc = AllocationId::new(&format!("payments-{suffix}")).expect("valid AllocationId");
            actual.insert(alloc.clone(), held_facts(&alloc, 1_700_000_000));
        }

        let state = SvidLifecycleState { desired: desired.clone(), actual: actual.clone() };
        let view = <SvidLifecycle as Reconciler>::View::default();
        let tick = make_tick(0);

        let (actions, next_view) = reconciler.reconcile(&state, &state, &view, &tick);

        // Universe slot 1: the emitted IssueSvid set.
        let issues: BTreeMap<AllocationId, Action> = actions
            .iter()
            .filter(|a| matches!(a, Action::IssueSvid { .. }))
            .map(|a| match a {
                Action::IssueSvid { alloc_id, .. } => (alloc_id.clone(), a.clone()),
                _ => unreachable!(),
            })
            .collect();

        // Exactly one IssueSvid per running-without-held alloc — no more, no less.
        prop_assert_eq!(
            issues.len(),
            running_suffixes.len(),
            "one IssueSvid per running ∧ ¬held alloc"
        );

        for alloc in desired.keys() {
            let expected_spiffe = SpiffeId::for_allocation(&workload(), alloc);
            let action = issues.get(alloc).expect("running ∧ ¬held alloc emits an IssueSvid");
            match action {
                Action::IssueSvid { alloc_id, spiffe_id, node_id, correlation } => {
                    prop_assert_eq!(alloc_id, alloc, "IssueSvid names the running alloc");
                    prop_assert_eq!(
                        spiffe_id,
                        &expected_spiffe,
                        "IssueSvid carries SpiffeId::for_allocation"
                    );
                    prop_assert_eq!(node_id, &node(), "IssueSvid carries the issuing node");
                    prop_assert_eq!(
                        correlation,
                        &expected_issue_correlation(alloc, &expected_spiffe),
                        "IssueSvid correlation derives from (svid-lifecycle/<alloc>, spec_hash, issue-svid)"
                    );
                }
                _ => unreachable!(),
            }
        }

        // No held alloc that is also running was dropped, and no extra-held
        // alloc was issued (the extra held set is disjoint from running, so it
        // is `¬running ∧ held → DropSvid`, never IssueSvid).
        for alloc in actual.keys() {
            prop_assert!(
                !issues.contains_key(alloc),
                "a held (not-running) alloc is never issued"
            );
        }

        // Universe slot 2: the next_view is the minimal Slice-01 view
        // (retry memory lands in 03-01) — unchanged default.
        prop_assert_eq!(
            next_view,
            <SvidLifecycle as Reconciler>::View::default(),
            "Slice-01 View is minimal: reconcile returns the default unchanged"
        );
    }
}

/// `@in-memory` `@S-WIM-03` — a stopped (no-longer-Running) allocation that still
/// has a held SVID yields exactly one `Action::DropSvid` for it (so the leaf key
/// becomes unreachable, ADR-0067 O2), and emits NO `IssueSvid` for the stopped
/// alloc. The still-running held alloc converges to a no-op.
#[test]
fn stopped_alloc_with_held_svid_emits_drop_svid() {
    let reconciler = SvidLifecycle::canonical();

    let stopped = AllocationId::new("payments-dead").expect("valid AllocationId");
    let still_running = AllocationId::new("payments-live").expect("valid AllocationId");

    // desired = only the still-running alloc.
    let mut desired: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
    desired.insert(still_running.clone(), running_alloc());

    // actual = both allocs are held (the stopped one is a leftover hold).
    let mut actual: BTreeMap<AllocationId, HeldSvidFacts> = BTreeMap::new();
    actual.insert(stopped.clone(), held_facts(&stopped, 1_700_000_000));
    actual.insert(still_running.clone(), held_facts(&still_running, 1_700_000_000));

    let state = SvidLifecycleState { desired, actual };
    let view = <SvidLifecycle as Reconciler>::View::default();
    let tick = make_tick(0);

    let (actions, _next_view) = reconciler.reconcile(&state, &state, &view, &tick);

    // Exactly one DropSvid, and it targets the stopped alloc.
    let drops: Vec<&Action> =
        actions.iter().filter(|a| matches!(a, Action::DropSvid { .. })).collect();
    assert_eq!(drops.len(), 1, "exactly one DropSvid for the ¬running ∧ held alloc");

    let stopped_spiffe = SpiffeId::for_allocation(&workload(), &stopped);
    match drops[0] {
        Action::DropSvid { alloc_id, correlation } => {
            assert_eq!(alloc_id, &stopped, "DropSvid names the stopped alloc");
            assert_eq!(
                correlation,
                &expected_drop_correlation(&stopped, &stopped_spiffe),
                "DropSvid correlation derives from (svid-lifecycle/<alloc>, spec_hash, drop-svid)"
            );
        }
        _ => unreachable!(),
    }

    // No IssueSvid for the stopped alloc, and the still-running held alloc is a
    // no-op (neither issued nor dropped).
    for action in &actions {
        match action {
            Action::IssueSvid { alloc_id, .. } => {
                panic!(
                    "no IssueSvid expected (stopped is dropped, live is already held): {alloc_id}"
                )
            }
            Action::DropSvid { alloc_id, .. } => {
                assert_ne!(
                    alloc_id, &still_running,
                    "the still-running held alloc is never dropped"
                );
            }
            _ => {}
        }
    }
}

/// `@in-memory` `@property` `@S-WIM-08` -- the View is retry memory only:
/// `IssueRetry { attempts, last_failure_seen_at }`, with no serial,
/// `issued_at`, `spiffe_id`, `expires_at`, or `next_renewal_at` success fact.
#[test]
#[should_panic(expected = "RED scaffold")]
fn svid_lifecycle_view_is_retry_memory_only() {
    red_scaffold("S-WIM-08 View is retry memory only");
}

/// `@in-memory` `@error` `@S-WIM-09` -- the #40 near-expiry branch is
/// structurally present but emit-gated until `cert_rotation` is registered,
/// so #35 never emits `UnknownWorkflow` every tick.
#[test]
#[should_panic(expected = "RED scaffold")]
fn near_expiry_rotation_seam_is_emit_gated_until_cert_rotation_registered() {
    red_scaffold("S-WIM-09 rotation seam is emit-gated");
}

// ---------------------------------------------------------------------------
// S-WIM-10 — `WorkloadLifecycle::reconcile` enqueues `SvidLifecycle` (ADR-0067
// D5b, producer 1). The pure reconciler is its own driving port (calling
// `reconcile` directly IS port-to-port at the domain layer); the observable
// universe is the emitted action list. These fixtures mirror the UI-06 / GAP-9
// shape in `workload_lifecycle_enqueues_bridge_on_alloc_transitions.rs`.
// ---------------------------------------------------------------------------

fn wl_workload(s: &str) -> WorkloadId {
    WorkloadId::new(s).expect("valid WorkloadId")
}
fn wl_node_id(s: &str) -> NodeId {
    NodeId::new(s).expect("valid NodeId")
}
fn wl_alloc(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}
fn wl_region() -> Region {
    Region::new("local").expect("valid Region")
}
fn wl_make_node(id: &str) -> Node {
    Node {
        id: wl_node_id(id),
        region: wl_region(),
        capacity: Resources { cpu_milli: 4_000, memory_bytes: 8 * 1024 * 1024 * 1024 },
    }
}
fn wl_make_job(id: &str) -> Job {
    Job {
        id: wl_workload(id),
        replicas: NonZeroU32::new(1).expect("1 is non-zero"),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: WorkloadDriver::Exec(Exec { command: "/bin/true".to_string(), args: vec![] }),
    }
}
fn wl_one_node_map(node_id: &str) -> BTreeMap<NodeId, Node> {
    let n = wl_make_node(node_id);
    let mut m = BTreeMap::new();
    m.insert(n.id.clone(), n);
    m
}
fn wl_alloc_running(
    alloc_id: &str,
    workload_id: &str,
    node_id: &str,
    state: AllocState,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: wl_alloc(alloc_id),
        workload_id: wl_workload(workload_id),
        node_id: wl_node_id(node_id),
        state,
        updated_at: LogicalTimestamp { counter: 1, writer: wl_node_id(node_id) },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: match state {
            AllocState::Pending => None,
            _ => Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        },
    }
}
fn wl_tick() -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    }
}

/// Assert `actions` contains exactly one `Action::EnqueueEvaluation` routed at
/// `svid-lifecycle` for `job/<workload_id>`. The SvidLifecycle enqueue is
/// UNGATED by workload kind — identity is needed by every running alloc.
fn assert_single_svid_enqueue(actions: &[Action], workload_id: &WorkloadId) {
    let mut count = 0;
    let mut found_target: Option<&TargetResource> = None;
    for action in actions {
        if let Action::EnqueueEvaluation { reconciler, target } = action
            && reconciler.as_str() == SVID_LIFECYCLE_NAME
        {
            count += 1;
            found_target = Some(target);
        }
    }
    assert_eq!(
        count, 1,
        "S-WIM-10 (ADR-0067 D5b): an alloc-mutating tick MUST emit exactly one \
         EnqueueEvaluation routed at 'svid-lifecycle'; got {count} in {actions:?}",
    );
    let target = found_target.expect("count==1 checked above");
    assert_eq!(
        target.as_str(),
        &format!("job/{workload_id}"),
        "S-WIM-10: svid-lifecycle enqueue target MUST be 'job/<workload_id>' (workload grain)"
    );
}

/// `@in-memory` `@S-WIM-10` -- `WorkloadLifecycle::reconcile` enqueues
/// `SvidLifecycle` evaluation for `job/<workload_id>` on a RUNNING-transition
/// class (`StartAllocation`). ADR-0067 D5b producer 1. Without this, the pure
/// reconciler can be correct but unreachable.
#[test]
fn start_allocation_transition_enqueues_svid_lifecycle() {
    let workload_id = wl_workload("payments");
    let nodes = wl_one_node_map("local");
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = wl_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert!(
        actions.iter().any(|a| matches!(a, Action::StartAllocation { .. })),
        "expected StartAllocation (Running transition); got {actions:?}"
    );
    assert_single_svid_enqueue(&actions, &workload_id);
}

/// `@in-memory` `@S-WIM-10` -- a STOPPED-transition class (`StopAllocation`)
/// also enqueues `SvidLifecycle` for `job/<workload_id>` so the
/// `¬running ∧ held → DropSvid` branch fires (ADR-0067 O2). Producer 1.
#[test]
fn stop_allocation_transition_enqueues_svid_lifecycle() {
    let workload_id = wl_workload("payments");
    let nodes = wl_one_node_map("local");
    let mut allocations = BTreeMap::new();
    allocations.insert(
        wl_alloc("alloc-payments-0"),
        wl_alloc_running("alloc-payments-0", "payments", "local", AllocState::Running),
    );
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("payments")),
        desired_to_stop: true,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = wl_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert!(
        actions.iter().any(|a| matches!(a, Action::StopAllocation { .. })),
        "expected StopAllocation (Stopped transition); got {actions:?}"
    );
    assert_single_svid_enqueue(&actions, &workload_id);
}

/// `@in-memory` `@S-WIM-10` -- a `FinalizeFailed` (backoff-exhausted) terminal
/// transition also enqueues `SvidLifecycle`, completing the Stopped-transition
/// class coverage. Producer 1.
#[test]
fn finalize_failed_transition_enqueues_svid_lifecycle() {
    let workload_id = wl_workload("payments");
    let nodes = wl_one_node_map("local");
    let mut allocations = BTreeMap::new();
    allocations.insert(
        wl_alloc("alloc-payments-0"),
        wl_alloc_running("alloc-payments-0", "payments", "local", AllocState::Failed),
    );
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let mut view = WorkloadLifecycleView::default();
    view.restart_counts.insert(wl_alloc("alloc-payments-0"), RESTART_BACKOFF_CEILING);
    let tick = wl_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert!(
        actions.iter().any(|a| matches!(
            a,
            Action::FinalizeFailed {
                terminal: Some(TerminalCondition::BackoffExhausted { .. }),
                ..
            }
        )),
        "expected FinalizeFailed (Stopped transition); got {actions:?}"
    );
    assert_single_svid_enqueue(&actions, &workload_id);
}

/// `@in-memory` `@S-WIM-10` -- the SvidLifecycle enqueue is UNGATED by workload
/// kind: a Job-kind StartAllocation enqueues svid-lifecycle just like Service
/// (unlike the Service-gated service-lifecycle enqueue). Identity is needed by
/// every running alloc. ADR-0067 D5b "ungated by kind".
#[test]
fn job_kind_transition_still_enqueues_svid_lifecycle() {
    let workload_id = wl_workload("batch");
    let nodes = wl_one_node_map("local");
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("batch")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Job,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("batch")),
        desired_to_stop: false,
        nodes,
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Job,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = wl_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert!(
        actions.iter().any(|a| matches!(a, Action::StartAllocation { .. })),
        "expected StartAllocation; got {actions:?}"
    );
    assert_single_svid_enqueue(&actions, &workload_id);
}

/// `@in-memory` `@S-WIM-10` -- a converged (no-op) tick emits ZERO
/// svid-lifecycle enqueues: the enqueue is paired ONLY with alloc-mutating
/// actions, so the broker is never churned on a quiet tick.
#[test]
fn converged_tick_emits_no_svid_enqueue() {
    let workload_id = wl_workload("payments");
    let nodes = wl_one_node_map("local");
    let mut allocations = BTreeMap::new();
    allocations.insert(
        wl_alloc("alloc-payments-0"),
        wl_alloc_running("alloc-payments-0", "payments", "local", AllocState::Running),
    );
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id,
        job: Some(wl_make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = wl_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    let svid_enqueues = actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                Action::EnqueueEvaluation { reconciler, .. }
                    if reconciler.as_str() == SVID_LIFECYCLE_NAME
            )
        })
        .count();
    assert_eq!(
        svid_enqueues, 0,
        "converged tick must emit ZERO svid-lifecycle enqueues; got {svid_enqueues} in {actions:?}"
    );
}
