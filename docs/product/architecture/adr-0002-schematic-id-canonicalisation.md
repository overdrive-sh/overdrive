# ADR-0002 — SchematicId canonicalisation uses rkyv-archived bytes

## Status

Accepted. 2026-04-21.

## Context

A **schematic** is a TOML document (whitepaper §23) whose SHA-256 hash is
its content ID. The hash MUST be deterministic across machines, toolchain
versions, and Rust editions — otherwise two builds of the same schematic
produce different IDs and the content-addressed OCI layout in the Image
Factory breaks silently.

Two defensible choices for the canonicalisation that feeds SHA-256:

- **rkyv-archived bytes** of the deserialised schematic value. rkyv's archival
  format is canonical by construction — field order matches the Rust struct
  definition, no whitespace, no map-key reordering, no float-format variance.
- **RFC 8785 JSON Canonicalisation Scheme (JCS)**: lexicographic key sort,
  specified number formatting, escape-sequence rules. Widely used in JWS,
  TUF, and web-standard attestations.

Both satisfy `.claude/rules/development.md` ("When a hash is used as an
identity… the serialization that feeds the hash MUST be deterministic"). The
*consistency* of the choice is what matters: split choices across the
codebase produce two incompatible hashes for the same schematic.

Relevant properties:

| Property | rkyv | JCS |
|---|---|---|
| External observability | Opaque (schema-bound bytes) | Human-readable |
| External toolchain interop | Rust-only | Any language with JCS |
| Field evolution | Strict additive only (rkyv archive version must be bumped) | Schema-permissive (unknown fields ignored by default) |
| Runtime cost | Zero — the archive already exists | Parse → sort → reformat |
| Fits `development.md` guidance | "Internal data → rkyv" (exact match) | "External / JSON data → RFC 8785" |
| Already in workspace | Yes (`rkyv = "0.8"`) | No (would add a new dep) |
| Interaction with `export_snapshot` | Snapshots are already rkyv-framed (AC: US-03) — schematic ID composes with the same toolchain | Parallel serialisation path |

The schematic is an *internal* Overdrive concept. There is no external
client that needs to compute a SchematicId in a non-Rust toolchain; the
Image Factory is Rust throughout (whitepaper §23). The development-rule
guidance explicitly labels "internal data → rkyv" and "external / JSON data
→ RFC 8785." SchematicId is unambiguously in the first bucket.

## Decision

**SchematicId is the SHA-256 of the rkyv-archived bytes of the schematic
struct.** Concretely:

```rust
// In overdrive-core or a future overdrive-schematic crate:
let archived: AlignedVec = rkyv::to_bytes::<_, 256>(&schematic)?;
let hash = ContentHash::of(&archived);
let id = SchematicId::new(hash);
```

The `Schematic` struct derives `rkyv::Archive + rkyv::Serialize` with
`#[rkyv(check_bytes)]` (via `bytecheck`). Field order is part of the
archive contract; adding a field is a new archive version and a new ID
for the same semantic schematic. Removing or reordering fields is a
breaking change.

## Alternatives considered

### Option A — RFC 8785 JCS over JSON form

Serialise the schematic to JSON with serde_json, canonicalise via a JCS
implementation, hash the canonical bytes. **Rejected.** Adds a new
dependency; parallel serialisation path for something already owned by
rkyv; runs against the `development.md` guidance for internal data; gains
no meaningful interop (the only consumer is the Overdrive Image Factory,
which is Rust throughout).

### Option B — Manual canonical TOML

Reformat the schematic TOML via a bespoke canonicaliser (sorted keys,
specified number formatting, etc.) and hash the resulting bytes.
**Rejected.** No ecosystem canonicalisation spec exists for TOML. Building
one is inventing a new standard for a problem rkyv already solves; it
invites subtle bugs (trailing-newline handling, float rendering, array
vs inline table equivalence) that rkyv sidesteps by construction.

### Option C — rkyv-archived bytes (chosen)

See Decision above.

## Consequences

### Positive

- Deterministic across builds and machines with no additional dependency.
- Zero marginal serialisation cost — if the platform already has the
  archived form (it does, for `StateSnapshot` and future reconciler I/O),
  the hash is a second SHA-256 pass.
- Composes with the snapshot framing (ADR is self-consistent with
  `IntentStore::export_snapshot` returning rkyv-archived bytes).
- Matches `development.md` internal-data guidance exactly.

### Negative

- Opaque to external tooling. A human debugging "why did the schematic
  ID change" cannot inspect the hashed bytes as text; they must
  round-trip through a Rust tool that reads the archived form.
- Schematic schema evolution is strict: field order and presence are part
  of the hash. Additive changes bump the SchematicId for the same
  semantic input. Acceptable — the Image Factory builds a fresh artifact
  per schematic version anyway (whitepaper §23).

### Neutral

- A future decision to expose schematic IDs in a non-Rust ecosystem (e.g.,
  a CLI plugin that authors schematics) would require either a Rust-WASM
  helper or a superseding ADR that adopts JCS. Not in scope now.

## References

- `docs/whitepaper.md` §23 (Image Factory, schematic as content hash)
- `.claude/rules/development.md` (*Hashing requires deterministic
  serialization* — rkyv for internal data)
- `crates/overdrive-core/src/id.rs::{ContentHash, SchematicId}`
- `rkyv` 0.8 docs — archival format stability guarantees
