# ADR-0108 — Durable reservation consumes successor allocation identity

## Status

**User-ratified as P-105-5A on 2026-09-13; the decision passed its iteration-2
technical check, while the complete corrective DESIGN received the final
`CHANGES_REQUESTED` verdict.** The user capped review at two cycles. This record
is not implementation authority without explicit user disposition of that
final verdict. It replaces withdrawn ADR-0107.

## Context

The convergence runtime durably persists `WorkloadLifecycle` View state before
dispatch. The same dispatch can fail before a successor effect begins, or a
successor can execute without an accepted observation row. Distinguishing an
unconsumed reservation from a consumed unpublished execution would require new
durable state or a revised dispatch-result protocol. Neither is necessary to
correct the reproduced #284 identity alias.

The existing persisted View can instead act as the issued-identity boundary.
This deliberately permits numeric gaps while guaranteeing that no attempted or
possibly attempted execution identity is assigned again.

## Decision

**Once a successor `AllocationId` reservation is durably persisted in the
`WorkloadLifecycle` View, that identity is consumed even if dispatch never
reaches a successor effect; every later successor uses a higher identity.**

## Alternatives considered

- **Consume only at a successor-owned effect.** Withdrawn with ADR-0107 because
  existing state cannot represent the transition without a new mechanism.
- **Reuse every reservation that has no accepted row.** Rejected because row
  absence does not prove that no physical execution used the identity.
- **Add a durable reservation/launch phase protocol.** Rejected as unnecessary
  persistence and recovery expansion for the bounded correction.

## Consequences

Positive:

- identity non-reuse is durable before any host effect;
- the existing View-fsync-before-dispatch boundary is sufficient; and
- predecessor read, handoff or cleanup failure cannot determine whether the
  successor suffix is reused.

Negative:

- failures before successor launch can leave unused numeric gaps;
- retained issued keys participate in later suffix selection even without an
  observation row; and
- allocation suffixes express monotonic uniqueness, not a gap-free count of
  accepted executions.

## Links

- [Canonical corrective feature delta](../../feature/vm-recreation-allocation-id-reuse/feature-delta.md)
- [ADR-0105 — driver-neutral physical allocation identity](adr-0105-driver-neutral-allocation-replacement-identity.md)
- [ADR-0106 — successor creation does not wait for cleanup](adr-0106-successor-creation-does-not-wait-for-predecessor-cleanup.md)
- [ADR-0109 — terminal predecessor handoff](adr-0109-replacement-requires-terminal-predecessor-handoff.md)
- [Withdrawn ADR-0107](adr-0107-successor-identity-consumption-follows-successor-owned-effect.md)
- [Architecture brief](brief.md)
- [C4 diagrams](c4-diagrams.md#driver-neutral-allocation-replacement-gh-284-corrective-proposal)
- [GH #284](https://github.com/overdrive-sh/overdrive/issues/284)
