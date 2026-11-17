# ADR-0299 — Benchmark substrates are a closed registry and the first entry is `native`

## Status

**Accepted.** Approved by the user 2026-11-17; independent design review resolved 2026-11-17. DDD-4 in the feature delta.

## Context

Ruling 6: each family declares its substrate the way the placement macros do, the
result records it, and results from different substrates are never compared. The
placement registry (ADR-0294) lists guest groups, which are test environments.
Research Knowledge Gap 4 records no guidance on benchmarks inside a nested guest,
and `testing.md` names the native x86_64 host as the only signal.

## Decision

`overdrive-bench` holds a closed registry of substrates with one entry, `native`
(the exclusive Metal host, no namespace). The attribute takes it as a bare
identifier, `substrate = native`, as the placement kinds are (`host_pure`); a string
literal is a build error, since a string is the form of a guest group name. It
validates the identifier by constant evaluation, so an unknown value is a build error naming it (the same pattern as
ADR-0294). The substrate is written to every record. No tool compares two records, so
results from different substrates are never compared. A `guest` substrate is not in the
registry: a benchmark that declares one fails to build with a message that says
guest substrates are not available.

## Alternatives

- **Accept `guest = "<group>"` now**: needs a runner inside a guest and a
  position on nested-virtualization noise nobody has measured; rejected for this
  feature.
- **Add `native` to the guest-group table**: mixes two vocabularies in one
  registry; rejected.

## Consequences

Guest-substrate benchmarks are a scope limitation of this feature, tracked by
[#362](https://github.com/overdrive-sh/overdrive/issues/362).

## Links

- [Decision DDD-4](../../feature/native-benchmark-program/feature-delta.md#ddd-4--substrate-registry)
