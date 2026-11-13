# ADR-0283 — The tests of a module share one guest through a per-module owner process

## Status

**Proposed — awaiting user approval (DDD-7, DDD-8 in the feature delta).**
If accepted, it reverses the "a broker is beyond this adaptation" alternative
recorded in ADR-0280, for the shared-guest case only.

## Context

Ordinary Metal runs boot one guest per guest-placed test attempt (about 0.45 s
staging plus 2.4 s boot). The explicit batched dispatcher shared a guest across
a group, and it is being retired. nextest runs every test in its own process,
so a guest shared by several tests must outlive any one test process. The
shared-guest probe showed a detached owner, started by the first test to
arrive and reached over a unix socket, keeps per-test results, retries and
`-E` filtering, and costs about one boot per module. It also showed a crashed
owner left its VMM running until the run ended.

## Decision

A test that opts in shares one guest with the other opted-in tests of the same
module, group and profile in the same test binary. The first arrival elects
one owner process under an exclusive lock and the owner boots the guest. Later
tests attach to that owner over a unix socket in the run's own runtime
directory. The owner runs as the init of its own PID namespace, so the VMM
dies with the owner, and it exits when the nextest process that anchors the run
is gone. Run teardown stays the backstop.

## Alternatives

- **A nextest setup script boots the guest**: it does not know the test
  binary, has no teardown hook, and would need the same socket and reaper;
  rejected.
- **One nextest test runs the whole module in one guest**: loses per-test
  results, `-E` filtering and retries; rejected (probe control).
- **The first test process owns the guest while it runs**: the guest dies
  when that test ends; rejected.
- **A parent-death signal on the VMM**: needs a change in the production
  launcher, which a test need must not shape; rejected.

## Consequences

One extra process kind (the owner), prepared by `guest-kit-prepare`. The
owner-death guarantee rests on a kernel behaviour that is read in the vendored
Linux source and proven on Metal before acceptance. If it fails, this decision
returns to DESIGN. The protocol is specified in `specs/quint/shared-guest-owner/`.

## Links

- [Interface contract](../../feature/metal-test-placement-attribute/feature-delta.md#wave-design--ref-interface-contracts)
- [Formal protocol model](../../../specs/quint/shared-guest-owner/README.md)
- [Shared-guest probe findings](../../feature/metal-test-placement-attribute/spike/findings-module-shared-guest.md)
- [ADR-0280](adr-0280-metal-nextest-execution-uses-explicit-test-placement.md)
