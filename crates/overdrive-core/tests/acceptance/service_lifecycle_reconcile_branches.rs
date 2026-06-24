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
        // Step 03-01 — this startup-branch suite carries no readiness
        // probe; the readiness branch is a no-op (no service_dataplane).
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        backend_spiffe: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/x")
            .expect("valid spiffe"),
        backend_addr: std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 8080)),
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
        restart_count: 0,
        restart_spec: liveness_restart_spec_default(),
    }
}

/// Minimal `AllocationSpec` for `ServiceAllocFact.restart_spec` in
/// builders that never exercise the liveness restart branch.
fn liveness_restart_spec_default() -> overdrive_core::traits::driver::AllocationSpec {
    overdrive_core::traits::driver::AllocationSpec {
        alloc: aid("alloc-x"),
        identity: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/x")
            .expect("valid spiffe"),
        command: "/bin/svc".to_string(),
        args: vec![],
        resources: overdrive_core::traits::driver::Resources {
            cpu_milli: 100,
            memory_bytes: 64 * 1024 * 1024,
        },
        probe_descriptors: vec![],
        // transparent-mtls-enrollment step 04-01 (JOIN-4/JOIN-6): off the mTLS-composed boot gate.
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
    }
}

fn one_alloc_state(f: ServiceAllocFact) -> ServiceLifecycleState {
    let mut allocs = BTreeMap::new();
    allocs.insert(f.alloc_id.clone(), f);
    ServiceLifecycleState { allocs, service_dataplane: None }
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
            assert_eq!(*exit_code, Some(42), "exit_code must be propagated from fact.exit_code");
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

/// A service alloc that reaches `Failed` BEFORE ever starting
/// (`started_at == None` — driver spawn error, ENOMEM on fork, missing
/// exec binary) must be recorded in `terminal_announced` so the Shape B
/// predicate returns false for it. The reconciler does NOT classify
/// this alloc (the elapsed-vs-deadline `EarlyExit` reasoning needs a
/// `started_at`), so it emits zero actions — but it MUST still
/// acknowledge the terminal-set membership.
///
/// Without that membership the alloc stays in `observed` but in neither
/// terminal set, so `has_alloc_mid_startup_window` stays true forever.
/// Once WorkloadLifecycle finalises the alloc and the action shim
/// archives the observation row, the alloc vanishes from `actual.allocs`
/// — the reconcile loop can no longer re-touch it to clear the stale
/// `observed` entry — and the runtime's `view_has_backoff_pending` arm
/// re-enqueues the reconciler in a no-op busy-loop until the service is
/// deleted. This pins the predicate-false outcome that breaks the loop.
#[test]
fn pre_running_failed_alloc_is_terminal_and_not_mid_startup_window() {
    // Failed with `started_at == None` — the EarlyExit branch's
    // `let Some(started) = ... else { ... }` arm fires.
    let f = ServiceAllocFact {
        started_at: None,
        ..fact(
            "alloc-svc-prefail",
            AllocState::Failed,
            1_000,
            Some(127),
            None,
            30,
            Duration::from_secs(60),
        )
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
    assert!(
        !next_view.has_alloc_mid_startup_window(),
        "a pre-Running Failed alloc must NOT keep the runtime spinning (predicate false)"
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

// -------------------------------------------------------------------
// E. Readiness branch (Slice 04 / step 03-01) — Backend.healthy is
//    recomputed every tick from the readiness input + the live
//    success_threshold + the View consecutive-Pass counter. These
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
        alloc_id: aid(&format!("svc-{index}")),
        state: AllocState::Running,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
        exit_code: None,
        latest_startup_probe: Some(ProbeStatus::Pass),
        max_attempts: 30,
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        inferred: false,
        startup_probes_empty: false,
        latest_readiness_probe: latest_readiness,
        has_readiness_probe,
        readiness_success_threshold: success_threshold,
        backend_spiffe: overdrive_core::SpiffeId::new(&format!(
            "spiffe://overdrive.local/job/svc/alloc/a{index}"
        ))
        .expect("valid spiffe"),
        backend_addr: std::net::SocketAddr::from((
            std::net::Ipv4Addr::new(192, 168, 1, u8::try_from(10 + index).unwrap_or(u8::MAX)),
            8080,
        )),
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
        restart_count: 0,
        restart_spec: liveness_restart_spec_default(),
    }
}

fn readiness_dataplane() -> overdrive_core::service_lifecycle::ServiceDataplaneIdentity {
    overdrive_core::service_lifecycle::ServiceDataplaneIdentity {
        service_id: overdrive_core::id::ServiceId::new(7).expect("valid service id"),
        vip: overdrive_core::id::ServiceVip::new(std::net::IpAddr::V4(std::net::Ipv4Addr::new(
            10, 96, 0, 9,
        )))
        .expect("valid vip"),
        writer: overdrive_core::id::NodeId::new("node-r").expect("valid node id"),
    }
}

fn readiness_state(facts: Vec<ServiceAllocFact>) -> ServiceLifecycleState {
    let mut allocs = BTreeMap::new();
    for f in facts {
        allocs.insert(f.alloc_id.clone(), f);
    }
    ServiceLifecycleState { allocs, service_dataplane: Some(readiness_dataplane()) }
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
    // post-increment count reaches threshold. Pins the `>= threshold`
    // comparison AND the `(latest==Pass) && (counter>=threshold)`
    // conjunction (both operands vary independently). Kills `&&→||`
    // and `>=→<`.
    #[test]
    fn readiness_pass_below_threshold_is_unhealthy(
        seed in 0u32..=8,
        threshold in 2u32..=6,
    ) {
        let reconciler = ServiceLifecycleReconciler::new();
        let fact = readiness_fact(0, Some(ProbeStatus::Pass), true, threshold);
        let key = (fact.alloc_id.clone(), overdrive_core::observation::ProbeIdx::new(0));
        let mut view = ServiceLifecycleView::default();
        view.readiness_consecutive_successes.insert(key, seed);
        let state = readiness_state(vec![fact]);
        let (actions, _v) = reconciler.reconcile(&state, &state, &view, &readiness_tick(100));
        let expected = seed.saturating_add(1) >= threshold;
        prop_assert_eq!(single_backend_row_healthy(&actions), expected);
    }
}

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

#[test]
fn readiness_no_dataplane_identity_emits_no_row() {
    // The `service_dataplane: None` guard: without a VIP, the readiness
    // branch is a no-op (no WriteServiceBackendRow).
    let reconciler = ServiceLifecycleReconciler::new();
    let mut allocs = BTreeMap::new();
    let f = readiness_fact(0, Some(ProbeStatus::Pass), true, 1);
    allocs.insert(f.alloc_id.clone(), f);
    let state = ServiceLifecycleState { allocs, service_dataplane: None };
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
    restart_count: u32,
) -> ServiceAllocFact {
    ServiceAllocFact {
        alloc_id: aid(alloc_id),
        state,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
        exit_code: None,
        // No startup-probe observation — keeps the Stable / EarlyExit /
        // StartupProbeFailed branches inert so the liveness branch is
        // the only emitter under test (startup branches require a Pass
        // or a Failed state we do not set here for the Running cases).
        latest_startup_probe: None,
        max_attempts: u32::MAX, // never trips StartupProbeFailed
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        inferred: false,
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        backend_spiffe: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/x")
            .expect("valid spiffe"),
        backend_addr: std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 8080)),
        latest_liveness_probe,
        has_liveness_probe: true,
        liveness_failure_threshold: failure_threshold,
        restart_count,
        restart_spec: liveness_restart_spec_default(),
    }
}

/// Pre-seed the next-View's liveness counter so the post-update streak
/// reaches the desired value WITHOUT requiring N separate Fail ticks
/// (the reconciler increments once per tick on a Fail observation).
/// `seed` is the counter BEFORE this tick's Fail observation, so a
/// seed of `k` followed by one Fail observation yields `k + 1`.
fn view_with_liveness_counter(alloc_id: &str, seed: u32) -> ServiceLifecycleView {
    let mut v = ServiceLifecycleView::default();
    if seed > 0 {
        v.liveness_consecutive_failures
            .insert((aid(alloc_id), overdrive_core::observation::ProbeIdx::new(0)), seed);
    }
    v
}

proptest! {
    /// S-SHCP-RECON-09 — `LivenessExhaustionTriggersRestartAllocation`.
    ///
    /// Universe: (alloc state ∈ {Running, others}) × (observed
    /// consecutive_failures 1..=10) × (failure_threshold 1..=10) ×
    /// (restart_count 0..=RESTART_BACKOFF_CEILING). Invariant: when
    /// `state == Running AND consecutive_failures >= failure_threshold
    /// AND restart_count < RESTART_BACKOFF_CEILING`, reconcile emits
    /// EXACTLY ONE `RestartAllocation { reason: LivenessExhausted {
    /// probe_idx: 0, consecutive_failures: <observed>, threshold:
    /// <observed> } }`; otherwise zero RestartAllocation.
    #[test]
    fn liveness_exhaustion_triggers_restart_allocation(
        running in any::<bool>(),
        observed_failures in 1u32..=10,
        failure_threshold in 1u32..=10,
        restart_count in 0u32..=overdrive_core::reconcilers::RESTART_BACKOFF_CEILING,
    ) {
        let state = if running { AllocState::Running } else { AllocState::Pending };
        // Seed counter to `observed_failures - 1`; this tick's Fail
        // observation increments to exactly `observed_failures`.
        let view = view_with_liveness_counter("svc-live-0", observed_failures - 1);
        let fact = liveness_fact(
            "svc-live-0",
            state,
            Some(ProbeStatus::Fail { last_fail_reason: "liveness refused".to_string() }),
            failure_threshold,
            restart_count,
        );
        let recon = ServiceLifecycleReconciler::new();
        let (actions, _next) = recon.reconcile(
            &ServiceLifecycleState::default(),
            &one_alloc_state(fact),
            &view,
            &tick_at_ms(10_000),
        );

        let restarts: Vec<_> = actions
            .iter()
            .filter(|a| matches!(a, Action::RestartAllocation { .. }))
            .collect();

        let should_restart = running
            && observed_failures >= failure_threshold
            && restart_count < overdrive_core::reconcilers::RESTART_BACKOFF_CEILING;

        if should_restart {
            prop_assert_eq!(restarts.len(), 1, "exactly one RestartAllocation when triggered");
            match restarts[0] {
                Action::RestartAllocation { reason: Some(r), .. } => {
                    prop_assert_eq!(
                        r,
                        &overdrive_core::reconcilers::RestartReason::LivenessExhausted {
                            probe_idx: 0,
                            consecutive_failures: observed_failures,
                            threshold: failure_threshold,
                        },
                    );
                }
                other => prop_assert!(false, "expected LivenessExhausted reason, got {other:?}"),
            }
        } else {
            prop_assert_eq!(restarts.len(), 0, "no RestartAllocation when predicate false");
        }
    }

    /// S-SHCP-RECON-10 — `LivenessRecoveryResetsCounter`. A Pass
    /// observation (recovery) resets the next-View counter to 0 (the
    /// entry is removed; absence == 0) AND emits zero RestartAllocation,
    /// regardless of how high the prior streak was (Fail/Fail/Pass).
    #[test]
    fn liveness_recovery_resets_counter(
        prior_streak in 0u32..=10,
        failure_threshold in 1u32..=10,
    ) {
        let view = view_with_liveness_counter("svc-rec-0", prior_streak);
        let fact = liveness_fact(
            "svc-rec-0",
            AllocState::Running,
            Some(ProbeStatus::Pass),
            failure_threshold,
            0,
        );
        let recon = ServiceLifecycleReconciler::new();
        let (actions, next) = recon.reconcile(
            &ServiceLifecycleState::default(),
            &one_alloc_state(fact),
            &view,
            &tick_at_ms(10_000),
        );

        let key = (aid("svc-rec-0"), overdrive_core::observation::ProbeIdx::new(0));
        prop_assert_eq!(
            next.liveness_consecutive_failures.get(&key).copied().unwrap_or(0),
            0,
            "a Pass resets the consecutive-failure counter to 0",
        );
        let restarts = actions.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).count();
        prop_assert_eq!(restarts, 0, "recovery emits no RestartAllocation");
    }

    /// S-SHCP-RECON-11 — `BackoffExhaustionAfterRestartCeiling`. When
    /// `restart_count == RESTART_BACKOFF_CEILING` AND liveness
    /// re-triggers (Running + streak >= threshold), reconcile emits
    /// `FinalizeFailed { ServiceFailed { LivenessProbeFailed } }` so
    /// operators can distinguish liveness-driven backoff from crash-loop
    /// backoff — and NO further RestartAllocation.
    #[test]
    fn backoff_exhaustion_after_restart_ceiling(
        observed_failures in 3u32..=10,
        failure_threshold in 1u32..=3,
    ) {
        let view = view_with_liveness_counter("svc-ceil-0", observed_failures - 1);
        let fact = liveness_fact(
            "svc-ceil-0",
            AllocState::Running,
            Some(ProbeStatus::Fail { last_fail_reason: "liveness refused".to_string() }),
            failure_threshold,
            overdrive_core::reconcilers::RESTART_BACKOFF_CEILING,
        );
        let recon = ServiceLifecycleReconciler::new();
        let (actions, _next) = recon.reconcile(
            &ServiceLifecycleState::default(),
            &one_alloc_state(fact),
            &view,
            &tick_at_ms(10_000),
        );

        let restarts = actions.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).count();
        prop_assert_eq!(restarts, 0, "at ceiling, no further RestartAllocation");

        let service_failed = actions.iter().any(|a| matches!(
            a,
            Action::FinalizeFailed {
                terminal: Some(TerminalCondition::ServiceFailed {
                    reason: ServiceFailureReason::LivenessProbeFailed { .. },
                }),
                ..
            }
        ));
        prop_assert!(service_failed, "at ceiling, liveness re-trigger finalises ServiceFailed(LivenessProbeFailed); got {:?}", actions);
    }
}

// ===================================================================
// Step 03-02 / Slice 08 — EarlyExit branch correctness (US-08 / K1)
// ===================================================================

proptest! {
    /// S-SHCP-RECON-05 — `ExitAfterStableIsNotEarlyExit`. When an alloc
    /// was previously announced Stable (it sits in
    /// `view.stable_announced`), a later Failed transition does NOT
    /// re-classify as `EarlyExit` — the Stable-dedup guard short-
    /// circuits the per-alloc body before the EarlyExit branch. Zero
    /// `ServiceFailed { EarlyExit }` actions for ANY exit_code /
    /// elapsed combination.
    #[test]
    fn exit_after_stable_is_not_early_exit(
        exit_code in any::<i32>(),
        elapsed_ms in 0u64..=120_000,
    ) {
        let started_ms = 1_000u64;
        let mut view = ServiceLifecycleView::default();
        view.stable_announced.insert(aid("svc-poststable-0"));
        let f = fact(
            "svc-poststable-0",
            AllocState::Failed,
            started_ms,
            Some(exit_code),
            None,
            u32::MAX,
            Duration::from_secs(60),
        );
        let recon = ServiceLifecycleReconciler::new();
        let (actions, _next) = recon.reconcile(
            &ServiceLifecycleState::default(),
            &one_alloc_state(f),
            &view,
            &tick_at_ms(started_ms + elapsed_ms),
        );
        let early_exits = actions.iter().filter(|a| matches!(
            a,
            Action::FinalizeFailed {
                terminal: Some(TerminalCondition::ServiceFailed {
                    reason: ServiceFailureReason::EarlyExit { .. },
                }),
                ..
            }
        )).count();
        prop_assert_eq!(early_exits, 0, "an alloc already Stable never re-emits EarlyExit");
    }

    /// S-SHCP-RECON-06 — `ExitZeroWithinDeadlineIsStillEarlyExit`. A
    /// Service alloc that Failed with exit_code 0, within the startup
    /// deadline, with no startup Pass observed, IS classified
    /// `EarlyExit { exit_code: 0 }` — a Service expects to stay
    /// long-lived, so even a clean exit before any probe passes is an
    /// early-exit failure (the RCA-A coinflip case). Pinned across
    /// every within-deadline elapsed value.
    #[test]
    fn exit_zero_within_deadline_is_still_early_exit(
        elapsed_ms in 0u64..59_000,
    ) {
        let started_ms = 1_000u64;
        let f = fact(
            "svc-exit0-0",
            AllocState::Failed,
            started_ms,
            Some(0),  // clean exit
            None,     // no startup Pass observed
            u32::MAX, // never trips StartupProbeFailed
            Duration::from_secs(60),
        );
        let recon = ServiceLifecycleReconciler::new();
        let (actions, _next) = recon.reconcile(
            &ServiceLifecycleState::default(),
            &one_alloc_state(f),
            &ServiceLifecycleView::default(),
            &tick_at_ms(started_ms + elapsed_ms),
        );
        let early_exit_zero = actions.iter().any(|a| matches!(
            a,
            Action::FinalizeFailed {
                terminal: Some(TerminalCondition::ServiceFailed {
                    reason: ServiceFailureReason::EarlyExit { exit_code: Some(0) },
                }),
                ..
            }
        ));
        prop_assert!(early_exit_zero, "exit-0-within-deadline IS EarlyExit{{exit_code:0}}; got {actions:?}");
    }
}

// ===================================================================
// Regression — signal-killed alloc must carry None exit_code
// ===================================================================

/// Before this fix, `unwrap_or(0)` mapped `None` → `0`, making a
/// SIGKILL look like a clean exit to operators.
#[test]
fn signal_killed_alloc_carries_none_exit_code() {
    let f = fact(
        "svc-sigkill-0",
        AllocState::Failed,
        1_000,
        None, // signal-killed — no exit code
        None, // no startup Pass observed
        u32::MAX,
        Duration::from_secs(60),
    );
    let recon = ServiceLifecycleReconciler::new();
    let (actions, _next) = recon.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(f),
        &ServiceLifecycleView::default(),
        &tick_at_ms(2_000),
    );
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        Action::FinalizeFailed {
            terminal:
                Some(TerminalCondition::ServiceFailed {
                    reason: ServiceFailureReason::EarlyExit { exit_code },
                }),
            ..
        } => {
            assert_eq!(
                *exit_code, None,
                "signal-killed alloc must carry exit_code: None, not Some(0)"
            );
        }
        other => panic!("expected EarlyExit with None exit_code, got {other:?}"),
    }
}

// ===================================================================
// Regression — startup × liveness dual-action in one tick
// ===================================================================

/// A Running alloc past its startup deadline with a failing startup
/// probe AND a liveness probe that has accumulated enough consecutive
/// failures MUST emit only the `StartupProbeFailed` terminal — the
/// liveness loop must skip allocs already given a terminal in the
/// startup loop on the same tick.
///
/// Without the `terminal_announced` guard in the liveness loop, BOTH
/// `FinalizeFailed { StartupProbeFailed }` AND `RestartAllocation {
/// LivenessExhausted }` fire for the same alloc, producing two
/// conflicting actions.
#[test]
fn startup_probe_failed_suppresses_liveness_restart_same_tick() {
    let id = "svc-dual-0";
    // Fact: Running, startup probe Fail, past deadline, AND liveness
    // probe Fail with threshold about to trip.
    let f = ServiceAllocFact {
        alloc_id: aid(id),
        state: AllocState::Running,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
        exit_code: None,
        latest_startup_probe: Some(ProbeStatus::Fail {
            last_fail_reason: "conn refused".to_string(),
        }),
        max_attempts: 1,
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        inferred: false,
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        backend_spiffe: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/x")
            .expect("valid spiffe"),
        backend_addr: std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 8080)),
        latest_liveness_probe: Some(ProbeStatus::Fail {
            last_fail_reason: "liveness refused".to_string(),
        }),
        has_liveness_probe: true,
        liveness_failure_threshold: 3,
        restart_count: 0,
        restart_spec: liveness_restart_spec_default(),
    };

    // Pre-seed liveness counter to 2; this tick's Fail bumps it to 3
    // which meets the threshold=3 gate in liveness_restart_action.
    let view = view_with_liveness_counter(id, 2);

    let recon = ServiceLifecycleReconciler::new();
    let (actions, next) = recon.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(f),
        &view,
        // 61s after started_at=1s → elapsed=60s >= deadline=60s
        &tick_at_ms(61_000),
    );

    // The startup loop fires StartupProbeFailed and inserts into
    // terminal_announced.
    let startup_failed = actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                Action::FinalizeFailed {
                    terminal: Some(TerminalCondition::ServiceFailed {
                        reason: ServiceFailureReason::StartupProbeFailed { .. },
                    }),
                    ..
                }
            )
        })
        .count();
    assert_eq!(startup_failed, 1, "exactly one StartupProbeFailed terminal");

    // The liveness loop must NOT emit a second action for the same alloc.
    let restarts = actions.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).count();
    assert_eq!(restarts, 0, "liveness must not restart an alloc already given a startup terminal");

    let backoff_exhausted = actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                Action::FinalizeFailed {
                    terminal: Some(TerminalCondition::BackoffExhausted { .. }),
                    ..
                }
            )
        })
        .count();
    assert_eq!(backoff_exhausted, 0, "no BackoffExhausted either");

    assert_eq!(actions.len(), 1, "exactly one action total (StartupProbeFailed only)");
    assert!(next.terminal_announced.contains(&aid(id)), "alloc recorded in terminal_announced");
}

/// Regression: `readiness_backend_row_action` must suppress
/// `WriteServiceBackendRow` when the backend set is unchanged
/// between ticks. Without dedup, each tick emits a row with a
/// monotonically increasing `LogicalTimestamp`, causing unnecessary
/// LWW gossip propagation at tick rate.
#[test]
fn readiness_branch_suppresses_write_on_unchanged_backends() {
    let reconciler = ServiceLifecycleReconciler::new();
    let state = readiness_state(vec![readiness_fact(0, Some(ProbeStatus::Pass), true, 1)]);

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

    // Second tick — same inputs, prior view fed back. Expect NO emission.
    let (actions_second, _view_after_second) =
        reconciler.reconcile(&state, &state, &view_after_first, &readiness_tick(200));
    let row_count_second = actions_second
        .iter()
        .filter(|a| matches!(a, Action::WriteServiceBackendRow { .. }))
        .count();
    assert_eq!(
        row_count_second, 0,
        "second tick with unchanged backends must suppress WriteServiceBackendRow"
    );
}

/// Regression complement: when a backend's `healthy` flag changes
/// between ticks (readiness probe transitions Pass → Fail), the
/// fingerprint changes and the write MUST be emitted.
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
    let state_unhealthy = readiness_state(vec![readiness_fact(
        0,
        Some(ProbeStatus::Fail { last_fail_reason: "conn refused".to_string() }),
        true,
        1,
    )]);
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

// ===================================================================
// Regression: liveness BackoffExhausted dedup via terminal_announced
// ===================================================================

/// Regression test for liveness `FinalizeFailed { ServiceFailed {
/// LivenessProbeFailed } }` duplicate emission. When liveness exhausts
/// the restart budget on tick 1, the alloc must be recorded in
/// `terminal_announced` so that tick 2 does NOT re-emit the same
/// terminal action. Without the fix, `terminal_announced` is never
/// populated by the liveness loop, and the terminal re-fires every tick
/// until the alloc state changes to non-Running (inconsistent with the
/// startup path's dedup discipline).
#[test]
fn liveness_backoff_exhausted_does_not_re_emit_on_second_tick() {
    let id = "svc-liveness-dedup-0";

    // Tick 1: Running alloc with liveness probe Fail, streak at
    // threshold, restart budget exhausted → BackoffExhausted.
    let fact_tick1 = liveness_fact(
        id,
        AllocState::Running,
        Some(ProbeStatus::Fail { last_fail_reason: "liveness refused".to_string() }),
        3, // failure_threshold
        overdrive_core::reconcilers::RESTART_BACKOFF_CEILING,
    );
    let view_tick1 = view_with_liveness_counter(id, 2); // seed=2, +1 Fail → 3 >= threshold

    let recon = ServiceLifecycleReconciler::new();
    let (actions_tick1, next_view_tick1) = recon.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(fact_tick1),
        &view_tick1,
        &tick_at_ms(10_000),
    );

    // Tick 1 must emit exactly one ServiceFailed { LivenessProbeFailed }.
    let terminal_count_tick1 = actions_tick1
        .iter()
        .filter(|a| {
            matches!(
                a,
                Action::FinalizeFailed {
                    terminal: Some(TerminalCondition::ServiceFailed {
                        reason: ServiceFailureReason::LivenessProbeFailed { .. },
                    }),
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        terminal_count_tick1, 1,
        "tick 1 emits exactly one ServiceFailed(LivenessProbeFailed)"
    );
    assert!(
        next_view_tick1.terminal_announced.contains(&aid(id)),
        "tick 1 must insert alloc into terminal_announced for dedup",
    );

    // Tick 2: same alloc still Running (action not yet applied by the
    // runtime). Feed tick 1's returned view back in — the dedup guard
    // must suppress re-emission.
    let fact_tick2 = liveness_fact(
        id,
        AllocState::Running,
        Some(ProbeStatus::Fail { last_fail_reason: "liveness refused".to_string() }),
        3,
        overdrive_core::reconcilers::RESTART_BACKOFF_CEILING,
    );

    let (actions_tick2, _next_view_tick2) = recon.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(fact_tick2),
        &next_view_tick1,
        &tick_at_ms(10_100),
    );

    let terminal_count_tick2 = actions_tick2
        .iter()
        .filter(|a| {
            matches!(
                a,
                Action::FinalizeFailed {
                    terminal: Some(TerminalCondition::ServiceFailed {
                        reason: ServiceFailureReason::LivenessProbeFailed { .. },
                    }),
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        terminal_count_tick2, 0,
        "tick 2 must NOT re-emit ServiceFailed — terminal_announced dedup must suppress it",
    );
    assert_eq!(
        actions_tick2.len(),
        0,
        "tick 2 emits zero actions for an alloc already given a terminal",
    );
}

/// Regression: when the startup probe passes (Stable) on the same tick
/// that liveness has accumulated enough consecutive failures to trip
/// `RestartAllocation`, the liveness loop must skip the alloc because
/// `stable_announced` already recorded it. Without the guard, both
/// `FinalizeFailed { Stable }` AND `RestartAllocation { LivenessExhausted }`
/// land in the same actions Vec — contradictory actions for one alloc.
#[test]
fn stable_announced_suppresses_liveness_restart_same_tick() {
    let id = "svc-stable-liveness-0";
    // Fact: Running, startup probe Pass (triggers Stable in the first
    // loop), AND liveness probe Fail with pre-seeded streak about to
    // trip the threshold.
    let f = ServiceAllocFact {
        alloc_id: aid(id),
        state: AllocState::Running,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
        exit_code: None,
        latest_startup_probe: Some(ProbeStatus::Pass),
        max_attempts: u32::MAX,
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        inferred: false,
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        backend_spiffe: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/x")
            .expect("valid spiffe"),
        backend_addr: std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 8080)),
        latest_liveness_probe: Some(ProbeStatus::Fail {
            last_fail_reason: "liveness refused".to_string(),
        }),
        has_liveness_probe: true,
        liveness_failure_threshold: 3,
        restart_count: 0,
        restart_spec: liveness_restart_spec_default(),
    };

    // Pre-seed liveness counter to 2; this tick's Fail bumps it to 3
    // which meets the threshold=3 gate in liveness_restart_action.
    let view = view_with_liveness_counter(id, 2);

    let recon = ServiceLifecycleReconciler::new();
    let (actions, next) = recon.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(f),
        &view,
        &tick_at_ms(10_000),
    );

    // The startup loop fires Stable and inserts into stable_announced.
    let stable_count = actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                Action::FinalizeFailed { terminal: Some(TerminalCondition::Stable { .. }), .. }
            )
        })
        .count();
    assert_eq!(stable_count, 1, "exactly one Stable terminal");

    // The liveness loop must NOT emit a restart for the same alloc.
    let restarts = actions.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).count();
    assert_eq!(restarts, 0, "liveness must not restart an alloc already announced Stable");

    assert_eq!(actions.len(), 1, "exactly one action total (Stable only)");
    assert!(next.stable_announced.contains(&aid(id)), "alloc recorded in stable_announced");
}

/// Regression: once an alloc entered `stable_announced` (after passing
/// its startup probe), the liveness loop permanently skipped it —
/// `stable_announced` persists across ticks and was never cleared. The
/// guard was meant to prevent same-tick double-emission only, but the
/// persistent set made the suppression permanent. On the NEXT tick after
/// Stable, liveness must resume monitoring and emit `RestartAllocation`
/// when the threshold is met.
#[test]
fn liveness_resumes_after_prior_tick_stable_announcement() {
    let id = "svc-post-stable-0";
    // Fact: Running, startup probe still Pass (Stable was already
    // announced on a prior tick), AND liveness probe Fail with
    // pre-seeded streak about to trip the threshold.
    let f = ServiceAllocFact {
        alloc_id: aid(id),
        state: AllocState::Running,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
        exit_code: None,
        latest_startup_probe: Some(ProbeStatus::Pass),
        max_attempts: u32::MAX,
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        inferred: false,
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        backend_spiffe: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/x")
            .expect("valid spiffe"),
        backend_addr: std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 8080)),
        latest_liveness_probe: Some(ProbeStatus::Fail {
            last_fail_reason: "liveness refused".to_string(),
        }),
        has_liveness_probe: true,
        liveness_failure_threshold: 3,
        restart_count: 0,
        restart_spec: liveness_restart_spec_default(),
    };

    // Pre-seed liveness counter to 2; this tick's Fail bumps it to 3
    // which meets the threshold=3 gate in liveness_restart_action.
    let mut view = view_with_liveness_counter(id, 2);
    // Simulate prior tick: alloc already announced Stable.
    view.stable_announced.insert(aid(id));
    view.observed.insert(aid(id));

    let recon = ServiceLifecycleReconciler::new();
    let (actions, _next) = recon.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(f),
        &view,
        &tick_at_ms(20_000),
    );

    // The startup loop skips this alloc (already in stable_announced —
    // correct dedup). The liveness loop MUST still evaluate it and emit
    // RestartAllocation because the liveness threshold is met.
    let restarts: Vec<_> =
        actions.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).collect();
    assert_eq!(
        restarts.len(),
        1,
        "liveness must emit RestartAllocation for a post-Stable alloc when threshold is met, got actions: {actions:?}",
    );

    // No Stable re-emission (the startup loop's stable_announced dedup
    // is correct and should still fire).
    let stable_count = actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                Action::FinalizeFailed { terminal: Some(TerminalCondition::Stable { .. }), .. }
            )
        })
        .count();
    assert_eq!(stable_count, 0, "Stable must not be re-emitted on subsequent ticks");
}

/// Regression: a Failed alloc with no readiness probe was emitted as
/// `Backend { healthy: true }` because `readiness_backend_row_action`
/// iterated all allocs without a state guard. Only Running allocs can
/// serve traffic; non-Running allocs must be excluded from the backend
/// set entirely.
#[test]
fn failed_alloc_excluded_from_backend_row() {
    let reconciler = ServiceLifecycleReconciler::new();
    // Failed alloc, no readiness probe → before the fix,
    // compute_backend_healthy returns true (backward-compat default).
    let mut f = readiness_fact(0, None, false, 1);
    f.state = AllocState::Failed;
    f.exit_code = Some(1);
    f.started_at = Some(UnixInstant::from_unix_duration(Duration::from_secs(1)));
    f.latest_startup_probe = None;
    let state = readiness_state(vec![f]);

    let mut view = ServiceLifecycleView::default();
    // Mark as terminal so the startup loop skips it (mirrors real
    // runtime: the previous tick already announced the failure).
    view.terminal_announced.insert(aid("svc-0"));

    let (actions, _v) = reconciler.reconcile(&state, &state, &view, &readiness_tick(5_000));

    // No WriteServiceBackendRow should be emitted — the only alloc is
    // Failed and cannot serve traffic.
    let backend_rows: Vec<_> =
        actions.iter().filter(|a| matches!(a, Action::WriteServiceBackendRow { .. })).collect();
    assert!(
        backend_rows.is_empty(),
        "Failed alloc must not appear in backend row, got {backend_rows:?}",
    );
}

/// Regression: a Pending alloc with no readiness probe was emitted as
/// `Backend { healthy: true }` — same root cause as the Failed case.
/// Pending allocs have not started yet and cannot serve traffic.
#[test]
fn pending_alloc_excluded_from_backend_row() {
    let reconciler = ServiceLifecycleReconciler::new();
    let mut f = readiness_fact(0, None, false, 1);
    f.state = AllocState::Pending;
    f.started_at = None;
    f.latest_startup_probe = None;
    let state = readiness_state(vec![f]);

    let (actions, _v) = reconciler.reconcile(
        &state,
        &state,
        &ServiceLifecycleView::default(),
        &readiness_tick(5_000),
    );

    let backend_rows: Vec<_> =
        actions.iter().filter(|a| matches!(a, Action::WriteServiceBackendRow { .. })).collect();
    assert!(
        backend_rows.is_empty(),
        "Pending alloc must not appear in backend row, got {backend_rows:?}",
    );
}

/// Regression: liveness consecutive-failure counter not reset after
/// `RestartAllocation`. Without the fix, when the restarted alloc
/// reaches Running with `latest_liveness_probe == None` (probes
/// haven't fired yet), the stale counter exceeds the threshold and
/// fires another `RestartAllocation` — one restart per tick until
/// `BackoffExhausted`.
///
/// Tick 1: Running, Fail observation pushes counter to threshold →
///         `RestartAllocation` emitted, counter must be cleared.
/// Tick 2: Alloc restarted (Running, restart_count += 1), probe is
///         `None` → no action (counter was cleared).
#[test]
fn liveness_counter_reset_after_restart_prevents_spurious_retrigger() {
    let id = "svc-liveness-reset-0";
    let threshold: u32 = 3;

    // Tick 1: seed counter to threshold - 1, plus one Fail → hits threshold.
    let fact_tick1 = liveness_fact(
        id,
        AllocState::Running,
        Some(ProbeStatus::Fail { last_fail_reason: "liveness tcp refused".to_string() }),
        threshold,
        0, // restart_count
    );
    let view_tick1 = view_with_liveness_counter(id, threshold - 1);

    let recon = ServiceLifecycleReconciler::new();
    let (actions_tick1, next_view_tick1) = recon.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(fact_tick1),
        &view_tick1,
        &tick_at_ms(10_000),
    );

    let restart_count_tick1 =
        actions_tick1.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).count();
    assert_eq!(restart_count_tick1, 1, "tick 1 must emit exactly one RestartAllocation");

    // The fix: the counter entry must have been removed after restart.
    let key = (aid(id), overdrive_core::observation::ProbeIdx::new(0));
    assert_eq!(
        next_view_tick1.liveness_consecutive_failures.get(&key),
        None,
        "counter must be cleared after RestartAllocation so the post-restart alloc starts fresh",
    );

    // Tick 2: alloc restarted — Running again with restart_count += 1.
    // No probe results yet (None). With the fix, the counter is 0 and
    // no action fires. Without the fix, the stale counter >=
    // threshold immediately re-triggers RestartAllocation.
    let fact_tick2 = liveness_fact(
        id,
        AllocState::Running,
        None, // probes haven't fired yet after restart
        threshold,
        1, // restart_count incremented by runtime
    );

    let (actions_tick2, _next_view_tick2) = recon.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(fact_tick2),
        &next_view_tick1,
        &tick_at_ms(10_100),
    );

    let restart_count_tick2 =
        actions_tick2.iter().filter(|a| matches!(a, Action::RestartAllocation { .. })).count();
    assert_eq!(
        restart_count_tick2, 0,
        "tick 2 must NOT fire RestartAllocation — counter was reset, probes haven't reported yet",
    );
}

/// Regression: liveness budget exhaustion must emit
/// `ServiceFailed { LivenessProbeFailed }`, NOT `BackoffExhausted`.
///
/// Before the fix, `liveness_restart_action` emitted
/// `TerminalCondition::BackoffExhausted` — the same terminal the
/// crash-loop pathway uses. The streaming projection hard-coded
/// `BackoffCause::AttemptBudget` for every `BackoffExhausted`, so the
/// operator's CLI rendered "attempt budget (crash loop)" instead of the
/// liveness-specific "liveness probe failed after N attempts".
///
/// `ServiceFailureReason::LivenessProbeFailed` was defined and handled
/// by both the streaming projection and the CLI render, but never
/// emitted. This test pins the correct terminal.
#[test]
fn liveness_budget_exhausted_emits_service_failed_not_backoff_exhausted() {
    let id = "svc-liveness-terminal-0";
    let threshold: u32 = 3;

    // Running alloc with liveness Fail, streak at threshold,
    // restart budget exhausted → must emit ServiceFailed.
    let fact = liveness_fact(
        id,
        AllocState::Running,
        Some(ProbeStatus::Fail { last_fail_reason: "liveness tcp refused".to_string() }),
        threshold,
        overdrive_core::reconcilers::RESTART_BACKOFF_CEILING,
    );
    // Seed counter to threshold - 1; one Fail this tick → threshold reached.
    let view = view_with_liveness_counter(id, threshold - 1);

    let recon = ServiceLifecycleReconciler::new();
    let (actions, _next_view) = recon.reconcile(
        &ServiceLifecycleState::default(),
        &one_alloc_state(fact),
        &view,
        &tick_at_ms(10_000),
    );

    // Must emit exactly one FinalizeFailed.
    let terminals: Vec<_> = actions
        .iter()
        .filter_map(|a| match a {
            Action::FinalizeFailed { terminal, .. } => terminal.as_ref(),
            _ => None,
        })
        .collect();
    assert_eq!(terminals.len(), 1, "exactly one terminal action expected");

    // The terminal MUST be ServiceFailed { LivenessProbeFailed },
    // NOT BackoffExhausted.
    match terminals[0] {
        TerminalCondition::ServiceFailed {
            reason: ServiceFailureReason::LivenessProbeFailed { probe_idx, attempts },
        } => {
            assert_eq!(*probe_idx, 0, "probe_idx must be 0");
            assert_eq!(*attempts, threshold, "attempts must equal consecutive failures");
        }
        TerminalCondition::BackoffExhausted { .. } => {
            panic!(
                "BUG: liveness budget exhaustion emitted BackoffExhausted \
                 (crash-loop terminal) instead of ServiceFailed {{ LivenessProbeFailed }}"
            );
        }
        other => {
            panic!("unexpected terminal condition: {other:?}");
        }
    }
}
