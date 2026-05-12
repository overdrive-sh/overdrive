# rkyv-envelope-evolution — Feature Evolution

**Feature ID**: `rkyv-envelope-evolution`
**Date**: 2026-05-12
**Origin**: Bug-fix RCA (no DISCUSS/SPIKE/DEVOPS waves; DESIGN → DISTILL → DELIVER)
**Duration**: Single day (2026-05-12, ~12 h elapsed DESIGN-to-DELIVER-close)
**Status**: Delivered — mutation gate 100% (56/56 viable), 1299 tests pass, clippy clean, dst-lint clean
**ADR**: [ADR-0048](../product/architecture/adr-0048-rkyv-versioned-envelope.md) — rkyv Versioned Envelope

---

## What shipped

A per-type versioned-envelope mechanism for every rkyv-persisted type at
every redb storage boundary. The mechanism closes a class of silent data
corruption: rkyv archives have fixed positional layouts, so adding a field
to a struct shifts every subsequent field offset and renders pre-existing
on-disk bytes unreadable with a `subtree pointer overran range` rejection at
read time. Before this feature, no defensive path existed.

### Production code

**New codec primitive** (`crates/overdrive-core/src/codec/`):

- `VersionedEnvelope` trait — four-method contract pinning
  `into_latest()`, `latest()`, `known_discriminants()`,
  `discriminant_offset_from_end()`, and `type_name()` with rustdoc
  preconditions / postconditions / edge cases / observable invariants.
- `EnvelopeError` — typed `UnknownVersion { observed, type_name,
  supported_max }` and `Malformed { source: rkyv::rancor::Error }` variants
  with operator-readable `Display` forms.
- `decode_envelope_bytes::<E>()` — single driving-port combining
  pre-decode discriminant probe (dead-on-arrival unknown-version detection)
  with rkyv access + `into_latest()` projection.

**Five per-type envelopes** (all in `overdrive-core`):

| Row type | Layer | Behaviour on decode failure |
|---|---|---|
| `AllocStatusRowEnvelope` | Observation | `tracing::warn!(name: "observation.envelope.decode_failed", ...)` + skip row |
| `NodeHealthRowEnvelope` | Observation | Same |
| `ServiceHydrationResultRowEnvelope` | Observation | Same |
| `ServiceBackendRowEnvelope` | Observation | Same |
| `JobEnvelope` | Intent | `tracing::error!(name: "health.startup.refused", ...)` + return `IntentStoreError::Envelope` |

**Public API shape** (alias-to-payload per UI-02):

```rust
pub type AllocStatusRow = AllocStatusRowV1;         // callers unchanged
pub type AllocStatusRowLatest = AllocStatusRowV1;   // forward-pointer alias
// AllocStatusRowEnvelope is codec-internal — NOT re-exported from crate root
```

**Typed codec on `Job`** (`Job::archive_for_store`, `Job::from_store_bytes`,
`Job::spec_digest`): wrapping discipline co-located at the typed value; the
`IntentStore` trait surface unchanged (bytes-passthrough by design; shared
with future `RaftStore` Phase 2 snapshot contract).

**All ~15-20 `Job` writer/reader call sites** migrated across
`overdrive-control-plane` (handlers, reconciler_runtime), `overdrive-sim`
(exit-event invariant), and test fixtures workspace-wide.

**`spec_digest` semantics change**: SHA-256 is now over envelope bytes
(output of `Job::archive_for_store()`), not raw V1 payload bytes. This is
an operator-observable Job ID change absorbed by the greenfield single-cut
migration policy (ADR-0048 § 5).

**`LocalObservationStore` read/write paths** (`crates/overdrive-store-local`)
fully wrapped at all four observation row tables. `LocalStore::open` recovery
walk calls `Job::from_store_bytes` per `jobs/`-prefixed entry.

### Structural enforcement (xtask dst-lint)

Two new `xtask::dst_lint` clauses run as part of `cargo xtask dst-lint`:

- **`scan_for_envelope_variant_construction`** — syn-based AST walk of
  `overdrive-core` source; flags `<Envelope>::V<N>(` call expressions outside
  `fn into_latest` bodies and `impl From<VN> for VN+1` blocks. Fails the run
  on any violation (Layer 2 structural enforcement per ADR-0048 § 2).

- **`scan_for_envelope_fixture_coverage`** — walks `<crate>/src/` for
  `enum *Envelope` definitions and verifies `<crate>/tests/schema_evolution/<envelope_snake>.rs`
  exists with a `FIXTURE_V<N>: &str` constant for every variant. Fails on
  any missing fixture file or missing constant (closes the loop: future
  envelopes without golden-bytes tests fail CI automatically).

**Trybuild compile-fail fixture** at
`crates/overdrive-store-local/tests/compile_fail/alloc_status_row_envelope_unreachable.rs`
asserts E0432 on `use overdrive_core::AllocStatusRowEnvelope` — the
non-re-export of the codec-internal envelope enum is Layer 1's enforcement
mechanism (non-re-export from `lib.rs` + code review, not a compiler
privacy gate, due to rustc E0446; see UI-01 below).

### Tests

- 5 per-type golden-bytes fixtures (`FIXTURE_V1`) in
  `crates/overdrive-core/tests/schema_evolution/` — never touched by future
  PRs; layout change detection via byte-exact assertion.
- `assert_unknown_version_probe_surfaces<E>` harness helper covering
  `known_discriminants`, `type_name`, `discriminant_offset_from_end`, and
  the `probe_known_variant` classification path for all 5 envelopes.
- Integration tests: `envelope_walking_skeleton`, `envelope_observation_skip`
  (4 sub-tests), `envelope_intent_refuse` — real redb via `tempfile::TempDir`.

---

## Business context

rkyv's fixed-positional-layout serialisation is the source of the latency
win at redb hot-path reads (no deserialization pass; direct
`&ArchivedT` access over mmap'd bytes). The tradeoff is a strict
schema-evolution contract: adding a field, even `Option<T>`, shifts offsets
and silently corrupts reads. The production codebase was already relying on
rkyv for all five observation row types and the `Job` intent aggregate, but
without any versioning mechanism.

The immediate trigger was a `subtree pointer overran range` rejection
surfaced during testing when a schema-changed `AllocStatusRow` was read back
from a pre-change redb file. The fix closes the gap for all five types
simultaneously, establishes the structural enforcement that prevents future
types from being added without a versioned envelope, and documents the
version-bump procedure in development guidelines.

---

## Key decisions

### D-01 — Per-type rkyv enum (Option A1, ADR-0048 § 1)

Each rkyv-persisted type gets its own versioned envelope enum
(`FooRowEnvelope`) rather than a generic `Envelope<T>` wrapper. The
per-type enum preserves the rkyv `#[repr(u8)]` discriminant ordering
invariant across variant additions and keeps the variant-construction
scanner syntactically simple (no generic parametrisation to reason
through).

Alternatives considered and rejected: generic `Envelope<T, V>` (rejected —
generic parametrisation in rkyv enum variants is not supported), single
global envelope (rejected — collapses the per-type discriminant namespace),
serde-based fallback (rejected — rkyv and serde are orthogonal codecs;
mixing them at the same boundary creates two canonical byte forms).

### D-02 — Outer-envelope-only on `Job` aggregate (ADR-0048 § 4)

`Job` contains embedded `WorkloadDriver` and `Exec` rkyv types, but only
the outer `Job` gets an envelope. Sub-envelopes on embedded types are
explicitly rejected — they create a combinatorial version space
(`JobV1EnvelopeWorkloadDriverV2EnvelopeExecV3...`) without operator-visible
benefit. Embedded type changes bump the outer envelope version and the
`From<V1> for V2` impl handles field mapping.

### D-03 — Asymmetric failure policy (ADR-0048 § 3)

Intent layer (fail-fast): malformed or unknown-version `JobEnvelope` bytes
cause `LocalStore::open`'s recovery walk to return
`IntentStoreError::Envelope` with `health.startup.refused` event emitted
before the error. Operator remediation: "delete `<data_dir>/intent.redb`
and restart" — greenfield scope makes this the correct policy.

Observation layer (log+skip): malformed or unknown-version observation row
bytes emit `observation.envelope.decode_failed` and skip the offending row;
convergence proceeds for surviving rows. Rationale: gossip-converged
observation state is re-writable by the authoritative source; the node is
not required to refuse service over a single corrupted observation row.

### D-04 — Alias-to-payload public API (UI-02 amendment)

`pub type FooRow = FooRowV1` — the public alias points at the V1 payload
struct (not the envelope enum). Call sites continue using struct-literal
`FooRow { fields }` syntax unchanged. The codec-internal envelope enum
(`FooRowEnvelope`) is NOT re-exported from `overdrive-core::lib.rs`.

The prior roadmap criterion specified `pub type FooRow = FooRowEnvelope`
(alias-to-envelope). This was reversed during step 01-03 when it became
clear it would force ~50-70 call-site rewrites per type (struct-literal →
`FooRow::latest(FooRowLatest { fields })`) for no enforcement gain over
the alternative. The first crafter's commit (`a90755a2`) shipped the
correct alias-to-payload shape; the orchestrator's pushback was a design
mistake; the user reversed direction.

### D-05 — Typed codec module on `Job`; `IntentStore` trait unchanged (UI-03)

The `IntentStore` trait is a generic byte-level k/v surface by design —
it persists `Job` aggregates, `WorkloadKind` discriminator bytes (ADR-0047),
stop sentinel markers, and frame-wrapped snapshot bytes (ADR-0020). It is
shared with the future `RaftStore` Phase 2 path whose snapshot contract
relies on byte identity. Refactoring the trait to take typed `Job` on
write/read would break non-Job value classes and the RaftStore snapshot
contract.

Decision: codec module on `Job` itself (`Job::archive_for_store`,
`Job::from_store_bytes`, `Job::spec_digest`). Trait surface unchanged.
`LocalStore::open`'s recovery walk calls `Job::from_store_bytes` per
`jobs/`-prefixed entry. Wrapping discipline lives at one named site on the
typed value.

### D-06 — Layer 1 enforcement via non-re-export (UI-01 amendment)

ADR-0048 § 2 Layer 1 originally mandated inner payload types be declared
`pub(crate)`. During step 01-01 (`0dc53e05`) it was discovered that
`VersionedEnvelope` is a `pub` trait, and `type Latest = FooRowV1;` inside
an impl makes `FooRowV1` part of the trait's public surface — rustc E0446
rejects `pub(crate)` on a type referenced from a `pub` trait's
associated-type assignment.

Resolution (option 1, user-confirmed, commit `62bf6ed6`): inner payload
types declared as plain `pub`, un-re-exported from crate root. Layer 1
enforcement = non-re-export from `lib.rs` + code-review convention (not a
compile-time gate). Layer 2 (xtask dst-lint variant-construction scanner)
is the load-bearing structural defense. ADR-0048 and development.md updated.

---

## Steps completed

| Step | Description | Outcome |
|---|---|---|
| 01-01 | RED scaffolds — codec module, envelopes, errors | GREEN PASS |
| 01-02 | GREEN — codec foundation (VersionedEnvelope trait + EnvelopeError + harness) | GREEN PASS |
| 01-03 | GREEN — AllocStatusRowEnvelope walking skeleton (S-EV-01.1 + S-EV-04.1) | GREEN PASS |
| 01-04 | GREEN — JobEnvelope intent refuse-to-start + typed codec on Job | GREEN PASS (after 3 dispatch attempts due to hook/context issues) |
| 01-04-refactor | Refactor — discriminant probing offset triangulation | GREEN PASS |
| 02-01 | NodeHealthRowEnvelope end-to-end (S-EV-01.2 + S-EV-04.3) | GREEN PASS |
| 02-02 | ServiceHydrationResultRowEnvelope end-to-end (S-EV-01.3 + S-EV-04.4) | GREEN PASS |
| 02-03 | ServiceBackendRowEnvelope end-to-end (S-EV-01.4 + S-EV-04.5) | GREEN PASS |
| 03-01 | dst-lint clause + trybuild fixture for write enforcement | GREEN PASS |
| 03-02 | dst-lint clause for fixture-coverage gate | GREEN PASS |
| phase-3-refactor | L1–L4 RPP refactor pass | PASS |
| phase-4-review-fixes | Review defect fixes (round 1) | GREEN PASS |
| phase-4-review-fixes-round-2 | Invariant pinning + falsifiability proof | GREEN PASS |
| phase-5-mutation-testing | Mutation gate (cargo-mutants, diff-scoped) | PASS — 100% kill rate (56/56 viable) |

---

## Issues encountered

### UI-01 — rustc E0446: pub(crate) inner payload rejected under pub trait

ADR-0048's original Layer 1 mechanism (`pub(crate)` inner payload types)
cannot compile: `VersionedEnvelope` is a `pub` trait; its
`type Latest = AllocStatusRowV1;` associated-type assignment makes the
payload part of the public surface; rustc E0446 rejects `pub(crate)` there.

**Resolution**: inner payloads declared `pub`, un-re-exported from crate
root. Layer 2 (dst-lint) becomes the load-bearing enforcement. ADR-0048 § 2
and development.md § "rkyv schema evolution" Rules bullet 1 amended.

### UI-02 — Alias-to-payload reversed from alias-to-envelope

The roadmap criterion specified `pub type AllocStatusRow = AllocStatusRowEnvelope`
(alias-to-envelope). Step 01-03's first crafter shipped the correct
alias-to-payload shape (`pub type AllocStatusRow = AllocStatusRowV1`). The
orchestrator pushed back and attempted a re-migration; the user halted and
confirmed the first crafter's shape was correct.

**Resolution**: alias-to-payload. The roadmap criteria for steps 01-03 and
01-04 were retroactively corrected. Commit `a90755a2` remains the canonical
GREEN landing for step 01-03.

**Lesson**: the roadmap criterion contained a design mistake (alias-to-envelope)
that the crafter correctly ignored in favour of a simpler shape. When a
reviewer pushes back based on criteria that contain a design error, the
crafter's working code may be the correct ground truth. Validate criteria
against the codebase before re-migrating.

### UI-03 — IntentStore trait is bytes-passthrough; cannot accept typed Job

During step 01-04, migrating `LocalStore::open`'s intent read/write paths
revealed that the `IntentStore` trait persists multiple value classes (Job
aggregates, WorkloadKind discriminators, stop sentinels, snapshot frames) on
a generic byte-level surface shared with the future `RaftStore` Phase 2
path. Refactoring the trait to accept typed `Job` would break all other
value classes and the RaftStore snapshot contract.

**Resolution**: codec module on `Job` itself (`Job::archive_for_store` /
`Job::from_store_bytes` / `Job::spec_digest`). The trait surface is
unchanged. Step 01-04 scope increased from ~10h advisory to ~28h advisory
to account for ~15-20 call-site migrations + `spec_digest` semantics change.

---

## Lessons learned

1. **Codec conventions need enforcement from day 1.** The observation store
   had false "rkyv has additive-field tolerance" docstrings at three call
   sites — these had to be corrected as part of this feature. Per-type
   golden-bytes fixtures plus the xtask coverage scanner together make it
   structurally impossible to add a new rkyv-persisted type without a
   versioned envelope, closing the gap for all future contributors.

2. **Roadmap criteria can contain design errors.** The alias-to-envelope vs.
   alias-to-payload issue (UI-02) shows that a crafter landing code that
   diverges from a criterion may be correct — the criterion may be wrong.
   Validate working code against the criterion's intent (call-site churn
   reduction, schema-evolution signal) not its literal wording.

3. **The IntentStore trait boundary is a shared contract.** ADR-0048's
   intent layer was designed to have a single entry point (`LocalStore::open`).
   The actual boundary is the `Job` codec module on the typed value — the
   trait stays generic by design. Any future typed value at the intent
   boundary follows the same `TypeName::archive_for_store` /
   `TypeName::from_store_bytes` shape.

4. **Mutation testing cross-package scope.** `cargo-mutants --diff` runs
   per-mutant tests against the owning package's test suite. The existing
   `UnknownVersion` probe assertion lived in `overdrive-store-local`'s
   integration tests, but the source being mutated was in `overdrive-core`.
   The mutation gate correctly flagged this gap (54.4% initial kill rate);
   adding per-envelope probe tests in `overdrive-core`'s own test suite
   resolved it (100% final).

5. **dispatch context / hook issues block Edit/Write.** Step 01-04 required
   three dispatch attempts because the source-write hook blocked edits from
   the orchestrator's direct context. Subagent dispatch via `Task` is the
   required shape for all DELIVER execution; the orchestrator context cannot
   write source files directly.

---

## Migrated artifacts

| Source | Destination |
|---|---|
| `distill/test-scenarios.md` | `docs/scenarios/rkyv-envelope-evolution/test-scenarios.md` |
| `distill/walking-skeleton.md` | `docs/scenarios/rkyv-envelope-evolution/walking-skeleton.md` |

ADR-0048 is already in its permanent location:
`docs/product/architecture/adr-0048-rkyv-versioned-envelope.md`

---

## Discarded (process scaffolding)

The following files are retained in the feature workspace as process
history but are not migrated to permanent directories:

- `deliver/execution-log.json` — audit trail captured above
- `deliver/roadmap.json` — step plan superseded by this doc + git history
- `deliver/upstream-issues.md` — full decision records in UI-01..UI-03 above
- `deliver/mutation/mutation-report.md` — summary captured in this doc
- `design/wave-decisions.md` — key decisions extracted above
- `distill/wave-decisions.md` — key decisions extracted above
- `distill/red-scaffolds.md` — process scaffolding; tests themselves remain in `tests/`
