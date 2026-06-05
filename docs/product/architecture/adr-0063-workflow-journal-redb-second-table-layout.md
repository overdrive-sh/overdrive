# ADR-0063 — Workflow `await`-point journal: a second redb table layout on the runtime-owned substrate (distinct from the reconciler `View` store), CBOR-encoded append-only entries keyed `(workflow_id, step)`

## Status

Accepted. 2026-06-05. Decision-makers: Morgan (proposing, PROPOSE
mode); user ratification pending (subagent context — the headline
journal-store call is surfaced for ratification in the DESIGN return
summary). Tags: phase-1, workflow-primitive, application-arch,
durable-memory.

**Extends ADR-0035** ("Reconciler memory: typed-View blob auto-persisted,
redb backend"). This ADR adds a *second* table layout to the same
runtime-owned redb substrate ADR-0035 established. It does not supersede
ADR-0035; the reconciler `View` store is unchanged. **Companion**:
ADR-0064 (the `Workflow` trait / `WorkflowCtx` surface + engine
boundary). **Composes with**: ADR-0023 (action-shim; the engine is
driven off the same per-tick pipeline), ADR-0037 (`TerminalCondition` /
`WorkflowResult` terminal modelling).

## Context

The `workflow-primitive` feature (GH #39, roadmap [3.2]) ships the §18
durable-async `Workflow` primitive. Its DIVERGE→DISCUSS direction is
**locked to "B′"** (`docs/feature/workflow-primitive/wave-decisions.md`
§ "RATIFIED DIRECTION"): a distinct durable-async `Workflow` primitive
(`async fn run(&self, ctx) -> WorkflowResult`) with its step/await
**journal in redb**, instance lifecycle owned by the workflow-lifecycle
reconciler, all non-determinism through `ctx`, correctness = deterministic
replay + bounded progress. Two decisions inside that locked direction are
this ADR's remit, decided per option below:

1. **Where the journal lives** relative to ADR-0035's `RedbViewStore`:
   extend that adapter to host two table layouts, or stand up a sibling
   `RedbJournalStore` adapter on the same backend.
2. **What codec** the journal entries use: rkyv versioned envelope
   (ADR-0048 discipline) or CBOR `#[serde(default)]` (ADR-0035 ViewStore
   discipline).

The whitepaper already reconciles the "one redb table layout" claim of
§17: reconciler memory is "a generic `(reconciler_name, target) → blob`
key space the runtime owns. (The workflow journal, §18, is a second redb
table layout — a per-instance append-only step/await record keyed by
`(workflow_id, step)` — sharing the redb substrate but not this key
space.)" (whitepaper §18 *The Reconciler Primitive*, §17 *Reconciler
Memory*). This ADR pins the concrete shape that claim implies.

The pre-DIVERGE whitepaper text said the journal lived in "per-primitive
libSQL". That phrasing predates the ADR-0035 move of reconciler memory
*off* libSQL onto redb for O6 (one-mechanism). R2 of the ratified
direction resolves the open assumption ([D4] of DIVERGE) in favour of
redb. This ADR records the libSQL→redb decision as the journal's
analogue of ADR-0035's reconciler-memory decision.

## Decision

### 1. The journal is a second redb table layout, NOT an extension of `RedbViewStore`

A new driven port `JournalStore` (in `overdrive-control-plane::journal`,
adapter-host class, symmetric with `view_store`) abstracts the journal.
Production adapter `RedbJournalStore`; sim adapter `SimJournalStore`
(in `overdrive-sim`). The two adapters share the same **redb substrate**
as the reconciler `View` store — one redb file per node — but the
`JournalStore` is a **distinct trait with a distinct table layout**, not a
method added to `ViewStore` / `RedbViewStore`.

**Why a distinct port + table layout, not an extension of `ViewStore`:**

- **The access patterns are structurally different.** `ViewStore` is a
  *single typed blob per `(reconciler, target)`*, overwritten in place,
  bulk-loaded into a `BTreeMap` at boot and read from RAM forever after
  (ADR-0035 §5). The journal is an *append-only ordered sequence of
  entries per `(workflow_id)`*, point-accessed by `(workflow_id, step)`
  on replay, never overwritten. Forcing the journal's append-many,
  range-scan-on-replay pattern onto `ViewStore`'s single-blob-overwrite
  trait surface would require a second, divergent method set on the same
  trait — the trait would no longer have one coherent contract (the
  development.md "Trait definitions specify behavior" rule: a trait is a
  contract; two divergent access patterns on one trait are two
  contracts).
- **The trait method shapes do not overlap.** `ViewStore::bulk_load_bytes`
  returns `BTreeMap<TargetResource, Vec<u8>>` (one blob per target);
  the journal needs `load_journal(workflow_id) -> Vec<JournalEntry>` (an
  ordered run) plus `append(workflow_id, entry)` (single fsync'd append).
  No `ViewStore` method is reusable as-is for the journal; sharing the
  trait would mean a union trait with two non-overlapping method groups.
- **The durability *ordering* contract is the SAME and is reused.** Both
  stores honour ADR-0035 §5's fsync-then-memory write-through ordering and
  the Earned-Trust `probe()` composition-root invariant. This ADR reuses
  that *discipline* verbatim (see §4) — it is the table layout and trait
  surface that differ, not the durability model.

**What IS shared (the substrate, per R2 / O6 / K5):** the same redb
**file** (`<data_dir>/reconcilers/memory.redb`) hosts both the
reconciler-`View` tables (one per reconciler kind, ADR-0035 §4) and the
workflow-journal table(s). One redb `Database` handle, opened once at
boot, is shared by both the `RedbViewStore` and the `RedbJournalStore`
adapters via `Arc<Database>` (redb's `begin_read`/`begin_write` both take
`&self`, so a shared handle is safe). This is the "one durable-memory
story for both primitives" the locked direction (R2) requires and K5
asserts (no NEW store; no libSQL journal table). The journal's tables are
namespaced by a reserved prefix (`__wf_journal__`, outside the
`ReconcilerName` grammar `^[a-z][a-z0-9-]{0,62}$`, same guard
`RedbViewStore` already uses for its `__probe__` table) so a reconciler
table name can never collide with a journal table name.

**Why one file, not a second redb file** (the inverse of ADR-0035 §4's
"separate from IntentStore" call): ADR-0035 separated reconciler memory
from IntentStore because they have *different durability tiers* (per-tick
fsync vs per-submit fsync). The reconciler `View` store and the workflow
journal have the **same** durability tier — both are per-`await`/per-tick
runtime-owned memory, both fsync per write. Splitting them across two
files would buy nothing and cost a second `Database` handle + a second
`probe()` site. One file, two table layouts, one durable-memory story.

### 2. Codec — CBOR via `ciborium`, the ADR-0035 discipline, NOT the rkyv envelope

Journal entries are **CBOR-encoded** (`ciborium`, the same crate and
discipline ADR-0035 §3 established for `View` blobs), with additive
schema evolution via `#[serde(default)]` and a `#[serde(tag = "v")]`
versioned envelope reserved for the first breaking change.

**Why CBOR, not the rkyv versioned envelope (ADR-0048):**

- **The journal is mutable, evolving, runtime-owned memory — ADR-0035's
  exact case, not ADR-0048's.** ADR-0048's rkyv envelope governs
  *content-addressed / hash-input* persisted types at the redb boundary
  (observation rows, `Job` intent aggregates) where the archived bytes
  are the canonical wire form and may feed a content hash. The journal is
  none of these: it is private runtime memory the workflow engine reads
  back on replay, never hashed, never content-addressed, never gossiped.
  ADR-0035 §3 already rejected rkyv for exactly this shape: "rkyv
  explicitly disclaims schema evolution support … rkyv stays in its
  current role for read-heavy hashed paths, NOT for mutable persisted
  view state." The journal is mutable persisted memory; it inherits
  ADR-0035's codec verdict.
- **Replay determinism does NOT require rkyv.** The K4 replay-equivalence
  property (D-INH-5) is about the *engine re-executing `run` and getting
  byte-identical `await` resolutions from the recorded journal* — it is a
  property of the engine's replay logic over decoded entries, not of the
  on-disk byte layout. CBOR decode is deterministic (a given byte
  sequence decodes to one value); that is all replay needs. rkyv's
  zero-copy archived-byte canonicality buys nothing here because the
  journal is decoded, not accessed-in-place, and is never hashed.
- **Schema evolution is the journal's live concern, and CBOR wins it.**
  Slices 02 (`ctx.sleep`) and 03 (`ctx.wait_for_signal` / `ctx.emit_action`)
  each add a new journal-entry variant. `#[serde(default)]` additive
  evolution (ADR-0035 §3) absorbs these additively; the rkyv
  fixed-positional layout (ADR-0048 — "adding a field shifts every
  subsequent offset and renders pre-existing bytes unreadable") would
  force a version-bump + golden-fixture ceremony for each await-surface
  slice. The journal is *designed to grow one entry-variant per slice*;
  CBOR's additive tolerance is the right tool for that growth shape.

**Entry shape** (`JournalEntry` enum, CBOR-tagged):

```text
JournalEntry =
  | Started      { spec_digest, input_digest }                  // slice 01
  | RunResult    { step, name, result_digest, result_bytes }    // slice 01 (ctx.run<T>)
  | SleepArmed   { step, deadline_unix }                        // slice 02 (ctx.sleep) — input, not "remaining"
  | SignalAwaited{ step, signal_key }                           // slice 03 (ctx.wait_for_signal)
  | SignalSeen   { step, signal_key, value_digest }             // slice 03
  | ActionEmitted{ step, action_digest }                        // slice 03 (ctx.emit_action)
  | Terminal     { result }                                     // slice 01 (WorkflowResult)
```

The `step` field is the monotonic `await`-point index (the journal
cursor — see ADR-0064 §3); **step identity is positional**, the cursor
itself, NOT a content correlation. `RunResult` is the journal entry for the
general durable-step primitive `ctx.run<T>(name, f)` (ADR-0064 §3): `name`
is the diagnostic label the closure was given AND the replay determinism
check (a mismatch at replay step N is a nondeterministic body → fail-closed);
`result_bytes` is the **CBOR encoding of the closure's `T`** (so replay
returns a byte-equal value by deserializing it), and `result_digest` is the
SHA-256 over those bytes (the replay-equivalence invariant compares it).
The per-step `correlation` field present in the pre-amendment `CallResult`
entry is **REMOVED** — it was unused for replay matching, since the cursor is
positional. (Instance-level `CorrelationKey` — the workflow instance
identity used by the engine and the `ObservationRow::WorkflowTerminal` row —
is a separate concern and is UNCHANGED.) `SleepArmed` records the **deadline**
(an input), not a "remaining duration" — per development.md "Persist inputs,
not derived state"; resume recomputes remaining wait from
`recorded_deadline − clock.now()` (slice-02 AC4). Signal/action effect
payloads are recorded as digests (`value_digest`, `action_digest`), not full
bodies, to keep those entries small; the digest is sufficient for
replay-equivalence (the engine re-derives the effect deterministically and
compares the digest). `RunResult` carries the full `result_bytes` (not only a
digest) because the recorded value IS the replay return — it must be
reconstructable byte-for-byte, not merely verifiable.

### 3. Table layout — one append-only table, key `(workflow_id, step)`

A single redb table `__wf_journal__` with key `(WorkflowId, u32)`
(workflow instance + step index) and value = CBOR-encoded `JournalEntry`.
redb's `(K1, K2)` tuple keys give an ordered range scan per `workflow_id`
for free (load-on-resume is a range query `(id, 0)..=(id, u32::MAX)`),
and a point write per append. This is the canonical redb append-mostly
point-access shape ADR-0035 §4 / whitepaper §17 name as "redb's
wheelhouse."

One table (not one table per workflow kind, the `ViewStore` shape):
workflow *instances* are unbounded in cardinality (unlike reconciler
*kinds*, which are a small fixed set), so a table-per-kind would not bound
table count, and the `(workflow_id, step)` composite key already isolates
instances within one table. Per-instance retention/compaction (#208) is a
forward concern; this ADR's layout does not preclude it (a retention
sweep is a range-delete per terminal `workflow_id`).

### 4. Durability + Earned-Trust — reused verbatim from ADR-0035

- **fsync-then-memory ordering** (ADR-0035 §5 step 7→8): `append` performs
  one redb write transaction with `Durability::Immediate` (fsync before
  return) BEFORE the engine suspends the `await` (slice-01 AC2,
  US-WP-2 AC2). On crash between fsync and suspend, the next boot's
  `load_journal` sees the persisted entry and replay resumes from it.
  The `WorkflowJournalWriteOrdering` DST invariant (§6 of ADR-0064) pins
  this, mirroring ADR-0035's `WriteThroughOrdering`.
- **Earned-Trust `probe()`**: `JournalStore::probe()` writes a sentinel
  `(__probe_wf__, 0)` entry → fsync → reads back byte-equal → deletes,
  exactly as `RedbViewStore::probe` does. The composition root invariant
  is "probe view-store AND journal-store, then use" — both probes run at
  boot before the first workflow starts; either failure refuses startup
  with `health.startup.refused` (Earned Trust principle 12). Because both
  adapters share one `Database`, the journal probe also transitively
  exercises the shared file's writability.

### 5. Single-node scope; cross-node resume not precluded (D3 / #205)

Phase-1 scope is single-node crash-resume (D3): kill the *process*,
restart on the *same node*, resume from the local redb journal. The
table layout does not preclude cross-node resume (#205): the journal is
keyed by `WorkflowId` (node-independent), the entries are
node-independent CBOR, and the durability contract is the same one a
future HA `RaftJournalStore` (or a Corrosion-gossiped journal-index) would
build on. No Phase-1 element hard-codes the local redb file as the only
possible `JournalStore` impl — `JournalStore` is a trait, `RedbJournalStore`
is one adapter, exactly as `ViewStore`/`RedbViewStore` leave room for a
Phase-2 `RaftStore`-shaped successor.

## Considered alternatives

### Alternative A — Second table layout on the shared redb substrate, distinct `JournalStore` port, CBOR (ACCEPTED, this ADR)

Above.

### Alternative B — Extend `RedbViewStore` into a generic two-layout memory adapter

Add journal methods (`append`, `load_journal`) directly to the `ViewStore`
trait + `RedbViewStore` adapter, so one adapter serves both layouts.

**Rejected because:**

1. **It overloads one trait with two non-overlapping contracts.** The
   `ViewStore` contract is single-blob-overwrite-per-target; the journal
   contract is append-only-ordered-run-per-instance. development.md
   ("Trait definitions specify behavior, not just signature") makes the
   trait the SSOT for one contract; a trait with two divergent method
   groups has two contracts and the DST equivalence test
   (`ViewStore`-equivalence) would have to cover both, conflating them.
2. **No method is shared.** Every journal method is new; the "extension"
   shares only the `struct RedbViewStore { db }` field — which Alternative
   A *also* shares (the same `Arc<Database>`), without the trait overload.
   The substrate reuse (the load-bearing O6/K5 property) is identical
   between A and B; B additionally couples the two trait surfaces for no
   reuse gain.
3. **It blocks independent evolution.** Slices 02/03 grow the journal
   entry variants; a future ADR-0035-style View change grows the View
   shape. Coupling them on one trait means a journal-only change touches
   the View adapter's contract surface and re-triggers its trybuild /
   equivalence fixtures.

The substrate (the redb file + `Database` handle) IS shared under A — B's
only real differentiator (one struct) is preserved without the trait
coupling.

### Alternative C — Journal in libSQL (the pre-DIVERGE whitepaper shape)

Keep the journal in a per-primitive libSQL database, as the whitepaper
text said before the ADR-0035 reconciler-memory move.

**Rejected because:**

1. **It reintroduces the two-engine operator surface ADR-0035 §Alt-E
   rejected.** redb for reconciler memory + libSQL for workflow journals
   = "the worst of both worlds; operators have to know both" (ADR-0035
   §Alternative E). O6/K5 explicitly require zero NEW stores; a libSQL
   journal table is a new store.
2. **The journal has no SQL access pattern on the critical path.** Replay
   is point-access by `(workflow_id, step)` and a range scan per instance
   — redb's COW B-tree is the canonical fit (whitepaper §17), exactly as
   it is for reconciler memory. SQL would only earn its keep for *ad-hoc
   journal queries*, which the whitepaper §18 names as "a deferrable
   read-only observability view, not a replay requirement." That view
   (#206 operator surface / future observability) can be a read-only
   libSQL projection later without making libSQL the journal's SSOT.
3. **R2 of the locked direction already decided this.** The journal lives
   in redb; this alternative is the pre-decision baseline, recorded for
   traceability, not a live contender.

### Alternative D — Append-only log file (not redb), e.g. a segmented WAL per instance

Write journal entries to a per-instance append-only log file on disk,
bypassing redb.

**Rejected because:**

1. **It is a third storage mechanism.** O6/K5 require reuse of the
   existing redb substrate; a bespoke log-file format is a new persistence
   engine to write, fsync correctly, crash-test, and compact — redb
   already solves all four (1PC+C durability, checksummed recovery,
   range-delete for compaction).
2. **redb's `(workflow_id, step)` tuple key already gives append-only
   ordered semantics** with a single point write per append and a range
   scan per load — the exact properties a hand-rolled WAL would provide,
   with none of the bespoke crash-recovery code.
3. **The Earned-Trust probe + fsync-ordering discipline is already built
   for redb** (ADR-0035); a log file would need its own probe and its own
   ordering proof.

### Alternative E — rkyv versioned envelope for journal entries (ADR-0048 discipline)

Same as A but encode journal entries with rkyv + the ADR-0048 versioned
envelope.

**Rejected** for the §2 reasons: the journal is mutable runtime memory
(ADR-0035's codec case), not a content-addressed / hashed type (ADR-0048's
case); replay determinism needs deterministic *decode*, which CBOR
provides, not zero-copy archived-byte canonicality, which buys nothing
when the bytes are decoded and never hashed; and each await-surface slice
adds an entry variant, which CBOR `#[serde(default)]` absorbs additively
where rkyv would force a per-slice version-bump + golden-fixture ceremony.

## Consequences

### Positive

- **One durable-memory story for both primitives (O6/K5).** Reconciler
  `View` blobs and workflow journals share one redb file, one `Database`
  handle, one Earned-Trust probe site, one fsync-ordering discipline. No
  libSQL journal table; grep/dep-graph clean (K5).
- **Codec discipline already proven.** CBOR + `#[serde(default)]` is the
  ADR-0035 path; the schema-evolution story, the DST roundtrip-invariant
  pattern, and the `ciborium` dependency are all reused, not invented.
- **Additive slice growth is friction-free.** Each await-surface slice
  (02 sleep, 03 signal/emit) adds one `JournalEntry` variant under
  `#[serde(default)]` — no version-bump, no golden-fixture per slice.
- **Cross-node resume not precluded (#205).** Node-independent
  `WorkflowId`-keyed CBOR entries behind a `JournalStore` trait leave room
  for a Phase-2 HA adapter, exactly as `ViewStore` does.
- **Replay-equivalence (K4) rests on deterministic decode**, which CBOR
  provides; the property is an engine concern (ADR-0064), and the journal
  codec does not complicate it.

### Negative

- **A second trait + two adapters to write and test.** `JournalStore` +
  `RedbJournalStore` + `SimJournalStore`, each with its own
  contract-equivalence and roundtrip coverage. Mitigation: the shapes
  mirror `ViewStore`/`RedbViewStore`/`SimViewStore` almost line-for-line;
  the cost is bounded and the patterns are copy-adaptable.
- **Journal-entry digest indirection.** Recording effect payloads as
  digests (not full bodies) means an observability view that wants the
  full body must reconstruct it (or the digest must resolve against the
  `external_call_results` ObservationStore row). Acceptable: replay needs
  only the digest; full-body observability is a #206/#208 forward concern.
- **Shared `Database` handle couples the two stores' open/probe lifecycle.**
  A corrupt shared file fails both probes. This is intended (one
  durable-memory story = one substrate health gate), but it means a
  journal-only fault and a View-only fault are not independently
  diagnosable at the file level. Acceptable at single-node Phase-1 scope.

### Quality-attribute impact

- **Maintainability — modifiability**: positive. Additive CBOR variants
  per slice; no per-slice migration ceremony.
- **Maintainability — testability**: positive. Sim adapter is an in-memory
  `BTreeMap<(WorkflowId, u32), Vec<u8>>` with injectable fsync-failure,
  mirroring `SimViewStore`; DST invariants extend cleanly.
- **Reliability — recoverability**: positive. redb 1PC+C checksummed
  recovery; bounded resume (range scan per instance, no log replay).
- **Reliability — fault tolerance**: neutral-to-positive. fsync-then-suspend
  ordering pinned by DST invariant.
- **Performance — resource utilisation**: small negative (the in-memory
  journal cursor + the shared `Database`'s journal tables grow with live
  instance count). Bounded by live-workflow cardinality; retention (#208)
  reclaims terminal instances.
- **Compatibility / Portability / Security**: neutral. redb + ciborium
  are both already in the dep graph; pure Rust; no new external surface.

## References

- ADR-0035 — Reconciler memory on redb; the precedent this ADR extends
  (substrate, codec, fsync-ordering, Earned-Trust probe all reused).
- ADR-0048 — rkyv versioned envelope; the discipline this ADR
  *deliberately does not* adopt for the journal (§2 rationale).
- ADR-0023 — Action shim; the engine is driven off the same per-tick
  pipeline (ADR-0064 §5).
- ADR-0037 — `TerminalCondition`; the terminal-modelling precedent
  `WorkflowResult` relates to (ADR-0064 §2).
- ADR-0064 — `Workflow` trait + `WorkflowCtx` + engine boundary
  (companion ADR; the journal is the engine's durable backing).
- Whitepaper §17 *Reconciler Memory* / §18 *The Workflow Primitive*,
  *Primitive Composition*, *Three-Layer State Taxonomy*, *Correctness
  Guarantees* — the SSOT this ADR pins to a concrete shape; already
  carries the "second redb table layout" reconciliation.
- `docs/feature/workflow-primitive/wave-decisions.md` § "RATIFIED
  DIRECTION" (R2 — journal in redb; D4/open-Q3 resolution).
- `docs/feature/workflow-primitive/feature-delta.md` (D-INH-2; US-WP-2;
  K5; slices 01–03).
- `.claude/rules/development.md` § "rkyv schema evolution" vs
  § "Reconciler I/O → Schema evolution" (the two codec disciplines this
  ADR chooses between) + § "Persist inputs, not derived state" (the
  `SleepArmed { deadline }` rule).

## Changelog

- 2026-06-05 — Initial accepted version. Extends ADR-0035 with the
  workflow-journal second table layout; CBOR codec; distinct
  `JournalStore` port on the shared redb substrate.
- 2026-06-05 — Renamed the slice-01 `JournalEntry::CallResult { step,
  correlation, response_digest }` variant to `JournalEntry::RunResult { step,
  name, result_digest, result_bytes }` to back the general `ctx.run<T>`
  durable-step primitive (ADR-0064 §3). `result_bytes` is the CBOR of the
  closure's `T` (byte-equal replay); `result_digest` is the SHA-256 over those
  bytes (replay-equivalence invariant). The per-step `correlation` field is
  removed (unused for replay — the cursor is positional); instance-level
  `CorrelationKey` is unchanged. Greenfield single-cut — slice-01 has no
  breaking journal history, so no version-bump / migration shim; the upgrade
  path is "delete the redb file." User-pinned 2026-06-05.
