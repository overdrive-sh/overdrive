//! `exit_observer` — the worker-side subsystem that consumes
//! `ExitEvent`s from the driver and writes classified `AllocStatusRow`s
//! to the `ObservationStore`.
//!
//! Per RCA `docs/feature/fix-exec-driver-exit-watcher/deliver/rca.md`
//! §Approved fix item 4: the driver owns the `Child` and emits an
//! `ExitEvent` from a per-alloc watcher task on `child.wait()`
//! resolution. The observer maps each event to `AllocState::Terminated`
//! (clean exit OR `intentional_stop = true`) or `AllocState::Failed`
//! (crash; reason carries the exit code or signal name) and writes a
//! row to obs. The reconciler picks the row up on the next convergence
//! tick and re-enqueues a fresh allocation.
//!
//! # Lifecycle event emission
//!
//! After every successful `obs.write`, the observer broadcasts a
//! [`LifecycleEvent`] on `state.lifecycle_events` — the same bus the
//! action shim emits on per architecture.md §10. Two consumer classes
//! depend on this: (a) the slice 02 streaming `submit --watch` handler
//! sees Failed/Terminated transitions in real-time without polling
//! the LWW snapshot, and (b) integration tests subscribe to the bus
//! to assert on transient transitions that LWW cannot preserve (the
//! restart-loop's `Running counter=N+1` write dominates the observer's
//! `Failed counter=2` under same-tick scheduling, so the snapshot
//! cannot be the test's observation surface).
//!
//! # LWW dominance
//!
//! The observer reads the most recent row for the alloc from obs,
//! takes its `LogicalTimestamp.counter`, increments by 1, and reuses
//! the `writer` (`node_id`). The row is correctly ordered against the
//! prior `Running` row under last-write-wins; the action shim's next
//! tick may dominate it, but the broadcasted `LifecycleEvent` is the
//! permanent record of the transition.
//!
//! # `intentional_stop` discriminator
//!
//! `Driver::stop` sets the per-alloc `intentional_stop` flag to `true`
//! BEFORE delivering any termination signal; the watcher reads this
//! flag at exit-classification time and propagates it on the
//! `ExitEvent`. The observer honours `event.intentional_stop` first:
//! when `true`, the exit collapses to `Terminated` regardless of
//! kind; when `false`, `ExitKind::CleanExit` ⇒ `Terminated` and
//! `ExitKind::Crashed` ⇒ `Failed`.

use std::sync::{Arc, Weak};
use std::time::Duration;

use overdrive_core::id::{AllocationId, WorkloadId};
use overdrive_core::reconcilers::backend_discovery_bridge::BackendDiscoveryBridge;
use overdrive_core::reconcilers::{Reconciler, ReconcilerName, TargetResource, WorkloadLifecycle};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, Driver, DriverType, ExitEvent, ExitKind, STDERR_TAIL_LINES,
};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
    ObservationStoreError,
};
use overdrive_core::transition_reason::TransitionReason;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::action_shim::LifecycleEvent;
use crate::api::{AllocStateWire, TransitionSource};
use crate::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::eval_broker::Evaluation;

// STDERR_TAIL_LINES lives at `overdrive_core::traits::driver::STDERR_TAIL_LINES`
// (the single SSOT per ADR-0033 Amendment 2026-05-10 / step 02-05).
// Callers import it directly from that path; no re-export is needed here.

/// Bounded retry budget for transient `ObservationStore::write` failures
/// in `handle_exit_event`. The first attempt is unbudgeted; on a
/// retryable error the loop backs off for the corresponding entry in
/// [`RETRY_BACKOFFS`], retries the whole `handle_exit_event` (so
/// `find_prior_row` re-reads under any concurrent writer), and gives up
/// once every entry is consumed. Total of 4 attempts (1 initial + 3
/// retries). Per RCA `docs/feature/fix-exit-observer-write-retry/
/// deliver/rca.md` §"Approved fix — Option A".
const RETRY_BACKOFFS: [Duration; 3] =
    [Duration::from_millis(50), Duration::from_millis(100), Duration::from_millis(200)];

/// Spawn the `exit_observer` subsystem. The returned task consumes
/// `ExitEvent`s from `driver.take_exit_receiver()`, writes
/// `AllocStatusRow`s to `obs`, and broadcasts `LifecycleEvent`s on
/// `events`.
///
/// # Arguments
///
/// - `obs` — the same `Arc<dyn ObservationStore>` the action shim
///   writes to. Direct sharing means the observer's writes appear in
///   the same row stream every reader (reconciler, gateway, status
///   handler) consumes.
/// - `driver` — the driver instance the observer drains exit events
///   from. The first call to `driver.take_exit_receiver()` returns
///   `Some(receiver)`; subsequent observers spawned against the same
///   driver get `None` and the spawn returns immediately. The test
///   harness wires exactly one observer per driver instance.
/// - `events` — broadcast sender for `LifecycleEvent`s. The same bus
///   the action shim emits on (`state.lifecycle_events`). Subscribers
///   see the observer's transitions interleaved with the shim's.
///
/// # Panics
///
/// None. Send/recv failures are best-effort and logged at `debug`.
///
/// # Returns
///
/// The `JoinHandle` of the spawned task. In production the handle is
/// retained in `ServerHandle` so `shutdown()` can drain the observer
/// task before tearing down `AppState`. Test callers (which spawn
/// directly during harness build) typically discard it — the obs
/// store outlives the task and any final pending events are
/// reconciled at next test boot.
pub fn spawn(
    obs: Arc<dyn ObservationStore>,
    driver: Arc<dyn Driver>,
    events: Arc<broadcast::Sender<LifecycleEvent>>,
    clock: Arc<dyn Clock>,
) -> tokio::task::JoinHandle<()> {
    spawn_with_runtime(obs, driver, events, clock, None, CancellationToken::new())
}

/// Production entry-point.
///
/// Same as [`spawn`] but additionally takes the [`ReconcilerRuntime`]
/// so the observer can re-enqueue an `Evaluation` on the broker after
/// each obs write. Phase 1 single-mode uses a job-lifecycle reconciler
/// that picks up the new row on the next tick; the re-enqueue
/// collapses the latency between exit classification and
/// reconciler-driven recovery from "wait for the next periodic tick"
/// to "next drain cycle, immediately."
///
/// Test callers (`exit_observer.rs` integration tests) drive the
/// convergence loop synchronously and do not need this path; they
/// pass `None` (via [`spawn`]) and let `run_convergence_tick` re-read
/// the row at the next test-driven tick.
///
/// # Shutdown
///
/// The observer task exits when EITHER:
///   - The driver's `exit_tx` is dropped — `rx.recv()` returns `None`
///     and the loop exits naturally. This is the steady-state shape:
///     the driver Arc is dropped when the convergence task and axum
///     router both release their `AppState` clones.
///   - `shutdown_token` is cancelled — the `tokio::select!` resolves
///     the cancellation branch and the loop breaks. This is the
///     fallback shape used by [`crate::ServerHandle::shutdown`]: when
///     a workload is still running at shutdown time (e.g. a `/bin/sleep`
///     watcher hasn't reaped yet, or an in-flight `Driver::stop` was
///     cancelled mid-flight), the watcher keeps `exit_tx` alive and
///     `rx.recv()` would block indefinitely. The cancellation token
///     gives `shutdown` a bounded await on the observer task, so the
///     `ServerHandle` Drop chain reliably runs to completion.
pub fn spawn_with_runtime(
    obs: Arc<dyn ObservationStore>,
    driver: Arc<dyn Driver>,
    events: Arc<broadcast::Sender<LifecycleEvent>>,
    clock: Arc<dyn Clock>,
    runtime: Option<Arc<ReconcilerRuntime>>,
    shutdown_token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let Some(mut rx) = driver.take_exit_receiver() else {
            // Driver already had its receiver consumed (e.g. a prior
            // `spawn` call won the race) or this driver does not emit
            // exit events. Nothing to observe.
            return;
        };
        let driver_kind = driver.r#type();
        // Downgrade to `Weak` BEFORE dropping the strong Arc. Holding a
        // strong reference for the lifetime of the observer task would
        // pin the driver alive across shutdown — its `exit_tx` would
        // never drop and `rx.recv().await` would block forever, leaking
        // the task. A `Weak` reference does NOT pin the driver, so the
        // shutdown path still drops `exit_tx` cleanly when `AppState`
        // releases its strong Arc.
        //
        // Per step 01-03 of `fix-exit-observer-running-gate`: the
        // `Weak` is upgraded transiently inside the
        // `RetryOutcome::Failed` arm to call
        // `Driver::release_for_exit_emission`. This fires the
        // Running-confirmed gate on the May-2 retry-exhaustion-degraded
        // path — required for liveness when a future evolution adds
        // action-shim-side retry on `obs.write(Running)` whose Err leg
        // would otherwise leave the watcher parked on the gate.
        // Today's action shim fires the gate post-Ok directly, so this
        // is belt-and-suspenders: the call is idempotent (the sender is
        // already `take`n on the Ok path) and structurally pins the
        // liveness invariant against future regressions.
        let driver_weak: Weak<dyn Driver> = Arc::downgrade(&driver);
        drop(driver);
        loop {
            let event = tokio::select! {
                biased;
                () = shutdown_token.cancelled() => break,
                event = rx.recv() => match event {
                    Some(event) => event,
                    None => break,
                },
            };
            let outcome = run_with_retry(obs.as_ref(), &event, clock.as_ref()).await;
            match outcome {
                RetryOutcome::Wrote { row, prior_state } => {
                    let lifecycle = build_lifecycle_event(
                        &row,
                        prior_state,
                        TransitionSource::Driver(driver_kind),
                    );
                    if let Err(err) = events.send(lifecycle) {
                        // No subscribers is the normal Phase 1 case;
                        // demote to debug so the no-subscriber path
                        // does not spam the log.
                        tracing::debug!(
                            target: "overdrive::exit_observer",
                            err = %err,
                            "lifecycle event broadcast send returned error (no subscribers?); ignored",
                        );
                    }
                    // The reconciler reads `actual` exclusively from the obs store. On a
                    // write failure the obs row is still `Running`, so re-enqueueing the
                    // reconciler produces no actions — the broker would churn an empty
                    // reconcile cycle (busy-loop trap). The terminal-escalation
                    // `LifecycleEvent` below is the operator-visible signal; the
                    // reconciler is intentionally NOT nudged when the obs row did not
                    // change. Per RCA `docs/evolution/2026-05-02-fix-exit-observer-
                    // write-retry.md` (or `docs/feature/fix-exit-observer-write-retry/
                    // deliver/rca.md` pre-archive).
                    if let Some(runtime) = runtime.as_ref()
                        && let Ok(target) = TargetResource::new(&format!("job/{}", row.workload_id))
                    {
                        runtime.broker().submit(Evaluation {
                            reconciler: workload_lifecycle_name(),
                            target: target.clone(),
                        });
                        // backend-discovery-bridge-service-reachability
                        // step 01-04 — re-enqueue the bridge for the
                        // same workload-scoped target so the next tick
                        // observes the AllocStatusRow transition the
                        // exit observer just wrote (per
                        // architecture.md § 3 step 5: "Broker
                        // re-enqueues `BackendDiscoveryBridge` for the
                        // workload (same enqueue site as
                        // `WorkloadLifecycle`, keyed by
                        // `WorkloadId`)"). The bridge's reconcile
                        // body observes its own prior write via the
                        // dedup fingerprint, so a Running → Failed
                        // transition that removes a backend from the
                        // running set fires a fresh
                        // `Action::WriteServiceBackendRow` with the
                        // updated `[backend]` slice.
                        runtime.broker().submit(Evaluation {
                            reconciler: backend_discovery_bridge_name(),
                            target: target.clone(),
                        });
                        // GAP-9 (Shape C) — also re-enqueue the
                        // service-lifecycle reconciler for the same
                        // workload-scoped target so it observes the
                        // AllocStatusRow transition the exit observer
                        // just wrote (a Running → Failed transition is
                        // exactly an EarlyExit / StartupProbeFailed
                        // witness for a Service alloc).
                        //
                        // UNCONDITIONAL (not kind-gated): the exit
                        // observer holds an `ObservationStore` but NOT
                        // an `IntentStore`, so the persisted
                        // workload-kind discriminator
                        // (`IntentKey::for_workload_kind`) is not
                        // cheaply readable here — kind-gating would
                        // require threading the intent store through
                        // the whole observer subsystem. Per the GAP-9
                        // brief, unconditional enqueue is permitted
                        // here PROVIDED it cannot busy-loop:
                        //
                        //   - For a Job-kind workload,
                        //     `hydrate_actual`'s service-lifecycle arm
                        //     reads `service_spec_from_intent`, which
                        //     returns `None` on a kind mismatch →
                        //     `actual.allocs` is empty → the reconcile
                        //     loop emits 0 actions and returns a
                        //     default `next_view` (empty `observed`).
                        //   - `view_has_backoff_pending`'s
                        //     ServiceLifecycle arm
                        //     (`has_alloc_mid_startup_window`) returns
                        //     false on that empty view.
                        //
                        // So a Job-kind enqueue runs exactly one empty
                        // reconcile and then drains — no re-enqueue, no
                        // churn. The kind-gated WorkloadLifecycle
                        // dual-emit (Shape C in workload_lifecycle.rs)
                        // is the precise first-tick path; this site is
                        // the on-exit nudge.
                        runtime.broker().submit(Evaluation {
                            reconciler: service_lifecycle_name(),
                            target: target.clone(),
                        });
                        // ADR-0067 D5b (producer 2) — also re-enqueue the
                        // svid-lifecycle reconciler for the same
                        // workload-scoped target so an observed exit ticks
                        // it and the `¬running ∧ held → DropSvid` branch
                        // fires (O2 — the node-held leaf private key is
                        // dropped on stop even when the stop is an EXIT,
                        // not an operator-driven `StopAllocation`).
                        //
                        // UNCONDITIONAL (not kind-gated), for the same
                        // reason the GAP-9 service-lifecycle enqueue above
                        // is: the exit observer holds no `IntentStore`, so
                        // the workload-kind discriminator is not cheaply
                        // readable here. Identity is needed by every running
                        // alloc regardless of kind, so unconditional is also
                        // semantically correct — and a spurious enqueue for
                        // an already-dropped / never-held alloc runs exactly
                        // one empty reconcile (the held snapshot has no entry
                        // → `desired ⊇ actual` already → `Noop`) and drains;
                        // it cannot busy-loop.
                        runtime
                            .broker()
                            .submit(Evaluation { reconciler: svid_lifecycle_name(), target });
                    }
                }
                RetryOutcome::NoPriorRow => {
                    // No prior row — event dropped (alloc never
                    // reached Running per the observer's vantage point).
                }
                RetryOutcome::Failed { error, attempts, prior_state } => {
                    // Fires the Running-confirmed gate exposed by
                    // Driver::start. Required for liveness — the
                    // watcher parks on this gate before emitting
                    // ExitEvent. The two firing sites
                    // (post-Running-Ok and post-degraded-escalation)
                    // are jointly load-bearing; missing either leaks
                    // the watcher. Per RCA `docs/feature/fix-exit-
                    // observer-running-gate/deliver/rca.md` (Solution
                    // 1', § "Liveness rail").
                    //
                    // This site fires on the May-2 retry-exhaustion-
                    // degraded escalation path. The action shim's
                    // post-Running-Ok fire is the primary site;
                    // `release_for_exit_emission` is idempotent so
                    // double-fire (action shim Ok then degraded
                    // escalation) is structurally a no-op via
                    // `Option::take` + `oneshot::Sender::send`
                    // consume-self. The `Weak::upgrade` is None when
                    // the driver has already been dropped (shutdown
                    // path); in that case there is no watcher left to
                    // unpark, so the no-op is correct.
                    if let Some(driver) = driver_weak.upgrade() {
                        let handle = AllocationHandle { alloc: event.alloc.clone(), pid: None };
                        driver.release_for_exit_emission(&handle);
                    }
                    tracing::error!(
                        target: "overdrive::exit_observer",
                        alloc = %event.alloc,
                        attempts = attempts,
                        err = ?error,
                        "exit_observer obs write failed after bounded retry; \
                         escalating via degraded LifecycleEvent",
                    );
                    let lifecycle =
                        build_escalation_event(&event, prior_state, &error, attempts, driver_kind);
                    if let Err(err) = events.send(lifecycle) {
                        tracing::debug!(
                            target: "overdrive::exit_observer",
                            err = %err,
                            "degraded lifecycle event broadcast send returned error (no subscribers?); ignored",
                        );
                    }
                }
            }
        }
    })
}

/// Outcome of [`run_with_retry`]: the observer either wrote a successor
/// row (`Wrote`), found no prior row to write a successor against
/// (`NoPriorRow`), or exhausted its retry budget against a terminal /
/// repeatedly-retryable error (`Failed`).
enum RetryOutcome {
    /// `row` is `Box<AllocStatusRow>` (not bare) to keep the enum's
    /// largest-variant size from drifting upward as `AllocStatusRow`
    /// grows additive fields (`reason`, `detail`, `terminal` per
    /// ADR-0032 / ADR-0037). The variant is constructed once per
    /// successful retry and consumed immediately at the call site, so
    /// the boxing cost is a single heap allocation per write — the
    /// alternative is the whole enum carrying ~250+ bytes for every
    /// `NoPriorRow` and `Failed` case as well.
    Wrote {
        row: Box<AllocStatusRow>,
        prior_state: AllocStateWire,
    },
    NoPriorRow,
    Failed {
        error: ObservationStoreError,
        attempts: u32,
        /// Best-effort prior wire state read from `find_prior_row`
        /// before the failed write. Used to set `from` on the degraded
        /// `LifecycleEvent`. `Pending` is the defensive default if the
        /// prior-row read itself failed.
        prior_state: AllocStateWire,
    },
}

/// Drive `handle_exit_event` through the bounded retry budget. Per RCA
/// the retry granularity is the WHOLE `handle_exit_event` (not just the
/// inner `obs.write`) so `find_prior_row` re-reads its `LogicalTimestamp`
/// counter on every attempt — keeping the LWW counter monotonic against
/// any concurrent writer that landed a new row between attempts.
///
/// Backoff uses the injected [`Clock::sleep`] (NOT `tokio::time::sleep`)
/// so DST stays deterministic. Per RCA §"Approved fix — Option A".
async fn run_with_retry(
    obs: &dyn ObservationStore,
    event: &ExitEvent,
    clock: &dyn Clock,
) -> RetryOutcome {
    let mut attempts: u32 = 0;
    loop {
        attempts = attempts.saturating_add(1);
        match handle_exit_event(obs, event).await {
            Ok(Some((row, prior_state))) => {
                return RetryOutcome::Wrote { row: Box::new(row), prior_state };
            }
            Ok(None) => {
                return RetryOutcome::NoPriorRow;
            }
            Err(HandleError::Observation(err)) => {
                let retryable = err.is_retryable();
                let backoff = RETRY_BACKOFFS.get((attempts - 1) as usize).copied();
                let prior_state = read_prior_state(obs, &event.alloc).await;
                if let (true, Some(backoff)) = (retryable, backoff) {
                    tracing::warn!(
                        target: "overdrive::exit_observer",
                        alloc = %event.alloc,
                        attempt = attempts,
                        backoff_ms = u64::try_from(backoff.as_millis()).unwrap_or(u64::MAX),
                        err = ?err,
                        "exit_observer obs write failed (retryable); backing off",
                    );
                    clock.sleep(backoff).await;
                    continue;
                }
                return RetryOutcome::Failed { error: err, attempts, prior_state };
            }
        }
    }
}

/// Best-effort prior-state read for the degraded `LifecycleEvent`'s
/// `from` field. Failures are absorbed to `Pending` (defensive default
/// per RCA: if even the read failed, the observer cannot establish a
/// prior state, but the escalation event must still surface).
async fn read_prior_state(obs: &dyn ObservationStore, alloc: &AllocationId) -> AllocStateWire {
    match obs.alloc_status_row(alloc).await {
        Ok(Some(row)) => row.state.into(),
        Ok(None) | Err(_) => AllocStateWire::Pending,
    }
}

/// Synthesize a degraded `LifecycleEvent` for the terminal-escalation
/// path (RCA §"Approved fix — Option A" item 3). Carries
/// `TransitionReason::DriverInternalError { detail }` where `detail`
/// names the underlying obs-store error and the attempt count, so
/// `submit --watch` subscribers see the failure surface rather than an
/// alloc silently stuck `Running`.
fn build_escalation_event(
    event: &ExitEvent,
    prior_state: AllocStateWire,
    error: &ObservationStoreError,
    attempts: u32,
    driver_kind: DriverType,
) -> LifecycleEvent {
    let detail = format!("obs write failed after {attempts} attempts: {error}");
    LifecycleEvent {
        alloc_id: event.alloc.clone(),
        // Best-effort workload_id: the observer did not successfully read a
        // prior row (or the read pre-empted the write failure path), so
        // the wire-format `WorkloadId` is the same defensive fallback the
        // alloc_id naming convention encodes (`alloc-<jobid>-N`). Phase
        // 1 wires workload_id from the AllocationId by stripping the
        // `alloc-` prefix and trailing `-N`; if that parse fails (alloc
        // id was not built by the action shim's standard pattern), use
        // an "unknown" sentinel so the event still broadcasts.
        workload_id: extract_job_id_or_unknown(&event.alloc),
        from: prior_state,
        to: AllocStateWire::Failed,
        reason: TransitionReason::DriverInternalError { detail },
        detail: None,
        source: TransitionSource::Driver(driver_kind),
        // Synthesized timestamp — the observer never landed a writer-
        // owned `LogicalTimestamp` for this transition because the
        // write itself failed. Mirror the `format_logical_timestamp`
        // shape with an "escalation" sentinel writer so downstream
        // parsers tolerate it.
        at: format!("escalation@{}", event.alloc),
        // Per ADR-0037 §4 the exit observer is NOT a reconciler tick
        // — it runs in a per-allocation watcher task and emits
        // `terminal: None` to express "I am not making a terminal
        // claim." The reconciler is the single writer for terminal
        // decisions; it will see this row on the next tick and emit
        // `Action::FinalizeFailed` if appropriate.
        terminal: None,
    }
}

/// Best-effort `WorkloadId` extraction from an [`AllocationId`] when the
/// observer's prior-row read failed. The action shim's allocation IDs
/// follow `alloc-<jobid>-<index>` (e.g. `alloc-payments-0`); strip the
/// known prefix and the trailing `-N` to recover `<jobid>`. Falls back
/// to `unknown-job` if the parse fails.
fn extract_job_id_or_unknown(alloc: &AllocationId) -> WorkloadId {
    let raw = alloc.as_str();
    let stripped = raw.strip_prefix("alloc-").unwrap_or(raw);
    let trimmed = stripped.rsplit_once('-').map_or(stripped, |(prefix, _suffix)| prefix);
    WorkloadId::new(trimmed).unwrap_or_else(|_| {
        #[allow(clippy::expect_used)]
        WorkloadId::new("unknown-job").expect("constant valid job id")
    })
}

/// Map an `ExitEvent` to an `AllocStatusRow` and write it to `obs`.
/// Returns the written row and the prior state as `AllocStateWire` on
/// success, so the caller can set the correct `from` field on the
/// resulting `LifecycleEvent`. Returns `Ok(None)` when no prior row
/// exists for the alloc (the alloc never reached Running per the
/// observer's vantage point — only possible under racy injection in
/// tests; production drivers always emit Running through the action
/// shim before any exit event can fire).
async fn handle_exit_event(
    obs: &dyn ObservationStore,
    event: &ExitEvent,
) -> Result<Option<(AllocStatusRow, AllocStateWire)>, HandleError> {
    let prior = find_prior_row(obs, &event.alloc).await?;
    let Some(prior) = prior else {
        return Ok(None);
    };
    let prior_state: AllocStateWire = prior.state.into();

    let (state, mut reason) = classify(&event.kind, event.intentional_stop);
    let updated_at = LogicalTimestamp {
        counter: prior.updated_at.counter.saturating_add(1),
        writer: prior.node_id.clone(),
    };
    // Bound the stderr_tail to the project-wide line budget at the
    // observation seam. The driver-side ring buffer in `ExecDriver`
    // already caps emission at `STDERR_TAIL_LINES`, but the observer
    // is the canonical defence-in-depth: any future driver impl that
    // emits a longer tail (test injection, alternate driver) is
    // truncated here before reaching the obs row.
    let stderr_tail =
        event.stderr_tail.as_deref().map(|raw| keep_last_n_lines(raw, STDERR_TAIL_LINES));
    // Patch stderr_tail into WorkloadCrashedImmediately so the typed
    // reason carries the same tail the row carries — both are
    // observation data and denormalisation is intentional here.
    if let TransitionReason::WorkloadCrashedImmediately { stderr_tail: ref mut tail, .. } = reason {
        tail.clone_from(&stderr_tail);
    }
    let row = AllocStatusRow {
        alloc_id: event.alloc.clone(),
        workload_id: prior.workload_id,
        node_id: prior.node_id,
        state,
        updated_at,
        reason: Some(reason),
        detail: None,
        // ADR-0037 §4: emission sites outside a reconciler tick (the
        // exit observer is one — it runs in a per-allocation watcher
        // task, not a reconcile loop) MUST emit `terminal: None`.
        // Structurally meaningful: "I am not making a terminal claim";
        // the reconciler is the single writer for terminal decisions.
        terminal: None,
        stderr_tail,
        // Phase-1 greenfield (ADR-0047 §4 / step 02-02 [D4]): the
        // exit observer inherits the prior row's kind so the
        // denormalised field stays accurate across the workload's
        // lifetime. The exit observer never invents a kind — it
        // always has a `prior` row in scope (loaded above) whose
        // `kind` is the authoritative value written at submit time.
        kind: prior.kind,
        listeners: Vec::new(),
        // Subsidiary GAP-1 fix: the exit observer is a successor-row
        // writer for a Terminated / Failed transition — by definition
        // the alloc MUST have reached Running at some point (the
        // exit-observer watcher is only ever armed after the action
        // shim records Pending → Running). Preserve the prior row's
        // `started_at` verbatim. Same forward-carry pattern as
        // `stderr_tail` / `kind` / `workload_id` / `node_id`.
        started_at: prior.started_at,
        // canonical-workload-address-inbound-tproxy (GH #241 /
        // AllocStatusRowV2): the exit observer is a successor-row
        // writer — forward-carry the prior row's canonical workload
        // address verbatim, same pattern as `started_at` / `kind`.
        // `None` for host-netns allocs (every current fixture); the
        // initial population off `spec.workload_addr` at the
        // Pending → Running write lands in a later slice (BLOCKER-2
        // exit-observer/status-write extend).
        workload_addr: prior.workload_addr,
    };
    obs.write(ObservationRow::AllocStatus(Box::new(row.clone()))).await?;
    Ok(Some((row, prior_state)))
}

/// Map `(ExitKind, intentional_stop)` to the typed obs row state +
/// transition reason. The `intentional_stop` flag wins: any operator
/// stop classifies as `Terminated::{by: Operator}` regardless of the
/// underlying kernel exit shape.
fn classify(kind: &ExitKind, intentional_stop: bool) -> (AllocState, TransitionReason) {
    if intentional_stop {
        return (
            AllocState::Terminated,
            TransitionReason::Stopped {
                by: overdrive_core::transition_reason::StoppedBy::Operator,
            },
        );
    }
    match kind {
        ExitKind::CleanExit => (
            AllocState::Terminated,
            TransitionReason::Stopped { by: overdrive_core::transition_reason::StoppedBy::Process },
        ),
        ExitKind::Crashed { exit_code, signal } => (
            AllocState::Failed,
            TransitionReason::WorkloadCrashedImmediately {
                exit_code: *exit_code,
                // Signal numbers on Linux/POSIX are 1–64, always within u8 range.
                // `u8::try_from` encodes that assumption and saturates to u8::MAX
                // for any out-of-range value the kernel cannot actually produce.
                signal: signal.map(|s| u8::try_from(s).unwrap_or(u8::MAX)),
                stderr_tail: None,
            },
        ),
    }
}

/// Find the LWW-winner row for this alloc — used to recover the
/// (`workload_id`, `node_id`) tuple and the prior `LogicalTimestamp` counter.
async fn find_prior_row(
    obs: &dyn ObservationStore,
    alloc: &AllocationId,
) -> Result<Option<AllocStatusRow>, HandleError> {
    Ok(obs.alloc_status_row(alloc).await?)
}

fn workload_lifecycle_name() -> ReconcilerName {
    // Sourced from the trait const per the
    // `refactor-reconciler-static-name` RCA: `WorkloadLifecycle::NAME` is
    // the single compile-time anchor for the kebab-case literal, so
    // there is exactly one place to change if the canonical name ever
    // moves. The `expect` is by-construction-safe — the validator's
    // `^[a-z][a-z0-9-]{0,62}$` grammar accepts `"job-lifecycle"` and
    // the per-call `ReconcilerName::new` invocation is the same shape
    // `WorkloadLifecycle::canonical()` itself uses.
    #[allow(clippy::expect_used)]
    ReconcilerName::new(<WorkloadLifecycle as Reconciler>::NAME)
        .expect("WorkloadLifecycle::NAME is a valid ReconcilerName by construction")
}

/// Canonical `ReconcilerName` for the bridge — sourced from the
/// trait const per the same `refactor-reconciler-static-name` RCA
/// pattern as [`workload_lifecycle_name`]. Used by the exit
/// observer's re-enqueue site to fan out to BOTH the workload
/// lifecycle reconciler AND the backend-discovery bridge for the
/// same workload-scoped `AllocStatusRow` transition, per
/// architecture.md § 3 step 5 of the backend-discovery-bridge
/// feature.
fn backend_discovery_bridge_name() -> ReconcilerName {
    #[allow(clippy::expect_used)]
    ReconcilerName::new(<BackendDiscoveryBridge as Reconciler>::NAME)
        .expect("BackendDiscoveryBridge::NAME is a valid ReconcilerName by construction")
}

/// GAP-9 — canonical `ReconcilerName` for the service-lifecycle
/// reconciler, sourced from the trait const per the same
/// `refactor-reconciler-static-name` RCA pattern as
/// [`workload_lifecycle_name`] / [`backend_discovery_bridge_name`].
///
/// Used by the exit observer's on-exit re-enqueue site (Shape C) to
/// nudge the service-lifecycle reconciler against the
/// `AllocStatusRow` transition the observer just wrote.
fn service_lifecycle_name() -> ReconcilerName {
    #[allow(clippy::expect_used)]
    ReconcilerName::new(
        <overdrive_core::service_lifecycle::ServiceLifecycleReconciler as Reconciler>::NAME,
    )
    .expect("ServiceLifecycleReconciler::NAME is a valid ReconcilerName by construction")
}

/// ADR-0067 D5b (producer 2) — canonical `ReconcilerName` for the
/// `svid-lifecycle` reconciler, sourced from the trait const per the
/// same `refactor-reconciler-static-name` RCA pattern as
/// [`workload_lifecycle_name`] / [`backend_discovery_bridge_name`] /
/// [`service_lifecycle_name`].
///
/// Used by the exit observer's on-exit re-enqueue site to nudge the
/// SVID reconciler against the `AllocStatusRow` transition the observer
/// just wrote, so an observed exit drops the held leaf key (O2).
fn svid_lifecycle_name() -> ReconcilerName {
    #[allow(clippy::expect_used)]
    ReconcilerName::new(
        <overdrive_core::reconcilers::svid_lifecycle::SvidLifecycle as Reconciler>::NAME,
    )
    .expect("SvidLifecycle::NAME is a valid ReconcilerName by construction")
}

/// Build a `LifecycleEvent` from an observer-written `AllocStatusRow`.
/// `prior_state` carries the actual allocation state before this
/// transition; `from` is set to `prior_state` so the event correctly
/// reflects the transition direction. Mirrors
/// `action_shim::build_lifecycle_event`.
///
/// Per ADR-0037 §4: the event's `terminal` field mirrors the row's
/// `terminal` field (which the exit observer always sets to `None`,
/// see `handle_exit_event`). The reconciler is the single writer for
/// terminal claims; the exit observer's job is to record what the
/// kernel observed (clean exit / crash) and let the reconciler decide
/// terminal classification on the next tick.
fn build_lifecycle_event(
    row: &AllocStatusRow,
    prior_state: AllocStateWire,
    source: TransitionSource,
) -> LifecycleEvent {
    let to_wire: AllocStateWire = row.state.into();
    LifecycleEvent {
        alloc_id: row.alloc_id.clone(),
        workload_id: row.workload_id.clone(),
        from: prior_state,
        to: to_wire,
        reason: row
            .reason
            .clone()
            .unwrap_or(TransitionReason::DriverInternalError { detail: String::new() }),
        detail: row.detail.clone(),
        source,
        at: format_logical_timestamp(&row.updated_at),
        terminal: row.terminal.clone(),
    }
}

/// Render a `LogicalTimestamp` as `counter@writer` for the wire/event
/// surface. Mirrors `action_shim::format_logical_timestamp`.
fn format_logical_timestamp(ts: &LogicalTimestamp) -> String {
    format!("{}@{}", ts.counter, ts.writer.as_str())
}

#[derive(Debug, thiserror::Error)]
enum HandleError {
    #[error("observation store rejected exit row: {0}")]
    Observation(#[from] overdrive_core::traits::observation_store::ObservationStoreError),
}

/// Truncate a multi-line string to its last `n` lines, joined by `\n`.
/// Pure helper used at the observation seam (defence-in-depth against
/// a driver emitting more than [`STDERR_TAIL_LINES`]) and exercised
/// directly by `keep_last_n_lines_tests`. The returned string carries
/// no trailing newline; rendering layers that want one append it.
///
/// Invariants:
/// - `keep_last_n_lines("", _)` returns `""`.
/// - For inputs with ≤ `n` lines, the original line ordering is
///   preserved verbatim (without any artificial trailing newline).
/// - For inputs with > `n` lines, the LAST `n` lines are kept in
///   their original order — the stderr_tail must read top-to-bottom
///   like the workload's actual exit-time stderr trailer.
fn keep_last_n_lines(input: &str, n: usize) -> String {
    if input.is_empty() || n == 0 {
        return String::new();
    }
    let lines: Vec<&str> = input.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

// ---------------------------------------------------------------------------
// Classification unit tests — pure-function coverage of the
// `(ExitKind, intentional_stop) → (AllocState, TransitionReason)`
// mapping. The integration tests in
// `crates/overdrive-control-plane/tests/integration/workload_lifecycle/
// exit_observer.rs` cover the end-to-end obs-write path.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod classify_tests {
    use super::*;
    use overdrive_core::transition_reason::StoppedBy;

    #[test]
    fn clean_exit_intentional_false_terminates_with_process_stop() {
        let (state, reason) = classify(&ExitKind::CleanExit, false);
        assert_eq!(state, AllocState::Terminated);
        assert!(matches!(reason, TransitionReason::Stopped { by: StoppedBy::Process }));
    }

    #[test]
    fn clean_exit_intentional_true_terminates_with_operator_stop() {
        let (state, reason) = classify(&ExitKind::CleanExit, true);
        assert_eq!(state, AllocState::Terminated);
        assert!(matches!(reason, TransitionReason::Stopped { by: StoppedBy::Operator }));
    }

    #[test]
    fn crashed_with_exit_code_intentional_false_fails_with_typed_reason() {
        let kind = ExitKind::Crashed { exit_code: Some(137), signal: None };
        let (state, reason) = classify(&kind, false);
        assert_eq!(state, AllocState::Failed);
        assert_eq!(
            reason,
            TransitionReason::WorkloadCrashedImmediately {
                exit_code: Some(137),
                signal: None,
                stderr_tail: None,
            },
            "crashed with exit_code=137 must emit WorkloadCrashedImmediately with typed fields",
        );
    }

    #[test]
    fn crashed_with_signal_intentional_false_fails_with_typed_reason() {
        let kind = ExitKind::Crashed { exit_code: None, signal: Some(9) };
        let (state, reason) = classify(&kind, false);
        assert_eq!(state, AllocState::Failed);
        assert_eq!(
            reason,
            TransitionReason::WorkloadCrashedImmediately {
                exit_code: None,
                signal: Some(9u8),
                stderr_tail: None,
            },
            "crashed with signal=9 must emit WorkloadCrashedImmediately with typed signal field",
        );
    }

    #[test]
    fn crashed_intentional_true_terminates_with_operator_stop() {
        // operator stop wins — even a crash classifies as terminated
        // when intentional_stop was set first.
        let kind = ExitKind::Crashed { exit_code: Some(1), signal: None };
        let (state, reason) = classify(&kind, true);
        assert_eq!(state, AllocState::Terminated);
        assert!(matches!(reason, TransitionReason::Stopped { by: StoppedBy::Operator }));
    }

    #[test]
    fn crashed_with_no_code_or_signal_fails_with_typed_reason() {
        let kind = ExitKind::Crashed { exit_code: None, signal: None };
        let (state, reason) = classify(&kind, false);
        assert_eq!(state, AllocState::Failed);
        assert_eq!(
            reason,
            TransitionReason::WorkloadCrashedImmediately {
                exit_code: None,
                signal: None,
                stderr_tail: None,
            },
            "crashed with no exit code or signal must emit WorkloadCrashedImmediately with all None fields",
        );
    }
}

// ---------------------------------------------------------------------------
// `keep_last_n_lines` — pure helper used by the driver-side stderr ring
// buffer to retain only the last N captured lines for inclusion on the
// `ExitEvent`. Per slice 02c (step 02-05) of `workload-kind-discriminator`
// per ADR-0033 Amendment 2026-05-10. `STDERR_TAIL_LINES = 5` is the
// project-wide SSOT for "how many lines"; it lives on the trait surface
// (`overdrive_core::traits::driver::STDERR_TAIL_LINES`) so both the
// driver (which fills the ring) and the observer (which reads the row)
// reference one constant.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod keep_last_n_lines_tests {
    use super::keep_last_n_lines;

    #[test]
    fn input_longer_than_budget_keeps_last_n_in_order() {
        let input = "ERR 1\nERR 2\nERR 3\nERR 4\nERR 5\nERR 6\nERR 7\n";
        let kept = keep_last_n_lines(input, 5);
        let lines: Vec<&str> = kept.lines().collect();
        assert_eq!(lines, vec!["ERR 3", "ERR 4", "ERR 5", "ERR 6", "ERR 7"]);
    }

    #[test]
    fn input_shorter_than_budget_keeps_all_lines_in_order() {
        let input = "first\nsecond\nthird\n";
        let kept = keep_last_n_lines(input, 5);
        let lines: Vec<&str> = kept.lines().collect();
        assert_eq!(lines, vec!["first", "second", "third"]);
    }

    #[test]
    fn empty_input_yields_empty_string() {
        let kept = keep_last_n_lines("", 5);
        assert_eq!(kept, "");
    }

    #[test]
    fn input_without_trailing_newline_still_kept() {
        // A workload that exits before flushing a final newline still
        // produces a usable tail.
        let input = "ERR 1\nERR 2\nERR 3";
        let kept = keep_last_n_lines(input, 5);
        let lines: Vec<&str> = kept.lines().collect();
        assert_eq!(lines, vec!["ERR 1", "ERR 2", "ERR 3"]);
    }
}
