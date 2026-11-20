# ADR-0310 — A VM workload declares its volume with a `[vm.volume]` table carrying a size

## Status

**Accepted** (decision VD-1 of feature `node-local-persistent-volume`, GH #348).
Approved by the user on 2026-11-17. Implemented by GH #348 DELIVER.
**Amended 2026-11-20** (GH #372): the size grammar is replaced; see § Amendment 2026-11-20.

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
- Scope limitations: one volume per workload is tracked by GH #364, size fixed at creation by GH #365
  (feature delta § "Scope limitations and exposures").

## Amendment 2026-11-20 — the size is a `ByteSize` refined to whole GiB (GH #372)

**Approval.** User decision R-1 (one shared byte-size grammar for `memory` and the volume size) and the rebase ruling (option a, keep R-1
after GH #348 DELIVER landed), both 2026-11-20. The contract above stands as accepted and implemented until this feature's DELIVER lands;
this amendment replaces its size grammar from that point.

**Amended decision.** The `[vm.volume] size` value is a byte size in the shared grammar of
[ADR-0339](adr-0339-memory-and-volume-sizes-share-one-binary-byte-size-grammar.md) (`ByteSize`), refined to a whole number of GiB of at
least 1 GiB. Everything else in the Decision (one volume, only key `size`, `replicas = 1`, part of the immutable spec) is unchanged.

| Spelling | Accepted contract (shipped) | Amended contract |
|---|---|---|
| `40GiB`, `2TiB` | accepted | accepted |
| `1024MiB`, `1073741824B` | refused (unit must be `GiB` or `TiB`) | accepted, the same size as `1GiB` |
| `512MiB`, `1500MiB`, `0GiB` | refused | refused (not a whole number of GiB, or zero) |
| `1.5GiB`, `1GB`, `40`, padded text | refused | refused (ADR-0339: binary units, integers, a unit required) |

**Why the amendment is sound.** The reason for the accepted grammar's restriction is unchanged: a whole number of GiB is a whole number of
extents for any power-of-two extent size up to 1 GiB, so the declared size is the allocated size. The refinement keeps exactly that guarantee
(the "`MiB` unit and a byte-granule rule" alternative lost to the same reasoning, and still does: no sub-GiB size is admitted). The
alternative the accepted text did not weigh is a volume-only `GiB`/`TiB` grammar beside the workload `memory` grammar; it is rejected because
two grammars for one kind of quantity drift (ADR-0339).

**Display and wire.** The shipped `VolumeSize` `Display` emits `<n>TiB` for an exact TiB multiple and `<n>GiB` otherwise
(`crates/overdrive-core/src/volume.rs`). After the amendment `Display` is the ADR-0339 canonical form (the largest binary unit that divides
the value exactly). For every value the refinement admits (a multiple of 1 GiB) that is the same text, so `Display`, the serde string form
and the JSON wire form are byte-for-byte unchanged for every size the shipped parser accepted. Only the set of accepted inputs grows.

**Compatibility.** `VolumeSize` is a single-field `u64` newtype that derives `rkyv::Archive`, and it is archived inside the workload-intent V2
payload (`VmVolume { size }`, ADR-0048). The amendment keeps the archived value a `u64` of bytes, so the archived layout is unchanged:
no envelope version is added, and the V2 golden fixtures (`FIXTURE_V2` and the service fixtures in
`crates/overdrive-core/tests/schema_evolution/workload_intent.rs`, `service_spec.rs`) must decode unchanged with their hex literals
untouched. A single-field newtype archives as its inner primitive (spike V-1,
`docs/feature/workload-resources-human-readable/spike/findings-v1-newtype-layout.md`, held for `ByteSize(u64)`); if a V2 fixture fails to
decode after the change, the layout claim is false and DELIVER stops and surfaces it to the user. The `spec_digest` of a stored spec with a
volume is unchanged, and two spellings of one size produce identical bytes because the spelling is not stored. No data migration.

**What changes in code (owned by GH #372 DELIVER).** `VolumeSize::from_str` parses a `ByteSize` and refuses zero or a non-GiB multiple;
its `Display` delegates to the `ByteSize` canonical form. The exact contract is in the
[feature delta](../../feature/workload-resources-human-readable/feature-delta.md) § Volume refinement.
Tests of GH #348 whose expectations change are listed there under DELIVER obligations. `ByteCount` (free space, `<n>B`) is unchanged.

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Spec surface (VD-1)
- [ADR-0339](adr-0339-memory-and-volume-sizes-share-one-binary-byte-size-grammar.md) (the shared byte-size grammar; the amendment above)
- [Feature delta of GH #372](../../feature/workload-resources-human-readable/feature-delta.md)
- [ADR-0311](adr-0311-a-volume-belongs-to-its-workload-and-outlives-stop-and-restart.md)
- GH #348, GH #372
