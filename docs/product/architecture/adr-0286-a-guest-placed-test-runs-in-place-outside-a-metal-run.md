# ADR-0286 — A guest-placed test runs in place outside a Metal run

## Status

**Accepted — approved by the user, 2026-11-14 (U-2, DDD-11 in the feature
delta).** Independent DESIGN review is pending.

## Context

Guest placement exists to isolate tests from each other on the shared Metal
box. Outside a Metal run the machine is private (CI runners, Lima, the
pre-commit and pre-push hooks on Lima, `cargo xtask ci`, mutation runs), so
there is nothing to isolate from. Today the wrapper hands the test binary to the
configured runner unchanged when there is no Metal run context, so a
guest-placed test's body runs in the current process and passes. This feature
changes where placement is declared, not where tests run. Tests that need
bare-metal KVM stay gated by the `kvm-tests` feature, which no off-Metal lane
enables.

## Decision

A test's placement is applied only inside a Metal run. A Metal run context is
present when any of `OVERDRIVE_METAL_RUN_ID`, `OVERDRIVE_METAL_EXECUTION_CLASS`
or `OVERDRIVE_METAL_SOURCE_ROOT` is non-empty in the test process's
environment: the predicate the wrapper already uses to decide it has nothing to
do. With no Metal run context, a guest-placed or lane-placed test runs its body
directly in the current process, exactly as it does today. Inside a guest
(`overdrive.test_guest=1`, ADR-0285) the body runs as before. On a Metal run a
guest-placed test never runs its body on the host: the wrapper places it, and
the test hands itself to the guest, or fails with a typed error.

## Alternatives

- **Fail closed outside Metal** (a typed `NoMetalContext` failure, the body
  never runs): turns every off-Metal lane that runs a guest-placed test red.
  The test counts, from the lane inventory in the feature delta, are the CI
  default nextest job (13), the CI integration job (about 614), the CI
  `bpf-unit` job (23), `cargo xtask ci` (13) and the nightly workspace
  mutation run (about 685); about 685 guest-placed tests pass in CI today and
  would fail. It also needs a Metal runner path for CI that nobody has asked
  for, to protect against a failure (cross-test interference) that cannot occur
  on a private machine; rejected.
- **Skip with a visible reason**: nextest cannot report a runtime skip, so a
  skip is a PASS; rejected.

## Consequences

No off-Metal lane changes. A PASS off Metal means the body ran in the current
process on a private machine, which is what it means today; only a Metal run
proves the guest isolation. The expansion of a guest-placed test and the lane
inventory are pinned in the feature delta.

## Links

- [Interface contract and lane inventory](../../feature/metal-test-placement-attribute/feature-delta.md#wave-design--ref-lifecycle-gate-ownership)
- [ADR-0285](adr-0285-a-test-knows-it-is-inside-a-guest-from-the-guest-kernel-command-line.md)
