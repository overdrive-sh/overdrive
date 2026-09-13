# ADR-0105 — Allocation identity is driver-neutral and names one physical execution

## Status

**User-ratified on 2026-09-13; the decision passed its iteration-2 technical
check, while the complete corrective DESIGN received the final
`CHANGES_REQUESTED` verdict.** The user capped review at two cycles. This record
is not implementation authority without explicit user disposition of that
final verdict.

On acceptance of the corrected DESIGN, this decision supersedes ADR-0104's
VM-only identity boundary. The ratified exact contract lives in the canonical
feature delta and is not duplicated here.

## Context

GH #284 proves that assigning one `AllocationId` to distinct VM executions
aliases allocation-derived host artifacts. ADR-0104/PR #292 corrected the VM
case but made `WorkloadLifecycle` select identity semantics from the VM driver
discriminator, leaving legacy Exec on a different same-ID contract.
That makes a temporary adapter, and every future driver variant, participate in
application identity policy.

`WorkloadId` already owns declared workload intent and lifecycle policy.
`AllocationId` already keys driver execution state, observations, SVIDs and
allocation-scoped capabilities. The two concepts therefore need one stable
meaning independent of adapter type.

## Decision

**One `AllocationId` identifies one physical execution attempt; every automatic
replacement receives a distinct fresh `AllocationId`, independent of driver,
while `WorkloadId` remains the stable logical owner.**

## Alternatives considered

- **VM-only fresh identity, Exec same-ID.** Rejected by the user because it
  places application identity policy on a driver discriminator and repeats the
  decision for every future execution adapter.
- **A driver policy method selecting replacement identity.** Rejected because
  it hides the same boundary error behind a port; drivers execute supplied
  allocations and do not own workload lineage.
- **Stable `AllocationId` plus a driver-specific incarnation identifier.**
  Rejected by GH #284 because it preserves the conflated allocation identity
  and adds another identifier to every artifact and cleanup path.
- **A permanent same-ID Exec compatibility exception.** Rejected. Exec is
  temporary and tracked for removal by GH #293; its presence does not define a
  second application meaning.

## Consequences

Positive:

- current and future execution adapters inherit one allocation identity model;
- allocation-scoped VM artifacts are structurally disjoint across replacements;
- no new driver policy surface or incarnation type is introduced.

Negative:

- legacy Exec changes physical allocation identity while it remains supported;
- existing same-key restart contracts cannot remain implementation authority
  and require re-DISTILL after this corrective design passes review; and
- durable reservation may create intentional numeric gaps under ADR-0108.

## Links

- [Canonical corrective feature delta](../../feature/vm-recreation-allocation-id-reuse/feature-delta.md)
- [ADR-0106 — cleanup completion does not gate successor creation](adr-0106-successor-creation-does-not-wait-for-predecessor-cleanup.md)
- [ADR-0108 — durable reservation consumes successor identity](adr-0108-durable-reservation-consumes-successor-allocation-identity.md)
- [ADR-0109 — terminal predecessor handoff](adr-0109-replacement-requires-terminal-predecessor-handoff.md)
- [Withdrawn ADR-0107](adr-0107-successor-identity-consumption-follows-successor-owned-effect.md)
- [Architecture brief](brief.md)
- [C4 diagrams](c4-diagrams.md#driver-neutral-allocation-replacement-gh-284-corrective-proposal)
- [GH #284](https://github.com/overdrive-sh/overdrive/issues/284)
- [GH #293](https://github.com/overdrive-sh/overdrive/issues/293)
- [PR #292](https://github.com/overdrive-sh/overdrive/pull/292)
