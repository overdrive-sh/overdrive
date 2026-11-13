# ADR-0285 — A test knows it is inside a guest from the guest kernel command line

## Status

**Proposed — awaiting user approval (DDD-10 in the feature delta).**

## Context

A guest-placed test runs its body only inside a guest; on the host it hands
the test binary to the guest kit. The walking skeleton tells the two apart with
an environment variable the guest attempt sets. A variable can be exported on a
host (a shell, a CI job, a nested harness), and the body would then run on the
host and report PASS, which defeats the placement.

## Decision

The guest kit boots every placed guest with a kernel command line parameter,
and a test is inside a guest exactly when its own kernel's command line carries
that parameter. The host's command line cannot be changed by a process or an
environment.

## Alternatives

- **Keep the environment variable**: spoofable on the host; rejected.
- **Strip the variable in the wrapper**: protects Metal runs only; direct runs
  of the binary stay spoofable; rejected.
- **A marker file in the guest image**: a host path can be created by any
  process; rejected.

## Consequences

One kernel parameter is part of the guest boot contract, and the guest must
pass the parameter through to its tests' view of the command line. A guest
launched from inside a guest (a nested group) sets the parameter for its own
kernel only.

## Links

- [Interface contract](../../feature/metal-test-placement-attribute/feature-delta.md#wave-design--ref-interface-contracts)
