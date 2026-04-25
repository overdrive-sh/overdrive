# RCA Context — fix-commit-counter-and-watch-doc

Source: code-review comments on `crates/overdrive-store-local/src/redb_backend.rs`.

## Bug 1 — Phantom commit_index advancement on failed writes

**Files affected**
- `crates/overdrive-store-local/src/redb_backend.rs` (lines 211-216, 277, 340, 385, 428)

**Root cause**
`bump_commit_inside` calls `fetch_add` *before* `table.insert` / `write.commit()`. If any operation between the bump and a successful commit fails, the counter advances without a corresponding committed row, violating the "commit_index reflects committed writes" invariant documented in the module header.

**Affected paths**
- `put` (line 277) — bump then insert then commit
- `put_if_absent::Inserted` branch (line 340) — bump then insert then commit
- `delete` (line 385) — bump conditional on `removed`, but commit can still fail after
- `txn` (line 428) — bump at start of transaction, all inserts/removes after

**Fix**
Split the helper into a peek + bump pair, both called inside the `spawn_blocking` body holding the redb write lock. redb serialises write transactions, so peek-then-bump is race-free without extra locking.

```rust
fn peek_next_inside(inner: &Inner) -> u64 {
    inner.commit_counter.load(Ordering::Acquire) + 1
}

fn bump_commit_after_commit(inner: &Inner) -> u64 {
    inner.commit_counter.fetch_add(1, Ordering::AcqRel) + 1
}
```

For each write path:
1. `peek_next_inside` BEFORE the commit — use the value as the per-entry `commit_index` in `encode_entry`.
2. `bump_commit_after_commit` AFTER `write.commit()` succeeds (and only if the op had observable effect — `removed` for delete, at-least-one effective op for txn).
3. `debug_assert_eq!` the two — sanity check on serialisation.

**Risk**
Low. The change is local to the redb backend. redb's write serialisation is documented and load-bearing for this fix. `commit_index()` callers (e.g. cluster_status handler) already expect monotone advancement; the change strengthens the invariant they rely on.

## Bug 2 — Subscription lag-drop docstring mismatch

**Files affected**
- `crates/overdrive-store-local/src/observation_backend.rs` (lines 30-35)

**Root cause**
Both `redb_backend.rs` (line 483-491) and `observation_backend.rs` (line 160) use `BroadcastStream::filter_map` to drop `Err(Lagged)` silently and keep the stream alive. The intent backend's module docstring describes this truthfully ("stream does not close"); the observation backend's module docstring claims lag "is signalled end-of-stream," which is a lie about the actual behaviour.

**Fix**
Doc-only change. Align `observation_backend.rs:30-35` with the actual `filter_map(Result::ok)` behaviour and the intent-backend's truthful description.

**Why doc-not-behaviour**
Changing the behaviour to actually close the stream on lag would force every Phase 1 subscriber to grow reconnect logic for a recovery path the platform docs already say belongs to Phase 2 gossip catch-up. Aligning docs to the existing implementation is the right call for Phase 1.

**Risk**
None — comment-only edit.

## Regression test shape

For Bug 1: invariant test asserting `store.commit_index() <= entries_on_disk_count()` after arbitrary sequences of `put` / `delete` / `txn`. If a clean failure-injection path through redb is tractable, prefer that; otherwise the invariant catches phantom advancement under any failure source.

For Bug 2: not test-gated (doc change). Optional: a `cargo test --doc` example demonstrating that `BroadcastStream` with `filter_map(Result::ok)` does not close on `Err(Lagged)`, but this is over-engineering for a comment fix.

## Suggested commit shape

Single commit per bug:
1. `fix(store-local): defer commit_counter bump until after redb commit`
2. `docs(store-local): align observation subscription lag-drop docstring with code`
