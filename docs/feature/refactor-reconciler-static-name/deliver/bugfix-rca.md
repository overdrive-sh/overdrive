# Refactor: `Reconciler::NAME: &'static str`, end-to-end static lifetime

**Origin**: Code-review comment on `crates/overdrive-control-plane/src/view_store/redb.rs:100-109`. The `table_def` helper calls `Box::leak` on every invocation, leaking a fresh `String` per call. Call sites — `bulk_load_bytes`, `write_through_bytes`, `delete` — fire on every reconciler tick (`DEFAULT_TICK_CADENCE = 100 ms`) per active target, so the leak is linear in runtime × active-target-count, not constant per-kind as the doc comment claims. ~10 leaks/s/target × ~30 bytes ≈ ~1 MB/hour at idle steady state.

**Root cause**: Mental-model error. `redb::TableDefinition<'static, ...>` requires `&'static str` for the table name. The author chose the simplest path that compiled — `Box::leak` of a per-call `String` — without recognising that `table_def` is called per-tick. The author's doc comment reasoned about *cardinality of distinct names* (bounded — single-digit reconciler kinds) but the leak rate is governed by *number of `Box::leak` calls* (unbounded). The invariant "kinds bounded ⇒ leak bounded" only holds if `table_def` interns; it does not.

**Fix shape — option 3 + TableDefinition registry (chosen)**: Add `const NAME: &'static str` to the `Reconciler` trait. Each `impl Reconciler` declares its kebab-case name as a trait const. Add a new trait method `ViewStore::register_reconciler(&self, name: &'static str) -> Result<(), ViewStoreError>` called by `ReconcilerRuntime::register` once per kind at boot, after `probe()` and before `bulk_load`. `RedbViewStore` holds an internal `parking_lot::RwLock<BTreeMap<&'static str, TableDefinition<'static, &'static str, &'static [u8]>>>`; `register_reconciler` constructs `TableDefinition::new(name)` and inserts. Hot-path methods (`bulk_load_bytes`, `write_through_bytes`, `delete`) take `&'static str`, look up the cached `TableDefinition` under a read lock — no per-call construction, no leak. Unregistered names return a structured error (defensive: surfaces wiring bugs where the runtime forgot to register a reconciler before calling its store). `SimViewStore::register_reconciler` is a no-op (sim doesn't use redb's `TableDefinition` shape). `ViewStore::{bulk_load_bytes, write_through_bytes, delete}` change parameter from `&ReconcilerName` to `&'static str`. `RedbViewStore::table_def` is deleted.

The cached value is `TableDefinition`, not just the static name — the user's literal "build once at startup, store in registry, look up later" semantic. `&'static str` from the trait const is the *key*; the `TableDefinition` is the *cached value*.

**Rejected alternatives**:
- *Per-call interner with `OnceLock<RwLock<HashMap<...>>>`* (reviewer's original suggestion). Correct, but adds global mutable state and a per-call lookup on the hot path. Option 3 is structurally cleaner — the static lifetime is encoded in the type system, not recovered at runtime.
- *Per-store interner on `RedbViewStore`*. Same effective behaviour as the global interner, scoped to one struct. Smaller diff, but still recovers staticness at runtime when the type system already knows it.

**Constraints**:
- `ViewStore` MUST stay dyn-compatible per ADR-0035 §7. The trybuild test `tests/compile_pass/view_store_dyn_compatible.rs` pins this. Changing parameter types from `&ReconcilerName` to `&'static str` keeps dyn-compatibility (no generics introduced, no associated consts on the trait — only on `Reconciler`).
- `Reconciler` trait stays parametrically dyn-compatible. Associated consts on a trait do not break object safety (they are not reachable through the dyn pointer, but the trait remains dyn-compatible). The existing `compile_pass/reconciler_trait_is_dyn_compatible.rs` test must stay green.
- The runtime uses `AnyReconciler` enum-dispatch, not `Box<dyn Reconciler>`, so `R::NAME` is reachable via match on the variant.

**Files in scope** (production):
- `crates/overdrive-core/src/reconciler.rs` — add `const NAME` to trait; declare on `NoopHeartbeat`, `JobLifecycle`; consider if `name(&self) -> &ReconcilerName` stays in the trait surface or becomes derivable from `Self::NAME`.
- `crates/overdrive-control-plane/src/view_store/mod.rs` — change `bulk_load_bytes`, `write_through_bytes`, `delete` to take `&'static str` instead of `&ReconcilerName`; update `ViewStoreExt`.
- `crates/overdrive-control-plane/src/view_store/redb.rs` — `table_def` takes `&'static str`; delete the leak; rewrite the doc comment at lines 100-105.
- `crates/overdrive-sim/src/adapters/view_store.rs` — match new trait sig.
- `crates/overdrive-control-plane/src/reconciler_runtime.rs` — `register` extracts `&'static str` from the `AnyReconciler` variant via match; pass through to `view_store.write_through_bytes` etc.
- `crates/overdrive-control-plane/src/handlers.rs`, `worker/exit_observer.rs` — call sites that build `ReconcilerName::new("job-lifecycle")` should switch to `JobLifecycle::NAME` where they're feeding into ViewStore-adjacent paths; runtime evaluation paths that compare reconciler names by `ReconcilerName` value can keep the existing shape.

**Files in scope** (tests):
- `crates/overdrive-core/tests/acceptance/reconciler_trait_surface.rs` — add an AC for `Reconciler::NAME` (variant declares matching kebab-case).
- `crates/overdrive-core/tests/compile_pass/reconciler_trait_is_dyn_compatible.rs` — verify still passes after const addition.
- `crates/overdrive-control-plane/tests/compile_pass/view_store_dyn_compatible.rs` — verify still passes after sig change.
- Acceptance/integration tests under `crates/overdrive-control-plane/tests/integration/job_lifecycle/*.rs`, `tests/acceptance/runtime_*.rs`, `tests/integration/redb_view_store.rs`, etc. — adjust call sites; many use `ReconcilerName::new("job-lifecycle")` to build evaluations, which is fine — only the ViewStore-bound calls need `&'static str`.
- New regression test in `tests/integration/redb_view_store.rs` (or a dedicated file) — assert that repeated `write_through` for the same reconciler does not allocate (a process-wide allocation counter via `cap` crate would be ideal but heavyweight; a simpler shape: assert `table_def`'s identity by calling it twice and comparing the `&'static str` pointer for equality after the change).

**Risk**: Medium. Wide call-site churn (many tests, many files), but every change is mechanical and the trait surface change is a single name in `core` plus the matching arms in two adapters. No behaviour change at runtime; on-disk byte layout unchanged.
