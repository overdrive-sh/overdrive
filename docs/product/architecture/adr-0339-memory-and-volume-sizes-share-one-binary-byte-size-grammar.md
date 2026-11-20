# ADR-0339 — Memory and volume sizes share one binary byte-size grammar

## Status

**Accepted** (2026-11-20; R-1 ruling, GH #372). Not implemented; stated in the present tense.

## Context

A workload declares `[resources] memory` and, with GH #348, `[vm.volume] size`. Both are byte quantities an operator types. Memory is a raw
integer today (`memory_bytes = 134217728`), which makes a dropped digit a valid, wrong size that `describe` cannot reveal. The volume size
(ADR-0310) was a `GiB`/`TiB`-only grammar with its own `ByteCount` for free space (`B` only). Three spellings of one kind of quantity would drift.

## Decision

**One byte-size grammar, `<uint><unit>`, owned by one `overdrive-core` type, `ByteSize`. `memory` is a `ByteSize`; the volume size is a
`ByteSize` refined to a whole number of GiB of at least 1 GiB (ADR-0310). The grammar is hand-built, with no new dependency.**

Units are binary only and integers only: a silent 2.4% difference from a decimal unit is the error class this decision removes, so decimal
units are refused. Equal sizes have one canonical spelling (the largest binary unit that divides the value exactly), so
`parse(display(x)) == x`. The exact grammar, error type and accessors are pinned in the
[feature delta](../../feature/workload-resources-human-readable/feature-delta.md) § Interface contracts. Workload memory adds its own
refinement on top of this grammar: [ADR-0343](adr-0343-a-workload-memory-is-a-whole-number-of-mib.md).

## Alternatives considered

- **A separate `MemorySize` newtype beside `VolumeSize`.** Rejected: two grammars for one quantity; they would diverge on the first unit added.
- **Extend `VolumeSize` itself to a general size.** Rejected: `VolumeSize` carries the volume's own refinement (whole GiB, extent alignment,
  ADR-0312); memory needs no such refinement and zero must stay representable for free space and capacity.
- **A published crate (`bytesize`, `byte-unit`).** Rejected by the user's ruling: a dependency for a five-unit grammar.
- **Accept fractions (`1.5GiB`).** Rejected: a fraction of a unit is not always a whole number of bytes, and the canonical form would need
  rounding rules. `1536MiB` says the same thing exactly.
- **Accept a bare number as bytes.** Rejected: it reintroduces the unit-less value.

## Consequences

- Positive: one spelling, one canonical form, one set of grammar tests for memory and volume; the volume accepts the same spellings as
  memory (`1024MiB` is a valid volume size, canonical `1GiB`).
- Negative: ADR-0310's unit decision changes (amended 2026-11-20 in ADR-0310, because GH #348 had already shipped it). `ByteCount` (free space, `<n>B`) is a
  strict subset of this grammar and is left as it is.
- No scope limitation is introduced.

## Links

- [Feature delta](../../feature/workload-resources-human-readable/feature-delta.md)
- [ADR-0310](adr-0310-vm-volume-is-declared-by-a-vm-volume-table-with-a-size.md), [ADR-0343](adr-0343-a-workload-memory-is-a-whole-number-of-mib.md)
- GH #372, GH #348
