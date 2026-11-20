# ADR-0341 — Workload resources carry the typed quantities from spec to driver

## Status

**Accepted** (2026-11-20). The user approved typed quantities end to end (decision 1a), node capacity carrying the same types (R-2), and no
envelope version bump (decision 2).

## Context

The `[resources]` values cross five layers: the parsed spec, the validated intent aggregate, the driver's `Resources`, the scheduler, and the
cgroup and VM derivation. The pair `cpu_milli: u32` and `memory_bytes: u64` is a measurement carried as raw primitives, with the unit in the field
name. `.claude/rules/rust.md` treats a measurement as a domain-bearing value that carries its type.

## Decision

**`CpuQuantity` and `ByteSize` (ADR-0339, ADR-0340) are carried end to end in resource fields named `cpu` and `memory`: the spec, the intent
aggregate, node capacity, the driver's resources and the API types. The primitive is unwrapped only where a foreign protocol needs a number or
the scheduler does arithmetic. `cpu_milli` and `memory_bytes` exist nowhere.** The affected types and signatures are pinned in the
[feature delta](../../feature/workload-resources-human-readable/feature-delta.md) § Interface contracts.

## Alternatives considered

- **Keep `Resources` as `u32`/`u64` and convert at the spec edge.** Rejected: leaves the measurement untyped for every internal consumer, and
  two spellings of a field (`cpu_milli` inside, `cpu` outside) are the drift this feature removes.
- **Bump the workload-intent and service-spec envelopes anyway.** Rejected: the byte layout does not change, so a new variant would add a
  frozen copy of every payload that no stored bytes need.
- **Make `Resources.memory` a non-zero type.** Rejected: free capacity and a node with no free memory are legitimate zero values in the
  scheduler; non-zero is an admission rule, as today.
- **Delete `ResourceEnvelope` in favour of `Resources`.** Rejected as unrequested: it keeps `TransitionReason` independent of the driver trait
  surface; only its field types and names change.

## Consequences

- Positive: one set of types from the operator's spelling to the scheduler; the unit cannot be dropped or misread internally.
- Negative: a mechanical rename across fixtures and projections.
- The archived layout of a `CpuQuantity` (`u32`) and a `ByteSize` (`u64`) is the layout of the primitives they replace (spike V-1), so old
  archives decode unchanged, an unchanged spec keeps its digest, and no envelope version is added. A reader expecting a bump will find none;
  the golden fixtures' untouched hex literals are the evidence.

## Links

- [Feature delta](../../feature/workload-resources-human-readable/feature-delta.md)
- [ADR-0339](adr-0339-memory-and-volume-sizes-share-one-binary-byte-size-grammar.md), [ADR-0340](adr-0340-cpu-is-declared-as-a-millicore-or-decimal-core-quantity.md)
- GH #372
