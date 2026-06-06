//! Slice 01 / US-WP-3 AC4 — the workflow-lifecycle reconciler re-hydrates
//! a running instance from `Action::StartWorkflow` on restart.
//!
//! Scenario S-WP-01-08. O2 (single-node). An instance that is `running`
//! in intent but has no live engine task after a process restart: the
//! pure-sync `reconcile` re-emits `Action::StartWorkflow { spec,
//! correlation }` for it (the engine's `load_journal` then RESUMES rather
//! than cold-starts), and the `reconcile` body performs no `.await` — the
//! `ReconcilerIsPure` DST invariant continues to hold with the
//! workflow-lifecycle reconciler registered alongside the existing
//! reconcilers. ADR-0064 §5. The shim-dispatch boundary (engine driven
//! off the shim, not run as a reconciler) is the sibling scenario
//! S-WP-01-11.
//!
//! # Port-to-port
//!
//! The driving port is the workflow-lifecycle reconciler's pure `reconcile`
//! signature (`AnyReconciler::reconcile`) — a pure domain function IS its
//! own driving port. The observable outcome is the returned
//! `Vec<Action>`: a running-in-intent instance with no live engine task
//! re-emits `Action::StartWorkflow` carrying the SAME `CorrelationKey` the
//! engine files its terminal row under. Purity (`ReconcilerIsPure`) is
//! asserted directly by twin-invocation: two calls with identical inputs
//! produce bit-identical `(actions, next_view)` — the runtime witness that
//! the reconcile body holds no `.await` and reads no wall-clock / RNG.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::id::{ContentHash, CorrelationKey};
use overdrive_core::reconcilers::{
    Action, AnyReconciler, AnyReconcilerView, AnyState, TickContext, WorkflowInstanceState,
    WorkflowLifecycle, WorkflowLifecycleState, WorkflowLifecycleView,
};
use overdrive_core::workflow::{WorkflowName, WorkflowResult, WorkflowSpec};

fn fresh_tick(now: Instant) -> TickContext {
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    }
}

fn provision_spec() -> WorkflowSpec {
    WorkflowSpec {
        name: WorkflowName::new("provision-record").expect("valid workflow name"),
        input: Vec::new(),
    }
}

fn correlation_for(spec: &WorkflowSpec) -> CorrelationKey {
    CorrelationKey::derive(
        "wf-provision-0001",
        &ContentHash::of(spec.name.as_str().as_bytes()),
        "start-workflow",
    )
}

#[test]
fn lifecycle_reconciler_re_emits_start_workflow_for_a_running_instance_with_no_live_task() {
    let reconciler = AnyReconciler::WorkflowLifecycle(WorkflowLifecycle::canonical());

    // Desired: the instance is running-in-intent. Actual (after a process
    // restart): the engine task is GONE (`has_live_task == false`) and no
    // terminal row has been observed. This is the crash-resume shape —
    // the reconciler does NOT know cold-start vs crash-resume; it simply
    // re-emits StartWorkflow, and the engine's `load_journal` decides.
    let spec = provision_spec();
    let correlation = correlation_for(&spec);
    let mut instances = std::collections::BTreeMap::new();
    instances.insert(
        correlation.clone(),
        WorkflowInstanceState {
            spec: spec.clone(),
            running_in_intent: true,
            has_live_task: false,
            terminal: None,
        },
    );
    let desired = AnyState::WorkflowLifecycle(WorkflowLifecycleState { instances });
    let actual = desired.clone();
    let view = AnyReconcilerView::WorkflowLifecycle(WorkflowLifecycleView::default());
    let tick = fresh_tick(Instant::now());

    // Driving port: pure reconcile. Twin invocation pins ReconcilerIsPure
    // (AC5) — bit-identical output proves no `.await`, no wall-clock, no RNG.
    let (actions_a, view_a) = reconciler.reconcile(&desired, &actual, &view, &tick);
    let (actions_b, view_b) = reconciler.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        actions_a, actions_b,
        "ReconcilerIsPure: workflow-lifecycle twin invocations must produce bit-identical actions; \
         got first={actions_a:?}, second={actions_b:?}"
    );
    assert_eq!(
        view_a, view_b,
        "ReconcilerIsPure: workflow-lifecycle twin invocations must produce bit-identical NextView; \
         got first={view_a:?}, second={view_b:?}"
    );

    // Observable outcome: exactly one StartWorkflow, carrying the SAME
    // correlation key the terminal row is filed under (ADR-0064 §2).
    let start_workflows: Vec<&Action> =
        actions_a.iter().filter(|a| matches!(a, Action::StartWorkflow { .. })).collect();
    assert_eq!(
        start_workflows.len(),
        1,
        "a running-in-intent instance with no live engine task must re-emit exactly one \
         StartWorkflow on restart; got {actions_a:?}"
    );
    match start_workflows[0] {
        Action::StartWorkflow { spec: emitted_spec, correlation: emitted_corr } => {
            assert_eq!(
                *emitted_spec, spec,
                "re-emitted spec must match the running instance's spec"
            );
            assert_eq!(
                *emitted_corr, correlation,
                "re-emitted StartWorkflow must carry the SAME CorrelationKey the terminal row is \
                 filed under (ADR-0064 §2 correlation linkage)"
            );
        }
        other => panic!("expected StartWorkflow, got {other:?}"),
    }
}

#[test]
fn lifecycle_reconciler_is_noop_when_instance_has_a_live_task() {
    let reconciler = AnyReconciler::WorkflowLifecycle(WorkflowLifecycle::canonical());
    let spec = provision_spec();
    let correlation = correlation_for(&spec);
    let mut instances = std::collections::BTreeMap::new();
    instances.insert(
        correlation,
        WorkflowInstanceState {
            spec,
            running_in_intent: true,
            has_live_task: true,
            terminal: None,
        },
    );
    let desired = AnyState::WorkflowLifecycle(WorkflowLifecycleState { instances });
    let actual = desired.clone();
    let view = AnyReconcilerView::WorkflowLifecycle(WorkflowLifecycleView::default());
    let tick = fresh_tick(Instant::now());

    let (actions, _next) = reconciler.reconcile(&desired, &actual, &view, &tick);
    assert!(
        actions.iter().all(|a| matches!(a, Action::Noop)),
        "a running instance with a live engine task must NOT re-emit StartWorkflow; got {actions:?}"
    );
}

#[test]
fn lifecycle_reconciler_converges_on_observed_terminal() {
    let reconciler = AnyReconciler::WorkflowLifecycle(WorkflowLifecycle::canonical());
    let spec = provision_spec();
    let correlation = correlation_for(&spec);
    let mut instances = std::collections::BTreeMap::new();
    instances.insert(
        correlation,
        WorkflowInstanceState {
            spec,
            running_in_intent: true,
            has_live_task: false,
            terminal: Some(WorkflowResult::Success),
        },
    );
    let desired = AnyState::WorkflowLifecycle(WorkflowLifecycleState { instances });
    let actual = desired.clone();
    let view = AnyReconcilerView::WorkflowLifecycle(WorkflowLifecycleView::default());
    let tick = fresh_tick(Instant::now());

    let (actions, _next) = reconciler.reconcile(&desired, &actual, &view, &tick);
    assert!(
        actions.iter().all(|a| matches!(a, Action::Noop)),
        "an instance with an observed terminal result is converged — no StartWorkflow re-emit; \
         got {actions:?}"
    );
}
