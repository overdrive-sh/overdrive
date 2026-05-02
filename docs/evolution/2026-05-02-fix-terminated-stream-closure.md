# fix-terminated-stream-closure

**Date**: 2026-05-02
**Type**: Bugfix
**Scope**: `overdrive-control-plane` — streaming submit handler

## Summary

`check_terminal` and `lagged_recover` in `streaming.rs` only handled
`AllocStateWire::Running` and `AllocStateWire::Failed`, falling through
to `None` for `AllocStateWire::Terminated`. When an operator stopped a
job during an active streaming submit, the stream waited for the 60-second
cap timer instead of closing immediately — reporting the intentional stop
as a timeout failure (exit 1).

## Root Cause

The streaming terminal specification (module docstring, ADR-0032 reference)
listed only two convergence paths when written. The stop endpoint
(`Action::StopAllocation`) landed as a separate slice and the streaming
handler was never cross-referenced. `TerminalReason` had no `Stopped`
variant; `check_terminal` had no `Terminated` arm; `lagged_recover` had
no `Terminated` arm.

## Fix

Added `ConvergedStopped { alloc_id, by }` as a new top-level
`SubmitEvent` variant (exit 0, distinct from `ConvergedFailed`).
`check_terminal` and `lagged_recover` now detect
`AllocStateWire::Terminated` / `AllocState::Terminated` and return
`ConvergedStopped`, closing the stream immediately.

## Steps Completed

| Step | Description | Commit |
|------|-------------|--------|
| 01-01 | RED regression test `s_cp_11` | `69b318d` |
| 01-02 | GREEN — add `ConvergedStopped`, handle `Terminated` | `e3531ff` |

## Files Changed

- `crates/overdrive-control-plane/src/api.rs` — `ConvergedStopped` variant + OpenAPI registration
- `crates/overdrive-control-plane/src/streaming.rs` — `Terminated` arms in `check_terminal` + `lagged_recover`, module docstring, unit test
- `crates/overdrive-control-plane/tests/acceptance/streaming_submit.rs` — `s_cp_11` acceptance test

## Lessons

- Terminal-state coverage should be checked against the full `AllocStateWire` enum whenever a new lifecycle path lands, not just the paths the original author had in scope.
- The `#[non_exhaustive]` attribute on `TerminalReason` was correctly forward-proofed but no tracking mechanism captured `Stopped` as a known gap.
