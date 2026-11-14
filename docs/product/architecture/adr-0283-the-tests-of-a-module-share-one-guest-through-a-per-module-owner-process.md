# ADR-0283 — The tests of a module share one guest through a per-declaration owner process

## Status

**Proposed — awaiting user approval (DDD-8 in the feature delta).**
The declaration shape the tests read (ADR-0289) was approved by the user on
2026-11-13; the owner remains proposed. If accepted, it reverses the "a broker
is beyond this adaptation" alternative recorded in ADR-0280, for the
shared-guest case only.

## Context

Ordinary Metal runs boot one guest per guest-placed test attempt (about 0.45 s
staging plus 2.4 s boot). The explicit batched dispatcher shared a guest across
a group, and it is being retired. nextest runs every test in its own process,
so a guest shared by several tests must outlive any one test process. The
shared-guest probe showed a detached owner, started by the first test to
arrive and reached over a unix socket, keeps per-test results, retries and
`-E` filtering, and costs about one boot per module.

## Decision

The tests that read one shared-guest declaration share one guest, within one
build of one test binary. The first arrival elects one owner process under an
exclusive lock on a per-scope file; the owner boots the guest. Later tests
attach to that owner over a unix socket in the run's own runtime directory. A
test that cannot elect or attach fails with a typed host failure naming the
stage. The owner's lifetime is ADR-0290 (its guest dies with it), ADR-0293 (it
exits when idle). The run's transient unit ends any leftover descendant at the
end of the command (ADR-0281).

## Alternatives

- **A nextest setup script boots the guest**: it does not know the test
  binary, has no teardown hook, and would need the same socket and reaper;
  rejected.
- **One nextest test runs the whole module in one guest**: loses per-test
  results, `-E` filtering and retries; rejected (probe control).
- **The first test process owns the guest while it runs**: the guest dies
  when that test ends; rejected.

## Consequences

One extra process kind (the owner), prepared by `guest-kit-prepare`. Election
is the kernel's lock, released when the holder dies, so a crashed elector or
owner leaves no stale claim (election detail is in the feature delta, DDD-8).
A client attached to a killed owner sees end of stream, which maps to an
owner-loss failure. The protocol is specified in
`specs/quint/shared-guest-owner/`.

## Links

- [Interface contract](../../feature/metal-test-placement-attribute/feature-delta.md#wave-design--ref-interface-contracts)
- [Formal protocol model](../../../specs/quint/shared-guest-owner/README.md)
- [Shared-guest probe findings](../../feature/metal-test-placement-attribute/spike/findings-module-shared-guest.md)
- [ADR-0280](adr-0280-metal-nextest-execution-uses-explicit-test-placement.md)
- [ADR-0289](adr-0289-a-module-opts-into-a-shared-guest-with-a-function-like-declaration-its-tests-read.md)
- [ADR-0290](adr-0290-the-shared-guest-owner-is-init-of-its-own-pid-namespace-so-its-guest-dies-with-it.md)
- [ADR-0293](adr-0293-a-shared-guest-owner-exits-when-it-is-idle.md)
