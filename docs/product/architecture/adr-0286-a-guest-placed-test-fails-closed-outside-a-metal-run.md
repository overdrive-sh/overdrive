# ADR-0286 — A guest-placed test fails closed outside a Metal run

## Status

**Proposed — awaiting user approval (DDD-11 in the feature delta).** It
changes what the off-Metal lanes do with guest-placed tests; see the feature
delta for the lanes affected.

## Context

With no Metal run context the wrapper hands the test binary to the configured
runner unchanged, so today a guest-placed test run in Lima, in the CI
integration job or in the pre-push hook runs its body on that host. After a
test declares its guest placement on itself, running its body in place would
report a PASS that never ran in a guest. nextest has no runtime skip: a test
either passes, fails or is ignored at compile time.

## Decision

Outside a Metal run context a guest-placed test fails with a typed error that
names the test, its group and the missing Metal context. It does not run its
body and does not report PASS.

## Alternatives

- **Run the body in place outside Metal** (today's behaviour): reports a
  placed test as passing without its isolation; rejected for guest placements.
- **Skip with a visible reason**: nextest cannot report a runtime skip, so a
  skip is a PASS; rejected.

## Consequences

Every lane that runs a guest-placed test must run it on Metal, or exclude it.
The lanes that run these binaries today are listed in the feature delta.

## Links

- [Interface contract and lane inventory](../../feature/metal-test-placement-attribute/feature-delta.md#wave-design--ref-lifecycle-gate-ownership)
