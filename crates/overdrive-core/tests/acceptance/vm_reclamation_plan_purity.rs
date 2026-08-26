//! `VmReclamation`'s pure diff — `microvm-driver-cloud-hypervisor` (GH #42),
//! cross-cutting (SD-1, `reconcilers.md` Bar 2).
//!
//! Per `docs/feature/microvm-driver-cloud-hypervisor/distill/test-scenarios.md`
//! § "Cross-cutting — `VmReclamation` reconciler" (S-VM-31, S-VM-32,
//! S-VM-92) and ADR-0083 §D7, `brief.md` §§105a.3-105a.4. All three are
//! `@property` scenarios and mandatory mutation targets.
//!
//! Per CLAUDE.md § "Implement to the design": `plan_reclamation` takes NO
//! port parameter (pure, no I/O) and `SupervisionSet` has EXACTLY two
//! inhabitants (`Unavailable` as `Default`, `Observed(set)`).
//!
//! S-VM-32's "Platform Reclamation" ending class is structurally
//! UNREACHABLE today: `StoppedBy::PlatformReclaimed` (ADR-0081 D5) has not
//! landed — its addition is bundled by ADR-0081's own "Narrows ADR-0078
//! §D1" section with a same-commit ADR-0078 amendment + an
//! `observation_store.rs` docstring correction, which lands with
//! `execute_reclaim_allocation` (the first real producer of the
//! disposition), not with this reconciler skeleton (step 02-01).
//!
//! Per ADR-0083 §D6, exactly ONE new PUBLIC Ending-Class predicate is
//! sanctioned: `overdrive_core::transition_reason::is_platform_reclaimed`.
//! The Intentional Stop leg's real classifier
//! (`is_intentionally_stopped`, `overdrive_core::reconcilers::
//! workload_lifecycle`) is module-private by design and unreachable from
//! this integration-test crate, so the totality/disjointness property
//! below expresses that leg with a LOCAL structural stand-in
//! (`intentional_stop_leg`) — see its docs for why. Independent
//! ground-truth assertions against the REAL, private classifier live in
//! `workload_lifecycle`'s own `#[cfg(test)] mod
//! is_intentionally_stopped_tests` (same crate, reaches the private fn);
//! see that module's docs for why this property test structurally cannot
//! make those assertions itself (02-01 review finding D2).

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use overdrive_core::aggregate::{Job, Node, Vm, WorkloadDriver, WorkloadKind};
use overdrive_core::id::{NodeId, Region, WorkloadId};
use overdrive_core::observation::ProbeStatus;
use overdrive_reconcilers::reconcilers::workload_lifecycle::RESTART_BACKOFF_CEILING;
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_reconcilers::{SupervisionSet, VmAllocFacts, VmReclamationState, WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView, plan_reclamation};
use overdrive_reconcilers::service_lifecycle::{
    ServiceAllocFact, ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::traits::vm_host_state::ScopeFacts;
use overdrive_core::transition_reason::{
    ServiceFailureReason, StoppedBy, TerminalCondition, TransitionReason, is_platform_reclaimed,
};
use overdrive_core::{AllocationId, SpiffeId, UnixInstant};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// S-VM-31 — plan_reclamation fixtures
// ---------------------------------------------------------------------------

/// The six rows of `brief.md` §105a.4's decision table.
#[derive(Debug, Clone, Copy)]
enum Row {
    NonTerminalAuthorised,
    NonTerminalHeld,
    Terminal,
    UnknownVmExclusiveAuthorised,
    UnknownVmExclusiveHeld,
    CgroupScopeOnly,
}

fn arb_row() -> impl Strategy<Value = Row> {
    prop_oneof![
        Just(Row::NonTerminalAuthorised),
        Just(Row::NonTerminalHeld),
        Just(Row::Terminal),
        Just(Row::UnknownVmExclusiveAuthorised),
        Just(Row::UnknownVmExclusiveHeld),
        Just(Row::CgroupScopeOnly),
    ]
}

/// Allocation ids drawn from `[offset, offset + 100_000)` — callers pass
/// disjoint offsets so two generated ids can never collide, deterministically
/// (not merely with high probability).
fn arb_alloc_id(offset: u32) -> impl Strategy<Value = AllocationId> {
    (0u32..100_000).prop_map(move |n| {
        AllocationId::new(&format!("vm-plan-{}", n + offset)).expect("valid AllocationId")
    })
}

fn arb_workload_id() -> impl Strategy<Value = WorkloadId> {
    (0u32..100_000).prop_map(|n| WorkloadId::new(&format!("wl-{n}")).expect("valid WorkloadId"))
}

/// Build the `(desired, actual, expected)` triple for one decision-table
/// row, keyed on `alloc` — the case under test — layered with a SECOND,
/// constant "noise" allocation (a supervised, non-terminal VM, row 2's own
/// shape) that must contribute NOTHING to the output regardless of which
/// row `alloc` exercises. Proves the six-row table AND the aggregation
/// over multiple allocations simultaneously.
fn build_case(
    row: Row,
    alloc: AllocationId,
    workload_id: WorkloadId,
    noise: AllocationId,
) -> (VmReclamationState, VmReclamationState, Vec<Action>) {
    let mut desired = VmReclamationState::default();
    let mut actual = VmReclamationState::default();

    desired
        .allocations
        .insert(noise.clone(), VmAllocFacts { workload_id: workload_id.clone(), terminal: false });
    actual.host.scopes.insert(noise.clone(), ScopeFacts::default());
    let mut held = BTreeSet::new();
    held.insert(noise);

    let expected = match row {
        Row::NonTerminalAuthorised => {
            desired
                .allocations
                .insert(alloc.clone(), VmAllocFacts { workload_id, terminal: false });
            actual.host.scopes.insert(alloc.clone(), ScopeFacts::default());
            vec![Action::ReclaimAllocation { alloc_id: alloc }]
        }
        Row::NonTerminalHeld => {
            desired
                .allocations
                .insert(alloc.clone(), VmAllocFacts { workload_id, terminal: false });
            actual.host.scopes.insert(alloc.clone(), ScopeFacts::default());
            held.insert(alloc);
            Vec::new()
        }
        Row::Terminal => {
            desired.allocations.insert(alloc.clone(), VmAllocFacts { workload_id, terminal: true });
            actual.host.scopes.insert(alloc.clone(), ScopeFacts::default());
            actual.host.run_dirs.insert(alloc.clone());
            // Exempt — even a HELD supervision must not suppress the
            // disposal; the exemption never consults the predicate.
            held.insert(alloc.clone());
            vec![Action::DiscardStrandedArtifacts { alloc_id: alloc }]
        }
        Row::UnknownVmExclusiveAuthorised => {
            actual.host.run_dirs.insert(alloc.clone());
            vec![Action::DiscardStrandedArtifacts { alloc_id: alloc }]
        }
        Row::UnknownVmExclusiveHeld => {
            actual.host.clones.insert(alloc.clone(), PathBuf::from("/staging/noise.img"));
            held.insert(alloc);
            Vec::new()
        }
        Row::CgroupScopeOnly => {
            actual.host.scopes.insert(alloc, ScopeFacts::default());
            Vec::new()
        }
    };

    actual.supervision = SupervisionSet::Observed(held);
    (desired, actual, expected)
}

// ---------------------------------------------------------------------------
// S-VM-32 — Ending Class fixtures
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn terminal_row(
    state: AllocState,
    terminal: Option<TerminalCondition>,
    reason: Option<TransitionReason>,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::new("vm-ending-class-0").expect("valid AllocationId"),
        workload_id: WorkloadId::new("wl-ending-class").expect("valid WorkloadId"),
        node_id: NodeId::new("local").expect("valid NodeId"),
        state,
        updated_at: LogicalTimestamp {
            counter: 1,
            writer: NodeId::new("local").expect("valid NodeId"),
        },
        reason,
        detail: None,
        terminal,
        stderr_tail: None,
        kind: WorkloadKind::Job,
        listeners: Vec::new(),
        started_at: None,
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

fn arb_stopped_by() -> impl Strategy<Value = StoppedBy> {
    prop_oneof![
        Just(StoppedBy::Operator),
        Just(StoppedBy::Reconciler),
        Just(StoppedBy::Process),
        Just(StoppedBy::SystemGc),
    ]
}

fn arb_terminal_condition() -> impl Strategy<Value = Option<TerminalCondition>> {
    prop_oneof![
        Just(None),
        arb_stopped_by().prop_map(|by| Some(TerminalCondition::Stopped { by })),
        any::<i32>().prop_map(|c| Some(TerminalCondition::Completed { exit_code: c })),
        prop::option::of(any::<i32>())
            .prop_map(|c| Some(TerminalCondition::Failed { exit_code: c })),
        (1u32..10).prop_map(|a| Some(TerminalCondition::BackoffExhausted { attempts: a })),
    ]
}

fn arb_transition_reason() -> impl Strategy<Value = Option<TransitionReason>> {
    prop_oneof![
        Just(None),
        arb_stopped_by().prop_map(|by| Some(TransitionReason::Stopped { by })),
        any::<String>().prop_map(|d| Some(TransitionReason::DriverInternalError { detail: d })),
    ]
}

fn arb_terminal_alloc_state() -> impl Strategy<Value = AllocState> {
    prop_oneof![Just(AllocState::Terminated), Just(AllocState::Failed)]
}

/// STRUCTURAL STAND-IN for the Intentional Stop leg of S-VM-32's totality
/// property below — NOT a reusable production predicate, and deliberately
/// NOT a new named `pub` fn (02-01 review finding D1 forbids reintroducing
/// exactly that shape). The real classifier
/// (`overdrive_reconcilers::reconcilers::workload_lifecycle::is_intentionally_stopped`)
/// is module-private by design (ADR-0083 §D6 names exactly ONE new public
/// Ending-Class predicate, `is_platform_reclaimed`) and unreachable from
/// this integration-test crate.
///
/// This inline copy does NOT close review finding D2 by itself: with
/// `is_platform_reclaimed` structurally `false` today, the
/// totality/disjointness property below holds by tautology (the
/// residual `workload-failure` absorbs whatever this leg misclassifies)
/// regardless of what this function actually computes. The real,
/// load-bearing ground-truth assertions against the REAL classifier live
/// in `workload_lifecycle`'s own `#[cfg(test)] mod
/// is_intentionally_stopped_tests` (same crate, reaches the private fn).
fn intentional_stop_leg(row: &AllocStatusRow) -> bool {
    row.state == AllocState::Terminated
        && (matches!(
            row.terminal,
            Some(TerminalCondition::Stopped { by: StoppedBy::Operator | StoppedBy::SystemGc })
        ) || matches!(
            row.reason,
            Some(TransitionReason::Stopped { by: StoppedBy::Operator | StoppedBy::SystemGc })
        ))
}

proptest! {
    /// S-VM-31 — `plan_reclamation(desired, actual) -> Vec<Action>` is
    /// pure and matches the design's six-row decision table exactly for
    /// every generated `(VmReclamationState, VmReclamationState)` pair.
    #[test]
    fn plan_reclamation_is_pure_and_matches_the_decision_table(
        row in arb_row(),
        alloc in arb_alloc_id(0),
        noise in arb_alloc_id(1_000_000),
        workload_id in arb_workload_id(),
    ) {
        let (desired, actual, expected) = build_case(row, alloc, workload_id, noise);

        let actions_first = plan_reclamation(&desired, &actual);
        prop_assert_eq!(
            &actions_first, &expected,
            "row {:?} did not match the six-row decision table", row
        );

        // PURITY (02-01 review finding D3): `plan_reclamation` takes
        // `&VmReclamationState` — no `&mut`, no port parameter — and
        // `VmReclamationState` has no interior mutability, so purity is
        // structurally guaranteed by the signature. A second call proves
        // nothing a type-check doesn't already guarantee; not re-asserted.
    }

    /// S-VM-32 — every terminal `AllocStatusRow` the design's
    /// classification can produce belongs to EXACTLY ONE of the three
    /// Ending Classes (Intentional Stop, Workload Failure, Platform
    /// Reclamation).
    ///
    /// STRUCTURAL property only (see `intentional_stop_leg`'s docs above
    /// and the module docs at the top of this file): with
    /// `is_platform_reclaimed` structurally `false` today, "exactly one
    /// of three" is a tautology of how `workload-failure` is defined (the
    /// residual), not a check on the classifier's correctness. The real
    /// classifier is pinned by `workload_lifecycle`'s
    /// `is_intentionally_stopped_tests` ground-truth assertions instead
    /// (02-01 review finding D2).
    #[test]
    fn ending_class_is_total_and_disjoint_over_terminal_rows(
        state in arb_terminal_alloc_state(),
        terminal in arb_terminal_condition(),
        reason in arb_transition_reason(),
    ) {
        let row = terminal_row(state, terminal, reason);
        prop_assert!(row.state.is_terminal(), "fixture precondition: state must be terminal");

        let intentional_stop = intentional_stop_leg(&row);
        let platform_reclaimed = is_platform_reclaimed(&row);
        let workload_failure = row.state.is_terminal() && !intentional_stop && !platform_reclaimed;

        let classes = [intentional_stop, workload_failure, platform_reclaimed];
        let true_count = classes.iter().filter(|c| **c).count();
        prop_assert_eq!(
            true_count, 1,
            "terminal row must belong to EXACTLY ONE Ending Class, got {} for {:?}", true_count, row
        );
    }

    /// S-VM-92 — `SupervisionSet::reclamation_authorised` is the ONE
    /// kill-authorising predicate; `Unavailable` always returns `false`,
    /// never "unsupervised".
    #[test]
    fn supervision_set_unavailable_never_authorises_reclamation(
        alloc in arb_alloc_id(0),
        other_held in prop::collection::btree_set(arb_alloc_id(2_000_000), 0..5),
        alloc_is_member in any::<bool>(),
    ) {
        prop_assert!(
            !SupervisionSet::Unavailable.reclamation_authorised(&alloc),
            "Unavailable must NEVER authorise reclamation"
        );

        let mut held_ids = other_held;
        if alloc_is_member {
            held_ids.insert(alloc.clone());
        }
        let observed = SupervisionSet::Observed(held_ids.clone());
        prop_assert_eq!(
            observed.reclamation_authorised(&alloc),
            !held_ids.contains(&alloc),
            "Observed(held) must authorise iff alloc is NOT a member of held"
        );
    }
}

// ---------------------------------------------------------------------------
// S-VM-26 / S-VM-27 / S-VM-29 / S-VM-80 -- sibling-reconciler + VmReclamation
// P2 fixtures (`brief.md` §105a.9's three binding sites; §105a.10/§113's P2
// extension to THREE reconcilers). `plan_reclamation`'s own six-row decision
// table is covered above (S-VM-31/32/92); these four scenarios prove the
// OTHER two reconcilers a reclaimed row also reaches (`WorkloadLifecycle`,
// `ServiceLifecycle`) never compete with the ending it already authored --
// P2's "no reconciler emits a terminal claim on a reclaimed row", now
// stated directly against all three.
// ---------------------------------------------------------------------------

/// A Job-kind VM allocation row EXACTLY matching
/// `execute_reclaim_allocation`'s write shape (`brief.md` §105a.5):
/// `state: Terminated`, `reason: Stopped { by: PlatformReclaimed }`,
/// `terminal: None` -- the executor stamps only `reason`, never a
/// reconciler terminal claim.
fn vm_reclaimed_row(
    alloc_id: AllocationId,
    workload_id: WorkloadId,
    node_id: &NodeId,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id,
        workload_id,
        node_id: node_id.clone(),
        state: AllocState::Terminated,
        updated_at: LogicalTimestamp { counter: 3, writer: node_id.clone() },
        reason: Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed }),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Job,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

/// A `[vm]`-driven Job matching `workload_id` -- VM allocations are
/// exclusively Job-kind until #257 ([vm]+[service] rejected).
fn vm_job(workload_id: &WorkloadId) -> Job {
    Job {
        id: workload_id.clone(),
        replicas: NonZeroU32::new(1).unwrap_or_else(|| unreachable!("1 is non-zero")),
        resources: Resources { cpu_milli: 500, memory_bytes: 512 * 1024 * 1024 },
        driver: WorkloadDriver::Vm(Vm {
            command: "/sbin/init".to_string(),
            args: Vec::new(),
            kernel: "/var/lib/overdrive/kernels/vmlinux".to_string(),
            rootfs: "/var/lib/overdrive/images/rootfs.img".to_string(),
        }),
    }
}

fn one_node_map(node_id: &NodeId) -> BTreeMap<NodeId, Node> {
    let n = Node {
        id: node_id.clone(),
        region: Region::new("local").unwrap_or_else(|_| unreachable!("'local' is a valid Region")),
        capacity: Resources { cpu_milli: 4_000, memory_bytes: 8 * 1024 * 1024 * 1024 },
    };
    let mut m = BTreeMap::new();
    m.insert(n.id.clone(), n);
    m
}

/// Build a `(desired, actual)` `WorkloadLifecycleState` pair for a
/// Job-kind `[vm]` workload carrying `actual_rows`. Mirrors
/// `workload_lifecycle_restart.rs::states`, pinned to `WorkloadKind::Job` /
/// `WorkloadDriver::Vm`.
fn vm_job_states(
    workload_id: &WorkloadId,
    node_id: &NodeId,
    actual_rows: Vec<AllocStatusRow>,
) -> (WorkloadLifecycleState, WorkloadLifecycleState) {
    let nodes = one_node_map(node_id);
    let mut allocations = BTreeMap::new();
    for r in actual_rows {
        allocations.insert(r.alloc_id.clone(), r);
    }
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(vm_job(workload_id)),
        desired_to_stop: false,
        generation: 0,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Job,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(vm_job(workload_id)),
        desired_to_stop: false,
        generation: 0,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Job,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
        service_ports: Vec::new(),
    };
    (desired, actual)
}

fn fresh_tick(now_unix: UnixInstant) -> TickContext {
    let now = Instant::now();
    TickContext { now, now_unix, tick: 0, deadline: now + Duration::from_secs(1) }
}

fn svc_spiffe() -> SpiffeId {
    SpiffeId::new("spiffe://overdrive.local/workload/svc-reclaim/alloc/x")
        .unwrap_or_else(|_| unreachable!("fixture SPIFFE id is valid"))
}

proptest! {
    /// S-VM-26 -- a Job-kind VM allocation reclaimed by the platform is
    /// re-driven through the restart/backoff branch, NEVER finalised via
    /// `FinalizeFailed { Failed { exit_code: Some(0) } }` -- the fabricated
    /// clean-exit DD-1 trap `is_natural_exit`'s new
    /// `&& !is_platform_reclaimed` clause exists to close
    /// (`workload_lifecycle.rs:1157-1163`, `brief.md` §104/§105a.10).
    #[test]
    fn job_kind_reclaimed_vm_is_restarted_never_fabricated_completed_zero(
        alloc in arb_alloc_id(10_000_000),
        workload_id in arb_workload_id(),
        attempts in 0u32..RESTART_BACKOFF_CEILING,
        elapsed_secs in 0u64..120,
    ) {
        let node_id = NodeId::new("local").expect("valid NodeId");
        let row = vm_reclaimed_row(alloc.clone(), workload_id.clone(), &node_id);
        prop_assert!(is_platform_reclaimed(&row), "fixture precondition: row IS platform-reclaimed");

        let (desired, actual) = vm_job_states(&workload_id, &node_id, vec![row]);
        let mut restart_counts = BTreeMap::new();
        restart_counts.insert(alloc.clone(), attempts);
        let mut last_failure_seen_at = BTreeMap::new();
        last_failure_seen_at.insert(alloc, UnixInstant::from_unix_duration(Duration::from_secs(0)));
        let view = WorkloadLifecycleView { restart_counts, last_failure_seen_at, ..Default::default() };
        let tick = fresh_tick(UnixInstant::from_unix_duration(Duration::from_secs(elapsed_secs)));

        let (actions, _next) = WorkloadLifecycle::canonical().reconcile(&desired, &actual, &view, &tick);

        prop_assert!(
            !actions.iter().any(|a| matches!(
                a,
                Action::FinalizeFailed {
                    terminal: Some(TerminalCondition::Failed { exit_code: Some(0) }),
                    ..
                }
            )),
            "a platform-reclaimed Job-kind row must NEVER be finalised with a fabricated \
             Failed{{exit_code:Some(0)}} clean-exit claim; got {:?}", actions
        );
        if elapsed_secs >= 1 {
            prop_assert!(
                actions.iter().any(|a| matches!(a, Action::RestartAllocation { .. })),
                "past the (degenerate 1s) backoff window, a platform-reclaimed row must be \
                 RE-DRIVEN (restarted), not left inert; got {:?}", actions
            );
        }
    }

    /// S-VM-27 -- six consecutive reclaim-then-restart cycles never trip
    /// `RestartBudgetExhausted`: the ceiling guard (`workload_lifecycle.rs`
    /// ~:680) excludes Platform Reclamation from the attempts count.
    #[test]
    fn six_consecutive_reclamations_never_trip_restart_budget_exhausted(
        alloc in arb_alloc_id(11_000_000),
        workload_id in arb_workload_id(),
        cycle in 1u32..=6,
    ) {
        // `attempts` models the CUMULATIVE restart_counts value six
        // consecutive reclaim-then-restart cycles would have driven this
        // alloc's counter to -- at and past RESTART_BACKOFF_CEILING (5),
        // exactly the shape an unguarded ceiling branch trips on.
        let attempts = RESTART_BACKOFF_CEILING + cycle - 1;
        let node_id = NodeId::new("local").expect("valid NodeId");
        let row = vm_reclaimed_row(alloc.clone(), workload_id.clone(), &node_id);

        let (desired, actual) = vm_job_states(&workload_id, &node_id, vec![row]);
        let mut restart_counts = BTreeMap::new();
        restart_counts.insert(alloc.clone(), attempts);
        let mut last_failure_seen_at = BTreeMap::new();
        last_failure_seen_at.insert(alloc, UnixInstant::from_unix_duration(Duration::from_secs(0)));
        let view = WorkloadLifecycleView { restart_counts, last_failure_seen_at, ..Default::default() };
        let tick = fresh_tick(UnixInstant::from_unix_duration(Duration::from_secs(10)));

        let (actions, _next) = WorkloadLifecycle::canonical().reconcile(&desired, &actual, &view, &tick);

        prop_assert!(
            !actions.iter().any(|a| matches!(
                a,
                Action::FinalizeFailed {
                    terminal: Some(TerminalCondition::BackoffExhausted { .. }),
                    ..
                }
            )),
            "a platform-reclaimed row must NEVER trip RestartBudgetExhausted, however high \
             attempts ({attempts}) has climbed from prior reclamations; got {:?}", actions
        );
        prop_assert!(
            actions.iter().any(|a| matches!(a, Action::RestartAllocation { .. })),
            "the allocation must remain restartable past the ceiling when reclaimed; got {:?}", actions
        );
    }

    /// S-VM-29 -- the Service-path analogue: a reclaimed allocation is
    /// never handed a fabricated `ServiceFailed { StartupProbeFailed }` --
    /// `startup_probe_failed_action`'s new `AllocState` gate
    /// (`service_lifecycle.rs:968`).
    #[test]
    fn reclaimed_service_alloc_never_gets_fabricated_startup_probe_failed(
        alloc in arb_alloc_id(12_000_000),
        attempts in 0u32..60,
        elapsed_ms in 0u64..120_000,
    ) {
        // Ground truth: a real platform-reclaimed AllocStatusRow projects
        // AllocState::Terminated onto ServiceAllocFact.state at hydration
        // (the hydrate-actual arm sources `state` verbatim from the row) --
        // this fixture's `state` is exactly that projection.
        let node_id = NodeId::new("local").expect("valid NodeId");
        let ground_truth = vm_reclaimed_row(
            alloc.clone(),
            WorkloadId::new("wl-svc-reclaim").expect("valid WorkloadId"),
            &node_id,
        );
        prop_assert!(is_platform_reclaimed(&ground_truth));
        prop_assert_eq!(ground_truth.state, AllocState::Terminated);

        let fact = ServiceAllocFact {
            alloc_id: alloc.clone(),
            state: AllocState::Terminated,
            started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
            exit_code: None,
            latest_startup_probe: Some(ProbeStatus::Fail { last_fail_reason: "tcp_refused".to_string() }),
            max_attempts: 30,
            startup_deadline: Duration::from_secs(60),
            mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
            inferred: false,
            startup_probes_empty: false,
            latest_readiness_probe: None,
            has_readiness_probe: false,
            readiness_success_threshold: 1,
            backend_spiffe: svc_spiffe(),
            backend_addr: std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 8080)),
            latest_liveness_probe: None,
            has_liveness_probe: false,
            liveness_failure_threshold: 3,
        };
        let mut allocs = BTreeMap::new();
        allocs.insert(alloc.clone(), fact);
        let actual = ServiceLifecycleState { allocs, service_dataplane: None, prior_backend_row_at: None };

        let mut attempts_map = BTreeMap::new();
        attempts_map.insert(alloc, attempts);
        let view = ServiceLifecycleView { startup_attempts_per_alloc: attempts_map, ..Default::default() };
        let tick = fresh_tick(UnixInstant::from_unix_duration(Duration::from_millis(elapsed_ms)));

        let r = ServiceLifecycleReconciler::new();
        let (actions, _next) = r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);

        prop_assert!(
            !actions.iter().any(|a| matches!(
                a,
                Action::FinalizeFailed {
                    terminal: Some(TerminalCondition::ServiceFailed {
                        reason: ServiceFailureReason::StartupProbeFailed { .. }
                    }),
                    ..
                }
            )),
            "a platform-reclaimed (Terminated) Service alloc must never be handed a fabricated \
             StartupProbeFailed claim, however exhausted attempts/deadline read; got {:?}", actions
        );
    }

    /// S-VM-80 -- P2 (`brief.md` §105a.10/§113) as a property directly over
    /// `VmReclamation`: for ANY generated `(VmReclamationState,
    /// VmReclamationState)` pair the emitted `Vec<Action>` carries no
    /// `FinalizeFailed` and no `StopAllocation { terminal: Some(_) }` for
    /// any alloc_id -- the missing THIRD leg beside S-VM-26
    /// (`WorkloadLifecycle`) and S-VM-29 (`ServiceLifecycle`).
    #[test]
    fn plan_reclamation_never_authors_a_terminal_claim_for_any_row(
        row in arb_row(),
        alloc in arb_alloc_id(13_000_000),
        noise in arb_alloc_id(14_000_000),
        workload_id in arb_workload_id(),
    ) {
        let (desired, actual, _expected) = build_case(row, alloc, workload_id, noise);
        let actions = plan_reclamation(&desired, &actual);
        for action in &actions {
            prop_assert!(
                !matches!(action, Action::FinalizeFailed { .. }),
                "plan_reclamation must never emit FinalizeFailed (P2); got {:?} for row {:?}", action, row
            );
            prop_assert!(
                !matches!(action, Action::StopAllocation { terminal: Some(_), .. }),
                "plan_reclamation must never emit StopAllocation{{terminal:Some}} (P2); got {:?} for row {:?}", action, row
            );
        }
    }

    /// S-VM-80's literal Given, grounded: an already-platform-reclaimed row
    /// (`is_platform_reclaimed(row) == true`) is ALWAYS represented in
    /// `VmReclamationState` as the Terminal row (row 3 of `plan_reclamation`'s
    /// decision table, `terminal: true`), which is exempt UNCONDITIONALLY --
    /// `DiscardStrandedArtifacts` only, never a competing ending, regardless
    /// of the supervision set.
    #[test]
    fn already_platform_reclaimed_row_is_the_exempt_terminal_row(
        alloc in arb_alloc_id(15_000_000),
        workload_id in arb_workload_id(),
    ) {
        let real_row = terminal_row(
            AllocState::Terminated,
            None,
            Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed }),
        );
        prop_assert!(is_platform_reclaimed(&real_row), "fixture precondition");

        let mut desired = VmReclamationState::default();
        desired.allocations.insert(alloc.clone(), VmAllocFacts { workload_id, terminal: true });
        let mut actual = VmReclamationState::default();
        actual.host.scopes.insert(alloc.clone(), ScopeFacts::default());
        actual.supervision = SupervisionSet::Unavailable;

        let actions = plan_reclamation(&desired, &actual);
        prop_assert_eq!(
            actions,
            vec![Action::DiscardStrandedArtifacts { alloc_id: alloc }],
            "an already-platform-reclaimed allocation occupies the terminal-row exemption -- \
             DiscardStrandedArtifacts only, never a competing ending"
        );
    }
}
