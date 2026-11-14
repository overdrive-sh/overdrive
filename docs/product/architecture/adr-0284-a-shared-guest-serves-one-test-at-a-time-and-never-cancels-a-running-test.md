# ADR-0284 — A shared guest serves one test at a time and never cancels a running test

## Status

**Proposed — awaiting user approval (DDD-9 in the feature delta).**

## Context

Tests in a shared guest share one kernel, cgroup tree, nft tables and `/tmp`.
A guest placement exists to keep node-global kernel effects away from other
tests, and the guest groups already run under a single-writer nextest group.
nextest can kill a test process on its slow-timeout while the in-guest test is
still running.

## Decision

The owner runs at most one test at a time and serves waiting tests in arrival
order. A test process that goes away while queued is dropped from the queue;
one that goes away while its test runs does not cancel it: the in-guest test
runs to its bound (ADR-0292), its result is discarded, and the next test is
served only after it has ended. What happens to the guest afterwards is
ADR-0291.

## Alternatives

- **Concurrent execs in one guest**: reintroduces the node-global races the
  guest isolates; rejected.
- **Refuse a second concurrent attach**: turns a nextest scheduling choice into
  a test failure; rejected.
- **Cancel the in-guest test when its process goes away**: cleanup-hostile,
  and a half-cancelled test overlaps the next one (`.claude/rules/reconcilers.md`
  § "Superseded in-flight work runs to its bound"); rejected.

## Consequences

A module whose tests are not in a single-writer nextest group queues waiting
tests, and the wait counts toward nextest's per-test timeout. The two
properties (one at a time, no cancel) are one decision because cancelling is
only meaningful against the slot, and each is exercised by its own model
hazard (`noSerial`, `cancelOnDisconnect`).

## Links

- [Interface contract](../../feature/metal-test-placement-attribute/feature-delta.md#wave-design--ref-interface-contracts)
- [Formal protocol model](../../../specs/quint/shared-guest-owner/README.md)
- [ADR-0283](adr-0283-the-tests-of-a-module-share-one-guest-through-a-per-module-owner-process.md)
- [ADR-0291](adr-0291-a-shared-guest-is-discarded-after-a-failed-lost-bounded-or-detached-test.md)
- [ADR-0292](adr-0292-every-shared-exec-has-a-bound.md)
