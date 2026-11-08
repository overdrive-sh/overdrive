# ADR-0280 — Metal nextest execution uses explicit test placement

## Status

**Accepted — 2026-11-08.** The user explicitly stated “Approve P1–P5”. Independent technical review is APPROVED; the exact accepted contract is in the feature delta.

## Context

The current runner forwards arbitrary privileged commands. The existing suite
has concrete guest groups, but does not automatically wrap direct nextest.
Ordinary test-kernel isolation therefore needs a declared execution boundary.

## Decision

Keep nextest as selection/retry/report/result owner and use its existing
per-test wrapper to place each exact body in its declared host, native-launcher
or existing guest context. Keep arbitrary privileged host authority explicit
and exclusive, rather than interpreting shell programs to guess their effects.

## Alternatives

A whole-run outer guest conflicts with the selected group boundary. Replacing
nextest with a batch-result interpreter changes command semantics. Sharing one
persistent guest across independent wrapper processes requires a broker beyond
this bounded adaptation. Allowing unknown bodies on the host violates isolation.

## Consequences

Direct nextest preserves selected argv, profiles, retries and actual aggregate
status, at the cost of a fresh existing-group instance per kernel attempt. The
explicit suite retains its batching. Ordinary command admission and proposed
`run --exclusive` affect shell chains/privileged probes and must be surfaced;
this ADR does not pretend they were already accepted. No new product runtime
port or generic scheduler is introduced.

## Links

- [Exact proposed ordinary/exclusive and wrapper contract](../../feature/metal-run-isolation/feature-delta.md#wave-design--ref-ordinary-command-and-test-execution--p4)
- [Closed native inventory](../../feature/metal-run-isolation/design/placement-inventory.md)
- [Native construction evidence](../../feature/metal-run-isolation/spike/findings.md)

- [Reproduced six-identity socket placement correction and rejected current CLI reachability claim](../../feature/metal-run-isolation/design/f1-disposition.md)
