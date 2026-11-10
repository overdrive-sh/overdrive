//! Branch + boundary coverage for `ServiceLifecycleReconciler::reconcile`.
//!
//! Pin every observable predicate the reconcile body relies on so that
//! mutation testing (cargo-mutants `--diff origin/main`) kills every
//! flipped operator and dropped match arm. Categories covered:
//!
//! A. **Stable branch** — `state == Running && Some(Pass)` ⇒
//!    one `Action::FinalizeFailed { Stable { settled_in_ms, witness } }`
//!    while the committed row remains the authority for re-announcement.
//! B. **D6 terminal rows** — a `Failed` or `Terminated` fact emits no action
//!    and joins `terminal_announced`; S-56 covers the state/startup grid.
//! C. **StartupProbeFailed branch** — `attempts >= max_attempts`
//!    ∧ `elapsed_ms >= deadline_ms` ∧ no Pass ⇒ `Action::FinalizeFailed {
//!    ServiceFailed { StartupProbeFailed { probe_idx: 0, last_fail,
//!    attempts } } }`. Boundary tests pin all three `>=` operators
//!    and the `&&` chain composition.
//! D. **`settled_in_ms` arithmetic** — proptest pins the
//!    `now_ms.saturating_sub(started_at_ms)` invariant against the
//!    canonical Rust semantics; mutants that replace the function with
//!    `0` or `1` lose every non-zero case.

#![allow(clippy::expect_used)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::too_many_lines)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::id::AllocationId;
use overdrive_core::observation::{
    ProbeIdx, ProbeResultRow, ProbeRole, ProbeRunSeq, ProbeStatus, SameStatusRuns,
};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::transition_reason::{ServiceFailureReason, StoppedBy, TerminalCondition};
use overdrive_reconcilers::service_lifecycle::{
    ServiceAllocFact, ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
};
use proptest::prelude::*;

// -------------------------------------------------------------------
// Fixtures
// -------------------------------------------------------------------

fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}

fn fact(
    alloc_id: &str,
    state: AllocState,
    started_at_unix_ms: u64,
    latest_startup_probe: Option<ProbeStatus>,
    max_attempts: u32,
    startup_deadline: Duration,
) -> ServiceAllocFact {
    ServiceAllocFact {
        stable_committed: false,
        alloc_id: aid(alloc_id),
        phase: overdrive_reconcilers::service_lifecycle::NotRunningState::new(state).map_or_else(
            || {
                overdrive_reconcilers::service_lifecycle::ServiceAllocPhase::Running(
                    overdrive_reconcilers::service_lifecycle::ServiceBackendIdentity {
                        spiffe: overdrive_core::SpiffeId::new(
                            "spiffe://overdrive.local/workload/svc/alloc/x",
                        )
                        .expect("valid spiffe"),
                        ip: std::net::Ipv4Addr::LOCALHOST,
                    },
                )
            },
            overdrive_reconcilers::service_lifecycle::ServiceAllocPhase::NotRunning,
        ),
        // Per GAP-1: every fact in this branch-coverage suite represents
        // an alloc that HAS reached Running (or Failed-after-Running) —
        // the inputs assume a concrete `started_at` value. Wrap in
        // `Some(_)` at the helper boundary.
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_millis(
            started_at_unix_ms,
        ))),
        latest_startup_probe_observed_at: latest_startup_probe
            .as_ref()
            .map(|_| UnixInstant::from_unix_duration(Duration::from_millis(1))),
        latest_startup_probe,
        startup_attempts_required: max_attempts,
        startup_deadline,
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        startup_probes_empty: false,
        // Step 03-01 — this startup-branch suite carries no readiness
        // probe; the readiness branch is a no-op (no service_dataplane).
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
        probe_rows: BTreeMap::new(),
    }
}

fn one_alloc_state(f: ServiceAllocFact) -> ServiceLifecycleState {
    let alloc_id = f.alloc_id.clone();
    let probe_statuses = [
        (ProbeRole::Startup, f.latest_startup_probe.clone()),
        (ProbeRole::Readiness, f.latest_readiness_probe.clone()),
        (ProbeRole::Liveness, f.latest_liveness_probe.clone()),
    ];
    let mut state = ServiceLifecycleState {
        allocs: BTreeMap::from([(alloc_id.clone(), f)]),
        ..ServiceLifecycleState::default()
    };
    for (role, status) in probe_statuses {
        if let Some(status) = status {
            put_probe_row(&mut state, &alloc_id, role, status, 1, 1);
        }
    }
    state
}

fn put_probe_row(
    state: &mut ServiceLifecycleState,
    alloc_id: &AllocationId,
    role: ProbeRole,
    status: ProbeStatus,
    run_seq: u64,
    same_status_runs: u32,
) {
    let observed_at = UnixInstant::from_unix_duration(Duration::from_millis(run_seq));
    let fact = state.allocs.get_mut(alloc_id).expect("fixture allocation");
    match role {
        ProbeRole::Startup => {
            fact.latest_startup_probe = Some(status.clone());
            fact.latest_startup_probe_observed_at = Some(observed_at);
        }
        ProbeRole::Readiness => fact.latest_readiness_probe = Some(status.clone()),
        ProbeRole::Liveness => fact.latest_liveness_probe = Some(status.clone()),
    }
    let probe_idx = ProbeIdx::new(0);
    fact.probe_rows.insert(
        (role, probe_idx),
        ProbeResultRow {
            alloc_id: alloc_id.clone(),
            probe_idx,
            role,
            status,
            last_observed_at_unix_ms: run_seq,
            inferred: false,
            run_seq: ProbeRunSeq::new(run_seq).expect("fixture run sequence is non-zero"),
            same_status_runs: SameStatusRuns::new(same_status_runs)
                .expect("fixture streak is non-zero"),
        },
    );
}

fn tick_at_ms(now_unix_ms: u64) -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_millis(now_unix_ms)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    }
}

// =====================================================================
// Category A — Stable branch (lines 256/257, 268 settled_in_ms call)
// =====================================================================

/// Stable branch fires when state == Running AND latest_startup_probe
/// == Some(Pass) AND the row has no committed Stable terminal.
///
/// Kills mutations:
///   - line 256:27 `replace == with != in fact.state == AllocState::Running`
///   - line 257:17 `replace && with || in <Running condition> && matches!(Pass)`
///   - line 268 settled_in_ms = now - started_at (also exercises line 330)
///   - B1: a missing committed Stable permits re-announcement
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn stable_fires_when_running_and_startup_probe_pass() {
    let f = fact(
        "alloc-svc-0",
        AllocState::Running,
        1_000,
        Some(ProbeStatus::Pass),
        30,
        Duration::from_secs(60),
    );
    let actual = one_alloc_state(f);
    let view = ServiceLifecycleView::default();
    let tick = tick_at_ms(5_000);

    let r = ServiceLifecycleReconciler::new();
    let (actions, next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);

    assert_eq!(actions.len(), 1, "stable branch must emit exactly one action; got {actions:?}");
    match &actions[0] {
        Action::FinalizeFailed {
            alloc_id,
            terminal: Some(TerminalCondition::Stable { settled_in_ms, witness }),
        } => {
            assert_eq!(alloc_id.as_str(), "alloc-svc-0");
            assert_eq!(*settled_in_ms, 4_000, "settled = now (5000) - started_at (1000)");
            assert_eq!(witness.probe_idx, 0);
            assert_eq!(witness.role, "startup");
            assert_eq!(witness.mechanic_summary, "tcp 0.0.0.0:8080");
            assert!(!witness.inferred);
        }
        other => panic!("expected Stable, got {other:?}"),
    }
    assert!(next_view.observed.contains(&aid("alloc-svc-0")));
}

/// Stable does NOT fire when state != Running (kills `==` → `!=` mutant
/// at line 256 — Running case fires WITHOUT the mutation. A terminal
/// Failed state now closes its watch under D6; this case uses Pending to
/// isolate the Stable state guard.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn stable_does_not_fire_when_state_is_not_running() {
    let f = fact(
        "alloc-svc-0",
        AllocState::Pending,
        1_000,
        Some(ProbeStatus::Pass),
        30,
        Duration::from_secs(60),
    );
    let actual = one_alloc_state(f);
    let view = ServiceLifecycleView::default();
    let tick = tick_at_ms(2_000);

    let r = ServiceLifecycleReconciler::new();
    let (actions, next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);

    assert!(actions.is_empty(), "no Stable emission for non-Running state; got {actions:?}");
    assert!(next_view.observed.contains(&aid("alloc-svc-0")), "the pending row stays observed");
}

/// Stable does NOT fire when state == Running but probe is Fail (kills
/// `&&` → `||` mutant at line 257). Without the `&&`, ANY Running alloc
/// (probe Fail / None) would also emit Stable.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn stable_does_not_fire_when_running_but_probe_is_fail() {
    let f = fact(
        "alloc-svc-0",
        AllocState::Running,
        1_000,
        Some(ProbeStatus::Fail { last_fail_reason: "connection refused".to_string() }),
        30,
        Duration::from_secs(60),
    );
    let actual = one_alloc_state(f);
    let view = ServiceLifecycleView::default();
    let tick = tick_at_ms(2_000);

    let r = ServiceLifecycleReconciler::new();
    let (actions, next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);

    assert!(
        actions.is_empty(),
        "Running + probe Fail must NOT fire Stable (kills && → || mutant); got {actions:?}"
    );
    assert!(next_view.observed.contains(&aid("alloc-svc-0")));
}

/// Stable does NOT fire when Running but probe is None (also kills
/// `&&` → `||` at line 257, in a different shape).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn stable_does_not_fire_when_running_but_no_probe_observed() {
    let f = fact("alloc-svc-0", AllocState::Running, 1_000, None, 30, Duration::from_secs(60));
    let actual = one_alloc_state(f);
    let view = ServiceLifecycleView::default();
    let tick = tick_at_ms(2_000);

    let r = ServiceLifecycleReconciler::new();
    let (actions, _next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);
    assert!(actions.is_empty());
}

/// B1/R2-2: a committed Stable row needs no re-announcement.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn stable_dedup_skips_already_announced_alloc() {
    let f = fact(
        "alloc-svc-0",
        AllocState::Running,
        1_000,
        Some(ProbeStatus::Pass),
        30,
        Duration::from_secs(60),
    );
    let actual = one_alloc_state(f);
    let view = ServiceLifecycleView::default();
    let mut actual = actual;
    actual.allocs.get_mut(&aid("alloc-svc-0")).expect("allocation fact").stable_committed = true;
    let tick = tick_at_ms(5_000);

    let r = ServiceLifecycleReconciler::new();
    let (actions, _next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);
    assert!(actions.is_empty(), "committed Stable must not re-emit; got {actions:?}");
}

// =====================================================================
// Category A' — Empty-probes opt-out Stable branch (branch (a'))
// =====================================================================
// ADR-0058 §4 / ADR-0059 Q5: when the operator declares
// `[[health_check.startup]] = []`, the first Running IS Stable. The
// gate is `fact.startup_probes_empty && fact.state == AllocState::Running`
// at service_lifecycle.rs:405. The Category-A tests above always carry
// `startup_probes_empty: false`, so they never enter branch (a') and do
// NOT discriminate the `== AllocState::Running` operator at line 405.
// These two tests pin it.

/// Build an empty-startup-probes fact (branch (a') opt-out shape). The
/// `fact()` helper hardcodes `startup_probes_empty: false`; this flips
/// it and clears the probe + witness shape that the opt-out path uses.
fn empty_probes_fact(
    alloc_id: &str,
    state: AllocState,
    started_at_unix_ms: u64,
) -> ServiceAllocFact {
    ServiceAllocFact {
        stable_committed: false,
        startup_probes_empty: true,
        latest_startup_probe: None,
        ..fact(alloc_id, state, started_at_unix_ms, None, 30, Duration::from_secs(60))
    }
}

/// Branch (a') fires when `startup_probes_empty == true` AND
/// `state == Running` ⇒ one `Stable { witness.mechanic_summary ==
/// "none (opted out)" }` while the row has no committed Stable.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn empty_probes_opt_out_fires_stable_when_running() {
    let f = empty_probes_fact("alloc-svc-optout", AllocState::Running, 1_000);
    let actual = one_alloc_state(f);
    let view = ServiceLifecycleView::default();
    let tick = tick_at_ms(5_000);

    let r = ServiceLifecycleReconciler::new();
    let (actions, next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);

    assert_eq!(actions.len(), 1, "opt-out branch must emit exactly one action; got {actions:?}");
    match &actions[0] {
        Action::FinalizeFailed {
            alloc_id,
            terminal: Some(TerminalCondition::Stable { settled_in_ms, witness }),
        } => {
            assert_eq!(alloc_id.as_str(), "alloc-svc-optout");
            assert_eq!(*settled_in_ms, 4_000, "settled = now (5000) - started_at (1000)");
            assert_eq!(witness.probe_idx, 0);
            assert_eq!(witness.role, "startup");
            assert_eq!(
                witness.mechanic_summary, "none (opted out)",
                "branch (a') witness names the opt-out mechanic"
            );
            assert!(!witness.inferred);
        }
        other => panic!("expected opt-out Stable, got {other:?}"),
    }
    assert!(next_view.observed.contains(&aid("alloc-svc-optout")));
}

/// Branch (a') does NOT fire when `startup_probes_empty == true` but
/// `state != Running` (Pending). Kills `== AllocState::Running` →
/// `!= AllocState::Running` at service_lifecycle.rs:405 — under the
/// mutation a Pending+empty-probes alloc would wrongly enter the
/// opt-out Stable path.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn empty_probes_opt_out_does_not_fire_when_state_is_not_running() {
    let f = empty_probes_fact("alloc-svc-optout", AllocState::Pending, 1_000);
    let actual = one_alloc_state(f);
    let view = ServiceLifecycleView::default();
    let tick = tick_at_ms(2_000);

    let r = ServiceLifecycleReconciler::new();
    let (actions, next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);

    assert!(
        actions.is_empty(),
        "opt-out Stable must NOT fire for non-Running state (kills == Running → != Running \
         mutant at line 405); got {actions:?}"
    );
    assert!(next_view.observed.contains(&aid("alloc-svc-optout")));
}

// =====================================================================
// Category C — StartupProbeFailed branch
// =====================================================================
// Line 304: delete `!` on no_pass
// Line 305: replace >= / && (4 mutants)
// Line 307: delete match arm Some(Fail { last_fail_reason })

/// D7 (S-21): when all three gates hold on a live allocation the action is
/// `StopAllocation { Stopped { StartupProbe } }`, never a `FinalizeFailed`; the
/// allocation joins `terminal_announced`.
///
/// Kills: the action swap, each `>=` of the gate, the `&&` chain, the `!` of
/// `no_pass`.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn startup_probe_failure_stops_the_allocation_when_all_three_gates_are_met() {
    // state is Pending so Stable does not fire.
    let mut f = fact(
        "alloc-svc-2",
        AllocState::Pending,
        1_000,
        Some(ProbeStatus::Fail { last_fail_reason: "tcp_refused".to_string() }),
        30,
        Duration::from_secs(60),
    );
    f.latest_startup_probe_observed_at =
        Some(UnixInstant::from_unix_duration(Duration::from_secs(60)));
    let mut actual = one_alloc_state(f);
    put_probe_row(
        &mut actual,
        &aid("alloc-svc-2"),
        ProbeRole::Startup,
        ProbeStatus::Fail { last_fail_reason: "tcp_refused".to_string() },
        30,
        30,
    );
    let view = ServiceLifecycleView::default();
    // elapsed = 61_000 - 1_000 = 60_000 >= 60_000 (deadline).
    let tick = tick_at_ms(61_000);
    let r = ServiceLifecycleReconciler::new();
    let (actions, next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);
    assert_eq!(actions.len(), 1, "the startup stop must fire; got {actions:?}");
    match &actions[0] {
        Action::StopAllocation {
            alloc_id,
            terminal: Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }),
        } => assert_eq!(alloc_id.as_str(), "alloc-svc-2"),
        other => panic!("expected StopAllocation {{ Stopped {{ StartupProbe }} }}, got {other:?}"),
    }
    assert!(next_view.terminal_announced.contains(&aid("alloc-svc-2")));
}

/// StartupProbeFailed does NOT fire when attempts == max - 1 (kills
/// `>=` → `>` mutant at line 305:25, since with `>` instead of `>=`,
/// attempts == max would NOT fire — but attempts == max - 1 production
/// also doesn't fire). We test the boundary at attempts < max.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn startup_probe_failed_does_not_fire_when_attempts_below_max() {
    let f = fact(
        "alloc-svc-2",
        AllocState::Pending,
        1_000,
        Some(ProbeStatus::Fail { last_fail_reason: "x".to_string() }),
        30,
        Duration::from_secs(60),
    );
    let mut actual = one_alloc_state(f);
    put_probe_row(
        &mut actual,
        &aid("alloc-svc-2"),
        ProbeRole::Startup,
        ProbeStatus::Fail { last_fail_reason: "x".to_string() },
        29,
        29,
    );
    let view = ServiceLifecycleView::default();
    let tick = tick_at_ms(61_000);
    let r = ServiceLifecycleReconciler::new();
    let (actions, _) = r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);
    assert!(
        actions.is_empty(),
        "producer streak(29) < max(30) => no StartupProbeFailed; got {actions:?}"
    );
}

/// StartupProbeFailed does NOT fire when elapsed_ms < deadline.
/// Kills `>=` → `>` mutant at line 305 elapsed-vs-deadline comparison.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn startup_probe_failed_does_not_fire_when_elapsed_below_deadline() {
    let f = fact(
        "alloc-svc-2",
        AllocState::Pending,
        1_000,
        Some(ProbeStatus::Fail { last_fail_reason: "x".to_string() }),
        30,
        Duration::from_secs(60),
    );
    let mut actual = one_alloc_state(f);
    put_probe_row(
        &mut actual,
        &aid("alloc-svc-2"),
        ProbeRole::Startup,
        ProbeStatus::Fail { last_fail_reason: "x".to_string() },
        30,
        30,
    );
    let view = ServiceLifecycleView::default();
    // elapsed = 30_000 - 1_000 = 29_000 < 60_000 deadline.
    let tick = tick_at_ms(30_000);
    let r = ServiceLifecycleReconciler::new();
    let (actions, _) = r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);
    assert!(actions.is_empty(), "elapsed < deadline => no StartupProbeFailed; got {actions:?}");
}

/// StartupProbeFailed does NOT fire when Pass observed (no_pass=false).
/// Kills line 304 `delete !` mutant.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn startup_probe_failed_does_not_fire_when_pass_observed() {
    // state Pending so Stable does not fire either.
    let f = fact(
        "alloc-svc-2",
        AllocState::Pending,
        1_000,
        Some(ProbeStatus::Pass),
        30,
        Duration::from_secs(60),
    );
    let mut actual = one_alloc_state(f);
    put_probe_row(&mut actual, &aid("alloc-svc-2"), ProbeRole::Startup, ProbeStatus::Pass, 30, 30);
    let view = ServiceLifecycleView::default();
    let tick = tick_at_ms(61_000);
    let r = ServiceLifecycleReconciler::new();
    let (actions, _) = r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);
    // A Pass row contributes zero failed attempts, and no_pass=false, so no emission.
    // Also no Stable because state != Running.
    assert!(
        actions.is_empty(),
        "no_pass=false (Pass observed) => no StartupProbeFailed; got {actions:?}"
    );
}

/// StartupProbeFailed `&&` → `||` mutation case 1: only one of the three
/// predicates true → no emission. Choose (attempts < max, elapsed >= deadline,
/// no_pass). Production: false. Mutant with first `&&` → `||`: would fire.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn startup_probe_failed_does_not_fire_with_only_two_of_three_gates() {
    let f = fact(
        "alloc-svc-2",
        AllocState::Pending,
        1_000,
        Some(ProbeStatus::Fail { last_fail_reason: "x".to_string() }),
        30,
        Duration::from_secs(60),
    );
    // attempts(0) < max(30) — first gate false. Other gates true.
    let view = ServiceLifecycleView::default();
    let tick = tick_at_ms(61_000);
    let r = ServiceLifecycleReconciler::new();
    let (actions, _) =
        r.reconcile(&ServiceLifecycleState::default(), &one_alloc_state(f), &view, &tick);
    assert!(
        actions.is_empty(),
        "first gate false => no StartupProbeFailed (kills && → ||); got {actions:?}"
    );
}

// =====================================================================
// Category D — settled_in_ms saturating arithmetic
// =====================================================================
// Line 330 - kills `replace settled_in_ms_from with 0` and `with 1`.
// The settled_in_ms surfaces inside Stable's TerminalCondition.

// =====================================================================
// Category E — GAP-9 Shape B: committed-row startup-window scheduling
// =====================================================================
// The former View-only predicate was removed by B1/R2-2. Reconcile and
// next_evaluation_at below prove the active and closed window outcomes.

/// An empty Service state and View schedule no startup boundary, so the
/// runtime does not re-enqueue a no-work evaluation.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn mid_startup_window_false_for_empty_view() {
    let state = ServiceLifecycleState::default();
    let view = ServiceLifecycleView::default();
    let tick = tick_at_ms(2_000);
    assert_eq!(
        ServiceLifecycleReconciler::new().next_evaluation_at(&state, &state, &view, &tick),
        None
    );
}

/// A committed Stable row with a late startup Fail has no startup boundary;
/// another observed Running allocation still schedules its own deadline.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn mid_startup_window_true_when_any_observed_alloc_still_mid_flight() {
    let mut committed = fact(
        "alloc-svc-0",
        AllocState::Running,
        1_000,
        Some(ProbeStatus::Fail { last_fail_reason: "late result".to_string() }),
        30,
        Duration::from_secs(60),
    );
    committed.stable_committed = true;
    let waiting =
        fact("alloc-svc-1", AllocState::Running, 2_000, None, 30, Duration::from_secs(60));
    let actual = ServiceLifecycleState {
        allocs: BTreeMap::from([(aid("alloc-svc-0"), committed), (aid("alloc-svc-1"), waiting)]),
        ..ServiceLifecycleState::default()
    };
    let view = ServiceLifecycleView {
        observed: BTreeSet::from([aid("alloc-svc-0"), aid("alloc-svc-1")]),
        ..ServiceLifecycleView::default()
    };
    assert_eq!(
        ServiceLifecycleReconciler::new().next_evaluation_at(
            &ServiceLifecycleState::default(),
            &actual,
            &view,
            &tick_at_ms(3_000),
        ),
        Some(UnixInstant::from_unix_duration(Duration::from_secs(62))),
        "only the still-waiting allocation retains its deadline"
    );
}

/// Reconcile populates `observed` for a mid-window alloc (Running, no
/// Pass yet — none of the terminal branches fire), and the resulting
/// next_view IS mid-startup-window. This is the integration point
/// between the reconcile body (Shape B producer) and the predicate
/// (Shape B consumer): the runtime would re-enqueue from this view.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn reconcile_records_observed_for_mid_window_alloc_and_predicate_is_true() {
    // Running, probe not yet observed → no branch fires, alloc is
    // recorded as observed (mid-flight).
    let f = fact("alloc-svc-9", AllocState::Running, 1_000, None, 30, Duration::from_secs(60));
    let actual = one_alloc_state(f);
    let view = ServiceLifecycleView::default();
    let tick = tick_at_ms(2_000);

    let r = ServiceLifecycleReconciler::new();
    let (actions, next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);

    assert!(actions.is_empty(), "mid-startup-window tick emits no actions; got {actions:?}");
    assert!(
        next_view.observed.contains(&aid("alloc-svc-9")),
        "reconcile must record the observed alloc so Shape B can keep it alive"
    );
    assert_eq!(
        r.next_evaluation_at(&ServiceLifecycleState::default(), &actual, &next_view, &tick),
        Some(UnixInstant::from_unix_duration(Duration::from_secs(61))),
        "the active window retains its startup-deadline evaluation"
    );
}

/// D7: reconcile inserts the alloc into `terminal_announced` when it emits the
/// startup stop, and the resulting next_view is NOT
/// mid-startup-window — the runtime stops re-enqueueing the dead alloc.
/// Also pins the dedup: a second reconcile against the SAME next_view
/// emits zero actions (no terminal re-emission busy-loop).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn reconcile_startup_stop_clears_mid_window_and_dedups() {
    let mut f = fact(
        "alloc-svc-8",
        AllocState::Pending,
        1_000,
        Some(ProbeStatus::Fail { last_fail_reason: "tcp_refused".to_string() }),
        5,
        Duration::from_secs(60),
    );
    f.latest_startup_probe_observed_at =
        Some(UnixInstant::from_unix_duration(Duration::from_secs(60)));
    let mut actual = one_alloc_state(f);
    put_probe_row(
        &mut actual,
        &aid("alloc-svc-8"),
        ProbeRole::Startup,
        ProbeStatus::Fail { last_fail_reason: "tcp_refused".to_string() },
        5,
        5,
    );
    let view = ServiceLifecycleView::default();
    let tick = tick_at_ms(61_000); // elapsed 60_000 >= deadline 60_000

    let r = ServiceLifecycleReconciler::new();
    let (actions, next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);

    assert_eq!(actions.len(), 1, "the startup stop must fire once; got {actions:?}");
    assert!(
        matches!(
            &actions[0],
            Action::StopAllocation {
                terminal: Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }),
                ..
            }
        ),
        "the action is the startup stop; got {actions:?}"
    );
    assert!(
        next_view.terminal_announced.contains(&aid("alloc-svc-8")),
        "terminal verdict must be recorded in terminal_announced"
    );
    assert_eq!(
        r.next_evaluation_at(&ServiceLifecycleState::default(), &actual, &next_view, &tick),
        None,
        "the startup stop closes the deadline evaluation"
    );

    // Dedup: a second reconcile against the same (now-terminal) view
    // emits ZERO actions — no terminal re-emission busy-loop.
    let (actions2, _next2) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &next_view, &tick);
    assert!(
        actions2.is_empty(),
        "terminal_announced dedup must skip re-emission on the next tick; got {actions2:?}"
    );
}

/// A service alloc that reaches `Failed` BEFORE ever starting
/// (`started_at == None` — driver spawn error, ENOMEM on fork, missing
/// exec binary) must be recorded in `terminal_announced` so the Shape B
/// predicate returns false for it. Under D6 it emits zero actions and
/// acknowledges terminal-set membership regardless of start history.
///
/// Without that membership the alloc stays in `observed` but in neither
/// terminal set, so a stale startup boundary could be scheduled forever.
/// Once WorkloadLifecycle finalises the alloc and the action shim
/// archives the observation row, the alloc vanishes from `actual.allocs`
/// — the reconcile loop can no longer re-touch it to clear the stale
/// `observed` entry — and the runtime's `view_has_backoff_pending` arm
/// re-enqueues the reconciler in a no-op busy-loop until the service is
/// deleted. This pins the predicate-false outcome that breaks the loop.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn pre_running_failed_alloc_is_terminal_and_not_mid_startup_window() {
    // Failed before Running: terminal membership closes the watch.
    let f = ServiceAllocFact {
        stable_committed: false,
        started_at: None,
        ..fact("alloc-svc-prefail", AllocState::Failed, 1_000, None, 30, Duration::from_secs(60))
    };
    let actual = one_alloc_state(f);
    let view = ServiceLifecycleView::default();
    let tick = tick_at_ms(2_000);

    let r = ServiceLifecycleReconciler::new();
    let (actions, next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);

    assert!(
        actions.is_empty(),
        "pre-Running Failed alloc is acknowledged-but-not-classified; emits no action, got {actions:?}"
    );
    assert!(
        next_view.terminal_announced.contains(&aid("alloc-svc-prefail")),
        "pre-Running Failed alloc must be recorded in terminal_announced"
    );
    assert_eq!(
        r.next_evaluation_at(&ServiceLifecycleState::default(), &actual, &next_view, &tick),
        None,
        "a pre-Running Failed allocation schedules no startup boundary"
    );
}

/// D6, DG-12 (S-56) `CrashedAllocationDoesNotKeepTheLoopAlive`: every allocation
/// `ServiceLifecycle` observes in `Failed` or `Terminated` joins
/// `terminal_announced`, whatever its `started_at`, latest startup probe, or
/// whether startup probes are declared. The returned view then holds no open
/// mid-startup window, `next_evaluation_at` is `None`, no action is emitted, and a
/// second evaluation of the same fact changes neither view nor actions. A
/// `Running` fact with no gate met is the control: it stays out of
/// `terminal_announced` and its window stays open.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn terminal_allocation_of_any_start_history_closes_its_mid_startup_window() {
    let r = ServiceLifecycleReconciler::new();
    let alloc = aid("alloc-svc-crashed");
    // Inside the startup deadline: no startup verdict can fire, so the only
    // observable outcome is the terminal-set membership.
    let tick = tick_at_ms(2_000);
    let probes = [
        None,
        Some(ProbeStatus::Fail { last_fail_reason: "tcp_refused".to_string() }),
        Some(ProbeStatus::Pass),
    ];
    for state in [AllocState::Failed, AllocState::Terminated] {
        for started in [true, false] {
            for probe in &probes {
                for declared in [true, false] {
                    let case = format!(
                        "state {state:?}, started {started}, probe {probe:?}, declared {declared}"
                    );
                    let mut f =
                        fact("alloc-svc-crashed", state, 1_000, None, 30, Duration::from_secs(60));
                    f.stable_committed = false;
                    f.startup_probes_empty = !declared;
                    if !started {
                        f.started_at = None;
                    }
                    let mut actual = one_alloc_state(f);
                    if let Some(status) = probe {
                        put_probe_row(
                            &mut actual,
                            &alloc,
                            ProbeRole::Startup,
                            status.clone(),
                            1,
                            1,
                        );
                    }
                    // As after its `Running` evaluation: the view already observed it.
                    let view = ServiceLifecycleView {
                        observed: BTreeSet::from([alloc.clone()]),
                        ..ServiceLifecycleView::default()
                    };

                    let (actions, next_view) =
                        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);

                    assert!(actions.is_empty(), "{case}: no action; got {actions:?}");
                    assert!(
                        next_view.terminal_announced.contains(&alloc),
                        "{case}: joins terminal_announced"
                    );
                    assert_eq!(
                        r.next_evaluation_at(
                            &ServiceLifecycleState::default(),
                            &actual,
                            &next_view,
                            &tick,
                        ),
                        None,
                        "{case}: no further evaluation is requested"
                    );
                    let (again_actions, again_view) =
                        r.reconcile(&ServiceLifecycleState::default(), &actual, &next_view, &tick);
                    assert!(again_actions.is_empty(), "{case}: second evaluation emits nothing");
                    assert_eq!(again_view, next_view, "{case}: second evaluation changes no view");
                }
            }
        }
    }

    // Control: a `Running` fact with no gate met is not terminal and keeps its window open.
    let running = one_alloc_state(fact(
        "alloc-svc-crashed",
        AllocState::Running,
        1_000,
        None,
        30,
        Duration::from_secs(60),
    ));
    let view = ServiceLifecycleView {
        observed: BTreeSet::from([alloc.clone()]),
        ..ServiceLifecycleView::default()
    };
    let (actions, next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &running, &view, &tick);
    assert!(actions.is_empty(), "control: no gate met, no action; got {actions:?}");
    assert!(!next_view.terminal_announced.contains(&alloc), "control: Running is not terminal");
    assert_eq!(
        r.next_evaluation_at(&ServiceLifecycleState::default(), &running, &next_view, &tick),
        Some(UnixInstant::from_unix_duration(Duration::from_secs(61))),
        "control: the running allocation retains its startup boundary"
    );
}

proptest! {
    /// Property: settled_in_ms (observable via Stable's TerminalCondition)
    /// equals now_ms.saturating_sub(started_at_ms). Kills mutants that
    /// replace the function body with 0 or 1: any test case where the
    /// true value is neither 0 nor 1 falsifies the mutant.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn stable_settled_in_ms_equals_saturating_sub(
        now_ms in 0u64..1_000_000u64,
        started_at_ms in 0u64..1_000_000u64,
    ) {
        let expected = now_ms.saturating_sub(started_at_ms);
        let f = fact(
            "alloc-prop",
            AllocState::Running,
            started_at_ms,
            Some(ProbeStatus::Pass),
            30,
            Duration::from_secs(3_600),
        );
        let actual = one_alloc_state(f);
        let view = ServiceLifecycleView::default();
        let tick = tick_at_ms(now_ms);
        let r = ServiceLifecycleReconciler::new();
        let (actions, _) = r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);
        prop_assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::FinalizeFailed {
                terminal: Some(TerminalCondition::Stable { settled_in_ms, .. }),
                ..
            } => {
                prop_assert_eq!(*settled_in_ms, expected);
            }
            other => panic!("expected Stable, got {other:?}"),
        }
    }
}

// -------------------------------------------------------------------
// E. Readiness branch (Slice 04 / step 03-01) — Backend.healthy is
//    recomputed every tick from the latest producer row + the live
//    success_threshold. These
//    tests are CO-LOCATED in overdrive-core (not just the control-plane
//    acceptance suite) so cargo-mutants `--package overdrive-core` kills
//    the `compute_backend_healthy` / `readiness_backend_row_action`
//    mutants — the killing tests must live in the mutated crate.
//
//    Port-to-port: driven through `Reconciler::reconcile`, asserting on
//    the emitted `Action::WriteServiceBackendRow` row's per-backend
//    `healthy` flags (the observable surface), never internal fields.
// -------------------------------------------------------------------

fn readiness_fact(
    index: usize,
    latest_readiness: Option<ProbeStatus>,
    has_readiness_probe: bool,
    success_threshold: u32,
) -> ServiceAllocFact {
    ServiceAllocFact {
        // D12: eligibility reads the committed Stable; a readiness-only fixture
        // models an allocation whose Stable is already committed.
        stable_committed: true,
        alloc_id: aid(&format!("svc-{index}")),
        phase: overdrive_reconcilers::service_lifecycle::ServiceAllocPhase::Running(
            overdrive_reconcilers::service_lifecycle::ServiceBackendIdentity {
                spiffe: overdrive_core::SpiffeId::new(&format!(
                    "spiffe://overdrive.local/workload/svc/alloc/a{index}"
                ))
                .expect("valid spiffe"),
                ip: std::net::Ipv4Addr::new(
                    192,
                    168,
                    1,
                    u8::try_from(10 + index).unwrap_or(u8::MAX),
                ),
            },
        ),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
        latest_startup_probe: Some(ProbeStatus::Pass),
        latest_startup_probe_observed_at: Some(UnixInstant::from_unix_duration(
            Duration::from_millis(1),
        )),
        startup_attempts_required: 30,
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        startup_probes_empty: false,
        latest_readiness_probe: latest_readiness,
        has_readiness_probe,
        readiness_success_threshold: success_threshold,
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
        probe_rows: BTreeMap::new(),
    }
}

fn readiness_dataplane() -> overdrive_reconcilers::service_lifecycle::ServiceDataplaneIdentity {
    overdrive_reconcilers::service_lifecycle::ServiceDataplaneIdentity {
        port: std::num::NonZeroU16::new(8080).expect("listener port"),
        protocol: overdrive_core::dataplane::backend_key::Proto::Tcp,
        vip: overdrive_core::id::ServiceVip::new(std::net::IpAddr::V4(std::net::Ipv4Addr::new(
            10, 96, 0, 9,
        )))
        .expect("valid vip"),
        writer: overdrive_core::id::NodeId::new("node-r").expect("valid node id"),
    }
}

fn readiness_state(facts: Vec<ServiceAllocFact>) -> ServiceLifecycleState {
    let mut state = ServiceLifecycleState {
        service_dataplane: BTreeMap::from([(
            overdrive_core::id::ServiceId::new(7).expect("service id"),
            readiness_dataplane(),
        )]),
        ..ServiceLifecycleState::default()
    };
    for f in facts {
        let alloc_id = f.alloc_id.clone();
        let startup = f.latest_startup_probe.clone();
        let readiness = f.latest_readiness_probe.clone();
        state.allocs.insert(alloc_id.clone(), f);
        if let Some(status) = startup {
            put_probe_row(&mut state, &alloc_id, ProbeRole::Startup, status, 1, 1);
        }
        if let Some(status) = readiness {
            put_probe_row(&mut state, &alloc_id, ProbeRole::Readiness, status, 1, 1);
        }
    }
    state
}

fn readiness_tick(now_ms: u64) -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_millis(now_ms)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    }
}

fn single_backend_row_healthy(actions: &[Action]) -> bool {
    let rows: Vec<_> = actions
        .iter()
        .filter_map(|a| match a {
            Action::WriteServiceBackendRow { row, .. } => Some(row.backends.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(rows.len(), 1, "exactly one WriteServiceBackendRow expected, got {actions:?}");
    let backends = &rows[0];
    assert_eq!(backends.len(), 1, "single-alloc state yields one backend");
    backends[0].healthy
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    // E1 — readiness Pass with success_threshold=1 ⇒ healthy true;
    // Fail ⇒ healthy false. Pins `compute_backend_healthy -> {true,
    // false}` and the `!`-deletion mutant. Universe = (latest ∈
    // {Pass, Fail}) — the emitted backend's `healthy` flag.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn readiness_pass_threshold_one_is_healthy_fail_is_not(passes in any::<bool>()) {
        let reconciler = ServiceLifecycleReconciler::new();
        let status = if passes {
            ProbeStatus::Pass
        } else {
            ProbeStatus::Fail { last_fail_reason: "x".to_string() }
        };
        let state = readiness_state(vec![readiness_fact(0, Some(status), true, 1)]);
        let (actions, _v) =
            reconciler.reconcile(&state, &state, &ServiceLifecycleView::default(), &readiness_tick(100));
        prop_assert_eq!(single_backend_row_healthy(&actions), passes);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    // E2 — readiness Pass with success_threshold>1 ⇒ healthy iff the
    // producer row's streak reaches threshold. Pins the `>= threshold`
    // comparison AND the `(latest==Pass) && (same_status_runs>=threshold)`
    // conjunction (both operands vary independently). Kills `&&→||`
    // and `>=→<`.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn readiness_pass_below_threshold_is_unhealthy(
        seed in 0u32..=8,
        threshold in 2u32..=6,
    ) {
        let reconciler = ServiceLifecycleReconciler::new();
        let fact = readiness_fact(0, Some(ProbeStatus::Pass), true, threshold);
        let alloc_id = fact.alloc_id.clone();
        let mut state = readiness_state(vec![fact]);
        let streak = seed.saturating_add(1);
        put_probe_row(
            &mut state,
            &alloc_id,
            ProbeRole::Readiness,
            ProbeStatus::Pass,
            u64::from(streak),
            streak,
        );
        let (actions, _v) = reconciler.reconcile(
            &state,
            &state,
            &ServiceLifecycleView::default(),
            &readiness_tick(100),
        );
        let expected = streak >= threshold;
        prop_assert_eq!(single_backend_row_healthy(&actions), expected);
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn readiness_branch_emits_write_service_backend_row() {
    // Kills `readiness_backend_row_action -> None`: a Service with a
    // dataplane identity and at least one alloc MUST emit exactly one
    // WriteServiceBackendRow.
    let reconciler = ServiceLifecycleReconciler::new();
    let state = readiness_state(vec![readiness_fact(0, Some(ProbeStatus::Pass), true, 1)]);
    let (actions, _v) = reconciler.reconcile(
        &state,
        &state,
        &ServiceLifecycleView::default(),
        &readiness_tick(100),
    );
    let row_count =
        actions.iter().filter(|a| matches!(a, Action::WriteServiceBackendRow { .. })).count();
    assert_eq!(
        row_count, 1,
        "readiness branch must emit one WriteServiceBackendRow, got {actions:?}"
    );
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn readiness_no_probe_backend_is_healthy() {
    // The `has_readiness_probe == false` early-return path: a backend
    // with no readiness probe is always healthy (backward-compat).
    let reconciler = ServiceLifecycleReconciler::new();
    let state = readiness_state(vec![readiness_fact(0, None, false, 1)]);
    let (actions, _v) = reconciler.reconcile(
        &state,
        &state,
        &ServiceLifecycleView::default(),
        &readiness_tick(100),
    );
    assert!(single_backend_row_healthy(&actions), "no-readiness backend is healthy");
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn readiness_no_dataplane_identity_emits_no_row() {
    // The `service_dataplane: BTreeMap::new()` guard: without a VIP, the readiness
    // branch is a no-op (no WriteServiceBackendRow).
    let reconciler = ServiceLifecycleReconciler::new();
    let mut allocs = BTreeMap::new();
    let f = readiness_fact(0, Some(ProbeStatus::Pass), true, 1);
    allocs.insert(f.alloc_id.clone(), f);
    let state = ServiceLifecycleState {
        allocs,
        service_dataplane: BTreeMap::new(),
        observed_backend_rows: BTreeMap::new(),
    };
    let (actions, _v) = reconciler.reconcile(
        &state,
        &state,
        &ServiceLifecycleView::default(),
        &readiness_tick(100),
    );
    let row_count =
        actions.iter().filter(|a| matches!(a, Action::WriteServiceBackendRow { .. })).count();
    assert_eq!(row_count, 0, "no dataplane identity ⇒ no row emitted");
}

// ===================================================================
// Step 03-02 / Slice 05 — liveness → RestartAllocation (US-05 / K3)
// ===================================================================
//
// These property-based tests drive the real
// `ServiceLifecycleReconciler::reconcile` (port-to-port at domain
// scope — the reconcile fn signature IS the driving port) and assert
// on the emitted `Action`s + the next-View counter slot (the
// port-exposed observable surface). Co-located in overdrive-core where
// the reconcile logic lives so cargo-mutants kills mutations in the
// liveness branch (per the 03-01 cross-crate lesson).

/// Liveness fact builder. Universe knobs: alloc `state`, the
/// `latest_liveness_probe` observation, the live `failure_threshold`,
/// and the shared `restart_count`. Every other field is a fixed
/// non-liveness default so the liveness branch is the sole variable
/// under test.
#[allow(clippy::too_many_arguments)]
fn liveness_fact(
    alloc_id: &str,
    state: AllocState,
    latest_liveness_probe: Option<ProbeStatus>,
    failure_threshold: u32,
) -> ServiceAllocFact {
    ServiceAllocFact {
        stable_committed: false,
        alloc_id: aid(alloc_id),
        phase: overdrive_reconcilers::service_lifecycle::NotRunningState::new(state).map_or_else(
            || {
                overdrive_reconcilers::service_lifecycle::ServiceAllocPhase::Running(
                    overdrive_reconcilers::service_lifecycle::ServiceBackendIdentity {
                        spiffe: overdrive_core::SpiffeId::new(
                            "spiffe://overdrive.local/workload/svc/alloc/x",
                        )
                        .expect("valid spiffe"),
                        ip: std::net::Ipv4Addr::LOCALHOST,
                    },
                )
            },
            overdrive_reconcilers::service_lifecycle::ServiceAllocPhase::NotRunning,
        ),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
        // No startup-probe observation — keeps the Stable and startup-stop
        // branches inert so the liveness branch is
        // the only emitter under test (startup branches require a Pass
        // or a Failed state we do not set here for the Running cases).
        latest_startup_probe: None,
        latest_startup_probe_observed_at: None,
        startup_attempts_required: u32::MAX, // never trips StartupProbeFailed
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        latest_liveness_probe,
        has_liveness_probe: true,
        liveness_failure_threshold: failure_threshold,
        probe_rows: BTreeMap::new(),
    }
}

// -------------------------------------------------------------------
// ADR-0087 D2 — the liveness branch is a DETECTOR: on threshold it
// emits ONE StopAllocation { terminal: Stopped { by: LivenessProbe } }.
// It reads NO restart budget and emits neither RestartAllocation nor
// FinalizeFailed — those decisions moved to WorkloadLifecycle (the sole
// restart authority). CONTRACT_SHAPE: pure-function (reconcile port).
// -------------------------------------------------------------------

// S-ROH-A-01 — a liveness threshold breach on a Running Service alloc
// emits EXACTLY one StopAllocation { Stopped { by: LivenessProbe } };
// below threshold or a non-Running alloc emits nothing on the liveness
// path. NO RestartAllocation, NO FinalizeFailed, NO budget read.
proptest! {
    /// S-ROH-A-01 — liveness threshold emits exactly one StopAllocation
    /// { Stopped { by: LivenessProbe } }; below threshold / non-Running
    /// emits nothing; never a RestartAllocation or FinalizeFailed.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn s_roh_a_01_liveness_threshold_emits_stop_allocation(
        running in any::<bool>(),
        observed_failures in 1u32..=10,
        failure_threshold in 1u32..=10,
    ) {
        let state = if running { AllocState::Running } else { AllocState::Pending };
        let fact = liveness_fact(
            "svc-live-0",
            state,
            Some(ProbeStatus::Fail { last_fail_reason: "liveness refused".to_string() }),
            failure_threshold,
        );
        let alloc_id = fact.alloc_id.clone();
        let mut actual = one_alloc_state(fact);
        put_probe_row(
            &mut actual,
            &alloc_id,
            ProbeRole::Liveness,
            ProbeStatus::Fail { last_fail_reason: "liveness refused".to_string() },
            u64::from(observed_failures),
            observed_failures,
        );
        let recon = ServiceLifecycleReconciler::new();
        let (actions, _next) = recon.reconcile(
            &ServiceLifecycleState::default(),
            &actual,
            &ServiceLifecycleView::default(),
            &tick_at_ms(10_000),
        );

        let triggered = running && observed_failures >= failure_threshold;

        // ServiceLifecycle NEVER emits a restart or a finalize on the liveness path.
        prop_assert!(
            !actions.iter().any(|a| matches!(a, Action::RestartAllocation { .. })),
            "ServiceLifecycle must never emit RestartAllocation (WorkloadLifecycle owns restart); got {:?}",
            actions
        );
        prop_assert!(
            !actions.iter().any(|a| matches!(a, Action::FinalizeFailed {
                terminal: Some(
                    TerminalCondition::BackoffExhausted { .. }
                    | TerminalCondition::ServiceFailed {
                        reason: ServiceFailureReason::LivenessProbeFailed { .. }
                    }
                ),
                ..
            })),
            "ServiceLifecycle must never finalise on the liveness path; got {:?}",
            actions
        );

        let stops: Vec<_> = actions.iter().filter(|a| matches!(a, Action::StopAllocation { .. })).collect();
        if triggered {
            prop_assert_eq!(stops.len(), 1, "exactly one StopAllocation when triggered; got {:?}", actions);
            match stops[0] {
                Action::StopAllocation { terminal, .. } => {
                    prop_assert_eq!(
                        terminal,
                        &Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }),
                        "the liveness terminate carries Stopped {{ by: LivenessProbe }}",
                    );
                }
                other => prop_assert!(false, "expected StopAllocation, got {:?}", other),
            }
        } else {
            prop_assert_eq!(stops.len(), 0, "no StopAllocation below threshold / non-Running; got {:?}", actions);
        }
    }
}

/// S-ROH-A-09 (idempotency) — on a liveness terminate the acted row's
/// run sequence is recorded, so re-reading that same row does not emit a
/// second terminate.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn liveness_terminate_records_run_and_same_row_does_not_reemit() {
    let recon = ServiceLifecycleReconciler::new();
    // Tick 1: the producer row's streak reaches threshold (3) → one stop.
    let fact1 = liveness_fact(
        "svc-idem-0",
        AllocState::Running,
        Some(ProbeStatus::Fail { last_fail_reason: "refused".to_string() }),
        3,
    );
    let alloc_id = fact1.alloc_id.clone();
    let mut actual = one_alloc_state(fact1);
    put_probe_row(
        &mut actual,
        &alloc_id,
        ProbeRole::Liveness,
        ProbeStatus::Fail { last_fail_reason: "refused".to_string() },
        3,
        3,
    );
    let (actions1, next1) = recon.reconcile(
        &ServiceLifecycleState::default(),
        &actual,
        &ServiceLifecycleView::default(),
        &tick_at_ms(10_000),
    );
    let stops1 = actions1.iter().filter(|a| matches!(a, Action::StopAllocation { .. })).count();
    assert_eq!(stops1, 1, "tick 1 emits exactly one StopAllocation; got {actions1:?}");
    let key = (aid("svc-idem-0"), ProbeIdx::new(0));
    assert_eq!(
        next1.liveness_last_acted_run.get(&key),
        Some(&ProbeRunSeq::new(3).expect("fixture run sequence is non-zero")),
        "the row acted on is recorded",
    );

    // Tick 2: no fresh row. Re-reading run 3 emits no second terminate.
    let (actions2, _next2) =
        recon.reconcile(&ServiceLifecycleState::default(), &actual, &next1, &tick_at_ms(11_000));
    assert!(
        !actions2.iter().any(|a| matches!(a, Action::StopAllocation { .. })),
        "tick 2 emits no StopAllocation for the row already acted on; got {actions2:?}",
    );
}

// ===================================================================
// Review D1 — restored coverage for three LIVE ServiceLifecycle
// behaviours that commit 0d9d0b73 (ADR-0087 single-cut) collapsed out
// of this file alongside the now-deleted RestartReason/restart_spec
// vocabulary. None of the three was CHANGED by ADR-0087; all are LIVE
// in the moved impl `overdrive-reconcilers::service_lifecycle`. Ported
// against the moved reconcile port, with the liveness assertion surface
// updated from the deleted `RestartAllocation` to the ADR-0087
// `StopAllocation { Stopped { by: LivenessProbe } }` DETECTOR shape.
// ===================================================================

// -------------------------------------------------------------------
// D1.1 — non-Running allocs are EXCLUDED from service backend
// membership. LIVE at service_lifecycle.rs:1103
// (`if fact.state != AllocState::Running { continue; }`). Dropping the
// `continue` routes east-west traffic to a dead backend (a Failed /
// Pending alloc with no readiness probe would be emitted as a healthy
// backend row via the backward-compat `healthy = true` default).
// Port-to-port: driven through `reconcile`, asserting on the emitted
// `Action::WriteServiceBackendRow` membership (its ABSENCE here).
// -------------------------------------------------------------------

/// A Failed alloc with no readiness probe MUST NOT appear in the backend
/// set — only Running allocs can serve traffic. Without the state guard,
/// `compute_backend_healthy` returns the backward-compat `true` default
/// and the dead alloc is emitted as a healthy backend. `terminal_announced`
/// is pre-seeded so the startup loop skips it (mirrors the real runtime:
/// a prior tick already announced the failure), isolating the readiness
/// exclusion under test.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn failed_alloc_excluded_from_backend_row() {
    let reconciler = ServiceLifecycleReconciler::new();
    // Failed alloc, no readiness probe → before the guard,
    // compute_backend_healthy returns true (backward-compat default).
    let mut f = readiness_fact(0, None, false, 1);
    f.phase = overdrive_reconcilers::service_lifecycle::ServiceAllocPhase::NotRunning(
        overdrive_reconcilers::service_lifecycle::NotRunningState::new(AllocState::Failed)
            .expect("not Running"),
    );
    f.started_at = Some(UnixInstant::from_unix_duration(Duration::from_secs(1)));
    f.latest_startup_probe = None;
    let state = readiness_state(vec![f]);

    let mut view = ServiceLifecycleView::default();
    // Mark terminal so the startup loop skips it (mirrors real runtime:
    // the previous tick already announced the failure).
    view.terminal_announced.insert(aid("svc-0"));

    let (actions, _v) = reconciler.reconcile(&state, &state, &view, &readiness_tick(5_000));

    let backend_rows: Vec<_> = actions
        .iter()
        .filter_map(|a| match a {
            Action::WriteServiceBackendRow { row, .. } => Some(row),
            _ => None,
        })
        .collect();
    assert_eq!(backend_rows.len(), 1, "current listener receives one complete row");
    assert!(
        backend_rows[0].backends.is_empty(),
        "Failed alloc must not appear in backend row (kills the dropped non-Running `continue` \
         that would route traffic to a dead backend); got {backend_rows:?}",
    );
}

/// A Pending alloc with no readiness probe MUST NOT appear in the backend
/// set — same root cause as the Failed case. Pending allocs have not
/// started and cannot serve traffic.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn pending_alloc_excluded_from_backend_row() {
    let reconciler = ServiceLifecycleReconciler::new();
    let mut f = readiness_fact(0, None, false, 1);
    f.phase = overdrive_reconcilers::service_lifecycle::ServiceAllocPhase::NotRunning(
        overdrive_reconcilers::service_lifecycle::NotRunningState::new(AllocState::Pending)
            .expect("not Running"),
    );
    f.started_at = None;
    f.latest_startup_probe = None;
    let state = readiness_state(vec![f]);

    let (actions, _v) = reconciler.reconcile(
        &state,
        &state,
        &ServiceLifecycleView::default(),
        &readiness_tick(5_000),
    );

    let backend_rows: Vec<_> = actions
        .iter()
        .filter_map(|a| match a {
            Action::WriteServiceBackendRow { row, .. } => Some(row),
            _ => None,
        })
        .collect();
    assert_eq!(backend_rows.len(), 1, "current listener receives one complete row");
    assert!(
        backend_rows[0].backends.is_empty(),
        "Pending alloc must not appear in backend row (kills the dropped non-Running `continue`); \
         got {backend_rows:?}",
    );
}

// -------------------------------------------------------------------
// ADR-0101 D3 — backend-row idempotence follows equal observed row content.
// Without dedup, each tick re-emits a `WriteServiceBackendRow` with a
// monotonically increasing `LogicalTimestamp`, causing unnecessary LWW
// gossip propagation at tick rate; a `healthy`-flag change must still
// emit (the complement).
// -------------------------------------------------------------------

/// Unchanged backends between ticks suppress the second write. First
/// tick (empty view, no prior fingerprint) emits; second tick with the
/// prior view and emitted row content fed back emits NOTHING.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn readiness_branch_suppresses_write_on_unchanged_backends() {
    let reconciler = ServiceLifecycleReconciler::new();
    let mut state = readiness_state(vec![readiness_fact(0, Some(ProbeStatus::Pass), true, 1)]);

    // First tick — write must happen (view starts empty, no prior fingerprint).
    let (actions_first, view_after_first) = reconciler.reconcile(
        &state,
        &state,
        &ServiceLifecycleView::default(),
        &readiness_tick(100),
    );
    let row_count_first =
        actions_first.iter().filter(|a| matches!(a, Action::WriteServiceBackendRow { .. })).count();
    assert_eq!(row_count_first, 1, "first tick must emit WriteServiceBackendRow");

    // Feed the actual first emission back as observed row content; View memory
    // alone cannot establish acknowledgement (ADR-0101 D3).
    for action in &actions_first {
        if let Action::WriteServiceBackendRow { row, .. } = action {
            state.observed_backend_rows.insert(row.service_id, row.clone());
        }
    }
    // Equal observed content suppresses the second write.
    let (actions_second, _view_after_second) =
        reconciler.reconcile(&state, &state, &view_after_first, &readiness_tick(200));
    let row_count_second = actions_second
        .iter()
        .filter(|a| matches!(a, Action::WriteServiceBackendRow { .. }))
        .count();
    assert_eq!(
        row_count_second, 0,
        "second tick with unchanged backends must suppress WriteServiceBackendRow (equal observed row content)"
    );
}

/// When a backend's `healthy` flag changes between ticks (readiness
/// probe Pass → Fail), the fingerprint changes and the write MUST be
/// emitted — the complement that keeps dedup honest.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn readiness_branch_emits_write_when_healthy_flag_changes() {
    let reconciler = ServiceLifecycleReconciler::new();

    // Tick 1: alloc with readiness probe Pass, threshold=1 → healthy=true.
    let state_healthy = readiness_state(vec![readiness_fact(0, Some(ProbeStatus::Pass), true, 1)]);
    let (actions_first, view_after_first) = reconciler.reconcile(
        &state_healthy,
        &state_healthy,
        &ServiceLifecycleView::default(),
        &readiness_tick(100),
    );
    assert_eq!(
        actions_first.iter().filter(|a| matches!(a, Action::WriteServiceBackendRow { .. })).count(),
        1,
        "first tick must emit"
    );

    // Tick 2: same alloc, readiness probe now Fail → healthy=false.
    let mut state_unhealthy = readiness_state(vec![readiness_fact(
        0,
        Some(ProbeStatus::Fail { last_fail_reason: "conn refused".to_string() }),
        true,
        1,
    )]);
    for action in &actions_first {
        if let Action::WriteServiceBackendRow { row, .. } = action {
            state_unhealthy.observed_backend_rows.insert(row.service_id, row.clone());
        }
    }
    let (actions_second, _) = reconciler.reconcile(
        &state_unhealthy,
        &state_unhealthy,
        &view_after_first,
        &readiness_tick(200),
    );
    assert_eq!(
        actions_second
            .iter()
            .filter(|a| matches!(a, Action::WriteServiceBackendRow { .. }))
            .count(),
        1,
        "healthy flag changed → must emit WriteServiceBackendRow"
    );
}

// -------------------------------------------------------------------
// D7/D9 — a startup stop suppresses a competing liveness stop at the pure
// reconciler boundary; in the reachable owner trajectory, liveness rows begin
// only after the Stable row commits. The later-tick case below pins the
// post-Stable liveness stop and the absence of a second Stable announcement.
// -------------------------------------------------------------------

/// D7: a Running alloc that fires the startup stop on the startup loop
/// (recorded in `terminal_announced`) AND has a liveness streak tripping
/// the threshold on the SAME tick must emit ONLY the startup terminal —
/// the liveness loop skips an alloc already given a terminal this tick.
/// Without the guard, both the startup stop AND a
/// `StopAllocation { LivenessProbe }` land for one alloc — contradictory.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn startup_probe_failure_suppresses_liveness_stop_same_tick() {
    let id = "svc-dual-0";
    // Running, startup probe Fail, past deadline (StartupProbeFailed
    // fires at max_attempts=1), AND liveness probe Fail with the streak
    // about to trip threshold 3.
    let f = ServiceAllocFact {
        stable_committed: false,
        latest_liveness_probe: Some(ProbeStatus::Fail {
            last_fail_reason: "liveness refused".to_string(),
        }),
        has_liveness_probe: true,
        liveness_failure_threshold: 3,
        ..fact(
            id,
            AllocState::Running,
            1_000,
            Some(ProbeStatus::Fail { last_fail_reason: "conn refused".to_string() }),
            1, // max_attempts — one observed Fail this tick trips StartupProbeFailed
            Duration::from_secs(60),
        )
    };
    let mut actual = one_alloc_state(f);
    put_probe_row(
        &mut actual,
        &aid(id),
        ProbeRole::Liveness,
        ProbeStatus::Fail { last_fail_reason: "liveness refused".to_string() },
        3,
        3,
    );
    let view = ServiceLifecycleView::default();

    let recon = ServiceLifecycleReconciler::new();
    // 61s after started_at=1s → elapsed=60s >= deadline=60s.
    let (actions, next) =
        recon.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick_at_ms(61_000));

    let startup_stops = actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                Action::StopAllocation {
                    terminal: Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }),
                    ..
                }
            )
        })
        .count();
    assert_eq!(startup_stops, 1, "exactly one startup stop; got {actions:?}");

    let liveness_stops = actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                Action::StopAllocation {
                    terminal: Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }),
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        liveness_stops, 0,
        "liveness must NOT stop an alloc already given a startup stop this tick; got {actions:?}"
    );
    assert_eq!(actions.len(), 1, "exactly one action total (the startup stop); got {actions:?}");
    assert!(next.terminal_announced.contains(&aid(id)), "alloc recorded in terminal_announced");
}

/// After the allocation's Stable row commits, the runner can produce a
/// liveness streak; `ServiceLifecycle` emits `StopAllocation { Stopped {
/// by: LivenessProbe } }` when the threshold is met, without re-emitting
/// Stable. This is the reachable D9 owner order.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn liveness_resumes_after_prior_tick_stable_announcement() {
    let id = "svc-post-stable-0";
    let f = ServiceAllocFact {
        stable_committed: true,
        latest_liveness_probe: Some(ProbeStatus::Fail {
            last_fail_reason: "liveness refused".to_string(),
        }),
        has_liveness_probe: true,
        liveness_failure_threshold: 3,
        ..fact(
            id,
            AllocState::Running,
            1_000,
            Some(ProbeStatus::Pass), // Stable committed on a prior tick
            u32::MAX,
            Duration::from_secs(60),
        )
    };
    // Prior tick committed Stable on the row; the view merely records observation.
    let mut actual = one_alloc_state(f);
    put_probe_row(
        &mut actual,
        &aid(id),
        ProbeRole::Liveness,
        ProbeStatus::Fail { last_fail_reason: "liveness refused".to_string() },
        3,
        3,
    );
    let mut view = ServiceLifecycleView::default();
    view.observed.insert(aid(id));

    let recon = ServiceLifecycleReconciler::new();
    let (actions, _next) =
        recon.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick_at_ms(20_000));

    // Startup loop skips the committed Stable row;
    // liveness loop MUST resume and terminate on the met threshold.
    let stops: Vec<_> =
        actions.iter().filter(|a| matches!(a, Action::StopAllocation { .. })).collect();
    assert_eq!(
        stops.len(),
        1,
        "liveness must emit StopAllocation for a post-Stable alloc when threshold is met \
         after the Stable row committed; got {actions:?}",
    );
    match stops[0] {
        Action::StopAllocation { terminal, .. } => {
            assert_eq!(
                terminal,
                &Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }),
                "the resumed liveness terminate carries Stopped {{ by: LivenessProbe }}",
            );
        }
        other => panic!("expected StopAllocation, got {other:?}"),
    }

    // No Stable re-emission from the committed row.
    let stable = actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                Action::FinalizeFailed { terminal: Some(TerminalCondition::Stable { .. }), .. }
            )
        })
        .count();
    assert_eq!(stable, 0, "Stable must not be re-emitted on subsequent ticks; got {actions:?}");
}
