# ADR-0267: A startup-probed allocation is a backend only after its startup has passed

## Status

Accepted, 2026-11-03 (approval: feature delta § Approval record, B4 / D12; the user
chose the Kubernetes rule over parity with the first-allocation behaviour; the user
ruled the same day that the redundant per-allocation veto and the veto-before-stop
ordering are removed). Not implemented. GH #318, GH #334. **Amends the operative
[ADR-0096](adr-0096-startup-failure-withdraws-backend-eligibility.md)** (the
amendment note is recorded there: D1 step 1 and D2 are superseded by this decision). The exact contract (D12), the lifecycle gate
(G-9), the model decision and the DISTILL obligations live in the
[feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md).
Related: [ADR-0265](adr-0265-servicelifecycle-announces-stable-from-the-committed-row.md)
(the committed `Stable` claim is the signal this decision reads),
[ADR-0261](adr-0261-liveness-probing-begins-when-the-service-is-stable.md) (startup
supervision ends at `Stable`), [ADR-0259](adr-0259-a-startup-probe-failure-stops-the-allocation-and-the-restart-authority-decides.md),
ADR-0101 (the backend row), ADR-0105 (a restart is a fresh allocation id).

## Context

ADR-0096 withdraws backend eligibility from an allocation whose startup contract
terminally failed, and keeps "a non-terminal no-readiness allocation stays
healthy". Two things in the code and in this feature make that rule leave a gap.

1. A first allocation of a Service that declares a startup probe and no readiness
   probe is `healthy` the moment it is `Running`, before its startup probe has said
   anything. Peers can be routed to a guest that has not passed the check the
   operator declared for exactly that purpose.
2. The only thing that keeps a restart successor out of eligibility until its own
   startup result is a lineage-wide hold: `compute_backend_healthy` returns false
   for a startup-probed allocation without a startup `Pass` while the view's
   `terminal_announced` set is non-empty (`service_lifecycle.rs:1136-1144 @ 04052ad75`).
   The hold keys on the whole view, not on the allocation's lineage, and the set is
   also filled by unrelated bookkeeping (a `Failed` row that never reached
   `Running`). Before this feature a crash inside the startup deadline was announced
   by the early-exit branch, so its successors were held incidentally; ADR-0258
   deletes that branch, so the hold would apply only to the successors of a startup
   stop (ADR-0259, ADR-0260), and a crash successor would be eligible at `Running`
   while a startup-stop successor would not.

Kubernetes treats the startup probe as a gate on everything else: readiness and
liveness do not run, and the pod is not ready (so not an endpoint), until the
startup probe has succeeded (kubelet `worker.go:348-359` and the kubernetes.io probe
documentation, both recorded in the
[research](../../research/orchestration/probe-failure-restart-authority-and-give-up-reporting-research.md)).
The rule applies to every pod, first or replacement, with no per-lineage state.

## Decision

**An allocation whose Service declares a startup probe is a backend (`healthy`)
only once its own startup has passed, which the system records as the allocation's
committed `Stable` claim. This applies to a first allocation and to a restart
successor alike. The readiness rule is evaluated only after that.**

- **Signal.** The `Stable` claim committed on the allocation's own status row
  (ADR-0265's hydrated fact). It is authoritative (the announcement requires the
  startup probe's latest result to be `Pass`), it is monotone for the allocation
  (no later writer drops it), and it does not depend on the startup probe rows,
  which stop updating when startup supervision ends at `Stable` (ADR-0261). So the
  allocation neither flaps nor strands after `Stable`, and no view marker is read.
- **Order of the predicate.** (1) a startup-probed allocation without a committed
  `Stable` is not healthy; (2) the existing readiness rule, unchanged.
- **The per-allocation backend veto is removed** (user ruling 2026-11-03). ADR-0096 D1
  step 1 withdrew eligibility from an allocation whose startup failure was decided.
  Such an allocation never has a committed `Stable` (the stop is gated on the latest
  startup row not being `Pass`, and `Stable` on it being `Pass`), so step (1) already
  makes it unhealthy; the veto and the veto-before-stop action ordering (ADR-0096 D2)
  defend nothing. The allocation also needs no `healthy: false` write ahead of its
  stop: it was never `healthy: true`.
- **Opt-out.** A Service that declares no startup probe (the explicit
  `[[health_check.startup]] = []` opt-out) is unchanged: its first `Running` is its
  `Stable` (ADR-0058) and step (2) does not apply.
- **The lineage hold is deleted.** Step (1) subsumes it for every case it covered
  and removes its two defects (view-wide scope, unrelated writers).
- No new field, state, action, wire or persisted shape: it reads the fact ADR-0265
  already adds.

## Alternatives considered

- **Parity with the first allocation (accept the narrowing).** Keep first
  allocations healthy at `Running`, drop the hold for crash successors, keep it for
  startup-stop successors. Rejected by the user: it leaves the first-allocation gap
  and a behaviour that depends on why the predecessor ended.
- **Key on the allocation's own latest startup row being `Pass`.** Rejected: the
  startup role stops at `Stable`, and a `Fail` row can land between the `Pass`
  and the role's retirement, so a `Stable` allocation could turn unhealthy and never
  recover. A second rule ("or Stable") would be needed anyway, and then the `Stable`
  claim alone is sufficient.
- **Key on a view marker of what was emitted** (the former `stable_announced` set).
  Rejected: lost with a failed write (the defect ADR-0265 removes; the marker is gone).
- **Keep the per-allocation veto beside step (1).** Rejected by the user 2026-11-03:
  it is unreachable for any allocation step (1) does not already exclude.
- **Hold crash successors with new per-lineage state.** Rejected: a new mechanism for
  a case a per-allocation rule already covers.
- **Gate the readiness probe task itself on `Stable`.** Rejected here: the user ruled
  D9 gates liveness only. The eligibility rule achieves the operator-visible effect
  (no traffic before startup) without touching the probe runner.

## Consequences

- Positive: one rule, per allocation, no lineage state; the incidental successor hold
  and its two defects are gone; Kubernetes-shaped semantics.
- Behaviour change (operator-visible): a startup-probed Service receives no traffic
  until its startup probe has passed. For a first allocation this is new; before, it
  was a backend at `Running`. A peer dialing a known frontend in that window gets the
  existing fail-closed result (no healthy backend) instead of reaching an
  unchecked guest.
- A lost `Stable` write holds traffic until ADR-0265's re-announcement lands, which
  is the same bound as liveness starting.
- Negative: eligibility lags the startup `Pass` by the `Stable` row write and one
  evaluation (the reconciler is woken by the `alloc_status` change).
- Residual difference from Kubernetes: readiness rows produced while the startup
  probe was running still count once startup has passed (the readiness probe task is
  not deferred, D9 ruling). Eligibility is gated; the probe schedule is not.
