# ADR-0254: Liveness probing begins when the Service is Stable

## Status

Accepted, 2026-11-02 (approval: feature delta § Approval record, D9 and the D9
extension): liveness acts only once the startup probe has passed, or when no
startup probe is declared. Not implemented. GH #318, GH #334. The exact contract
(D9), the lifecycle gate (G-6), the formal protocol model and the DISTILL
obligations live in the
[feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md).
Related: ADR-0054 § 2 and ADR-0080 § D4 (the probe runner and the `Stable` hook),
ADR-0087 (liveness stop, single restart authority),
[ADR-0252](adr-0252-a-startup-probe-failure-stops-the-allocation-and-the-restart-authority-decides.md)
(the same projection rule keeps a liveness stop off the open deploy stream).

## Context

The probe runner starts a task for every probe of every role when an allocation
reaches `Running`; only `Stable` retires the startup role. The liveness detector
checks that the allocation is `Running` and that a recorded failure streak crossed
its threshold; nothing requires the startup probe to have passed. A liveness stop
can therefore fire on an instance that is still starting, consuming the shared
restart budget and, at the ceiling, naming a liveness failure for a workload that
never became ready. The documented role contract already says liveness is
"continuous post-Stable". Kubernetes disables liveness until the startup probe
succeeds; Nomad scopes its health-check restart by a per-instance grace period.

The failure streak is recorded by the probe task from its own first row. A gate in
the detector that merely ignores liveness rows before `Stable` would leave the
streak accumulated during startup in place, and the first evaluation after
`Stable` would kill a healthy instance whose liveness probe failed while it was
starting.

## Decision

**Liveness probe tasks start when the allocation is announced `Stable`, not when it
reaches `Running`. The probe runner owns the gate, driven by the existing
`Stable` hook.**

- At `Running` the runner starts the startup and readiness tasks and retains the
  allocation's liveness probes without starting them.
- The `Stable` hook retires the startup role and starts the retained liveness
  tasks. Starting is idempotent, is a no-op for an unknown or ended allocation,
  and is serialised with the terminal hook, so a task is never started for an
  allocation whose supervision has been released.
- An opt-out Service (no startup probe) is announced `Stable` at its first
  `Running`, so its liveness starts at that point; there is no special case.
- Readiness is not gated.
- The Service deploy stream does not end on a liveness stop. After the gate a
  liveness stop cannot reach the stream waiting for `Stable`, but it can reach the
  stream of an unchanged redeploy of an already-`Stable` Service, which opens a
  stream and waits for a terminal event.

## Alternatives considered

- **Gate in the detector (ignore liveness rows until `Stable` is announced).**
  Rejected: the streak recorded during startup survives the gate (see Context).
  Fixing it needs a second persisted field recording the streak at `Stable`.
- **The runner starts liveness when its own startup task sees a pass.** Rejected: it
  reimplements the detector's `Stable` rule and runs ahead of the committed `Stable`
  row.
- **A grace period.** Rejected: a temporal guess where a structural signal exists,
  and it needs a duration derived from the spec.
- **Only guard the deploy stream.** Rejected: the premature kill and its budget
  consumption remain.

## Consequences

- Positive: a slow-starting Service with an aggressive liveness probe is no longer
  stopped during startup; the liveness streak starts at `Stable`.
- Positive: the code matches the documented role contract; the unreachable
  "stable this tick" skip in the liveness collection can be removed.
- Negative: one new public method on the probe runner and one docstring sentence on
  the driver hook; no archived, wire, persisted or spec change.
- Neutral: `workload describe` shows no liveness result before `Stable`.
- Dependency: liveness starts from the `Stable` hook, which fires after the `Stable`
  row commits. That the hook eventually fires for every allocation that should be
  `Stable` rests on `ServiceLifecycle` re-announcing a `Stable` whose row write
  failed, which
  [ADR-0258](adr-0258-servicelifecycle-announces-stable-from-the-committed-row.md)
  (accepted 2026-11-03) decides.
