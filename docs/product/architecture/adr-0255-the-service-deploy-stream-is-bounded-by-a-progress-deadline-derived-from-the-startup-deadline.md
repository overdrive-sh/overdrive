# ADR-0255: The Service deploy stream is bounded by a progress deadline derived from the startup deadline

## Status

Accepted, 2026-11-02 (approval: feature delta § Approval record, D10). Not
implemented. GH #318, GH #334. The exact contract, the new public surface, the
lifecycle gate (G-7), the formal protocol model and the DISTILL obligations live in
the [feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md).

**Why one ADR.** The decision is "the Service stream's bound is derived from the
workload's startup deadline, owned by the stream handler". Two clauses that look
separate are consequences of it: the owner (the handler that already owns the timer)
and the client envelope (the CLI's request timeout must exceed the server's deadline,
or the client hides the typed terminal). The timeout's `last_attempt` detail is
independently adoptable and reversible, and is
[ADR-0257](adr-0257-the-progress-deadline-timeout-carries-the-latest-attempt-as-labelled-detail.md).
Supporting evidence: the
[research](../../research/orchestration/probe-failure-restart-authority-and-give-up-reporting-research.md)
(Kubernetes `progressDeadlineSeconds`, Nomad `progress_deadline` and
`healthy_deadline`, ECS circuit breaker). Related:
[ADR-0252](adr-0252-a-startup-probe-failure-stops-the-allocation-and-the-restart-authority-decides.md),
ADR-0251, ADR-0059 Q4 (the cap-timer terminal is synthesised by the stream).

## Context

The Service deploy stream is bounded by one fixed server-side timer, 90 s, shared
with the Job stream, armed at stream open and fired as a failure whose reason names
the elapsed seconds and nothing else. The CLI sizes its HTTP envelope from it. The
bound does not follow the workload: a Service whose startup deadline exceeds about
60 s and which becomes `Stable` after 90 s is reported failed while healthy. With a
startup-probe failure now named at give-up (ADR-0252), about six startup deadlines
after the first attempt, the timeout is the operator's only terminal at the default
deadline, and it carries no cause.

Kubernetes and Nomad both bound a rollout with a clock distinct from the
per-instance restart budget, require it to exceed the startup or healthy deadline,
and report a generic timeout with specifics elsewhere. Neither ends at the first
failed attempt or waits for restart exhaustion. Both keep the clock in a controller
that owns a durable deployment object. Overdrive has no deployment object: the
Service intent is the workload, and `workload describe` already shows every
attempt's rows.

## Decision

**The Service deploy stream stops waiting at a progress deadline computed from the
workload's startup deadline, and reports an honest timeout. The restart authority's
give-up remains the only authoritative failure reason.**

- The deadline is the larger of the existing stream cap and the workload's startup
  deadline plus a fixed 30 s margin. It exceeds the startup deadline by
  construction, and is 90 s at the default spec. It is derived, not configurable:
  correctness needs no operator setting, so no spec key or flag is added.
- The owner is the server-side stream handler, which already owns the timer and
  receives every lifecycle event of the workload. The clock starts when the stream
  opens and is not extended: a `Stable` ends the Service stream.
- The timeout reuses the existing timeout failure; the archived failure-reason type
  is unchanged. What detail it carries is
  [ADR-0257](adr-0257-the-progress-deadline-timeout-carries-the-latest-attempt-as-labelled-detail.md).
- The CLI sizes its HTTP envelope from the same derivation, so a deadline above
  the old fixed envelope is not cut by the client. The floor in the derivation is
  the one constant the server uses in production, because the server's stream cap is
  never operator-configured; the client imports that constant, so the client timeout
  cannot fall below the server deadline.
- The margin is a fixed 30 s. It is exact, not slack: the VM boot deadline is also
  30 s, so a guest that boots at its deadline and then passes its startup probe on
  the last attempt reaches `Stable` after the progress deadline (feature delta,
  exposure E-4). The stream then ends with an honest timeout, never a false success.
- The Job stream keeps its cap and behaviour.

## Alternatives considered

- **A CLI-side timer.** The verdict lands on one client, which cannot know the
  attempt causes the server holds.
- **A server-side persisted condition (Kubernetes `ProgressDeadlineExceeded`,
  Nomad `progress_deadline`).** Needs a durable deployment object Overdrive does not
  have: a new row family, an author, a wake model and an envelope, for a consumer
  whose lifetime is one HTTP request.
- **Keep the fixed cap.** Reports a healthy slow-starting Service failed and leaves
  the timeout cause-less.
- **A deadline sized to the give-up bound.** Couples the wait to the restart
  budget, which the references avoid, and waits about six minutes at the default.
- **An operator-set progress deadline.** New configuration surface that
  correctness does not need; the user's to add.

## Consequences

- Positive: a healthy slow-starting Service ends `Stable`, not `Timeout`.
- Negative: new public surface (two pure functions, one constant, one changed
  stream builder signature, one changed client signature); the exact list is in the
  feature delta.
- Negative: the deadline restarts per `overdrive deploy` invocation and nothing is
  persisted; the durable record is the allocation rows.
- Negative: an unchanged redeploy of a workload with no recorded verdict sees only
  events after its stream opened (a recorded verdict is reported at once by
  [ADR-0256](adr-0256-a-service-deploy-stream-first-reports-the-verdict-the-workload-already-has.md)).
- Neutral: at the default spec the operator-visible timeout stays 90 s.
