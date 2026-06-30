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
        // per ADR-0049 (amendment 2026-06-28 — withhold-not-release).
        //
        // When the workload is a Service AND we have a spec_digest in
        // scope AND the workload's intent is WITHDRAWN
        // (`desired.job.is_none()` — logical-workload deletion) AND the
        // digest has NOT already been recorded in
        // `view.released_for_deletion`, emit `Action::ReleaseServiceVip`
        // exactly once and stamp the digest onto
        // `next_view.released_for_deletion` so the next tick's gate
        // short-circuits. Per `.claude/rules/development.md` § "Persist
        // inputs, not derived state": the recorded set is the input "we
        // already emitted release for this digest" — never a derived
        // "needs release now" boolean.
        //
        // A stopped-or-crashed-but-STILL-DECLARED Service
        // (`desired.job.is_some()`) RETAINS its VIP — the VIP is an
        // identity bound to the declared workload, symmetric with the
        // dial-by-name frontend `F` (ADR-0072). The release decision is
        // independent of (and additive to) the Stop / Absent / Run
        // branches below, and now COINCIDES with the Absent/GC branch's
        // own `desired.job.is_none()` trigger: on intent withdrawal the
        // reconciler GCs the running allocs AND releases the VIP.
        let release_pair = service_vip_release_emission(desired, view);

        let (mut actions, mut next_view) = Self::reconcile_inner(desired, actual, view, tick);
        if let Some((release_action, released_digest)) = release_pair {
            actions.push(release_action);
            next_view.released_for_deletion.insert(released_digest);
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

            // ADR-0067 D5b (producer 1) — enqueue `svid-lifecycle` for the
            // same workload-scoped target so the SVID reconciler re-converges
            // its held set against the running set. UNGATED by workload kind:
            // identity is needed by EVERY running allocation, not only Service
            // (unlike the service-lifecycle dual-emit above). Every one of the
            // four alloc-mutating variants ADDs or REMOVEs a Running alloc —
            // exactly when the held set must re-converge (Start/Restart →
            // `running ∧ ¬held → IssueSvid`; Stop/Finalize →
            // `¬running ∧ held → DropSvid`).
            //
            // Single emission per tick (not per action): the broker is LWW at
            // `(ReconcilerName, TargetResource)`, so a duplicate enqueue across
            // this site and the exit observer's `job/<workload_id>` submit
            // collapses to one dispatch per drain cycle.
            #[allow(clippy::expect_used)]
            {
                let svid_name = ReconcilerName::new(SVID_LIFECYCLE_NAME)
                    .expect("'svid-lifecycle' is a valid ReconcilerName by construction");
                let svid_target = TargetResource::new(&format!("job/{}", desired.workload_id))
                    .expect(
                        "'job/<workload_id>' is a valid TargetResource by construction \
                         (WorkloadId is constructor-validated, prefix is canonical)",
                    );
                actions
                    .push(Action::EnqueueEvaluation { reconciler: svid_name, target: svid_target });
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

/// ADR-0067 D5b — name of the `SvidLifecycle` reconciler.
///
/// Compile-time alias to `<SvidLifecycle as Reconciler>::NAME` — same
/// anti-drift discipline as [`BACKEND_DISCOVERY_BRIDGE_NAME`] /
/// [`SERVICE_LIFECYCLE_NAME`]: renaming the reconciler's `NAME` const
/// without updating this reference is a compile error, not a silent
/// dead handoff (the failure mode D5b exists to prevent).
const SVID_LIFECYCLE_NAME: &str = <super::svid_lifecycle::SvidLifecycle as Reconciler>::NAME;

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

                // backend-instance-replacement step 01-02 (ADR-0073 § 5).
                // `restart_pending` is the generation seam: a replace bumps
                // `desired.generation`, and while the reconciler has not yet
                // placed a fresh instance for it (`observed < desired`) the
                // workload is mid-restart. Computed here so BOTH the
                // running-origin R2 stop (below) and the scoped veto (after
                // the running check) can read it. The `<` comparison — not
                // `<=`/`==` — is load-bearing: a sequential restart that
                // advances `desired` past a previously-stamped `observed`
                // (S-BIR-SEQUENTIAL) re-arms `restart_pending` and re-enters
                // the cycle.
                let restart_pending = view.observed_generation < desired.generation;

                // Is any allocation already Running for this job?
                //
                // R1 (`!restart_pending`): converged — emit nothing.
                //
                // R2 (`restart_pending`, ADR-0073 § 5): a running-origin
                // restart must END the current Running instance before the
                // fresh placement, so the new instance is a genuine
                // REPLACEMENT (end-then-bring-up). Emit one
                // `StopAllocation { terminal: Stopped { by: Operator } }`
                // for the current Running instance. `observed_generation`
                // is NOT stamped on this stop tick — stamping here would
                // re-arm the veto before the fresh instance exists,
                // stranding the workload Terminated (the load-bearing
                // ordering). The placement + stamp happens on a later tick
                // (R3) once the prior instance is Terminated. R5 (no
                // duplicate stop while draining) falls out for free: once
                // the prior instance leaves Running it is no longer matched
                // here, and the broker's `(reconciler, target)` keying +
                // in-flight-action collapse debounce a same-tick re-emit.
                let running_alloc =
                    active_allocs_vec.iter().find(|r| r.state == AllocState::Running);
                if let Some(running) = running_alloc {
                    if restart_pending {
                        let action = Action::StopAllocation {
                            alloc_id: running.alloc_id.clone(),
                            terminal: Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
                        };
                        return (vec![action], view.clone());
                    }
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
                // backend-instance-replacement step 01-02 (ADR-0073 § 5).
                // A replace bumps `desired.generation`. While the
                // reconciler has not yet placed a fresh instance for this
                // generation (`observed < desired` ⇒ `restart_pending`),
                // the current instance's Operator-stop is OVERRIDABLE — the
                // operator's restart intent (the generation advance)
                // supersedes the prior stop. When generations are EQUAL
                // (a same-spec deploy did NOT bump), the veto stands —
                // Bug-3 preserved.
                //
                // CRITICAL: the veto keys off the workload's CURRENT
                // instance only (`current_alloc(...)`, the numerically
                // highest `mint_alloc_id` suffix), NOT
                // `any(is_operator_stopped)` across all history.
                // `mint_alloc_id` deliberately KEEPS the superseded
                // `payments-0 / Terminated{Operator}` row (that retention
                // is how `A1 ≠ A2` is achieved), but an Operator-stop from
                // a SUPERSEDED generation is history, not current intent —
                // it must not veto the current instance's lifecycle (incl.
                // crash-restart of the fresh alloc). Keying off `any(...)`
                // would let a stale superseded row re-arm the veto after
                // the fresh instance is placed and later crashes, wedging
                // the fresh instance forever (the post-iteration-2 bug
                // this scoping fixes). `restart_pending` was computed above
                // the running check (shared with the R2 stop).
                if !restart_pending && current_alloc(&allocs_vec).is_some_and(is_operator_stopped) {
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
                            // D-A1 / D-BLOCKER1 (GH #241): the declared
                            // Service listener ports, projected at
                            // hydrate-desired time. Same clone-from-desired
                            // shape as `probe_descriptors` above. Empty for
                            // Job-kind / Schedule-kind.
                            service_ports: desired.service_ports.clone(),
                            // The reconciler stays netns/veth/addr-AGNOSTIC
                            // (JOIN-2 + D-A1): the slot-derived netns name,
                            // host-veth name, and canonical workload_addr are
                            // runtime slot state injected ONLY at the action-shim
                            // C3 site, never carried in intent (criterion 6's
                            // rebuilt-on-restart model).
                            netns: None,
                            host_veth: None,
                            workload_addr: None,
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
                                // D-A1 / D-BLOCKER1 (GH #241): declared Service
                                // listener ports, projected at hydrate-desired
                                // time — same clone-from-desired shape as
                                // `probe_descriptors`. See the RestartAllocation
                                // spec above.
                                service_ports: desired.service_ports.clone(),
                                // Netns/veth/addr-agnostic reconciler (JOIN-2 +
                                // D-A1) — see the RestartAllocation spec above.
                                netns: None,
                                host_veth: None,
                                workload_addr: None,
                            },
                            kind: desired.workload_kind,
                        };
                        // backend-instance-replacement step 01-02
                        // (ADR-0073 § 5, R3/R4): the placement tick is the
                        // ONLY tick that stamps. When `restart_pending`
                        // (this fresh placement is satisfying a pending
                        // generation), stamp `observed_generation =
                        // desired.generation` — NOT `observed + 1`. The
                        // `= desired` stamp is what makes N pre-placement
                        // restarts coalesce into ONE placement
                        // (S-BIR-COALESCE-PLACE): two bumps to
                        // `desired = 2` over `observed = 0` place once and
                        // stamp `observed = 2`, so the next tick sees
                        // `observed == desired` and does not re-place
                        // (S-BIR-COALESCE-NO-REPLAY). When NOT
                        // `restart_pending` (an ordinary first placement /
                        // resubmit-after-GC), `observed_generation` is left
                        // unchanged.
                        let mut next_view = view.clone();
                        if restart_pending {
                            next_view.observed_generation = desired.generation;
                        }
                        (vec![action], next_view)
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

/// Parse the numeric attempt suffix `<N>` out of a minted alloc id of
/// the form `alloc-<workload>-<N>` (the [`mint_alloc_id`] grammar). A
/// suffix that fails to parse — a non-`mint_alloc_id`-shaped id, or one
/// whose suffix is not a base-10 `u32` — yields `None`.
///
/// Co-located with [`mint_alloc_id`] so the parse and the mint stay in
/// lockstep: the suffix grammar (`-<N>` at the tail) is
/// `mint_alloc_id`-internal, and a future change to the mint format
/// would break this parse loudly at the same site rather than silently
/// elsewhere.
fn alloc_attempt_index(alloc_id: &AllocationId) -> Option<u32> {
    alloc_id.as_str().rsplit_once('-').and_then(|(_, suffix)| suffix.parse::<u32>().ok())
}

/// The workload's **current** instance — the row with the
/// numerically-highest [`mint_alloc_id`] attempt suffix
/// (`alloc-<workload>-<N>`). This is the most-recently-placed instance;
/// every superseded prior generation has a strictly lower suffix.
/// Returns `None` for an empty alloc set.
///
/// **Determination is the NUMERIC max of the parsed `<N>` suffix — NOT
/// the `BTreeMap`/`.values()` iteration order, which is LEXICAL on the
/// raw `AllocationId` string** (`alloc-payments-10` sorts BEFORE
/// `alloc-payments-2`), so "last in iteration" is WRONG once the attempt
/// index reaches double digits (backend-instance-replacement step 01-02,
/// ADR-0073 § 5 / DDD-13). A row whose suffix fails to parse
/// ([`alloc_attempt_index`] → `None`) sorts below any parseable suffix
/// (a defensive floor — never the current instance when a parseable one
/// exists).
///
/// Robust by construction: `mint_alloc_id` mints
/// `attempt = allocs_vec.len()` and the feature relies on alloc rows
/// being **never deleted** (the superseded `payments-0` row is
/// intentionally retained), so the attempt indices are a
/// strictly-increasing `0, 1, 2, …` series and the numeric max is
/// unambiguously the latest placement. Needs no new per-row field (no
/// `generation` on `AllocStatusRow` ⇒ no ADR-0048 envelope bump).
fn current_alloc<'a>(allocs: &[&'a AllocStatusRow]) -> Option<&'a AllocStatusRow> {
    // `max_by_key` over `(Option<u32>, …)` orders `None` below every
    // `Some` (the defensive floor) and breaks ties on the later
    // iteration position — irrelevant here since the never-delete
    // invariant makes attempt indices unique, but deterministic.
    allocs.iter().copied().max_by_key(|row| alloc_attempt_index(&row.alloc_id))
}

/// service-vip-allocator step 03-01 — pure helper for the Service-arm
/// release-emission gate.
///
/// Returns `Some((action, digest))` when:
///
/// 1. `desired.workload_kind == WorkloadKind::Service`, AND
/// 2. `desired.service_spec_digest` is `Some(digest)`, AND
/// 3. `digest` is NOT already present in `view.released_for_deletion`,
///    AND
/// 4. the workload's intent is **withdrawn** — `desired.job.is_none()`
///    (logical-workload deletion).
///
/// Returns `None` otherwise — i.e. for non-Service kinds, when the
/// digest is absent (the runtime hydrator has not populated it), when
/// the workload's intent is still declared (`desired.job.is_some()` —
/// a stopped-or-crashed-but-still-declared Service RETAINS its VIP), or
/// when the digest is already recorded as released.
///
/// **Withhold-not-release** per ADR-0049 (amendment 2026-06-28, D1):
/// the VIP is an identity bound to the DECLARED workload and is released
/// only on intent withdrawal, symmetric with the dial-by-name frontend
/// `F` (ADR-0072 `FrontendAddrAllocator::release` = deletion-only). The
/// gate keys on `desired.job.is_none()` — the same signal the Absent/GC
/// branch in `reconcile_inner` uses — NOT on a terminal alloc,
/// `desired_to_stop`, or any GC/terminal stamp. (Superseded the original
/// release-on-terminal gate; § 6 / amendment 2026-05-15.)
///
/// The caller (the `WorkloadLifecycle::reconcile` wrapper) appends the
/// returned action to the inner reconcile's action list and stamps the
/// returned digest onto `next_view.released_for_deletion` so the next
/// tick short-circuits. Per `.claude/rules/development.md` § "Persist
/// inputs, not derived state".
fn service_vip_release_emission(
    desired: &WorkloadLifecycleState,
    view: &WorkloadLifecycleView,
) -> Option<(Action, crate::id::ContentHash)> {
    if desired.workload_kind != WorkloadKind::Service {
        return None;
    }
    let digest = desired.service_spec_digest?;
    if view.released_for_deletion.contains(&digest) {
        return None;
    }
    // ADR-0049 (amendment 2026-06-28, D1) — withhold-not-release. The
    // VIP is an identity bound to the DECLARED workload; release it only
    // when the workload's intent is withdrawn (logical-workload
    // deletion), detected as `desired.job.is_none()` — the same signal
    // the reconciler's Absent/GC branch (`reconcile_inner`,
    // `match desired.job.as_ref() { None => … }`) keys on. A
    // stopped-or-crashed-but-still-declared Service (`desired.job.is_some()`)
    // RETAINS its VIP, symmetric with the dial-by-name frontend `F`
    // (ADR-0072: `FrontendAddrAllocator::release` = deletion-only).
    //
    // MUST NOT key on `desired_to_stop`, `is_operator_stopped`,
    // `row.terminal`, or the GC terminal stamp — those all fire on
    // stop-while-declared, the exact case that must now retain the VIP.
    // A stop intent (`POST /v1/jobs/{id}/stop`) retains the spec key, so
    // under a stop `desired.job` stays `Some(_)`.
    if desired.job.is_some() {
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
    /// Desired-run generation, hydrated from `workloads/<id>/generation`
    /// (absent ⇒ 0). Bumped by `overdrive workload restart` (the
    /// atomic `TxnOp::IncrementU64` bump-and-clear). Per ADR-0073
    /// § "The six pinned signatures" item 5: the reconciler places a
    /// fresh instance when `view.observed_generation < desired.generation`
    /// (`restart_pending`) and, on the placement tick, stamps
    /// `observed_generation = desired.generation` so subsequent ticks
    /// see `observed == desired` and the current-instance-scoped veto
    /// re-arms for the fresh instance.
    pub generation: u64,
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
    /// Declared Service listener ports projected from the live intent at
    /// hydrate-desired time via [`project_service_listen_ports`]
    /// (canonical-workload-address inbound-TPROXY, D-A1 / D-BLOCKER1, GH
    /// #241). Empty for Job-kind / Schedule-kind / absent intent; a Service
    /// carries its `listeners[].port` set in declaration order. The
    /// reconciler clones this into every emitted `AllocationSpec.service_ports`
    /// at the IDENTICAL site/shape as [`Self::probe_descriptors`] so the
    /// inbound-TPROXY install (step 03-01) receives the declared port set.
    pub service_ports: Vec<std::num::NonZeroU16>,
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

/// Project a workload intent's **declared Service listener ports** for the
/// canonical-workload-address inbound-TPROXY path (D-A1 / D-BLOCKER1, GH
/// #241). Mirrors [`project_probe_descriptors`] one-for-one in shape.
///
/// This is the producer half of the **one-source / two-readers** invariant
/// (D-BLOCKER1): the declared `svc.listeners[].port` set is the SINGLE
/// source the inbound-rule `dport` install (step 03-01) and the
/// `BackendDiscoveryBridge` advertise path (step 02-01) both read. Keeping
/// this projection bottomed-out in `svc.listeners` is load-bearing — if a
/// second path derived the port set from anywhere else, the S-PORTSET
/// equality property (finalized 02-01) would break.
///
/// Per-variant projection:
///
/// - [`crate::aggregate::WorkloadIntent::Service(svc)`] →
///   `svc.listen_ports()` — the operator's declared listener ports in
///   declaration order, read through the single
///   [`crate::aggregate::ServiceV1::listen_ports`] source (D-BLOCKER1).
/// - [`crate::aggregate::WorkloadIntent::Job(_)`] → empty vec (Job-kind has
///   no listener surface; the canonical-address inbound path is a
///   Service-kind concern, same boundary as probes per ADR-0054 §3).
/// - [`crate::aggregate::WorkloadIntent::Schedule(_)`] → empty vec (the
///   schedule's per-fire instance is a Job, not a Service — no listeners).
#[must_use]
pub fn project_service_listen_ports(
    intent: &crate::aggregate::WorkloadIntent,
) -> Vec<std::num::NonZeroU16> {
    match intent {
        crate::aggregate::WorkloadIntent::Job(_)
        | crate::aggregate::WorkloadIntent::Schedule(_) => Vec::new(),
        crate::aggregate::WorkloadIntent::Service(svc) => svc.listen_ports(),
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
    ///
    /// Records the *input* "we already emitted release for this digest;
    /// do not re-emit" (per `.claude/rules/development.md` § "Persist
    /// inputs, not derived state"). Per ADR-0049 (amendment 2026-06-28)
    /// the release fires on logical-workload **deletion**
    /// (`desired.job.is_none()`), not on a terminal alloc — hence the
    /// `_for_deletion` name. The CBOR `#[serde(alias = ...)]` keeps any
    /// pre-rename persisted `released_for_terminal` blob readable
    /// (additive serde evolution per § "Reconciler I/O → Schema
    /// evolution"); the semantics are unchanged.
    #[serde(default, alias = "released_for_terminal")]
    pub released_for_deletion: BTreeSet<crate::id::ContentHash>,
    /// The generation this reconciler has already placed a fresh
    /// instance for. Persisted *input* per `.claude/rules/development.md`
    /// § "Persist inputs, not derived state": the reconciler places when
    /// `observed_generation < desired.generation` and stamps
    /// `observed_generation = desired.generation` once it does (on the
    /// placement tick only — never the stop tick). The `= desired` stamp
    /// (NOT `observed + 1`) is what makes N pre-placement restarts
    /// coalesce into ONE placement by construction (ADR-0073 § 5).
    /// Additive `#[serde(default)]` CBOR field — no rkyv envelope bump
    /// (per § "Reconciler I/O → Schema evolution").
    #[serde(default)]
    pub observed_generation: u64,
}

#[cfg(test)]
mod project_service_listen_ports_tests {
    //! Producer-side unit partition for `project_service_listen_ports`
    //! (canonical-workload-address-inbound-tproxy step 01-02, D-A1 /
    //! D-BLOCKER1). The projection is a pure function — its signature IS
    //! the driving port (port-to-port at the domain layer). It mirrors
    //! `project_probe_descriptors`: a Service projects its declared
    //! listener ports; Job and Schedule each project the empty vec.
    //!
    //! Fixtures build the `Service` arm end-to-end via
    //! `ServiceV1::from_submit` (the parser-side path), so the projection
    //! is exercised against the same `svc.listeners` shape the runtime
    //! hydrate path uses and the bridge reads in 02-01 — keeping the
    //! S-PORTSET equality property structurally honest (D-BLOCKER1: one
    //! source, two readers).

    use std::num::{NonZeroU16, NonZeroU32};

    use proptest::prelude::*;

    use crate::aggregate::{
        CronExpr, DriverInput, Exec, ExecInput, Job, ResourcesInput, ScheduleV1, ServiceV1,
        WorkloadDriver, WorkloadIntent,
    };
    use crate::api::submit::{ListenerInput, ServiceSpecInput};
    use crate::id::WorkloadId;
    use crate::traits::driver::Resources;

    use super::project_service_listen_ports;

    fn wid(s: &str) -> WorkloadId {
        WorkloadId::new(s).expect("valid WorkloadId")
    }

    fn make_job(id: &str) -> Job {
        Job {
            id: wid(id),
            replicas: NonZeroU32::new(1).expect("1 is non-zero"),
            resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
            driver: WorkloadDriver::Exec(Exec { command: "/bin/serve".to_string(), args: vec![] }),
        }
    }

    /// Build a `WorkloadIntent::Service` carrying the given listener
    /// ports (all TCP) via the validating parser-side path.
    fn service_with_ports(ports: &[u16]) -> WorkloadIntent {
        let listeners =
            ports.iter().map(|p| ListenerInput { port: *p, protocol: "tcp".to_string() }).collect();
        let input = ServiceSpecInput {
            id: "svc".to_string(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
            driver: DriverInput::Exec(ExecInput {
                command: "/bin/serve".to_string(),
                args: vec![],
            }),
            listeners,
            startup_probes: vec![],
            readiness_probes: vec![],
            liveness_probes: vec![],
        };
        let svc = ServiceV1::from_submit(input).expect("canonical ServiceSpecInput is valid");
        WorkloadIntent::Service(svc)
    }

    #[test]
    fn service_projects_its_declared_listener_ports() {
        // Declared in DESCENDING order on purpose: an ascending input
        // (e.g. [8080, 9090]) cannot distinguish an order-PRESERVING
        // projection from one that sorts its output — both yield the
        // same vec. A descending declaration makes the "declaration
        // order" claim falsifiable: a sorting projection would yield
        // [8080, 9090] and fail this assertion.
        let intent = service_with_ports(&[9090, 8080]);

        let projected = project_service_listen_ports(&intent);

        let expected: Vec<NonZeroU16> = vec![
            NonZeroU16::new(9090).expect("9090 is non-zero"),
            NonZeroU16::new(8080).expect("8080 is non-zero"),
        ];
        assert_eq!(
            projected, expected,
            "Service must project its listener ports in declaration order \
             (a sorting projection would yield [8080, 9090] and fail here)",
        );
    }

    #[test]
    fn job_kind_projects_the_empty_port_set() {
        let intent = WorkloadIntent::Job(make_job("a-job"));

        let projected = project_service_listen_ports(&intent);

        assert!(
            projected.is_empty(),
            "Job-kind has no listener surface — must project the empty vec, got {projected:?}",
        );
    }

    #[test]
    fn schedule_kind_projects_the_empty_port_set() {
        let intent = WorkloadIntent::Schedule(ScheduleV1 {
            id: wid("a-schedule"),
            job: make_job("a-schedule"),
            cron_expr: CronExpr::new("0 * * * *").expect("valid cron"),
        });

        let projected = project_service_listen_ports(&intent);

        assert!(
            projected.is_empty(),
            "Schedule-kind has no listener surface — must project the empty vec, got {projected:?}",
        );
    }

    proptest! {
        /// Producer side of S-PORTSET (finalized 02-01): over an
        /// arbitrary non-empty set of distinct listener ports, the
        /// projected set equals the declared set. D-BLOCKER1 — the
        /// projection bottoms out in `svc.listeners`, the single source
        /// the inbound-rule `dport` (03-01) and the bridge (02-01) also
        /// read.
        #[test]
        fn service_projection_equals_declared_listener_set(
            ports in prop::collection::btree_set(1u16..=u16::MAX, 1..=8)
        ) {
            let declared: Vec<u16> = ports.iter().copied().collect();
            let intent = service_with_ports(&declared);

            let projected = project_service_listen_ports(&intent);

            let expected: Vec<NonZeroU16> = declared
                .iter()
                .map(|p| NonZeroU16::new(*p).expect("port is non-zero"))
                .collect();
            prop_assert_eq!(
                projected,
                expected,
                "projected listener-port set must equal the declared listener set",
            );
        }
    }
}

#[cfg(test)]
mod service_vip_release_emission_tests {
    //! Direct unit partition for [`service_vip_release_emission`] — the
    //! private release-emission gate. The function signature IS the
    //! driving port (port-to-port at the domain scope), so calling it
    //! directly is the correct port-to-port shape for this seam.
    //!
    //! Pins the **withhold-not-release** trigger per ADR-0049
    //! (amendment 2026-06-28, D1 / crafter-facing design spec): the VIP
    //! is an identity bound to the *declared* workload and is released
    //! ONLY on logical-workload deletion (`desired.job.is_none()`),
    //! NEVER on a transient terminal-alloc while the workload stays
    //! declared. This is symmetric with the dial-by-name frontend `F`
    //! (ADR-0072: `FrontendAddrAllocator::release` = deletion-only).
    //!
    //! `desired.job` is the SSOT for "declared": the production hydrator
    //! (`reconciler_runtime::read_job`) returns `Some(job)` for a
    //! declared Service (the kind-agnostic projection) and `None` only
    //! when the spec intent is absent (withdrawn). A stop intent
    //! (`POST /v1/jobs/{id}/stop`) retains the spec key, so under a stop
    //! `desired.job` stays `Some(_)` — the case that must now RETAIN the
    //! VIP. The gate keys on `desired.job.is_none()` alone, never on
    //! `desired_to_stop`, `is_operator_stopped`, or `row.terminal`.

    use std::collections::{BTreeMap, BTreeSet};
    use std::num::NonZeroU32;

    use crate::aggregate::{Exec, Job, WorkloadDriver, WorkloadKind};
    use crate::id::{ContentHash, WorkloadId};
    use crate::reconcilers::Action;
    use crate::traits::driver::Resources;

    use super::{WorkloadLifecycleState, WorkloadLifecycleView, service_vip_release_emission};

    fn wid(s: &str) -> WorkloadId {
        WorkloadId::new(s).expect("valid WorkloadId")
    }

    fn make_job(id: &str) -> Job {
        Job {
            id: wid(id),
            replicas: NonZeroU32::new(1).expect("1 is non-zero"),
            resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
            driver: WorkloadDriver::Exec(Exec { command: "/bin/serve".to_string(), args: vec![] }),
        }
    }

    fn fixture_digest() -> ContentHash {
        ContentHash::of(b"adr-0049-2026-06-28-withhold-fixture-digest")
    }

    /// Build the `desired` Service state the new release gate reads.
    ///
    /// `declared` selects the intent-presence axis the gate keys on:
    /// - `declared == true`  → `desired.job = Some(_)` (intent retained;
    ///   a stop retains the spec key, so `desired_to_stop` is also true).
    ///   The VIP MUST be retained.
    /// - `declared == false` → `desired.job = None` (intent withdrawn /
    ///   logical deletion). The VIP MUST be released.
    ///
    /// The gate is `actual`-independent (a terminal alloc no longer
    /// triggers release), so the fixture builds only `desired`. The
    /// "terminal alloc present yet still no release" falsifiability is
    /// covered at the reconciler-level acceptance test
    /// (`workload_lifecycle_release_service_vip.rs`), which drives a real
    /// terminal alloc through the full `reconcile`.
    fn desired_service_state(
        workload_id: &str,
        digest: ContentHash,
        declared: bool,
    ) -> WorkloadLifecycleState {
        WorkloadLifecycleState {
            workload_id: wid(workload_id),
            job: if declared { Some(make_job(workload_id)) } else { None },
            // A stop intent retains the spec key, so under a stop both
            // `desired_to_stop == true` AND `desired.job.is_some()`.
            // The new gate MUST ignore `desired_to_stop`.
            desired_to_stop: declared,
            generation: 0,
            nodes: BTreeMap::new(),
            allocations: BTreeMap::new(),
            workload_kind: WorkloadKind::Service,
            service_spec_digest: Some(digest),
            probe_descriptors: Vec::new(),
            service_ports: Vec::new(),
        }
    }

    /// WITHHOLD: a Service whose intent is STILL DECLARED
    /// (`desired.job.is_some()`) MUST NOT emit `ReleaseServiceVip` — the
    /// VIP is an identity retained across the stopped-but-declared window
    /// (ADR-0049 D1), symmetric with the frontend `F`.
    ///
    /// This is the #251 RED → GREEN pin on the production gate. On the
    /// superseded release-on-terminal gate a declared Service with a
    /// terminal alloc returned `Some(ReleaseServiceVip)` (RED — which
    /// evicted the VIP memo mid-stop and broke name-retraction); on the
    /// new release-on-deletion gate a declared Service returns `None`.
    #[test]
    fn declared_service_retains_vip() {
        let digest = fixture_digest();
        let desired = desired_service_state("payments", digest, /* declared */ true);
        let view = WorkloadLifecycleView::default();

        let release = service_vip_release_emission(&desired, &view);

        assert!(
            release.is_none(),
            "a still-declared Service (desired.job.is_some()) MUST retain its VIP — \
             release is deletion-only per ADR-0049 (2026-06-28). got {release:?}",
        );
    }

    /// RELEASE-ON-DELETION: when the spec intent is WITHDRAWN
    /// (`desired.job.is_none()` — the same Absent/GC signal the
    /// reconciler keys on) and the digest is not already released, a
    /// Service MUST emit exactly one `ReleaseServiceVip` carrying that
    /// digest (ADR-0049 D1 positive direction).
    ///
    /// SCOPE: this pins the gate's release-on-deletion **logic**, not a
    /// v1-reachable production path. The `(job = None, digest = Some(_))`
    /// input is NOT producible by the v1 hydrator — `reconciler_runtime::
    /// read_job` returns `intent_digest = None` once the intent is
    /// withdrawn, so `service_spec_digest = None` and the gate's
    /// `let digest = desired.service_spec_digest?;` short-circuits.
    /// Release-on-deletion is therefore **inert** on the v1 convergence
    /// path (ADR-0049 D3); the stop-direction retention
    /// (`declared_service_retains_vip`) is the path that is live today.
    /// The logic pinned here goes live when the deletion verb wires a
    /// hydrate-time digest — tracked in `overdrive-sh/overdrive#211`; see
    /// the inline gap note in
    /// `tests/integration/vip_allocator_lifecycle.rs`.
    #[test]
    fn withdrawn_service_intent_releases_vip() {
        let digest = fixture_digest();
        let desired = desired_service_state("payments", digest, /* declared */ false);
        let view = WorkloadLifecycleView::default();

        let release = service_vip_release_emission(&desired, &view);

        match release {
            Some((Action::ReleaseServiceVip { spec_digest, .. }, recorded)) => {
                assert_eq!(
                    spec_digest, digest,
                    "release action must carry the workload's spec_digest"
                );
                assert_eq!(
                    recorded, digest,
                    "the recorded digest (stamped into released_for_deletion) must match"
                );
            }
            other => panic!(
                "withdrawn Service intent (desired.job.is_none()) MUST emit \
                 ReleaseServiceVip; got {other:?}"
            ),
        }
    }

    /// INERT-IN-V1: the production convergence path. When the intent is
    /// withdrawn, the v1 hydrator (`reconciler_runtime::read_job`) zeroes
    /// BOTH `desired.job` (the trigger) AND `desired.service_spec_digest`
    /// (`read_job` returns `intent_digest = None` once the intent is
    /// absent). With `(job = None, digest = None)` the gate's
    /// `service_spec_digest?` extraction short-circuits and NO release
    /// fires — release-on-deletion is inert in v1 production per ADR-0049
    /// D3, until `#211` wires a deletion verb that supplies the digest at
    /// hydrate time. This pins that inert behavior so a future
    /// digest-persistence change is a deliberate, test-visible decision
    /// (the test flips red the moment a withdrawn intent starts carrying
    /// a digest on the v1 path).
    #[test]
    fn withdrawn_service_without_digest_emits_no_release() {
        let desired = WorkloadLifecycleState {
            workload_id: wid("payments"),
            job: None,
            desired_to_stop: false,
            generation: 0,
            nodes: BTreeMap::new(),
            allocations: BTreeMap::new(),
            workload_kind: WorkloadKind::Service,
            // The v1 hydrator zeroes the digest alongside the intent.
            service_spec_digest: None,
            probe_descriptors: Vec::new(),
            service_ports: Vec::new(),
        };
        let view = WorkloadLifecycleView::default();

        let release = service_vip_release_emission(&desired, &view);

        assert!(
            release.is_none(),
            "a withdrawn Service with no hydrated digest (the v1 production \
             shape) MUST NOT emit ReleaseServiceVip — release-on-deletion is \
             inert until #211 supplies the digest at hydrate time; got {release:?}",
        );
    }

    /// Idempotency short-circuit is unchanged by the gate swap: once the
    /// digest is recorded as already-released, a withdrawn intent does
    /// NOT re-emit. Pins that the rename of the View field preserved the
    /// short-circuit's role.
    #[test]
    fn withdrawn_service_does_not_reemit_when_already_recorded() {
        let digest = fixture_digest();
        let desired = desired_service_state("payments", digest, /* declared */ false);
        let mut released = BTreeSet::new();
        released.insert(digest);
        let view = WorkloadLifecycleView {
            restart_counts: BTreeMap::new(),
            last_failure_seen_at: BTreeMap::new(),
            released_for_deletion: released,
            observed_generation: 0,
        };

        let release = service_vip_release_emission(&desired, &view);

        assert!(
            release.is_none(),
            "re-tick with the digest already in released_for_deletion MUST NOT \
             re-emit ReleaseServiceVip; got {release:?}",
        );
    }
}
