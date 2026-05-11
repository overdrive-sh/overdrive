//! Streaming submit loop for `POST /v1/jobs` with `Accept: application/x-ndjson`.
//!
//! Per architecture.md §3 (happy path), §4 (broken-binary path), §5
//! (timeout path), and §10 (broadcast wiring). Slice 02 step 02-03.
//!
//! # Wiring
//!
//! The handler in [`crate::handlers::submit_workload`] branches on the
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
//! 2. `clock.sleep(cap)` — wall-clock cap timer. Production
//!    (`SystemClock`) parks on a real timer; DST (`SimClock`) parks
//!    until the harness calls `sim_clock.tick(cap + ε)`. On expiry,
//!    emits `ConvergedFailed { Timeout }` and ends the stream.
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
//!
//! # Missed-events classes
//!
//! Five classes of missed-events failure are bridged or surfaced by
//! the loop:
//!
//! 1. Live event delivery — the `bus.recv()` arm projects each
//!    `LifecycleEvent` and runs `check_terminal` against the obs store.
//! 2. `RecvError::Lagged(_)` — buffer-overflow: a slow subscriber fell
//!    behind and the broadcast channel evicted older messages. Bridged
//!    by the `lagged_recover` snapshot in the `Lagged` arm.
//! 3. `RecvError::Closed` — the `lifecycle_events` sender was dropped;
//!    the loop emits a `ConvergedFailed` terminal and ends.
//! 4. Cap timer — `clock.sleep(cap)` fires; the loop emits
//!    `ConvergedFailed { Timeout }` and ends.
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
//!    projects it to a terminal `SubmitEvent` and the stream ends
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
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use tokio::sync::broadcast;

use crate::AppState;
use crate::action_shim::LifecycleEvent;
use crate::api::{AllocStateWire, IdempotencyOutcome, SubmitEvent, TerminalReason};
use crate::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::reconciler::TargetResource;
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};

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
    workload_id: WorkloadId,
    accepted: SubmitEvent,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    let bus = state.lifecycle_events.clone();
    let clock = state.clock.clone();
    let obs = state.obs.clone();
    let cap = state.streaming_cap;

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

        // Drop the local `Arc<Sender>` clone immediately after
        // subscribing. The `Receiver` we just created keeps the
        // channel alive for our reads; retaining the `Sender` clone
        // here would keep the channel open even when every external
        // sender has dropped, making the `Err(RecvError::Closed)` arm
        // below unreachable in normal operation. Dropping `bus` lets
        // external senders (the action shim, exit observer) be the
        // sole owners of the channel's send-side — when they all drop,
        // our `sub.recv()` produces `Err(RecvError::Closed)` and we
        // emit `TerminalReason::StreamInterrupted`.
        drop(bus);

        // 3. Bridge the pre-subscribe window: the upstream
        //    `put_if_absent` + broker enqueue may have already
        //    triggered a reconcile tick that wrote a terminal
        //    `AllocStatusRow` and broadcast its `LifecycleEvent`
        //    BEFORE the subscriber above existed. Such events are
        //    permanently lost (`tokio::sync::broadcast::Sender::send`
        //    only delivers to receivers `subscribe()`-d at send-time).
        //    Reuse the same snapshot-and-classify primitive used for
        //    `Lagged` recovery: read `obs.alloc_status_rows()` once and,
        //    if the latest row for this job is already terminal, emit
        //    the terminal event and end the stream — analogous to
        //    ADR-0032 §7 lagged-recovery, applied to the subscribe-race
        //    rather than the buffer-overflow class.
        if let Some(terminal) = lagged_recover(&*obs, &workload_id).await {
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

        let cap_future = clock.sleep(cap);
        tokio::pin!(cap_future);

        loop {
            tokio::select! {
                biased;
                recv = sub.recv() => {
                    match recv {
                        Ok(event) => {
                            if event.workload_id != workload_id {
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

                            // Terminal-detection: project the
                            // event's reconciler-emitted terminal claim
                            // into the wire-shape `SubmitEvent` per
                            // ADR-0037 §4. The event's `terminal` field
                            // is the single source of truth — no view
                            // lookups, no `restart_counts >= CEILING`
                            // recomputation.
                            if let Some(terminal) = check_terminal(
                                &*obs,
                                &workload_id,
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
                                &workload_id,
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
                            // Channel closed — every `Sender` clone has
                            // dropped (server shutdown, action shim
                            // teardown, etc.) and no further events can
                            // arrive. Emit `TerminalReason::StreamInterrupted`
                            // to distinguish this server-side
                            // interruption from a wall-clock cap timeout
                            // (`TerminalReason::Timeout`) and from a
                            // driver-error terminal
                            // (`TerminalReason::DriverError`).
                            let terminal = SubmitEvent::ConvergedFailed {
                                alloc_id: None,
                                terminal_reason: TerminalReason::StreamInterrupted,
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
                // Wall-clock cap timer. Production (`SystemClock`)
                // parks on a real timer; DST (`SimClock`) parks until
                // the harness calls `sim_clock.tick(cap + ε)`. On
                // expiry, emit the timeout terminal and end the stream.
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

/// Phase 1 walking-skeleton workloads have `replicas == 1`, so any
/// single `state == Running` row for the job triggers `ConvergedRunning`.
/// The reconciler is the gate that decides when to emit
/// `Action::StartAllocation` per the desired replica count, so a row
/// only gets written if the reconciler says go — which is what makes
/// the single-row shortcut safe at `replicas == 1`.
///
/// Per ADR-0037 §4: terminal classification is the reconciler's
/// decision, durably stamped onto `Action.terminal` and threaded
/// onto `LifecycleEvent.terminal` by the action shim. This function
/// reads `event.terminal` as the single source of truth for the
/// terminal projection — no view lookups, no `restart_counts >=
/// CEILING` recomputation.
///
/// TODO(#140): gate `ConvergedRunning` on `running_count >=
/// replicas_desired` once a multi-replica workload lands. Hydrate
/// `replicas_desired` once at stream start rather than reading the
/// IntentStore per broadcast event.
async fn check_terminal(
    obs: &dyn ObservationStore,
    workload_id: &WorkloadId,
    event: &LifecycleEvent,
) -> Option<SubmitEvent> {
    // Terminal path — project the reconciler-emitted terminal claim
    // (per ADR-0037 §4) into the wire-shape SubmitEvent. Both
    // `AllocStatusRow.terminal` and `LifecycleEvent.terminal` carry
    // the same value from the same dispatch frame — drift is
    // structurally impossible.
    if let Some(cond) = &event.terminal {
        return Some(submit_event_from_terminal(cond, event));
    }

    // Success path — Running observation for this job → ConvergedRunning.
    // Phase 1 walking-skeleton workloads have replicas=1 so a single
    // running row in the obs store meets the bar; see TODO(#140) on the
    // function docstring for the multi-replica gate. We read the obs
    // store rather than trusting event.to alone so that a Running
    // broadcast event without a corresponding obs row (e.g. a pre-stop
    // Running event in the stop-while-streaming scenario) does NOT
    // prematurely close the stream.
    if matches!(event.to, AllocStateWire::Running) {
        if let Ok(rows) = obs.alloc_status_rows().await {
            let has_running = rows.iter().any(|r| {
                r.workload_id == *workload_id
                    && r.state == overdrive_core::traits::observation_store::AllocState::Running
            });
            if has_running {
                return Some(SubmitEvent::ConvergedRunning {
                    alloc_id: event.alloc_id.to_string(),
                    started_at: event.at.clone(),
                });
            }
        }
    }
    None
}

/// Project a reconciler-emitted [`TerminalCondition`] into the wire-
/// shape [`SubmitEvent`] terminal variant. Pure over its inputs;
/// reused by both the live-event path (`check_terminal`) and the
/// snapshot recovery path (`lagged_recover`) so the projection is in
/// exactly one place.
fn submit_event_from_terminal(cond: &TerminalCondition, event: &LifecycleEvent) -> SubmitEvent {
    match cond {
        TerminalCondition::Stopped { by } => {
            SubmitEvent::ConvergedStopped { alloc_id: Some(event.alloc_id.to_string()), by: *by }
        }
        TerminalCondition::BackoffExhausted { attempts } => SubmitEvent::ConvergedFailed {
            alloc_id: Some(event.alloc_id.to_string()),
            terminal_reason: TerminalReason::BackoffExhausted {
                attempts: *attempts,
                cause: event.reason.clone(),
            },
            reason: Some(event.reason.clone()),
            error: event.detail.clone(),
        },
        TerminalCondition::Custom { type_name, .. } => SubmitEvent::ConvergedFailed {
            alloc_id: Some(event.alloc_id.to_string()),
            terminal_reason: TerminalReason::DriverError { cause: event.reason.clone() },
            reason: Some(event.reason.clone()),
            error: Some(format!("custom terminal: {type_name}")),
        },
        // Forward-compat for future `#[non_exhaustive]` additions to
        // `TerminalCondition`. Render unknown terminals as a generic
        // ConvergedFailed{DriverError} carrying the event's existing
        // cause so the stream still terminates rather than silently
        // dropping the event.
        _ => SubmitEvent::ConvergedFailed {
            alloc_id: Some(event.alloc_id.to_string()),
            terminal_reason: TerminalReason::DriverError { cause: event.reason.clone() },
            reason: Some(event.reason.clone()),
            error: event.detail.clone(),
        },
    }
}

/// On `Lagged(_)` (or the pre-subscribe snapshot), inspect the
/// LWW-winner `AllocStatusRow`. Per ADR-0037 §4 the row's `terminal`
/// field is the authoritative durable surface for terminal claims —
/// no view consultation needed.
async fn lagged_recover(
    obs: &dyn ObservationStore,
    workload_id: &WorkloadId,
) -> Option<SubmitEvent> {
    let rows = obs.alloc_status_rows().await.ok()?;
    let latest = rows
        .into_iter()
        .filter(|r| r.workload_id == *workload_id)
        .max_by_key(|r| r.updated_at.counter)?;

    if let Some(cond) = &latest.terminal {
        // Promote the row to a synthetic LifecycleEvent shape so the
        // projection helper can be reused. `from` mirrors `to`
        // because we have no prior-state context in the snapshot.
        let to_wire: AllocStateWire = latest.state.into();
        let event = LifecycleEvent {
            alloc_id: latest.alloc_id.clone(),
            workload_id: latest.workload_id.clone(),
            from: to_wire,
            to: to_wire,
            reason: latest.reason.clone().unwrap_or(
                overdrive_core::TransitionReason::DriverInternalError { detail: String::new() },
            ),
            detail: latest.detail.clone(),
            source: crate::api::TransitionSource::Reconciler,
            at: format!("{}@{}", latest.updated_at.counter, latest.updated_at.writer.as_str()),
            terminal: Some(cond.clone()),
        };
        return Some(submit_event_from_terminal(cond, &event));
    }

    // Non-terminal — preserve the prior success-path semantics for
    // Running rows (Phase 1 walking-skeleton single-replica gate).
    match latest.state {
        AllocState::Running => Some(SubmitEvent::ConvergedRunning {
            alloc_id: latest.alloc_id.to_string(),
            started_at: format!(
                "{}@{}",
                latest.updated_at.counter,
                latest.updated_at.writer.as_str()
            ),
        }),
        _ => None,
    }
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

/// Build the synchronous `JobSubmitEvent::Accepted` event the handler
/// emits before entering the Job-kind streaming loop.
///
/// Per ADR-0047 §3 [D7]: Job-kind submits stream the per-kind sibling
/// event enum [`JobSubmitEvent`]; the `Accepted` variant mirrors the
/// existing legacy `SubmitEvent::Accepted` shape so wire-format
/// migration is a renamed-tag change, not a payload change.
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
/// enum [`JobSubmitEvent`]; the legacy flat [`SubmitEvent::ConvergedRunning`]
/// variant is structurally unreachable on this code path because the
/// type carries no equivalent variant — Jobs are run-to-completion and
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
///   terminal variant (`Succeeded` / `Failed`) before closing.
/// - **Edge cases**:
///   - Operator-stop converged terminal → `Succeeded { exit_code: 0 }`
///     (clean stop is the Job-kind success path; preserves the existing
///     `ConvergedStopped → exit code 0` semantics from the legacy lane).
///   - Cap timer expiry → `Failed { exit_code: -1 }` (no kernel exit
///     observed within the wall-clock budget — distinguished from a
///     genuine non-zero exit by the sentinel `-1`).
///   - Broadcast `Closed` → `Failed { exit_code: -1 }` (server-side
///     stream interruption — analogous to `TerminalReason::StreamInterrupted`
///     on the legacy lane).
///   - Pre-subscribe race / `Lagged(_)` → snapshot-and-classify the
///     latest LWW-winner row; emit the matching terminal if reached.
/// - **Observable invariants**: every Job-kind submit produces exactly
///   ONE terminal variant (`Succeeded` or `Failed`) on the wire; the
///   stream never closes silently. The CLI process exit code equals
///   the terminal `exit_code` field per KPI K1 honesty contract.
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

        // 2. Subscribe BEFORE the pre-subscribe snapshot recovery — same
        //    ordering as `build_stream` to bridge the pre-subscribe
        //    event window per architecture.md §10 / ADR-0032 §7.
        let mut sub = bus.subscribe();
        drop(bus);

        // 3. Pre-subscribe race recovery — if the latest row already
        //    carries a terminal claim, emit the matching JobSubmitEvent
        //    terminal and end.
        if let Some(terminal) = workload_terminal_from_snapshot(&*obs, &workload_id).await {
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
                                continue;
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
                            let terminal = JobSubmitEvent::Failed {
                                exit_code: -1,
                                duration: String::new(),
                                attempts: 1,
                                max_attempts: 1,
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
                    let terminal = JobSubmitEvent::Failed {
                        exit_code: -1,
                        duration: format!("{after_seconds}s"),
                        attempts: 1,
                        max_attempts: 1,
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
/// `serde_json::to_writer(buf, &event)?` + `b'\n'`. Mirror of
/// [`emit_line`] for the legacy [`SubmitEvent`] surface.
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
        return Some(workload_event_from_terminal(obs, workload_id, event, cond).await);
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
            Some(JobSubmitEvent::AttemptFailed {
                attempt_index,
                exit_code,
                duration: event.at.clone(),
                will_restart,
                next_attempt_delay: None,
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
    _job_id: &WorkloadId,
    event: &LifecycleEvent,
    cond: &TerminalCondition,
) -> JobSubmitEvent {
    let row = obs.alloc_status_row(&event.alloc_id).await.ok().flatten();
    let stderr_tail = row.as_ref().and_then(|r| r.stderr_tail.clone());
    match cond {
        TerminalCondition::Completed { exit_code } => JobSubmitEvent::Succeeded {
            exit_code: *exit_code,
            duration: event.at.clone(),
            attempts: 1,
        },
        TerminalCondition::Failed { exit_code } => JobSubmitEvent::Failed {
            exit_code: *exit_code,
            duration: event.at.clone(),
            attempts: 1,
            max_attempts: 1,
            stderr_tail,
        },
        TerminalCondition::Stopped { by } => {
            JobSubmitEvent::Stopped { stopped_by: *by, duration: event.at.clone(), attempts: 1 }
        }
        TerminalCondition::BackoffExhausted { attempts } => JobSubmitEvent::Failed {
            exit_code: 1,
            duration: event.at.clone(),
            attempts: *attempts,
            max_attempts: *attempts,
            stderr_tail,
        },
        // Forward-compat for future `#[non_exhaustive]` additions to
        // `TerminalCondition`. Render unknown terminals as a generic
        // `Failed` carrying the prior exit semantics so the stream
        // still terminates rather than silently dropping the event.
        _ => JobSubmitEvent::Failed {
            exit_code: 1,
            duration: event.at.clone(),
            attempts: 1,
            max_attempts: 1,
            stderr_tail,
        },
    }
}

/// On `Lagged(_)` or pre-subscribe snapshot, inspect the LWW-winner
/// `AllocStatusRow` for the job. If it already carries a terminal
/// claim, project to the matching `JobSubmitEvent` terminal.
async fn workload_terminal_from_snapshot(
    obs: &dyn ObservationStore,
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
    Some(match cond {
        TerminalCondition::Completed { exit_code } => {
            JobSubmitEvent::Succeeded { exit_code: *exit_code, duration, attempts: 1 }
        }
        TerminalCondition::Failed { exit_code } => JobSubmitEvent::Failed {
            exit_code: *exit_code,
            duration,
            attempts: 1,
            max_attempts: 1,
            stderr_tail,
        },
        TerminalCondition::Stopped { by } => {
            JobSubmitEvent::Stopped { stopped_by: *by, duration, attempts: 1 }
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
            attempts: 1,
            max_attempts: 1,
            stderr_tail,
        },
    })
}

// ---------------------------------------------------------------------------
// Job streaming sub-path — slice 02 of `workload-kind-discriminator`.
// ---------------------------------------------------------------------------
//
// Per ADR-0047 §3 [D2] + [D7]: Job-kind submits stream a per-kind
// sibling enum `JobSubmitEvent` whose variants make the historical
// false-positive "is running with N/M replicas (took live)" rendering
// structurally unreachable. The enum has NO `ConvergedRunning`
// variant — the conjunction of RCA root causes B+C+D is rendered
// impossible at the type level.
//
// Job semantics (run-to-completion):
//   * `Accepted` — synchronous first-line ack (mirrors the existing
//     `SubmitEvent::Accepted` shape).
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
/// Per ADR-0047 §3 [D2] / [D7]: Job kind has NO `ConvergedRunning`
/// variant — the conjunction of RCA root causes B+C+D is rendered
/// structurally unreachable for Job by the type system itself.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum JobSubmitEvent {
    /// Submit was accepted. Mirrors `SubmitEvent::Accepted` — first
    /// NDJSON line on the wire. Synchronous; no broadcast wait.
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
// Per ADR-0047 §3 [D2] / [D7]: Service-kind retains the existing
// `ConvergedRunning` shape — long-running workloads converge on
// "running" and stream remains observable. The legacy flat
// `SubmitEvent` continues to serve Service-kind for backward
// compatibility with existing consumers (this slice introduces the
// per-kind enum surface; later slices may collapse the flat shape).

/// Streaming events emitted by the Service submit sub-path.
///
/// Per ADR-0047 §3 [D7]: Service-kind retains `ConvergedRunning`
/// (long-running workloads converge on the live state).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ServiceSubmitEvent {
    /// Submit was accepted. Mirrors `SubmitEvent::Accepted`.
    Accepted { spec_digest: String, intent_key: String, outcome: crate::api::IdempotencyOutcome },
    /// Terminal — convergence reached `Running` with replicas met.
    ConvergedRunning { alloc_id: String, started_at: String },
    /// Terminal — convergence failed.
    ConvergedFailed {
        alloc_id: Option<String>,
        terminal_reason: crate::api::TerminalReason,
        error: Option<String>,
    },
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

/// Streaming events emitted by the Schedule submit sub-path.
///
/// Per ADR-0047 §3 / [D7]: two variants, both emitted synchronously
/// at submit time. `Accepted` mirrors the existing
/// [`SubmitEvent::Accepted`] shape; `Registered` carries the cron
/// expression echoed verbatim and the deferral tracking URL. The
/// stream closes after `Registered` — Schedule has no firing
/// semantics this slice (tracked at GH #166).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ScheduleSubmitEvent {
    /// Submit was accepted. Mirrors the existing `SubmitEvent::Accepted`
    /// payload — `spec_digest` is the canonical 64-char lowercase-hex
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
    use std::str::FromStr;
    use std::sync::Arc;

    use overdrive_core::TransitionReason;
    use overdrive_core::aggregate::WorkloadKind;
    use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
    use overdrive_core::traits::driver::DriverType;
    use overdrive_core::traits::observation_store::{
        AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
    };
    use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
    use overdrive_sim::adapters::observation_store::SimObservationStore;

    use crate::action_shim::LifecycleEvent;
    use crate::api::{AllocStateWire, SubmitEvent, TransitionSource};

    use super::{
        JobSubmitEvent, check_terminal, workload_event_from_terminal,
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
        }
    }

    // -----------------------------------------------------------------------
    // check_terminal
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn check_terminal_projects_event_terminal_to_converged_stopped() {
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node, 0));
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");
        let workload_id = WorkloadId::from_str("job-0").expect("job id");

        let event = LifecycleEvent {
            alloc_id: alloc_id.clone(),
            workload_id: workload_id.clone(),
            from: AllocStateWire::Running,
            to: AllocStateWire::Terminated,
            reason: TransitionReason::Stopped { by: StoppedBy::Reconciler },
            detail: None,
            source: TransitionSource::Driver(DriverType::Exec),
            at: "1@node-a".to_string(),
            terminal: Some(TerminalCondition::Stopped { by: StoppedBy::Reconciler }),
        };

        let result = check_terminal(&*obs, &workload_id, &event).await;

        assert!(
            matches!(result, Some(SubmitEvent::ConvergedStopped { .. })),
            "expected Some(ConvergedStopped), got {result:?}"
        );
    }

    // -----------------------------------------------------------------------
    // workload_event_from_terminal — one test per TerminalCondition variant
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn event_from_terminal_completed_yields_succeeded() {
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node, 0));
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let event = make_lifecycle_event(&alloc_id, &wl_id);
        let cond = TerminalCondition::Completed { exit_code: 0 };

        let result = workload_event_from_terminal(&*obs, &wl_id, &event, &cond).await;

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
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node, 0));
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let event = make_lifecycle_event(&alloc_id, &wl_id);
        let cond = TerminalCondition::Failed { exit_code: 42 };

        let result = workload_event_from_terminal(&*obs, &wl_id, &event, &cond).await;

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
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node, 0));
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let event = make_lifecycle_event(&alloc_id, &wl_id);
        let cond = TerminalCondition::Stopped { by: StoppedBy::Operator };

        let result = workload_event_from_terminal(&*obs, &wl_id, &event, &cond).await;

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
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node, 0));
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let event = make_lifecycle_event(&alloc_id, &wl_id);
        let cond = TerminalCondition::BackoffExhausted { attempts: 5 };

        let result = workload_event_from_terminal(&*obs, &wl_id, &event, &cond).await;

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
    // workload_terminal_from_snapshot
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn snapshot_returns_none_when_no_terminal() {
        let node = NodeId::from_str("node-a").expect("node id");
        let obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));
        let wl_id = WorkloadId::from_str("job-0").expect("wl id");
        let alloc_id = AllocationId::from_str("alloc-0").expect("alloc id");

        let row = make_alloc_status_row(&alloc_id, &wl_id, &node, None);
        obs.write(ObservationRow::AllocStatus(row)).await.expect("write");

        let result = workload_terminal_from_snapshot(&*obs, &wl_id).await;
        assert!(result.is_none(), "expected None when row has no terminal, got {result:?}");
    }

    #[tokio::test]
    async fn snapshot_returns_none_for_unrelated_workload() {
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
        obs.write(ObservationRow::AllocStatus(row)).await.expect("write");

        let result = workload_terminal_from_snapshot(&*obs, &wl_id).await;
        assert!(result.is_none(), "expected None for non-matching workload_id, got {result:?}");
    }

    #[tokio::test]
    async fn snapshot_completed_yields_succeeded() {
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
        obs.write(ObservationRow::AllocStatus(row)).await.expect("write");

        let result = workload_terminal_from_snapshot(&*obs, &wl_id).await;
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
        obs.write(ObservationRow::AllocStatus(row)).await.expect("write");

        let result = workload_terminal_from_snapshot(&*obs, &wl_id).await;
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
        obs.write(ObservationRow::AllocStatus(row)).await.expect("write");

        let result = workload_terminal_from_snapshot(&*obs, &wl_id).await;
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
        obs.write(ObservationRow::AllocStatus(row)).await.expect("write");

        let result = workload_terminal_from_snapshot(&*obs, &wl_id).await;
        match result {
            Some(JobSubmitEvent::Failed { exit_code, attempts, max_attempts, .. }) => {
                assert_eq!(exit_code, 1);
                assert_eq!(attempts, 3);
                assert_eq!(max_attempts, 3);
            }
            other => panic!("expected Some(Failed) for backoff exhausted, got {other:?}"),
        }
    }
}
