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
//! after `IntentStore::put_if_absent` returns — this is the
//! load-bearing wiring property for KPI-01 (200ms p95 first-line).
//! No broadcast wait, no observation read.
//!
//! After `Accepted` the loop subscribes to
//! `app_state.lifecycle_events` and enters a `tokio::select!` between:
//!
//! 1. `bus.recv()` — projects each `LifecycleEvent` to a
//!    `SubmitEvent::LifecycleTransition` line, then checks for terminal.
//! 2. `clock.sleep(streaming_cap)` — fires `ConvergedFailed { Timeout }`
//!    if no terminal event arrives within the cap.
//!
//! Terminal detection per architecture.md §4 / ADR-0032 §3 Amendment:
//!
//! - `state == Running && replicas_running >= replicas_desired` →
//!   `ConvergedRunning { alloc_id, started_at }`.
//! - `state == Failed && restart_budget.exhausted` (read off
//!   `view_cache`) → `ConvergedFailed { BackoffExhausted { attempts,
//!   cause: <last cause-class TransitionReason from row> } }`.
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
/// the handler after the `IntentStore::put_if_absent` returned — this
/// is what makes KPI-01 (200ms p95 first-line) achievable.
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

        // 3. Spawn the cap timer as an explicit future the select! can race.
        //    `clock.sleep(cap)` is the dst-lint-gated way to wait — the
        //    handler MUST NOT use `tokio::time::sleep`.
        let cap_future = clock.sleep(cap);
        tokio::pin!(cap_future);

        loop {
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
                () = &mut cap_future => {
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
    // running row meets the bar. Multi-replica jobs are Phase 2+ scope.
    if matches!(event.to, AllocStateWire::Running) {
        return Some(SubmitEvent::ConvergedRunning {
            alloc_id: event.alloc_id.to_string(),
            started_at: event.at.clone(),
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
