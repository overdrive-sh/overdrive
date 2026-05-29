//! Branch + boundary coverage for `ServiceLifecycleReconciler::reconcile`.
//!
//! Pin every observable predicate the reconcile body relies on so that
//! mutation testing (cargo-mutants `--diff origin/main`) kills every
//! flipped operator and dropped match arm. Categories covered:
//!
//! A. **Stable branch** — `state == Running && Some(Pass)` ⇒
//!    one `Action::FinalizeFailed { Stable { settled_in_ms, witness } }`
//!    AND alloc inserted into `next_view.stable_announced`.
//! B. **EarlyExit branch** — `state == Failed` ∧ `elapsed_ms < deadline_ms`
//!    ∧ no Pass observed ⇒ `Action::FinalizeFailed { ServiceFailed {
//!    EarlyExit { exit_code } } }`. Boundary tests pin `<` vs `==/>/<=`.
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
use overdrive_core::observation::ProbeStatus;
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::service_lifecycle::{
    ServiceAllocFact, ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
};
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::transition_reason::{ServiceFailureReason, TerminalCondition};
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
    exit_code: Option<i32>,
    latest_startup_probe: Option<ProbeStatus>,
    max_attempts: u32,
    startup_deadline: Duration,
) -> ServiceAllocFact {
    ServiceAllocFact {
        alloc_id: aid(alloc_id),
        state,
        // Per GAP-1: every fact in this branch-coverage suite represents
        // an alloc that HAS reached Running (or Failed-after-Running) —
        // the inputs assume a concrete `started_at` value. Wrap in
        // `Some(_)` at the helper boundary.
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_millis(
            started_at_unix_ms,
        ))),
        exit_code,
        latest_startup_probe,
        max_attempts,
        startup_deadline,
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        inferred: false,
        startup_probes_empty: false,
    }
}

fn one_alloc_state(f: ServiceAllocFact) -> ServiceLifecycleState {
    let mut allocs = BTreeMap::new();
    allocs.insert(f.alloc_id.clone(), f);
    ServiceLifecycleState { allocs }
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
/// == Some(Pass) AND alloc not already in stable_announced.
///
/// Kills mutations:
///   - line 256:27 `replace == with != in fact.state == AllocState::Running`
///   - line 257:17 `replace && with || in <Running condition> && matches!(Pass)`
///   - line 268 settled_in_ms = now - started_at (also exercises line 330)
///   - line 270 next_view.stable_announced.insert(alloc_id)
#[test]
fn stable_fires_when_running_and_startup_probe_pass() {
    let f = fact(
        "alloc-svc-0",
        AllocState::Running,
        1_000,
        None,
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
    assert!(
        next_view.stable_announced.contains(&aid("alloc-svc-0")),
        "alloc must be inserted into stable_announced after emission"
    );
}

/// Stable does NOT fire when state != Running (kills `==` → `!=` mutant
/// at line 256 — Running case fires WITHOUT the mutation, and only Failed
/// (which fires EarlyExit/StartupProbeFailed) or Pending exercise the
/// alternative. We use Pending here — neither Failed-fall-through nor
/// StartupProbeFailed gate fires (max_attempts not reached).
#[test]
fn stable_does_not_fire_when_state_is_not_running() {
    let f = fact(
        "alloc-svc-0",
        AllocState::Pending,
        1_000,
        None,
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
    assert!(
        !next_view.stable_announced.contains(&aid("alloc-svc-0")),
        "alloc must NOT be inserted into stable_announced when Stable did not fire"
    );
}

/// Stable does NOT fire when state == Running but probe is Fail (kills
/// `&&` → `||` mutant at line 257). Without the `&&`, ANY Running alloc
/// (probe Fail / None) would also emit Stable.
#[test]
fn stable_does_not_fire_when_running_but_probe_is_fail() {
    let f = fact(
        "alloc-svc-0",
        AllocState::Running,
        1_000,
        None,
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
    assert!(!next_view.stable_announced.contains(&aid("alloc-svc-0")));
}

/// Stable does NOT fire when Running but probe is None (also kills
/// `&&` → `||` at line 257, in a different shape).
#[test]
fn stable_does_not_fire_when_running_but_no_probe_observed() {
    let f =
        fact("alloc-svc-0", AllocState::Running, 1_000, None, None, 30, Duration::from_secs(60));
    let actual = one_alloc_state(f);
    let view = ServiceLifecycleView::default();
    let tick = tick_at_ms(2_000);

    let r = ServiceLifecycleReconciler::new();
    let (actions, _next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);
    assert!(actions.is_empty());
}

/// Dedup: when alloc already in `stable_announced`, no Stable re-emission.
#[test]
fn stable_dedup_skips_already_announced_alloc() {
    let f = fact(
        "alloc-svc-0",
        AllocState::Running,
        1_000,
        None,
        Some(ProbeStatus::Pass),
        30,
        Duration::from_secs(60),
    );
    let actual = one_alloc_state(f);
    let mut announced = BTreeSet::new();
    announced.insert(aid("alloc-svc-0"));
    let view = ServiceLifecycleView { stable_announced: announced, ..Default::default() };
    let tick = tick_at_ms(5_000);

    let r = ServiceLifecycleReconciler::new();
    let (actions, _next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);
    assert!(actions.is_empty(), "Stable already announced; must not re-emit. got {actions:?}");
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
        startup_probes_empty: true,
        latest_startup_probe: None,
        ..fact(alloc_id, state, started_at_unix_ms, None, None, 30, Duration::from_secs(60))
    }
}

/// Branch (a') fires when `startup_probes_empty == true` AND
/// `state == Running` ⇒ one `Stable { witness.mechanic_summary ==
/// "none (opted out)" }` AND alloc inserted into `stable_announced`.
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
    assert!(
        next_view.stable_announced.contains(&aid("alloc-svc-optout")),
        "opt-out alloc must be inserted into stable_announced after emission"
    );
}

/// Branch (a') does NOT fire when `startup_probes_empty == true` but
/// `state != Running` (Pending). Kills `== AllocState::Running` →
/// `!= AllocState::Running` at service_lifecycle.rs:405 — under the
/// mutation a Pending+empty-probes alloc would wrongly enter the
/// opt-out Stable path.
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
    assert!(
        !next_view.stable_announced.contains(&aid("alloc-svc-optout")),
        "alloc must NOT be inserted into stable_announced when branch (a') did not fire"
    );
}

// =====================================================================
// Category B — EarlyExit branch
// =====================================================================
// elapsed_ms < deadline_ms gate at line 282
// no_pass at line 283 (delete `!` mutant)
// && at line 284 (&& → || mutant)
// state == Failed at line 276 (== → != mutant)

/// EarlyExit fires when state == Failed, well within deadline, no Pass.
///
/// Kills:
///   - line 276:27 `state == Failed` (== → !=)
///   - line 282:50 `elapsed_ms < deadline_ms` (< → ==/>/<=)
///   - line 283 `let no_pass = !matches!(...)` (delete `!`)
///   - line 284 `within_deadline && no_pass` (&& → ||)
#[test]
fn early_exit_fires_when_failed_within_deadline_no_pass() {
    let f =
        fact("alloc-svc-1", AllocState::Failed, 1_000, Some(42), None, 30, Duration::from_secs(60));
    let actual = one_alloc_state(f);
    let view = ServiceLifecycleView::default();
    // elapsed = 31_000 - 1_000 = 30_000 ms, well under 60_000 ms deadline.
    let tick = tick_at_ms(31_000);

    let r = ServiceLifecycleReconciler::new();
    let (actions, _next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);

    assert_eq!(actions.len(), 1, "EarlyExit must emit exactly one action; got {actions:?}");
    match &actions[0] {
        Action::FinalizeFailed {
            alloc_id,
            terminal:
                Some(TerminalCondition::ServiceFailed {
                    reason: ServiceFailureReason::EarlyExit { exit_code },
                }),
        } => {
            assert_eq!(alloc_id.as_str(), "alloc-svc-1");
            assert_eq!(*exit_code, 42, "exit_code must be propagated from fact.exit_code");
        }
        other => panic!("expected EarlyExit, got {other:?}"),
    }
}

/// Boundary: at elapsed_ms == deadline_ms - 1, EarlyExit STILL fires.
/// At elapsed_ms == deadline_ms (exactly), EarlyExit does NOT fire.
/// This pair kills the `<` → `==`, `<` → `<=`, `<` → `>` mutants at L282.
#[test]
fn early_exit_boundary_lt_deadline() {
    let f1 = fact(
        "alloc-svc-1",
        AllocState::Failed,
        1_000,
        Some(7),
        None,
        u32::MAX, // ensure StartupProbeFailed fall-through does NOT fire (attempts == 0 < u32::MAX)
        Duration::from_secs(10),
    );
    // elapsed = (10_999 - 1_000) = 9_999 < 10_000 (deadline). Fires.
    let tick_within = tick_at_ms(10_999);
    let r = ServiceLifecycleReconciler::new();
    let (actions, _) = r.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(f1.clone()),
        &ServiceLifecycleView::default(),
        &tick_within,
    );
    assert_eq!(actions.len(), 1, "elapsed = deadline-1 must fire EarlyExit; got {actions:?}");
    assert!(matches!(
        &actions[0],
        Action::FinalizeFailed {
            terminal: Some(TerminalCondition::ServiceFailed {
                reason: ServiceFailureReason::EarlyExit { .. }
            }),
            ..
        }
    ));

    // elapsed = 10_000 == deadline. NOT within (strict <). Does NOT fire EarlyExit.
    // StartupProbeFailed also does not fire (attempts == 0 < max u32::MAX).
    let tick_at = tick_at_ms(11_000);
    let (actions, _) = r.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(f1),
        &ServiceLifecycleView::default(),
        &tick_at,
    );
    assert!(
        actions.is_empty(),
        "elapsed == deadline must NOT fire EarlyExit (strict <); got {actions:?}"
    );
}

/// EarlyExit does NOT fire when Failed but Pass already observed.
/// Kills line 283 `delete !` mutant (without `!`, no_pass becomes
/// `matches!(probe, Some(Pass))` which is FALSE here, but combined with
/// `within_deadline=true`, the && becomes false, so EarlyExit would
/// continue NOT firing — except the `delete !` flips no_pass semantics
/// so `within_deadline && !no_pass` would fire when probe IS Pass).
/// This test sets probe = Pass + within deadline: production says
/// no_pass=false → no EarlyExit; mutant says no_pass=true → EarlyExit
/// fires (but only when within deadline AND no_pass=true), so flipping
/// `!` would make this test fire EarlyExit unexpectedly.
#[test]
fn early_exit_does_not_fire_when_pass_observed() {
    let f = fact(
        "alloc-svc-1",
        AllocState::Failed,
        1_000,
        Some(1),
        Some(ProbeStatus::Pass),
        30,
        Duration::from_secs(60),
    );
    // Within deadline (elapsed = 3000 < 60000), Pass observed.
    let tick = tick_at_ms(4_000);
    let r = ServiceLifecycleReconciler::new();
    let (actions, _) = r.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(f),
        &ServiceLifecycleView::default(),
        &tick,
    );
    // Note: state == Failed, Pass observed. Production: no Stable (state != Running),
    // no EarlyExit (no_pass=false). StartupProbeFailed gate: attempts(0) >= max(30) is false.
    // Expected: zero actions.
    assert!(
        actions.is_empty(),
        "Failed + Pass observed within deadline => no action (no_pass=false); got {actions:?}"
    );
}

/// EarlyExit `&&` → `||` mutation: with `||` instead of `&&`, EarlyExit
/// would fire when EITHER within_deadline OR no_pass is true. We pick
/// a case where exactly ONE is true: out-of-deadline (within_deadline=false)
/// but no_pass=true. Production must NOT fire EarlyExit; mutant WOULD fire.
/// Also need to ensure StartupProbeFailed does NOT fire to isolate the
/// EarlyExit gate — set max_attempts very high.
#[test]
fn early_exit_does_not_fire_out_of_deadline_even_with_no_pass() {
    let f = fact(
        "alloc-svc-1",
        AllocState::Failed,
        1_000,
        Some(99),
        None,
        u32::MAX, // StartupProbeFailed cannot fire (attempts(0) < u32::MAX)
        Duration::from_secs(10),
    );
    // elapsed = 30_000 - 1_000 = 29_000 > 10_000 deadline → within_deadline=false.
    let tick = tick_at_ms(30_000);
    let r = ServiceLifecycleReconciler::new();
    let (actions, _) = r.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(f),
        &ServiceLifecycleView::default(),
        &tick,
    );
    assert!(
        actions.is_empty(),
        "out-of-deadline + no_pass => no EarlyExit (kills && → || mutant); got {actions:?}"
    );
}

// =====================================================================
// Category C — StartupProbeFailed branch
// =====================================================================
// Line 304: delete `!` on no_pass
// Line 305: replace >= / && (4 mutants)
// Line 307: delete match arm Some(Fail { last_fail_reason })

/// StartupProbeFailed fires when attempts >= max, elapsed >= deadline,
/// no Pass observed. last_fail extracted from latest_startup_probe.
///
/// Kills:
///   - line 304 `delete !` on no_pass
///   - line 305 multiple `>=` → `==/>/<` and `&&` → `||`
///   - line 307 match arm Some(Fail) delete
#[test]
fn startup_probe_failed_fires_when_all_three_gates_met() {
    // state is Pending so neither Stable nor EarlyExit (Failed-gated) fires.
    let f = fact(
        "alloc-svc-2",
        AllocState::Pending,
        1_000,
        None,
        Some(ProbeStatus::Fail { last_fail_reason: "tcp_refused".to_string() }),
        30,
        Duration::from_secs(60),
    );
    let actual = one_alloc_state(f);
    let mut attempts_map = BTreeMap::new();
    // GAP-10: seed the PRIOR consecutive-fail count (29). This tick
    // observes one more Fail, so the body increments to 30 == max_attempts
    // BEFORE the gate reads it. The reported `attempts` is the
    // post-increment streak length (30).
    attempts_map.insert(aid("alloc-svc-2"), 29u32);
    let view =
        ServiceLifecycleView { startup_attempts_per_alloc: attempts_map, ..Default::default() };
    // elapsed = 61_000 - 1_000 = 60_000 >= 60_000 (deadline).
    let tick = tick_at_ms(61_000);
    let r = ServiceLifecycleReconciler::new();
    let (actions, _) = r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);
    assert_eq!(actions.len(), 1, "StartupProbeFailed must fire; got {actions:?}");
    match &actions[0] {
        Action::FinalizeFailed {
            alloc_id,
            terminal:
                Some(TerminalCondition::ServiceFailed {
                    reason:
                        ServiceFailureReason::StartupProbeFailed { probe_idx, last_fail, attempts },
                }),
        } => {
            assert_eq!(alloc_id.as_str(), "alloc-svc-2");
            assert_eq!(*probe_idx, 0);
            assert_eq!(last_fail, "tcp_refused", "last_fail must come from Fail.last_fail_reason");
            assert_eq!(*attempts, 30u32);
        }
        other => panic!("expected StartupProbeFailed, got {other:?}"),
    }
}

/// StartupProbeFailed does NOT fire when attempts == max - 1 (kills
/// `>=` → `>` mutant at line 305:25, since with `>` instead of `>=`,
/// attempts == max would NOT fire — but attempts == max - 1 production
/// also doesn't fire). We test the boundary at attempts < max.
#[test]
fn startup_probe_failed_does_not_fire_when_attempts_below_max() {
    let f = fact(
        "alloc-svc-2",
        AllocState::Pending,
        1_000,
        None,
        Some(ProbeStatus::Fail { last_fail_reason: "x".to_string() }),
        30,
        Duration::from_secs(60),
    );
    let mut attempts_map = BTreeMap::new();
    // GAP-10: seed PRIOR count 28; this tick's Fail increments to 29,
    // which is still < max(30), so StartupProbeFailed must NOT fire.
    attempts_map.insert(aid("alloc-svc-2"), 28u32);
    let view =
        ServiceLifecycleView { startup_attempts_per_alloc: attempts_map, ..Default::default() };
    let tick = tick_at_ms(61_000);
    let r = ServiceLifecycleReconciler::new();
    let (actions, _) =
        r.reconcile(&ServiceLifecycleState::default(), &one_alloc_state(f), &view, &tick);
    assert!(
        actions.is_empty(),
        "post-increment attempts(29) < max(30) => no StartupProbeFailed; got {actions:?}"
    );
}

/// StartupProbeFailed does NOT fire when elapsed_ms < deadline.
/// Kills `>=` → `>` mutant at line 305 elapsed-vs-deadline comparison.
#[test]
fn startup_probe_failed_does_not_fire_when_elapsed_below_deadline() {
    let f = fact(
        "alloc-svc-2",
        AllocState::Pending,
        1_000,
        None,
        Some(ProbeStatus::Fail { last_fail_reason: "x".to_string() }),
        30,
        Duration::from_secs(60),
    );
    let mut attempts_map = BTreeMap::new();
    attempts_map.insert(aid("alloc-svc-2"), 30u32);
    let view =
        ServiceLifecycleView { startup_attempts_per_alloc: attempts_map, ..Default::default() };
    // elapsed = 30_000 - 1_000 = 29_000 < 60_000 deadline.
    let tick = tick_at_ms(30_000);
    let r = ServiceLifecycleReconciler::new();
    let (actions, _) =
        r.reconcile(&ServiceLifecycleState::default(), &one_alloc_state(f), &view, &tick);
    assert!(actions.is_empty(), "elapsed < deadline => no StartupProbeFailed; got {actions:?}");
}

/// StartupProbeFailed does NOT fire when Pass observed (no_pass=false).
/// Kills line 304 `delete !` mutant.
#[test]
fn startup_probe_failed_does_not_fire_when_pass_observed() {
    // state Pending so Stable does not fire either.
    let f = fact(
        "alloc-svc-2",
        AllocState::Pending,
        1_000,
        None,
        Some(ProbeStatus::Pass),
        30,
        Duration::from_secs(60),
    );
    let mut attempts_map = BTreeMap::new();
    attempts_map.insert(aid("alloc-svc-2"), 30u32);
    let view =
        ServiceLifecycleView { startup_attempts_per_alloc: attempts_map, ..Default::default() };
    let tick = tick_at_ms(61_000);
    let r = ServiceLifecycleReconciler::new();
    let (actions, _) =
        r.reconcile(&ServiceLifecycleState::default(), &one_alloc_state(f), &view, &tick);
    // attempts >= max, elapsed >= deadline, but Pass observed => no_pass=false => no emission.
    // Also no Stable because state != Running.
    assert!(
        actions.is_empty(),
        "no_pass=false (Pass observed) => no StartupProbeFailed; got {actions:?}"
    );
}

/// StartupProbeFailed `&&` → `||` mutation case 1: only one of the three
/// predicates true → no emission. Choose (attempts < max, elapsed >= deadline,
/// no_pass). Production: false. Mutant with first `&&` → `||`: would fire.
#[test]
fn startup_probe_failed_does_not_fire_with_only_two_of_three_gates() {
    let f = fact(
        "alloc-svc-2",
        AllocState::Pending,
        1_000,
        None,
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

/// StartupProbeFailed extracts last_fail = "" when probe is None
/// (kills line 307 `delete match arm Some(Fail { last_fail_reason })` —
/// with the arm deleted, all-cases-default returns String::new()
/// regardless. We need a Some(Fail) case asserting last_fail is NOT
/// empty: that's the prior test `startup_probe_failed_fires_when_all_three_gates_met`
/// which sets last_fail to "tcp_refused" — flipping the arm would set
/// last_fail to "" and break that assertion).
///
/// Here we pin the None case explicitly: last_fail should be "".
/// Note: when probe is None, no_pass = !matches!(None, Some(Pass)) = true,
/// so StartupProbeFailed CAN fire. We also need attempts >= max + elapsed >= deadline.
#[test]
fn startup_probe_failed_last_fail_empty_when_probe_is_none() {
    let f = fact("alloc-svc-2", AllocState::Pending, 1_000, None, None, 5, Duration::from_secs(60));
    let mut attempts_map = BTreeMap::new();
    attempts_map.insert(aid("alloc-svc-2"), 5u32);
    let view =
        ServiceLifecycleView { startup_attempts_per_alloc: attempts_map, ..Default::default() };
    let tick = tick_at_ms(61_000);
    let r = ServiceLifecycleReconciler::new();
    let (actions, _) =
        r.reconcile(&ServiceLifecycleState::default(), &one_alloc_state(f), &view, &tick);
    assert_eq!(actions.len(), 1, "StartupProbeFailed must fire here; got {actions:?}");
    match &actions[0] {
        Action::FinalizeFailed {
            terminal:
                Some(TerminalCondition::ServiceFailed {
                    reason: ServiceFailureReason::StartupProbeFailed { last_fail, .. },
                }),
            ..
        } => {
            assert_eq!(last_fail, "", "None probe => last_fail must be empty string");
        }
        other => panic!("expected StartupProbeFailed, got {other:?}"),
    }
}

// =====================================================================
// Category D — settled_in_ms saturating arithmetic
// =====================================================================
// Line 330 - kills `replace settled_in_ms_from with 0` and `with 1`.
// The settled_in_ms surfaces inside Stable's TerminalCondition.

// =====================================================================
// Category E — GAP-9 Shape B: has_alloc_mid_startup_window predicate
// =====================================================================
//
// The runtime's `view_has_backoff_pending` ServiceLifecycle arm
// delegates to `ServiceLifecycleView::has_alloc_mid_startup_window`.
// The load-bearing contract: TRUE while an observed alloc is
// mid-startup-window (observed, not yet terminal); FALSE the instant
// the alloc reaches ANY terminal (Stable OR ServiceFailed). Get this
// wrong and GAP-9's fix trades a dead reconciler for a spinning one.
//
// This is a pure predicate over the View — the canonical port-to-port
// shape for a domain function (the method signature IS its driving
// port).

/// Empty view (no observed allocs) is NOT mid-startup-window — the
/// busy-loop-avoidance baseline: a default view (e.g. a Job-kind
/// enqueue that hydrated an empty Service state) must return false so
/// the runtime does not re-enqueue.
#[test]
fn mid_startup_window_false_for_empty_view() {
    let view = ServiceLifecycleView::default();
    assert!(
        !view.has_alloc_mid_startup_window(),
        "default/empty view must NOT be mid-startup-window (no observed alloc)"
    );
}

/// Observed-but-not-terminal alloc IS mid-startup-window → predicate
/// true → runtime self-re-enqueues. This is the active startup window:
/// the reconciler has recorded the alloc in `observed` but has not yet
/// announced Stable or a failure terminal.
#[test]
fn mid_startup_window_true_when_observed_not_terminal() {
    let mut view = ServiceLifecycleView::default();
    view.observed.insert(aid("alloc-svc-0"));
    assert!(
        view.has_alloc_mid_startup_window(),
        "observed alloc not in any terminal set must be mid-startup-window (true)"
    );
}

/// The instant the alloc reaches Stable (recorded in
/// `stable_announced`), the predicate flips to false — the runtime
/// stops re-enqueueing. Kills a mutant that ignores `stable_announced`
/// in the subtraction.
#[test]
fn mid_startup_window_false_when_observed_and_stable() {
    let mut view = ServiceLifecycleView::default();
    view.observed.insert(aid("alloc-svc-0"));
    view.stable_announced.insert(aid("alloc-svc-0"));
    assert!(
        !view.has_alloc_mid_startup_window(),
        "alloc in stable_announced is terminal → NOT mid-startup-window (no busy-loop)"
    );
}

/// The instant the alloc reaches a non-Stable terminal (recorded in
/// `terminal_announced`), the predicate flips to false. Kills a mutant
/// that ignores `terminal_announced` in the subtraction — without it a
/// dead (EarlyExit / StartupProbeFailed) alloc would spin the runtime
/// forever, which is worse than the GAP-9 gap.
#[test]
fn mid_startup_window_false_when_observed_and_terminal_failed() {
    let mut view = ServiceLifecycleView::default();
    view.observed.insert(aid("alloc-svc-0"));
    view.terminal_announced.insert(aid("alloc-svc-0"));
    assert!(
        !view.has_alloc_mid_startup_window(),
        "alloc in terminal_announced is terminal → NOT mid-startup-window (no busy-loop)"
    );
}

/// Mixed populations: one alloc still mid-flight, one terminal → the
/// predicate is true (the ANY semantics: as long as ONE observed alloc
/// is non-terminal, the runtime must keep re-enqueueing). Kills a
/// mutant that flips `any` → `all`.
#[test]
fn mid_startup_window_true_when_any_observed_alloc_still_mid_flight() {
    let mut view = ServiceLifecycleView::default();
    view.observed.insert(aid("alloc-svc-0"));
    view.observed.insert(aid("alloc-svc-1"));
    // alloc-0 reached Stable; alloc-1 still mid-flight.
    view.stable_announced.insert(aid("alloc-svc-0"));
    assert!(
        view.has_alloc_mid_startup_window(),
        "ANY mid-flight observed alloc keeps the predicate true (kills any→all mutant)"
    );
}

/// Reconcile populates `observed` for a mid-window alloc (Running, no
/// Pass yet — none of the terminal branches fire), and the resulting
/// next_view IS mid-startup-window. This is the integration point
/// between the reconcile body (Shape B producer) and the predicate
/// (Shape B consumer): the runtime would re-enqueue from this view.
#[test]
fn reconcile_records_observed_for_mid_window_alloc_and_predicate_is_true() {
    // Running, probe not yet observed → no branch fires, alloc is
    // recorded as observed (mid-flight).
    let f =
        fact("alloc-svc-9", AllocState::Running, 1_000, None, None, 30, Duration::from_secs(60));
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
    assert!(
        next_view.has_alloc_mid_startup_window(),
        "next_view from a mid-window tick must be mid-startup-window (runtime re-enqueues)"
    );
}

/// Reconcile inserts the alloc into `terminal_announced` on a
/// StartupProbeFailed terminal, and the resulting next_view is NOT
/// mid-startup-window — the runtime stops re-enqueueing the dead alloc.
/// Also pins the dedup: a second reconcile against the SAME next_view
/// emits zero actions (no terminal re-emission busy-loop).
#[test]
fn reconcile_terminal_failed_clears_mid_window_and_dedups() {
    let f = fact(
        "alloc-svc-8",
        AllocState::Pending,
        1_000,
        None,
        Some(ProbeStatus::Fail { last_fail_reason: "tcp_refused".to_string() }),
        5,
        Duration::from_secs(60),
    );
    let actual = one_alloc_state(f);
    let mut attempts_map = BTreeMap::new();
    // GAP-10: seed PRIOR count 4; this tick's Fail increments to 5 == max.
    attempts_map.insert(aid("alloc-svc-8"), 4u32);
    let view =
        ServiceLifecycleView { startup_attempts_per_alloc: attempts_map, ..Default::default() };
    let tick = tick_at_ms(61_000); // elapsed 60_000 >= deadline 60_000

    let r = ServiceLifecycleReconciler::new();
    let (actions, next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);

    assert_eq!(actions.len(), 1, "StartupProbeFailed must fire once; got {actions:?}");
    assert!(
        next_view.terminal_announced.contains(&aid("alloc-svc-8")),
        "terminal verdict must be recorded in terminal_announced"
    );
    assert!(
        !next_view.has_alloc_mid_startup_window(),
        "a terminal-failed alloc must NOT keep the runtime spinning (predicate false)"
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

// =====================================================================
// Category F — GAP-10: startup_attempts_per_alloc increment/reset
// =====================================================================
//
// Before GAP-10 the counter was READ by the StartupProbeFailed gate but
// NEVER WRITTEN, so `attempts` stayed 0 and the terminal was unreachable
// for any real spec (max_attempts >= 1) — a failure-path busy-loop. The
// fix increments by exactly 1 per observed startup-probe Fail and resets
// to 0 on the first Pass. These tests pin the four mutation-surface
// behaviours per the GAP-10 correctness checklist:
//   (a) +1 per observed Fail (no over/under-count)
//   (b) reset to 0 on Pass
//   (c) StartupProbeFailed fires at exactly attempts == max_attempts
//   (d) a Pass before max_attempts prevents StartupProbeFailed
//
// Observable surface: the counter is port-exposed via the returned
// `next_view.startup_attempts_per_alloc` (state-delta over a View slot)
// and via the emitted `StartupProbeFailed { attempts }` action.

/// (a) Each observed startup-probe Fail increments the per-alloc counter
/// by exactly 1. Kills `saturating_add(1)` → `saturating_add(0)` (no
/// movement → busy-loop returns) and `→ saturating_add(2)`/`* 2`
/// (over-count). State-delta: only the target alloc's slot moves, by +1.
#[test]
fn startup_attempt_counter_increments_by_one_per_observed_fail() {
    // Pending so no terminal branch fires; max high so StartupProbeFailed
    // does not consume the alloc — we observe the raw counter delta.
    let f = fact(
        "alloc-svc-f",
        AllocState::Pending,
        1_000,
        None,
        Some(ProbeStatus::Fail { last_fail_reason: "tcp_refused".to_string() }),
        u32::MAX,
        Duration::from_secs(60),
    );
    let actual = one_alloc_state(f);
    let r = ServiceLifecycleReconciler::new();
    let tick = tick_at_ms(2_000);

    // Tick 1: absent entry (0) + one Fail → 1.
    let (actions1, view1) = r.reconcile(
        &ServiceLifecycleState::default(),
        &actual,
        &ServiceLifecycleView::default(),
        &tick,
    );
    assert!(actions1.is_empty(), "mid-window Fail emits no action; got {actions1:?}");
    assert_eq!(
        view1.startup_attempts_per_alloc.get(&aid("alloc-svc-f")).copied(),
        Some(1),
        "first observed Fail must set the counter to exactly 1"
    );

    // Tick 2: 1 + one Fail → 2 (exactly +1, no over-count).
    let (_actions2, view2) = r.reconcile(&ServiceLifecycleState::default(), &actual, &view1, &tick);
    assert_eq!(
        view2.startup_attempts_per_alloc.get(&aid("alloc-svc-f")).copied(),
        Some(2),
        "second observed Fail must increment to exactly 2 (kills +0 and +2 mutants)"
    );
}

/// (b) A Pass resets the per-alloc counter to 0. Kills a mutant that
/// drops the reset arm (counter would stay at its prior streak value,
/// letting a recovered-then-flapping alloc fire StartupProbeFailed
/// prematurely). The alloc here is Pending (not Running) so branch (a)
/// Stable does NOT consume it — we observe the reset directly.
#[test]
fn startup_attempt_counter_resets_to_zero_on_pass() {
    let f = fact(
        "alloc-svc-g",
        AllocState::Pending,
        1_000,
        None,
        Some(ProbeStatus::Pass),
        30,
        Duration::from_secs(60),
    );
    let actual = one_alloc_state(f);
    // Seed a prior streak of 17 fails.
    let mut seeded = BTreeMap::new();
    seeded.insert(aid("alloc-svc-g"), 17u32);
    let view = ServiceLifecycleView { startup_attempts_per_alloc: seeded, ..Default::default() };
    let tick = tick_at_ms(2_000);
    let r = ServiceLifecycleReconciler::new();
    let (_actions, next_view) =
        r.reconcile(&ServiceLifecycleState::default(), &actual, &view, &tick);
    assert_eq!(
        next_view.startup_attempts_per_alloc.get(&aid("alloc-svc-g")).copied(),
        None,
        "an observed Pass must clear the consecutive-fail streak to 0 (entry removed)"
    );
}

/// (c)+(d) Boundary: with max_attempts = 3, three consecutive observed
/// Fails (past the deadline) reach the terminal at exactly the 3rd Fail
/// — and a Pass on the 2nd tick prevents it. This is the real never-binds
/// shape (max_attempts = 3) the GAP-10 busy-loop manifested on: the
/// terminal becomes reachable, flipping Shape B's predicate false.
#[test]
fn startup_probe_failed_reachable_at_exactly_max_and_prevented_by_pass() {
    let failing = fact(
        "alloc-svc-h",
        AllocState::Running,
        1_000,
        None,
        Some(ProbeStatus::Fail { last_fail_reason: "connection refused".to_string() }),
        3,
        Duration::from_secs(10),
    );
    // Past the deadline so the wall-clock gate is satisfied throughout.
    let tick = tick_at_ms(20_000); // elapsed 19_000 >= 10_000 deadline
    let r = ServiceLifecycleReconciler::new();
    let actual = one_alloc_state(failing.clone());

    // Fail 1 → attempts 1 < 3: no terminal.
    let (a1, v1) = r.reconcile(
        &ServiceLifecycleState::default(),
        &actual,
        &ServiceLifecycleView::default(),
        &tick,
    );
    assert!(a1.is_empty(), "1st Fail (attempts=1 < max=3) must not fire; got {a1:?}");

    // Fail 2 → attempts 2 < 3: no terminal.
    let (a2, v2) = r.reconcile(&ServiceLifecycleState::default(), &actual, &v1, &tick);
    assert!(a2.is_empty(), "2nd Fail (attempts=2 < max=3) must not fire; got {a2:?}");

    // Fail 3 → attempts 3 == 3: terminal fires at EXACTLY max, reports 3.
    let (a3, v3) = r.reconcile(&ServiceLifecycleState::default(), &actual, &v2, &tick);
    assert_eq!(a3.len(), 1, "3rd Fail (attempts == max) must fire StartupProbeFailed; got {a3:?}");
    match &a3[0] {
        Action::FinalizeFailed {
            terminal:
                Some(TerminalCondition::ServiceFailed {
                    reason: ServiceFailureReason::StartupProbeFailed { attempts, last_fail, .. },
                }),
            ..
        } => {
            assert_eq!(*attempts, 3, "reported attempts is the post-increment streak == max");
            assert_eq!(last_fail, "connection refused");
        }
        other => panic!("expected StartupProbeFailed at max, got {other:?}"),
    }
    // No-busy-loop proof: the alloc is now terminal, so the Shape B
    // predicate flips false — the runtime stops re-enqueueing.
    assert!(
        v3.terminal_announced.contains(&aid("alloc-svc-h")),
        "terminal must be recorded so view_has_backoff_pending returns false"
    );
    assert!(
        !v3.has_alloc_mid_startup_window(),
        "reachable terminal closes the busy-loop (predicate false)"
    );

    // (d) Prevention: a Pass on the 2nd tick clears the streak, so the
    // 3rd-tick Fail only reaches attempts == 1 — no terminal, alloc
    // recovers (Stable on the Pass tick because state == Running).
    let passing = fact(
        "alloc-svc-h",
        AllocState::Running,
        1_000,
        None,
        Some(ProbeStatus::Pass),
        3,
        Duration::from_secs(10),
    );
    let (_pa1, pv1) = r.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(failing),
        &ServiceLifecycleView::default(),
        &tick,
    );
    // Pass tick: counter reset to 0 AND state == Running + Pass ⇒ Stable.
    let (pa2, pv2) =
        r.reconcile(&ServiceLifecycleState::default(), &one_alloc_state(passing), &pv1, &tick);
    assert_eq!(pa2.len(), 1, "Pass on a Running alloc fires Stable; got {pa2:?}");
    assert!(
        matches!(
            &pa2[0],
            Action::FinalizeFailed { terminal: Some(TerminalCondition::Stable { .. }), .. }
        ),
        "Pass after one Fail recovers to Stable, NOT StartupProbeFailed; got {pa2:?}"
    );
    assert!(
        pv2.startup_attempts_per_alloc.get(&aid("alloc-svc-h")).copied().unwrap_or(0) == 0,
        "Pass cleared the streak so StartupProbeFailed is prevented"
    );
}

proptest! {
    /// Property: settled_in_ms (observable via Stable's TerminalCondition)
    /// equals now_ms.saturating_sub(started_at_ms). Kills mutants that
    /// replace the function body with 0 or 1: any test case where the
    /// true value is neither 0 nor 1 falsifies the mutant.
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
            None,
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
