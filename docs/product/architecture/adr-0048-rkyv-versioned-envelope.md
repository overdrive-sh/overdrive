# ADR-0048: rkyv versioned envelope for persisted types

## Status

Accepted (2026-05-12). Revised 2026-05-12 in response to peer review
(see § "Review-revision log" in
`docs/feature/rkyv-envelope-evolution/design/wave-decisions.md`).

## Context

Five rkyv-archive boundaries persist data via redb. Enumeration verified
complete on 2026-05-12 by exhaustive grep across `crates/` for
`redb::TableDefinition` constants (5 hits — 4 observation tables in
`crates/overdrive-store-local/src/observation_backend.rs:74-94`, 1 intent
table in `crates/overdrive-store-local/src/redb_backend.rs:64`) plus
all `rkyv::Archive` derive sites whose values reach a redb `value: &[u8]`
write path:

| Boundary | Type | Layer |
|---|---|---|
| `observation_alloc_status` | `AllocStatusRow` | Observation |
| `observation_node_health` | `NodeHealthRow` | Observation |
| `observation_service_hydration_results` | `ServiceHydrationResultRow` | Observation |
| `observation_service_backends` | `ServiceBackendRow` | Observation |
| intent aggregate (`entries` table) | `Job` (embeds `WorkloadDriver`, `Exec`) | Intent |

**Out-of-band rkyv use sites confirmed NOT to be redb-persisted
boundaries**:

- `crates/overdrive-store-local/src/snapshot_frame.rs` — IntentStore
  full-state export. Uses an explicit `magic = b"OSNP"` + `u16 version`
  frame header with its own `FrameError::UnknownVersion` evolution
  mechanism (frame v1 only as of Phase 1; ADR-0020 supersedes the v2
  experiment). Not a redb value — produced by `IntentStore::
  export_snapshot` and consumed by `bootstrap_from`.
- `crates/overdrive-worker/src/cgroup_manager.rs` — `rkyv::Archive`
  derive marked "deferred to durable boundary (Phase 1 transient)" in
  source; not persisted.
- `crates/overdrive-core/src/dataplane/fingerprint.rs::FingerprintInput`
  — hash input only (rkyv-canonicalised bytes feed SHA-256); never
  persisted.
- Newtypes (`BackendKey`, `MaglevTableSize`, `TransitionReason`, etc.)
  with field-level `rkyv::Archive` derives — components of the five
  row types enumerated above; their evolution is governed transitively
  by their containing envelope.

Each derives `rkyv::{Archive, Serialize, Deserialize}` on a plain
struct or enum. Three docstrings on `AllocStatusRow`
(`crates/overdrive-core/src/traits/observation_store.rs` lines
278–282, 311–313, 359–361) claim rkyv has "additive-field tolerance"
— that `Option<T>` fields appended to a struct deserialise from
older archives.

This is false. rkyv archives are **fixed positional layouts**. Adding
a field shifts every subsequent field's offset; the validator
(`rkyv-0.8.15/src/validation/archive/validator.rs:47-56`) rejects
pre-existing bytes at read time with a `subtree pointer overran
range` error.

The failure surfaced 2026-05-12 as
`WARN convergence tick error e=ObservationRead("...subtree pointer
overran range...")` after the `WorkloadKind` discriminator
(commit `6ffa9270`) and `listeners: Vec<ListenerRow>` (commit
`e7b40282`) were appended to `AllocStatusRow` against existing redb
files.

ADR-0035 and ADR-0036 established a CBOR / serde-versioning envelope
on the View / ViewStore side. That envelope is correct *for CBOR*
(ignore-unknown-fields plus `#[serde(default)]` carry additive
evolution); rkyv's layout semantics do not permit the same shape.

## Decision

Every rkyv-persisted type at a redb persistence boundary is wrapped
in a per-type **versioned envelope enum**. Writers go through a
`VersionedEnvelope::latest()` constructor. Readers up-convert to
`Latest` via `into_latest()`. Schema bumps add a new variant + a new
`From<VN> for VN+1` impl, and the prior version's golden bytes
continue to decode.

### 1. Envelope shape — per-type rkyv enum

```rust
// overdrive-core::codec::envelope
pub trait VersionedEnvelope: rkyv::Archive + rkyv::Serialize<...> {
    type Latest;
    fn latest(payload: Self::Latest) -> Self;
    fn into_latest(self) -> Result<Self::Latest, EnvelopeError>;
}

pub enum EnvelopeError {
    UnknownVersion { observed: u8, supported_max: u8 },
    Malformed { source: rkyv::rancor::Error },
}
```

Per-type shape (one example; same pattern for each of the five):

```rust
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, ...)]
pub enum AllocStatusRowEnvelope {
    V1(AllocStatusRowV1),
    V2(AllocStatusRowV2),
}
pub type AllocStatusRow       = AllocStatusRowEnvelope;   // exported as "the row"
pub type AllocStatusRowLatest = AllocStatusRowV2;
```

Generic `Envelope<T>` was rejected — see Alternatives below.

**Why a per-type rkyv enum is forward-compatible across variant
additions.** Two independent sources of confidence:

1. **rkyv 0.8 source semantics.** rkyv-0.8.15's derive emits a
   `#[repr(u8)]` tag for the enum discriminant followed by per-variant
   `#[repr(C)]` payload structs of the shape `(Tag, payload_fields...)`.
   The canonical reference shape is visible in the stdlib `Result<T,E>`
   impl (
   `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rkyv-0.8.15/src/impls/core/result.rs:11-26`
   — `#[repr(u8)] enum ArchivedResultTag { Ok, Err }`, then
   `#[repr(C)] struct ArchivedResultVariantOk<T>(ArchivedResultTag, T)`).
   Dispatch is by tag-byte value at offset 0; the payload layout is
   per-variant. **Appending a new variant `V<N+1>` at the end of an
   envelope enum allocates a new tag value (N), does NOT shift the
   discriminant for `V1..V<N>`, and does NOT change the archived
   layout of any existing variant's payload.** Old bytes (tag = V1,
   payload V1) continue to decode through the new envelope; the
   validator branches on tag and finds the V1 layout unchanged.

2. **In-repo precedent — `ServiceHydrationStatus`.**
   `crates/overdrive-core/src/traits/observation_store.rs:415-447`
   declares an rkyv-archived enum
   (`#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
   pub enum ServiceHydrationStatus { Pending, Completed {...}, Failed
   {...} }`) and its docstring (lines 412-414) states *"Variant
   ordering and discriminants are STABLE — additions are minor-version
   per ADR-0037 K8s-Condition convention."* This is the exact
   end-of-enum variant-addition policy the envelope design relies on.
   ServiceHydrationStatus is not yet stress-tested by a multi-version
   golden-bytes fixture (Phase 1 has no V2), so this precedent
   establishes the *shape* in production code, not the *tested
   invariant*. The golden-bytes fixtures specified in § 6 of this ADR
   close that gap structurally for every envelope going forward.

The combination — rkyv source semantics + in-repo enum precedent +
mandatory golden-bytes fixtures — is what justifies Option A1.

### 2. Write-time invariant — visibility + lint backstop

The write invariant is enforced by **two complementary layers**, each
honest about what it actually blocks:

**Layer 1 — Rust visibility blocks cross-crate payload construction.**
The `V1` / `V2` payload structs (`AllocStatusRowV1`, `AllocStatusRowV2`,
…) are declared `pub(crate)` inside their defining module of
`overdrive-core` and are NOT re-exported from the crate root. A
cross-crate writer (the dominant case — every writer lives in
`overdrive-store-local`) cannot construct `AllocStatusRowV1 { ... }`
because the *type* is unreachable from outside `overdrive-core`. The
only way to construct an envelope from `overdrive-store-local` is
`AllocStatusRowEnvelope::latest(latest_payload)`, where
`latest_payload`'s type is the publicly-exported
`<Foo>Latest` alias.

**What Layer 1 does NOT block.** This is the important honesty. The
envelope enum itself is `pub`, so the variant constructor expression
`AllocStatusRowEnvelope::V1(<expr>)` is syntactically reachable from
any crate — Rust's variant-visibility model exposes constructors
through the enum's visibility, not the payload's. Layer 1 only blocks
*constructing the payload value* cross-crate; it does NOT block
calling the variant constructor with any payload-typed expression. An
in-crate caller (within `overdrive-core`) can write
`AllocStatusRowEnvelope::V1(payload)` against any `pub(crate)`
payload it can name. A cross-crate caller cannot — but only because
they cannot produce a value of the payload type, not because the
variant is unreachable.

For this codebase the cross-crate boundary is the dominant writer
surface: every IntentStore / ObservationStore write site lives in
`overdrive-store-local`, not in `overdrive-core`. Layer 1 covers that
case structurally.

**Layer 2 — `xtask::dst_lint` clause closes the in-crate hole.**
Within `overdrive-core` itself, the residual hole is closed by a
**one-clause addition to the existing `xtask::dst_lint` scanner**
(precedent: `xtask/src/dst_lint.rs`, already syntactically scans
`core`-class crate sources for banned shapes — see ADR-0003). The
new clause walks `overdrive-core` source for `<Envelope>::V<N>(`
literal expression patterns *outside* the defining module's own
`From` / `into_latest` impls, and fails the PR at CI time.

**Why dst_lint, not a separate crate binary.** The
`.claude/rules/development.md` § "xtask is build / test / dev
orchestration, NOT a runtime entry point" rule forbids `overdrive-*`
crate dependencies inside `xtask` (the chicken-and-egg failure mode
documented there). The existing `xtask::dst_lint` is **purely
syntactic** — it reads source files as text/AST and matches patterns;
it does NOT import any `overdrive-*` crate. The envelope-variant
construction check is the same shape: variant construction is the
textual pattern `<Envelope>::V<N>(` outside the defining module, and
the scanner needs only source text + path information to enforce it.
This is grep-tier matching; AST walking via the existing `syn`-based
scanner is sufficient and strictly stronger than regex (it can
distinguish the call expression `AllocStatusRowEnvelope::V1(x)` from
a doc-comment mention of the same string).

Decision: **extend the existing `xtask::dst_lint` subcommand**, do
NOT add an `overdrive-*` crate dependency to xtask and do NOT relocate
the check to a runtime binary. The scanner is syntactic; the xtask
boundary stays intact.

Rejected alternatives for write enforcement:

- **`#[non_exhaustive]`** — only blocks downstream-crate exhaustive
  matches; does NOT prevent construction.
- **`pub(in path)` on variants** — Rust does not support per-variant
  visibility narrower than the enum.
- **`compile_fail` trybuild per envelope** — would require one
  fixture per envelope; the dst-lint clause covers all five (and
  every future envelope) with a single rule. A *single* trybuild
  fixture is still recommended as a complement (see § 6 testing)
  to pin the cross-crate visibility property.
- **Standalone `overdrive-envelope-lint` binary** — would require
  `overdrive-core` for type resolution. Rejected because the check
  is syntactic; pulling `overdrive-core` into a lint tool reintroduces
  the bootstrap-graph cost without buying any signal.

### 3. Read-time policy — asymmetric by layer

| Layer | Unknown variant (`V3` against a V2 binary) | Malformed bytes |
|---|---|---|
| **Intent** (`Job`) | Refuse to start: `health.startup.refused` with `IntentStoreError::UnknownEnvelope` | Refuse to start: `IntentStoreError::MalformedEnvelope` |
| **Observation** (all four row types) | Log + skip the row (single-row degradation) | Log + skip the row |

Rationale: intent is the SSOT; losing one row is data loss. Observation
is gossiped and converges; losing one row is a tick away from
recovery. The asymmetry preserves the existing platform discipline
(whitepaper §18 state-layer hygiene).

Error variants:

- `overdrive-core::codec::envelope::EnvelopeError` — canonical.
- `IntentStoreError::Envelope { #[from] source: EnvelopeError }`.
- `ObservationStoreError::Envelope { #[from] source: EnvelopeError }`.

### 4. Intent aggregate — outer envelope only

The `Job` aggregate carries embedded `WorkloadDriver` and `Exec`
types. The envelope lives at the outer `Job` level only. Embedded
type changes (e.g. a new `WorkloadDriver::MicroVm` variant, a new
`Exec` field) bump the outer `Job` envelope version.

Rationale: one version axis per persisted unit. Sub-envelopes on
`WorkloadDriver` and `Exec` would create a combinatorial version
space (`Job V1 + WorkloadDriver V2 + Exec V1` is one point;
`Job V1 + V1 + V2` is another) that operators cannot reason about.
Coupling internal-type changes to the outer envelope version is the
correct shape — the *file format* is what changed.

### 5. Migration

Greenfield single-cut for Phase 1. Per
`feedback_single_cut_greenfield_migrations.md`: existing dev
`~/.overdrive/data` files must be deleted on this PR landing. The
envelope exists so that **future** versions can read today's
`V<latest>` files without rebuild — not so that today's binaries can
read pre-envelope files.

### 6. Operator Remediation

When the control-plane boots and the intent envelope decode fails —
either because the bytes are pre-envelope (no `V1` tag at offset 0),
unrecognised future variant (e.g. a downgrade from a `V2`-aware
binary), or genuinely corrupted — the binary MUST refuse to start
with a typed, operator-actionable error. Phase 1 contract:

- **Failure surface**: structured `health.startup.refused` event
  emitted on stderr/log before exit; non-zero process exit code.
- **Error type carried**: `IntentStoreError::Envelope { source:
  EnvelopeError }` (per § 3 above), with `Display` form including
  the concrete redb path and the originating `EnvelopeError`
  variant (`UnknownVersion { observed, supported_max }` vs
  `Malformed { source }`).
- **Documented remediation** (in the `Display` of the typed error
  AND in the PR description landing ADR-0048): *"delete
  `<data_dir>/intent.redb` and restart the control-plane."* Phase 1
  is single-node greenfield per
  `feedback_single_cut_greenfield_migrations.md`; there is no
  in-place migration tooling and no Phase-1 fleet that would warrant
  one. Operators deleting the file accept Phase-1 single-cut
  semantics, which is the documented contract.
- **Out of scope for Phase 1**: in-place migration tooling, partial
  recovery, intent-row salvage. Phase 2+ may reconsider when a
  production fleet exists; until then, the envelope discipline is
  *forward*-compatibility only (today's `V<latest>` readable by
  tomorrow's binary), not backward-compatibility against
  pre-envelope bytes.

Observation rows do NOT trigger refuse-to-start — per § 3, they
degrade gracefully (log + skip the offending row, convergence
proceeds for surviving rows). The asymmetry is load-bearing: intent
is the SSOT (refusing to start preserves "no data loss"); observation
is gossiped and converges (refusing to start would cascade single-row
corruption into cluster-wide downtime, the explicit failure mode of
Option 4 rejected below).

## Alternatives Considered

### Option 1 — Generic `Envelope<T>` workspace primitive (rejected)

A single `enum Envelope<T> { V1(T), V2(NewerT), … }` parameterised
over the inner type. Rejected because:

- Generic instantiation couples version axes across unrelated types
  (`Envelope<AllocStatusRowV1>` and `Envelope<JobV1>` are distinct
  types but share the bump cadence in source).
- Forward-read (V1 binary peeking at V2 bytes) requires two-stage
  decode that rkyv's positional layout makes awkward — the enum
  discriminant is fixed at offset 0 but the payload pointer offset
  follows the type's archived size, which differs per `T`.

### Option 2 — `Tagged<T> { version: u16, payload: T }` struct (rejected)

A fixed-offset version tag at byte 0. Rejected because every
version's payload would have to share the same archived type, which
defeats the purpose. Carrying different payload types per version
requires an enum anyway.

### Option 3 — `Option<T>` additive fields (rejected — buggy)

The pre-incident understanding. False — `Option<T>` is positional
like every other rkyv field and shifts offsets.

### Option 4 — Symmetric refuse-to-start on observation rows (rejected)

Considered for write-side symmetry with intent. Rejected because
observation rows are gossiped and converge; refusing to start on a
single malformed observation row would cascade single-row corruption
into cluster-wide downtime. Single-row degradation matches the
existing observation-layer eventual-consistency contract.

### Option 5 — Sub-envelopes on `WorkloadDriver` and `Exec` (rejected)

See § 4 above. Combinatorial version space without operator-visible
benefit at Phase 1 scope. Phase 2+ may re-evaluate.

## Consequences

**Positive**:
- Schema bumps become a structural, type-system-enforced operation.
- Golden-bytes test discipline (per `.claude/rules/testing.md`
  addition) catches silent layout drift at PR time.
- Intent / observation asymmetric policy preserves SSOT integrity:
  intent fails fast; observation converges through degradation.
- Cross-crate payload construction is blocked by Rust visibility on
  the inner `pub(crate)` types — the dominant writer surface
  (`overdrive-store-local` → `overdrive-core`) is closed by the
  type system. The xtask dst-lint clause is required only for
  in-crate construction (a strict minority of writer sites today).

**Negative**:
- Every persisted type gains a per-variant enum overhead (one `u8`
  discriminant in the archive). Storage cost ~1 byte per row —
  negligible.
- Every internal-type change to a subtype of `Job` bumps the outer
  `Job` envelope version. Stated coupling, not a bug.
- Greenfield single-cut means existing dev `~/.overdrive/data` files
  must be deleted on this PR landing.
- Cross-crate variant construction is NOT blocked by `pub(crate)`
  alone — only payload *construction* is. The variant constructor
  `Envelope::V1(<expr>)` remains syntactically reachable from any
  crate because the envelope enum is `pub`; it is rendered uncallable
  in practice by the payload type being unnameable. Within
  `overdrive-core` (in-crate writers) the variant constructor IS
  reachable and IS callable; the dst-lint clause is the load-bearing
  gate for that case. This honest framing replaces the earlier
  "`pub(crate)` closes the cross-crate writer hole" wording, which
  was correct in effect (because no cross-crate caller can supply a
  payload value) but misleading about mechanism.

## References

- `docs/feature/rkyv-envelope-evolution/design/wave-decisions.md`
- ADR-0003 (crate-class taxonomy; `xtask::dst_lint` precedent)
- ADR-0035 (Reconciler memory collapse — CBOR envelope analog on
  the View side)
- ADR-0036 (AnyState amendment — schema-evolution policy precedent)
- `.claude/rules/development.md` § "rkyv schema evolution" (new
  section)
- `.claude/rules/testing.md` § "Property-based testing (proptest)"
  → "Mandatory call sites" (new bullet)
- `feedback_single_cut_greenfield_migrations.md` (migration policy)
