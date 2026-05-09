# Slice 05 — `[schedule]` parses and validates composition rules (execution deferred)

**Outcome**: an operator can write a `[job] + [schedule]` spec that parses, gets an
honest "Schedule registered, execution not yet implemented" submit echo with a
tracking issue URL, and sees the same deferral reflected consistently in `alloc
status`. Cron *execution semantics* (firing on tick, ConcurrencyPolicy, history
retention) are deferred to a follow-up feature.

**Stories**: US-05 (Schedule parsing + honest deferral).

**Learning hypothesis**: shipping the syntactic surface for `[schedule]` without the
execution semantics lets operators *plan* recurring jobs without the platform lying
about what it can actually do. The deferral language is honest enough to maintain
operator trust per J-OPS-002's "no silent blank outputs" clause.

## What ships in this slice

- Parser support for the `[schedule]` block alongside `[job]`:
  - `cron` field (string) is required.
  - Other Schedule-specific fields (concurrency, history limits, starting deadline)
    are NOT yet supported — adding them is part of the deferred execution feature.
  - Validation rules: `[schedule]` only with `[job]` (never `[service]`); `cron` is
    required when `[schedule]` is present.
- New `WorkloadSpec::Schedule { job_inner, cron_expr }` variant (architect to confirm
  shape).
- A new `examples/nightly-backup.toml` shipped with the slice exercising the shape.
- CLI submit echo for Schedule kind:
  ```
  Submitting schedule '<id>' (kind=Schedule)
  Spec digest: sha256:...
  Endpoint: ...
  Schedule registered.

  NOTE: schedule execution is not yet implemented in this Phase 1 slice.
        The spec has been validated and persisted as intent; no Job runs
        will be spawned automatically.
        Tracking: https://github.com/overdrive-sh/overdrive/issues/166
  ```
- CLI alloc status render for Schedule kind:
  ```
  Job: <id>    (kind: Schedule)
  Spec: sha256:...
  Cron: <cron-expr>

  No allocations have been spawned yet.

  Reason: Schedule execution is not yet implemented (issue #166).
  ```
- A single CLI config constant (e.g. `SCHEDULE_EXECUTION_TRACKING_URL`) is the SSOT
  for the deferral URL. Both the submit echo and alloc status read from this constant
  — they cannot drift.
- The submitted Schedule spec IS persisted as intent (per J-OPS-002 — submitted things
  are committed, even if execution is deferred).

## End-to-end value

- An operator can write a Scheduled Job spec today, get it validated, and have the
  platform honestly tell them when execution support arrives. They can build their
  Schedule manifests now and commit them to source control.

## Acceptance evidence

- `crates/overdrive-cli/tests/integration/job_submit_schedule.rs` (new file) covers:
  - Valid Schedule spec parses and produces the documented submit echo.
  - The deferral URL in the echo equals the URL in the alloc status output (byte-
    identical assertion).
  - Invalid combinations (`[schedule]` without `[job]`, `[schedule]` with `[service]`,
    missing `cron` field, malformed cron expression at the *string-only* level) are
    rejected with named guidance.

## Effort estimate (advisory)

~1 day. Parser additions are straightforward; the deferral render is small.

## Deferral note

**This slice's CLI output references a tracked deferral.** The follow-up feature is
[overdrive-sh/overdrive#166](https://github.com/overdrive-sh/overdrive/issues/166)
("Scheduled job execution semantics") — user approved on 2026-05-09. The CLI constant
`SCHEDULE_EXECUTION_TRACKING_URL` MUST equal
`https://github.com/overdrive-sh/overdrive/issues/166` byte-for-byte; both the submit
echo and the `alloc status` deferral notice read from it (KPI K5 asserts they match).

## Risks

- Cron-expression-string validation: how strict is the string check at parse time?
  Recommendation: accept any non-empty string here; defer real cron syntax validation
  to the execution feature. Captures the operator's intent without partial-correctness
  surface area.
- `examples/nightly-backup.toml` ships an example whose `cron` value will not actually
  fire — this is acceptable because the submit echo is explicit about it.

## DoR fit

Lowest-priority slice. Could be deferred to its own feature without harming Slices
01–04. Kept in this feature because the parser work lives next to the parser work for
`[service]` / `[job]` and is small.
