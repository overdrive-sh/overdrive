# Slice 02 — Job submit terminates on Succeeded/Failed (closes the bug)

**Outcome**: `overdrive job submit examples/coinflip.toml` returns
`Job 'coinflip' succeeded.` (CLI exit 0) on a SUCCESS run and `Job 'coinflip' failed.`
(CLI exit non-zero, exit code 1, attempt count) on an ERROR run. The string
`is running with 1/1 replicas (took live)` is not produced for any Job-kind submit.

**Stories**: US-02 (Job submit terminal verdict).

**Learning hypothesis**: separating `JobSubmitEvent` from `ServiceSubmitEvent` (research
R2) makes the bug structurally unreachable for Jobs. The streaming protocol's
"converged" semantics become kind-aware: for a Job, "converged" means "reached terminal
exit"; for a Service, "converged" means "reached and remained Running" (existing
semantics, preserved in Slice 04).

## What ships in this slice

- A `JobSubmitEvent` enum with variants:
  - `Accepted { commit_index, spec_digest, .. }` (existing shape).
  - `Pending` (existing shape).
  - `Running { since, .. }` (informational — emitted but NOT terminal).
  - `AttemptFailed { attempt_index, exit_code, duration, will_restart, next_attempt_delay }`
    (intermediate; stream stays open).
  - `Succeeded { exit_code: 0, duration, attempts }` (terminal).
  - `Failed { exit_code, duration, attempts, max_attempts, stderr_tail }` (terminal).
- The streaming subscriber for a Job-kind alloc waits for the ExitObserver's terminal
  observation row before emitting `Succeeded` / `Failed`. The current "first Running
  row equals converged" semantics is ONLY used on the Service-kind code path.
- New CLI render functions:
  - `format_job_succeeded_summary(name, exit_code, duration, attempts)`
  - `format_job_failed_summary(name, exit_code, duration, attempts, max_attempts, stderr_tail)`
  - `format_job_attempt_failed(name, attempt, exit_code, duration, retry_in)`
- The CLI's submit command handler exits with the workload's terminal exit code (0 on
  Succeeded; non-zero on Failed).
- Existing `format_running_summary` is retained but its only call site is on the
  Service code path (Slice 04 will rename the function and adjust vocabulary).

## End-to-end value

- The bug under audit is gone. The empirical reproduction
  (`overdrive job submit examples/coinflip.toml` against an exit-1 workload) now
  returns a Failed verdict with exit code 1.
- The honesty KPI moves from 0% to ≥99% over 100 trials of the coinflip workload.

## Acceptance evidence

- `crates/overdrive-cli/tests/integration/job_submit.rs` gains scenarios for Succeeded
  and Failed terminal outcomes.
- The streaming layer's tests cover the new event types and assert that the
  `ConvergedRunning` variant is NOT producible for a Job-kind submit.
- Anti-scenario test: assert that no line of CLI output for a Job submit contains the
  substring `is running with` or `(took live)`.

## Effort estimate (advisory)

~1.5 days. The largest slice — touches streaming protocol, CLI command handler, render
layer. Could be split into "streaming protocol per-kind events" + "CLI render
adoption" if the PR ends up larger than reviewable, but kept whole here because the
structural fix is one move.

## Risks

- Backward-compat with the wire format. Phase 1 ships single binary, so wire compat is
  internal — the architect can rev `SubmitEvent` freely. Captured in
  `wave-decisions.md` as risk R2.
- Stderr tail truncation: spec defaults to last 5 lines; architect may want to make
  this configurable.

## DoR fit

This is the slice that delivers the operator-visible behaviour change. The bug under
audit is its single load-bearing acceptance test.
