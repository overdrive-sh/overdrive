# Slice 03 — `alloc status` surfaces exit code for Failed Jobs

**Outcome**: `overdrive alloc status --job <id>` for a Job whose attempts all exited 1
shows `Verdict: Failed (backoff exhausted)`, a per-attempt table with the actual exit
codes, and the stderr tail. This is the user's explicit framing journey.

**Stories**: US-03 (alloc status kind-aware Job render).

**Learning hypothesis**: kind-aware `alloc status` is the operator's definitive "what
really happened?" surface. By rendering per-attempt exit codes for Job kind (and
hiding them for Service kind, where they are not the operator-relevant signal), the
CLI's output matches the operator's mental model for each kind.

## What ships in this slice

- `AllocStatusRow.kind` field is denormalised at write time from the originally-
  submitted spec's kind. (The architect will pin where this is written and how it
  flows through observation rows; the user-facing requirement is that it matches the
  declared kind.)
- CLI render branches in `alloc status`:
  - Service: header (`kind: Service`, `Replicas (desired/running): N/M`), per-alloc
    table with columns `Alloc / State / Restarts / Since`. **No Exit column.**
  - Job: header (`kind: Job`, `Verdict: Succeeded / Failed / Failed (backoff
    exhausted) / In progress`), per-attempt table with columns `Attempt / State /
    Exit / Started / Duration`.
  - Schedule: deferred to Slice 05 (kind: Schedule, Cron, deferral notice).
- New render functions for the Job branch:
  - `format_job_alloc_status_header(name, kind, spec_digest, verdict)`
  - `format_job_alloc_status_attempts_table(rows)`
- For Job-kind Failed allocs whose stderr was captured, the render includes the last
  5 lines.
- For a mid-flight Job (state Running, no terminal yet), the Exit column shows `—`
  (em-dash) and Verdict shows `In progress`.

## End-to-end value

- An operator who runs the bug-affected `coinflip` workload, sees the streaming submit
  return Failed (Slice 02), and runs `alloc status --job coinflip` afterwards now
  sees a coherent post-hoc view that matches what the streaming submit told them:
  three Failed attempts, exit 1 per attempt, stderr `ERROR`.
- An operator who runs a Service sees the existing alloc status shape (preserved by
  Slice 04 vocabulary changes only).

## Acceptance evidence

- `crates/overdrive-cli/tests/integration/alloc_status.rs` (new file) covers:
  - Service kind: replicas + restarts + since; no Exit column.
  - Job kind Succeeded: Verdict Succeeded; one row with Exit 0.
  - Job kind Failed: Verdict Failed (backoff exhausted); three rows with Exit 1; stderr
    tail present.
  - Job kind In progress: Verdict In progress; Exit em-dash.
- Anti-scenario test: assert that for a Job-kind alloc, no line of `alloc status`
  output contains the substring `is running with`.

## Effort estimate (advisory)

~1 day. Render layer changes only; data flows already exist (ExitObserver writes
exit_code to AllocStatusRow today).

## Risks

- `AllocStatusRow.kind` denormalisation: rows written before this slice landed do not
  carry kind. The architect must decide whether to (a) backfill kind from the spec at
  read time, (b) skip migration since Phase 1 has no rows that survive a restart, or
  (c) render `kind: Unknown` for rows without the field. Recommendation: (b) — Phase 1
  is greenfield.
- stderr tail capture is already a Phase 1 concern; this slice surfaces it but does not
  change the capture mechanism.

## DoR fit

User's explicit framing journey: *"in the terms of a job, it is actually correct
behavior. what should happen, is when we check the status using overdrive alloc status
--job <id> then it should show that it failed during execution"*. This slice ships
that exact behaviour.
