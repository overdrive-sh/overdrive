# ADR-0278 — Metal ordinary runs share the maintenance barrier

## Status

**Accepted — 2026-11-08.** The user explicitly stated “Approve P1–P5”. Independent technical review is APPROVED; the exact accepted contract is in the feature delta.

## Context

Existing durable services already separate invocation records and logs, but an
exclusive box-wide lease and singleton ordinary owner serialize independent
worktrees. Source transfer/build consistency and physical maintenance still
need explicit owners.

## Decision

Ordinary durable invocations share the existing physical-maintenance barrier
and exclusively own their checkout; physical provisioning, shell and explicitly
exclusive commands retain exclusive admission. Ordinary owner records belong to
the invocation, not a singleton shared owner.

## Alternatives

Removing flock without replacing singleton ownership loses maintenance and
source qualification. Keeping ordinary exclusive admission fails the requested
concurrency outcome. A new scheduler adds no necessary ownership guarantee.

## Consequences

Different ordinary worktrees can execute together. Same-checkout exclusion,
durable registration/START/result and honest collector failures remain. One
existing admission deadline applies. Completion ownership is separately decided
by ADR-0281.

## Links

- [Exact proposed ports, wire, records and error mappings](../../feature/metal-run-isolation/feature-delta.md#wave-design--ref-driven-ports-maintenance-and-durable-ownership--p2)
- [Native evidence](../../feature/metal-run-isolation/spike/findings.md)
- [Proposed C4](c4-diagrams.md#metal-run-isolation-proposed)

- [Bounded P2/P5 formal protocol and pinned owner projections](../../feature/metal-run-isolation/feature-delta.md#wave-design--ref-formal-protocol-model)
