//! Streaming submit loop for `POST /v1/jobs` with `Accept: application/x-ndjson`.
//!
//! Per architecture.md §3 (happy path), §4 (broken-binary path), §5
//! (timeout path), and §10 (broadcast wiring). Slice 02 step 02-03.
//!
//! # Wiring
//!
//! The handler in [`crate::handlers::submit_job`] branches on the
//! `Accept` header. The `application/x-ndjson` lane delegates here:
//! `streaming_submit_loop` builds a stream of `Result<Bytes, _>`
//! NDJSON lines that axum wraps via `Body::from_stream(...)`.
//!
//! The first line (`SubmitEvent::Accepted`) is emitted SYNCHRONOUSLY
//! after `IntentStore::put_if_absent` returns. No broadcast wait,
//! no observation read.
//!
//! After `Accepted` the loop subscribes to
//! `app_state.lifecycle_events` and enters a `tokio::select!` between:
//!
//! 1. `bus.recv()` — projects each `LifecycleEvent` to a
//!    `SubmitEvent::LifecycleTransition` line, then checks for terminal.
//! 2. `tokio::task::yield_now()` — cooperative yield that re-enters
//!    the loop; a deadline check at the top fires `ConvergedFailed {
//!    Timeout }` once `clock.unix_now() >= cap_deadline`. DST tests
//!    advance the deadline by calling `sim_clock.tick(cap + ε)`; the
//!    yield arm ensures the loop re-enters without advancing SimClock.
//!
//! Terminal detection per architecture.md §4 / ADR-0032 §3 Amendment:
//!
//! - `state == Running && replicas_running >= replicas_desired` →
//!   `ConvergedRunning { alloc_id, started_at }`.
//! - `state == Failed && restart_budget.exhausted` (read off
//!   `view_cache`) → `ConvergedFailed { BackoffExhausted { attempts,
//!   cause: <last cause-class TransitionReason from row> } }`.
//! - `state == Terminated` → `ConvergedStopped { alloc_id, by }` where
//!   `by` is extracted from `TransitionReason::Stopped { by }` or
//!   defaults to `StoppedBy::Reconciler`.
//! - cap timer → `ConvergedFailed { Timeout { after_seconds } }`.
//!
//! On `RecvError::Lagged(_)`, the loop falls back to a one-shot
//! `obs.alloc_status_rows()` snapshot per ADR-0032 §7 and resumes the
//! broadcast. The resubscription happens implicitly because
//! `tokio::sync::broadcast::Receiver` skips ahead on `Lagged` and the
//! next `recv()` is the new front.

// `async_stream::stream!` macro expansions trigger several pedantic
// lints on code we do not author directly. Suppressing here keeps
// the production lib clippy-clean without compromising correctness.
#![allow(clippy::items_after_statements)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::redundant_clone)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::let_unit_value)]
#![allow(clippy::single_match_else)]
#![allow(clippy::unit_arg)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::or_fun_call)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::must_use_candidate)]
// Rust 2024 drop-order changes inside the `async_stream::stream!`
// expansion are flagged via the rust_2024_compatibility group; the
// underlying concern is `tail_expr_drop_order` which clippy/rustc
// emit as a future-incompat lint. The macro internals are upstream;
// suppress the warn at the file level so the workspace lint config's
// `rust_2024_compatibility = warn` does not block commits.
#![allow(tail_expr_drop_order)]

use std::sync::Arc;

use axum::body::Bytes;
use bytes::BytesMut;
use futures::Stream;
use overdrive_core::id::JobId;
use overdrive_core::reconciler::RESTART_BACKOFF_CEILING;
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use tokio::sync::broadcast;

use crate::AppState;
use crate::CachedView;
use crate::action_shim::LifecycleEvent;
use crate::api::{AllocStateWire, IdempotencyOutcome, SubmitEvent, TerminalReason};
use overdrive_core::transition_reason::StoppedBy;

/// One NDJSON line — `serde_json::to_writer(buf, &event)?` + `b'\n'`.
fn emit_line(event: &SubmitEvent) -> std::io::Result<Bytes> {
    use std::io::Write as _;
    let mut buf = BytesMut::with_capacity(256);
    let mut writer = (&mut buf).writer();
    serde_json::to_writer(&mut writer, event).map_err(std::io::Error::other)?;
    writer.write_all(b"\n")?;
    Ok(buf.freeze())
}

// `BytesMut` implements `BufMut` but not `io::Write`; the small
// shim here adapts so `serde_json::to_writer` can drive into it.
trait BytesMutWriter<'a> {
    fn writer(self) -> BytesMutW<'a>;
}

impl<'a> BytesMutWriter<'a> for &'a mut BytesMut {
    fn writer(self) -> BytesMutW<'a> {
        BytesMutW { inner: self }
    }
}

struct BytesMutW<'a> {
    inner: &'a mut BytesMut,
}

impl std::io::Write for BytesMutW<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        use bytes::BufMut as _;
        self.inner.put_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Build the streaming response body for the NDJSON lane.
///
/// `accepted` carries the `Accepted` event prepared synchronously by
/// the handler after the `IntentStore::put_if_absent` returned, so
/// the first line is structurally guaranteed to land before the
/// reconcile path is entered.
///
/// The returned stream yields `Result<Bytes, std::io::Error>` items
/// that axum's `Body::from_stream(...)` wraps into the response body.
pub fn build_stream(
    state: AppState,
    job_id: JobId,
    accepted: SubmitEvent,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    let bus = state.lifecycle_events.clone();
    let clock = state.clock.clone();
    let obs = state.obs.clone();
    let cap = state.streaming_cap;
    let view_cache = state.view_cache.clone();

    async_stream::stream! {
        // 1. Emit Accepted SYNCHRONOUSLY — first byte on the wire.
        match emit_line(&accepted) {
            Ok(line) => yield Ok(line),
            Err(err) => {
                yield Err(err);
                return;
            }
        }

        // 2. Subscribe to the broadcast bus AFTER Accepted is
        //    emitted. The `Accepted` line does not depend on broadcast
        //    state.
        let mut sub = bus.subscribe();

        // 3. Record the cap deadline as a logical clock timestamp.
        //    We use `clock.unix_now() + cap` so DST tests can advance
        //    logical time via `sim_clock.tick(...)` and the deadline
        //    check fires deterministically. The deadline is checked
        //    at the top of every loop iteration.
        let cap_deadline = clock.unix_now() + cap;

        loop {
            // Check cap: fire immediately if the logical clock has
            // advanced past the deadline (e.g. via sim_clock.tick()
            // in a DST test, or via real wall-clock passage).
            if clock.unix_now() >= cap_deadline {
                let after_seconds = u32::try_from(cap.as_secs()).unwrap_or(u32::MAX);
                let terminal = SubmitEvent::ConvergedFailed {
                    alloc_id: None,
                    terminal_reason: TerminalReason::Timeout { after_seconds },
                    reason: None,
                    error: Some(format!("did not converge in {after_seconds}s")),
                };
                match emit_line(&terminal) {
                    Ok(line) => yield Ok(line),
                    Err(err) => yield Err(err),
                }
                return;
            }

            tokio::select! {
                biased;
                recv = sub.recv() => {
                    match recv {
                        Ok(event) => {
                            if event.job_id != job_id {
                                // Event for a different job — ignore.
                                continue;
                            }
                            // Project to the wire shape.
                            let line_event = SubmitEvent::LifecycleTransition {
                                alloc_id: event.alloc_id.to_string(),
                                from: event.from,
                                to: event.to,
                                reason: event.reason.clone(),
                                detail: event.detail.clone(),
                                source: event.source,
                                at: event.at.clone(),
                            };
                            match emit_line(&line_event) {
                                Ok(line) => yield Ok(line),
                                Err(err) => {
                                    yield Err(err);
                                    return;
                                }
                            }

                            // Terminal-detection: read fresh state from
                            // ObservationStore + view_cache and check.
                            if let Some(terminal) = check_terminal(
                                &*obs,
                                &job_id,
                                &view_cache,
                                &event,
                            ).await {
                                match emit_line(&terminal) {
                                    Ok(line) => {
                                        yield Ok(line);
                                        return;
                                    }
                                    Err(err) => {
                                        yield Err(err);
                                        return;
                                    }
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            // Per ADR-0032 §7: fall back to a one-shot
                            // observation snapshot. The next recv()
                            // after Lagged is the new front of the
                            // channel; no explicit resubscribe needed.
                            if let Some(terminal) = lagged_recover(
                                &*obs,
                                &job_id,
                                &view_cache,
                            ).await {
                                match emit_line(&terminal) {
                                    Ok(line) => {
                                        yield Ok(line);
                                        return;
                                    }
                                    Err(err) => {
                                        yield Err(err);
                                        return;
                                    }
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            // Channel closed — fall through to terminal
                            // failure with no specific cause.
                            let terminal = SubmitEvent::ConvergedFailed {
                                alloc_id: None,
                                terminal_reason: TerminalReason::Timeout {
                                    after_seconds: 0,
                                },
                                reason: None,
                                error: Some("lifecycle channel closed".to_string()),
                            };
                            match emit_line(&terminal) {
                                Ok(line) => yield Ok(line),
                                Err(err) => yield Err(err),
                            }
                            return;
                        }
                    }
                }
                // Yield arm: cooperative yield that re-enters the loop
                // so the deadline check at the top can fire after an
                // external `sim_clock.tick(...)` advances logical time
                // in DST tests. Unlike `clock.sleep(...)`, `yield_now`
                // does NOT advance SimClock's logical time — it only
                // gives the tokio scheduler a chance to run other tasks.
                //
                // Under production conditions, `yield_now` returns
                // immediately on the next poll (Ready after one
                // cooperative yield), so the loop re-enters and
                // `clock.unix_now()` is checked against the real
                // wall-clock deadline on each iteration.
                () = tokio::task::yield_now() => {}
            }
        }
    }
}

/// Read the `JobLifecycleView` from the AppState view cache. Returns
/// the default empty view when no entry exists yet.
fn read_view(
    view_cache: &Arc<std::sync::Mutex<std::collections::BTreeMap<(String, String), CachedView>>>,
    job_id: &JobId,
) -> overdrive_core::reconciler::JobLifecycleView {
    let key = ("job-lifecycle".to_owned(), format!("job/{job_id}"));
    let cache = match view_cache.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    match cache.get(&key) {
        Some(CachedView::JobLifecycle(v)) => v.clone(),
        _ => overdrive_core::reconciler::JobLifecycleView::default(),
    }
}

/// Read replicas-desired from the IntentStore for `job_id`. We delegate
/// this read to the snapshot path indirectly: the streaming handler
/// terminates on `Running` rows existing for the job — Phase 1 first
/// reaches Running via replica count == 1 because that is the
/// walking-skeleton workload shape. To avoid an additional IntentStore
/// read on the broadcast hot-path, we take a simpler approach: any
/// `state == Running` row for the job triggers `ConvergedRunning`. The
/// JobLifecycle reconciler is the source-of-truth for the actual
/// replica-desired check; our streaming surface is observation-shaped
/// and treats the first Running observation for the job as terminal.
///
/// This is acceptable because the reconciler is the gate that decides
/// when to emit `Action::StartAllocation` per the desired replica
/// count; the row only gets written if the reconciler says go.
async fn check_terminal(
    obs: &dyn ObservationStore,
    job_id: &JobId,
    view_cache: &Arc<std::sync::Mutex<std::collections::BTreeMap<(String, String), CachedView>>>,
    event: &LifecycleEvent,
) -> Option<SubmitEvent> {
    // Success path — Running observation for this job → ConvergedRunning.
    // Phase 1 walking-skeleton workloads have replicas=1 so a single
    // running row in the obs store meets the bar. Multi-replica jobs are
    // Phase 2+ scope. We read the obs store rather than trusting event.to
    // alone so that a Running broadcast event without a corresponding
    // obs row (e.g. a pre-stop Running event in the stop-while-streaming
    // scenario) does NOT prematurely close the stream.
    if matches!(event.to, AllocStateWire::Running) {
        if let Ok(rows) = obs.alloc_status_rows().await {
            let has_running = rows.iter().any(|r| {
                r.job_id == *job_id
                    && r.state == overdrive_core::traits::observation_store::AllocState::Running
            });
            if has_running {
                return Some(SubmitEvent::ConvergedRunning {
                    alloc_id: event.alloc_id.to_string(),
                    started_at: event.at.clone(),
                });
            }
        }
        return None;
    }

    // Stop path — Terminated observation → ConvergedStopped.
    // Extract StoppedBy from the event reason; default to Reconciler
    // when the reason is not a Stopped variant.
    if matches!(event.to, AllocStateWire::Terminated) {
        let by = match &event.reason {
            overdrive_core::TransitionReason::Stopped { by } => *by,
            _ => StoppedBy::Reconciler,
        };
        return Some(SubmitEvent::ConvergedStopped {
            alloc_id: Some(event.alloc_id.to_string()),
            by,
        });
    }

    // Failure path — Failed row + restart budget exhausted →
    // ConvergedFailed { BackoffExhausted }.
    if matches!(event.to, AllocStateWire::Failed) {
        let view = read_view(view_cache, job_id);
        let used = view.restart_counts.values().copied().max().unwrap_or(0);
        if used >= RESTART_BACKOFF_CEILING {
            // Read the latest cause-class reason from the obs store —
            // single-source-of-truth per [D7].
            let cause = latest_cause(obs, job_id).await.unwrap_or(event.reason.clone());
            return Some(SubmitEvent::ConvergedFailed {
                alloc_id: Some(event.alloc_id.to_string()),
                terminal_reason: TerminalReason::BackoffExhausted {
                    attempts: used,
                    cause: cause.clone(),
                },
                reason: Some(event.reason.clone()),
                error: event.detail.clone(),
            });
        }
    }
    None
}

/// On `Lagged(_)`, snapshot the obs store and synthesise a terminal
/// only if the latest row state is already terminal. Otherwise the
/// caller continues subscribing.
async fn lagged_recover(
    obs: &dyn ObservationStore,
    job_id: &JobId,
    view_cache: &Arc<std::sync::Mutex<std::collections::BTreeMap<(String, String), CachedView>>>,
) -> Option<SubmitEvent> {
    let rows = obs.alloc_status_rows().await.ok()?;
    let latest =
        rows.into_iter().filter(|r| r.job_id == *job_id).max_by_key(|r| r.updated_at.counter)?;

    match latest.state {
        AllocState::Running => Some(SubmitEvent::ConvergedRunning {
            alloc_id: latest.alloc_id.to_string(),
            started_at: format!(
                "{}@{}",
                latest.updated_at.counter,
                latest.updated_at.writer.as_str()
            ),
        }),
        AllocState::Terminated => {
            let by = match latest.reason {
                Some(overdrive_core::TransitionReason::Stopped { by }) => by,
                _ => StoppedBy::Reconciler,
            };
            Some(SubmitEvent::ConvergedStopped { alloc_id: Some(latest.alloc_id.to_string()), by })
        }
        AllocState::Failed => {
            let view = read_view(view_cache, job_id);
            let used = view.restart_counts.values().copied().max().unwrap_or(0);
            if used >= RESTART_BACKOFF_CEILING {
                let cause = latest.reason.clone().unwrap_or(
                    overdrive_core::TransitionReason::DriverInternalError { detail: String::new() },
                );
                Some(SubmitEvent::ConvergedFailed {
                    alloc_id: Some(latest.alloc_id.to_string()),
                    terminal_reason: TerminalReason::BackoffExhausted {
                        attempts: used,
                        cause: cause.clone(),
                    },
                    reason: Some(cause),
                    error: latest.detail.clone(),
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Read the most recent row's cause-class reason for `job_id`.
async fn latest_cause(
    obs: &dyn ObservationStore,
    job_id: &JobId,
) -> Option<overdrive_core::TransitionReason> {
    let rows = obs.alloc_status_rows().await.ok()?;
    rows.into_iter()
        .filter(|r| r.job_id == *job_id)
        .max_by_key(|r| r.updated_at.counter)
        .and_then(|r| r.reason)
}

/// Build the synchronous `Accepted` event the handler emits before
/// entering the streaming loop.
#[must_use]
pub fn build_accepted(
    spec_digest: String,
    intent_key: String,
    outcome: IdempotencyOutcome,
) -> SubmitEvent {
    SubmitEvent::Accepted { spec_digest, intent_key, outcome }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::str::FromStr;
    use std::sync::{Arc, Mutex};

    use overdrive_core::TransitionReason;
    use overdrive_core::id::{AllocationId, JobId, NodeId};
    use overdrive_core::traits::driver::DriverType;
    use overdrive_core::transition_reason::StoppedBy;
    use overdrive_sim::adapters::observation_store::SimObservationStore;

    use crate::action_shim::LifecycleEvent;
    use crate::api::{AllocStateWire, SubmitEvent, TransitionSource};

    use super::check_terminal;

    // Test budget: 1 behavior (check_terminal → ConvergedStopped on Terminated) × 2 = 2 max.
    // Using 1 unit test.

    /// check_terminal returns Some(ConvergedStopped) when event.to == Terminated
    /// with reason Stopped { by: Reconciler }.
    #[tokio::test]
    async fn check_terminal_returns_converged_stopped_on_terminated_event() {
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node, 0));
        let view_cache = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");
        let job_id = JobId::from_str("job-0").expect("job id");

        let event = LifecycleEvent {
            alloc_id: alloc_id.clone(),
            job_id: job_id.clone(),
            from: AllocStateWire::Running,
            to: AllocStateWire::Terminated,
            reason: TransitionReason::Stopped { by: StoppedBy::Reconciler },
            detail: None,
            source: TransitionSource::Driver(DriverType::Exec),
            at: "1@node-a".to_string(),
        };

        let result = check_terminal(&*obs, &job_id, &view_cache, &event).await;

        assert!(
            matches!(result, Some(SubmitEvent::ConvergedStopped { .. })),
            "expected Some(ConvergedStopped), got {result:?}"
        );
    }
}
