//! The workflow-lifecycle reconciler (ADR-0064 §5).
//!
//! # Extension Justification
//!
//! WHY-NEW-FILE: crates/overdrive-core/src/reconcilers/workflow_lifecycle.rs
//!   CLOSEST-EXISTING: crates/overdrive-core/src/reconcilers/workload_lifecycle.rs
//!   EXTENSION-COST: `WorkloadLifecycle` is a 1000+ LOC reconciler with its
//!     own `State`/`View` (allocation scheduling, restart budgets, service
//!     VIP release); the workflow reconciler has a disjoint `State`
//!     (workflow *instances*, not allocations), a disjoint `View`, and a
//!     different action surface (`StartWorkflow`, not `StartAllocation`).
//!   PARALLEL-RATIONALE: the two reconcilers have incompatible associated
//!     types (`State`/`View`) and model different §18 primitives (workflow
//!     instances vs allocations) — co-locating would force one struct's
//!     `reconcile` to branch on two unrelated state shapes, which the
//!     `AnyReconciler` enum-dispatch exists specifically to keep separate.
//!
//! # The two-primitive doctrine (R3, ADR-0064 §5)
//!
//! This reconciler manages WHICH workflow instances should exist; the
//! [`WorkflowEngine`](../../../overdrive_control_plane/workflow_runtime/struct.WorkflowEngine.html)
//! manages HOW each instance's steps execute. The reconciler is pure-sync
//! — it emits [`Action::StartWorkflow`], observes terminal rows, and NEVER
//! `.await`s the workflow body. On restart, a running-in-intent instance
//! with no live engine task is re-emitted as `StartWorkflow`; the engine's
//! `load_journal` then RESUMES rather than cold-starts (the reconciler does
//! NOT know cold-start vs crash-resume — it keeps `reconcile` clean & pure
//! by deferring that decision to the engine).
//!
//! The terminal row the engine writes is keyed by the instance
//! [`CorrelationKey`]; the `StartWorkflow` this reconciler emits carries
//! the SAME key (`development.md` Reconciler I/O rule 2 — correlation, not
//! request ID, links cause to response).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::id::CorrelationKey;
use crate::workflow::{WorkflowStart, WorkflowStatus};

use super::{Action, Reconciler, ReconcilerName, TickContext};

/// Per-instance projection of a workflow's lifecycle state, keyed in
/// [`WorkflowLifecycleState::instances`] by the instance
/// [`CorrelationKey`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowInstanceState {
    /// The durable start intent the engine resolves to a registered
    /// `Workflow` factory. Re-emitted verbatim on `StartWorkflow` so the
    /// engine drives the same workflow kind on resume.
    pub spec: WorkflowStart,
    /// Whether intent says this instance should be running. Derived by
    /// the runtime's hydrate-desired from the intent SSOT; a `false`
    /// value means the instance is not desired and the reconciler does
    /// nothing for it.
    pub running_in_intent: bool,
    /// Whether the engine currently holds a live task for this instance.
    /// Derived by the runtime's hydrate-actual from the engine's tracked
    /// task set. After a process restart this is `false` for every
    /// previously-running instance — the trigger for the re-emit.
    pub has_live_task: bool,
    /// The observed terminal status, if the engine has written a
    /// `WorkflowTerminal` observation row for this instance. `Some(_)`
    /// means the instance has converged — no re-emit.
    pub terminal: Option<WorkflowStatus>,
}

/// `desired`/`actual` projection for the workflow-lifecycle reconciler.
///
/// Both `desired` and `actual` are the same shape; the runtime populates
/// `running_in_intent` from the intent SSOT (desired) and `has_live_task`
/// / `terminal` from the engine + observation store (actual). The
/// reconcile body reads the merged view via `actual`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkflowLifecycleState {
    /// Per-instance state, keyed by instance [`CorrelationKey`].
    /// `BTreeMap` for deterministic iteration across seeds per
    /// `.claude/rules/development.md` § "Ordered-collection choice" —
    /// the reconcile body iterates this map, so order is observable.
    pub instances: BTreeMap<CorrelationKey, WorkflowInstanceState>,
}

/// Typed memory for the workflow-lifecycle reconciler.
///
/// Phase 1 carries no memory — the re-emit decision is a pure function of
/// `actual` (running-in-intent + no-live-task + no-terminal). The struct
/// exists for the `Reconciler::View` associated-type contract and grows
/// additively when a retry/budget policy lands (per `development.md`
/// § "Persist inputs, not derived state": persist inputs, recompute
/// deadlines).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WorkflowLifecycleView {}

/// The workflow-lifecycle reconciler.
pub struct WorkflowLifecycle {
    name: ReconcilerName,
}

impl WorkflowLifecycle {
    /// Construct the canonical `workflow-lifecycle` instance.
    ///
    /// # Panics
    ///
    /// Never — `Self::NAME` is a compile-time string literal satisfying
    /// every `ReconcilerName` validation rule.
    #[must_use]
    pub fn canonical() -> Self {
        #[allow(clippy::expect_used)]
        let name = ReconcilerName::new(<Self as Reconciler>::NAME)
            .expect("'workflow-lifecycle' is a valid ReconcilerName by construction");
        Self { name }
    }
}

impl Reconciler for WorkflowLifecycle {
    /// Canonical kebab-case name; single compile-time anchor.
    const NAME: &'static str = "workflow-lifecycle";

    type State = WorkflowLifecycleState;
    type View = WorkflowLifecycleView;

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    /// Pure-sync `reconcile` (ADR-0035 shape). For each instance:
    ///
    /// - **observed terminal** → converged, emit nothing.
    /// - **running-in-intent, no live task, no terminal** → re-emit
    ///   `StartWorkflow` carrying the instance's spec + correlation. On
    ///   restart this re-hydrates the instance; the engine's
    ///   `load_journal` resumes rather than cold-starts (US-WP-3 AC4).
    /// - **running-in-intent, live task** → no-op (the engine is driving
    ///   it).
    /// - **not running-in-intent** → no-op (instance not desired).
    ///
    /// The body holds no `.await`, reads no wall-clock, and consults no
    /// RNG — `ReconcilerIsPure` holds (AC5). `_tick` is unused: the
    /// re-emit decision is independent of time (Phase 1 has no retry
    /// backoff).
    fn reconcile(
        &self,
        _desired: &Self::State,
        actual: &Self::State,
        _view: &Self::View,
        _tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        let mut actions: Vec<Action> = Vec::new();
        for (correlation, instance) in &actual.instances {
            // Converged: a terminal row has been observed. No re-emit.
            if instance.terminal.is_some() {
                continue;
            }
            // Not desired, or the engine already holds a live task. No
            // action either way.
            if !instance.running_in_intent || instance.has_live_task {
                continue;
            }
            // Running-in-intent with no live engine task and no terminal —
            // re-emit StartWorkflow. The engine's `load_journal` decides
            // cold-start vs crash-resume; the reconciler stays pure.
            actions.push(Action::StartWorkflow {
                start: instance.spec.clone(),
                correlation: correlation.clone(),
            });
        }
        // The §18 self-re-enqueue gate (`run_convergence_tick`) treats an
        // all-Noop vector as "converged this tick". Emit a single Noop
        // when nothing needed doing so the gate reads the converged shape.
        if actions.is_empty() {
            actions.push(Action::Noop);
        }
        (actions, WorkflowLifecycleView::default())
    }
}
