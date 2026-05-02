# fix-clean-exit-stopped-by

**Date**: 2026-05-02
**Type**: Bugfix
**Branch**: marcus-sa/phase1-first-workload

## Summary

`ExitKind::CleanExit` with `intentional_stop=false` was incorrectly attributed to `StoppedBy::Reconciler` in the exit observer's `classify()` function. A process that exits naturally (clean exit, not externally stopped) is semantically distinct from a reconciler-initiated stop. The fix adds `StoppedBy::Process` as a new enum variant and routes `CleanExit` to it.

## Root Cause

`StoppedBy` had only two variants (`Operator`, `Reconciler`), and `classify()` fell through to `StoppedBy::Reconciler` as an implicit catch-all for `ExitKind::CleanExit` when `intentional_stop=false`. There was no variant representing natural process completion.

## Business Context

Incorrect `StoppedBy` attribution corrupts the audit trail for allocation lifecycle events: operators and the LLM observability agent cannot distinguish "the reconciler requested a stop" from "the process exited on its own". This matters for incident analysis and right-sizing decisions.

## Changes Per File

| File | Change |
|---|---|
| `crates/overdrive-core/src/transition_reason.rs` | Added `StoppedBy::Process` as last variant (discriminant 2) with all required derives; updated `human_readable()`; fixed `Reconciler` docstring; added `stopped_by_process_human_readable` unit test; added `is_failure` branch coverage tests |
| `crates/overdrive-control-plane/src/worker/exit_observer.rs` | `classify()` `CleanExit` arm: `StoppedBy::Reconciler` → `StoppedBy::Process`; unit test renamed and assertion updated |
| `crates/overdrive-control-plane/tests/acceptance/alloc_status_row_archive_roundtrip.rs` | `Just(StoppedBy::Process)` added to `arb_transition_reason()` proptest generator |
| `crates/overdrive-control-plane/tests/acceptance/submit_event_serialization.rs` | Same proptest update |
| `crates/overdrive-control-plane/tests/acceptance/alloc_status_snapshot.rs` | Same proptest update |

## Key Decisions

**`StoppedBy::Process` appended last, not inserted.** rkyv serialization uses integer discriminants; `Operator=0`, `Reconciler=1` are already in archived bytes in the store. Inserting before `Reconciler` would shift its discriminant and silently corrupt existing archived rows. Appending as discriminant 2 is the only safe choice for an additive schema change.

**`is_failure()` returns `false` for all `Stopped` variants.** A process stopping cleanly — whether by operator, reconciler, or its own exit — is not a failure. Only cause-class variants (`DriverInternalError`, `RestartBudgetExhausted`) return `true`. This was verified by adding explicit unit tests after mutation testing found the method had no coverage.

## Steps Completed

| Step | Name | Commits |
|---|---|---|
| 01-01 | Add StoppedBy::Process variant to overdrive-core | `a10c67f` |
| 01-02 | Fix classify() and update all StoppedBy generators | `44f3fec` |
| bonus | Add is_failure() unit test coverage (mutation gap) | `7a1f40e` |

All phases: PREPARE → RED_ACCEPTANCE → RED_UNIT → GREEN → COMMIT/PASS.

## Lessons Learned

**Mutation testing surfaces coverage gaps in adjacent code.** `is_failure()` was not part of the original fix scope, but it lived in the same file. The mutation run flagged two missed mutations on it, prompting two tests that now protect the method's branches.

**Proptest generators must be kept in sync with enum variants.** Three acceptance test files each had an `arb_transition_reason()` generator enumerating `StoppedBy` variants. Adding a variant without updating all three would silently under-test the new code path across archive roundtrip, serialization, and snapshot scenarios.

## Issues Encountered

None blocking. The `crash_recovery` integration test timed out during an early mutation baseline run (pre-existing flaky test, unrelated to this change).
