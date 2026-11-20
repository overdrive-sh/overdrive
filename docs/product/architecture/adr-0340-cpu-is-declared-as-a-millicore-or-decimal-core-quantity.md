# ADR-0340 — CPU is declared as a millicore or decimal-core quantity

## Status

**Accepted** (2026-11-20). The Kubernetes-style quantity is the user's ruling R-3 (GH #372); the pinned details (resolution, canonical form, case, zero, bound) work it out within that ruling.

## Context

`[resources] cpu_milli = 500` encodes the unit in the key name and not in the value: `0.5` and `500` are indistinguishable at a glance, and
`2` (two cores) reads as two millicores. Operators who know the Kubernetes spelling write `500m` and `0.5`.

## Decision

**`cpu` is a string in a Kubernetes-style quantity grammar (millicores `500m`, whole cores `2`, decimal cores `0.5`), owned by one
`overdrive-core` type, `CpuQuantity`, at millicore resolution. A value finer than a millicore is refused, not rounded; zero is accepted (the
scheduler already treats a zero request as no reservation and the guest still gets one vCPU, today's behaviour). Hand-built, with no new
dependency.** The exact grammar, bound, canonical form and error type are pinned in the
[feature delta](../../feature/workload-resources-human-readable/feature-delta.md) § Interface contracts.

## Alternatives considered

- **Millicore suffix or whole cores only (`500m`, `2`).** Rejected by the ruling in favour of decimal cores, which operators from
  Kubernetes expect.
- **Decimal cores only (`0.5`).** Rejected: no way to spell a small request without a leading `0.00`.
- **A `cores` suffix.** Rejected: a third spelling of whole cores for no new meaning.
- **Case-insensitive `m`.** Rejected: `M` is mega in the convention this grammar follows, so accepting it would silently give it another meaning.
- **Refuse zero.** Rejected: it adds a refusal the spec does not have today and the scheduler already handles a zero request.
- **`k8s-openapi` / `kube_quantity`.** Rejected by the ruling: a large dependency for a three-form grammar.

## Consequences

- Positive: the unit is in the value; `describe` and the API show one spelling.
- Negative: finer-than-millicore values that Kubernetes would round are refused here.
- No scope limitation is introduced.

## Links

- [Feature delta](../../feature/workload-resources-human-readable/feature-delta.md)
- [ADR-0339](adr-0339-memory-and-volume-sizes-share-one-binary-byte-size-grammar.md)
- GH #372
