# fix-restart-backoff-deadline-not-written — Feature Evolution

**Feature ID**: fix-restart-backoff-deadline-not-written
**Type**: Bug fix (`/nw-bugfix` → `/nw-deliver`)
**Branch**: `marcus-sa/phase1-first-workload`
**Date**: 2026-04-28
**Commits**:
- `b2af74d` — `test(reconciler): RED scaffold for next_attempt_at write-side regression`
- `ea0817c` — `fix(reconciler): materialise next_attempt_at deadline in RestartAllocation branch`
**Status**: Delivered.

---

## Symptom

`JobLifecycle::reconcile` read `view.next_attempt_at` to gate
`RestartAllocation` emission behind a time-based backoff deadline
(`crates/overdrive-core/src/reconciler.rs:1145-1149`), but the field
was never populated. The emission branch only incremented
`next_view.restart_counts`; no deadline was ever inserted into
`next_view.next_attempt_at`. Consequence: on every 100 ms tick where a
Terminated alloc existed, the reconciler re-emitted `RestartAllocation`
immediately. A workload that failed on each attempt exhausted
`RESTART_BACKOFF_CEILING = 5` within ~500 ms, marking the job
permanently `Failed`. The spec'd `initial_backoff` window
(`docs/feature/phase-1-first-workload/discuss/user-stories.md:421-424`,
*Domain Example 2*) never fired; the read-side gate was dead code.

## Root cause

The `JobLifecycleView` field shape was specified in US-03 AC and the
read-side gate was wired correctly, but the write-side (deadline
materialisation in `next_view`) was never implemented. The field was
inert from the first commit. CI did not catch it because the existing
acceptance test
(`repeatedly_crashing_workload_exhausts_backoff_and_stops_retrying`)
asserts only the count ceiling and never exercises a tick-time
inequality against a populated `next_attempt_at`. Spec scenario 3.8 in
`test-scenarios.md:425-438` also omits an explicit timing-gate
assertion.

## Fix

Three production edits in `crates/overdrive-core/src/reconciler.rs`,
landed in a single cohesive commit (`ea0817c`):

1. **Constant** — `pub const RESTART_BACKOFF_DURATION: Duration =
   Duration::from_secs(1)` adjacent to `RESTART_BACKOFF_CEILING`. The
   constant matches `user-stories.md:424`'s SSOT (`initial_backoff` —
   singular, no progression). Promotion to exponential backoff is
   industry intuition (kubelet `CrashLoopBackOff` style) but is not
   what the spec prescribes; that would be a separate spec-amendment
   PR. 1 s × ceiling 5 ≈ 5 s wall-clock to "Failed (backoff
   exhausted)" — observable in metrics, not operator-frustrating.
2. **Write** — `next_view.next_attempt_at.insert(failed.alloc_id.clone(),
   tick.now + RESTART_BACKOFF_DURATION)` inserted in the
   `RestartAllocation` emission branch, immediately after the
   restart-count bump. Uses `tick.now` exclusively per
   `.claude/rules/development.md` § *Reconciler I/O*; never
   `Instant::now()`. dst-lint clean.
3. **Docstring** — `JobLifecycleView::next_attempt_at` rustdoc updated
   to cite `RESTART_BACKOFF_DURATION` by name (was prose
   `backoff_duration`).

Scope discipline notes:

- **Constant chosen over exponential per SSOT.** Reviewer's "exponential"
  framing was acknowledged but explicitly out of scope per RCA §*Spec
  source for the timing semantics*; user APPROVED the constant approach
  on 2026-04-28.
- **Single-cut migration** per `feedback_single_cut_greenfield_migrations`:
  constant + write + docstring all in one commit, no shadow constant,
  no deprecation, no two-phase rollout.
- **Reconciler purity preserved.** The write is a pure derived value
  (`tick.now + RESTART_BACKOFF_DURATION`) on `NextView`; no async, no
  I/O, no banned APIs.

## Tests

Three new `#[test]` functions appended to the existing acceptance file
`crates/overdrive-core/tests/acceptance/job_lifecycle_reconcile_branches.rs`,
default lane (pure unit-shaped over `JobLifecycle::reconcile` typed
inputs — no I/O, no `SimDriver`, no feature gate):

1. `restart_allocation_writes_next_attempt_at_deadline` — single-tick;
   asserts `next_view.next_attempt_at[<alloc_id>] == tick.now +
   RESTART_BACKOFF_DURATION` on first failure. Kills the "wrote
   nothing" / "wrote `Duration::ZERO`" mutation classes.
2. **`subsequent_tick_within_backoff_window_emits_nothing`** — two-tick
   chain, +500 ms; tick 1's `next_view` becomes tick 2's `view`;
   asserts the gate fires (`actions_2.is_empty()`, `restart_counts`
   unchanged, deadline preserved). **This is the regression-evidence
   artefact** — applied to current main, tick 2 re-emits
   `RestartAllocation` because tick 1 never wrote a deadline. The
   chain-link shape is what makes the user-observable bug surface in
   a test.
3. `tick_after_backoff_elapsed_emits_restart_and_advances_deadline` —
   two-tick chain, +`RESTART_BACKOFF_DURATION + 1 ms`; asserts another
   restart fires, count++, and the deadline rolls forward to
   `new_tick.now + RESTART_BACKOFF_DURATION` (not previous deadline +
   window). Kills the "rolled deadline from previous deadline rather
   than from new tick.now" mutation class.

The RED commit (`b2af74d`) referenced
`overdrive_core::reconciler::RESTART_BACKOFF_DURATION` directly; the
unresolved-import compile failure was the RED state, per
`.claude/rules/testing.md` § *RED scaffolds and intentionally-failing
commits*. No shadow constant; committed with `--no-verify`. The GREEN
commit closed the import and the runtime assertions in one step.

## Verification

All gates from the execution log:

- `cargo nextest run --workspace` — 507 passed.
- `cargo test --doc -p overdrive-core` — passed.
- `cargo clippy -p overdrive-core --all-targets -- -D warnings` — clean.
- `cargo xtask dst-lint` — clean (no `Instant::now()` / `SystemTime::now()`
  in `reconcile`).
- `cargo xtask dst` — 14 invariants passed.
- `cargo nextest run --workspace --features integration-tests --no-run`
  — typechecks on macOS.
- Mutation gate — `cargo xtask mutants --diff origin/main --package
  overdrive-core --file crates/overdrive-core/src/reconciler.rs`
  reported **94.1% kill rate**, comfortably above the ≥80% gate. The
  load-bearing assertions in the three new tests killed the expected
  mutation classes (constant value, deletion of the `insert(...)`
  line, deadline-rolling arithmetic).
- Reviewer (nw-software-crafter-reviewer) — **APPROVE** on the GREEN
  commit.

Per RCA §*Risk*, the named candidate
`repeatedly_crashing_workload_exhausts_backoff_and_stops_retrying`
passed unchanged: its `<= 6` ceiling assertion is satisfied with
equal-or-fewer `Driver::start` calls under the new gate, so no fixture
cadence adjustment was needed.

`git grep 'backoff_duration' crates/overdrive-core/src/reconciler.rs`
returned no matches outside the rustdoc reference to the named
constant — single-cut migration confirmed.

## Follow-ups (non-blocking, out of scope for this fix)

1. **Spec-layer scenario amendment**. Add an explicit timing-gate
   scenario to
   `docs/feature/phase-1-first-workload/distill/test-scenarios.md`
   complementing
   `repeatedly_crashing_workload_exhausts_backoff_and_stops_retrying`,
   which only asserts the count ceiling. The implementation-side
   chain-link test
   (`subsequent_tick_within_backoff_window_emits_nothing`) now pins
   the invariant; the spec scenario would close the belt-and-braces
   loop at the SSOT layer. Separate PR if/when wanted.
2. **Unrelated missed mutation**. The mutation pass surfaced one
   surviving mutation in `node_free_capacity` at
   `crates/overdrive-core/src/reconciler.rs:1240`. Pre-existing,
   unrelated to the `next_attempt_at` write, and out of scope for this
   bug fix. File a follow-up if the missed mutation indicates a real
   gap rather than test-asserting-on-a-different-thing.

## Lineage

This is the third of a sequence of small fixes refining the Phase 1
single-node single-workload envelope. Prior fixes in the same family:

- `2026-04-26-fix-xtask-mutants-zero-mutant-crash` — wrapper-side
  empty-filter handling for the per-step mutation gate, which is the
  gate that reported the 94.1% kill rate above.
- The Phase 1 first-workload feature itself
  (`2026-04-28-phase-1-first-workload`) — landed concurrently; this
  fix closes a defect spotted in code review of that feature's
  reconciler.
