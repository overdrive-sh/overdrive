# ADR-0250: ServiceLifecycle announces a startup failure only for a live allocation

## Status

Accepted, 2026-11-02 (approval: feature delta § Approval record, D5). GH #318,
GH #334. Not implemented. The exact guard, the lifecycle gate (G-3), the formal
protocol model and the DISTILL obligations live in the
[feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md).
Related: [ADR-0248](adr-0248-the-component-that-observes-an-allocations-end-fires-the-terminal-hook.md),
[ADR-0251](adr-0251-the-restart-authority-names-a-pre-startup-crash-loop-at-give-up.md)
(who names a crash that precedes any startup pass), ADR-0087 (single restart
authority), ADR-0055 (service terminal verdicts).

## Context

`ServiceLifecycle` finalises a service allocation as
`ServiceFailed { StartupProbeFailed }` when three gates hold: the producer's
startup-failure streak reached the declared maximum, the startup deadline has
passed, and no startup probe ever passed. The only state check on that branch
excludes `Terminated`, added so a reclaimed allocation is not given a
fabricated verdict. It still acts on `Failed`.

A `Failed` row is the exit observer's record that the guest process crashed.
With ADR-0248 the crashed guest stops being probed, so its streak stops growing
at the crash and keeps the value it had. A `Failed` row of an allocation that
reached `Running`, whose streak had already reached the maximum before the
crash, observed at or after the deadline with no probe ever passing, meets all
three gates. `ServiceLifecycle` then writes `StartupProbeFailed` over the crash
row and runs the genuine-terminal teardown on an allocation `WorkloadLifecycle`,
the single restart authority, is about to restart or give up on. Two reconcilers
then decide the ending of one allocation, and the recorded cause is the probe
streak although the cause was the crash. The three gates make the window narrow:
the streak must be at the maximum and the deadline passed when the crash is first
observed. With ADR-0251 `ServiceLifecycle` has no other branch that acts on a
crashed row, so this guard is the only thing standing between a crash row and a
startup-probe verdict.

## Decision

**`ServiceLifecycle` announces a startup-probe failure only for an allocation
whose row is not terminal. A `Failed` or `Terminated` row has its ending
authored elsewhere, and `ServiceLifecycle` does not finalise it.** The
terminal-state test is the one `WorkloadLifecycle` already uses for a natural
exit, so the two reconcilers cannot disagree about which rows are terminal.
For a `Failed` allocation the restart-or-give-up decision, and the terminal
claim at give-up, belong to `WorkloadLifecycle`
([ADR-0251](adr-0251-the-restart-authority-names-a-pre-startup-crash-loop-at-give-up.md)
decides how a crash that precedes any startup pass is named there).

Scope: this covers the startup-failure branch. No state meaning changes.

## Alternatives considered

- **Allow only `Pending` and `Running`.** Equivalent in production, because a
  production `Pending` row carries no start time and the branch already skips
  it. It re-lists live states by hand where a shared terminal predicate exists,
  so a new state would silently be excluded or included by drift.
- **Act on `Failed` unless the row records a crash cause.** A heuristic that
  classifies the cause from the row's reason text to repair a branch that should
  not have run. It is a fallback around a fact the state already carries.
- **Let the action shim refuse the finalisation on a terminal row.** Moves the
  ownership check to the executor; the reconciler would still emit and record
  the verdict, and the executor would decide what the reconciler owns.
- **Keep the behaviour and rely on `WorkloadLifecycle` winning the race.** The
  two reconcilers act through separate evaluations, so no ordering is
  guaranteed, and the crash cause is overwritten when `ServiceLifecycle` writes
  first.

## Consequences

- Positive: one owner per ending. A crashed service allocation ends with the
  crash as its cause, and the restart budget and give-up claim are
  `WorkloadLifecycle`'s alone.
- User-visible: a service allocation that crashes after its startup-probe
  attempts were exhausted and after the deadline passed is restarted under the
  restart budget; at exhaustion the operator sees the crash-loop terminal
  (ADR-0251) instead of a startup-probe failure. An allocation that stays
  `Running` with exhausted probes past the deadline is acted on by this branch;
  what the branch emits for it (today a failure finalisation) is the subject of
  [ADR-0252](adr-0252-a-startup-probe-failure-stops-the-allocation-and-the-restart-authority-decides.md),
  which this guard does not depend on.
- Neutral: a crashed allocation no longer enters the reconciler's
  terminal-announced set through this branch. It behaves as a crash after the
  deadline with a streak below the maximum already does today.
- Negative: one existing sim test asserts the removed behaviour on a `Failed`
  fact and must be rewritten in DISTILL (feature delta, S-13).
