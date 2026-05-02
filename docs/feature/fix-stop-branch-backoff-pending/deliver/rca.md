# RCA: Stop branch leaves backoff-pending view intact — convergence loop spins indefinitely

## Bug summary

When `desired_to_stop = true` and all allocations are already in non-`Running`
states (e.g., `Failed` mid-retry with `restart_counts < CEILING`), the Stop
branch in `JobLifecycleReconciler::reconcile` returns `([], view.clone())`
without clearing the outstanding `next_attempt_at` entry. The runtime's
`view_has_backoff_pending` predicate sees the unchanged entry, sets
`has_work = true`, and the broker re-enqueues the target every tick until
either:

- `restart_counts` reaches `RESTART_BACKOFF_CEILING` (the predicate then
  returns false and the loop self-heals), or
- the job is removed.

Net effect: a deterministic ~5-second hot spin per stop (5 attempts ×
1-second backoff per memory note 38682), with thousands of broker
evaluations during the window. The convergence-loop CPU and broker traffic
are real; the leak is bounded by the ceiling but the symptom is incorrect
behaviour at the §18 *Level-triggered inside the reconciler* contract.

## Reproduction

1. Submit a job whose binary is missing (`/does/not/exist`).
2. The exec driver returns `StartRejected`; the alloc transitions to
   `Failed`. The reconciler enters the restart-with-backoff loop:
   `view.restart_counts[alloc] = 1`, `view.next_attempt_at[alloc] = deadline`.
3. Operator issues `job stop` BEFORE the ceiling is reached.
4. Next tick: `desired_to_stop = true`, no Running allocs, Stop branch
   fires and returns `([], view.clone())`.
5. Runtime: `actions.is_empty() = true`, `backoff_pending = true` (count 1
   < 5, deadline still present). `has_work = true`. Re-enqueue.
6. Repeat every tick for ~5 wall-clock seconds.

## 5 Whys

1. **Why does the convergence loop spin after a stop?**
   The runtime self-re-enqueues every tick because `has_work = true`
   (`reconciler_runtime.rs:340-345`).
2. **Why is `has_work` true?**
   `has_work = actions.iter().any(...) || backoff_pending`
   (`reconciler_runtime.rs:297`). Actions is empty (no Running allocs),
   but `backoff_pending` is true.
3. **Why does `view_has_backoff_pending` return true?**
   `view.next_attempt_at` still contains the failed alloc's entry, and
   `restart_counts[alloc] < CEILING` (`reconciler_runtime.rs:425-428`).
4. **Why is `next_attempt_at` still populated?**
   The Stop branch at `reconciler.rs:1019-1027` returns
   `(stop_actions, view.clone())` unconditionally — never touches
   transitional backoff state.
5. **Why was the Stop branch written that way?**
   The §18 Stop semantics were authored against the converged-Running
   case (alloc is Running → emit StopAllocation → deadline never existed).
   The intersection of a Failed-mid-backoff alloc with a stop intent
   was not encoded; the load-bearing invariant "no pending work once
   stop is complete" is missing from the Stop branch's view contract.

## Root cause

The Stop branch's view-pass-through (`view.clone()`) ignores transitional
backoff state. When the stop is *complete* (no Running allocs to stop),
the view should reflect "stop complete; no pending work" by clearing
`next_attempt_at`. The predicate already encodes the right semantics —
the Stop branch just didn't honour them.

## Proposed fix (minimal)

In `crates/overdrive-core/src/reconciler.rs`, modify the Stop branch:

```rust
if desired.desired_to_stop && desired.job.is_some() {
    let stop_actions: Vec<Action> = actual
        .allocations
        .values()
        .filter(|r| r.state == AllocState::Running)
        .map(|r| Action::StopAllocation { alloc_id: r.alloc_id.clone() })
        .collect();
    let mut next_view = view.clone();
    if stop_actions.is_empty() {
        next_view.next_attempt_at.clear();
    }
    return (stop_actions, next_view);
}
```

Rationale:

- When `stop_actions` is non-empty (Running allocs to stop), behaviour is
  unchanged — actions emitted set `has_work = true` regardless of view
  state.
- When `stop_actions` is empty, clearing `next_attempt_at` signals
  "stop is complete; no pending work" to the predicate, breaking the
  re-enqueue loop.
- `restart_counts` left intact preserves the historical record. The
  predicate (`runtime:425-428`) only checks counts for entries that exist
  in `next_attempt_at`, so clearing only `next_attempt_at` is sufficient
  to break the predicate.
- Pure-function contract preserved (sync, no I/O, deterministic).

## Risk

**Low.** Change is purely additive in the empty-stop-actions branch.
Behaviour for Running allocs is unchanged. No I/O, no async, no
cross-state-layer leakage.

## Files affected

- `crates/overdrive-core/src/reconciler.rs` — Stop branch (production fix).
- `crates/overdrive-core/src/reconciler.rs` (test module) OR
  `crates/overdrive-core/tests/...` — unit test pinning the contract:
  Stop branch with non-empty `next_attempt_at` returns view with empty
  `next_attempt_at`.
- DST harness — convergence-loop test: submit failing job → stop →
  run N ticks → assert broker drains and stays empty (target
  `crates/overdrive-control-plane/tests/integration/...` or wherever the
  convergence-loop DST tests live in this codebase).

## Test scope (user-confirmed)

Both:

1. **Unit test** on `reconcile()` directly — pins the pure-function
   contract; cheap, fast, prevents drift.
2. **DST acceptance test** — pins the user-visible symptom (broker drains
   and stays empty after stop with Failed-mid-backoff alloc).

## Out of scope

- Clearing `restart_counts` in the Stop branch (separate cleanup
  question; no behavioural impact since predicate gates on
  `next_attempt_at`).
- Reviewing the symmetric question of "should backoff for a non-Running
  alloc be cleared when a *different* alloc is being stopped?" — out of
  scope; the Phase-1 reconciler is single-alloc-per-job.
- Refactoring the Stop branch's view-pass-through pattern — out of
  scope per `/nw-bugfix` discipline; refactoring belongs in `/nw-refactor`.
