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
Under this decision five lanes turn red until moved: the CI default nextest job
(13 guest-placed tests compile without any feature), the CI integration job
(about 614), the CI `bpf-unit` job (23), `cargo xtask ci` (13) and the nightly
workspace mutation run (about 685); the pre-commit, pre-push and Lima inner-loop
lanes turn red only when they fall back to Lima. The 13 feature-free tests can
be re-placed or filtered; the integration lanes cannot be filtered, because
guest-placed and host-pure tests share binaries. The feature delta holds the
inventory and the alternative of running in place, which keeps every lane green
and reports PASS for tests that never ran in a guest.

## Links

- [Interface contract and lane inventory](../../feature/metal-test-placement-attribute/feature-delta.md#wave-design--ref-lifecycle-gate-ownership)
