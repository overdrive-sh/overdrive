//! GH #284 corrective acceptance properties at the existing
//! `WorkloadLifecycle::reconcile` and `Reconciler::next_evaluation_at`
//! driving ports.
//!
//! The file name is retained for feature-history continuity. Its contract is
//! no longer VM-specific: one stable `WorkloadId` owns policy while every
//! physical execution receives a fresh `AllocationId`, including legacy Exec
//! until GH #293 removes that adapter. Accepted allocation rows define the
//! numeric-current predecessor; `WorkloadLifecycleView::restart_counts` keys
//! are the driver-neutral durable issued-ID ledger. Tests observe only Actions,
//! the returned View, and immutable input rows. No private helper or new
//! production seam is exposed for testing.

#![allow(clippy::doc_markdown, clippy::expect_used, clippy::missing_panics_doc)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::SpiffeId;
use overdrive_core::aggregate::{Exec, Job, Node, Vm, WorkloadDriver, WorkloadKind};
use overdrive_core::id::{AllocationId, NodeId, Region, WorkloadId};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::driver::{AllocationSpec, DriverPayload, Resources};
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::{
    WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView, backoff_for_attempt,
};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

#[derive(Clone, Copy, Debug)]
enum ComposedDriver {
    Exec,
    Vm,
}

impl ComposedDriver {
    const ALL: [Self; 2] = [Self::Exec, Self::Vm];

    fn job(self, workload: &str) -> Job {
        let driver = match self {
            Self::Exec => WorkloadDriver::Exec(Exec {
                command: "/bin/workload".to_owned(),
                args: vec!["--serve".to_owned()],
            }),
            Self::Vm => WorkloadDriver::Vm(Vm {
                command: "/sbin/workload".to_owned(),
                args: vec!["--serve".to_owned()],
                kernel: "/srv/vm/kernel".to_owned(),
                rootfs: "/srv/vm/rootfs.ext4".to_owned(),
            }),
        };
        Job {
            id: wid(workload),
            replicas: NonZeroU32::new(1).expect("one replica"),
            resources: Resources { cpu_milli: 500, memory_bytes: 128 * 1024 * 1024 },
            driver,
        }
    }

    const fn matches_payload(self, payload: &DriverPayload) -> bool {
        matches!(
            (self, payload),
            (Self::Exec, DriverPayload::Exec(_)) | (Self::Vm, DriverPayload::Vm(_))
        )
    }
}

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

fn row(workload: &str, suffix: u32, state: AllocState) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(&format!("alloc-{workload}-{suffix}")),
        workload_id: wid(workload),
        node_id: nid("local"),
        state,
        updated_at: LogicalTimestamp { counter: u64::from(suffix) + 1, writer: nid("local") },
        reason: Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(23),
            signal: None,
            stderr_tail: Some("workload failed".to_owned()),
        }),
        detail: Some("accepted predecessor failure".to_owned()),
        terminal: None,
        stderr_tail: Some("workload failed".to_owned()),
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(instant(10)),
        workload_addr: None,
        last_terminated: None,
        restart_count: 7,
    }
}

fn failed_row(workload: &str, suffix: u32) -> AllocStatusRow {
    row(workload, suffix, AllocState::Failed)
}

fn draining_row(workload: &str, suffix: u32) -> AllocStatusRow {
    row(workload, suffix, AllocState::Draining)
}

fn reclaimed_row(workload: &str, suffix: u32) -> AllocStatusRow {
    let mut row = row(workload, suffix, AllocState::Terminated);
    row.reason = Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed });
    row.detail = Some("platform reclaimed the physical allocation".to_owned());
    row.stderr_tail = None;
    row
}

fn operator_stopped_row(workload: &str, suffix: u32) -> AllocStatusRow {
    let mut row = row(workload, suffix, AllocState::Terminated);
    row.reason = Some(TransitionReason::Stopped { by: StoppedBy::Operator });
    row.terminal = Some(TerminalCondition::Stopped { by: StoppedBy::Operator });
    row.detail = None;
    row.stderr_tail = None;
    row
}

fn system_gc_row(workload: &str, suffix: u32) -> AllocStatusRow {
    let mut row = row(workload, suffix, AllocState::Terminated);
    row.reason = Some(TransitionReason::Stopped { by: StoppedBy::SystemGc });
    row.terminal = Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc });
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

fn allocation_actions(actions: &[Action]) -> Vec<&Action> {
    actions
        .iter()
        .filter(|action| {
            matches!(
                action,
                Action::StartAllocation { .. }
                    | Action::RestartAllocation { .. }
                    | Action::StopAllocation { .. }
                    | Action::FinalizeFailed { .. }
            )
        })
        .collect()
}

fn only_restart(actions: &[Action]) -> (&AllocationId, &AllocationSpec) {
    let allocations = allocation_actions(actions);
    assert_eq!(allocations.len(), 1, "exactly one allocation action; got {actions:?}");
    match allocations[0] {
        Action::RestartAllocation { alloc_id, spec, .. } => (alloc_id, spec),
        other => panic!("MISSING_CORRECTED_BEHAVIOR: expected RestartAllocation, got {other:?}"),
    }
}

fn only_start(actions: &[Action]) -> (&AllocationId, &AllocationSpec) {
    let allocations = allocation_actions(actions);
    assert_eq!(allocations.len(), 1, "exactly one allocation action; got {actions:?}");
    match allocations[0] {
        Action::StartAllocation { alloc_id, spec, .. } => (alloc_id, spec),
        other => panic!("expected preserved StartAllocation placement, got {other:?}"),
    }
}

fn assert_restart_identity(
    driver: ComposedDriver,
    workload: &str,
    predecessor: &AllocationId,
    successor: &AllocationId,
    actions: &[Action],
) {
    let (action_predecessor, spec) = only_restart(actions);
    assert_eq!(action_predecessor, predecessor);
    assert_eq!(&spec.alloc, successor);
    assert_ne!(action_predecessor, &spec.alloc);
    assert_eq!(spec.identity, SpiffeId::for_allocation(&wid(workload), successor));
    assert!(driver.matches_payload(&spec.driver));
}

/// S-284-PURE-01 — the existing `RestartAllocation` fields carry a distinct
/// predecessor and successor for every currently composed driver.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending corrective re-DELIVER roadmap: driver-neutral allocation replacement"]
fn replacement_identity_is_driver_neutral_for_exec_and_vm() {
    for driver in ComposedDriver::ALL {
        let workload = match driver {
            ComposedDriver::Exec => "exec-replacement",
            ComposedDriver::Vm => "vm-replacement",
        };
        let predecessor = failed_row(workload, 0);
        let predecessor_id = predecessor.alloc_id.clone();
        let successor = aid(&format!("alloc-{workload}-1"));
        let (desired, actual) = states(driver.job(workload), [predecessor]);

        let (actions, next) = reconcile(&desired, &actual, &WorkloadLifecycleView::default(), 20);

        assert_restart_identity(driver, workload, &predecessor_id, &successor, &actions);
        assert_eq!(next.restart_counts.get(&predecessor_id), None);
        assert_eq!(next.restart_counts.get(&successor), Some(&1));
        assert_eq!(next.last_failure_seen_at.get(&successor), Some(&instant(20)));
    }
}

/// S-284-PURE-02 — one Workload Failure charge lands on the successor
/// candidate, while predecessor policy inputs and accepted history remain
/// immutable.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending corrective re-DELIVER roadmap: driver-neutral allocation replacement"]
fn workload_failure_charges_only_the_fresh_successor_candidate() {
    for driver in ComposedDriver::ALL {
        let workload = match driver {
            ComposedDriver::Exec => "exec-budget",
            ComposedDriver::Vm => "vm-budget",
        };
        let predecessor = failed_row(workload, 4);
        let predecessor_id = predecessor.alloc_id.clone();
        let before_predecessor = predecessor.clone();
        let successor = aid(&format!("alloc-{workload}-5"));
        let (desired, actual) = states(driver.job(workload), [predecessor]);
        let mut view = WorkloadLifecycleView::default();
        view.restart_counts.insert(predecessor_id.clone(), 2);
        view.last_failure_seen_at.insert(predecessor_id.clone(), instant(10));

        let (actions, next) = reconcile(&desired, &actual, &view, 20);

        assert_restart_identity(driver, workload, &predecessor_id, &successor, &actions);
        assert_eq!(next.restart_counts.get(&predecessor_id), Some(&2));
        assert_eq!(next.last_failure_seen_at.get(&predecessor_id), Some(&instant(10)));
        assert_eq!(next.restart_counts.get(&successor), Some(&3));
        assert_eq!(next.last_failure_seen_at.get(&successor), Some(&instant(20)));
        assert_eq!(actual.allocations.get(&predecessor_id), Some(&before_predecessor));
    }
}

/// S-284-PURE-03 — Platform Reclamation carries policy without charging or
/// restamping it, independent of the execution adapter.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending corrective re-DELIVER roadmap: driver-neutral allocation replacement"]
fn platform_reclamation_carries_policy_without_failure_charge_for_every_driver() {
    for driver in ComposedDriver::ALL {
        let workload = match driver {
            ComposedDriver::Exec => "exec-reclaimed",
            ComposedDriver::Vm => "vm-reclaimed",
        };
        let predecessor = reclaimed_row(workload, 4);
        let predecessor_id = predecessor.alloc_id.clone();
        let successor = aid(&format!("alloc-{workload}-5"));
        let (desired, actual) = states(driver.job(workload), [predecessor]);
        let mut view = WorkloadLifecycleView::default();
        view.restart_counts.insert(predecessor_id.clone(), 4);
        view.last_failure_seen_at.insert(predecessor_id.clone(), instant(41));

        let (actions, next) = reconcile(&desired, &actual, &view, 80);

        assert_restart_identity(driver, workload, &predecessor_id, &successor, &actions);
        assert_eq!(next.restart_counts.get(&predecessor_id), Some(&4));
        assert_eq!(next.last_failure_seen_at.get(&predecessor_id), Some(&instant(41)));
        assert_eq!(next.restart_counts.get(&successor), Some(&4));
        assert_eq!(next.last_failure_seen_at.get(&successor), Some(&instant(41)));
    }
}

/// S-284-PURE-04 — an eligible desired-generation replacement carries the
/// current candidate's policy into a distinct successor without charging a
/// Workload Failure.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending corrective re-DELIVER roadmap: driver-neutral allocation replacement"]
fn desired_generation_replacement_carries_policy_without_failure_charge() {
    for driver in ComposedDriver::ALL {
        let workload = match driver {
            ComposedDriver::Exec => "exec-generation",
            ComposedDriver::Vm => "vm-generation",
        };
        let predecessor = operator_stopped_row(workload, 0);
        let predecessor_id = predecessor.alloc_id.clone();
        let successor = aid(&format!("alloc-{workload}-1"));
        let (mut desired, mut actual) = states(driver.job(workload), [predecessor]);
        desired.generation = 1;
        actual.generation = 1;
        let mut view = WorkloadLifecycleView { observed_generation: 0, ..Default::default() };
        view.restart_counts.insert(predecessor_id.clone(), 2);
        view.last_failure_seen_at.insert(predecessor_id.clone(), instant(10));

        let (actions, next) = reconcile(&desired, &actual, &view, 20);

        assert_restart_identity(driver, workload, &predecessor_id, &successor, &actions);
        assert_eq!(next.observed_generation, 1);
        assert_eq!(next.restart_counts.get(&predecessor_id), Some(&2));
        assert_eq!(next.restart_counts.get(&successor), Some(&2));
        assert_eq!(next.last_failure_seen_at.get(&successor), Some(&instant(10)));
    }
}

/// S-284-PURE-05 — only the accepted numeric-current `Failed|Terminated`
/// predecessor is eligible; `Draining` and historical terminal rows cannot
/// hand off replacement ownership.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending corrective re-DELIVER roadmap: terminal predecessor handoff"]
fn only_numeric_current_failed_or_terminated_predecessor_is_eligible() {
    for driver in ComposedDriver::ALL {
        let prefix = match driver {
            ComposedDriver::Exec => "exec-handoff",
            ComposedDriver::Vm => "vm-handoff",
        };
        let draining = draining_row(prefix, 0);
        let (desired, actual) = states(driver.job(prefix), [draining]);
        let view = WorkloadLifecycleView::default();
        let (actions, next) = reconcile(&desired, &actual, &view, 20);
        assert!(allocation_actions(&actions).is_empty(), "Draining must not hand off: {actions:?}");
        assert_eq!(next, view);

        let historical = failed_row(prefix, 2);
        let current = reclaimed_row(prefix, 10);
        let current_id = current.alloc_id.clone();
        let successor = aid(&format!("alloc-{prefix}-11"));
        let (desired, actual) = states(driver.job(prefix), [historical, current]);
        let (actions, _) = reconcile(&desired, &actual, &WorkloadLifecycleView::default(), 20);
        assert_restart_identity(driver, prefix, &current_id, &successor, &actions);
    }
}

/// S-284-PURE-06 — initial placement joins the driver-neutral issued-ID
/// ledger, while SystemGc resubmission remains the pre-existing fresh
/// `StartAllocation` placement because no eligible predecessor exists.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending corrective re-DELIVER roadmap: driver-neutral issued-ID ledger"]
fn initial_placement_reserves_zero_and_system_gc_resubmit_stays_fresh_placement() {
    for driver in ComposedDriver::ALL {
        let initial_workload = match driver {
            ComposedDriver::Exec => "exec-initial",
            ComposedDriver::Vm => "vm-initial",
        };
        let (desired, actual) = states(driver.job(initial_workload), []);
        let (actions, next) = reconcile(&desired, &actual, &WorkloadLifecycleView::default(), 20);
        let (initial, spec) = only_start(&actions);
        assert_eq!(initial.as_str(), format!("alloc-{initial_workload}-0"));
        assert_eq!(&spec.alloc, initial);
        assert_eq!(spec.identity, SpiffeId::for_allocation(&wid(initial_workload), initial));
        assert_eq!(next.restart_counts.get(initial), Some(&0));
        assert!(!next.last_failure_seen_at.contains_key(initial));

        let gc_workload = match driver {
            ComposedDriver::Exec => "exec-gc",
            ComposedDriver::Vm => "vm-gc",
        };
        let predecessor = system_gc_row(gc_workload, 0);
        let predecessor_id = predecessor.alloc_id.clone();
        let (desired, actual) = states(driver.job(gc_workload), [predecessor]);
        let (actions, _) = reconcile(&desired, &actual, &WorkloadLifecycleView::default(), 20);
        let (fresh, spec) = only_start(&actions);
        assert_eq!(fresh.as_str(), format!("alloc-{gc_workload}-1"));
        assert_ne!(fresh, &predecessor_id);
        assert_eq!(&spec.alloc, fresh);
    }
}

/// S-284-PURE-07 — an accepted failed successor becomes numeric-current; a
/// higher View-only reservation consumes identity but never becomes the
/// predecessor or policy candidate.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending corrective re-DELIVER roadmap: accepted-row current projection"]
fn accepted_failed_successor_is_current_while_view_only_reservation_is_not() {
    for driver in ComposedDriver::ALL {
        let workload = match driver {
            ComposedDriver::Exec => "exec-current",
            ComposedDriver::Vm => "vm-current",
        };
        // Lexical iteration yields suffix 1 before suffix 10; numeric-current
        // must nevertheless choose 10 for both drivers.
        let historical = failed_row(workload, 1);
        let current = failed_row(workload, 10);
        let current_id = current.alloc_id.clone();
        let reservation = aid(&format!("alloc-{workload}-12"));
        let successor = aid(&format!("alloc-{workload}-13"));
        let (desired, actual) = states(driver.job(workload), [historical.clone(), current]);
        let mut view = WorkloadLifecycleView::default();
        view.restart_counts.insert(historical.alloc_id.clone(), 4);
        view.last_failure_seen_at.insert(historical.alloc_id.clone(), instant(1));
        view.restart_counts.insert(current_id.clone(), 1);
        view.last_failure_seen_at.insert(current_id.clone(), instant(10));
        view.restart_counts.insert(reservation.clone(), 99);
        view.last_failure_seen_at.insert(reservation.clone(), instant(99));

        let (actions, next) = reconcile(&desired, &actual, &view, 20);

        assert_restart_identity(driver, workload, &current_id, &successor, &actions);
        assert!(!actual.allocations.contains_key(&reservation));
        assert_eq!(next.restart_counts.get(&historical.alloc_id), Some(&4));
        assert_eq!(next.restart_counts.get(&current_id), Some(&1));
        assert_eq!(next.restart_counts.get(&reservation), Some(&99));
        assert_eq!(next.restart_counts.get(&successor), Some(&2));
        assert_eq!(next.last_failure_seen_at.get(&successor), Some(&instant(20)));
    }
}

/// S-284-PURE-08 — retry/backoff reads only the numeric-current accepted
/// candidate, independent of driver and unrelated historical/View-only values.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending corrective re-DELIVER roadmap: driver-neutral candidate policy"]
fn retry_deadline_is_derived_only_from_numeric_current_accepted_candidate() {
    for driver in ComposedDriver::ALL {
        let workload = match driver {
            ComposedDriver::Exec => "exec-deadline",
            ComposedDriver::Vm => "vm-deadline",
        };
        // Lexical iteration yields suffix 1 before suffix 10; numeric-current
        // must nevertheless choose 10 for both drivers.
        let historical = failed_row(workload, 1);
        let current = failed_row(workload, 10);
        let historical_id = historical.alloc_id.clone();
        let current_id = current.alloc_id.clone();
        let reservation = aid(&format!("alloc-{workload}-12"));
        let (desired, actual) = states(driver.job(workload), [historical, current]);
        let mut view = WorkloadLifecycleView::default();
        view.restart_counts.insert(historical_id.clone(), 4);
        view.last_failure_seen_at.insert(historical_id, instant(100));
        view.restart_counts.insert(current_id.clone(), 2);
        view.last_failure_seen_at.insert(current_id, instant(200));
        view.restart_counts.insert(reservation.clone(), 0);
        view.last_failure_seen_at.insert(reservation, instant(900));

        let deadline =
            WorkloadLifecycle::canonical().next_evaluation_at(&desired, &actual, &view, &tick(200));

        assert_eq!(deadline, Some(instant(200) + backoff_for_attempt(2)));
    }
}

/// S-284-PURE-09 — unparseable accepted rows and reservation keys do not
/// participate in the numeric allocator; the first canonical suffix is zero.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending corrective re-DELIVER roadmap: checked driver-neutral allocator"]
fn unparseable_attempts_do_not_participate_in_allocation() {
    for driver in ComposedDriver::ALL {
        let workload = match driver {
            ComposedDriver::Exec => "exec-unparseable",
            ComposedDriver::Vm => "vm-unparseable",
        };
        let mut malformed = failed_row(workload, 0);
        malformed.alloc_id = aid(&format!("legacy-{workload}-attempt"));
        let malformed_id = malformed.alloc_id.clone();
        let malformed_reservation = aid(&format!("reserved-{workload}-not-a-number"));
        let (desired, actual) = states(driver.job(workload), [malformed]);
        let mut view = WorkloadLifecycleView::default();
        view.restart_counts.insert(malformed_reservation, 3);

        let (actions, next) = reconcile(&desired, &actual, &view, 20);
        let (fresh, spec) = only_start(&actions);

        assert_eq!(fresh.as_str(), format!("alloc-{workload}-0"));
        assert_ne!(fresh, &malformed_id);
        assert_eq!(&spec.alloc, fresh);
        assert_eq!(next.restart_counts.get(fresh), Some(&0));
    }
}

/// S-284-PURE-10 — `u32::MAX - 1` issues the last identity once and a
/// `u32::MAX` maximum emits no allocation action or View mutation for either
/// driver.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending corrective re-DELIVER roadmap: checked driver-neutral allocator"]
fn allocator_issues_final_u32_identity_once_then_stops_without_wrap() {
    for driver in ComposedDriver::ALL {
        let workload = match driver {
            ComposedDriver::Exec => "exec-exhaustion",
            ComposedDriver::Vm => "vm-exhaustion",
        };
        let predecessor = failed_row(workload, u32::MAX - 1);
        let predecessor_id = predecessor.alloc_id.clone();
        let final_id = aid(&format!("alloc-{workload}-{}", u32::MAX));
        let (desired, actual) = states(driver.job(workload), [predecessor]);
        let (actions, next) = reconcile(&desired, &actual, &WorkloadLifecycleView::default(), 20);
        assert_restart_identity(driver, workload, &predecessor_id, &final_id, &actions);
        assert!(next.restart_counts.contains_key(&final_id));

        let exhausted = failed_row(workload, u32::MAX);
        let (desired, actual) = states(driver.job(workload), [exhausted]);
        let view = WorkloadLifecycleView::default();
        let (actions, next) = reconcile(&desired, &actual, &view, 20);
        assert!(allocation_actions(&actions).is_empty(), "exhaustion must emit no allocation");
        assert_eq!(next, view);
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/acceptance/vm_recreation_allocation_identity.proptest-regressions",
        )))),
        ..ProptestConfig::default()
    })]

    /// S-284-PROP-01 — for arbitrary gaps below the checked ceiling, both
    /// composed drivers choose exactly one above the maximum accepted-row or
    /// issued-reservation suffix and retain the accepted row as predecessor.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending corrective re-DELIVER roadmap: checked driver-neutral allocator"]
    fn identity_advances_above_rows_and_reservations_for_every_driver(
        accepted_suffix in 0_u32..100_000,
        reservation_gap in 0_u32..100_000,
    ) {
        let reserved_suffix = accepted_suffix.saturating_add(reservation_gap);
        for driver in ComposedDriver::ALL {
            let workload = match driver {
                ComposedDriver::Exec => "exec-property",
                ComposedDriver::Vm => "vm-property",
            };
            let predecessor = failed_row(workload, accepted_suffix);
            let predecessor_id = predecessor.alloc_id.clone();
            let (desired, actual) = states(driver.job(workload), [predecessor]);
            let reserved = aid(&format!("alloc-{workload}-{reserved_suffix}"));
            let mut view = WorkloadLifecycleView::default();
            view.restart_counts.insert(predecessor_id.clone(), 0);
            view.restart_counts.insert(reserved, 0);

            let (actions, _) = reconcile(&desired, &actual, &view, 20);
            let expected_suffix = accepted_suffix.max(reserved_suffix) + 1;
            let successor = aid(&format!("alloc-{workload}-{expected_suffix}"));
            let (action_predecessor, spec) = only_restart(&actions);

            prop_assert_eq!(action_predecessor, &predecessor_id);
            prop_assert_eq!(&spec.alloc, &successor);
            prop_assert_eq!(
                spec.identity.clone(),
                SpiffeId::for_allocation(&wid(workload), &successor),
            );
            prop_assert!(driver.matches_payload(&spec.driver));
        }
    }
}
