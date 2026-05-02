# fix-exit-observer-write-retry — Feature Evolution

**Feature ID**: fix-exit-observer-write-retry
**Type**: Bug fix (`/nw-bugfix` → `/nw-deliver`)
**Branch**: `marcus-sa/phase1-first-workload`
**Date**: 2026-05-02
**Commits**:
- `2080daa` — `test(control-plane): RED — exit_observer obs-write failure regression`
- `23487d9` — `fix(control-plane): bounded retry + terminal escalation on exit-observer obs-write failure`

**Status**: Delivered.

---

## Symptom

A code-review comment on `crates/overdrive-control-plane/src/worker/exit_observer.rs:186-207` flagged the `Err` arm of the `match handle_exit_event(...)` block: when `ObservationStore::write` rejects the exit row, the observer logs at `warn!` and `continue`s. The reporter's surface fix was "move the re-enqueue at lines 200-205 outside the success-only branch so the reconciler picks the work up again." Investigation showed the reporter's fix does NOT unstick the alloc — re-enqueueing the reconciler against a stale `Running` row is a no-op because the reconciler reads observation exclusively, sees `Running`, and emits zero actions. The bug was real (silent log-and-drop on the only durable signal channel for process exit), but the fix the report proposed would have masked the bug rather than resolved it.

## Root cause

**The `exit_observer` write path is the single durable channel by which a process exit becomes a reconciler-visible event, and it has no failure handling beyond log-and-drop.** Five-whys traced the failure surface through `exit_observer.rs:188-196` (single opaque `Err` arm, no retry, no kind classification, no fallback) → `reconciler_runtime.rs:494-518` (`hydrate_actual` reads only `obs.alloc_status_rows()`, no `Driver::status()` probe) → `reconciler.rs:1066-1069` (a `Running` alloc for the desired job emits zero actions). Phase 1's deliberate "obs is the sole observation surface" design (per `fix-exec-driver-exit-watcher` RCA) means a write failure is a control-plane invariant violation with no fallback — but the failure semantics were `warn!` and continue, which is too quiet for a violation that strands the alloc. Contributing factors: no `is_retryable()` predicate on `ObservationStoreError` (single opaque variant), no DST invariant for "exit event → eventual obs convergence" (same gap noted in the prior RCA, never closed), and `SimObservationStore` had no way to model write rejection.

## Fix

**Approved fix shape**: **Option A** — bounded retry + classification + terminal escalation. Rejected alternatives: **Option B** (panic the observer task — trades stuck-alloc for total observation blackout for the rest of the process lifetime), **Option C** (move re-enqueue + add `Driver::status()` probe to `hydrate_actual` — refactor not bugfix; re-introduces architectural debt the prior `fix-exec-driver-exit-watcher` RCA explicitly resolved), and the **reporter's fix as stated** (move re-enqueue outside `Err` — does not fix the bug; the busy-loop trap is documented in the RCA's "Why the reporter's fix doesn't help" section).

The implementation adds `ObservationStoreError::is_retryable()` classification on the trait, wraps `handle_exit_event` in a bounded retry loop (3 attempts, 50/100/200ms exponential backoff via the injected `Clock` trait so DST stays deterministic), and on terminal-or-exhausted emits `tracing::error!` AND synthesises a degraded `LifecycleEvent` so `submit --watch` subscribers see the failure surface. The re-enqueue stays gated on the `Ok(_)` arm with a doc-comment explaining why: re-enqueueing the reconciler against a stale `Running` row is a no-op because the reconciler reads observation exclusively, so re-enqueueing without a new row is a busy-loop trap. The retry granularity is the whole `handle_exit_event` (not just the inner write) to keep the LWW counter monotonic across retries — `find_prior_row` re-runs per attempt.

## Files changed

`git diff --stat 2080daa^..23487d9`:

| Path | Lines | Role |
|---|---|---|
| `crates/overdrive-core/src/traits/observation_store.rs` | +113 | `is_retryable()` predicate + error-kind split |
| `crates/overdrive-control-plane/src/worker/exit_observer.rs` | +181/-18 | Retry loop, classification, terminal escalation, doc-comment on re-enqueue gate |
| `crates/overdrive-control-plane/src/lib.rs` | +1 | Thread injected `Clock` through `spawn_with_runtime` |
| `crates/overdrive-sim/src/adapters/observation_store.rs` | +43 | `inject_write_failure(kind)` to exercise retry path under DST |
| `crates/overdrive-control-plane/tests/integration/job_lifecycle/crash_recovery_obs_write_rejected.rs` | +303 | New regression test (transient + terminal) |
| `crates/overdrive-control-plane/tests/integration/job_lifecycle/exit_observer.rs` | +7/-0 | Wiring update for new `Clock` parameter |
| `crates/overdrive-control-plane/tests/integration/job_lifecycle/crash_recovery.rs` | +1 | Wiring update for new `Clock` parameter |
| `crates/overdrive-control-plane/tests/integration.rs` | +1 | Module declaration for new test file |

Total: 650 insertions, 18 deletions across 8 files.

## Tests added

- **NEW**: `crates/overdrive-control-plane/tests/integration/job_lifecycle/crash_recovery_obs_write_rejected.rs`:
  - `transient_obs_write_recovers_on_retry` — injects a transient retryable write failure on the exit row; asserts the retry succeeds and the alloc transitions to a terminal state, and the broker is re-enqueued exactly once on the recovering write.
  - `terminal_obs_write_escalates_via_lifecycle_event` — injects a terminal (non-retryable) write failure; asserts a degraded `LifecycleEvent` is broadcast and `tracing::error!` is emitted; asserts the broker is NOT re-enqueued (the busy-loop trap from the RCA).

- **No existing fixture needed updating** beyond mechanical `Clock` wiring on `crash_recovery.rs` and `exit_observer.rs` integration tests.

## Quality gates

- **DES integrity** — both steps have complete 5-phase traces in `docs/feature/fix-exit-observer-write-retry/deliver/execution-log.json` (PREPARE, RED_ACCEPTANCE, RED_UNIT, GREEN, COMMIT for 01-01; the same shape for 01-02 with RED_UNIT correctly SKIPPED + reason `integration test asserts retry behaviour through the public spawn entrypoint`).
- **Workspace nextest** on Linux via Lima (`cargo xtask lima run -- cargo nextest run --workspace --features integration-tests`) — the targeted retry tests pass and the affected `worker` / `job_lifecycle` integration suite is green.
- **Mutation gate** — **SKIPPED per user instruction during finalize.** No `cargo xtask mutants` evidence collected for this delivery.
- **Refactor + adversarial review** — **SKIPPED per user instruction during finalize.**

## Out of scope (flagged for follow-up)

- **`overdrive-worker::stop_escalates_to_sigkill_when_sigterm_ignored` regression** — surfaced by the full Lima run, lives in a different crate, was introduced by prior commits per `git log -- crates/overdrive-worker/`. Tracked separately; explicitly out of scope for this bugfix per user direction.
- **DST invariant for "exit event → eventual obs convergence OR terminal-failure event"** — the contributing factor named in the RCA. The new regression tests cover the success+failure branches at the integration layer for this code path; promoting the same invariant to a DST property over the full state space (every running alloc whose exit-event fired eventually transitions in obs OR broadcasts terminal-failure) remains open and is the same gap noted in the prior `fix-exec-driver-exit-watcher` RCA.
- **Mutation testing** of the new retry/classification logic — explicitly skipped per user instruction. The RCA's risk-assessment section identifies the structurally novel branches (retryable-vs-terminal classification, retry-counter bounds, degraded-`LifecycleEvent` synthesis); without mutation evidence the suite's defensiveness on those branches is unverified.
- **`Refactor` and `Adversarial Review` phases** — both skipped during this finalize.

## References

- **RCA (durable spec for this fix)**: `docs/feature/fix-exit-observer-write-retry/deliver/rca.md` — the workspace itself was finalized into `docs/evolution/` (this file), but the RCA's body is the spec; it is preserved verbatim in git history as part of commit `23487d9`'s tree.
- **Prior obs-as-truth fix**: `docs/evolution/2026-05-01-fix-exec-driver-exit-watcher.md` — same architectural seam (single observation surface, exit-event → obs convergence); this fix closes the failure-handling gap that the prior fix's design left implicit.
- **Test discipline**: `.claude/rules/testing.md` § "RED scaffolds and intentionally-failing commits", § "Lima rule for `--features integration-tests`".
- **Reconciler purity contract**: `.claude/rules/development.md` § "Reconciler I/O" — the `reconcile` function reads observation as input state, which is why re-enqueueing against a stale `Running` row cannot make progress and the reporter's fix would have been a no-op.
- **Commits**: `2080daa` (RED), `23487d9` (GREEN).
