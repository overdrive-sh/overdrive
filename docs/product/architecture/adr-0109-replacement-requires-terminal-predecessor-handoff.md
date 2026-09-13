# ADR-0109 — Replacement requires a terminal predecessor handoff

## Status

**User-ratified as P-105-4A on 2026-09-13; recorded as the focused disposition
of final review finding F-05.** This ADR was added after the user-capped second
and final review iteration, so it is not independently reviewer-approved and
is not implementation authority without explicit user disposition of that
final `CHANGES_REQUESTED` verdict.

## Context

`WorkloadLifecycle` currently treats `Draining`, `Failed` and `Terminated` as
restartable state candidates. `Draining` still represents an ending operation
whose ownership handoff is incomplete, while an accepted `Failed` or
`Terminated` numeric-current row is the existing durable observation from
which replacement policy can proceed. Historical terminal rows must not
authorize replacement of a newer accepted allocation.

This state boundary is independently reversible from both the driver-neutral
identity decision in ADR-0105 and the post-handoff cleanup ordering in
ADR-0106.

## Decision

**`WorkloadLifecycle` may emit replacement only for the accepted numeric-current
predecessor in `Failed` or `Terminated`; `Draining` is not a sufficient
ownership handoff.**

## Alternatives considered

- **Also admit `Draining`.** Rejected because it starts replacement before the
  existing ending owner has published the ratified handoff state.
- **Admit any retained terminal row.** Rejected because a historical failure
  could replace a newer accepted physical allocation.
- **Let each driver report replacement readiness.** Rejected because it moves
  application lifecycle policy into execution adapters and would recreate the
  boundary rejected in ADR-0105.
- **Require predecessor cleanup completion.** Rejected separately by ADR-0106;
  cleanup completion is not the ownership handoff.

## Consequences

Positive:

- ending ownership is durably handed off before a successor is selected;
- historical rows cannot initiate replacement of the current allocation; and
- the handoff is common to microVM-family adapters and legacy Exec.

Negative:

- a predecessor that remains `Draining` does not receive a replacement from
  this lifecycle evaluation; and
- liveness depends on the existing ending owner eventually publishing
  `Failed` or `Terminated`, without adding a timeout or recovery mechanism in
  this correction.

## Links

- [Canonical corrective feature delta](../../feature/vm-recreation-allocation-id-reuse/feature-delta.md)
- [ADR-0105 — driver-neutral physical allocation identity](adr-0105-driver-neutral-allocation-replacement-identity.md)
- [ADR-0106 — successor creation does not wait for cleanup](adr-0106-successor-creation-does-not-wait-for-predecessor-cleanup.md)
- [ADR-0108 — durable reservation consumes successor identity](adr-0108-durable-reservation-consumes-successor-allocation-identity.md)
- [Architecture brief](brief.md)
- [C4 diagrams](c4-diagrams.md#driver-neutral-allocation-replacement-gh-284-corrective-proposal)
- [GH #284](https://github.com/overdrive-sh/overdrive/issues/284)
