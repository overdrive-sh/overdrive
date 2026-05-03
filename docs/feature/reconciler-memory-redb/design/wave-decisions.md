# DESIGN — reconciler-memory-redb

**Wave**: DESIGN | **Architect**: Morgan (Application) | **Mode**: propose | **Date**: 2026-05-03

---

## Context

This feature began as a libSQL view-cache cleanup (the `view_cache`
in-memory shadow already removed by commit `1ea4ec2` on the current
branch) and converged during the present conversation on a fundamental
redesign of the reconciler-memory abstraction. The in-flight DELIVER
roadmap (7 steps, branch `marcus-sa/libsql-view-cache`, 6 commits
landed) is the *previous* design. This DESIGN supersedes it.

The trigger was friction concrete enough to grep:
`crates/overdrive-core/src/reconciler.rs:1386-1526` (the
`JobLifecycle` reconciler) carries ~100 lines of `migrate` / `hydrate`
/ `persist` plumbing per reconciler — CREATE TABLE, SELECT +
row-decoding, DELETE+INSERT in a per-row loop — that does nothing
reconciler-specific. Two research passes converged independently:

- `docs/research/control-plane/reconciler-memory-abstraction-options.md`
  (Restate / Cloudflare DOs / kube-rs / Anvil / Akka / Temporal /
  Marten / rkyv / Component Model / controller-runtime — 30 sources,
  avg reputation 0.97, High confidence) → typed-View blob is the
  industry-converged ergonomic surface.
- `docs/research/control-plane/redb-vs-rocksdb-reconciler-memory.md`
  → redb is the right backing store; rocksdb is structurally
  disqualified by whitepaper §2 design principle 7.

The user pushed further during the conversation: reconciler memory
should NOT be read on every tick. Bulk-load at register/boot time
into one `BTreeMap<TargetResource, View>` per reconciler, kept in
RAM as the steady-state read SSOT; redb is the durable SSOT
written-through on persist. The trait collapses to a single
synchronous method.

---

## Mode

**Propose.** Architect reads SSOT + research, presents 2–3 concrete
options with trade-offs; user accepts/redirects once at the end.

---

## Reading Checklist

See preceding architect message — every mandatory file in the prompt
was read; no skips.

---

## Reuse Analysis (HARD GATE)

Existing assets evaluated for EXTEND vs REPLACE vs DELETE before any
new component is proposed:

| Asset | Location | Verdict | Justification |
|---|---|---|---|
| `Reconciler` trait | `crates/overdrive-core/src/reconciler.rs` (4 methods: `migrate` / `hydrate` / `persist` / `reconcile`) | **REPLACE** | The trait shape itself is the cost; collapsing to one method is the entire feature. Cannot be additively migrated — the new shape removes three of the four methods. |
| `ReconcilerRuntime` | `crates/overdrive-control-plane/src/reconciler_runtime.rs` | **EXTEND** | Stays the runtime; gains a boot-time bulk-load step, an in-memory `BTreeMap<TargetResource, View>` per reconciler, and a write-through path on persist. The evaluation broker, registration model, and tick-loop shape all survive. |
| `EvaluationBroker` | `crates/overdrive-control-plane/src/eval_broker.rs` | **UNCHANGED** | Storage shape change does not touch broker semantics. ADR-0013 §8 stays valid. |
| `LibsqlHandle` newtype | `crates/overdrive-core/src/reconciler.rs` | **DELETE** | The rationale (give reconciler authors raw libSQL) dissolves. No reconciler holds a handle in the new design. |
| `libsql_provisioner` module | `crates/overdrive-control-plane/src/libsql_provisioner.rs` | **DELETE in this scope; reconsider for incident-memory in Phase 3** | Path provisioner for per-reconciler libSQL files is no longer needed — redb takes over with a single file and a key-namespace scheme (see §Decision 3). The path-validation logic (canonicalise + `starts_with` defence-in-depth) is generally useful and may be lifted into the redb-key namespace check, but that is a copy of the *idea*, not the file. The libSQL story for incident memory (whitepaper §17, Phase 3+) is a separate problem; if it needs path provisioning at that point, the module is recreated then, against the Phase-3 requirements. |
| In-memory `view_cache` | (formerly `reconciler_runtime.rs`) | **DELETE** | Already removed by commit `1ea4ec2` on the current branch. Flag for continuity: the new in-memory `BTreeMap<TargetResource, View>` is **not** a re-introduction of `view_cache` — it is the steady-state read SSOT, hydrated once at boot, rather than a tick-grain cache layered above libSQL. The two have different invariants. |
| `redb` dependency | Already in `Cargo.toml` workspace deps (used by `IntentStore`, openraft log) | **EXTEND** | One redb file under a new path (`<data_dir>/reconcilers/memory.redb`) with one table per reconciler keyed by `(reconciler_name, target_resource)`. No new third-party dep. Whitepaper §17 already names redb as the embedded ACID KV. |
| `libsql` workspace dep | `Cargo.toml` workspace deps | **RETAIN for incident memory only** | Whitepaper §12 (incident memory) and §17 (telemetry catalog via DuckLake) still call for libSQL. Removing it from reconciler memory does not remove the dep from the workspace; reconcilers stop reaching for it. |
| `AnyReconciler` / `AnyReconcilerView` / `AnyState` enum-dispatch | `crates/overdrive-core/src/reconciler.rs` (per ADR-0013 §2a + ADR-0021) | **EXTEND** | Enum-dispatch convention survives unchanged. The associated-type and the variant per reconciler stay; only the `migrate` / `hydrate` / `persist` arms in the dispatch impl are removed. |
| `Reconciler::View` associated type bound | `Send + Sync` today | **EXTEND** | Add `Serialize + DeserializeOwned + Default + Send + Sync + Clone` (Clone for the bulk-load → in-memory copy). |
| `TickContext` | `overdrive-core::reconciler` | **UNCHANGED** | ADR-0013 §2c stays valid; time injection has no relationship to storage shape. |

Net: **one new component (`ViewStore`)**, **one trait replacement
(`Reconciler`)**, **one runtime extension (`ReconcilerRuntime`)**,
**three deletions (`LibsqlHandle`, `libsql_provisioner`, the
collapsed trait methods)**.

---

## Options Considered

### Option A — Maximalist collapse (single-method `Reconciler`; in-memory BTreeMap is read SSOT)

Trait collapses to:

```rust
trait Reconciler: Send + Sync {
    type State: Send + Sync;
    type View:  Serialize + DeserializeOwned + Default + Clone + Send + Sync;

    fn name(&self) -> &ReconcilerName;
    fn reconcile(
        &self,
        desired: &Self::State,
        actual:  &Self::State,
        view:    &Self::View,
        tick:    &TickContext,
    ) -> (Vec<Action>, Self::View);
}
```

No `async`, no `migrate`, no `hydrate`, no `persist`, no DB handle.

A new `ViewStore` component (runtime-owned) is the durable
abstraction over redb. At boot the runtime calls
`ViewStore::bulk_load(name) -> BTreeMap<TargetResource, View>` for
each registered reconciler and stores the result alongside the
`AnyReconciler` in the registry. Steady-state reads come from the
in-memory map; `reconcile` returns `(Vec<Action>, NextView)`; the
runtime persists the `NextView` through `ViewStore::write_through`
(write to redb fsync, then write to the in-memory map) before
acknowledging the tick. WASM third-party reconcilers see a
Component-Model `record` boundary against `Self::View`; the SQL
surface is structurally unavailable.

No escape hatch.

**Strengths**:

- Plumbing per reconciler drops from ~100 lines to zero. The author
  derives `Serialize + Deserialize + Default + Clone` on the View
  struct and writes `reconcile`. Nothing else.
- Steady-state hydrate cost is a `BTreeMap::get` — nanoseconds, not
  a libSQL roundtrip per tick. Cold-start cost is one redb read per
  reconciler at boot.
- Schema evolution by `#[serde(default)]` matches the project's
  additive-only migration discipline (CLAUDE.md). No CREATE TABLE,
  no ALTER TABLE, no row-decoder rewrite.
- Trait is dyn-compatibility-clean for the first time — no
  `async fn`. The `enum AnyReconciler` workaround in ADR-0013 §2a
  is still kept for the typed-View dispatch (associated-type
  erasure), but the async-fn driver is gone.
- WASM extension story (whitepaper §18 *Extension Model*) is
  structurally hardened — no SQL handle to expose to a sandbox.
- Bit-identical DST replay preserved: the `BTreeMap` is initialised
  deterministically from redb (or a `SimViewStore` in-memory map
  under simulation); write-through is deterministic; `reconcile`
  remains a pure function over `(desired, actual, view, tick)`.
- ESR verifiability preserved: Anvil's `reconcile_core` shape is
  verbatim — pure transition over inputs, runtime owns I/O. The
  proof obligation does not grow with the storage swap.
- **Persist inputs, not derived state** (development.md) is enforced
  identically to today — the View is the inputs; storage shape is
  irrelevant to the rule.
- redb is already in the dep graph (whitepaper §17 / §4 — IntentStore
  single-mode, openraft log). No new third-party dep. Pure Rust.
  Whitepaper §2 design principle 7 satisfied.

**Weaknesses**:

- No SQL escape hatch. A reconciler that legitimately needs `SELECT
  count(*) FROM things WHERE x > N` cannot express it efficiently;
  it materialises the count into View on every tick.
- `BTreeMap<TargetResource, View>` per reconciler grows linearly in
  target cardinality. Phase 1 bound: O(jobs); Phase 2+ bound: O(jobs
  × allocations × ...) per reconciler kind. At hundreds-of-MB scale
  this is fine; at 10k+ targets per reconciler this becomes a real
  RAM budget the design must acknowledge. Spike target #1 in the
  abstraction-options research (10k-row view, CBOR encode + redb
  write latency) measures the exact number.
- Whole-View serialisation per persist. For Phase 1 view sizes (KB)
  this is in the noise; for Phase-N hypothetical large views this is
  quadratic in the same shape the current `JobLifecycle::persist`
  documents as a Phase-1 expedient. The diff-merge `NextView`
  associated type is the additive future extension when this bites
  (research Recommendation Lane).
- Operator inspectability degrades — `redb-cli` shows a CBOR blob;
  decoding requires an `overdrive reconciler view-cat <name>
  <target>` CLI. Not a blocker but a real cost.
- Schema-version mismatch on rollback (v2-written View read by v1
  code) needs explicit testing — serde with `#[serde(default)]`
  ignores unknown fields, which is the right default but should be
  pinned by trybuild + integration test.
- The `LibsqlHandle` newtype, `libsql_provisioner` module, and three
  trait methods all delete in the same PR. The current branch has 6
  commits implementing the *previous* design. Refactor cost is real
  (see open question O1).

### Option B — Collapsed trait with `ReconcilerSql` extension trait escape hatch

Same as Option A, but reserve a marker extension trait:

```rust
trait Reconciler: Send + Sync { /* as Option A */ }

trait ReconcilerSql: Reconciler {
    async fn migrate_sql(&self, db: &SqlHandle) -> Result<(), HydrateError>;
    async fn hydrate_sql(
        &self,
        target: &TargetResource,
        db:     &SqlHandle,
    ) -> Result<Option<Self::View>, HydrateError>;
    async fn persist_sql(
        &self,
        target: &TargetResource,
        view:   &Self::View,
        db:     &SqlHandle,
    ) -> Result<bool, HydrateError>;
}
```

Cloudflare-DO-shaped (Finding 6 in the abstraction-options research).
Two registration paths on the runtime: `register::<R: Reconciler>` for
the blob-default path; `register_sql::<R: ReconcilerSql>` for the SQL
path. WASM third-party always blob-only.

Hydrate priority: if a reconciler implements `ReconcilerSql` and
`hydrate_sql` returns `Some(view)`, the SQL view overrides the
blob-loaded view; otherwise the runtime reads from the
in-memory/redb path. Persist priority: if `persist_sql` returns
`true`, the reconciler took ownership; otherwise the runtime
persists through the blob path.

**Strengths**:

- 90% of reconcilers (every existing one + every Phase-2 one in the
  whitepaper §18 list) get Option A's ergonomics.
- The 10% that need SQL (cross-target queries, custom indexes, large
  memory with selective loads) get the full Option-A-prior surface
  opt-in. No reconciler is forced to materialise an aggregation into
  View on every tick.
- WASM extension story preserved — the WASM trait surface only
  exposes `Reconciler`, never `ReconcilerSql`. SQL is host-Rust-only.
- Backwards-compatible if the architect ever wants to add SQL later
  — the marker trait can ship in a follow-up release.

**Weaknesses**:

- Two persistence surfaces to test, document, DST-replay. Twice the
  invariant catalogue (a `ReconcilerIsPure` fires for both blob-
  and SQL-backed reconcilers, but the pre-condition setup differs).
- The escape hatch is governance — reviewers must check "is the SQL
  path justified?" on every PR that uses it. Soft, easy to slip.
- Trait surface grows from one method to four (one mandatory + three
  on a sister trait). Some of the simplification benefit is lost.
- **No current reconciler needs it.** The abstraction-options research
  verified by grep that zero existing reconcilers do cross-target
  SQL queries. Shipping the escape hatch in v1 builds infrastructure
  for a hypothetical future caller.
- The user explicitly pushed back on this option during the
  conversation. The "no concrete reconciler needs it; ship blob-only
  and add the escape hatch when a real driver appears" position is
  the user's stated preference.

### Option C — Status quo with ergonomic reduction (no architectural change)

Keep `migrate` + `hydrate` + sync `reconcile` (today's shape, with
the `persist` method added by issue-139's earlier passes). Reduce
ceremony with codegen — a `#[derive(ReconcilerView)]` macro on the
View type that generates the boilerplate:

- For each `BTreeMap<K, V>` field of the View, emit a `CREATE TABLE
  IF NOT EXISTS view_<field>` (key column = `K::display`, value
  column = `serde_json::Value` or `BLOB`).
- Emit `hydrate_<field>` and `persist_<field>` helpers; the author's
  `hydrate` body becomes `Ok(Self::View { field_a:
  hydrate_field_a(db).await?, field_b: hydrate_field_b(db).await? })`.

**Strengths**:

- Zero architectural risk. The trait surface, the runtime contract,
  and the existing 6 commits on the branch all stay valid.
- Operator inspectability stays at full SQL — `sqlite3 reconciler.db`
  shows columnar tables, not blobs.
- Cross-target queries remain trivially expressible — the View
  decoding is per-target, but the underlying SQL surface supports
  arbitrary queries the macro doesn't generate.
- libSQL stays the embedded engine (whitepaper §4 / §17 still names
  it); no second SQLite-flavour-equivalent dependency.

**Weaknesses**:

- The plumbing problem is *displaced*, not *removed*. The macro hides
  the SQL but the schema/struct desync hazard remains — adding a
  field to View requires re-running the macro and re-deriving;
  removing a field leaves a phantom column. Schema evolution is
  still ALTER TABLE; serde-default-style additivity does not exist.
- The macro becomes a load-bearing component the project has to
  maintain. Today's hand-written SQL can be debugged with `cargo
  expand`; macro-generated SQL needs more tooling.
- Async cognitive load stays. `hydrate` and `persist` remain `async
  fn` on the trait — `AnyReconciler` still needs the enum-dispatch
  workaround for dyn-compatibility (ADR-0013 §2a).
- Steady-state hydrate cost remains a libSQL roundtrip per tick per
  reconciler — bypassable in production by an in-memory cache (which
  is what commit `1ea4ec2` removed because it was a nuisance), but
  the cost is paid in DST and in tick wall-clock.
- WASM extension story stays soft — sandboxed WASM cannot ship
  arbitrary SQL safely (Finding 8 in the abstraction-options
  research). Either the macro generates a sub-surface for WASM
  (more macro complexity) or WASM gets a different trait (two-trait
  problem).
- `block_on` over libSQL's async API is still rejected (ADR-0013
  Alternative G — research §9), so `reconcile` stays sync-pure but
  every tick still pays an `.await` round-trip on `hydrate` /
  `persist`. The pre-hydration / `TickContext` machinery
  ADR-0013 §2 / §2c added is *all* there to dodge this cost; under
  Option A the machinery becomes unnecessary because the
  in-memory-BTreeMap is the source of truth.
- The user has explicitly chosen the maximalist collapse; Option C
  preserves a shape the user finds friction with.

---

## Trade-off Matrix

| Dimension | A: Maximalist collapse | B: Hybrid + `ReconcilerSql` escape | C: Status quo + macro |
|---|---|---|---|
| Lines of plumbing per reconciler | ~0 | ~0 (default) / ~100 (escape) | ~0 (macro-hidden) |
| Cognitive load per reconciler | Low (1 method, sync) | Medium (1 method + 3 escape methods) | Medium (4 methods + macro semantics) |
| Steady-state hydrate cost | `BTreeMap::get` (ns) | `BTreeMap::get` or libSQL roundtrip | libSQL roundtrip per tick |
| Schema evolution (additive) | `#[serde(default)]` | Either | Macro-generated ALTER (unclear) |
| Schema evolution (breaking) | Versioned envelope + custom upcaster | Either | Manual ALTER TABLE + backfill |
| Cross-target queries | Impossible | Possible via SQL escape | Trivially expressible |
| Cross-target queries actually used today | NO (verified by grep) | — | — |
| Operator inspectability | Blob (needs CLI) | Hybrid | Full SQL |
| DST replay determinism | Preserved | Preserved | Preserved |
| ESR (Anvil) verifiability | Preserved | Preserved | Preserved |
| WASM third-party safety | Strong (typed Component-Model record) | Strong (blob path only) | Weak (SQL surface in sandbox) |
| Engineering risk to ship | Moderate (clean replacement) | Moderate (two paths) | Low (additive macro) |
| Refactor cost on current branch | High (replaces 6 commits' direction) | High (same — still replaces) | Low (additive) |
| Production precedent | Restate, kube-rs, Cloudflare KV path | Cloudflare DO (KV + SQL) | Bespoke |
| Whitepaper §17 alignment | Amend (libSQL → redb for reconciler memory) | Amend (libSQL → redb + libSQL escape) | Unchanged |
| Storage tier complexity | One engine per tier (redb) | Two engines per tier (redb + libSQL) | One engine per tier (libSQL) |
| Number of embedded engines on a node | Three (redb intent, redb obs, redb reconciler) — ALL redb | Four (libSQL added) | Four (libSQL added) |
| Persist inputs, not derived state (rule) | Identical compliance | Identical compliance | Identical compliance |

---

## Recommendation

**Option A — Maximalist collapse. Confidence: High.**

### Why A over B

The only material upside of B over A is "preserves the SQL escape
hatch for a hypothetical future reconciler." The abstraction-options
research verified by grep that **zero existing reconcilers use
cross-target SQL queries**, and the user has stated that no current
roadmap reconciler needs it either. Building infrastructure for a
hypothetical caller is exactly the resume-driven shape the
sa-critique-dimensions skill calls out (CRITICAL severity).

If a future reconciler genuinely needs SQL, it can be added then —
as an `Action::SqlQuery`-shaped capability or as a new
`ReconcilerSql` extension trait, additively. The trait surface in
Option A does not preclude this addition; it just refuses to
*pre-build* it.

### Why A over C

C displaces the plumbing instead of removing it. The macro becomes a
maintained component. The async cognitive load stays. The steady-
state hydrate cost stays. The WASM extension story stays weak. The
`AnyReconciler` enum-dispatch workaround for `async fn` stays
necessary. C optimises the *symptom* (LOC per reconciler); A removes
the *cause* (the trait shape forcing per-tick I/O).

The user has explicitly chosen A. C is an honest alternative for
completeness; the user's reasoning (the per-reconciler SQL ceremony
is unjustified friction) is sound.

### What would change the recommendation

1. **A worst-case Phase-N reconciler with 10k+ target rows per
   instance ships before the ergonomic gain compounds.** The
   in-memory BTreeMap RAM cost crosses a threshold that
   `redb`-roundtrip-per-tick cost would not. Mitigation: Spike target
   #1 in the abstraction-options research measures the exact
   threshold; the result is either "fine through Phase 5" (likely)
   or "Option B's escape hatch becomes load-bearing rather than
   theoretical" (unlikely but checkable).
2. **A reconciler appears that genuinely needs cross-target SQL
   *before* Phase 2 ships.** Then Option B is the right shape and
   the design re-opens. The grep evidence says this is not on the
   roadmap.
3. **rkyv adds documented schema-evolution support and the wire-
   format choice re-opens.** rkyv would replace ciborium and the
   read path becomes zero-copy. Out of scope for this DESIGN; this
   is the abstraction-options research Gap 1.

---

## Decision

**Adopt Option A.** Concrete shape:

### D1. Trait surface

```rust
// in overdrive-core::reconciler
pub trait Reconciler: Send + Sync {
    /// Per-reconciler typed projection of intent + observation.
    /// Per ADR-0021; unchanged.
    type State: Send + Sync;

    /// Per-reconciler typed memory. Persisted as a CBOR blob in the
    /// runtime-owned ViewStore. `Default` for new targets that have
    /// no prior persisted view; `Clone` for the bulk-load → BTreeMap
    /// copy and the persist-then-cache write-through path; serde
    /// bounds for the wire format.
    type View: Serialize + DeserializeOwned + Default + Clone + Send + Sync;

    fn name(&self) -> &ReconcilerName;

    /// Pure synchronous transition. No `.await`. No I/O. Wall-clock
    /// only via `tick.now`. Storage I/O lives in the runtime.
    fn reconcile(
        &self,
        desired: &Self::State,
        actual:  &Self::State,
        view:    &Self::View,
        tick:    &TickContext,
    ) -> (Vec<Action>, Self::View);
}
```

`migrate` / `hydrate` / `persist` are deleted. `LibsqlHandle` is
deleted. The pre-hydration pattern in ADR-0013 §2 is preserved
*conceptually* — `view` is still a pre-computed input — but the
async hydration phase moves out of the trait into the runtime, and
the read source for steady-state ticks is the in-memory `BTreeMap`,
not a per-tick libSQL roundtrip.

### D2. `ViewStore` component (NEW)

```rust
// in overdrive-control-plane::view_store (NEW MODULE)
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
    /// successful `reconcile`. Durable (fsync) before return; the
    /// in-memory BTreeMap update is the runtime's responsibility,
    /// after this call returns Ok.
    async fn write_through<V>(
        &self,
        reconciler: &ReconcilerName,
        target:     &TargetResource,
        view:       &V,
    ) -> Result<(), ViewStoreError>
    where
        V: Serialize + Sync;

    /// Delete a view (the target no longer exists). Optional;
    /// called when the runtime detects a target has been retired.
    /// Phase 1 deferral acceptable — leaked rows are bounded by
    /// the lifetime of the reconciler kind, not by tick count.
    async fn delete(
        &self,
        reconciler: &ReconcilerName,
        target:     &TargetResource,
    ) -> Result<(), ViewStoreError>;
}
```

The trait sits in `overdrive-control-plane` (adapter-host class),
**not** in `overdrive-core`. Two reasons:

- The reconciler trait surface itself is opinionated about *what*
  storage is — it just demands a Serialize + Deserialize bound. The
  runtime owns the trait that abstracts the engine.
- The dst-lint posture: `overdrive-core` has no I/O dependencies;
  putting a `ViewStore` trait there with an `async fn` would either
  pull `tokio` into core (rejected) or leak a non-`Send` future
  shape into the dispatch (worse). The runtime-side placement is
  consistent with how `EvaluationBroker` is sited (`overdrive-
  control-plane::eval_broker`, not core).

The DST-controllability point of view (per development.md § "Port-
trait dependencies"): `ViewStore` is a port; `RedbViewStore` is the
production adapter; `SimViewStore` is the simulation adapter (an
in-memory `BTreeMap<(ReconcilerName, TargetResource), Vec<u8>>`
keyed on CBOR-encoded blobs). Wiring crates pick host; tests pick
sim. Builder-pattern overrides are the anti-pattern; constructor-
required is the correct shape.

### D3. `RedbViewStore` adapter

One redb file per node at `<data_dir>/reconcilers/memory.redb`
(replaces the per-reconciler libSQL files at
`<data_dir>/reconcilers/<name>/memory.db` from ADR-0013 §5).

Schema: one redb table per reconciler kind (`alloc_status_v1`,
`job_lifecycle_v1`, ...), keyed on `TargetResource::display()`,
value is a CBOR-serialised `View` blob. The "one file, multiple
tables" shape is the canonical redb idiom and matches IntentStore's
existing redb-file usage (whitepaper §17, §4 "Single mode —
`LocalStore`").

Wire format: **CBOR via `ciborium`**, per the abstraction-options
research recommendation. Pure Rust, no codegen, serde-compatible,
`#[serde(default)]` covers additive evolution. NOT rkyv (rkyv
explicitly disclaims schema evolution per its FAQ — Finding 9).
NOT JSON (CBOR is one order of magnitude smaller for typical View
sizes).

Durability: redb's default 1PC+C (single fsync per commit + checksum
+ monotonic txn id) per the redb-vs-rocksdb research §3. Each
`write_through` is one redb transaction, one fsync, one row write.

Crash recovery: bounded; verify checksum + txn id, walk allocator
state. No WAL replay. Boot-time `bulk_load` is a single read
transaction over the table — no log scan, no compaction queue.

Path-traversal safety: `ReconcilerName` regex
(`^[a-z][a-z0-9-]{0,62}$`) is preserved as the table-name source.
The `libsql_provisioner`-style canonicalisation + `starts_with`
defence-in-depth is no longer needed because the path is now a
single per-node file, not a name-derived path.

### D4. `ReconcilerRuntime` extension

Boot sequence (replaces ADR-0013 §2b step 2 + 4):

```
register(reconciler):
  1. ViewStore::bulk_load::<R::View>(reconciler.name())
       .await? → BTreeMap<TargetResource, R::View>
  2. Stash (AnyReconciler, BTreeMap<TargetResource, View-erased blob>)
     in registry.
  (No libSQL handle. No per-reconciler file.)
```

Tick loop (replaces ADR-0013 §2b steps 4–6):

```
for evaluation in broker.drain_pending():
  1. registry lookup → (AnyReconciler, in-memory BTreeMap)
  2. tick = TickContext::snapshot(clock)         (unchanged, ADR-0013 §2c)
  3. desired = AnyReconciler::hydrate_desired(...)  (unchanged, ADR-0021)
  4. actual  = AnyReconciler::hydrate_actual(...)   (unchanged, ADR-0021)
  5. view    = btreemap.get(target).cloned()
                .unwrap_or_else(R::View::default)
  6. (actions, next_view) = reconciler.reconcile(
       &desired, &actual, &view, &tick)
  7. ViewStore::write_through(name, target, &next_view).await?
                                                  (durable fsync)
  8. btreemap.insert(target.clone(), next_view)   (after fsync OK)
  9. action_shim::dispatch(actions, ...)          (unchanged, ADR-0023)
```

The "fsync before in-memory update" ordering is load-bearing. If
the process crashes between fsync and the `BTreeMap::insert`, the
next boot sees the persisted view in `bulk_load` — convergence is
preserved. The inverse ordering (memory then disk) would let an
acknowledged tick disappear on crash, breaking the durability
contract.

The in-memory `BTreeMap` is `BTreeMap`, NOT `HashMap`, per
development.md § "Ordered-collection choice" — drained / iterated
on `bulk_load`, observed by DST invariants, must be deterministic
across seeds.

### D5. Whitepaper §17 amendment

Replace the "Reconciler Memory — libSQL (per-reconciler)" tier with:

> ### Reconciler Memory — redb (one file per node, runtime-owned key
> space, in-memory hot copy)
>
> Each reconciler's typed `View` is persisted in a single per-node
> redb file under `<data_dir>/reconcilers/memory.redb`, with one
> redb table per reconciler kind keyed on the `TargetResource`. The
> runtime bulk-loads every persisted view at register time into a
> per-reconciler in-memory `BTreeMap<TargetResource, View>`; this
> map is the steady-state read SSOT for `reconcile`. Writes flow
> through as `View → CBOR bytes → redb transaction (one fsync) →
> in-memory map update`, in that order. Reconcile remains a pure
> synchronous function over `(desired, actual, view, tick)` — the
> `View` is hydrated by the runtime's BTreeMap lookup, not by the
> reconciler. CBOR via `ciborium` carries the wire format; serde
> with `#[serde(default)]` handles additive schema evolution.
> libSQL is retained for incident memory and DuckLake catalog
> (Phase 3+), not for reconciler memory.

### D6. ESR + DST property preservation

Preserved verbatim:

- `ReconcilerIsPure` (ADR-0017): twin invocation with identical
  `(desired, actual, view, tick)` → bit-identical `(Vec<Action>,
  NextView)`. The View is now CBOR-decoded then cloned from the
  BTreeMap rather than libSQL-decoded; the trait contract is
  unchanged.
- `AtLeastOneReconcilerRegistered`: post-boot registry non-empty.
- `DuplicateEvaluationsCollapse`: broker semantics unchanged.

Added:

- `ViewStoreRoundtripIsLossless`: for every reconciler kind R, for
  every `View: R::View` value v, `ViewStore::write_through(name,
  t, &v).await; ViewStore::bulk_load(name).await.get(&t)` returns a
  `View` byte-equal to v. Property test under proptest with the
  `Arbitrary` impl on every reconciler's View.
- `BulkLoadIsDeterministic`: two `bulk_load` calls against the same
  redb file produce equal `BTreeMap`s (`PartialEq`-equal). Catches
  iteration-order regressions in the redb adapter.
- `WriteThroughOrdering`: under `SimViewStore` with injected
  fsync-failure on the next call, the in-memory map is *not*
  updated. Catches the inverse ordering that would break
  durability.

### D7. WASM third-party reconciler ABI (forward-looking)

The Component Model `record` is the View boundary. A WASM
reconciler declares:

```wit
interface reconciler {
    record view { /* author-defined fields */ }
    record action { /* enumeration mapped to Rust Action */ }
    record state { /* author-defined */ }
    record tick { now: u64, tick: u64, deadline: u64 }

    reconcile: func(desired: state, actual: state, view: view, tick: tick)
        -> tuple<list<action>, view>;
}
```

The host serialises `View` via Canonical ABI on the way in,
deserialises on the way out, and persists through the same
`ViewStore` shape. No SQL handle is exposed to the sandbox — the
`ReconcilerSql` extension trait would be host-Rust-only if it ever
ships.

This is a forward-looking note, NOT a Phase 1 deliverable. Phase 1
ships first-party Rust reconcilers only.

---

## Quality-attribute impact (ISO 25010)

| Attribute | Impact | Evidence |
|---|---|---|
| Performance — time behaviour | **Positive (large)**. Steady-state hydrate cost: libSQL roundtrip → `BTreeMap::get`. Order-of-magnitude latency improvement on the tick critical path. | `BTreeMap::get` is O(log n) with no syscalls; libSQL roundtrip is at minimum one `read(2)` plus parse. Phase-1 reconciler latency budget unchanged at 100 ms tick (ADR-0023); the reduction lifts headroom. |
| Performance — resource utilisation | **Negative (small, bounded)**. RAM grows by `Σ over reconcilers of (target_count × sizeof(View))`. Phase 1: O(jobs × ~1KB) ≈ KB to MB. Phase N hypothetical: spike target #1 measures threshold. | redb-vs-rocksdb research §workload-recap; abstraction-options research §7 spike #1. |
| Maintainability — modifiability | **Positive (large)**. Plumbing per reconciler 100 → 0 LOC. Schema evolution `#[serde(default)]` instead of ALTER TABLE. | Direct line-count comparison against `JobLifecycle` reconciler. |
| Maintainability — testability | **Positive**. Single trait method; `SimViewStore` is an in-memory `BTreeMap`; DST invariants extend cleanly (D6). | New invariants covered in D6. |
| Reliability — fault tolerance | **Neutral**. Crash semantics: redb's 1PC+C + boot-time bulk-load match libSQL-equivalent durability. Write-through ordering (fsync before BTreeMap update) preserves the durability contract. | redb-vs-rocksdb research §3. |
| Reliability — recoverability | **Positive**. Bounded recovery (verify checksum, walk allocator) vs libSQL's WAL-replay-on-open shape. | redb design.md (cited in research). |
| Compatibility — coexistence | **Neutral**. redb is already in the dep graph (IntentStore, openraft log). No new third-party dep. | Whitepaper §4, §17. |
| Security — confidentiality | **Neutral**. Reconciler memory is per-node private state; the file lives under the same data_dir as IntentStore + ObservationStore. Same operator-facing surface. | No change. |
| Portability | **Neutral**. redb is pure Rust; no platform-specific dependencies introduced. | redb-vs-rocksdb research Finding 1. |
| Functional suitability — correctness | **Positive (small)**. Deletes an entire class of bug (schema/struct desync in hand-written `migrate`/`hydrate`). The compiler proves View shape-correctness via serde derive. | development.md § "Persist inputs, not derived state" (rule preserved by construction). |

---

## Open questions for the user

**O1. Refactor the current branch in place, or reset?** The branch
`marcus-sa/libsql-view-cache` has 6 commits implementing the
*previous* design (libSQL hydrate/persist + `LibsqlHandle` newtype).
Two paths:

- **Refactor in place**: revert the libSQL-specific commits; keep
  the `view_cache` removal commit (`1ea4ec2`) and any commits that
  are still relevant; layer the new design on top. Lower
  blast-radius; the branch history reads as evolution.
- **Reset**: drop the branch, start fresh from `main`. Cleaner
  history; the in-flight DELIVER work is fully discarded. Higher
  blast-radius; but the new design supersedes most of what the
  branch did anyway.

The architect's recommendation is **reset**, on these grounds: (1)
the trait surface change is structural — `migrate` / `hydrate` /
`persist` all delete in the same PR — and the branch's commits are
specifically about populating those methods. (2) The new design
deletes `LibsqlHandle` entirely, which the branch's first commits
introduce. (3) A new DELIVER roadmap (which the user owns; DESIGN
does not produce roadmaps) will be cleaner against `main` than
against a partially-undone branch. The user owns this call.

**O2. Per-reconciler tables vs single global table?** The
`RedbViewStore` design above has one redb table per reconciler kind.
An alternative shape is one global table keyed on `(ReconcilerName,
TargetResource)`. The per-table shape gives blast-radius isolation
between reconcilers (a corrupt key in one reconciler does not affect
another's bulk_load); the global shape is simpler. Architect's
recommendation: per-reconciler tables, matching the per-reconciler
file isolation that ADR-0013 §5 originally bought. The user can
override.

**O3. Spike target #1 — measure RAM cost before shipping?** The
abstraction-options research flagged a 10k-row synthetic
`JobLifecycleView`-shaped View encoding cost as a spike that, if
>5ms per write, makes Option B's escape hatch load-bearing. The
architect's recommendation is to defer this to DELIVER (the spike
is cheap and measurable in-tree against the new `RedbViewStore`),
but the user can elevate it to a DESIGN-time gate if Phase-N
reconciler sizing is on the critical path.

**O4. redb file shared with IntentStore, or separate?** Whitepaper
§4 / §17 already names redb for IntentStore single-mode. The
reconciler memory could live in the same file with a separate table
namespace (one less file to manage) or in a separate file (blast-
radius isolation between the two stores). The redb-vs-rocksdb
research Open Question #1 deferred this to the architect.
**Recommendation: separate file.** Reconciler memory has different
durability characteristics (per-tick fsync) than IntentStore
(per-submit fsync); mixing the two on a single redb file would
introduce write-amplification cross-talk. One file per state-layer
is the cleaner shape.

---

## Architecture style enforcement

**Hexagonal — preserved.** The `ViewStore` is a new port; the
`Reconciler` trait is the existing port (collapsed). Wiring crates
pick host adapters (`RedbViewStore`); tests pick sim adapters
(`SimViewStore`). The trait surfaces in `overdrive-core` and
`overdrive-control-plane` map cleanly to the existing brief.md §1
*Architectural style* declaration.

**No new architectural rule violations.** The dst-lint gate
(brief.md §1, ADR-0006) continues to scan core-class crates for
banned APIs. The new `ViewStore` trait sits in `overdrive-control-
plane` (adapter-host); `RedbViewStore` is in
`overdrive-control-plane`; `SimViewStore` is in `overdrive-sim`
(adapter-sim, dev-only). The dependency direction is preserved:
`overdrive-core ← overdrive-control-plane`; nothing in core depends
on the runtime adapter.

**Enforcement tooling, language-appropriate (Earned Trust principle 11):**

- **dst-lint** continues to enforce banned APIs in `core`-class
  crates.
- **trybuild** compile-fail fixture asserts that `&LibsqlHandle`
  cannot appear anywhere — the type no longer exists, so any
  reference fails to compile. (This is stronger than the existing
  fixture which asserted the handle could not appear in
  `reconcile` only; under Option A the handle is gone everywhere.)
- **proptest roundtrip** on every reconciler's `View` (Serialize ↔
  Deserialize bit-identical) — covered by D6's
  `ViewStoreRoundtripIsLossless` invariant, which is a property
  test, not just an integration test.
- **trybuild compile-fail fixture** asserting the trait is
  dyn-compatible (no `async fn` in the trait), pinning the property
  the redesign achieves.
- **DST invariants**: D6's three new invariants
  (`ViewStoreRoundtripIsLossless`, `BulkLoadIsDeterministic`,
  `WriteThroughOrdering`) execute against `SimViewStore` on every
  `cargo xtask dst` run.

---

## Earned Trust (principle 12) — adapter probes

`RedbViewStore` is a driven adapter against the real redb engine
(filesystem, fsync semantics). Per the rule, it ships with a
`probe()` method:

```rust
impl RedbViewStore {
    pub async fn probe(&self) -> Result<(), ProbeError> {
        // 1. Open the redb file (or refuse to start).
        // 2. Begin a write transaction; insert a probe row;
        //    commit (forces fsync).
        // 3. Begin a read transaction; assert the probe row reads
        //    back byte-equal.
        // 4. Delete the probe row; commit.
        // 5. On any step failure → ProbeError with structured cause;
        //    composition root logs `health.startup.refused`
        //    and exits non-zero.
    }
}
```

The fault-injection scenarios the probe must survive:

- **Read-only filesystem** — fail at step 2 with
  `ProbeError::WriteFailed`. (The existing libsql_isolation test
  fixture already covers this shape; the reconciler-memory
  equivalent inherits it.)
- **fsync no-op (Docker overlayfs, tmpfs)** — Phase 1 acceptable;
  Phase 5+ probe extends to an explicit fsync-write-then-crash-then-
  read sequence under the Image Factory's known-bad substrate
  catalog (whitepaper §23 deferred topic). Flagged in
  upstream-changes.md.
- **Disk full** — fail at step 2 with `ProbeError::CommitFailed`.

Composition-root invariant: `ReconcilerRuntime::register` calls
`view_store.probe()` once before the first `bulk_load`; probe
failure surfaces as `ControlPlaneError::Internal` and the runtime
refuses to start. This matches the ADR-0013 §6 "register-time
failure surfaces synchronously" property.

---

## Handoff to acceptance-designer (DISTILL)

The user has not requested a DISTILL pass for this feature; the
DESIGN output goes directly to DELIVER planning at the user's
discretion. If a DISTILL pass is invoked later:

- Source of AC: this wave-decisions.md + ADR-0035 + ADR-0036.
- Trait surface and `ViewStore` port are stable; test scenarios
  can name `Reconciler::View`, `ViewStore`, `RedbViewStore`,
  `SimViewStore` directly.
- AC are observable through the new DST invariants
  (`ViewStoreRoundtripIsLossless`, `BulkLoadIsDeterministic`,
  `WriteThroughOrdering`), the trybuild fixtures, and the
  `cargo xtask dst` output.

## Handoff to platform-architect (DEVOPS)

- External integrations: **none added by this DESIGN**. redb is
  already in the dep graph; ciborium is a new pure-Rust serde
  format library with no platform-build requirements. No contract
  tests recommended.
- CI integration: existing gates continue to apply
  (`cargo xtask dst`, `cargo xtask dst-lint`, `cargo nextest run`,
  mutation-testing kill rate). New DST invariants in §D6 are
  additive on `cargo xtask dst`.
- New workspace dep: `ciborium` (MIT/Apache-2.0; pure Rust; serde-
  compatible; no codegen). Add via `[workspace.dependencies]` per
  development.md § "Dependencies" rule.
- Architecture rules continue to be enforced by dst-lint + trybuild
  + DST invariants. No new rule infrastructure required.

---

*End of wave-decisions.md.*
