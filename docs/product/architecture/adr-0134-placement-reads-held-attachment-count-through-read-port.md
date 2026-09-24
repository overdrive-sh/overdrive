# ADR-0134 — Placement reads the authoritative held-attachment count through a hydration read-port

## Status

**Accepted (2026-09-24).** GH #295 correctness-recovery replacement DESIGN,
decision D-295-R8. Proposed 2026-09-23 and revised 2026-09-24; reviewed by
independent DESIGN review rounds 3, 4, and 5 and the round-5 verification
(`arch_rev_20260924_netns295_r5_verify`); accepted by the user on 2026-09-24
with the replacement DESIGN. Depends on ADR-0132 and ADR-0133 (the ADR-0133
counting policy is the user ruling D-295-R7 of 2026-09-24). Extends the ADR-0086 hydration boundary with one
narrow read-port. Exact signatures live only in the #295 feature delta.

## Context

Once admission linearizes at assignment (ADR-0132), a placement that ignores it
would emit `StartAllocation` actions that dispatch refuses. The reconciler
runtime re-enqueues any evaluation that emitted a non-noop action with
immediate eligibility. A refused start at the cap would therefore re-evaluate
and re-refuse without pause. Reconcilers cannot see dispatch errors, so they
cannot back off themselves.

ADR-0086 requires every hydration input to come through an injected read-port
with a sim implementation.

The pattern is established: the Kubernetes scheduler places against an
optimistic "assumed" cache that admission may later refuse, and Nomad's
schedulers plan optimistically against a snapshot that the plan applier may
reject. In both, placement is advisory and one serialized authority decides.
(Research:
`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Findings 4.1 and 4.2.)

## Decision

Add one narrow read-port to the reconciler hydration bundle. In one consistent
snapshot it exposes, from the server's guest-address pool:

- the held count (Admitted plus Retiring) and its Retiring subset;
- the lease each of the workload's allocations holds.

`WorkloadLifecycle` hydrates it. The scheduler returns the existing
`NoCapacity` outcome, emitting no action, when the held count has reached the
cap.

`RestartAllocation` is gated on the same snapshot. Its predecessor's lease
counts, because it counts until the predecessor's cleanup finishes
(ADR-0133). When the node is at the cap and the predecessor still holds a lease,
the reconciler emits the predecessor's row-neutral reclaim instead (ADR-0136);
once the lease is released, the next evaluation sees room and emits the
restart. A race between concurrent evaluations is bounded to one refused
dispatch per contended slot. The next hydration then observes the cap.

## Alternatives considered

### No read-port; rely on dispatch refusal

Rejected. An at-cap node re-dispatches refused starts without pause.

### Record admission refusals in a new observation row or View field

Rejected. It persists a process-local, derived fact and still requires a new
feedback channel from dispatch into reconciliation.

### Leave `RestartAllocation` ungated

Rejected. At the cap it would be refused at dispatch and re-evaluated
immediately, which is the hot loop this decision removes.

### Gate `RestartAllocation` excluding its own predecessor's lease

Rejected. The user ruling of 2026-09-24 makes the predecessor count until its
cleanup finishes, so excluding it would admit a successor with no room.

## Consequences

Positive: there is no hot retry loop at the cap, the scheduler keeps its
existing outcome vocabulary, and the hydration input stays DST-injectable. The
Retiring subset is visible to placement and to the operator-facing refusal
event.

Negative: the core hydration bundle gains a fifth read-port, which needs a
production implementation over the pool and a sim implementation. The
scheduler's signature changes. The node-wide CPU/memory accounting defect is
separate and tracked by
[GH #261](https://github.com/overdrive-sh/overdrive/issues/261) (decision
D-295-R9).
