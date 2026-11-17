# ADR-0297 — A benchmark is declared with an attribute that also emits its smoke test

## Status

**Accepted.** Approved by the user 2026-11-17; independent design review resolved 2026-11-17. DDD-2 in the feature delta.

## Context

Ruling 8 approves an attribute macro reusing the placement registry, with the
smoke variant placed by `#[overdrive_test]`. Ruling 10 delegates the macro crate;
`overdrive-testing-macros` already owns the placement parsing. The placement
attribute never inserts `#[test]` (ADR-0282). `.claude/rules/testing.md` says a
product test is never `native_launcher`, and a benchmark's own substrate is
native.

## Decision

`#[overdrive_bench(...)]` is a new attribute in `overdrive-testing-macros`,
re-exported by `overdrive-bench`. It leaves the function in place, registers it
(ADR-0298), and emits a smoke `#[test]` that runs one replicate per arm per
profile with no statistics and no timing assertion. The smoke test carries its
own placement from a required `smoke` argument that uses the placement
vocabulary and the ADR-0294 registry (`host_pure` or `guest = "<group>"` with an
optional profile), and is gated on `integration-tests`. `smoke(none)` is accepted
only when the crate's manifest directory is under `spike-scratch/`, checked by
constant evaluation. The attribute adds no ELF placement record kind, so the
wrapper reader and `placement-lint` are unchanged. The bench target is not
placed: it runs only under exclusive admission, where the wrapper does not read
placement records (O2).

## Alternatives

- **A new macro crate**: needs its own dev-dependency ruling and duplicates the
  attribute parser; rejected.
- **The smoke test inherits the benchmark's substrate**: would make a product
  test `native_launcher`, which `testing.md` forbids; rejected.
- **Place the bench target through a new record kind**: changes the wrapper
  reader for a target nothing places; rejected.

## Consequences

A maintained benchmark without a smoke test is unrepresentable. A typo in a
declared metric name is found by the smoke test, not by a Metal run.

## Links

- [Decision DDD-2 and the attribute grammar](../../feature/native-benchmark-program/feature-delta.md#ddd-2--the-overdrive_bench-attribute-and-its-smoke-test)
