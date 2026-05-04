# ADR-0035 — Reconciler memory: collapse trait to one method, typed-View blob auto-persisted, redb backend, in-memory hot copy as steady-state read SSOT

## Status

Accepted. 2026-05-03. Decision-makers: Morgan (proposing); user
ratification 2026-05-03 (mode: propose, single-pass option
selection — Option A from
`docs/feature/reconciler-memory-redb/design/wave-decisions.md`).
Tags: phase-1, reconciler-primitive, application-arch.

**Supersedes ADR-0013** ("Reconciler primitive: trait in
`overdrive-core`, runtime in `overdrive-control-plane`, libSQL
private memory, shipped whole"). ADR-0013 §2, §2a (partial), §2b,
§5, §6, and Alternative G are overturned by this decision; the
parts of ADR-0013 that survive are explicitly enumerated in
*Compliance* below.

**Companion**: ADR-0036 (amendment to ADR-0021 reflecting the
removal of the per-reconciler `hydrate(target, db)` surface).

## Context

Issue #139 began as cleanup of an in-memory `view_cache` shadow above
libSQL (the cache was removed by commit `1ea4ec2` on the
`marcus-sa/libsql-view-cache` branch; the in-flight DELIVER roadmap
of 7 steps was iterating on follow-on shape changes — `migrate`
lifecycle hook, eager `LibsqlHandle::open` at register, etc.). Two
research passes then converged on a fundamental redesign:

- **Reconciler memory abstraction options** (`docs/research/control-
  plane/reconciler-memory-abstraction-options.md`, 30 sources, avg
  reputation 0.97, High confidence): industry consensus across ten
  precedents (Restate, Cloudflare Durable Objects, kube-rs,
  controller-runtime, Anvil OSDI '24, Akka Persistence, Temporal,
  Marten, rkyv, Component Model) is **typed-record persistence as
  the default ergonomic surface**, with SQL or raw KV reserved as
  opt-in escape hatches. Restate ships `ctx.get/set` opaque-typed.
  Cloudflare DOs ship typed KV alongside SQL on the same handle.
  kube-rs offers no persistence — operators reach for sidecars.
  Anvil's verification target is the pure transition function
  regardless of storage shape.
- **redb vs rocksdb as embedded KV for reconciler memory**
  (`docs/research/control-plane/redb-vs-rocksdb-reconciler-memory.md`,
  14 sources, High confidence): rocksdb is structurally disqualified
  by whitepaper §2 design principle 7 (no FFI to C++ in the critical
  path); redb is pure Rust, already in the dep graph (IntentStore
  single-mode + openraft log per whitepaper §17), and the workload
  shape (small blobs, point access, modest cardinality, fsync per
  write) is the canonical COW B-tree fit.

User direction during the present DESIGN conversation pushed
further: reconciler memory should NOT be read on every tick. Bulk-
load at register/boot time into a per-reconciler in-memory
`BTreeMap<TargetResource, View>` kept in RAM as the steady-state
read SSOT; redb is the durable SSOT, written-through on persist.
The trait collapses to a single synchronous method.

The grounding example (`crates/overdrive-core/src/reconciler.rs:1386-1526`):
the existing `JobLifecycle` reconciler's three storage methods
(`migrate` + `hydrate` + `persist`) total ~100 source lines of
which ~95 are mechanical translation between two `BTreeMap` View
fields and two libSQL tables.

## Decision

### 1. Trait surface — single sync method

**`overdrive-core::reconciler::Reconciler` collapses to:**

```rust
pub trait Reconciler: Send + Sync {
    /// Per-reconciler typed projection of intent + observation.
    /// Per ADR-0021 (amended by ADR-0036); unchanged by this ADR.
    type State: Send + Sync;

    /// Per-reconciler typed memory. Persisted as a CBOR blob in the
    /// runtime-owned ViewStore. Author derives the four bounds; the
    /// runtime owns persistence.
    type View: Serialize + DeserializeOwned + Default + Clone + Send + Sync;

    fn name(&self) -> &ReconcilerName;

    /// Pure synchronous transition. No `.await`. No I/O. Wall-clock
    /// only via `tick.now`. Storage is the runtime's responsibility.
    fn reconcile(
        &self,
        desired: &Self::State,
        actual:  &Self::State,
        view:    &Self::View,
        tick:    &TickContext,
    ) -> (Vec<Action>, Self::View);
}
```

The four ADR-0013 §2 trait methods (post-issue-139:
`migrate` / `hydrate` / `reconcile` / `persist`) collapse to one.
The author derives `Serialize + Deserialize + Default + Clone` on
the `View` struct and writes `reconcile`. Nothing else.

The §18 contract is preserved verbatim: `reconcile` is a pure
function over `(desired, actual, view, tick)`. The pre-hydration
*property* is preserved (the author sees a pre-computed `&View`,
not a live handle); the pre-hydration *machinery* moves out of the
trait into the runtime.

### 2. `ViewStore` port — runtime-owned storage abstraction

**`overdrive-control-plane::view_store::ViewStore` (new module):**

```rust
pub trait ViewStore: Send + Sync {
    /// Read every persisted view for a reconciler. Called once per
    /// reconciler at boot, before the first tick. Materialises the
    /// in-memory BTreeMap that becomes the steady-state read SSOT.
    async fn bulk_load<V>(
        &self,
        reconciler: &ReconcilerName,
    ) -> Result<BTreeMap<TargetResource, V>, ViewStoreError>
    where
        V: DeserializeOwned + Send;

    /// Write a single view. Called by the runtime after each
    /// successful `reconcile`. Durable (fsync) before return.
    async fn write_through<V>(
        &self,
        reconciler: &ReconcilerName,
        target:     &TargetResource,
        view:       &V,
    ) -> Result<(), ViewStoreError>
    where
        V: Serialize + Sync;

    /// Delete a view (target retired). Phase 1 deferral acceptable;
    /// leaked rows are bounded by reconciler-kind cardinality, not
    /// by tick count.
    async fn delete(
        &self,
        reconciler: &ReconcilerName,
        target:     &TargetResource,
    ) -> Result<(), ViewStoreError>;

    /// Earned-trust probe (per development.md / Earned Trust principle).
    /// Open file → write probe row → fsync → read back → assert
    /// byte-equal → delete. Composition root invariant: probe before
    /// first bulk_load; failure refuses startup with structured
    /// `health.startup.refused` event.
    async fn probe(&self) -> Result<(), ProbeError>;
}
```

The trait sits in `overdrive-control-plane` (adapter-host class),
**not** in `overdrive-core`. Two reasons:

- The `Reconciler` trait surface is opinionated about *what*
  storage is (Serialize + Deserialize + Default + Clone bounds);
  the runtime owns the trait that abstracts the engine. Symmetric
  with how `EvaluationBroker` is sited
  (`overdrive-control-plane::eval_broker`, not core).
- Putting an `async fn` trait in `overdrive-core` would either
  pull `tokio` into core (rejected by ADR-0013 Alternative A) or
  leak a non-`Send` future shape across the dispatch.

**Production adapter**: `RedbViewStore`
(`overdrive-control-plane::view_store::redb`). One redb file per
node at `<data_dir>/reconcilers/memory.redb`; one redb table per
reconciler kind keyed on `TargetResource::display()`; value is a
CBOR-serialised `View` blob.

**Sim adapter**: `SimViewStore`
(`overdrive-sim::view_store`). In-memory
`BTreeMap<(ReconcilerName, TargetResource), Vec<u8>>` keyed on
CBOR-encoded blobs. Supports injected fsync-failure for the
`WriteThroughOrdering` invariant (§6).

Constructor-required, not builder-defaulted: `ReconcilerRuntime::new`
takes `Arc<dyn ViewStore>` as a mandatory parameter
(per development.md § "Port-trait dependencies" — mandatory in
`new()`, no `with_view_store` builder method, no fallthrough to a
production binding). Tests pass `Arc::new(SimViewStore::new())`;
production passes `Arc::new(RedbViewStore::open(path).await?)`.

### 3. Wire format — CBOR via `ciborium`

CBOR is the persisted shape. `ciborium` (MIT/Apache-2.0; pure Rust;
serde-compatible; no codegen) is the encoder/decoder.

Rejected alternatives:
- **JSON via `serde_json`**: ~10× larger encoded size for typical
  View shapes; otherwise equivalent.
- **rkyv**: explicitly disclaims schema evolution support per its
  FAQ ("lacks a full schema system and isn't well equipped for
  data migration and schema upgrades"). rkyv stays in its current
  role for read-heavy hashed paths (IntentStore archived bytes per
  ADR-0002), NOT for mutable persisted view state.
- **Postcard, bincode**: pure Rust, compact, but neither has the
  `#[serde(default)]` schema-evolution discipline ciborium gets
  for free via serde tolerant deserialization.

Schema evolution: **`#[serde(default)]` on additive fields** —
ignore-unknown-fields-by-default is serde's behaviour and is the
correct shape for additive evolution. Breaking changes use a
versioned envelope:

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "v")]
enum JobLifecycleViewEnvelope {
    #[serde(rename = "1")] V1(JobLifecycleViewV1),
    #[serde(rename = "2")] V2(JobLifecycleViewV2),
}
```

Phase 1 has no breaking-change history; the envelope shape lands
when the first breaking change ships (additive future extension).

### 4. `RedbViewStore` adapter — concrete shape

**Path**: `<data_dir>/reconcilers/memory.redb` (one file per node).

**Tables**: one redb `TableDefinition` per reconciler kind, keyed on
`&str` (the `TargetResource::display()` form), value is `&[u8]`
(the CBOR-encoded View). Table name = `ReconcilerName::display()`.

**Why one file with multiple tables, NOT one file per reconciler**:
- redb's "one file, multiple tables" is the canonical idiom and
  matches IntentStore's existing redb-file usage
  (whitepaper §17, §4 "Single mode — `LocalStore`").
- One file per reconciler (the ADR-0013 §5 shape) was justified by
  filesystem-level isolation under the libSQL design where path
  traversal was a real risk because the `ReconcilerName` was
  embedded in a filesystem path. Under the new design the
  `ReconcilerName` becomes a redb table name (also validated by
  the `^[a-z][a-z0-9-]{0,62}$` regex), and table-name-validation
  is a smaller attack surface than path-validation.
- Operator concern is bounded — `redb-cli` shows tables per file,
  one file is operationally simpler than N files.

**Why this redb file is SEPARATE from the IntentStore redb file**
(per wave-decisions.md §O4 architect recommendation): reconciler
memory has different durability characteristics (per-tick fsync)
than IntentStore (per-submit fsync); mixing the two on a single
redb file would introduce write-amplification cross-talk. One file
per state-layer is the cleaner shape.

**Durability**: redb's default 1PC+C (single fsync per commit +
checksum + monotonic txn id) per the redb-vs-rocksdb research §3.
Each `write_through` is one redb transaction, one fsync, one row
write.

**Crash recovery**: bounded; verify checksum + txn id, walk
allocator state (redb design.md). No WAL replay. Boot-time
`bulk_load` is a single read transaction over the table — no log
scan, no compaction queue.

### 5. `ReconcilerRuntime` extension

Replaces ADR-0013 §2b. Two phases:

**Boot / register-time** (once per reconciler):

```
register(reconciler):
  1. view_store.probe().await?                   (Earned Trust gate)
  2. views = view_store.bulk_load::<R::View>(
         reconciler.name()).await?                (BTreeMap)
  3. registry.insert(name, (AnyReconciler, views))
```

**Steady-state tick** (every `tick_period_ms`, default 100 ms per
ADR-0023):

```
for evaluation in broker.drain_pending():
  1. (any_reconciler, views) = registry.lookup(name)
  2. tick = TickContext::snapshot(clock)         (ADR-0013 §2c — survives)
  3. desired = AnyReconciler::hydrate_desired(...)  (ADR-0021 — survives)
  4. actual  = AnyReconciler::hydrate_actual(...)   (ADR-0021 — survives)
  5. view    = views.get(target).cloned()
                .unwrap_or_else(R::View::default)
  6. (actions, next_view) = reconciler.reconcile(
       &desired, &actual, &view, &tick)
  7. view_store.write_through(name, target, &next_view).await?
                                                  (durable fsync)
  8. views.insert(target.clone(), next_view)      (after fsync OK)
  9. action_shim::dispatch(actions, ...)          (ADR-0023 — survives)
```

**Step ordering 7 → 8 is load-bearing.** If the process crashes
between fsync and the `BTreeMap::insert`, the next boot sees the
persisted view in `bulk_load` — convergence is preserved. The
inverse ordering (memory then disk) would let an acknowledged
tick disappear on crash, breaking durability.

**`BTreeMap`, NOT `HashMap`**, per development.md § "Ordered-
collection choice". The map is drained / iterated on `bulk_load`,
observed by DST invariants; iteration order must be deterministic
across seeds.

### 6. New DST invariants

Added to `overdrive-sim::invariants::Invariant`:

- **`ViewStoreRoundtripIsLossless`** — proptest-backed. For every
  reconciler kind R, for every `View: R::View` value v constructible
  by R's `Arbitrary` impl: `view_store.write_through(name, t, &v)
  .await; view_store.bulk_load(name).await.get(&t)` returns a
  `View` byte-equal to v after CBOR roundtrip. Catches
  serde-derive regressions, ciborium-version skew, schema-evolution
  oversights.
- **`BulkLoadIsDeterministic`** — two `bulk_load` calls against
  the same `RedbViewStore` (or `SimViewStore`) state produce
  `PartialEq`-equal `BTreeMap`s. Catches iteration-order
  regressions in the redb adapter.
- **`WriteThroughOrdering`** — under `SimViewStore` with injected
  fsync-failure on the next call: assert the in-memory map is
  *not* updated. Catches the inverse ordering shape that would
  break durability.

The existing `ReconcilerIsPure` (ADR-0017) continues to fire; the
trait contract is unchanged. `AtLeastOneReconcilerRegistered` and
`DuplicateEvaluationsCollapse` (ADR-0013 §Enforcement) continue to
fire unchanged.

### 7. Compile-time enforcement

Per development.md / Earned Trust principle 12:

- **trybuild** compile-fail fixture asserts `&LibsqlHandle` cannot
  appear anywhere — the type no longer exists, so any reference
  fails to compile. Stronger than the ADR-0013 fixture which
  asserted the handle could not appear in `reconcile` only; under
  this ADR the handle is gone everywhere.
- **trybuild** compile-pass fixture asserts the trait is
  dyn-compatible (no `async fn` in the trait). Pins the property
  the redesign achieves.
- **proptest roundtrip** on every reconciler's `View` (Serialize ↔
  Deserialize bit-identical) — covered by the
  `ViewStoreRoundtripIsLossless` invariant in §6.
- **dst-lint** continues to enforce banned APIs in `core`-class
  crates. The new `ViewStore` trait sits in
  `overdrive-control-plane` (adapter-host); no dst-lint scope
  change.

## Considered alternatives

### Alternative A — Ship the maximalist collapse (ACCEPTED, this ADR)

Above. Single trait method; runtime owns redb + in-memory BTreeMap;
boot-time bulk-load; write-through-on-persist; no escape hatch.

### Alternative B — Collapsed trait with `ReconcilerSql` extension trait escape hatch

Same as A, but reserve `ReconcilerSql: Reconciler` as a marker
trait for first-party reconcilers that legitimately need raw SQL
access (custom indexes, cross-target queries, large memory with
selective loads). Cloudflare-DO-shaped (`ctx.storage.kv` + `ctx
.storage.sql` on the same handle, hidden `__cf_kv` table).

**Rejected** because:

1. **No current reconciler needs it.** The abstraction-options
   research verified by grep that zero existing reconcilers do
   cross-target SQL queries, and no roadmap reconciler does
   either.
2. **The escape hatch is governance.** Reviewers must check "is
   the SQL path justified?" on every PR using it; soft, easy to
   slip.
3. **Two persistence surfaces double the test + invariant
   surface** (a `ReconcilerIsPure` fires for both blob- and
   SQL-backed reconcilers, with different setup).
4. **The user explicitly weighed both options and chose A.**

The escape hatch can ship later, additively, when a real driver
appears. The trait shape in this ADR does not preclude it.

### Alternative C — Status quo with codegen helper (`#[derive(ReconcilerView)]`)

Keep the `migrate` / `hydrate` / `reconcile` / `persist` shape;
reduce ceremony with a derive macro on the View type that
generates the `CREATE TABLE` / `SELECT` / `INSERT` boilerplate
from the View struct's fields.

**Rejected** because:

1. **Displaces the plumbing instead of removing it.** The macro
   becomes a maintained component the project owns. Schema/struct
   desync hazard remains — adding a field requires re-running the
   macro and re-deriving; removing a field leaves a phantom
   column.
2. **Async cognitive load stays.** `hydrate` and `persist` remain
   `async fn` on the trait; the `AnyReconciler` enum-dispatch
   workaround for `async fn` non-dyn-compatibility (ADR-0013 §2a)
   remains structurally necessary.
3. **Steady-state hydrate cost stays at one libSQL roundtrip per
   tick per reconciler** — the bypass is the in-memory cache that
   commit `1ea4ec2` removed because it was a nuisance.
4. **WASM extension story stays soft.** Sandboxed WASM cannot
   ship arbitrary SQL safely; the macro would need a sub-surface
   for WASM (more macro complexity).
5. The user has explicitly chosen the maximalist collapse.

### Alternative D — Event-sourced View (Akka Persistence shape)

Persist a stream of `ViewDelta` records; recover `View` by folding
deltas through a pure `apply(view, delta) -> view` function;
snapshot periodically to bound replay length.

**Rejected** because:

1. **No production precedent in the orchestrator domain.** Akka
   does this for actors; Temporal does this for workflows; no major
   reconciler framework has shipped this shape.
2. **Two pure functions per reconciler instead of one.**
3. **The "audit log for free" benefit is already covered** by the
   `external_call_results` ObservationStore table per
   development.md § "Reconciler I/O".

### Alternative E — Keep libSQL but adopt typed-View blob shape (just swap the two)

Same as A, but persist the CBOR blob in libSQL rather than redb.

**Rejected** because:

1. **redb is already in the dep graph** (IntentStore single-mode +
   openraft log per whitepaper §17); libSQL is also in the dep
   graph (incident memory + DuckLake catalog). Either choice
   carries no new dep. But redb's COW B-tree is the canonical fit
   for the workload (small blobs, point access, modest cardinality,
   fsync per write); libSQL's SQLite engine is a B-tree-with-WAL
   that pays WAL costs the workload doesn't need.
2. **Operational simplicity.** Two storage engines for the same
   workload shape (redb for IntentStore, libSQL for reconciler
   memory) is the worst of both worlds — operators have to know
   both. One engine per state-layer with the same engine for
   intent + reconciler memory keeps the operator surface small.
3. **redb's 1PC+C durability** matches the per-tick fsync workload
   exactly; libSQL's WAL adds replay-on-open latency for no
   workload benefit (per the redb-vs-rocksdb research §3).

## Consequences

### Positive

- **Plumbing per reconciler drops from ~100 LOC to 0.** The author
  derives four serde bounds and writes `reconcile`. Nothing else.
- **Steady-state hydrate cost drops from libSQL roundtrip to
  `BTreeMap::get`** (nanoseconds, no syscall). Cold-start cost is
  one redb read per reconciler at boot.
- **Trait is dyn-compatibility-clean for the first time.** No
  `async fn` in the trait. The `enum AnyReconciler` workaround
  is retained for the typed-View associated-type erasure (the
  rule survives), but the async-fn driver is gone — a future
  `Box<dyn Reconciler>` becomes a real option.
- **Schema evolution by `#[serde(default)]`.** Matches the
  project's additive-only migration discipline. No CREATE TABLE,
  no ALTER TABLE, no row-decoder rewrite.
- **WASM extension story structurally hardened** — no SQL handle
  to expose to a sandbox.
- **Crash recovery bounded** by redb's checksum + monotonic txn id
  (no WAL replay).
- **Operator surface simplified** — one redb file per node for
  reconciler memory replaces N libSQL files (one per reconciler).

### Negative

- **No SQL escape hatch.** A reconciler that legitimately needs
  `SELECT count(*) FROM things WHERE x > N` cannot express it
  efficiently; it materialises the count into View on every tick.
  Mitigation: shipped as additive `ReconcilerSql` extension trait
  (Alternative B) when a real driver appears.
- **In-memory `BTreeMap` per reconciler grows linearly in target
  cardinality.** Phase 1 bound: O(jobs) ≈ KB to MB. Spike target
  (deferred to DELIVER): measure 10k-row View RAM cost before
  Phase-N reconciler sizing crosses a threshold.
- **Operator inspectability degrades** — `redb-cli` shows a CBOR
  blob; decoding requires an `overdrive reconciler view-cat <name>
  <target>` CLI. Tooling cost; not a blocker.
- **Schema-version mismatch on rollback** (v2-written View read by
  v1 code) needs explicit testing — serde with `#[serde(default)]`
  ignores unknown fields, which is the right default but pinned by
  trybuild + integration test.
- **The current branch (`marcus-sa/libsql-view-cache`) supersedes
  itself.** 6 commits implementing the previous design are now
  outdated; the user owns the refactor-in-place vs reset call (per
  wave-decisions.md §O1).

### Quality-attribute impact

- **Performance — time behaviour**: positive (large). Steady-state
  hydrate cost: libSQL roundtrip → `BTreeMap::get`. Order-of-
  magnitude latency improvement on the tick critical path.
- **Performance — resource utilisation**: negative (small,
  bounded). RAM grows by `Σ over reconcilers of (target_count ×
  sizeof(View))`. Phase 1: KB to MB. Spike target measures
  Phase-N threshold.
- **Maintainability — modifiability**: positive (large). 100 → 0
  LOC plumbing per reconciler.
- **Maintainability — testability**: positive. Single trait
  method; `SimViewStore` is an in-memory `BTreeMap`; DST
  invariants extend cleanly.
- **Reliability — fault tolerance**: neutral. Crash semantics
  match libSQL-equivalent durability via redb 1PC+C.
- **Reliability — recoverability**: positive. Bounded recovery
  vs libSQL's WAL-replay-on-open shape.
- **Compatibility — coexistence**: neutral. redb already in dep
  graph; ciborium is one new pure-Rust serde format library.
- **Security — confidentiality**: neutral.
- **Portability**: neutral. redb is pure Rust.

## Compliance — what survives from ADR-0013

ADR-0013 is superseded by this ADR. The following ADR-0013
sections **survive verbatim** under ADR-0035:

- **§1 (Module ownership)** — `Reconciler` trait still in
  `overdrive-core::reconciler`; `ReconcilerRuntime` still in
  `overdrive-control-plane::reconciler_runtime`.
- **§2c (Time injection — `TickContext`)** — entirely preserved.
  `tick: &TickContext` is still the fourth `reconcile` parameter;
  the runtime still snapshots `Clock::now()` once per evaluation.
- **§3 (Action enum — Phase 1 shape)** — preserved.
- **§4 (`ReconcilerName` newtype, `^[a-z][a-z0-9-]{0,62}$` regex)**
  — preserved; the regex is now the redb-table-name source instead
  of a filesystem-path source.
- **§7 (Slice 4 ships whole)** — historical.
- **§8 (Evaluation broker shape)** — entirely preserved.
- **§9 (`noop-heartbeat` reconciler)** — preserved; trivially
  satisfies the new shape (`type View = ()`).
- **§Enforcement** triple-defence shape — preserved, with the
  trybuild fixture extended (the handle is gone everywhere, not
  just from `reconcile`'s parameter list).

The following ADR-0013 sections are **superseded** by this ADR:

- **§2 (Trait shape — pre-hydration pattern)** — the four-method
  shape is replaced by a single sync `reconcile`. The pre-
  hydration *property* survives (the `view` is a pre-computed
  input); the pre-hydration *machinery* moves out of the trait
  into the runtime.
- **§2a (Dyn-compatibility strategy — `enum AnyReconciler`)** —
  the `AnyReconciler` enum is *retained* for typed-View
  associated-type erasure, but the `async fn` driver of dyn-
  incompatibility goes away. A future `Box<dyn Reconciler>`
  becomes structurally possible.
- **§2b (Runtime hydrate-then-reconcile contract)** — replaced by
  this ADR §5.
- **§5 (libSQL per-primitive path derivation)** — replaced by the
  single per-node `<data_dir>/reconcilers/memory.redb` file with
  one redb table per reconciler kind.
- **§6 (Per-primitive storage — libSQL via `LibsqlHandle`)** —
  replaced by the typed-View blob via `ViewStore` (this ADR §2).
  `LibsqlHandle` is deleted.
- **Alternative G (Sync `reconcile` with `block_on` over libsql's
  async API)** — academic; libsql is no longer in the trait
  surface.

The following ADR-0013 sections are **partially overturned**
(survive in part):

- **§Enforcement bullet "Reconciler trait is in `overdrive-core`;
  the dst-lint gate scans any core-class crate that imports it"**
  — survives. The new `ViewStore` trait is in
  `overdrive-control-plane`, not core; dst-lint scope is unchanged.

## References

- ADR-0013 — superseded; the load-bearing predecessor.
- ADR-0017 — `overdrive-invariants`; `ReconcilerIsPure` is
  preserved.
- ADR-0021 — `AnyState` enum; amended in part by ADR-0036.
- ADR-0023 — Action shim placement + 100 ms tick cadence;
  preserved.
- Whitepaper §17 — Storage Architecture; amended (see
  upstream-changes.md §B1).
- Whitepaper §18 — Reconciler and Workflow Primitives; trait shape
  example amended (see upstream-changes.md §B2).
- `.claude/rules/development.md` § Reconciler I/O — rewritten in
  DELIVER (see upstream-changes.md §C1).
- `docs/research/control-plane/reconciler-memory-abstraction-options.md`
  — Restate / Cloudflare DOs / kube-rs / Anvil / Akka / Temporal /
  Marten / rkyv / Component Model / controller-runtime; 30 sources;
  avg reputation 0.97; High confidence.
- `docs/research/control-plane/redb-vs-rocksdb-reconciler-memory.md`
  — redb is pure Rust + already in dep graph + canonical workload
  fit; rocksdb structurally disqualified by whitepaper §2 principle 7.
- `docs/feature/reconciler-memory-redb/design/wave-decisions.md`
  — Option A rationale + alternatives + open questions.
- `docs/feature/reconciler-memory-redb/design/upstream-changes.md`
  — full enumeration of artifacts that need updating.

## Changelog

- 2026-05-03 — Initial accepted version. Supersedes ADR-0013.
- 2026-05-04 — Additive extension: runtime Eq-diff skip on
  `persist_view`. The runtime compares `next_view` against the
  in-memory value (`PartialEq` on `&Self::View`) and skips both
  `ViewStore::write_through` and the in-memory map insert when
  equal. Motivation: elide the per-tick fsync on no-op ticks (a
  converged target whose reconciler emits `Noop` and an unchanged
  view). Trait surface change: `Reconciler::View` gains an `Eq`
  bound (was `Serialize + DeserializeOwned + Default + Clone +
  Send + Sync`; now adds `Eq`). The fsync-then-memory ordering
  for the non-equal case is unchanged and remains pinned by the
  `WriteThroughOrdering` invariant (§6); the equality check is
  pinned by the `runtime_skips_write_through_when_next_view_equals_in_memory`
  integration test in
  `crates/overdrive-control-plane/tests/integration/reconciler_runtime_view_store.rs`.
  An alternative `ViewAction::{Noop, Update(V)}` enum at the
  reconciler return site was considered and rejected: runtime
  Eq-diff pushes zero discipline onto reconciler authors and
  cannot be silently miscoded.
