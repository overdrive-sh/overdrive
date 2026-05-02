# RCA — `exit_observer` obs-write failure leaves alloc stuck Running

**Status**: Approved fix direction (Option A) — 2026-05-02
**Reporter**: code review comment on `crates/overdrive-control-plane/src/worker/exit_observer.rs:186-207`
**Investigator**: nw-troubleshooter

## Defect

`exit_observer.rs:188-196` (the `Err` arm of the `match handle_exit_event(...)` block in `spawn_with_runtime`'s loop) logs an obs-write failure at `warn!` and `continue`s the loop. When `ObservationStore::write` rejects the exit row (e.g. a redb write error), the alloc's row in obs remains `Running` while the process is actually dead. There is no retry, no alternative signal channel, and no escalation — the failure is silently absorbed.

The reporter's surface-level fix (move the re-enqueue at `exit_observer.rs:200-205` outside the success-only branch) does **not** unstick the alloc. See *Why the reporter's fix doesn't help* below.

## Five Whys

### WHY 1 — Symptom

Alloc remains observed as `Running` after process exit when `obs.write` fails.

- `exit_observer.rs:166-197` — `Err` arm logs and `continue`s without touching obs again. Prior `Running` row stands.

### WHY 2 — Re-enqueue alone cannot help

Even if the re-enqueue ran, the reconciler cannot detect the dead process.

- `reconciler_runtime.rs:494-518` — `hydrate_actual` for `JobLifecycle` reads exclusively from `state.obs.alloc_status_rows()`. No driver-side liveness probe, no `driver.status()`, no PID check.
- `reconciler.rs:1066-1069` — when reconcile sees a `Running` alloc for the desired job, it returns `(Vec::new(), view.clone())`. No actions, no work.

### WHY 3 — Stale `Running` row dominates

The obs store still carries the prior `Running` row because the new `Failed`/`Terminated` write was rejected and the observer dropped it.

- `exit_observer.rs:243` is the only write site in this path; on `Err` it returns `HandleError::Observation` (line 346-347), the caller logs at warn (line 188-194), then `continue`. No retry. No alternative signal channel.
- `exit_observer.rs:230-233` — the new row's `LogicalTimestamp` already correctly increments the prior counter. Under LWW it would dominate. The dominance never materialises because the write itself failed.

### WHY 4 — No retry, no alternative signal, no health bit

The error-handling design treats an obs-write failure as "log and forget" — it does not classify the failure (transient redb retry vs permanent rejection vs schema error) and has no fallback path that surfaces "this alloc has a known-dead process."

- `exit_observer.rs:188-196` — single `Err` arm, no match on error kind, no retry counter, no fallback like writing a degraded-health row, no panic to force operator notice. The classification work was already done (`classify` at `exit_observer.rs:251-272`); the result is discarded on write failure.
- `exit_observer.rs:343-347` — `HandleError` has exactly one variant (`Observation`), which transparently wraps `ObservationStoreError` via `#[from]`. Callers cannot distinguish kinds; the only contract is "write failed, somehow."

### WHY 5 — Obs is the sole observation surface (Phase 1 design choice)

The Phase 1 architecture intentionally consolidates "what is" into the ObservationStore. The `Driver::status()` trait method exists but has zero callers — the prior RCA reified "every observation flows through the obs store" by routing exit detection through a single owner-writer (the `exit_observer`) into a single rendezvous (`AllocStatusRow`). That single rendezvous is also a single point of failure.

- `docs/feature/fix-exec-driver-exit-watcher/deliver/rca.md:9-14` — "convergence loop reads actual state exclusively from `obs.alloc_status_rows()`; `driver.status()` exists on the trait ... but has zero callers."

### Root cause

**The `exit_observer` write path is the single durable channel by which a process exit becomes a reconciler-visible event, and it has no failure handling beyond log-and-drop.** Re-enqueuing the reconciler does nothing because the reconciler reads only the obs store, and the obs store still says `Running`. The bug is "no failure handling on the only signal channel," not "missing re-enqueue."

## Why the reporter's fix doesn't help

Trace the post-fix behaviour step by step (assume re-enqueue is moved outside the `Err` arm):

1. Process exits → `ExitEvent` arrives at `exit_observer`.
2. `handle_exit_event` calls `obs.write(...)` → `Err(ObservationStoreError)`.
3. Observer logs warn, falls through to re-enqueue block.
4. `target_for_event` (`exit_observer.rs:294-300`) reads the prior `Running` row, builds `target = job/<id>`, returns `Some(target)`.
5. `runtime.broker().submit(...)` — broker now has a pending evaluation.
6. Next drain tick fires `run_convergence_tick` for `(job-lifecycle, job/<id>)`.
7. `hydrate_actual` reads `obs.alloc_status_rows()` — gets the stale `Running` row.
8. `JobLifecycle::reconcile` sees `running_alloc.is_some()` for the desired job → returns `(Vec::new(), view.clone())`.
9. `has_work = false` → no self-re-enqueue. Broker drains empty. Tick loop sleeps. **State is identical to the original bug.**

The only thing the reporter's fix changes: the broker spins one extra time before going idle. The alloc is still stuck.

## Approved fix — Option A (bounded retry + classification + escalation)

Treat obs-write failure as a transient condition for known-retryable error kinds and retry with bounded backoff before falling back to a louder failure mode. The exit row is small and idempotent under LWW (the `LogicalTimestamp.counter` is captured once from `find_prior_row` per `handle_exit_event` call; the row is bit-identical across retries within one call → re-write is safe). On final retry exhaustion, escalate.

**Behaviour**:

1. Classify `ObservationStoreError` into retryable-transient vs terminal at the observer's vantage point.
2. Retry retryable kinds up to N times with exponential backoff. Backoff uses the injected `Clock` trait (not `tokio::time::sleep`) so DST stays deterministic.
3. On terminal-or-exhausted: emit `tracing::error!` AND synthesize a degraded `LifecycleEvent` so `submit --watch` subscribers see the failure surface.
4. Keep the re-enqueue gated on `Ok(_)` and add a doc-comment explaining why: the reconciler cannot make progress on a stale `Running` row, so re-enqueueing without a new row is a busy-loop trap.

**Out of scope** (rejected alternatives):

- **Option B (panic the observer task)**: trades one stuck-alloc for total observation blackout for the rest of the process lifetime; Phase 1 wiring does not restart the observer.
- **Option C (move re-enqueue + add `Driver::status()` probe to `hydrate_actual`)**: refactor not bugfix; re-introduces architectural debt the prior `fix-exec-driver-exit-watcher` RCA explicitly resolved.
- **Reporter's fix as stated (move re-enqueue outside `Err`)**: does not fix the bug.

## Files affected

| Path | Change |
|---|---|
| `crates/overdrive-control-plane/src/worker/exit_observer.rs` | Wrap `handle_exit_event` call in bounded retry loop; classify `Err` into retryable-transient vs terminal; on terminal-or-exhausted, emit `error!` and synthesise a degraded `LifecycleEvent`; keep re-enqueue gated on success path with explanatory comment; thread `Clock` through `spawn_with_runtime` |
| `crates/overdrive-core/src/traits/observation_store.rs` | Split `ObservationStoreError` into kinds; add `is_retryable()` predicate. Verify before touching — may already exist in some shape |
| `crates/overdrive-control-plane/src/lib.rs` | Pass injected `Clock` through to `spawn_with_runtime` |
| `crates/overdrive-sim/src/adapters/observation_store.rs` | Add `inject_write_failure(kind)` to `SimObservationStore` so the retry path can be exercised under DST |
| `crates/overdrive-control-plane/tests/integration/job_lifecycle/crash_recovery_obs_write_rejected.rs` | New regression test: inject a transient write failure on the exit row, assert the retry succeeds and the alloc transitions to Failed; inject a terminal failure, assert the degraded `LifecycleEvent` surfaces |

## Risk assessment

**Regression surface**:

- The retry loop introduces a new wall-clock-sensitive shape into the observer; using the injected `Clock` keeps DST deterministic but changes `spawn_with_runtime`'s signature, rippling to `lib.rs::run_server_with_obs_and_driver`.
- The synthesised terminal `LifecycleEvent` will surface to `submit --watch` subscribers; the renderer must handle it gracefully.

**Race conditions**:

- Reading `find_prior_row` per retry is required if redb's view advanced between retries (e.g. a successful reconciler-driven write inserted a new row). The retry granularity must be the whole `handle_exit_event`, not just the inner write, to keep the LWW counter monotonic.
- The mpsc channel between watcher and observer queues backed-up events during the retry loop; check the `Driver::take_exit_receiver` capacity is bounded enough to avoid memory growth but generous enough to absorb retry latency.

**Test coverage gaps closed by this work**:

- DST invariant currently absent: "every running alloc whose exit-event fired eventually shows non-Running in obs OR a terminal-failure event is broadcast." Same gap noted in `fix-exec-driver-exit-watcher/deliver/rca.md:34-35` as a contributing factor for the parent bug — this is the same gap manifesting at a different layer. Add the invariant as part of this work.
- `SimObservationStore` does not currently model write rejection. Add `inject_write_failure(kind)`.
- `crash_recovery.rs` asserts the success path; add `crash_recovery_obs_write_rejected.rs` for the failure paths (transient retried-and-recovered, terminal escalated).

## Contributing factors

1. **No error-kind classification on `ObservationStoreError`** at the observer's vantage point. Single opaque variant collapses transient and permanent failures.
2. **Re-enqueue gated on success path with no comment explaining why**. Future contributors and reviewers read it as "missing the failure path" rather than "intentionally not re-enqueuing because the reconciler cannot make progress without a new row." The reporter's review comment is the predicted form of that confusion.
3. **No DST invariant for "exit event → eventual obs convergence"** — same gap noted in the prior RCA's contributing factors, never closed.
4. **Single observation surface design** (Phase 1 obs-as-truth) means a write failure is a control-plane invariant violation with no fallback channel. Acceptable for Phase 1 scope but warrants louder failure semantics than `warn!`.

## Files cited

- `crates/overdrive-control-plane/src/worker/exit_observer.rs` (lines 166-207, 188-196, 200-205, 219-245, 251-272, 284-289, 343-347)
- `crates/overdrive-control-plane/src/reconciler_runtime.rs` (lines 224-347, 494-520, 422-430)
- `crates/overdrive-core/src/reconciler.rs` (lines 963-1216, esp. 1066-1069, 1123-1174)
- `docs/feature/fix-exec-driver-exit-watcher/deliver/rca.md` (lines 9-14, 34-35, 84-95)
