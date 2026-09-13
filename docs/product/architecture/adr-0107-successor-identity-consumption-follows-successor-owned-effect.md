# ADR-0107 — Successor identity consumption follows a successor-owned effect

## Status

**Withdrawn before acceptance on 2026-09-13.** The user explicitly selected
P-105-5A, the opposite consumption boundary, after this record was drafted.
[ADR-0108](adr-0108-durable-reservation-consumes-successor-allocation-identity.md)
records the ratified replacement decision. This ADR never became implementation
authority.

## Context

This draft separated durable reservation from consumed physical identity so
that a predecessor-owned failure before successor launch could reuse the
reserved suffix. Review established that the existing View-fsync-before-
dispatch boundary cannot represent that distinction without new state or a
revised result protocol. The user chose the existing durable reservation as
the consumption boundary and accepted unused numeric gaps.

## Withdrawn decision

**A successor identity would have been consumed only when a successor-owned
launch attempt began or successor publication was rejected.**

## Alternatives considered

- **Durable reservation itself consumes identity.** Selected as P-105-5A and
  recorded by ADR-0108 because it is expressible through existing View
  persistence without new API or recovery state.
- **An unrowed reservation is always reusable.** Rejected because a launched
  execution whose publication is not accepted could then be assigned the same
  physical identity again.
- **Add a reservation phase/result protocol.** Rejected for this correction;
  it expands public/private state and recovery ownership beyond the proven
  #284 need.

## Consequences

- No implementation or downstream test may rely on this withdrawn boundary.
- The numeric gap caused by a durably reserved but never launched successor is
  intentional under ADR-0108.
- The withdrawal creates no compatibility API, migration or cleanup mechanism.

## Links

- [ADR-0108 — durable reservation consumes successor identity](adr-0108-durable-reservation-consumes-successor-allocation-identity.md)
- [ADR-0105 — driver-neutral physical allocation identity](adr-0105-driver-neutral-allocation-replacement-identity.md)
- [ADR-0106 — successor creation does not wait for cleanup](adr-0106-successor-creation-does-not-wait-for-predecessor-cleanup.md)
- [ADR-0109 — terminal predecessor handoff](adr-0109-replacement-requires-terminal-predecessor-handoff.md)
- [Canonical corrective feature delta](../../feature/vm-recreation-allocation-id-reuse/feature-delta.md)
- [Architecture brief](brief.md)
- [GH #284](https://github.com/overdrive-sh/overdrive/issues/284)
