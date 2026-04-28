# RCA — `JobLifecycle::reconcile` never writes `next_attempt_at`

**Feature ID**: `fix-restart-backoff-deadline-not-written`
**Reported via**: code review on `crates/overdrive-core/src/reconciler.rs:1151-1157`
**RCA produced**: 2026-04-28
**User review**: APPROVED 2026-04-28 (Phase 2 of `/nw-bugfix`).

## Symptom

`JobLifecycle::reconcile` reads `view.next_attempt_at` to gate `RestartAllocation` emission behind a time-based backoff deadline (`reconciler.rs:1145-1149`), but the field is never populated. After emitting `RestartAllocation` the branch only increments `next_view.restart_counts`; no deadline is inserted into `next_view.next_attempt_at`.

Practical consequence: on every 100 ms tick where a Terminated alloc exists, the reconciler emits another `RestartAllocation` immediately. A workload that fails on each attempt exhausts `RESTART_BACKOFF_CEILING = 5` within ~500 ms, marking the job permanently Failed. The backoff *delay* (the spec's `initial_backoff` window) never fires; the gate is dead code.

The acceptance test `repeatedly_crashing_workload_exhausts_backoff_and_stops_retrying` covers the count ceiling but never advances `tick.now` against a populated `next_attempt_at`, so the gap passes CI silently.

## 5 Whys

- **WHY 1** — `RestartAllocation` is emitted on every tick a Terminated alloc exists. *Evidence: `reconciler.rs:1145-1149` — `if let Some(deadline) = view.next_attempt_at.get(...)` always falls through.*
- **WHY 2** — The `BTreeMap` entry never exists for any alloc id. *Evidence: `reconciler.rs:1151-1156` — emission branch writes `next_view.restart_counts` only; no `next_attempt_at.insert(...)`.*
- **WHY 3** — The implementation realised the count ceiling (`RESTART_BACKOFF_CEILING = 5`, line 1035) but not the timing-gate companion. No `RESTART_BACKOFF_DURATION` constant exists.
- **WHY 4** — CI did not catch this. The acceptance test scenario asserts ceiling-reached behaviour after N ticks at constant `tick.now`; it never asserts that `tick.now < deadline → empty actions`. Spec scenario 3.8 (test-scenarios.md:425-438) itself omits the timing assertion.
- **WHY 5 — ROOT CAUSE** — The `JobLifecycleView` field shape was specified in US-03 AC (user-stories.md:495 + slice-3-lifecycle-reconciler.md:30) and the read-side gate was wired, but the write-side (deadline materialisation in `next_view`) was never implemented. The field was inert from the first commit.

## Spec source for the timing semantics

`docs/feature/phase-1-first-workload/discuss/user-stories.md:421-424`, *Domain Example 2*:

> Reconciler reads `view.restart_counts[old_alloc] = 0`, emits a fresh `StartAllocation` with a new `alloc_id`. View's NextView increments `restart_counts[new_alloc] = 0` (per-alloc counter; reset by alloc_id). Backoff `next_attempt_at` is set from `tick.now + initial_backoff`.

`initial_backoff` — singular, no progression. The fix uses a single constant; the reviewer's "exponential" framing is industry intuition (kubelet CrashLoopBackOff style) but is not what the SSOT specifies. Promotion to exponential is a separate spec amendment, not part of this bug fix.

## Approved fix (single-cut PR)

1. **Add constant** `RESTART_BACKOFF_DURATION: Duration = Duration::from_secs(1)` in `crates/overdrive-core/src/reconciler.rs`, alongside `RESTART_BACKOFF_CEILING`. One-second window covers transient hiccups (slow startup, dependency flap) within Phase 1's single-node envelope; constant 1 s × ceiling 5 = ~5 s wall-clock to "Failed (backoff exhausted)" — observable in metrics but not operator-frustrating.

2. **Materialise the deadline** in the `RestartAllocation` emission branch (`reconciler.rs:1151-1156`). After cloning `view` into `next_view` and incrementing the restart count, insert:
   ```rust
   next_view
       .next_attempt_at
       .insert(failed.alloc_id.clone(), tick.now + RESTART_BACKOFF_DURATION);
   ```

3. **Fix the docstring drift**. `JobLifecycleView::next_attempt_at` (line 1262-1263) currently says "computed from `tick.now + backoff_duration`" — replace `backoff_duration` with the named constant `RESTART_BACKOFF_DURATION` so the prose and the implementation cite the same identifier.

4. **Regression test** (load-bearing artifact per `nw-bugfix`):
   - Pure unit-shaped test, default lane (no I/O, no SimDriver — `reconcile` is sync over typed inputs). Lives in `crates/overdrive-core/tests/` or as a `#[test]` in `reconciler.rs`'s existing test module.
   - **Arm A — gate fires**: construct a `JobLifecycleView` with `next_attempt_at[alloc_id] = tick.now + Duration::from_millis(500)`; construct `actual` with that alloc Terminated; assert `reconcile` returns `(actions, next_view)` where `actions.is_empty()` AND `next_view.restart_counts == view.restart_counts` (no count increment when gated).
   - **Arm B — gate elapsed → restart fires**: same setup, advance `tick.now` past the deadline; assert exactly one `RestartAllocation`, restart count incremented by 1, AND `next_view.next_attempt_at[alloc_id] == tick.now + RESTART_BACKOFF_DURATION` (the new deadline is materialised).
   - **Arm C — fresh entry on first failure**: alloc Terminated, `view.next_attempt_at` empty for that id; assert `RestartAllocation` emitted once, `next_view.next_attempt_at[alloc_id]` populated.

   The Arm B "deadline written" assertion is the specific gap that turned this bug into dead code.

## Files affected

| File | Change |
|---|---|
| `crates/overdrive-core/src/reconciler.rs` | Add `RESTART_BACKOFF_DURATION` constant; insert `next_attempt_at` write in `RestartAllocation` branch; fix `JobLifecycleView` field docstring. |
| `crates/overdrive-core/src/reconciler.rs` (or `crates/overdrive-core/tests/...`) | Regression test (Arms A + B + C above). |

No spec-document edits in this PR. The SSOT (`user-stories.md`, `slice-3-lifecycle-reconciler.md`, `test-scenarios.md`) already prescribes the correct behaviour; the implementation just didn't realise the write side.

## Risk

- **Scope**: additive write + new constant + one docstring fix + new test. No existing behaviour changes for callers that don't populate `next_attempt_at` (the `BTreeMap` was empty before; now it carries deadlines).
- **DST exposure**: any existing DST scenario that submits a failing alloc and expects rapid restarts within a single second will now see the gate fire. Run `cargo xtask dst` as part of the fix; investigate any scenario that implicitly depended on zero-delay restarts (those were exercising the bug, not the spec).
- **Acceptance test impact**: `repeatedly_crashing_workload_exhausts_backoff_and_stops_retrying` may need its tick advancement to step past `RESTART_BACKOFF_DURATION` between attempts to keep asserting the ceiling behaviour at the same wall-clock duration. Adjust the test fixture's tick cadence as part of the fix; do not weaken the assertion.
- **Mutation testing**: the new constant + write are high-value mutation targets. The Arm B assertion (`next_view.next_attempt_at[id] == tick.now + RESTART_BACKOFF_DURATION`) is what kills the "wrote `Duration::ZERO`" mutation; the Arm A assertion kills the "skipped the gate" mutation. Verify via `cargo xtask mutants --diff origin/main --package overdrive-core --file crates/overdrive-core/src/reconciler.rs`.
- **Reconciler purity**: the fix uses `tick.now` exclusively, never `Instant::now()`. dst-lint clean.

## User review record

| Date | Reviewer | Verdict | Notes |
|---|---|---|---|
| 2026-04-28 | user (Marcus) | **APPROVED** | Confirmed root cause; approved approach (A) constant `RESTART_BACKOFF_DURATION` matching `user-stories.md:424` SSOT. Exponential is a separate spec-amendment PR if/when wanted. Scope: bug fix + regression test only. |
