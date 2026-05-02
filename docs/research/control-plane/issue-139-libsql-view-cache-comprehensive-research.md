# Issue #139 — Replace in-memory `view_cache` with libSQL diff-and-persist

Implementer's reference for replacing `AppState::view_cache` with the
ADR-0013 §2b hydrate-then-reconcile contract using per-primitive libSQL.

Branch state at research time: `marcus-sa/reconciler-view-cache-comment`,
3 modified files (`lib.rs`, `reconciler_runtime.rs`, `streaming.rs`),
**comments only** — no implementation has begun. 0 commits ahead of
`origin/main`.

---

## 1. Surface that changes

### 1.1 Production fields, types, helpers being deleted

| File:line | Symbol | Action |
|---|---|---|
| `crates/overdrive-control-plane/src/lib.rs:109` | `AppState::view_cache: Arc<Mutex<BTreeMap<(String, String), CachedView>>>` | DELETE |
| `crates/overdrive-control-plane/src/lib.rs:142-148` | `pub enum CachedView { Unit, JobLifecycle(JobLifecycleView) }` | DELETE |
| `crates/overdrive-control-plane/src/lib.rs:183` | `view_cache: Arc::new(Mutex::new(BTreeMap::new()))` (inside `AppState::new`) | DELETE field init |
| `crates/overdrive-control-plane/src/reconciler_runtime.rs:355-357` | `fn cache_key(...)` | DELETE |
| `crates/overdrive-control-plane/src/reconciler_runtime.rs:361-382` | `fn cached_view_or_default(...)` | DELETE |
| `crates/overdrive-control-plane/src/reconciler_runtime.rs:385-400` | `fn store_cached_view(...)` | DELETE |
| `crates/overdrive-control-plane/src/streaming.rs:108` | `use crate::CachedView;` | DELETE |
| `crates/overdrive-control-plane/src/streaming.rs:345-358` | `fn read_view(view_cache, job_id) -> JobLifecycleView` | REWRITE (new source) |
| `crates/overdrive-control-plane/src/handlers.rs:30` | `use crate::CachedView;` | DELETE |
| `crates/overdrive-control-plane/src/handlers.rs:614-622` | `fn read_job_lifecycle_view(state, job_id) -> JobLifecycleView` | REWRITE (new source) |

### 1.2 Production logic that changes shape

`crates/overdrive-control-plane/src/reconciler_runtime.rs:254-267` —
the discard-and-cache pattern in `run_convergence_tick`:

```rust
// TODO(#139): wire `LibsqlHandle` for real per ADR-0013 §2b and
// use the returned view directly; drop the discard-and-cache
// shape below.
let db = LibsqlHandle::default_phase1();
let _ = reconciler.hydrate(target, &db).await.map_err(ConvergenceError::Hydrate)?;
let view = cached_view_or_default(reconciler, target, state);
```

Replace with: open/reuse a `LibsqlHandle` for the named reconciler from
the runtime, call `hydrate`, use the returned view directly.

`crates/overdrive-control-plane/src/reconciler_runtime.rs:299-304` — the
write path:

```rust
// Persist the next-view back into the in-memory cache.
// TODO(#139): replace with libSQL diff-and-persist per ADR-0013 §2b.
store_cached_view(reconciler, target, state, next_view);
```

Replace with libSQL diff-and-persist (full-View replacement per Phase 1
convention; see §7).

### 1.3 Stub being deleted

`crates/overdrive-core/src/reconciler.rs:233-242`:

```rust
/// Phase 1 default handle — no underlying libSQL connection. The
/// runtime tick loop hands this to every `Reconciler::hydrate`
/// call until Phase 2+ wires per-primitive libSQL files.
/// Reconcilers that touch the handle in Phase 1 are a bug — every
/// Phase 1 reconciler's `View = ()` (or carries no row data) and
/// returns `Ok(default)` without using the handle.
#[must_use]
pub const fn default_phase1() -> Self {
    Self { _handle: None }
}
```

`LibsqlHandle::default_phase1` is the only out-of-test caller anywhere
(grep: `default_phase1` is referenced in `reconciler_runtime.rs:265` and
the bugfix RCA at `docs/feature/fix-eval-reconciler-discarded/deliver/bugfix-rca.md:171` — same line). Delete the constructor; the
`#[allow(dead_code)] empty()` constructor at lines 227-231 may also be
removed depending on the new shape.

The internals of `LibsqlHandle` itself (`crates/overdrive-core/src/reconciler.rs:204-217`) need to grow real connection state. Today
they're explicitly placeholder:

```rust
pub struct LibsqlHandle {
    // Phase 1: the connection handle is `Option::None` because no
    // current reconciler opens its DB. ...
    // Typed as `Arc<()>` for now rather than `Arc<libsql::Connection>`
    // so the core crate does not pull libsql onto its compile graph
    // until a reconciler author actually needs a connection.
    _handle: Option<Arc<()>>,
}
```

Note: `overdrive-core/Cargo.toml:54` already lists
`libsql.workspace = true`, so the dep is already on the core compile
graph (used for `HydrateError::Libsql(#[from] libsql::Error)` at
`reconciler.rs:269`). The "doesn't pull libsql onto its compile graph"
comment is stale.

### 1.4 Tests that depend on the in-memory cache shape

Grep over `crates/overdrive-control-plane/tests` shows two acceptance
tests reference `view_cache` directly:

1. `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs:192-207, 283-313, 469-484`
   — `eval_dispatch_runs_only_the_named_reconciler` and
   `stop_after_failed_alloc_drains_broker` both observe the cache to
   prove dispatch routing / convergence behaviour. Counting strategy
   (verbatim from `runtime_convergence_loop.rs:191-194`):

   > Counting strategy: every reconciler that runs through
   > `run_convergence_tick` writes a `(reconciler_name, target_string)`
   > entry into `AppState::view_cache` via `store_cached_view`
   > (`reconciler_runtime.rs:248`). The cache is `pub` and observable
   > from the test:

   These tests must be rewritten to observe libSQL state instead, or
   change their counting strategy entirely (e.g. count broker
   `dispatched` deltas, or count distinct emitted lifecycle events).

2. `crates/overdrive-control-plane/src/streaming.rs:520, 542, 557`
   — the inline `#[cfg(test)]` `check_terminal_returns_converged_stopped_on_terminated_event` test takes
   `view_cache` as a `BTreeMap` literal:

   ```rust
   let view_cache = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
   ```

   Test must be migrated to whatever new view-source signature
   `check_terminal` ends up with.

3. `crates/overdrive-control-plane/tests/acceptance/alloc_status_snapshot.rs` — referenced by Grep but not opened in this
   research; touches `JobLifecycleView` / `restart_counts` /
   `next_attempt_at` as a value type. The handler-side migration
   (`handlers.rs:614`) drives whatever changes are needed here.

4. `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs` — also references `JobLifecycleView` directly (Grep). These uses are likely just constructing or destructuring the value
   type; the type itself stays.

Other tests under `tests/integration/job_lifecycle/{submit_to_running,
stop_to_terminated, crash_recovery, ...}.rs` exercise the convergence
loop end-to-end. They observe through the obs store and lifecycle
events, not `view_cache` directly — they pass through unchanged unless
the `JobLifecycleView` semantics need to change (they don't, per §7).

### 1.5 Cargo + workspace state already in place

- Workspace dep: `Cargo.toml:70`
  `libsql = { version = "0.5", default-features = false, features = ["core"] }`
- Already pulled into:
  - `crates/overdrive-core/Cargo.toml:54` → `libsql.workspace = true`
    (used for `libsql::Error` in `HydrateError`)
  - `crates/overdrive-control-plane/Cargo.toml:107` →
    `libsql = { workspace = true }` (used by `libsql_provisioner.rs`)
- The provisioner is fully implemented at
  `crates/overdrive-control-plane/src/libsql_provisioner.rs` — both
  `provision_db_path` and `open_db` (returning a `libsql::Database`)
  exist and are tested at
  `crates/overdrive-control-plane/tests/integration/libsql_isolation.rs`.

The path provisioner home the issue mentions is real and ready.

---

## 2. ADR-0013 contract — verbatim quotes

### 2.1 §2b runtime contract (the spec the issue calls out)

Verbatim from `docs/product/architecture/adr-0013-reconciler-primitive-runtime.md:178-202`:

> ### 2b. Runtime hydrate-then-reconcile contract
>
> The runtime's tick loop for each dispatched `Evaluation`:
>
> ```
> 1. Pick reconciler from registry by name           (enum dispatch)
> 2. Open (or reuse cached) LibsqlHandle for name    (path from §5)
> 3. tick <- TickContext { now: clock.now(),         (snapshot once;
>                           tick: counter,             see §2c)
>                           deadline: now + budget }
> 4. view <- reconciler.hydrate(target, db).await    (async; runtime owns the .await)
> 5. (actions, next_view) =
>        reconciler.reconcile(&desired, &actual,     (sync; pure function)
>                             &view, &tick)
> 6. Persist diff(view, next_view) to libsql         (runtime owns the write)
> 7. Dispatch actions to the runtime's action shim   (Phase 3)
> ```
>
> The runtime never hands `&LibsqlHandle` to `reconcile`. Writes are
> expressed as data in `NextView`, persisted by the runtime. Reconcile
> remains pure over its inputs — DST-replayable and ESR-verifiable
> (research §1.1, §10.5).
>
> Phase 1 convention: `NextView = Self::View` (full replacement). The
> runtime diffs against the prior view and persists the delta.
> Full-View replacement is simplest and imposes no per-author
> diff-protocol. A typed-diff shape (`NextView = ViewDiff<View>`) is an
> additive future extension when View size makes re-serialisation
> costly; deferred until a real reconciler drives the need (research
> Recommendation Lane).

### 2.2 §5 path derivation

Verbatim from ADR-0013 lines 393-417:

> ### 5. libSQL per-primitive path derivation
>
> ```
> <data_dir>/reconcilers/<reconciler_name>/memory.db
> ```
>
> - `data_dir` — defaults to `~/.local/share/overdrive` (XDG), overridable
>   by `--data-dir` at control-plane startup.
> - `reconciler_name` — validated `ReconcilerName`; by construction
>   cannot contain `..`, `/`, `\`, or other path-traversal characters.
> - `memory.db` — the libSQL database file. One per reconciler, full
>   filesystem isolation.
>
> The path provisioner in `overdrive-control-plane::reconciler_runtime`:
>
> - Canonicalises `data_dir` via `std::fs::canonicalize` at startup (or
>   creates it if missing) to resolve symlinks once.
> - Concatenates `<canonicalised_data_dir>/reconcilers/<name>/memory.db`.
> - Asserts the resulting path starts with `<canonicalised_data_dir>/reconcilers/`
>   — defence-in-depth in case the newtype regex ever regresses.
> - Creates the directory tree and opens the libSQL file.
> - Returns an `Arc<Db>` handle exclusive to that reconciler.

The provisioner already implements all of this except the "creates the
directory tree and opens" + "Arc<Db> handle" composition — those are
spread across `provision_db_path` and `open_db` and need to be glued
into `ReconcilerRuntime`.

### 2.3 §6 LibsqlHandle

Verbatim from ADR-0013 lines 419-446:

> ### 6. Per-primitive storage — libSQL via `LibsqlHandle`
>
> **Workspace adds `libsql` as the per-primitive private-memory backend.**
>
> - License: MIT (Turso fork of SQLite). Pure Rust.
> - Version: 0.5.x lineage per workspace pin; exact revision chosen at
>   implementation time.
> - Usage: one libSQL connection per reconciler, owned by the runtime,
>   exposed to `hydrate` as `&LibsqlHandle` (a newtype wrapping the
>   live `libsql::Connection`). The handle is **only** visible inside
>   `hydrate`; `reconcile` never sees it. Writes produced by
>   `reconcile` flow through the returned `NextView`, diffed and
>   persisted by the runtime.
> - No migration framework in Phase 1 — schemas are per-reconciler and
>   the runtime does not manage them. The `noop-heartbeat` reconciler
>   uses `type View = ()` and writes nothing.
>
> `LibsqlHandle` is a real newtype over `Arc<libsql::Connection>` (or
> equivalent) — not a placeholder. The type exists from step 04-01a
> onward even though `noop-heartbeat`'s `hydrate` ignores it (its View
> is the unit type).

Schema management (ADR-0013 lines 448-453):

> Schema evolution is the author's responsibility, at the View-struct
> level. Changing `Self::View` is the compile-time trigger to revisit
> the `CREATE TABLE` / `ALTER TABLE` statements inside `hydrate`. This
> matches the Elm-style "typed Model, compiler proves shape-correctness"
> pattern (research §10.3). Framework-level migrations are deferred to
> Phase 3+.

### 2.4 §2 trait shape (already implemented)

The `Reconciler` trait at `crates/overdrive-core/src/reconciler.rs:302-372`
already matches the §2 prescription verbatim — `hydrate(target, db) ->
Future<View>` async, `reconcile(desired, actual, view, tick) -> (Vec<Action>, NextView)` sync. The trait is **non-dyn-compatible** (associated types
+ `async fn` in trait) and uses `enum AnyReconciler` dispatch
(ADR-0013 §2a; implemented at `reconciler.rs:804-913`).

### 2.5 Whitepaper §17 / §18 cross-references

`docs/whitepaper.md:1950-1952` (§17 Reconciler Memory):

> Each reconciler gets a private libSQL database for stateful memory
> across reconciliation cycles — restart tracking, placement history,
> resource sample accumulation. Reconciler DB writes are strictly
> private; cluster mutations always route through a typed store — the
> IntentStore for intent, the ObservationStore for observation — never
> through the reconciler's private DB. The three consistency models
> (private libSQL, linearizable Raft, eventually-consistent CR-SQLite)
> never mix.

§18 also pins the contract; ADR-0013 is the operational expansion and
should be the primary reference.

### 2.6 Constraints not obvious from issue body

- **Reconciler authors own SQL and schema** (ADR-0013:65-69 hydrate
  doc, §6 schema-evolution clause). The runtime does NOT hand authors
  a managed `restart_counts` table — the `JobLifecycle::hydrate` impl
  must contain the `CREATE TABLE IF NOT EXISTS` for
  `restart_counts` and `next_attempt_at` (or a single combined table)
  before any `SELECT`. The runtime's job is to provide the connection,
  not the schema.

- **`JobLifecycle::hydrate` body today is a stub** — at
  `crates/overdrive-core/src/reconciler.rs:991-1002`:

  ```rust
  async fn hydrate(
      &self,
      _target: &TargetResource,
      _db: &LibsqlHandle,
  ) -> Result<Self::View, HydrateError> {
      // Phase 1 02-02 carries the View shape; the libSQL hydrate
      // path itself (CREATE TABLE IF NOT EXISTS, SELECT decode)
      // lands in 02-03 alongside the runtime tick loop. For 02-02
      // a fresh empty View is sufficient — the convergence loop is
      // not yet driven, so the View has no rows to materialise.
      Ok(JobLifecycleView::default())
  }
  ```

  Implementing #139 means writing the real CREATE TABLE / SELECT in
  this body. The View struct itself
  (`reconciler.rs:1323-1331`) is `restart_counts: BTreeMap<AllocationId, u32>` plus `next_attempt_at: BTreeMap<AllocationId, Instant>`.

- **Pre-hydration is the contract reason `reconcile` is sync.** The
  rule chain (ADR-0013:103-117): libsql 0.5 is async-only ⇒ pre-hydration
  ⇒ `reconcile` stays pure/sync ⇒ ESR + DST replay. Removing pre-hydration
  is not on the table, even if it would simplify wiring.

- **`reconcile` purity is enforced at three layers** (ADR-0013:610-643): trait signature, dst-lint, and the
  `reconciler_is_pure` DST invariant. The compile-fail fixture at
  `crates/overdrive-core/tests/compile_fail/reconcile_cannot_take_libsql_handle.rs` pins this: passing `&LibsqlHandle` into
  `reconcile`'s parameter list must fail with E0053 (verified
  `.stderr` at the same path). The fixture is unaffected by #139 and
  must continue to pass.

---

## 3. Existing libSQL infrastructure

### 3.1 Workspace + crate deps

Workspace pin: `Cargo.toml:70`:

```toml
libsql = { version = "0.5", default-features = false, features = ["core"] }
```

In-graph already:
- `crates/overdrive-core/Cargo.toml:54` (`libsql.workspace = true`)
- `crates/overdrive-control-plane/Cargo.toml:107`
  (`libsql = { workspace = true }`)

Search for `rusqlite` / `sqlx-sqlite` — neither appears anywhere
(workspace-wide grep returns no hits). libSQL is the sole SQLite
flavour, per ADR-0013 Alternative E rejection.

### 3.2 `LibsqlHandle` impl/usage outside the stub

`LibsqlHandle` is referenced in 19 files. Only three are production
Rust:
- `crates/overdrive-core/src/reconciler.rs` — defines the struct + the
  `default_phase1` / `empty` stub constructors
- `crates/overdrive-control-plane/src/reconciler_runtime.rs:265`
  — the `default_phase1()` call site (the only place a handle is built
  in production)
- `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs` — references
  `LibsqlHandle` for tests but does not construct one outside the
  `run_convergence_tick` path

There is no production code that opens a real libSQL connection for a
reconciler today. That is exactly what this issue creates.

### 3.3 `LibsqlPathProvisioner` — already implemented as
`provision_db_path` + `open_db`

`crates/overdrive-control-plane/src/libsql_provisioner.rs` provides:

- `pub fn provision_db_path(data_dir: &Path, name: &ReconcilerName) -> Result<PathBuf, ControlPlaneError>` — derives, canonicalises, defence-in-depth
  checks. Tested at `tests/integration/libsql_isolation.rs:27-100`.
- `pub async fn open_db(path: &Path) -> Result<libsql::Database, ControlPlaneError>` — `Builder::new_local(path).build().await`
  with parent dir creation. Tested at `libsql_isolation.rs:106-128`.
  Returns `libsql::Database`, not `libsql::Connection` — the provisioner
  doc at lines 84-87 explains: `Database::connect()` is the sync factory
  for connections.

`ReconcilerRuntime::register` already calls `provision_db_path`
eagerly at registration time
(`reconciler_runtime.rs:115-127`):

```rust
pub fn register(&mut self, reconciler: AnyReconciler) -> Result<(), ControlPlaneError> {
    let name = reconciler.name().clone();
    if self.reconcilers.contains_key(&name) {
        return Err(ControlPlaneError::Conflict { ... });
    }
    // Path derivation only — surfaces permission / traversal errors
    // at register time rather than deferring to first DB open.
    let _path = provision_db_path(&self.data_dir, &name)?;
    self.reconcilers.insert(name, reconciler);
    Ok(())
}
```

The `_path` is currently discarded. The implementation needs to either
store the path for lazy open, or open eagerly and cache the
`libsql::Database` / connection alongside the reconciler.

### 3.4 Existing libSQL adapters in `overdrive-host` / `overdrive-sim`

None. Grep `LibsqlHandle | libsql::` in `crates/overdrive-host` and
`crates/overdrive-sim` returns no files. The `Sim*` adapter pattern
(see §4) is not yet applied to libSQL — there is no `SimLibsqlHandle`.

### 3.5 Reconciler `hydrate` impls — current state

There are exactly two `Reconciler::hydrate` impls in the codebase
(`AnyReconciler` only has `NoopHeartbeat` and `JobLifecycle` variants):

1. `NoopHeartbeat::hydrate` at `crates/overdrive-core/src/reconciler.rs:770-776`
   — `Ok(())`, ignores both args. Stays this way (no memory needed).

2. `JobLifecycle::hydrate` at `crates/overdrive-core/src/reconciler.rs:991-1002`
   — currently `Ok(JobLifecycleView::default())`, ignores `_db`. This
   is the impl that grows real SQL.

`AnyReconciler::hydrate` dispatch at `reconciler.rs:829-840` already
threads `&LibsqlHandle` through; nothing to change in the enum
plumbing.

---

## 4. Sim/host trait split for libSQL

### 4.1 Rule (verbatim)

`.claude/rules/development.md` § "Port-trait dependencies" pins the
shape:

> | Crate | Class | Use | Cargo.toml placement |
> | `overdrive-host` | `adapter-host` | Production bindings (`SystemClock`, `OsEntropy`, `TcpTransport`, …) | `[dependencies]` of crates that ship a production wiring |
> | `overdrive-sim` | `adapter-sim` | Simulation bindings (`SimClock`, `SimTransport`, `SimEntropy`, `SimDriver`, `SimObservationStore`, `SimLlm`) | `[dev-dependencies]` of any crate whose tests need DST controllability |

And § "Production code is not shaped by simulation":

> **Production code MUST NOT carry extra logic, extra arms, extra
> yields, extra polling, or extra structural concessions whose only
> purpose is to make a `Sim*` adapter behave correctly under DST.**

### 4.2 What this means for LibsqlHandle

ADR-0013 §6 (lines 436-437) is explicit:

> `LibsqlHandle` is a real newtype over `Arc<libsql::Connection>` (or
> equivalent) — not a placeholder.

`LibsqlHandle` is NOT a port-trait — it is a concrete newtype wrapping
a libsql connection. There is no `Clock`-shaped trait abstraction over
SQLite-flavoured storage.

The reasoning chain:

1. libSQL is pure-Rust embedded SQLite (no network, no kernel calls
   that need DST control).
2. The non-determinism it would add to DST is wall-clock leakage
   (e.g. SQLite's `CURRENT_TIMESTAMP`). Schema authors are warned not
   to depend on it; the View carries `Instant` values plumbed in from
   `tick.now`, never read from SQL.
3. DST controllability comes from running libSQL against an
   in-process file (per-test `:memory:` URL, or a `tempfile::TempDir`
   path). No simulator needed — same trick `LocalIntentStore` uses
   over redb.

### 4.3 Precedent — `IntentStore` and `ObservationStore` are traits;
`LocalStore` is the production impl that runs everywhere

- `IntentStore` trait — `crates/overdrive-core/src/traits/intent_store.rs`
- `LocalIntentStore` — `crates/overdrive-store-local`, used in BOTH
  production AND tests (no `SimIntentStore` exists). Tests get
  determinism by using a fresh `tempfile::TempDir` per test and
  controlling redb directly. See e.g.
  `tests/integration/libsql_isolation.rs:29-30`:

  ```rust
  let tmp = TempDir::new().expect("tempdir");
  let data_dir = tmp.path().to_path_buf();
  ```

- `ObservationStore` — has both production (`LocalObservationStore`,
  `crates/overdrive-store-local`) AND simulation (`SimObservationStore`,
  `crates/overdrive-sim`) impls because Corrosion has gossip / partition
  / SWIM behaviour DST controls; libSQL has none of that.

The `LocalIntentStore` precedent is the one to follow for libSQL: one
production type, used in tests with a per-test temp dir.

### 4.4 Production-code-not-shaped-by-sim implications

The issue body raises the test-shape question:

> Whatever wiring lands MUST keep DST replay deterministic. libSQL
> backend must be reachable through the same trait surface DST
> controls; if a sim adapter is needed it lives in `overdrive-sim`. See
> `.claude/rules/development.md` § "Production code is not shaped by
> simulation" — Sim adapter must NOT impose a shape on the production
> hot path.

Concretely: if you add a `SimLibsqlHandle` for tests, every production
call site must continue to work against the real `LibsqlHandle` shape
**without** any branches, traits, or arms whose only purpose is to
satisfy the sim. Given libSQL itself is pure-Rust embedded SQLite
already deterministic given the same input bytes, the recommended path
is **no sim adapter** — use a real `LibsqlHandle` in tests too,
backed by a `tempfile::TempDir` (or `:memory:`).

---

## 5. DST replay implications

### 5.1 Other state-bearing components — how they handle DST

`LocalIntentStore` (`crates/overdrive-store-local`) — production redb
backend, used in tests against `tempfile::TempDir`. From
`runtime_convergence_loop.rs:222-223`:

```rust
let store_path = tmp.path().join("intent.redb");
let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
```

Tests are deterministic because:
- `redb` reads/writes are deterministic given the same input order,
- temp dir is fresh per test,
- DST harness controls clock + transport but not storage.

`SimObservationStore` (`crates/overdrive-sim`) — separate sim impl
because Corrosion has gossip-delay + partition-matrix knobs DST needs
to inject. Verified by Cargo dep at
`crates/overdrive-control-plane/Cargo.toml:122`:

```toml
[dev-dependencies]
overdrive-sim.path       = "../overdrive-sim"
```

### 5.2 `:memory:` vs `TempDir` for libSQL tests

libSQL's `Builder::new_local(":memory:")` is the SQLite in-memory
mode. Pros: no filesystem touch, no cleanup needed, fastest. Cons:
each `connect()` returns an isolated DB unless backed by a single
shared connection — for tests that simulate restart-across-tick, this
is wrong.

`TempDir` (already used by `libsql_isolation.rs:29`) gives per-test
on-disk file isolation with the same shape as production. Use
`TempDir` for any test that exercises libSQL.

### 5.3 Integration-test gating

`.claude/rules/testing.md` § "Integration vs unit gating" says:

> "Real infrastructure" means anything outside the in-process pure-Rust
> fixture envelope:
> - Real filesystem I/O — opening real `redb` / `libSQL` / `sqlite`
>   files, writing to `tempfile::TempDir`, mmap'ing on-disk artifacts.

Verbatim hit on libSQL. **Tests that open a real libSQL file on disk
must be gated behind `--features integration-tests`.** The existing
`libsql_isolation.rs` is already at
`crates/overdrive-control-plane/tests/integration/libsql_isolation.rs` and gated through `tests/integration.rs` (which carries
`#![cfg(feature = "integration-tests")]` at top per the same rules
file).

Pure-logic tests (e.g. `JobLifecycleView` value semantics, `cache_key`
shape, anything testable against a `MemoryDb` / `:memory:` URL with
zero filesystem touch) MAY remain in the unit lane.

### 5.4 The test-shape question for the runtime tick loop

`tests/acceptance/runtime_convergence_loop.rs` is in the **acceptance
lane** (default unit lane, not integration). Today it exercises
`run_convergence_tick` with `LibsqlHandle::default_phase1()` (the no-op
stub). Once `run_convergence_tick` opens a real libSQL file, those
tests fail the unit-lane budget unless a `tempfile::TempDir` is
acceptable in the unit lane.

Per `testing.md`:

> Tests that touch real infrastructure MUST be gated behind an
> `integration-tests` feature on their owning crate. "Real
> infrastructure" means … real `libSQL` … files, writing to
> `tempfile::TempDir` …

So `runtime_convergence_loop.rs` tests almost certainly need to move
to the integration lane (`tests/integration/runtime_convergence_loop.rs`)
once libSQL writes are real, OR the runtime needs to support a path
that doesn't touch disk in the unit lane. The latter contradicts the
"real on-disk file == integration test" rule. Simpler: move the tests
that exercise the live runtime tick loop to the integration lane.

---

## 6. streaming.rs / handlers.rs read pattern

### 6.1 Today's view-cache reads

The 11 streaming.rs sites and the one handlers.rs site all want the
same thing: the latest persisted `JobLifecycleView` for a given job, so
they can decide whether `restart_counts.values().max() >= RESTART_BACKOFF_CEILING`.

Read sites and what they need:

1. `streaming.rs:168` — `state.view_cache.clone()` cloned into the
   stream closure for later reads inside the `async_stream::stream!`
   block. Needs a Cloneable handle to "the source of truth for the
   view."

2. `streaming.rs:210` — pre-subscribe-window bridge:
   `lagged_recover(&*obs, &job_id, &view_cache)`. Reads view to gate
   `BackoffExhausted` on the subscribe-race recovery path.

3. `streaming.rs:255-259, 282` — terminal-detection on each broadcast
   event:

   ```rust
   if let Some(terminal) = check_terminal(
       &*obs,
       &job_id,
       &view_cache,
       &event,
   ).await { ... }
   ```

4. `streaming.rs:346-350` — `read_view` helper, called from
   `check_terminal` (line 418) and `lagged_recover` (line 470):

   ```rust
   fn read_view(view_cache: ..., job_id: &JobId) -> JobLifecycleView {
       let key = ("job-lifecycle".to_owned(), format!("job/{job_id}"));
       let cache = match view_cache.lock() { ... };
       match cache.get(&key) {
           Some(CachedView::JobLifecycle(v)) => v.clone(),
           _ => JobLifecycleView::default(),
       }
   }
   ```

5. `streaming.rs:374, 377, 418` (`check_terminal`) — same. Reads
   `restart_counts` to compute `used` and gate `BackoffExhausted`:

   ```rust
   let view = read_view(view_cache, job_id);
   let used = view.restart_counts.values().copied().max().unwrap_or(0);
   if used >= RESTART_BACKOFF_CEILING { ... }
   ```

6. `streaming.rs:444-446, 469-470` (`lagged_recover`) — same.

7. `handlers.rs:617` (`read_job_lifecycle_view`) — the
   `alloc_status` handler reads the view to populate `RestartBudget`
   on the wire response. Single sync read at HTTP-handler dispatch
   time.

### 6.2 What "read view from libSQL" needs to look like

The view source moves from `Arc<Mutex<BTreeMap<...>>>` to a
`libsql::Connection` opened against the `job-lifecycle` reconciler's
DB file. Reading is async (libsql 0.5 is async-only), so any read site
must either:

- (a) **be async** — `streaming.rs` already runs inside
  `async_stream::stream!` and hits `.await` freely (e.g. line 386
  `obs.alloc_status_rows().await`); `handlers.rs::alloc_status` is also
  `async fn`. Both can `.await` on a libSQL read directly.

- (b) **be passed a pre-read view** — the issue body recommends this
  shape:

  > Recommendation: keep view reads on the runtime's own async path and
  > pass results down rather than scatter `LibsqlHandle::open` across
  > the crate.

### 6.3 Plumbing options for the recommended approach

The runtime is the only entity that opens the libSQL handle. To get
the view to streaming.rs / handlers.rs without scattering opens:

**Option A — view rides on `LifecycleEvent`.**
`crate::action_shim::LifecycleEvent` is broadcast from the action
shim after every `obs.write()` (per `lib.rs:111-115`). The shim sees
each emitted action but does NOT see the reconciler view. The view
is only known at the call site of `reconcile` in
`run_convergence_tick`. So adding the view to `LifecycleEvent` requires
plumbing it from `reconciler_runtime.rs:270` into
`action_shim::dispatch` (which currently takes `actions, driver, obs,
lifecycle_events, tick`). Concrete shape: extend the shim's
`LifecycleEvent` to carry the view (or a relevant projection like
`restart_count_max: u32`), add the value at the broadcast site.

**Option B — `AppState` carries a "latest view per job" snapshot
the runtime updates after each tick.**
Reintroduces shared mutable state (the very thing #139 deletes), so
this is just renaming `view_cache`. Reject.

**Option C — every reader opens its own libSQL connection.**
Contradicts the issue's recommendation; scatters `LibsqlHandle::open`
across the crate; pulls libsql into `streaming.rs` and `handlers.rs`
direct dependencies. Each handler pays an open cost per request.
Functionally correct but discouraged.

**Option D — a single shared `Arc<libsql::Database>` per reconciler
held by the runtime; readers ask the runtime for it.**
The runtime already owns one connection per reconciler. Expose it
read-only via `ReconcilerRuntime::libsql_handle(&ReconcilerName) ->
Option<&LibsqlHandle>` (or `Arc<...>`). Readers do their own
`SELECT restart_counts ...` against that handle. Connections in libsql
0.5 are independent given the same `Database`, so multiple async
readers don't contend. Schema knowledge (`SELECT alloc_id, count FROM
restart_counts`) lives in `streaming.rs` / `handlers.rs` though —
violates the ADR-0013 §6 "schema is the author's responsibility,
inside hydrate" clause if the schema knowledge is duplicated.

**Option E — readers call the reconciler's `hydrate` method directly.**
`hydrate(&target, &handle).await` already returns the view as a
typed `Self::View`. Schema lives only in the reconciler. The reader
asks the registry for the named reconciler, builds a `TargetResource`,
calls hydrate. Cost: one hydrate per read (potentially per broadcast
event in streaming.rs). For Phase 1 single-node walking-skeleton
loads (1 replica, low event rate), this is acceptable.

Each option has its own tradeoffs; the implementer (with architect if
needed) should pick. The issue body leans toward Option A.

---

## 7. Diff-and-persist semantics

### 7.1 Phase 1 convention from ADR-0013

ADR-0013:204-208:

> Phase 1 convention: `NextView = Self::View` (full replacement). The
> runtime diffs against the prior view and persists the delta.
> Full-View replacement is simplest and imposes no per-author
> diff-protocol. A typed-diff shape (`NextView = ViewDiff<View>`) is
> an additive future extension when View size makes re-serialisation
> costly; deferred until a real reconciler drives the need.

Concretely:

- Type-level: `Reconciler::reconcile` returns `(Vec<Action>, Self::View)` — `NextView` and `View` are the same Rust type (full replacement).
- Persistence-level: the runtime writes the new view to libSQL,
  replacing the prior view. ADR-0013 says "diffs ... and persists the
  delta" — but with `NextView = View`, "diff" and "delta" are
  conceptual: the runtime can either compute row-level diffs and
  emit per-row UPSERT/DELETE, or simply DELETE-then-INSERT-all the
  rows that comprise the view, or use `INSERT ON CONFLICT REPLACE`
  per row.

### 7.2 What "diff" means concretely for `JobLifecycleView`

`JobLifecycleView` (`crates/overdrive-core/src/reconciler.rs:1323-1331`):

```rust
pub struct JobLifecycleView {
    pub restart_counts:   BTreeMap<AllocationId, u32>,
    pub next_attempt_at:  BTreeMap<AllocationId, Instant>,
}
```

Two maps keyed on `AllocationId`. The "diff" decision is open. Three
candidate shapes; ADR-0013 does NOT specify which:

**Shape 1 — full replacement per tick, single transaction.**
DELETE FROM restart_counts; INSERT ... for every key in the new view.
Same for next_attempt_at. Simplest; quadratic in view size which is
fine for Phase 1.

**Shape 2 — per-row UPSERT/DELETE diff.**
Compute set difference between old and new keys; emit one INSERT/
UPDATE/DELETE per changed key. More efficient at scale but requires
reading the old view to diff against.

**Shape 3 — UPSERT-all + tombstone.**
INSERT OR REPLACE for every key in next_view; DELETE rows whose key
is not in next_view (from the previous read). Hybrid.

Shape 1 is the obvious Phase 1 default per ADR-0013's "simplest /
imposes no per-author diff-protocol" clause. The author writes the
schema in `hydrate`; the runtime opens a transaction, runs the schema-aware
write, commits.

### 7.3 Runtime knows about schema or not?

ADR-0013:65-69 says schema is the author's responsibility inside
`hydrate`. Symmetrically, the **persist** step also needs to know the
schema. Options:

- The trait grows a `persist(view: &Self::View, db: &LibsqlHandle)` async method, called by the runtime after `reconcile`. Author owns
  both schema (in `hydrate`) and persist SQL (in `persist`).
- The runtime introspects `Self::View` via something generic
  (rkyv, serde, custom trait). Loses Elm-style typed-Model
  cleanness.
- The runtime holds raw bytes only — author's `Self::View` is
  serialised wholesale into one BLOB row. Pragmatic Phase 1 default;
  matches "Phase 1 convention is full replacement"; no schema design
  needed.

Approach 1 (trait grows a `persist` method paired with `hydrate`) is
the most consistent extension of ADR-0013's "author owns SQL" clause.
ADR-0013 does NOT prescribe this — it's an implementer decision that
should likely surface as an ADR amendment if the third option (BLOB
storage) is rejected.

### 7.4 Reconciler trait shape today (verbatim)

`crates/overdrive-core/src/reconciler.rs:302-372`:

```rust
pub trait Reconciler: Send + Sync {
    type State: Send + Sync;
    type View: Send + Sync;

    fn name(&self) -> &ReconcilerName;

    fn hydrate(
        &self,
        target: &TargetResource,
        db: &LibsqlHandle,
    ) -> impl std::future::Future<Output = Result<Self::View, HydrateError>> + Send;

    fn reconcile(
        &self,
        desired: &Self::State,
        actual: &Self::State,
        view: &Self::View,
        tick: &TickContext,
    ) -> (Vec<Action>, Self::View);
}
```

There is no `persist` method. The signature `(Vec<Action>, Self::View)` means `NextView == View` (full replacement); a future
typed-diff shape would change the second tuple element.

---

## 8. Adjacent issues + recent commit context

### 8.1 Issue #140

Referenced from `streaming.rs:367, 379, 451` and twice elsewhere via
`TODO(#140)`. From `streaming.rs:367`:

> TODO(#140): gate `ConvergedRunning` on `running_count >=
> replicas_desired` once a multi-replica workload lands. Hydrate
> `replicas_desired` once at stream start rather than reading the
> IntentStore per broadcast event.

#140 is the multi-replica gating concern, separate from #139's
view-source migration. They share a file (streaming.rs) but no
overlapping code lines.

### 8.2 Recent shipped commit `21315f6`

From `git log --oneline`:

```
21315f6 Phase 1 first-workload — convergence loop, cgroup isolation,
        exec driver, CLI streaming submit (#135)
```

This is the umbrella commit that landed the Phase 1 first-workload
slice. The deferred items in that commit are exactly:

- `view_cache` was retained as a temporary stand-in for libSQL
  diff-and-persist (per the `TODO(#139)` markers added in this branch's
  comment-only edits)
- Multi-replica gating (`TODO(#140)`)

#139 is the cleanup of the `view_cache` deferral.

### 8.3 Other related artefacts

- `docs/feature/fix-eval-reconciler-discarded/deliver/bugfix-rca.md` — an RCA pinning the dispatch-routing fix that introduced the
  `cached_view_or_default` / `store_cached_view` shape. Its line-171 quote of
  `let db = LibsqlHandle::default_phase1();` is the same call site
  this issue removes.

- `docs/architecture/cli-submit-vs-deploy-and-alloc-status/c4-component.md` and `architecture.md` — diagrams that
  reference `view_cache` as the view source for streaming. These are
  documentation; #139 implementer should update / land an arch follow-up to
  reflect libSQL.

- `docs/product/architecture/adr-0033-alloc-status-snapshot-enrichment.md` — references `view_cache` as the source of
  `restart_counts` for the `RestartBudget` field on the
  `alloc_status` handler. The handler's view source migrates to libSQL
  via the same plumbing #139 introduces.

---

## 9. Risks + open questions for the implementer

### 9.1 Decisions ADR-0013 does NOT settle

| Question | ADR position | Implementer decision |
|---|---|---|
| Is the SQL schema author-defined per reconciler, or runtime-managed? | "Schema evolution is the author's responsibility, at the View-struct level" (ADR-0013:448) | Author-defined inside `hydrate` |
| Does the trait grow a `persist` method, or does the runtime introspect the View? | Not specified | OPEN — implementer decides |
| If the View serialises wholesale (BLOB), is that compatible with "diff-and-persist"? | "diff against prior view and persist the delta" (ADR-0013:202) | Ambiguous — BLOB IS the delta when full replacement; needs clarification |
| Is the libSQL connection opened eagerly at register-time, or lazily on first hydrate? | "Returns an `Arc<Db>` handle exclusive to that reconciler" (ADR-0013:413) — silent on timing | OPEN — implementer decides; eager is simpler |
| Does the ObservationStore reader-side path open its own libSQL handle, or get one from the runtime? | Issue body recommends "keep view reads on the runtime's own async path and pass results down" | Recommended Option A or E from §6.3 |

### 9.2 Failure modes

- **First-tick: libSQL file does not exist yet.** `open_db` materialises
  the parent dir but returns a handle to an empty DB. The
  reconciler's `hydrate` runs `CREATE TABLE IF NOT EXISTS` first, then
  `SELECT` — empty result set returns the default View. Verified by
  `libsql_isolation.rs:106-128`. Implementer must ensure the schema
  CREATE is idempotent and runs on every hydrate (or on the first one,
  via a `OnceCell`).

- **libSQL open fails (permission, disk full, FS readonly).** Today
  surfaces as `ControlPlaneError::Internal` from
  `libsql_provisioner::open_db` (line 102-107). The runtime's tick
  loop already handles `ConvergenceError::Hydrate` (which would now
  carry the `libsql::Error` via `HydrateError::Libsql(#[from])`).
  Logged warning + return, per `lib.rs:670-678`. Server stays up; eval
  is dropped. Self-re-enqueue does NOT fire (because the `?` short-circuits
  before `has_work` is computed). Operator visibility: log only. May
  want a fail-fast at `runtime.register()` time so the boot path
  surfaces a libSQL open failure synchronously rather than first
  appearing under load.

- **WAL / fsync mode tuning.** libsql 0.5 default journal mode is WAL
  (Builder docs). Phase 1 single-node, single-writer per file, no
  cross-process coordination. Default is fine. Performance tuning
  (`PRAGMA synchronous=NORMAL`) is a Phase 2+ concern.

- **Corruption of a per-reconciler libSQL file.** Single-node Phase 1
  has no replication. A corrupted file is a hard failure for that
  reconciler. ADR-0013 silent. Defer to Phase 2+ snapshot/recovery
  story; reasonable Phase 1 fallback is "delete the file, restart
  reconciler with empty View" — but doing this automatically silently
  drops state. Recommend logging + manual operator action for Phase 1.

- **Test-lane mismatch.** `runtime_convergence_loop.rs` is currently in
  the acceptance lane (default unit). Touching real libSQL bumps it to
  integration lane (per testing.md). Acceptance tests under
  `tests/acceptance/` that need the libSQL must either move to
  `tests/integration/` or be split into pure-logic + real-FS halves.

### 9.3 Constraints from rules files

- **Lima/macOS gating** — every `cargo nextest run --features integration-tests` on macOS goes through `cargo xtask lima run --`
  per `testing.md`. New integration tests inherit this.

- **Mutation-testing scope** — per `testing.md` § "Mutation testing",
  reconciler logic is in mutation-testing scope (kill rate ≥ 80%
  required). The diff-and-persist write path is reconciler-runtime
  logic; CI's `cargo xtask mutants --diff origin/main --features integration-tests` will surface kill-rate gaps if the persist code
  has weak assertions.

- **Async fs ban in adapter-host async fns** — `.claude/rules/development.md` § "Concurrency & async" bans
  `std::fs::*` inside `async fn` bodies in adapter-host crates. The
  runtime's hydrate path already uses `tokio::fs::create_dir_all` via
  `open_db` (`libsql_provisioner.rs:94`). Stay on this side.

- **No locks across `.await`** — `.claude/rules/development.md`
  § Concurrency & async. The current `parking_lot::Mutex` around
  `view_cache` is used only sync-and-released-before-await. Replacement
  must preserve this; do NOT hold the libSQL `Connection` across
  unrelated `.await` points (libsql 0.5 connections are not
  `Send`-across-await-friendly anyway in some patterns; verify when
  wiring).

- **Workspace ordered-collection rule** — `.claude/rules/development.md`
  § "Ordered-collection choice". Any new `HashMap` keyed on
  `ReconcilerName` (e.g. for cached `LibsqlHandle`s) needs a
  `// dst-lint: hashmap-ok <reason>` marker or a `BTreeMap`. The
  registry already uses `BTreeMap` (`reconciler_runtime.rs:55`); the
  same applies to any new map.

---

## 10. Recommended implementation outline (RED-GREEN ordered)

This section is structural guidance, not architecture. Order respects
outside-in TDD: drive each green step with a previously-RED test.

### 10.1 Plumb a real `LibsqlHandle`

1. **Update `LibsqlHandle` to carry a real connection.**
   `crates/overdrive-core/src/reconciler.rs:204-217`. Replace
   `_handle: Option<Arc<()>>` with `Arc<libsql::Connection>` (or
   `Arc<libsql::Database>` plus a `connect()` accessor, depending on
   whether the runtime hands out one connection or many). Add public
   query/exec methods or expose the inner connection.

2. **Delete `LibsqlHandle::default_phase1` and `empty`.** Find the
   single production caller at `reconciler_runtime.rs:265` and
   replace with a real handle (step 3).

3. **Open libSQL connections at register time in
   `ReconcilerRuntime::register`.** `reconciler_runtime.rs:115-127`
   already calls `provision_db_path`. Extend to also call `open_db`
   and stash the resulting `Database` (or a new `LibsqlHandle`)
   alongside the reconciler in the registry. Failure to open
   surfaces as `ControlPlaneError::Internal` at boot.

4. **Add a registry accessor returning the opened handle.** Shape
   like `pub fn libsql_handle(&self, name: &ReconcilerName) -> Option<&LibsqlHandle>` or `Arc<LibsqlHandle>`.

### 10.2 Real hydrate body for `JobLifecycle`

5. **Implement `JobLifecycle::hydrate` against real libSQL.**
   `crates/overdrive-core/src/reconciler.rs:991-1002`. CREATE TABLE
   IF NOT EXISTS for the two maps (one combined or two separate
   tables). SELECT into `BTreeMap<AllocationId, u32>` and `BTreeMap<AllocationId, Instant>`. Test against a
   `tempfile::TempDir`-backed handle (integration lane).

   `Instant` is not directly serialisable across process restart —
   it's a monotonic clock reading. Phase 1 single-process never
   restarts mid-test, so storing as `i64` nanoseconds-since-some-epoch
   is acceptable in-process. Cross-process semantics (a real
   restart of the control plane reading back the deadline) is a
   distinct decision and may require migrating `next_attempt_at` to
   `SystemTime` or a logical-time shape — flag this for the architect.

### 10.3 Wire runtime tick loop

6. **Replace the `default_phase1()` + discard-and-cache shape in
   `run_convergence_tick`.** `reconciler_runtime.rs:254-267`. Get the
   handle from the runtime (step 4), call `hydrate(target, handle).await`, use the returned view directly as `view`.
   Delete `cached_view_or_default`.

7. **Implement diff-and-persist after `reconcile`.**
   `reconciler_runtime.rs:299-304`. Replace `store_cached_view` with
   the chosen persist path (§7.3). If using the trait-`persist`-method
   shape, add `Reconciler::persist(view, db) -> Future<Result>` and
   implement for `JobLifecycle` (full-replacement INSERT OR REPLACE
   for both maps in a transaction) and `NoopHeartbeat` (no-op).
   Delete `store_cached_view` once unused.

### 10.4 Migrate streaming.rs / handlers.rs view reads

8. **Pick a view-read path** (§6.3). The implementer should agree
   with the issue's recommendation — keep reads on the runtime's
   async path. Concretely: extend `LifecycleEvent` to carry the
   `restart_count_max: u32` (or a similarly small projection) so
   `streaming.rs::check_terminal` and `lagged_recover` no longer
   need a libSQL read. For `handlers.rs::alloc_status` (one read per
   HTTP request), call `runtime.libsql_handle(...).await?` ↪
   `JobLifecycle::hydrate(target, handle).await` and project to
   `RestartBudget`.

9. **Delete `read_view` (streaming.rs:345), `read_job_lifecycle_view`
   (handlers.rs:614), and the `restart_budget_from_view`
   helper if it becomes redundant.**

### 10.5 Drop the cache

10. **Delete `AppState::view_cache`** (`lib.rs:109`),
    **`CachedView` enum** (`lib.rs:142-148`), the field init in
    `AppState::new` (`lib.rs:183`), and the `use crate::CachedView;`
    imports in `streaming.rs:108` + `handlers.rs:30`.

### 10.6 Update tests

11. **Migrate `runtime_convergence_loop.rs`** —
    `eval_dispatch_runs_only_the_named_reconciler` and
    `stop_after_failed_alloc_drains_broker` need a new counting
    strategy (e.g. broker counters, lifecycle-event count, or a
    SELECT against the test's libSQL file). The test is in the
    acceptance lane today; bump to `tests/integration/` if it needs
    real libSQL files.

12. **Migrate `streaming.rs` inline test** —
    `check_terminal_returns_converged_stopped_on_terminated_event`
    constructs `view_cache` directly. Reshape per step 8's view-read
    decision.

13. **Update other tests touching `JobLifecycleView` value type** —
    `tests/acceptance/alloc_status_snapshot.rs` and any test
    constructing a `JobLifecycleView` literal. The View type itself
    stays; only the source changes.

14. **Add tests for libSQL-backed hydrate / persist** in the
    integration lane:
    - `JobLifecycle::hydrate` returns the persisted view after a
      restart-equivalent (new `LibsqlHandle` over the same path).
    - Diff-and-persist over many ticks does not unbounded-grow the
      DB.
    - Two reconcilers' libSQL files are isolated (already covered
      by `libsql_isolation.rs:166-202`).

### 10.7 Integration-test gating

Per `testing.md` § "Integration vs unit gating":

- New tests that touch a real libSQL file go in
  `crates/overdrive-control-plane/tests/integration/<scenario>.rs` and
  inherit the feature gate from `tests/integration.rs`.
- `runtime_convergence_loop.rs` likely moves to
  `tests/integration/runtime_convergence_loop.rs` once the runtime
  tick path opens a real libSQL file.
- macOS contributors run via `cargo xtask lima run -- cargo nextest run --workspace --features integration-tests`.

### 10.8 Loose ends

- Stale comment at `crates/overdrive-core/src/reconciler.rs:212-215`
  (claims libsql is not on core's compile path — already false). Fix
  while in the area.

- The `JobLifecycleView` doc at `reconciler.rs:1320-1322` says:

  > Phase 1 hydrates this from the runtime's view cache (`AppState::view_cache`); Phase 2+ migrates the cache to per-primitive
  > libSQL via `CREATE TABLE IF NOT EXISTS` inside `hydrate` per
  > ADR-0013 §2b.

  Update to reflect the post-#139 reality.

- Documentation under `docs/architecture/cli-submit-vs-deploy-and-alloc-status/` references `view_cache`. Architect should update those
  diagrams; flag in the implementer's PR description.
