# ADR-0106 — Successor creation does not wait for predecessor cleanup

## Status

**User-ratified on 2026-09-13; the decision passed its iteration-2 technical
check, and its F-06 consequence wording was corrected afterward.** The complete
corrective DESIGN received the final `CHANGES_REQUESTED` verdict, and the user
capped review at two cycles. This record is not implementation authority
without explicit user disposition of that final verdict.

The ratified exact action-shim ordering, handoff and error contract lives in the
canonical feature delta and is not duplicated here.

## Context

ADR-0105 gives predecessor and successor executions distinct physical
`AllocationId` values. Requiring all predecessor cleanup to complete before a
successor can start preserves a temporal dependency that fresh identity is
intended to remove. Conversely, cleanup still needs the exact predecessor
identity and must not be redirected through the logical workload's current
allocation.

The current replacement action-shim arm owns driver, mTLS and structural
network calls, but orders old cleanup before successor creation. Specialized VM
artifact disposal is also owned by exact-old-ID `VmDriver`/`VmReclamation`
capabilities. The available components can address both execution identities
without a new cleanup owner or policy port.

## Decision

**Once the predecessor reaches the approved terminal/ownership handoff,
successor creation may proceed without waiting for predecessor cleanup;
exact-old-ID cleanup completion or failure cannot block or roll back the
successor.**

## Alternatives considered

- **Await all predecessor cleanup before successor creation.** Rejected by the
  user because cleanup completion would remain an availability gate despite
  distinct physical identities.
- **Blindly ignore or log cleanup failure as success.** Rejected because it
  silently changes typed failure semantics and can strand exact-old resources.
- **Spawn detached cleanup.** Rejected because task submission is not effect
  completion and would add an unowned cancellation/error path.
- **Create a new retrying cleanup subsystem immediately.** Not selected. It is
  an independently decidable ownership/persistence expansion and requires
  explicit approval plus a separate exact design.

## Consequences

Positive:

- successor availability no longer depends on the latency or success of old
  cleanup;
- cleanup remains exact-old-ID and cannot mutate the successor;
- driver-specific effects remain behind existing ports.

Negative:

- predecessor and successor resources may overlap temporarily;
- a held predecessor network slot can make the successor consume another slot
  or encounter existing slot exhaustion; and
- when the successor succeeded, a failed one-shot predecessor cleanup is
  returned as its existing typed action error; when both fail, the successor
  error is returned and cleanup is secondary structured tracing; no generic
  cleanup retry owner is added.

## Links

- [Canonical corrective feature delta](../../feature/vm-recreation-allocation-id-reuse/feature-delta.md)
- [ADR-0105 — driver-neutral physical allocation identity](adr-0105-driver-neutral-allocation-replacement-identity.md)
- [ADR-0108 — durable reservation consumes successor identity](adr-0108-durable-reservation-consumes-successor-allocation-identity.md)
- [ADR-0109 — terminal predecessor handoff](adr-0109-replacement-requires-terminal-predecessor-handoff.md)
- [Withdrawn ADR-0107](adr-0107-successor-identity-consumption-follows-successor-owned-effect.md)
- [Architecture brief](brief.md)
- [C4 diagrams](c4-diagrams.md#driver-neutral-allocation-replacement-gh-284-corrective-proposal)
- [GH #284](https://github.com/overdrive-sh/overdrive/issues/284)
- [PR #292](https://github.com/overdrive-sh/overdrive/pull/292)
