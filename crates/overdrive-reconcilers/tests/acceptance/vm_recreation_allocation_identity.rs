//! GH #284 / ADR-0104 acceptance properties at the existing
//! `WorkloadLifecycle::reconcile` and `Reconciler::next_evaluation_at`
//! driving ports.
//!
//! The stable owner is one `WorkloadId`. Retained `AllocStatusRow`s are
//! accepted physical-attempt history; VM keys in
//! `WorkloadLifecycleView::restart_counts` are issued-ID reservations even
//! when no row was accepted. The tests observe only returned Actions, the
//! returned View, and the immutable input rows. No private helper or new
//! production seam is exposed for testing.

#![allow(clippy::doc_markdown, clippy::expect_used, clippy::missing_panics_doc)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::SpiffeId;
use overdrive_core::aggregate::{Exec, Job, Node, Vm, WorkloadDriver, WorkloadKind};
use overdrive_core::id::{AllocationId, NodeId, Region, WorkloadId};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::driver::{DriverPayload, Resources};
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::{
    WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView, backoff_for_attempt,
};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

fn wid(raw: &str) -> WorkloadId {
    WorkloadId::new(raw).expect("valid workload id")
}

fn aid(raw: &str) -> AllocationId {
    AllocationId::new(raw).expect("valid allocation id")
}

fn nid(raw: &str) -> NodeId {
    NodeId::new(raw).expect("valid node id")
}

const fn instant(seconds: u64) -> UnixInstant {
    UnixInstant::from_unix_duration(Duration::from_secs(seconds))
}

fn tick(seconds: u64) -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: instant(seconds),
        tick: seconds,
        deadline: now + Duration::from_secs(1),
    }
}

fn node() -> Node {
    Node {
        id: nid("local"),
        region: Region::new("local").expect("valid region"),
        capacity: Resources { cpu_milli: 4_000, memory_bytes: 8 * 1024 * 1024 * 1024 },
    }
}

fn job(workload: &str, driver: WorkloadDriver) -> Job {
    Job {
        id: wid(workload),
        replicas: NonZeroU32::new(1).expect("one replica"),
        resources: Resources { cpu_milli: 500, memory_bytes: 128 * 1024 * 1024 },
        driver,
    }
}

fn vm_job(workload: &str) -> Job {
    job(
        workload,
        WorkloadDriver::Vm(Vm {
            command: "/sbin/workload".to_owned(),
            args: vec!["--serve".to_owned()],
            kernel: "/srv/vm/kernel".to_owned(),
            rootfs: "/srv/vm/rootfs.ext4".to_owned(),
        }),
    )
}

fn exec_job(workload: &str) -> Job {
    job(
        workload,
        WorkloadDriver::Exec(Exec {
            command: "/bin/workload".to_owned(),
            args: vec!["--serve".to_owned()],
        }),
    )
}

fn failed_row(workload: &str, suffix: u32) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(&format!("alloc-{workload}-{suffix}")),
        workload_id: wid(workload),
        node_id: nid("local"),
        state: AllocState::Failed,
        updated_at: LogicalTimestamp { counter: u64::from(suffix) + 1, writer: nid("local") },
        reason: Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(23),
            signal: None,
            stderr_tail: Some("guest workload failed".to_owned()),
        }),
        detail: Some("accepted predecessor failure".to_owned()),
        terminal: None,
        stderr_tail: Some("guest workload failed".to_owned()),
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(instant(10)),
        workload_addr: None,
        last_terminated: None,
        restart_count: 7,
    }
}

fn reclaimed_row(workload: &str, suffix: u32) -> AllocStatusRow {
    let mut row = failed_row(workload, suffix);
    row.state = AllocState::Terminated;
    row.reason = Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed });
    row.detail = Some("platform reclaimed the physical VM".to_owned());
    row.stderr_tail = None;
    row
}

fn operator_stopped_row(workload: &str, suffix: u32) -> AllocStatusRow {
    let mut row = failed_row(workload, suffix);
    row.state = AllocState::Terminated;
    row.reason = Some(TransitionReason::Stopped { by: StoppedBy::Operator });
    row.terminal = Some(TerminalCondition::Stopped { by: StoppedBy::Operator });
    row.detail = None;
    row.stderr_tail = None;
    row
}

fn states(
    job: Job,
    rows: impl IntoIterator<Item = AllocStatusRow>,
) -> (WorkloadLifecycleState, WorkloadLifecycleState) {
    states_for_kind(job, rows, WorkloadKind::Service)
}

fn states_for_kind(
    job: Job,
    rows: impl IntoIterator<Item = AllocStatusRow>,
    workload_kind: WorkloadKind,
) -> (WorkloadLifecycleState, WorkloadLifecycleState) {
    let workload_id = job.id.clone();
    let nodes = BTreeMap::from([(nid("local"), node())]);
    let allocations =
        rows.into_iter().map(|row| (row.alloc_id.clone(), row)).collect::<BTreeMap<_, _>>();
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(job.clone()),
        desired_to_stop: false,
        generation: 0,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id,
        job: Some(job),
        desired_to_stop: false,
        generation: 0,
        nodes,
        allocations,
        workload_kind,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    (desired, actual)
}

fn reconcile(
    desired: &WorkloadLifecycleState,
    actual: &WorkloadLifecycleState,
    view: &WorkloadLifecycleView,
    seconds: u64,
) -> (Vec<Action>, WorkloadLifecycleView) {
    WorkloadLifecycle::canonical().reconcile(desired, actual, view, &tick(seconds))
}

fn only_start(
    actions: &[Action],
) -> (&AllocationId, &WorkloadId, &overdrive_core::traits::driver::AllocationSpec) {
    let mut starts = actions.iter().filter_map(|action| match action {
        Action::StartAllocation { alloc_id, workload_id, spec, .. } => {
            Some((alloc_id, workload_id, spec))
        }
        _ => None,
    });
    let start = starts.next().expect("one fresh VM StartAllocation");
    assert!(starts.next().is_none(), "exactly one VM StartAllocation; got {actions:?}");
    start
}

/// S-284-PURE-01 — a genuine VM Workload Failure consumes one owner-level
/// retry attempt, reserves a fresh physical ID, and leaves the accepted
/// predecessor row immutable.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn vm_failure_reserves_fresh_execution_and_preserves_predecessor() {
    let predecessor = failed_row("payments", 0);
    let (desired, actual) = states(vm_job("payments"), [predecessor.clone()]);
    let mut view = WorkloadLifecycleView::default();
    view.restart_counts.insert(predecessor.alloc_id.clone(), 2);
    view.last_failure_seen_at.insert(predecessor.alloc_id.clone(), instant(10));
    let before_actual = actual.clone();

    let (actions, next) = reconcile(&desired, &actual, &view, 20);

    let (fresh, owner, spec) = only_start(&actions);
    assert_eq!(fresh.as_str(), "alloc-payments-1");
    assert_eq!(owner, &wid("payments"));
    assert_eq!(&spec.alloc, fresh);
    assert_eq!(spec.identity, SpiffeId::for_allocation(owner, fresh));
    assert!(matches!(&spec.driver, DriverPayload::Vm(_)));
    assert!(actions.iter().all(|action| !matches!(action, Action::RestartAllocation { .. })));
    assert_eq!(next.restart_counts.get(&predecessor.alloc_id), Some(&3));
    assert_eq!(next.restart_counts.get(fresh), Some(&3));
    assert_eq!(next.last_failure_seen_at.get(&predecessor.alloc_id), Some(&instant(20)));
    assert_eq!(next.last_failure_seen_at.get(fresh), Some(&instant(20)));
    assert_eq!(actual, before_actual, "the predecessor/history input is immutable");
}

/// S-284-PURE-02 — Platform Reclamation reserves a fresh VM execution ID
/// without consuming Workload Failure budget or inventing a failure time.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn vm_reclamation_carries_policy_without_charging_failure_budget() {
    let mut predecessor = reclaimed_row("render", 4);
    predecessor.kind = WorkloadKind::Job;
    let (desired, actual) =
        states_for_kind(vm_job("render"), [predecessor.clone()], WorkloadKind::Job);
    let mut view = WorkloadLifecycleView::default();
    view.restart_counts.insert(predecessor.alloc_id.clone(), 4);
    view.last_failure_seen_at.insert(predecessor.alloc_id.clone(), instant(41));

    let (actions, next) = reconcile(&desired, &actual, &view, 80);

    let (fresh, _, spec) = only_start(&actions);
    assert_eq!(fresh.as_str(), "alloc-render-5");
    assert_eq!(&spec.alloc, fresh);
    assert_eq!(next.restart_counts.get(&predecessor.alloc_id), Some(&4));
    assert_eq!(next.restart_counts.get(fresh), Some(&4));
    assert_eq!(next.last_failure_seen_at.get(&predecessor.alloc_id), Some(&instant(41)));
    assert_eq!(next.last_failure_seen_at.get(fresh), Some(&instant(41)));
}

/// S-284-PURE-03 — an unpublished but fsynced reservation is not current,
/// yet its physical ID is consumed and the next VM decision skips it.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn unpublished_reservation_is_skipped_without_becoming_current() {
    let predecessor = failed_row("search", 0);
    let rejected = aid("alloc-search-1");
    let (desired, actual) = states(vm_job("search"), [predecessor.clone()]);
    let mut view = WorkloadLifecycleView::default();
    view.restart_counts.insert(predecessor.alloc_id.clone(), 1);
    view.restart_counts.insert(rejected.clone(), 1);
    view.last_failure_seen_at.insert(predecessor.alloc_id.clone(), instant(10));

    let (actions, next) = reconcile(&desired, &actual, &view, 20);

    let (fresh, _, _) = only_start(&actions);
    assert_eq!(fresh.as_str(), "alloc-search-2");
    assert!(
        !actual.allocations.contains_key(&rejected),
        "reservation alone is not an accepted row"
    );
    assert_eq!(next.restart_counts.get(&rejected), Some(&1));
    assert_eq!(next.restart_counts.get(&predecessor.alloc_id), Some(&2));
    assert_eq!(next.restart_counts.get(fresh), Some(&2));
}

/// S-284-PURE-04 — retry/backoff reads the numerically-current accepted VM
/// candidate only; unrelated historical values cannot advance or delay it.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn retry_deadline_is_derived_only_from_current_accepted_candidate() {
    // Lexically `...-2` sorts after `...-10`; numerically suffix 10 is
    // current. Distinct policy values make a lexical-last mutation observable.
    let historical = failed_row("catalog", 2);
    let current = failed_row("catalog", 10);
    let historical_id = historical.alloc_id.clone();
    let current_id = current.alloc_id.clone();
    let (desired, actual) = states(vm_job("catalog"), [historical, current]);
    let mut view = WorkloadLifecycleView::default();
    view.restart_counts.insert(historical_id.clone(), 4);
    view.last_failure_seen_at.insert(historical_id, instant(100));
    view.restart_counts.insert(current_id.clone(), 2);
    view.last_failure_seen_at.insert(current_id, instant(200));
    let at = tick(200);

    let deadline = WorkloadLifecycle::canonical().next_evaluation_at(&desired, &actual, &view, &at);

    assert_eq!(
        deadline,
        Some(instant(200) + backoff_for_attempt(2)),
        "the historical suffix-2 deadline is unrelated; numeric suffix-10 is current",
    );
}

/// S-284-PURE-05 — a historical restartable predecessor can never be
/// re-driven or donate its retry values after a newer accepted row exists.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn numeric_current_row_owns_replacement_and_history_stays_isolated() {
    // Lexical ordering chooses suffix 2; numeric ordering must choose 10.
    let historical = failed_row("ledger", 2);
    let current = failed_row("ledger", 10);
    let (desired, actual) = states(vm_job("ledger"), [historical.clone(), current.clone()]);
    let before_actual = actual.clone();
    let mut view = WorkloadLifecycleView::default();
    view.restart_counts.insert(historical.alloc_id.clone(), 4);
    view.last_failure_seen_at.insert(historical.alloc_id.clone(), instant(1));
    view.restart_counts.insert(current.alloc_id.clone(), 1);
    view.last_failure_seen_at.insert(current.alloc_id.clone(), instant(10));

    let (actions, next) = reconcile(&desired, &actual, &view, 20);

    let (fresh, _, _) = only_start(&actions);
    assert_eq!(fresh.as_str(), "alloc-ledger-11");
    assert_eq!(next.restart_counts.get(&historical.alloc_id), Some(&4));
    assert_eq!(next.last_failure_seen_at.get(&historical.alloc_id), Some(&instant(1)));
    assert_eq!(next.restart_counts.get(&current.alloc_id), Some(&2));
    assert_eq!(next.restart_counts.get(fresh), Some(&2));
    assert_eq!(actual, before_actual, "accepted predecessor/history rows are immutable");
}

/// S-284-PURE-06 — VM and Exec keep intentionally different recreation
/// contracts: VM creates a fresh allocation; Exec restarts the same ID.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn vm_uses_fresh_start_while_exec_keeps_same_id_restart() {
    let vm_predecessor = failed_row("vm-control", 0);
    let (vm_desired, vm_actual) = states(vm_job("vm-control"), [vm_predecessor]);
    let (vm_actions, _) = reconcile(&vm_desired, &vm_actual, &WorkloadLifecycleView::default(), 20);
    let (vm_fresh, _, vm_spec) = only_start(&vm_actions);
    assert_eq!(vm_fresh.as_str(), "alloc-vm-control-1");
    assert_eq!(&vm_spec.alloc, vm_fresh);

    let exec_predecessor = failed_row("exec-control", 0);
    let exec_id = exec_predecessor.alloc_id.clone();
    let (exec_desired, exec_actual) = states(exec_job("exec-control"), [exec_predecessor]);
    let (exec_actions, exec_view) =
        reconcile(&exec_desired, &exec_actual, &WorkloadLifecycleView::default(), 20);
    assert!(exec_actions.iter().any(|action| matches!(
        action,
        Action::RestartAllocation { alloc_id, spec, .. }
            if alloc_id == &exec_id
                && spec.alloc == exec_id
                && matches!(&spec.driver, DriverPayload::Exec(_))
    )));
    assert!(exec_actions.iter().all(|action| !matches!(action, Action::StartAllocation { .. })));
    assert_eq!(exec_view.restart_counts.get(&exec_id), Some(&1));
}

/// S-284-PURE-07 — the checked `u32` attempt domain stops safely at
/// exhaustion; it never clamps to or reuses the maximum ID.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn exhausted_vm_attempt_domain_emits_no_execution_action() {
    let exhausted = failed_row("exhausted", u32::MAX);
    let (desired, actual) = states(vm_job("exhausted"), [exhausted]);
    let view = WorkloadLifecycleView::default();

    let (actions, next) = reconcile(&desired, &actual, &view, 20);

    assert!(actions.iter().all(|action| !matches!(
        action,
        Action::StartAllocation { .. } | Action::RestartAllocation { .. }
    )));
    assert_eq!(next, view);
}

/// S-284-PURE-08 — the final representable successor (`u32::MAX - 1` to
/// `u32::MAX`) is issued once; the separate exhaustion scenario proves the
/// following decision stops rather than wraps.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn vm_max_minus_one_issues_the_final_attempt_identity() {
    let predecessor = failed_row("last-attempt", u32::MAX - 1);
    let (desired, actual) = states(vm_job("last-attempt"), [predecessor]);

    let (actions, next) = reconcile(&desired, &actual, &WorkloadLifecycleView::default(), 20);

    let (fresh, _, _) = only_start(&actions);
    assert_eq!(fresh.as_str(), "alloc-last-attempt-4294967295");
    assert!(next.restart_counts.contains_key(fresh));
}

/// S-284-PURE-09 — a retained legacy/non-minted allocation ID has no numeric
/// suffix. It remains the accepted failure candidate, while VM identity
/// selection begins the canonical minted domain at suffix zero.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn malformed_attempt_suffix_is_not_silently_reused() {
    let mut predecessor = failed_row("legacy", 0);
    predecessor.alloc_id = aid("legacy-vm-allocation");
    let (desired, actual) = states(vm_job("legacy"), [predecessor.clone()]);

    let (actions, next) = reconcile(&desired, &actual, &WorkloadLifecycleView::default(), 20);

    let (fresh, _, _) = only_start(&actions);
    assert_eq!(fresh.as_str(), "alloc-legacy-0");
    assert_ne!(fresh, &predecessor.alloc_id);
    assert!(next.restart_counts.contains_key(fresh));
}

/// S-284-PURE-10 — initial VM placement participates in the same
/// fsync-before-dispatch issued-ID rule: suffix zero is present in the returned
/// View even though no accepted allocation row exists yet.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn initial_vm_placement_reserves_zero_before_dispatch() {
    let (desired, actual) = states(vm_job("initial-vm"), []);

    let (actions, next) = reconcile(&desired, &actual, &WorkloadLifecycleView::default(), 20);

    let (fresh, _, spec) = only_start(&actions);
    assert_eq!(fresh.as_str(), "alloc-initial-vm-0");
    assert_eq!(&spec.alloc, fresh);
    assert_eq!(next.restart_counts.get(fresh), Some(&0));
    assert!(!next.last_failure_seen_at.contains_key(fresh));
}

/// S-284-PURE-11 — explicit generation replacement uses the existing fresh
/// placement path and carries current candidate policy into its reservation
/// without manufacturing another Workload Failure.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn generation_replacement_reserves_fresh_id_without_failure_increment() {
    let predecessor = operator_stopped_row("generation-vm", 0);
    let (mut desired, mut actual) = states(vm_job("generation-vm"), [predecessor.clone()]);
    desired.generation = 1;
    actual.generation = 1;
    let mut view = WorkloadLifecycleView { observed_generation: 0, ..Default::default() };
    view.restart_counts.insert(predecessor.alloc_id.clone(), 2);
    view.last_failure_seen_at.insert(predecessor.alloc_id.clone(), instant(10));

    let (actions, next) = reconcile(&desired, &actual, &view, 20);

    let (fresh, _, _) = only_start(&actions);
    assert_eq!(fresh.as_str(), "alloc-generation-vm-1");
    assert_eq!(next.observed_generation, 1);
    assert_eq!(next.restart_counts.get(&predecessor.alloc_id), Some(&2));
    assert_eq!(next.restart_counts.get(fresh), Some(&2));
    assert_eq!(next.last_failure_seen_at.get(&predecessor.alloc_id), Some(&instant(10)));
    assert_eq!(next.last_failure_seen_at.get(fresh), Some(&instant(10)));
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/acceptance/vm_recreation_allocation_identity.proptest-regressions",
        )))),
        ..ProptestConfig::default()
    })]

    /// S-284-PROP-01 — over the unbounded allocation-attempt domain sampled
    /// below the checked ceiling, VM identity selection is the successor of
    /// the greatest accepted-row or issued-reservation suffix.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn vm_identity_advances_above_rows_and_reservations(
        accepted_suffix in 0_u32..100_000,
        reservation_gap in 0_u32..100_000,
    ) {
        let reserved_suffix = accepted_suffix.saturating_add(reservation_gap);
        let predecessor = failed_row("property-vm", accepted_suffix);
        let predecessor_id = predecessor.alloc_id.clone();
        let (desired, actual) = states(vm_job("property-vm"), [predecessor]);
        let reserved = aid(&format!("alloc-property-vm-{reserved_suffix}"));
        let mut view = WorkloadLifecycleView::default();
        view.restart_counts.insert(predecessor_id, 0);
        view.restart_counts.insert(reserved, 0);

        let (actions, _) = reconcile(&desired, &actual, &view, 20);
        let (fresh, _, spec) = only_start(&actions);
        let expected_suffix = accepted_suffix.max(reserved_suffix) + 1;

        prop_assert_eq!(fresh.as_str(), format!("alloc-property-vm-{expected_suffix}"));
        prop_assert_eq!(&spec.alloc, fresh);
        prop_assert_eq!(
            spec.identity.clone(),
            SpiffeId::for_allocation(&wid("property-vm"), fresh),
        );
    }
}
