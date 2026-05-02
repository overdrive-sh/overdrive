# fix-stop-branch-backoff-pending

**Date**: 2026-05-02
**Type**: Bugfix
**Scope**: `overdrive-core` — `JobLifecycleReconciler::reconcile` Stop branch

## Summary

When an operator stopped a job whose allocation was `Failed` mid-backoff
(restart_counts < `RESTART_BACKOFF_CEILING`), the reconciler's Stop branch
returned `(stop_actions, view.clone())` unconditionally — even when no
Running allocs remained. The runtime's `view_has_backoff_pending`
predicate kept seeing the stale `next_attempt_at` entry, set
`has_work = true`, and re-enqueued the target every tick for ~5 wall-clock
seconds (5 attempts × 1-second backoff per memory note 38682) until the
backoff ceiling exhausted.

## Root Cause

The §18 Stop semantics were authored against the converged-Running case
(alloc is Running → emit `StopAllocation` → deadline never existed). The
intersection of a Failed-mid-backoff alloc with a stop intent was not
encoded; the load-bearing invariant *"no pending work once stop is
complete"* was missing from the Stop branch's view contract.

Chain:

1. `has_work = actions.iter().any(...) || backoff_pending` at
   `reconciler_runtime.rs:297`.
2. `actions` empty (no Running allocs to stop), but `backoff_pending`
   true because `view.next_attempt_at` still holds the failed alloc.
3. The Stop branch at `reconciler.rs:1019-1027` returned
   `(stop_actions, view.clone())` and never touched transitional backoff
   state — so the predicate stayed hot until `restart_counts` hit
   `RESTART_BACKOFF_CEILING` and the predicate self-healed.

## Fix

In the Stop branch, bind `next_view = view.clone()` and clear
`next_attempt_at` when `stop_actions.is_empty()` — signalling
"stop complete; no pending work" to the predicate. `restart_counts` left
intact: the predicate at `reconciler_runtime.rs:425-428` only checks
counts for entries that exist in `next_attempt_at`, so clearing the
deadline map alone is sufficient and preserves the historical record.

Pure-function contract preserved (sync, no I/O, deterministic).
Single-cut migration: no deprecation, no shadow predicate, no
feature-flagged old path.

## Steps Completed

| Step | Description | Commit |
|------|-------------|--------|
| 01-01 | RED — `#[ignore]`-gated regression tests (unit + DST acceptance) | `4b46fe6` |
| 01-02 | GREEN — Stop branch fix; un-ignore both regression tests | `de7f919` |

## Files Changed

- `crates/overdrive-core/src/reconciler.rs` — Stop branch clears
  `next_attempt_at` when `stop_actions.is_empty()`.
- `crates/overdrive-core/tests/acceptance/job_lifecycle_reconcile_branches.rs`
  — new unit regression test pinning the cleared-`next_attempt_at` contract.
- `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs`
  — new DST acceptance regression test pinning the broker-drains symptom.
- `api/openapi.yaml` — incidental regeneration in the GREEN commit.

## Lessons

- **Transitional view state must be cleared when the reconciler
  considers itself converged.** The §18 *Level-triggered inside the
  reconciler* contract requires every termination branch to honour the
  re-enqueue predicate's contract — not just the converged-happy-path
  branches. When a branch returns "no actions," it must also return a
  view that the predicate reads as "no work pending," or the runtime
  will spin forever. Each terminal branch needs its view-pass-through
  audited against the predicate's read set.
- **Same defect class as `fix-noop-self-reenqueue` (2026-04-29).** Both
  bugs are the runtime's `has_work` re-enqueue gate being tripped by a
  signal the reconciler considered transitional. There the dispatcher
  honoured `Action::Noop` semantically but the broker's gate operated
  on syntactic `is_empty()`; here every branch *except* Stop honoured
  the `next_attempt_at` clear contract. The structural pattern is
  *adding a sentinel/transitional state without auditing every site
  that branches on its presence/absence*. A DST invariant of the form
  *"after K ticks against a converged-or-stopped cluster,
  `broker.dispatched` is bounded by the number of distinct
  edge-triggered submits"* would catch this entire class — already
  flagged as a follow-up under `fix-noop-self-reenqueue`.
- **The two-step `#[ignore]`-then-uncomment Outside-In TDD shape works.**
  RED tests landed in commit 1 with `#[ignore]` markers so lefthook
  stayed green; GREEN commit applied the fix and removed the markers in
  the same cohesive commit. This is the project's standard shape for
  runtime-assertion RED scaffolds (distinct from the `panic!("RED
  scaffold")` shape used for compile-fail / exhaustive-match scaffolds
  in `testing.md`).

## References

- RCA: `docs/feature/fix-stop-branch-backoff-pending/deliver/rca.md`
  (preserved in feature workspace; user-validated 2026-05-02)
- Whitepaper §18 *Reconciler and Workflow Primitives* —
  *Triggering Model — Hybrid by Design*, *Level-triggered inside the
  reconciler*
- Precedent: `docs/evolution/2026-04-29-fix-noop-self-reenqueue.md`
  (same defect class — runtime re-enqueue gate vs reconciler
  transitional state)
- Test discipline: `.claude/rules/testing.md` § "RED scaffolds and
  intentionally-failing commits"
