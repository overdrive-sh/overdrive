# ADR-0310 — A VM workload declares its volume with a `[vm.volume]` table carrying a size

## Status

**Accepted** (decision VD-1 of feature `node-local-persistent-volume`, GH #348).
Approved by the user on 2026-11-17.

## Context

A VM workload has no persistent storage: `[vm]` is `command`, `args`, `kernel`,
`rootfs`, and the guest rootfs is a disposable per-launch clone. GH #348 asks for
a node-local volume the workload declares with a size. The spec is immutable per
workload id (a different body under the same id is a conflict, not an update), so
the declaration must be part of the spec that is hashed and stored.

## Decision

**A VM-driver workload (Job or Service) declares at most one volume with a
`[vm.volume]` sub-table of `[vm]` whose only key is `size`: a byte size in the
shared grammar of ADR-0339, refined to a whole number of GiB of at least 1 GiB
(`40GiB`, `2TiB` and `1024MiB` are accepted; `512MiB`, `1500MiB` and `0GiB` are
refused). A workload that declares a volume must have `replicas = 1`. The
declaration is part of the immutable workload spec.**

## Alternatives considered

- **`[[vm.volume]]` array / several volumes.** Rejected: nothing in #348 or the
  motivating dev-box needs two, and several volumes multiply identity (names),
  guest disk ordering, serial uniqueness and per-volume lifecycle. A named form can
  be added later without changing the single-volume spelling.
- **A flat `volume_size` key on `[vm]`.** Rejected: cannot grow options (tier,
  name) without a second spelling.
- **Size as a raw byte integer.** Rejected: tens-of-GB sizes as byte counts are
  error-prone to read and write; the validated unit string parses to one canonical
  byte count in the aggregate.
- **A volume-only grammar limited to the `GiB` and `TiB` units.** Rejected: the
  workload `memory` is declared in the same units, and two grammars for one kind of
  quantity drift. The shared grammar plus a value refinement gives the same
  guarantee: every whole number of GiB is a whole number of extents for any
  power-of-two extent size up to 1 GiB, so the refinement alone guarantees the
  declared size is the allocated size.
- **Allow `replicas > 1` with one volume.** Rejected: the volume has exactly one
  writer; a second replica's VMM would fail to open it. Admission refuses the
  combination.

## Consequences

- Positive: one obvious spelling; validation happens at admission, before any node
  work.
- Negative: the first field added to `Vm` bumps the workload-intent rkyv envelope
  (ADR-0048 discipline) and the parser-side `ServiceSpec` envelope.
- Scope limitations: one volume per workload is tracked by GH #364, size fixed at creation by GH #365
  (feature delta § "Scope limitations and exposures").

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Spec surface (VD-1)
- [ADR-0339](adr-0339-memory-and-volume-sizes-share-one-binary-byte-size-grammar.md) (the shared byte-size grammar)
- [ADR-0311](adr-0311-a-volume-belongs-to-its-workload-and-outlives-stop-and-restart.md)
- GH #348
