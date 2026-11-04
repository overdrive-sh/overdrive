# ADR-0259: A startup-probe failure stops the allocation and the restart authority decides

## Status

Accepted, 2026-11-02 (approval: feature delta § Approval record, D7, including the
restated K1, the new public stopped-by disposition, the changed value meaning of
`StartupProbeFailed.attempts` and D5's changed user-visible sentence). Not
implemented. GH #318, GH #334. The exact contract, the lifecycle gate (G-5), the
formal protocol model and the DISTILL obligations live in the
[feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md)
(D7). Companions: [ADR-0257](adr-0257-service-lifecycle-announces-a-startup-failure-only-for-a-live-allocation.md)
(the guard that keeps this decision off terminal rows),
[ADR-0258](adr-0258-the-restart-authority-names-a-pre-startup-crash-loop-at-give-up.md)
(the same give-up classification point, for a crash),
[ADR-0260](adr-0260-a-failed-startup-stop-is-retried-once-per-new-startup-probe-row.md)
(the stop is retried when it does not land),
[ADR-0261](adr-0261-liveness-probing-begins-when-the-service-is-stable.md) (the
liveness stop cannot precede `Stable`),
[ADR-0262](adr-0262-the-service-deploy-stream-is-bounded-by-a-progress-deadline-derived-from-the-startup-deadline.md)
(the deploy stream's bound), ADR-0087 (the liveness decision this one mirrors;
single restart authority).

Supporting evidence, from
[the research](../../research/orchestration/probe-failure-restart-authority-and-give-up-reporting-research.md)
(Kubernetes @7b4bd5d3, Nomad @64cdf9f9): in Kubernetes a startup-probe kill and a
liveness kill take the same path as a crash restart
(`pkg/kubelet/kuberuntime/kuberuntime_manager.go:1440-1470`: both set
`ContainersToKill` and, when the restart policy says so, `ContainersToStart`), and
the probe worker only publishes a result (`pkg/kubelet/prober/worker.go:386-392`);
in Nomad a `check_restart` restart is signalled with `failure=true` and increments
the same restart tracker as a crash (`client/serviceregistration/watcher.go:105-117`,
`client/allocrunner/taskrunner/restarts/restarts.go:105-118`, `:201-226`). Neither
finalises an instance as a terminal failure on a probe verdict.

## Context

`ServiceLifecycle` finalises a live Service allocation as
`ServiceFailed { StartupProbeFailed }` when the producer's startup-failure streak
reached its maximum, the startup deadline passed and no startup probe ever
passed. The action shim runs the genuine-terminal path for that finalisation and
writes the allocation's row as `Failed` with the terminal claim. The instance is
not stopped by this path.

`WorkloadLifecycle`, the single restart authority (ADR-0087), then selects the
same `Failed` row as restartable, because its restart selection ignores the row's
terminal claim, and restarts it under the shared budget. At the ceiling it
finalises `BackoffExhausted` over the earlier claim. So one allocation's ending is
authored by two reconcilers with no ordering: the first claim names a startup
failure and the allocation is then restarted anyway. The deploy stream closes on
the first terminal event, so the operator is told the workload failed while the
platform keeps trying.

The liveness path already has the right shape (ADR-0087 D2 to D4):
`ServiceLifecycle` detects the failure and only stops the allocation, with a
disposition recording the cause on the observed row; `WorkloadLifecycle` restarts
it under its one budget and, at the ceiling, names the cause from the final
attempt's row. Kubelet is the precedent: a failed startup or liveness probe kills
the container, and the restart policy and back-off decide what happens next.

One consequence of the shape needs a decision of its own. The action shim echoes
a stop's terminal claim onto the lifecycle event that the Service deploy stream
consumes, and the stream treats a stop terminal as the end of the deploy (exit 0,
"stopped"). A liveness stop happens after the allocation is Stable, when the
stream has already closed. A startup-probe stop happens before Stable, while the
stream is open.

## Decision

**A startup-probe failure on a live Service allocation stops the allocation with a
new stopped-by disposition that records the cause on the observed row.
`ServiceLifecycle` writes no failure terminal for it. `WorkloadLifecycle` alone
restarts it and, when the budget is spent, finalises the give-up.**

- The detection gates are unchanged (streak at maximum, deadline passed, no pass),
  and the guard of ADR-0257 is unchanged: a terminal row yields no action. The
  action changes from a failure finalisation to a stop.
- The stopped row is `Terminated` and restartable, like a liveness-stopped row.
  The intentional-stop exemption (operator, system garbage collection) does not
  cover it, so it consumes restart budget exactly as a crash does.
- At the restart ceiling `WorkloadLifecycle` selects the give-up terminal from the
  final attempt's observed row, in this order: liveness kill (unchanged), startup
  kill (a startup-probe failure), pre-startup crash (ADR-0258), anything else
  (`BackoffExhausted`, unchanged). A cause written by the killer on the row
  outranks a cause inferred from a crash reason and probe rows. The last cause
  wins.
- The startup-probe failure terminal is written only at give-up. The failure
  detail it carries (the probe's last failure reason and attempt count) is read
  from the final attempt's startup probe row by `WorkloadLifecycle`, as the
  detector read it.
- The deploy stream does not end on the stop, nor on a liveness stop (ADR-0261
  keeps the same projection rule). It ends on the give-up terminal, on `Stable` for
  a successor that passes, or on its own progress deadline (ADR-0262).

## Alternatives considered

- **Keep the finalisation and make the restart authority skip it.** Teaching
  `WorkloadLifecycle` that a startup-probe terminal is final would stop the
  restart after one attempt: a restart-policy change, not an ownership change,
  and a Service whose dependency is slow at boot stops being retried. It also
  leaves the instance running after the platform declared it failed.
- **Give up at the first startup-probe stop instead of restarting.** The only
  alternative that keeps the startup-failure honesty bound literally (deadline
  plus one tick). It makes the restart authority decide "do not restart" for this
  cause, which is a policy change that the user did not approve for the crash
  case either (ADR-0258 restarts and then gives up). Not taken without the user.
- **Reuse the liveness disposition for the startup stop.** The two causes then
  cannot be told apart at the ceiling, so the give-up would name a liveness
  failure for a workload that never became ready. Distinct causes need distinct
  dispositions, as crash and liveness already have.
- **Have the shim or the stream drop the stop's terminal instead of a new
  disposition.** The cause must travel on the shared observed row for the restart
  authority to read it (ADR-0087 D3); removing it from the row removes the input
  the give-up needs.
- **Let the stream end on the stop and rely on the next stream.** The stream
  would report "stopped" with exit 0 at the first startup failure. That is
  dishonest, and it is the failure the honesty target exists to prevent.

## Consequences

- Positive: one author for every ending of an allocation. `ServiceLifecycle`
  detects and stops; `WorkloadLifecycle` restarts and gives up. The instance that
  failed its startup probes is actually stopped before its successor is placed.
- Positive: a Service that fails its startup probes once and starts correctly on
  the next attempt reads as `Stable`, not as `Failed` followed by silent recovery.
- Negative, a changed target: for a Service that stays alive and never passes its
  startup probes, the failure terminal arrives when the restart budget is spent,
  after about six startup deadlines, not after one. With the default deadline that
  is longer than the deploy stream's bound, so the stream ends with its own timeout
  failure (carrying the last attempt's cause as labelled detail under ADR-0264) and
  the startup-probe failure is then visible on the allocation's row and in
  `workload describe`. For a startup deadline under about fourteen seconds the
  terminal reaches the stream. Feature delta § Changed and unchanged assumptions
  records the restated target.
- Neutral: a failed stop is retried once per new startup probe row (ADR-0260).
- Neutral: one new stopped-by disposition is added at the tail of an archived,
  wire and API-spec enum. It is fieldless, so no archived layout grows and no
  envelope version is minted; the feature delta states the exact fixture
  obligation.
- Neutral: the startup-probe failure reason keeps its shape and meaning for the
  operator; only its author and the moment it is written change.
