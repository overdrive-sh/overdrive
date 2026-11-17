# ADR-0298 — Declared benchmarks register at link time in a distributed slice

## Status

**Accepted.** Approved by the user 2026-11-17; independent design review resolved 2026-11-17. DDD-3 in the feature delta.

## Context

The runner `main` in a `harness = false` target must find every declared
benchmark of a family without a hand-kept list, so adding a family touches no
runner code (KPI K1). The workspace builds the bench profile with thin LTO and one
codegen unit.

## Decision

The macro registers each declaration in a `linkme` distributed slice owned by
`overdrive-bench`. The runner enumerates the slice, selects by family, and runs
the benchmarks of that family in declaration-name order. Spike V-5 confirmed
collection under the bench profile (thin LTO, one codegen unit, `linkme` 0.3.38).
rustc drops a dependency crate that nothing references, and its entries vanish with
no error. So the bench `main` of the families crate, and the `main` of any spike
crate, names the declaring crate (`use <declaring_crate> as _;`), and the runner
refuses a `--family` that no registered benchmark declares, naming the families it
found.

## Alternatives

- **`inventory`**: registers through life-before-main constructors; a hidden
  execution phase for no gain over a link-section slice; rejected.
- **An explicit list in the bench target**: every new family edits the runner
  entry, contradicting K1; rejected (V-5 held, so the fallback is not needed).
- **A custom ELF section read back from `/proc/self/exe`**: reimplements what
  `linkme` already does; rejected.

## Consequences

One new dependency, `linkme` (MIT OR Apache-2.0, maintained by dtolnay). The
slice is visible in both the bench binary and the lib's unit-test binary, which is
what the smoke test needs. The reference to the declaring crate is a one-line
obligation on each bench `main`; the undeclared-family refusal turns its omission
from a silent drop into a refusal. A hard-coded family count is not used: it would
change with every new family, against K1.

## Links

- [Decision DDD-3](../../feature/native-benchmark-program/feature-delta.md#ddd-3--link-time-registration)
