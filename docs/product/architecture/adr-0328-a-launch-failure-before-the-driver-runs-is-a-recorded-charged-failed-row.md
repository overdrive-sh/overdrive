# ADR-0328 — A launch failure before the driver runs is a recorded, charged `Failed` row

## Status

**Accepted** 2026-11-18 (decision SD-6, clause B, of the node-local persistent volume feature, GH #348; user ruling of 2026-11-18, option B).
User rulings of 2026-11-19: **J-1** (option A: a Job's failure before the driver is final, a Job is never relaunched) and **DE-1** (a non-start
`DriverError` from `driver.start` is not part of this decision). This ADR is not implemented and is stated in the present tense. It depends on
[ADR-0326](adr-0326-a-restart-generation-is-handled-when-its-successor-is-observed.md) (a generation is pending until its successor has a row)
and changes behaviour that exists on `main`.

## Context

The action shim has failure paths that return an error before the driver runs and write no allocation row: an SVID issuance failure
(`ensure_intercept_identity`) and an empty driver registry in the restart arm. The runtime requeues `Immediate` after any tick that emitted an
action, whatever the dispatch returned. With the generation derived from observation (ADR-0326) a pending generation skips the backoff gate and
carries the predecessor's attempt count, so each tick mints a fresh allocation id, grows the View, persists it and retries the failing step (the
SVID mint calls the CA): unpaced, unbounded and invisible to the operator. The same loop exists on `main` for a first placement (the placement
arm re-issues every tick while no allocation exists) and for a crash restart (the backoff gate reads the unchanged predecessor).

## Decision

**A failure that precedes the driver writes a `Failed` allocation row with a typed reason, releases what the dispatch acquired, and is charged
and backed off exactly as a start rejection is. A Job's row is final.**

1. An SVID issuance failure writes `MtlsInterceptInstallFailed { stage: "identity_issue", detail }`. An empty driver registry in a restart
   follows the missing-driver path (a synthesised unclassified start failure, `DriverInternalError`). No `TransitionReason` variant is added.
2. The dispatch releases what it acquired before the row is written (the guest network lease, retired then torn down). A teardown failure does
   not suppress the row and is returned after it, exactly as for a start rejection.
3. **A Service** is charged and backed off like any start failure: a persistent failure ends in `BackoffExhausted`, a transient one recovers on
   the next successful launch.
4. **A Job** is not relaunched (J-1). The reconciler finalises any ended Job row and places a Job only while it has no row, so a Job whose
   first launch fails before the driver ends `Failed { exit_code: Some(0) }` (the default arm of the natural-exit classification; code
   citations in the feature delta, S5-J and RA11). That is the outcome of a Job whose start is rejected today, whose `exit_code: Some(0)` is
   equally misleading; this decision does not change it.
5. A non-start `DriverError` from `driver.start` is not decided here (DE-1). `VmDriver::start` returns only `StartRejected` and
   `GuestTransportMissing`; the second is a composition error placement never issues, so only a `SimDriver` script produces another error.
   The shim's `Err(other)` arm stays an internal typed error with no row.
6. This also changes a first placement and a crash restart on `main`, which return the error and write no row today.

Reasons, outcome table with code citations: feature delta § "Launch outcomes".

## Alternatives considered

- **Leave pre-driver failures as dispatch errors and rely on the pending-generation rule.** Rejected: unpaced, unbounded View growth and a
  retried CA call every tick, with nothing visible to the operator.
- **Pace the pending path with a View counter of unobserved ids.** Rejected: the failure stays invisible to `overdrive workload describe` (a
  convergent record cannot answer "did it happen", `.claude/rules/development.md`), and it is a second budget beside the restart budget that
  the single restart authority (ADR-0087) owns.
- **Charge every issued id that never gets a row.** Rejected: a supersession is indistinguishable from a fault by the View alone, so an
  operator's stop or a shutdown would spend the restart budget (ADR-0326).
- **Relaunch a Job whose first launch failed before the driver.** Rejected (J-1): the Job contract is run once (ADR-0037, ADR-0047) and the
  reconciler's finalise-any-ended-row rule already ends a Job whose start is rejected; a separate rule for a pre-driver failure would be a
  second Job restart policy.
- **Write a `Failed` row for any non-start `DriverError`.** Rejected (DE-1): no driver this feature changes produces one.

## Consequences

- Positive: an identity failure is visible in `overdrive workload describe`, backs off, and ends a Service in `BackoffExhausted` instead of
  retrying every tick.
- Negative: ordinary first placements and crash restarts on `main` that hit these failures now produce `Failed` rows with new reason text.
- Negative: a Job whose SVID issuance fails once is `Failed` for good, as a Job whose start is rejected is today.
- Negative: the `stage` vocabulary of `MtlsInterceptInstallFailed` gains `"identity_issue"` (a documentation change; no type, envelope or
  OpenAPI schema change).

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Launch outcomes
- Formal model: `specs/quint/restart-generation/` (action `LaunchPreFail`; hazard `prefailNoRow`, property `PreFailExhausts`)
- [ADR-0326](adr-0326-a-restart-generation-is-handled-when-its-successor-is-observed.md),
  [ADR-0087](adr-0087-single-restart-authority-workload-lifecycle-owns-crash-and-liveness.md), `.claude/rules/reconcilers.md`
