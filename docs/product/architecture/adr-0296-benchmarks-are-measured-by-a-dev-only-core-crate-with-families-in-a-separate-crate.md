# ADR-0296 — Benchmarks are measured by a dev-only core crate, with the families in a separate crate

## Status

**Accepted.** Approved by the user 2026-11-17; independent design review resolved 2026-11-17. DDD-1 in the feature delta.

## Context

Ruling 4 (2026-11-16) puts the measurement core in a new dev-dependency-only
`adapter-host` crate shaped like `overdrive-testing`. The measurement core
(schedule, statistics, record, report, host eligibility) is pure or kernel-light.
The families (connect-hook, VM lifecycle, DNS, mTLS) pull in the production
composition: control plane, worker, dataplane, the CLI library and a VM image
fixture. A `spike-scratch/` probe must depend on the core by path without
compiling any of that.

## Decision

Two new workspace crates, both `crate_class = "adapter-host"`, both
dev-dependency-only (never in the `[dependencies]` of a production crate):

- `overdrive-bench`: the measurement core, the runner entry, the macro runtime
  support and the `bench-report` tool binary.
- `overdrive-bench-families`: the family declarations and fixtures, with one
  `harness = false` bench target. It drives the production composition in
  process through `overdrive-control-plane`, `overdrive-worker`,
  `overdrive-dataplane` and the operator driving port in the `overdrive-cli`
  library; it never starts the `overdrive` binary.

A spike crate depends on `overdrive-bench` (and `overdrive-testing`) by path.

## Alternatives

- **One crate holding core and families**: every spike would compile the control
  plane and the VM stack, and the core could not be unit-tested without them;
  rejected.
- **Families in the benchmarked crates' own `benches/`**: scatters the catalogue,
  the mTLS family has no single owner, the Metal form needs one package to name,
  and a `harness = false` target cannot hold the smoke test; rejected.
- **Put the core into `overdrive-testing`**: that crate holds real-OS fixtures,
  not measurement, and ruling 4 asks for a new crate; rejected.

## Consequences

`overdrive-bench-families` needs a key-encryption-key adapter to boot
`run_server`; the hermetic one is in `overdrive-sim`, so the crate takes
`overdrive-sim` (`SimKek`) as a dependency in its `[dependencies]`, which
`development.md` keeps out of production crates; the user approved this for the
dev-only families crate (2026-11-17). The families crate is not reachable from
production, in the same standing as `overdrive-testing`. The rejected alternative
is a benchmark-owned in-memory key adapter, a second sim-like adapter to maintain.

## Links

- [Decision DDD-1](../../feature/native-benchmark-program/feature-delta.md#ddd-1--two-dev-only-adapter-host-crates)
