# ADR-0343 — A workload memory is a whole number of MiB

## Status

**Accepted** (2026-11-20; user decision 5a, GH #372). Not implemented; stated in the present tense.

## Context

Workload `memory` is guest RAM: the VM is the only workload driver. Cloud Hypervisor refuses a guest RAM size that is not a multiple of the
host page size, and today a value such as `memory_bytes = 1000` passes `deploy` (only zero is refused) and fails later, at VM launch. The
evidence is in the feature delta § Cloud Hypervisor memory constraint. The shared `ByteSize` grammar (ADR-0339) deliberately admits any byte
count, so the rule belongs on the workload, not on the grammar.

## Decision

**`overdrive deploy` refuses a workload `memory` that is zero or not a whole number of MiB, at the aggregate admission site that already
refuses zero memory. The `ByteSize` grammar is unchanged; the rule is a check, not a type.**

## Alternatives considered

- **Refuse only at VM launch (today).** Rejected: the operator learns of a wrong size from a failed launch, not from `deploy`.
- **Round to the nearest page or MiB.** Rejected: silently a different size, the error class this feature removes.
- **Check the host page size.** Rejected: the admitting node is not the launching node, and whole MiB covers 4 KiB, 16 KiB and 64 KiB pages
  with no guest needing sub-MiB RAM.
- **A MiB-granular memory newtype.** Rejected: one caller pair and nothing downstream needs the guarantee in its type.

## Consequences

- Positive: a misaligned size fails at `deploy` with the rule named; every admitted workload memory is launchable.
- Negative: sub-MiB workload sizes are not expressible; none is useful for a guest.
- Node capacity is not guest RAM and keeps only its non-zero check. The volume size is unaffected (whole GiB already implies whole MiB).
- No scope limitation is introduced.

## Links

- [Feature delta](../../feature/workload-resources-human-readable/feature-delta.md)
- [ADR-0339](adr-0339-memory-and-volume-sizes-share-one-binary-byte-size-grammar.md)
- GH #372
