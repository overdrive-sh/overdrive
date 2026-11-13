# ADR-0284 — A shared guest serves one test at a time and is never reused after a failure

## Status

**Proposed — awaiting user approval (DDD-9 in the feature delta).**

## Context

Tests in a shared guest share one kernel, cgroup tree, nft tables and `/tmp`.
A guest placement exists to keep node-global kernel effects away from other
tests, and the guest groups already run under a single-writer nextest group.
nextest can kill a test process on its slow-timeout while the in-guest test is
still running. A test that fails, or a guest that is lost, may leave state that
changes the result of later tests and of a retry.

## Decision

The owner runs at most one test at a time and serves waiting tests in arrival
order. A test process that goes away while queued is dropped from the queue;
one that goes away while its test runs does not cancel it. The in-guest test
runs to its bound, its result is discarded, and the next test is served only
after it has ended. A guest that ran a failed test, or was lost, is discarded
and the next test boots a replacement guest. A passing test leaves the guest
for the next one. A nextest retry is a new test process and so gets a fresh
guest after a failure.

## Alternatives

- **Concurrent execs in one guest**: reintroduces the node-global races the
  guest isolates; rejected.
- **Refuse a second concurrent attach**: turns a nextest scheduling choice into
  a test failure; rejected.
- **Cancel the in-guest test when its process goes away**: cleanup-hostile,
  and a half-cancelled test overlaps the next one (`.claude/rules/reconcilers.md`
  § "Superseded in-flight work runs to its bound"); rejected.
- **Reuse the guest after a failure**: later results depend on residue from
  the failed test; rejected.

## Consequences

A failure costs one extra boot (about 3 s) for the next test. A module whose
tests are not in a single-writer nextest group queues waiting tests, and the
wait counts toward nextest's per-test timeout. State left by a passing test is
visible to the next one; sharing is an explicit opt-in per test.

## Links

- [Interface contract](../../feature/metal-test-placement-attribute/feature-delta.md#wave-design--ref-interface-contracts)
- [Formal protocol model](../../../specs/quint/shared-guest-owner/README.md)
- [ADR-0283](adr-0283-the-tests-of-a-module-share-one-guest-through-a-per-module-owner-process.md)
