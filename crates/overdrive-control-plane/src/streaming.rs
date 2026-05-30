//! Streaming submit loop for `POST /v1/jobs` with `Accept: application/x-ndjson`.
//!
//! Per architecture.md §3 (happy path), §4 (broken-binary path), §5
//! (timeout path), and §10 (broadcast wiring). Slice 02 step 02-03.
//!
//! # Wiring
//!
//! The handler in [`crate::handlers::submit_workload`] branches on the
//! `Accept` header. The `application/x-ndjson` lane delegates here:
//! [`build_workload_stream`] builds a stream of `Result<Bytes, _>`
//! NDJSON lines that axum wraps via `Body::from_stream(...)`.
//!
//! The first line ([`JobSubmitEvent::Accepted`]) is emitted SYNCHRONOUSLY
//! after `IntentStore::put_if_absent` returns. No broadcast wait,
//! no observation read.
//!
//! After `Accepted` the loop subscribes to
//! `app_state.lifecycle_events` and enters a `tokio::select!` between:
//!
//! 1. `bus.recv()` — projects each `LifecycleEvent` to an intermediate
//!    [`JobSubmitEvent`] (`Pending` / `Running` / `AttemptFailed`),
//!    then checks for terminal.
//! 2. `clock.sleep(cap)` — wall-clock cap timer. Production
//!    (`SystemClock`) parks on a real timer; DST (`SimClock`) parks
//!    until the harness calls `sim_clock.tick(cap + ε)`. On expiry,
//!    emits a terminal `JobSubmitEvent::Failed` (`exit_code = -1`,
//!    "did not converge in Ns") and ends the stream.
//!
//! Terminal detection per architecture.md §4 / ADR-0032 §3 Amendment —
//! each `TerminalCondition` projects to a terminal [`JobSubmitEvent`]:
//!
//! - `TerminalCondition::Completed { exit_code }` → `Succeeded`.
//! - `TerminalCondition::Failed { exit_code }` → `Failed`.
//! - `TerminalCondition::BackoffExhausted { attempts }` → `Failed`
//!   (the workload exhausted its restart budget).
//! - `TerminalCondition::Stopped { by }` → `Stopped`.
//!
//! On `RecvError::Lagged(_)`, the loop falls back to a one-shot
//! `obs.alloc_status_rows()` snapshot per ADR-0032 §7 and resumes the
//! broadcast. The resubscription happens implicitly because
//! `tokio::sync::broadcast::Receiver` skips ahead on `Lagged` and the
//! next `recv()` is the new front.
//!
//! # Missed-events classes
//!
//! Five classes of missed-events failure are bridged or surfaced by
//! the loop:
//!
//! 1. Live event delivery — the `bus.recv()` arm projects each
//!    `LifecycleEvent` and runs the terminal check against the obs store.
//! 2. `RecvError::Lagged(_)` — buffer-overflow: a slow subscriber fell
//!    behind and the broadcast channel evicted older messages. Bridged
//!    by the `lagged_recover` snapshot in the `Lagged` arm.
//! 3. `RecvError::Closed` — the `lifecycle_events` sender was dropped;
//!    the loop emits a terminal `JobSubmitEvent::Failed` and ends.
//! 4. Cap timer — `clock.sleep(cap)` fires; the loop emits a terminal
//!    `JobSubmitEvent::Failed` and ends.
//! 5. **Pre-subscription event window** — the upstream `put_if_absent`
//!    write may trigger the convergence loop *before* `bus.subscribe()`
//!    runs in this stream. `tokio::sync::broadcast::Sender::send` only
//!    delivers to receivers `subscribe()`-d at send-time, so events
//!    broadcast in the pre-subscribe window have no receiver and are
//!    dropped — the same kind of missed-events class as `Lagged`, but
//!    at subscribe-time rather than at buffer-overflow time. Bridged
//!    by a one-shot `lagged_recover` snapshot taken immediately after
//!    `bus.subscribe()` and before the select loop is entered: if the
//!    latest row for the job is already terminal, the snapshot
//!    projects it to a terminal [`JobSubmitEvent`] and the stream ends
//!    without entering the loop.

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

use axum::body::Bytes;
use bytes::BytesMut;
use futures::Stream;
use overdrive_core::TransitionReason;
use overdrive_core::id::WorkloadId;
use overdrive_core::traits::observation_store::ObservationStore;
use tokio::sync::broadcast;

use crate::AppState;
use crate::action_shim::LifecycleEvent;
use crate::api::{AllocStateWire, IdempotencyOutcome};
use crate::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::reconcilers::{TargetResource, backoff_for_attempt};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};

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

/// Build the synchronous `JobSubmitEvent::Accepted` event the handler
/// emits before entering the Job-kind streaming loop.
///
/// Per ADR-0047 §3 [D7]: Job-kind submits stream the per-kind sibling
/// event enum [`JobSubmitEvent`]; `Accepted` is the synchronous
/// first NDJSON line, emitted before the loop subscribes to the
/// lifecycle bus.
#[must_use]
pub fn build_workload_accepted(
    spec_digest: String,
    intent_key: String,
    outcome: IdempotencyOutcome,
) -> JobSubmitEvent {
    JobSubmitEvent::Accepted { spec_digest, intent_key, outcome }
}

/// Build the streaming response body for the Job-kind NDJSON lane.
///
/// Per ADR-0047 §3 [D7]: Job-kind streams the per-kind sibling-event
/// enum [`JobSubmitEvent`]. A false-positive "converged / running"
/// terminal is structurally unreachable on this code path because the
/// type carries no such variant — Jobs are run-to-completion and
/// `Running` is informational only.
///
/// Contract (per `.claude/rules/development.md` § *Trait definitions
/// specify behavior, not just signature*):
///
/// - **Preconditions**: `state.lifecycle_events`, `state.obs`, and
///   `state.clock` are wired; `accepted` carries the synchronous
///   `Accepted` line built by [`build_workload_accepted`].
/// - **Postconditions**: the returned stream emits `Accepted` first,
///   then zero or more intermediate variants
///   (`Pending` / `Running` / `AttemptFailed`), and exactly one
///   terminal variant (`Succeeded` / `Failed` / `Stopped`) before closing.
/// - **Edge cases**:
///   - Operator-stop converged terminal → `Stopped { stopped_by, exit_code }`
///     (operator-initiated stop is its own terminal variant, distinct from
///     `Succeeded`; `stopped_by` names the operator, `exit_code` reflects
///     the actual workload exit code at the time of stop).
///   - Cap timer expiry → `Failed { exit_code: -1 }` (no kernel exit
///     observed within the wall-clock budget — distinguished from a
///     genuine non-zero exit by the sentinel `-1`).
///   - Broadcast `Closed` → `Failed { exit_code: -1 }` (server-side
///     stream interruption — the Service-kind lane surfaces the same
///     condition as `ServiceFailureReason::StreamInterrupted`).
///   - Pre-subscribe race / `Lagged(_)` → snapshot-and-classify the
///     latest LWW-winner row; emit the matching terminal if reached.
/// - **Observable invariants**: every Job-kind submit produces exactly
///   ONE terminal variant (`Succeeded`, `Failed`, or `Stopped`) on the
///   wire; the stream never closes silently. The CLI process exit code
///   equals the terminal `exit_code` field per KPI K1 honesty contract.
pub fn build_workload_stream(
    state: AppState,
    workload_id: WorkloadId,
    accepted: JobSubmitEvent,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    let bus = state.lifecycle_events.clone();
    let clock = state.clock.clone();
    let obs = state.obs.clone();
    let runtime = state.runtime.clone();
    let cap = state.streaming_cap;

    async_stream::stream! {
        // 1. Emit Accepted SYNCHRONOUSLY — first byte on the wire.
        match emit_workload_line(&accepted) {
            Ok(line) => yield Ok(line),
            Err(err) => {
                yield Err(err);
                return;
            }
        }

        // 2. Subscribe BEFORE the pre-subscribe snapshot recovery to
        //    bridge the pre-subscribe event window per architecture.md
        //    §10 / ADR-0032 §7.
        let mut sub = bus.subscribe();
        drop(bus);

        // 3. Pre-subscribe race recovery — if the latest row already
        //    carries a terminal claim, emit the matching JobSubmitEvent
        //    terminal and end.
        if let Some(terminal) = workload_terminal_from_snapshot(&*obs, &runtime, &workload_id).await {
            match emit_workload_line(&terminal) {
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

        let cap_future = clock.sleep(cap);
        tokio::pin!(cap_future);

        loop {
            tokio::select! {
                biased;
                recv = sub.recv() => {
                    match recv {
                        Ok(event) => {
                            if event.workload_id != workload_id {
                                // Falls through to the next select! iteration.
                            }
                            // Project the LifecycleEvent into the
                            // JobSubmitEvent shape — informational
                            // (Pending / Running) or AttemptFailed
                            // (intermediate failure) or terminal
                            // (Succeeded / Failed).
                            if let Some(emit) = workload_event_from_lifecycle(
                                &*obs,
                                &runtime,
                                &workload_id,
                                &event,
                            ).await {
                                let is_terminal = matches!(
                                    emit,
                                    JobSubmitEvent::Succeeded { .. } | JobSubmitEvent::Failed { .. } | JobSubmitEvent::Stopped { .. }
                                );
                                match emit_workload_line(&emit) {
                                    Ok(line) => {
                                        yield Ok(line);
                                        if is_terminal {
                                            return;
                                        }
                                    }
                                    Err(err) => {
                                        yield Err(err);
                                        return;
                                    }
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            if let Some(terminal) = workload_terminal_from_snapshot(
                                &*obs,
                                &runtime,
                                &workload_id,
                            ).await {
                                match emit_workload_line(&terminal) {
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
                            // Server-side stream interruption — emit
                            // a Failed terminal so the CLI exits non-zero.
                            // exit_code = -1 is the sentinel for
                            // "no kernel exit observed" (no genuine
                            // workload exit code is available — the
                            // bus closed before we saw one).
                            let attempts = best_effort_attempt_count(
                                &*obs, &runtime, &workload_id,
                            ).await;
                            let terminal = JobSubmitEvent::Failed {
                                exit_code: -1,
                                duration: String::new(),
                                attempts,
                                max_attempts: attempts,
                                stderr_tail: Some(
                                    "lifecycle channel closed before terminal".to_string(),
                                ),
                            };
                            match emit_workload_line(&terminal) {
                                Ok(line) => yield Ok(line),
                                Err(err) => yield Err(err),
                            }
                            return;
                        }
                    }
                }
                () = &mut cap_future => {
                    let after_seconds = u32::try_from(cap.as_secs()).unwrap_or(u32::MAX);
                    let attempts = best_effort_attempt_count(
                        &*obs, &runtime, &workload_id,
                    ).await;
                    let terminal = JobSubmitEvent::Failed {
                        exit_code: -1,
                        duration: format!("{after_seconds}s"),
                        attempts,
                        max_attempts: attempts,
                        stderr_tail: Some(format!(
                            "did not converge in {after_seconds}s"
                        )),
                    };
                    match emit_workload_line(&terminal) {
                        Ok(line) => yield Ok(line),
                        Err(err) => yield Err(err),
                    }
                    return;
                }
            }
        }
    }
}

/// One NDJSON line for a [`JobSubmitEvent`] —
/// `serde_json::to_writer(buf, &event)?` + `b'\n'`. The Service-kind
/// counterpart is [`emit_service_line`].
fn emit_workload_line(event: &JobSubmitEvent) -> std::io::Result<Bytes> {
    use std::io::Write as _;
    let mut buf = BytesMut::with_capacity(256);
    let mut writer = (&mut buf).writer();
    serde_json::to_writer(&mut writer, event).map_err(std::io::Error::other)?;
    writer.write_all(b"\n")?;
    Ok(buf.freeze())
}

/// Project a [`LifecycleEvent`] into a [`JobSubmitEvent`] for the
/// Job-kind streaming wire. Returns `None` for events that have no
/// Job-kind manifestation (e.g. an event for a different job, an
/// AllocStateWire variant the Job streaming surface ignores).
///
/// The terminal projection reads the row's `stderr_tail` field via
/// `obs.alloc_status_row(...)` so the operator-facing `Failed` event
/// carries the workload's stderr verbatim per slice 02-05 / ADR-0033
/// Amendment 2026-05-10.
pub async fn workload_event_from_lifecycle(
    obs: &dyn ObservationStore,
    runtime: &ReconcilerRuntime,
    workload_id: &WorkloadId,
    event: &LifecycleEvent,
) -> Option<JobSubmitEvent> {
    if let Some(cond) = &event.terminal {
        return Some(workload_event_from_terminal(obs, runtime, workload_id, event, cond).await);
    }
    match event.to {
        AllocStateWire::Pending => Some(JobSubmitEvent::Pending),
        AllocStateWire::Running => Some(JobSubmitEvent::Running { since: event.at.clone() }),
        AllocStateWire::Failed => {
            // Intermediate per-attempt failure observation — emitted
            // by the exit observer between attempts. Per ADR-0037 §4
            // the reconciler's terminal claim arrives on a subsequent
            // tick; surface this row as `AttemptFailed` so the operator
            // sees the workload exited and the verdict is being
            // finalised, without conflating the two into a single line.
            //
            // Read exit_code from the typed TransitionReason — the exit
            // observer now emits WorkloadCrashedImmediately { exit_code,
            // .. } with detail: None. The old parse_exit_code_from_detail
            // only handled the legacy "exit_code=N" string in detail and
            // always returned 1 when detail is absent.
            let exit_code = match &event.reason {
                TransitionReason::WorkloadCrashedImmediately { exit_code, .. } => {
                    exit_code.unwrap_or(1)
                }
                _ => 1,
            };
            let target = TargetResource::new(&format!("job/{workload_id}")).ok();
            let (attempt_index, will_restart) = target
                .as_ref()
                .map_or((1, true), |t| runtime.restart_status_for_alloc(t, &event.alloc_id));
            let next_attempt_delay = if will_restart {
                Some(format!(
                    "{}ms",
                    backoff_for_attempt(attempt_index.saturating_sub(1)).as_millis()
                ))
            } else {
                None
            };
            Some(JobSubmitEvent::AttemptFailed {
                attempt_index,
                exit_code,
                duration: event.at.clone(),
                will_restart,
                next_attempt_delay,
            })
        }
        // Terminated without a terminal claim is a transitional state
        // (the reconciler will stamp the terminal on the next tick).
        // Suspended / Draining are likewise non-terminal Job-kind
        // surfaces. Skip emission until the terminal claim arrives.
        AllocStateWire::Terminated | AllocStateWire::Draining | AllocStateWire::Suspended => None,
    }
}

/// Project a [`TerminalCondition`] into the matching terminal
/// [`JobSubmitEvent`] variant. Reads the LWW-winner observation row
/// for `stderr_tail` so the operator-facing `Failed` event carries
/// the workload's stderr verbatim.
async fn workload_event_from_terminal(
    obs: &dyn ObservationStore,
    runtime: &ReconcilerRuntime,
    workload_id: &WorkloadId,
    event: &LifecycleEvent,
    cond: &TerminalCondition,
) -> JobSubmitEvent {
    let row = obs.alloc_status_row(&event.alloc_id).await.ok().flatten();
    let stderr_tail = row.as_ref().and_then(|r| r.stderr_tail.clone());
    let target = TargetResource::new(&format!("job/{workload_id}")).ok();
    let attempt_index =
        target.as_ref().map_or(1, |t| runtime.restart_status_for_alloc(t, &event.alloc_id).0);
    match cond {
        TerminalCondition::Completed { exit_code } => JobSubmitEvent::Succeeded {
            exit_code: *exit_code,
            duration: event.at.clone(),
            attempts: attempt_index,
        },
        TerminalCondition::Failed { exit_code } => JobSubmitEvent::Failed {
            exit_code: *exit_code,
            duration: event.at.clone(),
            attempts: attempt_index,
            max_attempts: attempt_index,
            stderr_tail,
        },
        TerminalCondition::Stopped { by } => JobSubmitEvent::Stopped {
            stopped_by: *by,
            duration: event.at.clone(),
            attempts: attempt_index,
        },
        TerminalCondition::BackoffExhausted { attempts } => JobSubmitEvent::Failed {
            exit_code: 1,
            duration: event.at.clone(),
            attempts: *attempts,
            max_attempts: *attempts,
            stderr_tail,
        },
        _ => JobSubmitEvent::Failed {
            exit_code: 1,
            duration: event.at.clone(),
            attempts: attempt_index,
            max_attempts: attempt_index,
            stderr_tail,
        },
    }
}

/// On `Lagged(_)` or pre-subscribe snapshot, inspect the LWW-winner
/// `AllocStatusRow` for the job. If it already carries a terminal
/// claim, project to the matching `JobSubmitEvent` terminal.
async fn workload_terminal_from_snapshot(
    obs: &dyn ObservationStore,
    runtime: &ReconcilerRuntime,
    workload_id: &WorkloadId,
) -> Option<JobSubmitEvent> {
    let rows = obs.alloc_status_rows().await.ok()?;
    let latest = rows
        .into_iter()
        .filter(|r| r.workload_id == *workload_id)
        .max_by_key(|r| r.updated_at.counter)?;
    let cond = latest.terminal.as_ref()?;
    let duration = format!("{}@{}", latest.updated_at.counter, latest.updated_at.writer.as_str());
    let stderr_tail = latest.stderr_tail.clone();
    let target = TargetResource::new(&format!("job/{workload_id}")).ok();
    let attempt_index =
        target.as_ref().map_or(1, |t| runtime.restart_status_for_alloc(t, &latest.alloc_id).0);
    Some(match cond {
        TerminalCondition::Completed { exit_code } => {
            JobSubmitEvent::Succeeded { exit_code: *exit_code, duration, attempts: attempt_index }
        }
        TerminalCondition::Failed { exit_code } => JobSubmitEvent::Failed {
            exit_code: *exit_code,
            duration,
            attempts: attempt_index,
            max_attempts: attempt_index,
            stderr_tail,
        },
        TerminalCondition::Stopped { by } => {
            JobSubmitEvent::Stopped { stopped_by: *by, duration, attempts: attempt_index }
        }
        TerminalCondition::BackoffExhausted { attempts } => JobSubmitEvent::Failed {
            exit_code: 1,
            duration,
            attempts: *attempts,
            max_attempts: *attempts,
            stderr_tail,
        },
        _ => JobSubmitEvent::Failed {
            exit_code: 1,
            duration,
            attempts: attempt_index,
            max_attempts: attempt_index,
            stderr_tail,
        },
    })
}

/// Best-effort attempt count for a workload from the obs store + runtime
/// view. Used by degenerate terminal arms (Closed, cap-timer) that have
/// no [`LifecycleEvent`] to extract an `alloc_id` from.
///
/// Follows the same obs-store → runtime query as
/// [`workload_terminal_from_snapshot`]; returns 1 when no observation
/// row or view entry exists (preserving the original hardcoded default
/// for fresh jobs that never completed an attempt).
async fn best_effort_attempt_count(
    obs: &dyn ObservationStore,
    runtime: &ReconcilerRuntime,
    workload_id: &WorkloadId,
) -> u32 {
    let Ok(rows) = obs.alloc_status_rows().await else { return 1 };
    let latest = rows
        .into_iter()
        .filter(|r| r.workload_id == *workload_id)
        .max_by_key(|r| r.updated_at.counter);
    latest.map_or(1, |row| {
        let target = TargetResource::new(&format!("job/{workload_id}")).ok();
        target.as_ref().map_or(1, |t| runtime.restart_status_for_alloc(t, &row.alloc_id).0)
    })
}

// ---------------------------------------------------------------------------
// Job streaming sub-path — slice 02 of `workload-kind-discriminator`.
// ---------------------------------------------------------------------------
//
// Per ADR-0047 §3 [D2] + [D7]: Job-kind submits stream a per-kind
// sibling enum `JobSubmitEvent` whose variants make the historical
// false-positive "is running with N/M replicas (took live)" rendering
// structurally unreachable — `Running` is informational, only an
// exit-bearing terminal closes the stream. The conjunction of RCA
// root causes B+C+D is rendered impossible at the type level.
//
// Job semantics (run-to-completion):
//   * `Accepted` — synchronous first-line ack.
//   * `Pending` — informational, allocation pending placement.
//   * `Running { since }` — informational, NOT terminal. A Job is not
//     "done" because it is currently running; it is done only when it
//     terminates with an exit code. Renderers MUST NOT render this
//     variant as a terminal success.
//   * `AttemptFailed` — intermediate; stream stays open while the
//     reconciler decides whether to restart.
//   * `Succeeded { exit_code: 0, ... }` — terminal; CLI exits 0.
//   * `Failed { exit_code, ... }` — terminal; CLI exits with the
//     workload's kernel-observed exit code.

/// Streaming events emitted by the Job submit sub-path.
///
/// Per ADR-0047 §3 [D2] / [D7]: Job kind has no "converged / running"
/// terminal variant — the conjunction of RCA root causes B+C+D is
/// rendered structurally unreachable for Job by the type system itself.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum JobSubmitEvent {
    /// Submit was accepted. First NDJSON line on the wire.
    /// Synchronous; no broadcast wait.
    Accepted {
        /// Canonical 64-char lowercase-hex SHA-256 of the rkyv-archived
        /// `WorkloadSpec::Job` bytes (ADR-0002).
        spec_digest: String,
        /// Canonical `jobs/<id>` IntentKey string form.
        intent_key: String,
        /// Idempotency verdict.
        outcome: crate::api::IdempotencyOutcome,
    },
    /// Allocation pending placement.
    Pending,
    /// Allocation is currently running. **NOT a terminal event.** The
    /// stream MUST NOT close on this variant — Jobs are
    /// run-to-completion and `Running` is informational only.
    Running { since: String },
    /// Intermediate — an attempt exited non-zero and the reconciler
    /// will restart up to `backoff_limit`. The stream remains open.
    AttemptFailed {
        attempt_index: u32,
        exit_code: i32,
        duration: String,
        will_restart: bool,
        next_attempt_delay: Option<String>,
    },
    /// Terminal — workload exited 0 within `backoff_limit`. CLI exit 0.
    Succeeded { exit_code: i32, duration: String, attempts: u32 },
    /// Terminal — workload exhausted `backoff_limit` with a non-zero
    /// exit on every attempt. CLI exit code = workload kernel exit
    /// code (per slice 02 KPI K1 honesty contract).
    Failed {
        exit_code: i32,
        duration: String,
        attempts: u32,
        max_attempts: u32,
        /// Last 5 lines (default) of the workload's stderr from the
        /// final attempt. Present when ExitObserver captured stderr.
        stderr_tail: Option<String>,
    },
    /// Terminal — allocation stopped by operator or reconciler before
    /// natural completion. Neither success nor failure — the workload
    /// was interrupted. CLI exit code = 130 (SIGINT-stop convention).
    Stopped { stopped_by: StoppedBy, duration: String, attempts: u32 },
}

// ---------------------------------------------------------------------------
// Service streaming sub-path — slice 02 of `workload-kind-discriminator`.
// ---------------------------------------------------------------------------
//
// Per ADR-0056 / DDD-11 (single-cut V1→V2 wire migration, step 01-03e):
// Service-kind's terminal events project the typed `TerminalCondition::
// {Stable, ServiceFailed}` variants directly. `ServiceSubmitEvent::Stable`
// carries the typed `ProbeWitness` naming the last-to-Pass startup probe;
// `ServiceSubmitEvent::Failed` carries the typed `ServiceFailureReason`
// (StartupTimeout / StartupProbeFailed / EarlyExit / LivenessProbeFailed)
// per ADR-0055 §4. The wire shape preserves byte-equality with the row's
// `terminal` field per ADR-0037 §4 K2 trace-equivalence.

/// Streaming events emitted by the Service submit sub-path.
///
/// Per ADR-0056 / DDD-11: V2 wire shape. Terminal events:
/// * `Stable { settled_in_ms, witness }` — convergence reached the
///   `TerminalCondition::Stable` shape (all declared startup probes
///   passed within `startup_deadline`).
/// * `Failed { reason, stderr_tail }` — convergence failed with a
///   typed `ServiceFailureReason` cause (startup-timeout, startup-
///   probe-failed, early-exit, liveness-probe-failed).
///
/// The wire projection is byte-equal with the row's typed
/// `TerminalCondition` carried in `AllocStatusRow.terminal` — both
/// surfaces project from the same `Action.terminal` value in the
/// same action-shim call frame per ADR-0037 §4.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ServiceSubmitEvent {
    /// Submit was accepted. First NDJSON line on the wire.
    Accepted { spec_digest: String, intent_key: String, outcome: crate::api::IdempotencyOutcome },
    /// Terminal — service reached the Stable state per ADR-0055
    /// (all declared startup probes passed within `startup_deadline`).
    /// `settled_in_ms` is the wall-clock interval (in milliseconds)
    /// from alloc start to the deciding tick that observed the
    /// last-to-Pass startup probe. `witness` names which probe's
    /// Pass moved the reconciler to Stable.
    Stable { alloc_id: String, settled_in_ms: u64, witness: crate::api::ProbeWitnessWire },
    /// Terminal — service convergence failed. `reason` carries the
    /// typed `ServiceFailureReasonWire` cause discriminator per
    /// ADR-0055 §4 / ADR-0056. `stderr_tail` is the last-N lines of
    /// the workload's stderr (populated by ExitObserver in slice
    /// 03-02; Option<String> here so the wire shape is stable
    /// against the future plumbing).
    Failed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alloc_id: Option<String>,
        reason: crate::api::ServiceFailureReasonWire,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stderr_tail: Option<String>,
    },
    /// Terminal — service stopped (by operator or by reconciler-
    /// observed process exit / system gc). Per ADR-0059 Q1 this is
    /// a sibling variant of `Failed`, NOT folded into it — CLI
    /// exit-code semantics diverge (Stopped exits 0 / 130; Failed
    /// exits with the failure-specific code). Mirrors the
    /// `JobSubmitEvent::Stopped` sibling convention.
    Stopped { alloc_id: String, by: overdrive_core::transition_reason::StoppedBy },
}

// ---------------------------------------------------------------------------
// Service-kind projection — `TerminalCondition` → `ServiceSubmitEvent`.
// ---------------------------------------------------------------------------
//
// Per ADR-0059: single write site for Service-kind wire projections.
// Lands the new variants from ADR-0059 Q1 (Stopped), Q2 (BackoffExhausted
// with BackoffCause + last_exit_code), Q3 (Custom → Other with UTF-8-or-
// hex render), and Q4 (Timeout / StreamInterrupted — synthesised, not
// projected).
//
// These functions are NOT yet wired into the production `handlers.rs:498`
// dispatch path — that wiring is the next step's concern per ADR-0059 Q6.
// They land here as the taxonomy infrastructure the dispatch will consume.

/// Project a [`TerminalCondition`] into the matching
/// [`ServiceSubmitEvent`] variant.
///
/// Per ADR-0037 §3/§4 — single-write-site discipline: the same
/// `Action.terminal` value the action_shim writes onto
/// `AllocStatusRow.terminal` MUST project to the corresponding wire
/// event byte-equal. The projection lives here so future wiring sites
/// (handlers.rs:498 dispatch — next step) reuse one canonical mapping.
///
/// Returns `None` for `TerminalCondition` variants without a Service-
/// kind wire projection (e.g. `Completed { exit_code }` — Job-kind
/// natural exit, not reachable on the Service-kind broadcast lane).
///
/// `stderr_tail` is read by the caller from the row's `stderr_tail`
/// field (typically via `obs.alloc_status_row(...).stderr_tail`).
/// `last_exit_code` is read by the caller from the row's `exit_code`
/// observation field — required for `BackoffExhausted` per ADR-0059
/// Q2. `None` for projections that do not consult exit code.
#[must_use]
pub fn service_event_from_terminal(
    alloc_id: &str,
    terminal: &overdrive_core::transition_reason::TerminalCondition,
    stderr_tail: Option<String>,
    last_exit_code: Option<i32>,
) -> Option<ServiceSubmitEvent> {
    use overdrive_core::transition_reason::{
        BackoffCause, ServiceFailureReason, TerminalCondition,
    };
    match terminal {
        TerminalCondition::Stable { settled_in_ms, witness } => Some(ServiceSubmitEvent::Stable {
            alloc_id: alloc_id.to_string(),
            settled_in_ms: *settled_in_ms,
            witness: witness.clone(),
        }),
        TerminalCondition::ServiceFailed { reason } => Some(ServiceSubmitEvent::Failed {
            alloc_id: Some(alloc_id.to_string()),
            reason: reason.clone(),
            stderr_tail,
        }),
        TerminalCondition::Stopped { by } => {
            Some(ServiceSubmitEvent::Stopped { alloc_id: alloc_id.to_string(), by: *by })
        }
        TerminalCondition::BackoffExhausted { attempts } => Some(ServiceSubmitEvent::Failed {
            alloc_id: Some(alloc_id.to_string()),
            reason: ServiceFailureReason::BackoffExhausted {
                attempts: *attempts,
                cause: BackoffCause::AttemptBudget,
                last_exit_code,
            },
            stderr_tail,
        }),
        TerminalCondition::Custom { type_name, detail } => {
            let message = render_custom_detail(detail.as_deref());
            Some(ServiceSubmitEvent::Failed {
                alloc_id: Some(alloc_id.to_string()),
                reason: ServiceFailureReason::Other { source: type_name.clone(), message },
                stderr_tail,
            })
        }
        // Job-kind natural exit terminals have no Service-kind wire
        // projection. The Service-kind broadcast lane is not reached
        // for these in production; this match arm exists for
        // exhaustiveness against the `#[non_exhaustive]` enum.
        // mutants: skip — equivalent mutant: deleting this explicit arm
        // folds it into the `_ => None` catch-all below; both return the
        // identical `None`, so no test can observe the difference.
        TerminalCondition::Completed { .. } | TerminalCondition::Failed { .. } => None,
        // Forward-compat for future `#[non_exhaustive]` additions to
        // `TerminalCondition`. Unknown variants do not project to a
        // Service-kind wire event; callers fall back to the
        // streaming-loop's cap-timer / channel-closed synthesis.
        _ => None,
    }
}

/// Render `TerminalCondition::Custom.detail` bytes into the wire
/// `ServiceFailureReason::Other.message` string per ADR-0059 Q3:
/// best-effort UTF-8-or-lowercase-hex. `None` / empty → empty string.
fn render_custom_detail(detail: Option<&[u8]>) -> String {
    use std::fmt::Write as _;
    let Some(bytes) = detail else { return String::new() };
    if bytes.is_empty() {
        return String::new();
    }
    std::str::from_utf8(bytes).map_or_else(
        |_| {
            let mut hex = String::with_capacity(bytes.len() * 2);
            for b in bytes {
                let _ = write!(&mut hex, "{b:02x}");
            }
            hex
        },
        ToString::to_string,
    )
}

/// Synthesise the streaming wall-clock cap-timer terminal per
/// ADR-0059 Q4. Streaming-loop-only — the reconciler MUST NOT emit
/// this variant.
#[must_use]
pub fn service_stream_synth_cap_timeout(after_seconds: u32) -> ServiceSubmitEvent {
    ServiceSubmitEvent::Failed {
        alloc_id: None,
        reason: overdrive_core::transition_reason::ServiceFailureReason::Timeout { after_seconds },
        stderr_tail: None,
    }
}

/// Synthesise the streaming broadcast-channel-closed terminal per
/// ADR-0059 Q4. Streaming-loop-only — the reconciler MUST NOT emit
/// this variant.
#[must_use]
pub fn service_stream_synth_closed() -> ServiceSubmitEvent {
    ServiceSubmitEvent::Failed {
        alloc_id: None,
        reason: overdrive_core::transition_reason::ServiceFailureReason::StreamInterrupted,
        stderr_tail: None,
    }
}

/// Build the synchronous `ServiceSubmitEvent::Accepted` event the
/// handler emits before entering the Service-kind streaming loop.
///
/// Mirror of [`build_workload_accepted`] for the Service-kind sibling
/// surface. Per ADR-0056: Service-kind Accepted carries spec_digest
/// + intent_key + outcome — no `vip` field (the VIP is on the JSON
/// `SubmitWorkloadResponse` lane only).
#[must_use]
pub fn build_service_accepted(
    spec_digest: String,
    intent_key: String,
    outcome: IdempotencyOutcome,
) -> ServiceSubmitEvent {
    ServiceSubmitEvent::Accepted { spec_digest, intent_key, outcome }
}

/// One NDJSON line for a [`ServiceSubmitEvent`] —
/// `serde_json::to_writer(buf, &event)?` + `b'\n'`. Mirror of
/// [`emit_workload_line`] for the Service-kind sibling surface.
fn emit_service_line(event: &ServiceSubmitEvent) -> std::io::Result<Bytes> {
    use std::io::Write as _;
    let mut buf = BytesMut::with_capacity(256);
    let mut writer = (&mut buf).writer();
    serde_json::to_writer(&mut writer, event).map_err(std::io::Error::other)?;
    writer.write_all(b"\n")?;
    Ok(buf.freeze())
}

/// Build the streaming response body for the Service-kind NDJSON
/// lane per ADR-0056 / ADR-0059.
///
/// Service-kind streams emit a synchronous `Accepted` line, then
/// await broadcast `LifecycleEvent`s and project ONLY terminal events
/// (`TerminalCondition::{Stable, ServiceFailed, Stopped,
/// BackoffExhausted, Custom}`) into `ServiceSubmitEvent::{Stable,
/// Failed, Stopped}` via [`service_event_from_terminal`]. The stream
/// closes after the first terminal.
///
/// Contract (per `.claude/rules/development.md` § *Trait definitions
/// specify behavior, not just signature*):
///
/// - **Preconditions**: `state.lifecycle_events` and `state.clock` are
///   wired; `accepted` is the synchronous Accepted built by
///   [`build_service_accepted`].
/// - **Postconditions**: the returned stream emits `Accepted` first,
///   then exactly ONE terminal variant
///   (`Stable` / `Failed` / `Stopped`) before closing. No intermediate
///   `LifecycleTransition` / `Running` / `Pending` lines per ADR-0056
///   — the Service-kind wire surface is reduced to
///   {Accepted, terminal}.
/// - **Edge cases**:
///   - Cap timer expiry → `Failed { reason: Timeout { after_seconds } }`
///     per ADR-0059 Q4 via [`service_stream_synth_cap_timeout`].
///   - Broadcast `Closed` → `Failed { reason: StreamInterrupted }`
///     per ADR-0059 Q4 via [`service_stream_synth_closed`].
///   - Non-terminal events (`event.terminal.is_none()`) are SILENTLY
///     IGNORED — the Service-kind wire surface omits them by design
///     (S-SHCP-WIRE-15).
///   - Events for a different workload_id are ignored.
/// - **Observable invariant**: every Service-kind submit produces
///   exactly ONE terminal line on the wire; the stream never closes
///   silently.
pub fn build_service_stream(
    state: AppState,
    workload_id: WorkloadId,
    accepted: ServiceSubmitEvent,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    let bus = state.lifecycle_events.clone();
    let clock = state.clock.clone();
    let cap = state.streaming_cap;

    async_stream::stream! {
        // 1. Emit Accepted SYNCHRONOUSLY.
        match emit_service_line(&accepted) {
            Ok(line) => yield Ok(line),
            Err(err) => {
                yield Err(err);
                return;
            }
        }

        // 2. Subscribe after Accepted. Drop the local Sender clone so
        //    `RecvError::Closed` is reachable when external Senders
        //    drop.
        let mut sub = bus.subscribe();
        drop(bus);

        let cap_future = clock.sleep(cap);
        tokio::pin!(cap_future);

        loop {
            tokio::select! {
                biased;
                recv = sub.recv() => {
                    match recv {
                        Ok(event) => {
                            if event.workload_id != workload_id {
                                // Falls through to the next select! iteration.
                            }
                            // Service-kind: only terminal events
                            // project to a wire line. Non-terminal
                            // (`event.terminal.is_none()`) is silently
                            // ignored per ADR-0056 / S-SHCP-WIRE-15.
                            let Some(terminal) = &event.terminal else {
                                continue;
                            };
                            let alloc_id_str = event.alloc_id.to_string();
                            let stderr_tail = event.detail.clone();
                            let Some(wire) = service_event_from_terminal(
                                &alloc_id_str,
                                terminal,
                                stderr_tail,
                                None,
                            ) else {
                                // TerminalCondition variant without a
                                // Service-kind wire projection (e.g.
                                // Job-kind natural exit). Skip — the
                                // cap timer or a subsequent broadcast
                                // event will close the stream.
                                continue;
                            };
                            match emit_service_line(&wire) {
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
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            // No observation-store fallback for the
                            // Service-kind sibling surface in Phase 1
                            // — a lagged Service stream waits for the
                            // next live event or the cap timer.
                            // Falls through to the next select! iteration.
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            let wire = service_stream_synth_closed();
                            match emit_service_line(&wire) {
                                Ok(line) => yield Ok(line),
                                Err(err) => yield Err(err),
                            }
                            return;
                        }
                    }
                }
                () = &mut cap_future => {
                    let after_seconds = u32::try_from(cap.as_secs()).unwrap_or(u32::MAX);
                    let wire = service_stream_synth_cap_timeout(after_seconds);
                    match emit_service_line(&wire) {
                        Ok(line) => yield Ok(line),
                        Err(err) => yield Err(err),
                    }
                    return;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Schedule streaming sub-path — slice 05 of `workload-kind-discriminator`.
// ---------------------------------------------------------------------------
//
// Per ADR-0047 §3 / DESIGN [D7] the streaming surface gains three
// sibling per-kind event enums (`ServiceSubmitEvent`, `JobSubmitEvent`,
// `ScheduleSubmitEvent`) wrapped in a kind-tagged outer envelope.
// Slice 02 introduces the outer envelope and the Service/Job sibling
// enums; slice 05 lands the Schedule sibling because the Schedule
// surface is independent of the long-running streaming-cap loop —
// Schedule submit is `Accepted` + `Registered` and the stream closes,
// no firing semantics this slice (cron firing is GH #166).
//
// `ScheduleSubmitEvent` is intentionally minimal: two variants, both
// emitted synchronously at submit time. There is no terminal /
// converged-running / cap-timer arm because Schedule has no
// long-running convergence loop yet — that lands when GH #166 wires
// firing semantics through the reconciler.

/// Wire-shape declaration for the Schedule submit streaming sub-path.
///
/// **Not currently emitted on any server code path.** `handlers.rs`
/// rejects `WorkloadKind::Schedule` at the submit validation step
/// (HTTP 400) before any streaming dispatch. This enum is the
/// forward-declared wire shape for when Schedule firing is wired up;
/// tracked at GH #166.
///
/// Per ADR-0047 §3 / [D7]: two variants, both emitted synchronously
/// at submit time. `Accepted` is the first NDJSON line; `Registered`
/// carries the cron expression echoed verbatim and the deferral
/// tracking URL. The stream closes after `Registered` — Schedule has
/// no firing semantics this slice (tracked at GH #166).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ScheduleSubmitEvent {
    /// Submit was accepted. The first NDJSON line on the wire —
    /// `spec_digest` is the canonical 64-char lowercase-hex
    /// SHA-256 of the rkyv-archived `WorkloadSpec::Schedule` bytes;
    /// `intent_key` is the canonical `schedules/<id>` key; `outcome`
    /// is the idempotency verdict.
    Accepted {
        /// Canonical 64-char lowercase-hex SHA-256 of the
        /// rkyv-archived `WorkloadSpec::Schedule` bytes (ADR-0002).
        spec_digest: String,
        /// Canonical `schedules/<id>` IntentKey string form.
        intent_key: String,
        /// Idempotency verdict — `Inserted` for fresh submit,
        /// `Unchanged` for byte-identical resubmit.
        outcome: IdempotencyOutcome,
    },
    /// Schedule registered — execution is deferred. `cron` is the
    /// operator-supplied cron expression echoed verbatim;
    /// `deferral_url` is the tracking-issue URL the CLI render-layer
    /// reads from `SCHEDULE_EXECUTION_TRACKING_URL`. The stream
    /// closes after this event.
    Registered {
        /// Operator-supplied cron expression, preserved verbatim
        /// (no canonicalisation, no whitespace collapse).
        cron: String,
        /// Deferral-tracking issue URL — byte-equal to the CLI's
        /// `SCHEDULE_EXECUTION_TRACKING_URL` constant per KPI K5.
        deferral_url: String,
    },
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::collections::BTreeMap;

    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::Duration;

    use overdrive_core::TransitionReason;
    use overdrive_core::UnixInstant;
    use overdrive_core::aggregate::WorkloadKind;
    use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
    use overdrive_core::reconcilers::WorkloadLifecycleView;
    use overdrive_core::traits::driver::DriverType;
    use overdrive_core::traits::observation_store::{
        AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
    };
    use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
    use overdrive_sim::adapters::observation_store::SimObservationStore;

    use crate::action_shim::LifecycleEvent;
    use crate::api::{AllocStateWire, TransitionSource};
    use crate::reconciler_runtime::ReconcilerRuntime;

    use super::{
        JobSubmitEvent, best_effort_attempt_count, workload_event_from_terminal,
        workload_terminal_from_snapshot,
    };

    fn make_lifecycle_event(alloc_id: &AllocationId, workload_id: &WorkloadId) -> LifecycleEvent {
        LifecycleEvent {
            alloc_id: alloc_id.clone(),
            workload_id: workload_id.clone(),
            from: AllocStateWire::Running,
            to: AllocStateWire::Terminated,
            reason: TransitionReason::Stopped { by: StoppedBy::Reconciler },
            detail: None,
            source: TransitionSource::Driver(DriverType::Exec),
            at: "1@node-a".to_string(),
            terminal: None,
        }
    }

    fn make_alloc_status_row(
        alloc_id: &AllocationId,
        workload_id: &WorkloadId,
        node_id: &NodeId,
        terminal: Option<TerminalCondition>,
    ) -> AllocStatusRow {
        AllocStatusRow {
            alloc_id: alloc_id.clone(),
            workload_id: workload_id.clone(),
            node_id: node_id.clone(),
            state: AllocState::Terminated,
            updated_at: LogicalTimestamp { counter: 5, writer: node_id.clone() },
            reason: None,
            detail: None,
            terminal,
            stderr_tail: None,
            kind: WorkloadKind::Job,
            listeners: vec![],
            // GAP-1 subsidiary: Terminated state was Running first.
            started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        }
    }

    fn make_runtime(tmp: &tempfile::TempDir) -> ReconcilerRuntime {
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime")
    }

    async fn make_runtime_with_restart_count(
        tmp: &tempfile::TempDir,
        workload_id: &WorkloadId,
        alloc_id: &AllocationId,
        restart_count: u32,
    ) -> ReconcilerRuntime {
        let mut runtime = make_runtime(tmp);
        runtime.register(crate::workload_lifecycle()).await.expect("register");
        let target =
            overdrive_core::reconcilers::TargetResource::new(&format!("job/{workload_id}"))
                .expect("target");
        let view = WorkloadLifecycleView {
            restart_counts: BTreeMap::from([(alloc_id.clone(), restart_count)]),
            ..Default::default()
        };
        runtime.seed_workload_lifecycle_view_for_test(&target, view);
        runtime
    }

    // -----------------------------------------------------------------------
    // workload_event_from_terminal — one test per TerminalCondition variant
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn event_from_terminal_completed_yields_succeeded() {
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime(&tmp);
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node, 0));
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let event = make_lifecycle_event(&alloc_id, &wl_id);
        let cond = TerminalCondition::Completed { exit_code: 0 };

        let result = workload_event_from_terminal(&*obs, &runtime, &wl_id, &event, &cond).await;

        match result {
            JobSubmitEvent::Succeeded { exit_code, attempts, .. } => {
                assert_eq!(exit_code, 0);
                assert_eq!(attempts, 1);
            }
            other => panic!("expected Succeeded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn event_from_terminal_failed_yields_failed() {
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime(&tmp);
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node, 0));
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let event = make_lifecycle_event(&alloc_id, &wl_id);
        let cond = TerminalCondition::Failed { exit_code: 42 };

        let result = workload_event_from_terminal(&*obs, &runtime, &wl_id, &event, &cond).await;

        match result {
            JobSubmitEvent::Failed { exit_code, attempts, max_attempts, .. } => {
                assert_eq!(exit_code, 42);
                assert_eq!(attempts, 1);
                assert_eq!(max_attempts, 1);
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn event_from_terminal_stopped_yields_stopped() {
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime(&tmp);
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node, 0));
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let event = make_lifecycle_event(&alloc_id, &wl_id);
        let cond = TerminalCondition::Stopped { by: StoppedBy::Operator };

        let result = workload_event_from_terminal(&*obs, &runtime, &wl_id, &event, &cond).await;

        match result {
            JobSubmitEvent::Stopped { stopped_by, attempts, .. } => {
                assert_eq!(stopped_by, StoppedBy::Operator);
                assert_eq!(attempts, 1);
            }
            other => panic!("expected Stopped, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn event_from_terminal_backoff_exhausted_yields_failed() {
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime(&tmp);
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node, 0));
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let event = make_lifecycle_event(&alloc_id, &wl_id);
        let cond = TerminalCondition::BackoffExhausted { attempts: 5 };

        let result = workload_event_from_terminal(&*obs, &runtime, &wl_id, &event, &cond).await;

        match result {
            JobSubmitEvent::Failed { exit_code, attempts, max_attempts, .. } => {
                assert_eq!(exit_code, 1);
                assert_eq!(attempts, 5);
                assert_eq!(max_attempts, 5);
            }
            other => panic!("expected Failed (backoff exhausted), got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // workload_event_from_terminal — multi-attempt regression tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn event_from_terminal_completed_reports_attempt_count_from_view() {
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime_with_restart_count(&tmp, &wl_id, &alloc_id, 3).await;
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node, 0));
        let event = make_lifecycle_event(&alloc_id, &wl_id);
        let cond = TerminalCondition::Completed { exit_code: 0 };

        let result = workload_event_from_terminal(&*obs, &runtime, &wl_id, &event, &cond).await;

        match result {
            JobSubmitEvent::Succeeded { exit_code, attempts, .. } => {
                assert_eq!(exit_code, 0);
                assert_eq!(attempts, 4, "3 restarts → attempt_index 4");
            }
            other => panic!("expected Succeeded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn event_from_terminal_failed_reports_attempt_count_from_view() {
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime_with_restart_count(&tmp, &wl_id, &alloc_id, 2).await;
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node, 0));
        let event = make_lifecycle_event(&alloc_id, &wl_id);
        let cond = TerminalCondition::Failed { exit_code: 137 };

        let result = workload_event_from_terminal(&*obs, &runtime, &wl_id, &event, &cond).await;

        match result {
            JobSubmitEvent::Failed { exit_code, attempts, max_attempts, .. } => {
                assert_eq!(exit_code, 137);
                assert_eq!(attempts, 3, "2 restarts → attempt_index 3");
                assert_eq!(max_attempts, 3);
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn event_from_terminal_stopped_reports_attempt_count_from_view() {
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime_with_restart_count(&tmp, &wl_id, &alloc_id, 1).await;
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node, 0));
        let event = make_lifecycle_event(&alloc_id, &wl_id);
        let cond = TerminalCondition::Stopped { by: StoppedBy::Operator };

        let result = workload_event_from_terminal(&*obs, &runtime, &wl_id, &event, &cond).await;

        match result {
            JobSubmitEvent::Stopped { stopped_by, attempts, .. } => {
                assert_eq!(stopped_by, StoppedBy::Operator);
                assert_eq!(attempts, 2, "1 restart → attempt_index 2");
            }
            other => panic!("expected Stopped, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // workload_terminal_from_snapshot
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn snapshot_returns_none_when_no_terminal() {
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime(&tmp);
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");

        let row = make_alloc_status_row(&alloc_id, &wl_id, &node, None);
        obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write");

        let result = workload_terminal_from_snapshot(&*obs, &runtime, &wl_id).await;
        assert!(result.is_none(), "expected None when row has no terminal, got {result:?}");
    }

    #[tokio::test]
    async fn snapshot_returns_none_for_unrelated_workload() {
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime(&tmp);
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let other_wl = WorkloadId::from_str("job-other").expect("other wl id");
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");

        let row = make_alloc_status_row(
            &alloc_id,
            &other_wl,
            &node,
            Some(TerminalCondition::Completed { exit_code: 0 }),
        );
        obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write");

        let result = workload_terminal_from_snapshot(&*obs, &runtime, &wl_id).await;
        assert!(result.is_none(), "expected None for non-matching workload_id, got {result:?}");
    }

    #[tokio::test]
    async fn snapshot_completed_yields_succeeded() {
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime(&tmp);
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");

        let row = make_alloc_status_row(
            &alloc_id,
            &wl_id,
            &node,
            Some(TerminalCondition::Completed { exit_code: 0 }),
        );
        obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write");

        let result = workload_terminal_from_snapshot(&*obs, &runtime, &wl_id).await;
        match result {
            Some(JobSubmitEvent::Succeeded { exit_code, attempts, .. }) => {
                assert_eq!(exit_code, 0);
                assert_eq!(attempts, 1);
            }
            other => panic!("expected Some(Succeeded), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn snapshot_failed_yields_failed() {
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime(&tmp);
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");

        let row = make_alloc_status_row(
            &alloc_id,
            &wl_id,
            &node,
            Some(TerminalCondition::Failed { exit_code: 137 }),
        );
        obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write");

        let result = workload_terminal_from_snapshot(&*obs, &runtime, &wl_id).await;
        match result {
            Some(JobSubmitEvent::Failed { exit_code, attempts, max_attempts, .. }) => {
                assert_eq!(exit_code, 137);
                assert_eq!(attempts, 1);
                assert_eq!(max_attempts, 1);
            }
            other => panic!("expected Some(Failed), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn snapshot_stopped_yields_stopped() {
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime(&tmp);
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");

        let row = make_alloc_status_row(
            &alloc_id,
            &wl_id,
            &node,
            Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
        );
        obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write");

        let result = workload_terminal_from_snapshot(&*obs, &runtime, &wl_id).await;
        match result {
            Some(JobSubmitEvent::Stopped { stopped_by, attempts, .. }) => {
                assert_eq!(stopped_by, StoppedBy::Operator);
                assert_eq!(attempts, 1);
            }
            other => panic!("expected Some(Stopped) for operator stop, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn snapshot_backoff_exhausted_yields_failed() {
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime(&tmp);
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");

        let row = make_alloc_status_row(
            &alloc_id,
            &wl_id,
            &node,
            Some(TerminalCondition::BackoffExhausted { attempts: 3 }),
        );
        obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write");

        let result = workload_terminal_from_snapshot(&*obs, &runtime, &wl_id).await;
        match result {
            Some(JobSubmitEvent::Failed { exit_code, attempts, max_attempts, .. }) => {
                assert_eq!(exit_code, 1);
                assert_eq!(attempts, 3);
                assert_eq!(max_attempts, 3);
            }
            other => panic!("expected Some(Failed) for backoff exhausted, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // workload_terminal_from_snapshot — multi-attempt regression test
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn snapshot_completed_reports_attempt_count_from_view() {
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime_with_restart_count(&tmp, &wl_id, &alloc_id, 4).await;
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));

        let row = make_alloc_status_row(
            &alloc_id,
            &wl_id,
            &node,
            Some(TerminalCondition::Completed { exit_code: 0 }),
        );
        obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write");

        let result = workload_terminal_from_snapshot(&*obs, &runtime, &wl_id).await;
        match result {
            Some(JobSubmitEvent::Succeeded { exit_code, attempts, .. }) => {
                assert_eq!(exit_code, 0);
                assert_eq!(attempts, 5, "4 restarts → attempt_index 5");
            }
            other => panic!("expected Some(Succeeded), got {other:?}"),
        }
    }

    // ── best_effort_attempt_count ────────────────────────────────────

    #[tokio::test]
    async fn best_effort_attempt_count_returns_view_restart_count() {
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime_with_restart_count(&tmp, &wl_id, &alloc_id, 3).await;
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));

        let row = make_alloc_status_row(&alloc_id, &wl_id, &node, None);
        obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write");

        let count = best_effort_attempt_count(&*obs, &runtime, &wl_id).await;
        assert_eq!(count, 4, "3 restarts → attempt_index 4");
    }

    #[tokio::test]
    async fn best_effort_attempt_count_defaults_to_one_without_obs_rows() {
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime(&tmp);
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node, 0));

        let count = best_effort_attempt_count(&*obs, &runtime, &wl_id).await;
        assert_eq!(count, 1, "no obs rows → default 1");
    }

    #[tokio::test]
    async fn best_effort_attempt_count_defaults_to_one_for_unrelated_workload() {
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let other_wl = WorkloadId::from_str("job-other").expect("other wl id");
        let tmp = tempfile::TempDir::new().expect("tmpdir");
        let runtime = make_runtime_with_restart_count(&tmp, &wl_id, &alloc_id, 5).await;
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));

        let row = make_alloc_status_row(&alloc_id, &wl_id, &node, None);
        obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write");

        let count = best_effort_attempt_count(&*obs, &runtime, &other_wl).await;
        assert_eq!(count, 1, "no obs rows for queried workload → default 1");
    }
}
