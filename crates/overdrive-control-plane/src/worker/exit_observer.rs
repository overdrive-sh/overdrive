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

use std::sync::Arc;
use std::time::Duration;

use overdrive_core::id::{AllocationId, JobId};
use overdrive_core::reconciler::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType, ExitEvent, ExitKind};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
    ObservationStoreError,
};
use overdrive_core::transition_reason::TransitionReason;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::action_shim::LifecycleEvent;
use crate::api::{AllocStateWire, TransitionSource};
use crate::eval_broker::Evaluation;
use crate::reconciler_runtime::ReconcilerRuntime;

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
        // Drop our `Arc<dyn Driver>` reference now that we've taken the
        // receiver and captured the driver kind. Holding it for the
        // lifetime of the observer task would pin the driver alive
        // across shutdown — its `exit_tx` would never drop and
        // `rx.recv().await` would block forever, leaking the task.
        // The receiver is what carries the exit-event lifetime; the
        // driver Arc is no longer needed.
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
                    if let Some(runtime) = runtime.as_ref() {
                        if let Some(target) = target_for_event(obs.as_ref(), &event.alloc).await {
                            runtime
                                .broker()
                                .submit(Evaluation { reconciler: job_lifecycle_name(), target });
                        }
                    }
                }
                RetryOutcome::NoPriorRow => {
                    // No prior row — event dropped (alloc never
                    // reached Running per the observer's vantage point).
                }
                RetryOutcome::Failed { error, attempts, prior_state } => {
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
    Wrote {
        row: AllocStatusRow,
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
                return RetryOutcome::Wrote { row, prior_state };
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
        // Best-effort job_id: the observer did not successfully read a
        // prior row (or the read pre-empted the write failure path), so
        // the wire-format `JobId` is the same defensive fallback the
        // alloc_id naming convention encodes (`alloc-<jobid>-N`). Phase
        // 1 wires job_id from the AllocationId by stripping the
        // `alloc-` prefix and trailing `-N`; if that parse fails (alloc
        // id was not built by the action shim's standard pattern), use
        // an "unknown" sentinel so the event still broadcasts.
        job_id: extract_job_id_or_unknown(&event.alloc),
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
    }
}

/// Best-effort `JobId` extraction from an [`AllocationId`] when the
/// observer's prior-row read failed. The action shim's allocation IDs
/// follow `alloc-<jobid>-<index>` (e.g. `alloc-payments-0`); strip the
/// known prefix and the trailing `-N` to recover `<jobid>`. Falls back
/// to `unknown-job` if the parse fails.
fn extract_job_id_or_unknown(alloc: &AllocationId) -> JobId {
    let raw = alloc.as_str();
    let stripped = raw.strip_prefix("alloc-").unwrap_or(raw);
    let trimmed = stripped.rsplit_once('-').map_or(stripped, |(prefix, _suffix)| prefix);
    JobId::new(trimmed).unwrap_or_else(|_| {
        #[allow(clippy::expect_used)]
        JobId::new("unknown-job").expect("constant valid job id")
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

    let (state, reason) = classify(&event.kind, event.intentional_stop);
    let updated_at = LogicalTimestamp {
        counter: prior.updated_at.counter.saturating_add(1),
        writer: prior.node_id.clone(),
    };
    let row = AllocStatusRow {
        alloc_id: event.alloc.clone(),
        job_id: prior.job_id,
        node_id: prior.node_id,
        state,
        updated_at,
        reason: Some(reason),
        detail: None,
    };
    obs.write(ObservationRow::AllocStatus(row.clone())).await?;
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
            TransitionReason::DriverInternalError {
                detail: format_crash_detail(*exit_code, *signal),
            },
        ),
    }
}

fn format_crash_detail(exit_code: Option<i32>, signal: Option<i32>) -> String {
    match (exit_code, signal) {
        (Some(c), _) => format!("exit_code={c}"),
        (None, Some(s)) => format!("signal={s}"),
        (None, None) => "unknown_exit".to_owned(),
    }
}

/// Find the LWW-winner row for this alloc — used to recover the
/// (`job_id`, `node_id`) tuple and the prior `LogicalTimestamp` counter.
async fn find_prior_row(
    obs: &dyn ObservationStore,
    alloc: &AllocationId,
) -> Result<Option<AllocStatusRow>, HandleError> {
    Ok(obs.alloc_status_row(alloc).await?)
}

/// Best-effort target derivation from the alloc's prior obs row's
/// `job_id`. Used by the production `spawn_with_runtime` path to
/// re-enqueue the job-lifecycle reconciler after writing a new state.
async fn target_for_event(
    obs: &dyn ObservationStore,
    alloc: &AllocationId,
) -> Option<TargetResource> {
    let row = obs.alloc_status_row(alloc).await.ok()??;
    TargetResource::new(&format!("job/{}", row.job_id)).ok()
}

fn job_lifecycle_name() -> ReconcilerName {
    // Static canonical name; constructed once per submit. Phase 1
    // ships exactly one job-lifecycle reconciler; the name is
    // hardcoded against `crate::job_lifecycle()`'s registration.
    #[allow(clippy::expect_used)]
    ReconcilerName::new("job-lifecycle")
        .expect("job-lifecycle is a valid reconciler name (constant)")
}

/// Build a `LifecycleEvent` from an observer-written `AllocStatusRow`.
/// `prior_state` carries the actual allocation state before this
/// transition; `from` is set to `prior_state` so the event correctly
/// reflects the transition direction. Mirrors
/// `action_shim::build_lifecycle_event`.
fn build_lifecycle_event(
    row: &AllocStatusRow,
    prior_state: AllocStateWire,
    source: TransitionSource,
) -> LifecycleEvent {
    let to_wire: AllocStateWire = row.state.into();
    LifecycleEvent {
        alloc_id: row.alloc_id.clone(),
        job_id: row.job_id.clone(),
        from: prior_state,
        to: to_wire,
        reason: row
            .reason
            .clone()
            .unwrap_or(TransitionReason::DriverInternalError { detail: String::new() }),
        detail: row.detail.clone(),
        source,
        at: format_logical_timestamp(&row.updated_at),
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

// ---------------------------------------------------------------------------
// Classification unit tests — pure-function coverage of the
// `(ExitKind, intentional_stop) → (AllocState, TransitionReason)`
// mapping. The integration tests in
// `crates/overdrive-control-plane/tests/integration/job_lifecycle/
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
    fn crashed_with_exit_code_intentional_false_fails_with_code_detail() {
        let kind = ExitKind::Crashed { exit_code: Some(137), signal: None };
        let (state, reason) = classify(&kind, false);
        assert_eq!(state, AllocState::Failed);
        let TransitionReason::DriverInternalError { detail } = reason else {
            panic!("expected DriverInternalError, got {reason:?}");
        };
        assert_eq!(detail, "exit_code=137");
    }

    #[test]
    fn crashed_with_signal_intentional_false_fails_with_signal_detail() {
        let kind = ExitKind::Crashed { exit_code: None, signal: Some(9) };
        let (state, reason) = classify(&kind, false);
        assert_eq!(state, AllocState::Failed);
        let TransitionReason::DriverInternalError { detail } = reason else {
            panic!("expected DriverInternalError, got {reason:?}");
        };
        assert_eq!(detail, "signal=9");
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
    fn crashed_with_no_code_or_signal_fails_with_unknown_detail() {
        let kind = ExitKind::Crashed { exit_code: None, signal: None };
        let (state, reason) = classify(&kind, false);
        assert_eq!(state, AllocState::Failed);
        let TransitionReason::DriverInternalError { detail } = reason else {
            panic!("expected DriverInternalError, got {reason:?}");
        };
        assert_eq!(detail, "unknown_exit");
    }
}
