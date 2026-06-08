//! `WorkloadLifecycle` reconciler — first real reconciler (US-03).

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use crate::SpiffeId;
use crate::aggregate::{Exec, Job, Node, ProbeDescriptor, WorkloadDriver, WorkloadKind};
use crate::id::{AllocationId, CorrelationKey, NodeId, WorkloadId};
use crate::traits::driver::{AllocationSpec, Resources};
use crate::traits::observation_store::{AllocState, AllocStatusRow};
use crate::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};
use crate::wall_clock::UnixInstant;

use super::backend_discovery_bridge::BackendDiscoveryBridge;
use super::{Action, Reconciler, ReconcilerName, TargetResource, TickContext};

/// Maximum restart attempts before `WorkloadLifecycle` gives up on an alloc.
///
/// Past this count the reconciler stops emitting `RestartAllocation`
/// for a persistently failing alloc. Per US-03 step 02-03 — the
/// ceiling exists to keep a repeatedly-crashing workload from
/// consuming infinite driver resources.
pub const RESTART_BACKOFF_CEILING: u32 = 5;

/// Backoff window between successive `RestartAllocation` emissions
/// for the same alloc.
///
/// Per US-03 Domain Example 2 (user-stories.md:421-424) the deadline
/// is `tick.now + initial_backoff` — singular, no progression. One
/// second balances transient-hiccup tolerance (slow startup,
/// dependency flap) against operator visibility within Phase 1's
/// wall-clock to "Failed (backoff exhausted)".
pub const RESTART_BACKOFF_DURATION: Duration = Duration::from_secs(1);

/// Per-attempt restart backoff policy lookup.
///
/// **Today this is degenerate-constant**: every `attempt` value
/// yields the same [`RESTART_BACKOFF_DURATION`]. The function exists
/// as a stability anchor so call sites stay unchanged when
/// operator-configurable per-job policy lands — TODO(#137), deferred
/// from #141's 'Out' section. The leading underscore on `_attempt`
/// is deliberate: the parameter is currently unused (degenerate
/// policy ignores attempt count) but lives in the signature so a
/// future progressive-backoff schedule (e.g.
/// `RESTART_BACKOFF_DURATION * 2_u32.pow(attempt)`) does not require
/// a breaking API change.
///
/// TODO(#137): operator-configurable per-job policy will thread a
/// `&RestartPolicy` through this signature rather than relying on
/// the workspace-global constant.
///
/// Persist-inputs discipline: callers MUST persist the *attempt
/// count* (and a `last_failure_seen_at` timestamp), not the deadline
/// this function computes from them — see
/// `.claude/rules/development.md` § "Persist inputs, not derived
/// state". Recomputing on every read picks up future policy changes
/// without a schema migration.
#[must_use]
pub const fn backoff_for_attempt(_attempt: u32) -> Duration {
    RESTART_BACKOFF_DURATION
}

/// The Phase 1 first real reconciler. Converges declared replica
/// count for a `Job` against the running `AllocStatusRow` set.
///
/// Trait shape pinned by ADR-0021; convergence + backoff logic per
/// US-03 (phase-1-first-workload, slice 3).
///
/// The reconciler reads `desired.job` (the target job) and
/// `actual.allocations` (running set), calls
/// `overdrive_scheduler::schedule(...)` on `desired.nodes` +
/// `desired.job`, and emits `Action::StartAllocation` /
/// `Action::StopAllocation` to converge. Restart counts are tracked
/// in `view.restart_counts`; backoff is gated by recomputing the
/// deadline as `view.last_failure_seen_at + backoff_for_attempt(...)`
/// against `tick.now_unix` (NEVER `Instant::now()` /
/// `SystemTime::now()`). Per `.claude/rules/development.md` §
/// "Persist inputs, not derived state".
pub struct WorkloadLifecycle {
    name: ReconcilerName,
}

impl WorkloadLifecycle {
    /// Construct the canonical `job-lifecycle` instance.
    ///
    /// # Panics
    ///
    /// Never — `Self::NAME` is a compile-time string literal
    /// satisfying every `ReconcilerName` validation rule.
    #[must_use]
    pub fn canonical() -> Self {
        #[allow(clippy::expect_used)]
        let name = ReconcilerName::new(<Self as Reconciler>::NAME)
            .expect("'job-lifecycle' is a valid ReconcilerName by construction");
        Self { name }
    }
}

impl Reconciler for WorkloadLifecycle {
    /// Canonical kebab-case name; single compile-time anchor.
    const NAME: &'static str = "job-lifecycle";

    type State = WorkloadLifecycleState;
    type View = WorkloadLifecycleView;

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    // Per ADR-0023 + ADR-0037 §4 the reconcile body is the single
    // dispatch surface for every WorkloadLifecycle decision branch (Stop,
    // Absent, Run → {Running, Operator-stopped, Job-natural-exit,
    // Restart-with-budget, NoCapacity-fresh-schedule}). Splitting it
    // into N helper fns would require threading every read of
    // `desired` / `actual` / `view` / `tick` through arguments;
    // each branch is short and self-contained, and the line count is
    // dominated by the inline-comment audit trail per the project's
    // documentation discipline (`.claude/rules/development.md`).
    #[allow(clippy::too_many_lines)]
    fn reconcile(
        &self,
        desired: &Self::State,
        actual: &Self::State,
        view: &Self::View,
        tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        // service-vip-allocator step 03-01 — Service-arm VIP release
        // per ADR-0049 (amended 2026-05-15).
        //
        // When the workload is a Service AND we have a spec_digest in
        // scope AND any allocation has reached a terminal-state
        // observation row (i.e. `row.terminal.is_some()`) AND the
        // digest has NOT already been recorded in
        // `view.released_for_terminal`, emit
        // `Action::ReleaseServiceVip` exactly once and stamp the digest
        // onto `next_view.released_for_terminal` so the next tick's
        // gate short-circuits. Per `.claude/rules/development.md`
        // § "Persist inputs, not derived state": the recorded set is
        // the input "we already emitted release for this digest" —
        // never a derived "needs release now" boolean.
        //
        // The release decision is independent of (and additive to)
        // the existing Stop / Absent / Run branches below: a terminal
        // operator-stopped Service alloc flows through the Run-branch
        // operator-stopped short-circuit (returns no other actions)
        // AND ALSO emits the release here. The two-action shape is
        // intentional — the reconciler is the single source of every
        // terminal claim, and Service-VIP release is a terminal claim
        // per ADR-0049.
        let release_pair = service_vip_release_emission(desired, actual, view);

        let (mut actions, mut next_view) = Self::reconcile_inner(desired, actual, view, tick);
        if let Some((release_action, released_digest)) = release_pair {
            actions.push(release_action);
            next_view.released_for_terminal.insert(released_digest);
        }

        // UI-06 (F1 fix per audit-reconciler-handoff-topology.md):
        // dual-emit `Action::EnqueueEvaluation` routed at the
        // `backend-discovery-bridge` whenever this tick mutates the
        // alloc set the bridge depends on (StartAllocation /
        // RestartAllocation / StopAllocation / FinalizeFailed).
        //
        // Pre-UI-06 the only enqueue site was the exit observer
        // (`exit_observer.rs:253-256`) which fires only on workload
        // exit. For long-lived Service workloads the bridge therefore
        // never ticked after Pending → Running, never observed the
        // Running alloc, never wrote a `ServiceBackendRow`, and the
        // entire downstream hydrator → dataplane chain was structurally
        // unreachable. The fix mirrors the UI-05 bridge → hydrator
        // dual-emit pattern at the reconciler surface.
        //
        // Single emission per tick (not per action): the broker is LWW
        // at `(ReconcilerName, TargetResource)` per ADR-0013 §8 /
        // whitepaper §18, so duplicate enqueues collapse to one
        // dispatch per drain cycle. Emitting once keeps the action
        // vector compact and reflects the broker's actual dispatch
        // shape. The target is `job/<workload_id>` — same scope the
        // exit observer's bridge enqueue uses (`exit_observer.rs:231`),
        // so post-UI-06 BOTH enqueue sites address the same broker key.
        if actions.iter().any(is_alloc_mutating_action) {
            #[allow(clippy::expect_used)]
            {
                let bridge_name = ReconcilerName::new(BACKEND_DISCOVERY_BRIDGE_NAME)
                    .expect("'backend-discovery-bridge' is a valid ReconcilerName by construction");
                let bridge_target = TargetResource::new(&format!("job/{}", desired.workload_id))
                    .expect(
                        "'job/<workload_id>' is a valid TargetResource by construction \
                         (WorkloadId is constructor-validated, prefix is canonical)",
                    );
                actions.push(Action::EnqueueEvaluation {
                    reconciler: bridge_name,
                    target: bridge_target,
                });
            }

            // GAP-9 (Shape C) — dual-emit `Action::EnqueueEvaluation`
            // routed at the `service-lifecycle` reconciler for
            // Service-kind workloads, on alloc-STARTING transitions only
            // (`StartAllocation` / `RestartAllocation`). This gives the
            // service-lifecycle reconciler its FIRST tick: a fresh
            // Service alloc starting or restarting is the moment its
            // startup probes become relevant. Without this enqueue the
            // reconciler — registered at boot — was never submitted by
            // any production path, so after the initial broker drain it
            // was never re-ticked and its terminal branches were
            // structurally unreachable.
            //
            // Narrower than the bridge predicate above: the bridge
            // re-renders its backend set on ADD *and* REMOVE
            // (Start/Restart/Stop/Finalize), but the service-lifecycle
            // reconciler only cares when an alloc starts probing —
            // Stop / FinalizeFailed are terminal-removal events that
            // bring no new startup window. The exit observer (Shape C
            // part 2) is the on-exit nudge for the failure path; this
            // site is the on-start nudge. Restricting to the starting
            // pair also keeps Stop/GC/Finalize tick shapes unchanged.
            //
            // Job-kind / Schedule workloads do NOT emit this — the
            // service-lifecycle reconciler is a Service-kind concern.
            // The `desired.workload_kind` gate is what keeps a Job-kind
            // StartAllocation from spuriously enqueueing it (which would
            // hydrate an empty Service state → 0 actions → broker churn).
            //
            // Same `job/<workload_id>` target keying as the bridge
            // dual-emit, same single-emission-per-tick discipline (the
            // broker is LWW at `(ReconcilerName, TargetResource)`),
            // reuses the existing `Action::EnqueueEvaluation` variant.
            if desired.workload_kind == WorkloadKind::Service
                && actions.iter().any(is_service_alloc_starting_action)
            {
                #[allow(clippy::expect_used)]
                {
                    let service_name = ReconcilerName::new(SERVICE_LIFECYCLE_NAME)
                        .expect("'service-lifecycle' is a valid ReconcilerName by construction");
                    let service_target =
                        TargetResource::new(&format!("job/{}", desired.workload_id)).expect(
                            "'job/<workload_id>' is a valid TargetResource by construction \
                         (WorkloadId is constructor-validated, prefix is canonical)",
                        );
                    actions.push(Action::EnqueueEvaluation {
                        reconciler: service_name,
                        target: service_target,
                    });
                }
            }
        }

        (actions, next_view)
    }
}

/// UI-06 — name of the `BackendDiscoveryBridge` reconciler.
///
/// Compile-time alias to `<BackendDiscoveryBridge as Reconciler>::NAME`
/// — a rename of the bridge's `NAME` constant without updating this
/// reference is a compile error, not a silent handoff failure.
const BACKEND_DISCOVERY_BRIDGE_NAME: &str = <BackendDiscoveryBridge as Reconciler>::NAME;

/// GAP-9 — name of the `ServiceLifecycleReconciler`.
///
/// Compile-time alias to
/// `<ServiceLifecycleReconciler as Reconciler>::NAME` — same anti-drift
/// discipline as [`BACKEND_DISCOVERY_BRIDGE_NAME`]: renaming the
/// reconciler's `NAME` const without updating this reference is a
/// compile error, not a silent GAP-9-style dead handoff.
const SERVICE_LIFECYCLE_NAME: &str =
    <crate::service_lifecycle::ServiceLifecycleReconciler as Reconciler>::NAME;

/// UI-06 — predicate: is `action` one of the four alloc-mutating
/// variants the `BackendDiscoveryBridge` cares about?
///
/// The bridge re-renders `ServiceBackendRow` from the Running-alloc
/// set on every tick; only transitions that ADD or REMOVE a Running
/// alloc, or finalize an alloc as failed, change the bridge's view.
/// The wildcard arm covers `Noop`, `HttpCall`, `WriteServiceBackendRow`,
/// `DataplaneUpdateService`, `ReleaseServiceVip`, `EnqueueEvaluation` —
/// none of which change the alloc set.
const fn is_alloc_mutating_action(action: &Action) -> bool {
    matches!(
        action,
        Action::StartAllocation { .. }
            | Action::RestartAllocation { .. }
            | Action::StopAllocation { .. }
            | Action::FinalizeFailed { .. }
    )
}

/// GAP-9 — predicate: is `action` an alloc-STARTING transition the
/// `service-lifecycle` reconciler cares about?
///
/// Strictly narrower than [`is_alloc_mutating_action`]: only
/// `StartAllocation` / `RestartAllocation` open a fresh startup window
/// in which the Service's startup probes become relevant. `Stop` /
/// `FinalizeFailed` are terminal-removal events — the service-lifecycle
/// reconciler has nothing new to converge on them, and the failure
/// path is nudged separately by the exit observer (Shape C part 2). The
/// wildcard arm therefore covers `StopAllocation`, `FinalizeFailed`,
/// `Noop`, and every non-alloc action.
const fn is_service_alloc_starting_action(action: &Action) -> bool {
    matches!(action, Action::StartAllocation { .. } | Action::RestartAllocation { .. })
}

impl WorkloadLifecycle {
    // The original reconcile body, factored out so the Service-VIP
    // release branch can wrap it without duplicating every branch's
    // return tuple. The inner method's contract is unchanged from the
    // pre-step-03-01 shape; the wrapper above is the only Service-arm
    // augmentation. Associated function (no `&self`) because the
    // reconcile logic is purely a function of `(desired, actual, view,
    // tick)` — the reconciler instance carries only the name newtype.
    #[allow(clippy::too_many_lines)]
    fn reconcile_inner(
        desired: &WorkloadLifecycleState,
        actual: &WorkloadLifecycleState,
        view: &WorkloadLifecycleView,
        tick: &TickContext,
    ) -> (Vec<Action>, WorkloadLifecycleView) {
        // Per ADR-0021 + US-03 AC: handle Stop / Absent / Run branches.
        //
        // Stop: when a stop intent is recorded (`desired.desired_to_stop`)
        // AND a job spec exists, emit `Action::StopAllocation` for every
        // Running alloc. Allocs in any other state (Pending, Draining,
        // Terminated) require no action; the next tick's hydrate
        // re-evaluates. A stop intent against an absent job is a no-op
        // (the second `desired.job.is_some()` clause).
        //
        // Transitional-view-state contract (whitepaper §18 *Level-triggered
        // inside the reconciler* + `fix-stop-branch-backoff-pending` RCA):
        // when `stop_actions.is_empty()` the stop is complete — there is
        // nothing left for the runtime to do. Clearing
        // `last_failure_seen_at` is what tells the runtime's
        // `view_has_backoff_pending` predicate to stop re-enqueueing;
        // without it, a Failed-mid-backoff alloc keeps the predicate
        // `true` and the broker spins for ~5 s until `restart_counts`
        // reaches the ceiling. `restart_counts` is intentionally left
        // intact: the predicate only checks counts for entries that
        // exist in `last_failure_seen_at`, so clearing the
        // observation-timestamp map is sufficient — and the historical
        // record is preserved.
        if desired.desired_to_stop && desired.job.is_some() {
            // Per ADR-0037 §4: an operator-initiated stop is a
            // terminal moment. The reconciler stamps the
            // by-source onto every emitted StopAllocation — the
            // action shim writes the same value onto
            // AllocStatusRow.terminal AND echoes onto
            // LifecycleEvent.terminal in step 02-02.
            let operator_stop_terminal =
                Some(TerminalCondition::Stopped { by: StoppedBy::Operator });
            let stop_actions: Vec<Action> = actual
                .allocations
                .values()
                .filter(|r| r.state == AllocState::Running)
                .map(|r| Action::StopAllocation {
                    alloc_id: r.alloc_id.clone(),
                    terminal: operator_stop_terminal.clone(),
                })
                .collect();
            // When nothing is Running, the stop is complete.
            // Clear backoff state so view_has_backoff_pending does not re-enqueue.
            let mut next_view = view.clone();
            if stop_actions.is_empty() {
                next_view.last_failure_seen_at.clear();
            }
            return (stop_actions, next_view);
        }
        match desired.job.as_ref() {
            // Absent: no desired job. The Stop branch above handles
            // explicit stops via `desired_to_stop`; the GC branch
            // here handles the case where the workload's intent has
            // been *withdrawn* entirely (hard-delete, multi-node
            // drain, crash-recovery surgery).
            //
            // GC branch (closes #148 per ADR-0037 Amendment
            // 2026-05-14): withdraw any non-terminal allocations by
            // stamping a system-GC terminal claim. Structural mirror
            // of the operator-Stop branch above (filter Running rows,
            // emit one StopAllocation per row, clear
            // `last_failure_seen_at` when no work remains).
            // `StoppedBy::SystemGc` is the load-bearing
            // discriminator: it lets the action shim, lifecycle
            // event consumers, and operator-facing surfaces
            // distinguish "the operator stopped this" from "the
            // system reaped this because no intent referenced it".
            //
            // Filter shape (architecture.md § 8 Open Q3): only
            // Running rows are stopped. A Pending row has no
            // driver-side runtime to stop; a Draining row is already
            // being torn down by the worker. Same shape as the
            // operator-Stop branch above; mutation tests pin the
            // filter at `state == Running`.
            //
            // Kind-agnostic: branches on `desired.job.is_none()`,
            // not on `desired.workload_kind`. An orphan-row scenario
            // can occur for any workload kind (architecture.md § 8
            // Open Q2).
            None => {
                let gc_terminal = Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc });
                let stop_actions: Vec<Action> = actual
                    .allocations
                    .values()
                    .filter(|r| r.state == AllocState::Running)
                    .map(|r| Action::StopAllocation {
                        alloc_id: r.alloc_id.clone(),
                        terminal: gc_terminal.clone(),
                    })
                    .collect();
                let mut next_view = view.clone();
                if stop_actions.is_empty() {
                    // No work left — clear backoff inputs so
                    // `view_has_backoff_pending` returns false and
                    // the broker stops re-enqueueing this target.
                    // Mirrors the Stop branch's view-cleanup shape;
                    // input clearance, not derived-deadline
                    // clearance, per `.claude/rules/development.md`
                    // § "Persist inputs, not derived state".
                    next_view.last_failure_seen_at.clear();
                }
                (stop_actions, next_view)
            }
            // Run: a job is desired.
            Some(job) => {
                // Pure first-fit placement (inlined from
                // overdrive-scheduler::schedule). Pulled inline rather
                // than calling the scheduler crate because
                // overdrive-core cannot depend on overdrive-scheduler
                // (would invert the dependency direction).
                let allocs_vec: Vec<&AllocStatusRow> = actual.allocations.values().collect();

                // Per workload-gc-absent-stale-allocs step 01-04: derive
                // a second view that excludes intentional-stop rows
                // (Operator OR SystemGc). Used by the running-alloc /
                // natural-exit / failed-alloc checks below so that a
                // re-submit after GC lands a fresh placement (the
                // architecture.md § 5 promise) rather than spuriously
                // restarting / finalizing the GC'd row. Operator-
                // stopped rows continue to short-circuit at the top
                // of the Run branch via the narrower
                // `is_operator_stopped` check (their semantics is
                // strictly stronger — see comment block below).
                let active_allocs_vec: Vec<&AllocStatusRow> =
                    allocs_vec.iter().filter(|r| !is_intentionally_stopped(r)).copied().collect();

                // Is any allocation already Running for this job? If so
                // we are converged — emit nothing. Failed allocs flow
                // into the restart-with-backoff branch below.
                let running_alloc =
                    active_allocs_vec.iter().find(|r| r.state == AllocState::Running);
                if running_alloc.is_some() {
                    return (Vec::new(), view.clone());
                }

                // Per `fix-exec-driver-exit-watcher` Step 01-02 RCA
                // §Bug 3: an Operator-stopped Terminated alloc is a
                // terminal intentional stop. The reconciler MUST NOT
                // schedule a fresh replacement allocation for the
                // job — operator stop intent is the load-bearing
                // discriminator and a fresh schedule would undo the
                // operator's stop. The alloc record remains in obs
                // as the terminal state until the operator explicitly
                // re-submits the job intent.
                //
                // (Distinct from `desired.desired_to_stop`, which is
                // a separate signal carried by `IntentKey::for_workload_stop`
                // and handled at the Stop branch above. The
                // Operator-stopped row arrives via the watcher's
                // `intentional_stop` flag — set by `Driver::stop`
                // even when no `for_job_stop` intent exists, e.g.
                // by direct CLI / API operator action.)
                //
                // **Asymmetry vs SystemGc** (workload-gc-absent-stale-
                // allocs step 01-04). SystemGc-stopped rows are NOT
                // short-circuited here — they are filtered out of
                // `active_allocs_vec` above so that the Run branch
                // falls through to fresh placement on resubmit. The
                // semantic difference: Operator stop is OVERRIDING
                // (operator's intent outranks the new submit; a
                // fresh schedule would undo the operator's stop),
                // while SystemGc stop is OVERRIDABLE (system stop
                // was system-initiated because intent disappeared;
                // a fresh submit IS the operator's overriding new
                // intent and should land a fresh allocation —
                // architecture.md § 5).
                if allocs_vec.iter().any(|r| is_operator_stopped(r)) {
                    return (Vec::new(), view.clone());
                }

                // Job-kind natural-exit handler per ADR-0037 Amendment
                // 2026-05-10 / ADR-0047 §1. A run-to-completion workload
                // (`WorkloadKind::Job`) terminates on the first observed
                // exit — clean OR crashed — and the reconciler emits
                // `Action::FinalizeFailed` carrying the typed terminal
                // claim. There are no restart attempts (the workload's
                // contract is "run once, until it exits"). Service-kind
                // (and Schedule-kind, Phase 1 no-op) flow through the
                // existing restart-budget branch below — preserves the
                // pre-feature kind-agnostic semantics that the Service
                // shape today emulates.
                //
                // Idempotency guard: if the row already carries a
                // Completed/Failed terminal claim the reconciler has
                // already finalised this alloc on a prior tick — do not
                // re-emit. Without this guard the action shim's
                // level-triggered re-enqueue would emit FinalizeFailed
                // every tick forever once the alloc reached terminal.
                if desired.workload_kind == WorkloadKind::Job
                    && let Some(terminal_alloc) =
                        active_allocs_vec.iter().find(|r| is_natural_exit(r))
                {
                    if matches!(
                        terminal_alloc.terminal,
                        Some(
                            TerminalCondition::Completed { .. } | TerminalCondition::Failed { .. }
                        )
                    ) {
                        return (Vec::new(), view.clone());
                    }
                    let typed = classify_natural_exit_terminal(terminal_alloc);
                    let action = Action::FinalizeFailed {
                        alloc_id: terminal_alloc.alloc_id.clone(),
                        terminal: Some(typed),
                    };
                    return (vec![action], view.clone());
                }

                // Failed alloc with attempt budget remaining and
                // backoff elapsed → emit RestartAllocation. Per US-03
                // the reconciler tracks restart attempts in
                // `view.restart_counts` and STOPS emitting
                // RestartAllocation once `RESTART_BACKOFF_CEILING` is
                // reached. The alloc then stays Terminated indefinitely
                // (backoff exhausted).
                // Per ADR-0032 §5 + slice 02 step 02-01: the action
                // shim now writes `AllocState::Failed` on driver
                // `StartRejected` (instead of `Terminated`) to
                // distinguish operator-stop from driver-could-not-
                // start. The restart-budget logic treats both states
                // identically — both are "this alloc is not Running
                // and the reconciler should consider restarting it"
                // — so the matcher includes both.
                //
                // Per `fix-exec-driver-exit-watcher` Step 01-02 RCA
                // §Bug 3: an alloc whose obs row carries
                // `reason: Some(Stopped { by: Operator })` is a
                // terminal intentional stop. The reconciler MUST NOT
                // restart it — operator stop intent is observed via
                // the watcher's `intentional_stop` flag (mapped to
                // `StoppedBy::Operator` by the worker `exit_observer`
                // subsystem) and is the load-bearing discriminator
                // distinguishing operator-driven termination from
                // crash. A reconciler that restarts an Operator-
                // stopped alloc would over-write the observer's
                // `Terminated` row with a fresh `Running`, masking
                // the operator's stop in obs and contradicting the
                // §intentional_stop ordering invariant on
                // `Driver::take_exit_receiver`.
                let failed_alloc = active_allocs_vec.iter().find(|r| is_restartable(r));
                if let Some(failed) = failed_alloc {
                    // Backoff exhaustion check — emit no further
                    // RestartAllocation past the ceiling. Pure check
                    // against `view.restart_counts`.
                    let attempts = view.restart_counts.get(&failed.alloc_id).copied().unwrap_or(0);
                    if attempts >= RESTART_BACKOFF_CEILING {
                        // Idempotency guard: if the row already carries
                        // a BackoffExhausted terminal claim the
                        // reconciler has already finalised this alloc on
                        // a prior tick — do not re-emit. Without this
                        // guard the action shim's level-triggered
                        // re-enqueue would emit FinalizeFailed every
                        // tick forever once the alloc reached ceiling.
                        if matches!(
                            failed.terminal,
                            Some(TerminalCondition::BackoffExhausted { .. })
                        ) {
                            return (Vec::new(), view.clone());
                        }
                        // Backoff exhausted — emit the synthetic
                        // FinalizeFailed action carrying the typed
                        // terminal claim per ADR-0037 §4. The action
                        // shim consumes this in step 02-02 to write
                        // AllocStatusRow.terminal AND echo onto
                        // LifecycleEvent.terminal — both surfaces
                        // populated from the same value at the same
                        // dispatch site (drift is structurally
                        // impossible). The reconciler is the single
                        // source of every terminal claim.
                        let action = Action::FinalizeFailed {
                            alloc_id: failed.alloc_id.clone(),
                            terminal: Some(TerminalCondition::BackoffExhausted { attempts }),
                        };
                        return (vec![action], view.clone());
                    }
                    // Persist-inputs read site (issue #141): recompute
                    // the backoff deadline on every tick from the
                    // persisted *inputs* (`last_failure_seen_at`,
                    // `restart_counts`) against the current policy
                    // (`backoff_for_attempt`). Mirrors the precedent at
                    // `crates/overdrive-control-plane/src/worker/exit_observer.rs:291`
                    // (`RETRY_BACKOFFS.get((attempts - 1) as usize)`).
                    // A future operator-configurable per-job
                    // `backoff_for_attempt` policy lands without a
                    // schema migration — every persisted row picks up
                    // the new policy on the next reconcile tick.
                    if let Some(seen_at) = view.last_failure_seen_at.get(&failed.alloc_id) {
                        let backoff = backoff_for_attempt(attempts);
                        if tick.now_unix < *seen_at + backoff {
                            // Backoff window not yet elapsed.
                            return (Vec::new(), view.clone());
                        }
                    }
                    // Per ADR-0031 §5 the Restart action carries the
                    // fully-populated `AllocationSpec` — mirroring the
                    // Start path. The reconciler has the live Job in
                    // scope; constructing the spec here is pure (two
                    // .clone() calls + identity derivation), and
                    // preserves the shim's stateless-dispatcher
                    // contract per ADR-0023.
                    let identity = SpiffeId::for_allocation(&job.id, &failed.alloc_id);
                    // Per ADR-0031 Amendment 1: destructure the
                    // tagged-enum `WorkloadDriver` to project to the
                    // flat `AllocationSpec` (which stays flat per
                    // ADR-0030 §6). The destructure is irrefutable
                    // today (single Phase-1 variant); when Phase-2+
                    // adds variants it becomes a `match` and each arm
                    // projects to its per-driver-class spec.
                    let WorkloadDriver::Exec(Exec { command, args }) = &job.driver;
                    let action = Action::RestartAllocation {
                        alloc_id: failed.alloc_id.clone(),
                        spec: AllocationSpec {
                            alloc: failed.alloc_id.clone(),
                            identity,
                            command: command.clone(),
                            args: args.clone(),
                            resources: job.resources,
                            // Per ADR-0054 §3 + GAP-8 close-out: the
                            // descriptor vec is projected from the live
                            // intent at hydrate-desired time. Job-kind
                            // workloads carry an empty vec (no probe
                            // surface); Service-kind workloads carry
                            // startup → readiness → liveness in
                            // canonical role order. See
                            // `WorkloadLifecycleState::probe_descriptors`.
                            probe_descriptors: desired.probe_descriptors.clone(),
                        },
                        kind: desired.workload_kind,
                        // Crash-loop restart pathway — the restart cause is
                        // implicit in the prior alloc's terminal. The typed
                        // liveness cause (`RestartReason::LivenessExhausted`)
                        // is stamped only by the `service-lifecycle`
                        // reconciler (step 03-02 / Slice 05); this site
                        // keeps `None` per the additive-`Option` contract on
                        // `Action::RestartAllocation`.
                        reason: None,
                    };
                    let mut next_view = view.clone();
                    let count =
                        next_view.restart_counts.entry(failed.alloc_id.clone()).or_insert(0);
                    *count = count.saturating_add(1);
                    // Persist-inputs write site (issue #141): record
                    // the wall-clock observation timestamp of this
                    // failure (`tick.now_unix`) — NOT the precomputed
                    // deadline `tick.now + RESTART_BACKOFF_DURATION`,
                    // which would lock in the policy-at-write-time and
                    // break the "policy evolution is a no-op for the
                    // schema" guarantee. The deadline is recomputed at
                    // the read site on every tick from this seen_at +
                    // `backoff_for_attempt(restart_count)`.
                    next_view.last_failure_seen_at.insert(failed.alloc_id.clone(), tick.now_unix);
                    return (vec![action], next_view);
                }

                // No Running, no failed-needs-restart → schedule a
                // fresh allocation. Inline first-fit over BTreeMap.
                let placement = first_fit_place(&desired.nodes, job, &allocs_vec);
                placement.map_or_else(
                    || {
                        // NoCapacity — emit no action. The Pending row
                        // remains in obs (the renderer surfaces the
                        // reason at render time). Backoff is irrelevant
                        // here (nothing to back off from).
                        (Vec::new(), view.clone())
                    },
                    |node_id| {
                        // Fresh-id derivation per workload-gc-absent-
                        // stale-allocs step 01-04: index the new
                        // alloc by the number of pre-existing rows
                        // for this workload. With zero rows the
                        // suffix is `0` (preserves the prior shape);
                        // with a SystemGc-Terminated row already in
                        // `allocs_vec` (resubmit-after-GC), the
                        // suffix is `1` and the new alloc gets a
                        // distinct id. This makes the action shim's
                        // LWW write of the new `Running` row land
                        // on a NEW key rather than overwrite the
                        // prior SystemGc terminal stamp — making
                        // good on architecture.md § 5's
                        // `resubmit.preserves_prior_gc_terminal`
                        // promise.
                        let attempt = u32::try_from(allocs_vec.len()).unwrap_or(u32::MAX);
                        let alloc_id = mint_alloc_id(&job.id, attempt);
                        let identity = SpiffeId::for_allocation(&job.id, &alloc_id);
                        // Per ADR-0031 §5 + Amendment 1: the Start
                        // action carries the operator-declared command
                        // + args projected from the tagged-enum
                        // `WorkloadDriver` field on `Job`. No more
                        // literal `/bin/sleep` / `["60"]`. The
                        // destructure is irrefutable today (single
                        // Phase-1 variant); future variants append.
                        let WorkloadDriver::Exec(Exec { command, args }) = &job.driver;
                        let action = Action::StartAllocation {
                            alloc_id: alloc_id.clone(),
                            workload_id: job.id.clone(),
                            node_id,
                            spec: AllocationSpec {
                                alloc: alloc_id,
                                identity,
                                command: command.clone(),
                                args: args.clone(),
                                resources: job.resources,
                                // Per ADR-0054 §3 + GAP-8 close-out:
                                // projected from the live intent at
                                // hydrate-desired time. Job-kind = empty
                                // vec; Service-kind = startup → readiness
                                // → liveness in canonical order. See
                                // `WorkloadLifecycleState::probe_descriptors`.
                                probe_descriptors: desired.probe_descriptors.clone(),
                            },
                            kind: desired.workload_kind,
                        };
                        (vec![action], view.clone())
                    },
                )
            }
        }
    }
}

/// Pure first-fit placement helper. Inlined here because
/// `overdrive-core` cannot depend on `overdrive-scheduler` (would
/// invert the dependency direction; the scheduler is a `core`-class
/// crate that depends on `overdrive-core`). The algorithm is the same
/// as `overdrive_scheduler::schedule`'s happy path: walk `nodes` in
/// `BTreeMap` order, return the first `NodeId` whose free capacity
/// covers the job's resource envelope.
pub(crate) fn first_fit_place(
    nodes: &BTreeMap<NodeId, Node>,
    job: &Job,
    current_allocs: &[&AllocStatusRow],
) -> Option<NodeId> {
    for (node_id, node) in nodes {
        let free = node_free_capacity(node, current_allocs, &job.resources);
        if free.cpu_milli >= job.resources.cpu_milli
            && free.memory_bytes >= job.resources.memory_bytes
        {
            return Some(node_id.clone());
        }
    }
    None
}

/// Free capacity of `node` after subtracting reserved envelope of
/// Running allocations targeting it. Inline counterpart to
/// `overdrive_scheduler::free_capacity`.
fn node_free_capacity(
    node: &Node,
    current_allocs: &[&AllocStatusRow],
    per_alloc: &Resources,
) -> Resources {
    let running_on_node: u64 = u64::try_from(
        current_allocs
            .iter()
            .filter(|alloc| alloc.node_id == node.id && alloc.state == AllocState::Running)
            .count(),
    )
    .unwrap_or(u64::MAX);
    let total_cpu_reserved = u64::from(per_alloc.cpu_milli).saturating_mul(running_on_node);
    let total_mem_reserved = per_alloc.memory_bytes.saturating_mul(running_on_node);
    let cpu_after = u64::from(node.capacity.cpu_milli).saturating_sub(total_cpu_reserved);
    Resources {
        cpu_milli: u32::try_from(cpu_after).unwrap_or(u32::MAX),
        memory_bytes: node.capacity.memory_bytes.saturating_sub(total_mem_reserved),
    }
}

/// Mint a deterministic [`AllocationId`] for a job at attempt index
/// `attempt`. Pure function over `(workload_id, attempt)` so two
/// reconcile calls with the same desired/actual produce the same
/// alloc id (purity contract).
///
/// **The `attempt` parameter is the count of pre-existing alloc
/// rows for the workload at placement time** (per workload-gc-
/// absent-stale-allocs step 01-04). With zero pre-existing rows the
/// suffix is `0` (preserves the pre-Phase-1.4 single-shot shape);
/// after a SystemGc stop leaves one Terminated row behind, a
/// resubmit's placement passes `attempt = 1` and mints
/// `alloc-{workload_id}-1` — distinct from the GC'd row's
/// `alloc-{workload_id}-0`. This is the structural defence against
/// the resurrection class where the action shim's LWW write of the
/// new `Running` row would otherwise overwrite the prior SystemGc
/// terminal stamp.
fn mint_alloc_id(workload_id: &WorkloadId, attempt: u32) -> AllocationId {
    let raw = format!("alloc-{}-{}", workload_id.as_str(), attempt);
    #[allow(clippy::expect_used)]
    AllocationId::new(&raw).expect("derived alloc id format is valid")
}

/// service-vip-allocator step 03-01 — pure helper for the Service-arm
/// release-emission gate.
///
/// Returns `Some((action, digest))` when:
///
/// 1. `desired.workload_kind == WorkloadKind::Service`, AND
/// 2. `desired.service_spec_digest` is `Some(digest)`, AND
/// 3. at least one observed allocation in `actual.allocations` carries
///    a terminal claim (`row.terminal.is_some()`), AND
/// 4. `digest` is NOT already present in `view.released_for_terminal`.
///
/// Returns `None` otherwise — i.e. for non-Service kinds, when the
/// digest is absent (the runtime hydrator has not populated it), when
/// no terminal-state observation row exists yet, or when the digest
/// is already recorded as released.
///
/// The caller (the `WorkloadLifecycle::reconcile` wrapper) appends the
/// returned action to the inner reconcile's action list and stamps the
/// returned digest onto `next_view.released_for_terminal` so the next
/// tick short-circuits. Per ADR-0049 (amended 2026-05-15) +
/// `.claude/rules/development.md` § "Persist inputs, not derived
/// state".
fn service_vip_release_emission(
    desired: &WorkloadLifecycleState,
    actual: &WorkloadLifecycleState,
    view: &WorkloadLifecycleView,
) -> Option<(Action, crate::id::ContentHash)> {
    if desired.workload_kind != WorkloadKind::Service {
        return None;
    }
    let digest = desired.service_spec_digest?;
    if view.released_for_terminal.contains(&digest) {
        return None;
    }
    let terminal_observed = actual.allocations.values().any(|row| row.terminal.is_some());
    if !terminal_observed {
        return None;
    }
    let target = format!("job-lifecycle/{}", desired.workload_id.as_str());
    let correlation = CorrelationKey::derive(&target, &digest, "release-service-vip");
    Some((Action::ReleaseServiceVip { spec_digest: digest, correlation }, digest))
}

/// True iff the alloc row carries a terminal Operator-stop record.
///
/// Two writers produce operator-stop rows with different field shapes:
///
/// - **Exit observer** (direct observation): writes
///   `reason: Stopped { by: Operator }`, `terminal: None`.
/// - **Action shim** (ADR-0037 §4 SSOT): writes
///   `reason: Stopped { by: Reconciler }`,
///   `terminal: Stopped { by: Operator }`.
///
/// The function checks BOTH `terminal` (the ADR-0037 §4 SSOT) and
/// `reason` (the exit-observer path) so that operator-stop rows from
/// either writer are recognised. See GH #149 for the regression that
/// motivated the dual check.
///
/// **Narrow semantics — load-bearing.** This predicate matches
/// `StoppedBy::Operator` only and is used by the Run-branch's
/// short-circuit: an Operator-stopped row preserves a stronger
/// contract than the broader intentional-stop class. Operator stop
/// overrides re-submit (the operator's intent outranks the new
/// submit), so the Run branch returns no actions even when desired
/// intent is present. Use [`is_intentionally_stopped`] for restart /
/// natural-exit / placement-candidacy decisions where `Operator` and
/// `SystemGc` share the "don't restart, don't finalize" semantics.
fn is_operator_stopped(row: &AllocStatusRow) -> bool {
    row.state == AllocState::Terminated
        && (matches!(
            row.terminal,
            Some(crate::transition_reason::TerminalCondition::Stopped {
                by: crate::transition_reason::StoppedBy::Operator
            })
        ) || matches!(
            row.reason,
            Some(crate::transition_reason::TransitionReason::Stopped {
                by: crate::transition_reason::StoppedBy::Operator
            })
        ))
}

/// True iff the alloc row carries a terminal *intentional-stop class*
/// record — `state == Terminated` AND its `terminal` OR `reason`
/// carries `Stopped { by: ∈ {Operator, SystemGc} }`.
///
/// **Asymmetry vs [`is_operator_stopped`] — load-bearing.** This
/// predicate is the broader query covering both intentional-stop
/// sources; [`is_operator_stopped`] is the narrower Operator-only
/// query.
fn is_intentionally_stopped(row: &AllocStatusRow) -> bool {
    row.state == AllocState::Terminated
        && (matches!(
            row.terminal,
            Some(crate::transition_reason::TerminalCondition::Stopped {
                by: crate::transition_reason::StoppedBy::Operator
                    | crate::transition_reason::StoppedBy::SystemGc
            })
        ) || matches!(
            row.reason,
            Some(crate::transition_reason::TransitionReason::Stopped {
                by: crate::transition_reason::StoppedBy::Operator
                    | crate::transition_reason::StoppedBy::SystemGc
            })
        ))
}

/// True iff the alloc row is a candidate for a `RestartAllocation`
/// action — i.e. it sits in a restartable terminal state AND is NOT
/// part of the intentional-stop class (Operator OR SystemGc).
fn is_restartable(row: &AllocStatusRow) -> bool {
    let restartable_state =
        matches!(row.state, AllocState::Terminated | AllocState::Draining | AllocState::Failed);
    restartable_state && !is_intentionally_stopped(row)
}

/// True iff the alloc row represents a *natural exit* the Job-kind
/// reconciler should finalize on.
fn is_natural_exit(row: &AllocStatusRow) -> bool {
    let terminal_state = matches!(row.state, AllocState::Terminated | AllocState::Failed);
    terminal_state && !is_intentionally_stopped(row)
}

/// Classify a natural-exit alloc row into the typed
/// `TerminalCondition::Completed` / `TerminalCondition::Failed`
/// variant per ADR-0037 Amendment 2026-05-10.
fn classify_natural_exit_terminal(row: &AllocStatusRow) -> TerminalCondition {
    if row.state == AllocState::Terminated
        && matches!(row.reason, Some(TransitionReason::Stopped { by: StoppedBy::Process }))
    {
        return TerminalCondition::Completed { exit_code: 0 };
    }
    if let Some(TransitionReason::WorkloadCrashedImmediately { exit_code, .. }) = row.reason {
        return TerminalCondition::Failed { exit_code };
    }
    TerminalCondition::Failed { exit_code: Some(0) }
}

/// Desired/actual projection consumed by `WorkloadLifecycle::reconcile`.
/// Hydrated by the runtime from `IntentStore` (job + nodes) and
/// `ObservationStore` (allocations) per ADR-0021.
///
/// The same struct serves both `desired` and `actual` — the
/// reconciler interprets `desired.job` as "what should exist" and
/// `actual.allocations` as "what is currently running." Field shapes
/// are pinned by ADR-0021 §1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadLifecycleState {
    /// Kind-agnostic workload identity, always available from the
    /// `TargetResource` at hydration time.
    pub workload_id: WorkloadId,
    /// The target job. `None` when the desired-state read returned
    /// no row (job was deleted) or the actual-state read found no
    /// surviving row to project against.
    pub job: Option<Job>,
    /// Whether a stop intent has been recorded for this job.
    pub desired_to_stop: bool,
    /// Registered nodes with their declared capacity.
    pub nodes: BTreeMap<NodeId, Node>,
    /// Current allocations belonging to this job, keyed by alloc id.
    pub allocations: BTreeMap<AllocationId, AllocStatusRow>,
    /// Workload kind discriminator per ADR-0047 §1 / ADR-0037 Amendment
    /// 2026-05-10.
    pub workload_kind: WorkloadKind,
    /// Content-addressed `spec_digest` for the workload.
    pub service_spec_digest: Option<crate::id::ContentHash>,
    /// Probe descriptors projected from the live intent at
    /// hydrate-desired time. Per ADR-0054 §3 + closes GAP-8 from the
    /// Phase 01 structural audit: for Service-kind workloads this
    /// carries the concatenation of `startup_probes`, `readiness_probes`,
    /// and `liveness_probes` in canonical role order (startup →
    /// readiness → liveness) so `ProbeRunner::start_alloc`'s
    /// `iter().enumerate()` assigns deterministic `probe_idx` values.
    /// For Job-kind workloads this is always empty — Job-kind has no
    /// probe surface (`ProbeRunner` is a Service-kind concern).
    ///
    /// The reconciler clones this vec into every emitted
    /// `Action::StartAllocation { spec, .. }` and
    /// `Action::RestartAllocation { spec, .. }` so the runtime's
    /// per-descriptor probe-task spawn loop (GAP-7) receives the live
    /// descriptor set the operator declared. Pre-GAP-8 the reconciler
    /// hardcoded `probe_descriptors: Vec::new()` at both sites,
    /// silently dropping Service-kind probes even after GAP-6 admission
    /// + GAP-7 spawn-loop wiring landed.
    pub probe_descriptors: Vec<ProbeDescriptor>,
}

/// Project the operator-declared probe descriptors of a
/// [`crate::aggregate::WorkloadIntent`] into the flat vector consumed
/// by `Action::StartAllocation { spec, .. }` /
/// `Action::RestartAllocation { spec, .. }` (via
/// [`WorkloadLifecycleState::probe_descriptors`]) and downstream by
/// `ProbeRunner::start_alloc`'s per-descriptor spawn loop.
///
/// Closes GAP-8 from the Phase 01 structural audit. Pre-patch the
/// reconciler hardcoded an empty `Vec` at both action arms with a
/// comment justifying it for Job-kind; Service-kind silently inherited
/// the empty vec even though `ServiceV1` carries three probe vectors
/// (GAP-6 admission close-out). The runtime now calls this helper at
/// hydrate-desired time and stamps the result onto
/// [`WorkloadLifecycleState::probe_descriptors`]; the reconciler
/// clones it into both action arms.
///
/// **Projection order is canonical**: `startup → readiness → liveness`.
/// This matches [`crate::observation::ProbeRole`]'s declared order
/// (`Startup`, `Readiness`, `Liveness`) and is the order
/// `ProbeRunner::start_alloc` consumes the vec via `iter().enumerate()`,
/// so `probe_idx` lands at a deterministic position per role. Reordering
/// the concatenation would break the downstream
/// `(alloc_id, probe_idx) → ProbeResultRow` slot mapping that the
/// `ServiceLifecycleReconciler` hydrate path reads.
///
/// Per-variant projection:
///
/// - `WorkloadIntent::Job(_)` → empty vec (Job-kind has no probe
///   surface per ADR-0054 §3; `ProbeRunner` is a Service-kind concern).
/// - `WorkloadIntent::Service(svc)` → `svc.startup_probes ++
///   svc.readiness_probes ++ svc.liveness_probes` (clones, since the
///   helper is `pub fn(&WorkloadIntent)` and the projection escapes
///   the borrow).
/// - `WorkloadIntent::Schedule(_)` → empty vec (the schedule's per-fire
///   instance is a Job, not a Service; probes on a Schedule's inner
///   Job make no semantic sense in Phase 1).
#[must_use]
pub fn project_probe_descriptors(
    intent: &crate::aggregate::WorkloadIntent,
) -> Vec<ProbeDescriptor> {
    match intent {
        crate::aggregate::WorkloadIntent::Job(_)
        | crate::aggregate::WorkloadIntent::Schedule(_) => Vec::new(),
        crate::aggregate::WorkloadIntent::Service(svc) => {
            let mut out = Vec::with_capacity(
                svc.startup_probes.len() + svc.readiness_probes.len() + svc.liveness_probes.len(),
            );
            out.extend(svc.startup_probes.iter().cloned());
            out.extend(svc.readiness_probes.iter().cloned());
            out.extend(svc.liveness_probes.iter().cloned());
            out
        }
    }
}

/// `WorkloadLifecycle` reconciler's typed view — the runtime-persisted
/// private memory per ADR-0035.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct WorkloadLifecycleView {
    /// How many times each alloc has been started under this
    /// reconciler's lifecycle.
    #[serde(default)]
    pub restart_counts: BTreeMap<AllocationId, u32>,
    /// Wall-clock observation timestamp of the last failure per alloc.
    #[serde(default)]
    pub last_failure_seen_at: BTreeMap<AllocationId, UnixInstant>,
    /// Set of `spec_digest`s for which `Action::ReleaseServiceVip`
    /// has already been emitted.
    #[serde(default)]
    pub released_for_terminal: BTreeSet<crate::id::ContentHash>,
}
