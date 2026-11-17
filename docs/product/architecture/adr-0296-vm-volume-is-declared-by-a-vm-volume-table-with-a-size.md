# ADR-0296 — A VM workload declares its volume with a `[vm.volume]` table carrying a size

## Status

**Proposed** (decision VD-1 of feature `node-local-persistent-volume`, GH #348).
Not accepted until the user approves it in the conversation. Nothing here is
implementation authority.

## Context

A VM workload has no persistent storage: `[vm]` is `command`, `args`, `kernel`,
`rootfs`, and the guest rootfs is a disposable per-launch clone. GH #348 asks for
a node-local volume the workload declares with a size. The spec is immutable per
workload id (a different body under the same id is a conflict, not an update), so
the declaration must be part of the spec that is hashed and stored.

## Decision

**A VM-driver workload (Job or Service) declares at most one volume with a
`[vm.volume]` sub-table of `[vm]` whose only key is `size`: a whole number of at
least 1 followed by `GiB` or `TiB`. A workload that declares a volume must have
`replicas = 1`. The declaration is part of the immutable workload spec.**

## Alternatives considered

- **`[[vm.volume]]` array / several volumes.** Rejected: nothing in #348 or the
  motivating dev-box needs two, and several volumes multiply identity (names),
  guest disk ordering, serial uniqueness and per-volume lifecycle. A named form can
  be added later without changing the single-volume spelling.
- **A flat `volume_size` key on `[vm]`.** Rejected: cannot grow options (tier,
  name) without a second spelling.
- **Size as a raw byte integer, like `memory_bytes`.** Rejected: tens-of-GB sizes
  as byte counts are error-prone to read and write; the validated unit string
  parses to one canonical byte count in the aggregate.
- **A `MiB` unit and a byte-granule rule.** Rejected: every `GiB` multiple is a whole
  number of extents for any power-of-two extent size up to 1 GiB, so the grammar
  alone guarantees the declared size is the allocated size.
- **Allow `replicas > 1` with one volume.** Rejected: the volume has exactly one
  writer; a second replica's VMM would fail to open it. Admission refuses the
  combination.

## Consequences

- Positive: one obvious spelling; validation happens at admission, before any node
  work.
- Negative: the first field added to `Vm` bumps the workload-intent rkyv envelope
  (ADR-0048 discipline) and the parser-side `ServiceSpec` envelope.
- Scope limitations (one volume per workload; size fixed at creation) are tracked in
  the feature delta § "Scope limitations".

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Spec surface (VD-1)
- [ADR-0297](adr-0297-a-volume-belongs-to-its-workload-and-outlives-stop-and-restart.md)
- GH #348
