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

use std::num::NonZeroU32;

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
use overdrive_core::reconciler::{TargetResource, backoff_for_attempt};
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
/// `replicas_desired` is hydrated once at stream start (from the
/// validated `ServiceV1.replicas` aggregate the handler has in scope)
/// per the issue #140 fix. The streaming task captures it by value
/// (`NonZeroU32: Copy`) and passes it to both `check_terminal` and
/// `lagged_recover` so the `ConvergedRunning` gate (`running_count >=
/// replicas_desired`) is evaluated against the operator-declared
/// replica count, not the single-row shortcut. Phase 1 invariants
/// (immutable aggregate; re-submit of a different spec is `Conflict`)
/// guarantee `replicas_desired` cannot change mid-stream, so the
/// one-shot hydration is correct for the stream's entire lifetime.
///
/// The returned stream yields `Result<Bytes, std::io::Error>` items
/// that axum's `Body::from_stream(...)` wraps into the response body.
pub fn build_stream(
    state: AppState,
    workload_id: WorkloadId,
    accepted: SubmitEvent,
    replicas_desired: NonZeroU32,
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
        if let Some(terminal) = lagged_recover(&*obs, &workload_id, replicas_desired).await {
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
                                replicas_desired,
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
                                replicas_desired,
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

/// Project a single broadcast [`LifecycleEvent`] into a terminal-or-
/// running [`SubmitEvent`], gated on the operator-declared
/// `replicas_desired` per issue #140.
///
/// Per ADR-0037 §4: terminal classification is the reconciler's
/// decision, durably stamped onto `Action.terminal` and threaded onto
/// `LifecycleEvent.terminal` by the action shim. This function reads
/// `event.terminal` as the single source of truth for the terminal
/// projection — no view lookups, no `restart_counts >= CEILING`
/// recomputation.
///
/// Contract (per `.claude/rules/development.md` § *Trait definitions
/// specify behavior, not just signature*):
///
/// * **Preconditions** — `obs` is the live observation store wired
///   into `AppState`; `workload_id` is the Service the stream is
///   bound to; `replicas_desired` is the validated `ServiceV1.replicas`
///   carrying the `NonZero` invariant from the constructor.
/// * **Postconditions** — returns `Some(ConvergedRunning { ... })`
///   only when **every** of the following holds: `event.terminal` is
///   `None`, `event.to == Running`, and the observation store
///   carries at least `replicas_desired.get()` rows with
///   `(workload_id == self, state == Running)`. Returns
///   `Some(<terminal variant>)` whenever `event.terminal.is_some()`,
///   bypassing the running-count gate (fail-fast on any single
///   terminal claim — RCA §7). Otherwise returns `None`.
/// * **Edge cases** — `replicas_desired == 1` behaves identically to
///   the prior single-row shortcut by construction
///   (`running_count >= 1` is equivalent to `rows.any(state ==
///   Running)`). An obs read that returns `Err(_)` is treated as
///   "not yet converged" and the function returns `None`; the next
///   broadcast event will re-attempt the snapshot.
/// * **Invariant** — a single terminal claim closes the stream
///   regardless of the running-count gate state. This matches the
///   pre-fix terminal-projection semantics (RCA §7 Q1 / Q2): any
///   one allocation's `BackoffExhausted` / `Stopped` / `Custom`
///   surfaces immediately, even with `replicas_desired > 1` and
///   other allocations still pending or running.
async fn check_terminal(
    obs: &dyn ObservationStore,
    workload_id: &WorkloadId,
    event: &LifecycleEvent,
    replicas_desired: NonZeroU32,
) -> Option<SubmitEvent> {
    // Terminal path — project the reconciler-emitted terminal claim
    // (per ADR-0037 §4) into the wire-shape SubmitEvent. Both
    // `AllocStatusRow.terminal` and `LifecycleEvent.terminal` carry
    // the same value from the same dispatch frame — drift is
    // structurally impossible. Fail-fast: a single terminal claim
    // closes the stream regardless of how many other allocations
    // are still pending or running (RCA §7).
    if let Some(cond) = &event.terminal {
        return Some(submit_event_from_terminal(cond, event));
    }

    // Success path — emit `ConvergedRunning` only when at least
    // `replicas_desired` rows for this workload have reached
    // `state == Running`. We read the obs store rather than trusting
    // `event.to` alone so that a Running broadcast event without a
    // corresponding obs row (e.g. a pre-stop Running event in the
    // stop-while-streaming scenario) does NOT prematurely close the
    // stream — and so the count is computed against the durable
    // surface, not a transient broadcast view.
    if matches!(event.to, AllocStateWire::Running) {
        if let Ok(rows) = obs.alloc_status_rows().await {
            let running_count: u32 = rows
                .iter()
                .filter(|r| r.workload_id == *workload_id && r.state == AllocState::Running)
                .count()
                .try_into()
                .unwrap_or(u32::MAX);
            if running_count >= replicas_desired.get() {
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

/// On `Lagged(_)` (or the pre-subscribe snapshot), classify the
/// observation-store snapshot for this workload against the
/// operator-declared `replicas_desired`. Per ADR-0037 §4 the row's
/// `terminal` field is the authoritative durable surface for
/// terminal claims — no view consultation needed.
///
/// Contract (per `.claude/rules/development.md` § *Trait definitions
/// specify behavior, not just signature*):
///
/// * **Preconditions** — `obs` is the live observation store;
///   `workload_id` is the Service the stream is bound to;
///   `replicas_desired` carries the `NonZero` invariant from the
///   validated `ServiceV1.replicas`. Called at exactly two sites in
///   [`build_stream`]: the pre-subscribe-window bridge and the
///   `broadcast::error::RecvError::Lagged(_)` recovery arm.
/// * **Postconditions** — returns `Some(<terminal variant>)` when
///   the LWW-winner row for `workload_id` carries
///   `Some(TerminalCondition)` (fail-fast: any single terminal row
///   closes the stream regardless of running-count). Returns
///   `Some(ConvergedRunning { ... })` when the count of rows with
///   `state == Running` for `workload_id` meets or exceeds
///   `replicas_desired.get()`; the emitted `alloc_id` / `started_at`
///   identify the **most-recently-updated** Running row (not
///   necessarily the LWW winner of the whole row set, which may be
///   a non-Running transition even when sibling Running counts have
///   met the gate). Otherwise returns `None`.
/// * **Edge cases** — `replicas_desired == 1` behaves identically to
///   the prior single-row shortcut by construction. Zero observation
///   rows for `workload_id` returns `None`. An obs read failure
///   returns `None` (the next broadcast event re-classifies).
/// * **Invariant** — terminal-projection bypasses the running-count
///   gate (RCA §7 Q1 / Q2 — fail-fast semantics preserved across the
///   replicas-aware refactor).
async fn lagged_recover(
    obs: &dyn ObservationStore,
    workload_id: &WorkloadId,
    replicas_desired: NonZeroU32,
) -> Option<SubmitEvent> {
    let rows = obs.alloc_status_rows().await.ok()?;
    let job_rows: Vec<_> = rows.into_iter().filter(|r| r.workload_id == *workload_id).collect();
    // Fail-fast: ANY terminal row closes the stream regardless of
    // running-count (docstring invariant, RCA §7 Q1/Q2).  Scan all
    // rows, not just the LWW winner, because a sibling allocation may
    // have a higher counter yet be non-terminal.
    if let Some(terminal_row) =
        job_rows.iter().filter(|r| r.terminal.is_some()).max_by_key(|r| r.updated_at.counter)
    {
        let cond = terminal_row
            .terminal
            .as_ref()
            .unwrap_or_else(|| unreachable!("filter guarantees terminal.is_some()"));
        let to_wire: AllocStateWire = terminal_row.state.into();
        let event = LifecycleEvent {
            alloc_id: terminal_row.alloc_id.clone(),
            workload_id: terminal_row.workload_id.clone(),
            from: to_wire,
            to: to_wire,
            reason: terminal_row.reason.clone().unwrap_or(
                overdrive_core::TransitionReason::DriverInternalError { detail: String::new() },
            ),
            detail: terminal_row.detail.clone(),
            source: crate::api::TransitionSource::Reconciler,
            at: format!(
                "{}@{}",
                terminal_row.updated_at.counter,
                terminal_row.updated_at.writer.as_str()
            ),
            terminal: Some(cond.clone()),
        };
        return Some(submit_event_from_terminal(cond, &event));
    }

    // Guard: need at least one row to proceed to the running-count gate.
    if job_rows.is_empty() {
        return None;
    }

    // Non-terminal — count Running rows for this workload and emit
    // `ConvergedRunning` only when the count meets the operator's
    // `replicas_desired`. The `alloc_id` / `started_at` seed for the
    // wire event is the most-recently-updated Running row — `latest`
    // may itself be a non-Running transition (e.g. a Pending row
    // that arrived after sibling Running rows already met the gate).
    let running_count: u32 = job_rows
        .iter()
        .filter(|r| r.state == AllocState::Running)
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    if running_count >= replicas_desired.get() {
        let running = job_rows
            .iter()
            .filter(|r| r.state == AllocState::Running)
            .max_by_key(|r| r.updated_at.counter)
            .unwrap_or_else(|| {
                unreachable!("running_count >= 1 guarantees at least one Running row")
            });
        return Some(SubmitEvent::ConvergedRunning {
            alloc_id: running.alloc_id.to_string(),
            started_at: format!(
                "{}@{}",
                running.updated_at.counter,
                running.updated_at.writer.as_str()
            ),
        });
    }
    None
}

/// Build the synchronous `Accepted` event the handler emits before
/// entering the streaming loop.
///
/// `vip` is the allocator-issued Service VIP per ADR-0049 (amended
/// 2026-05-15); the legacy Service streaming lane carries it on
/// `SubmitEvent::Accepted` so a consumer of the NDJSON Service stream
/// observes the same VIP the JSON `SubmitWorkloadResponse` echoes.
/// Pass `None` for Schedule / non-Service streams.
#[must_use]
pub fn build_accepted(
    spec_digest: String,
    intent_key: String,
    outcome: IdempotencyOutcome,
    vip: Option<String>,
) -> SubmitEvent {
    SubmitEvent::Accepted { spec_digest, intent_key, outcome, vip }
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
///     stream interruption — analogous to `TerminalReason::StreamInterrupted`
///     on the legacy lane).
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

        // 2. Subscribe BEFORE the pre-subscribe snapshot recovery — same
        //    ordering as `build_stream` to bridge the pre-subscribe
        //    event window per architecture.md §10 / ADR-0032 §7.
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

/// Wire-shape declaration for the Schedule submit streaming sub-path.
///
/// **Not currently emitted on any server code path.** `handlers.rs`
/// routes `WorkloadKind::Schedule` through the legacy `build_stream`
/// path, which emits [`SubmitEvent`]-shaped NDJSON. This enum is the
/// forward-declared wire shape for when that dispatch is wired up;
/// tracked at GH #166.
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
    use std::collections::BTreeMap;
    use std::num::NonZeroU32;
    use std::str::FromStr;
    use std::sync::Arc;

    use overdrive_core::TransitionReason;
    use overdrive_core::aggregate::WorkloadKind;
    use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
    use overdrive_core::reconciler::WorkloadLifecycleView;
    use overdrive_core::traits::driver::DriverType;
    use overdrive_core::traits::observation_store::{
        AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
    };
    use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
    use overdrive_sim::adapters::observation_store::SimObservationStore;

    use crate::action_shim::LifecycleEvent;
    use crate::api::{AllocStateWire, SubmitEvent, TransitionSource};
    use crate::reconciler_runtime::ReconcilerRuntime;

    use super::{
        JobSubmitEvent, best_effort_attempt_count, check_terminal, workload_event_from_terminal,
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
        let target = overdrive_core::reconciler::TargetResource::new(&format!("job/{workload_id}"))
            .expect("target");
        let view = WorkloadLifecycleView {
            restart_counts: BTreeMap::from([(alloc_id.clone(), restart_count)]),
            ..Default::default()
        };
        runtime.seed_workload_lifecycle_view_for_test(&target, view);
        runtime
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

        // `replicas_desired` is immaterial here — `event.terminal.is_some()`
        // takes the fail-fast terminal-projection branch which bypasses
        // the running-count gate per the RCA §7 invariant.
        let replicas_desired = NonZeroU32::new(1).expect("non-zero");
        let result = check_terminal(&*obs, &workload_id, &event, replicas_desired).await;

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
        obs.write(ObservationRow::AllocStatus(row)).await.expect("write");

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
        obs.write(ObservationRow::AllocStatus(row)).await.expect("write");

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
        obs.write(ObservationRow::AllocStatus(row)).await.expect("write");

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
        obs.write(ObservationRow::AllocStatus(row)).await.expect("write");

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
        obs.write(ObservationRow::AllocStatus(row)).await.expect("write");

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
        obs.write(ObservationRow::AllocStatus(row)).await.expect("write");

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
        obs.write(ObservationRow::AllocStatus(row)).await.expect("write");

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
        obs.write(ObservationRow::AllocStatus(row)).await.expect("write");

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
        obs.write(ObservationRow::AllocStatus(row)).await.expect("write");

        let count = best_effort_attempt_count(&*obs, &runtime, &other_wl).await;
        assert_eq!(count, 1, "no obs rows for queried workload → default 1");
    }
}
