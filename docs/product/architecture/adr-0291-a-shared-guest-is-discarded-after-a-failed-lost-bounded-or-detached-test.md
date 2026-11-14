# ADR-0291 — A shared guest is discarded after a failed, lost, bounded or detached test

## Status

**Proposed — pending independent DESIGN review (DDD-9 in the feature delta).**
The clause for a test whose client went away while it ran (DDD-18) was approved
by the user on 2026-11-14 (N-3); the rest follows DDD-9.

## Context

A test that fails, a guest that is lost, an exec that hits its bound, or a test
whose process nextest killed while it ran may leave state that changes the
result of later tests and of a retry. A retry exists because a test failed, and
the failed test is the one most likely to have left state.

## Decision

A guest that ran a failed test, was lost, hit its exec bound (ADR-0292), or ran
a test whose client went away while it ran is discarded; the next test boots a
replacement (generation + 1, about 3 s). A passing test whose client stayed
attached leaves the guest for the next test. A nextest retry is a new test
process and so gets a fresh guest after a failure. When tests are waiting, the
replacement boot starts in the same event that discards the guest.

## Alternatives

- **Reuse the guest after a failure**: later results depend on residue from the
  failed test; rejected.
- **Discard immediately when the client goes away**: cancels the in-guest test
  mid-effect (ADR-0284); rejected. The discard waits for the exec to end.
- **Keep the guest after a killed test passes** (N-3 alternative): residue of a
  timed-out test reaches the next test unobserved; rejected in this proposal.

## Consequences

A failure, loss, bound or kill costs one extra boot for the next test. Tests
that share a guest share its kernel, cgroup tree, nft tables and `/tmp`; the
owner runs each profile's per-test setup and cleanup before and after every test
and resets nothing else.

## Links

- [Decisions DDD-9 and DDD-18](../../feature/metal-test-placement-attribute/feature-delta.md#ddd-9--one-at-a-time-no-cancel-discard-after-failure-question-3)
- [Formal protocol model](../../../specs/quint/shared-guest-owner/README.md)
- [ADR-0284](adr-0284-a-shared-guest-serves-one-test-at-a-time-and-never-cancels-a-running-test.md)
