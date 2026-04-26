# RCA — `commit_index` is NOT per-entry write index

**Status**: User-approved 2026-04-25 ("sounds good")
**Reporter**: code-review comment on `crates/overdrive-control-plane/src/handlers.rs:176-182`

## Problem

Two endpoints (`describe_job`, `submit_job::Inserted` arm) populate the
`commit_index` field in their JSON response with the store's *current*
counter at response-construction time, contradicting the per-entry contract
documented for `JobDescription` (`api.rs:50-52` rustdoc) and ADR-0015 / US-03 AC.

After jobs B and C are written following job A, a subsequent
`GET /v1/jobs/{a_id}` returns a `commit_index` reflecting B's or C's
write — not A's. Same race exists in `submit_job::Inserted`: the counter
is bumped inside the `spawn_blocking` body but read on the calling task
*after* the await returns, so a concurrent committer for a different key
can interleave between bump and read.

## Causal Chain (5 Whys, multi-causal)

### Branch A — `describe_job` returns wrong index

- **WHY 1A**: Reader returns store-wide current counter, not the index at
  which this entry was committed. *Evidence*: `handlers.rs:180`
  `commit_index: state.store.commit_index()` after `get` returned bytes.
- **WHY 2A**: `IntentStore::get` returns `Option<Bytes>` only — no index.
  *Evidence*: `crates/overdrive-core/src/traits/intent_store.rs:140`.
- **WHY 3A**: No durable per-entry index is stored alongside the value.
  redb table is `entries: &[u8] -> &[u8]`
  (`crates/overdrive-store-local/src/redb_backend.rs:55`); the counter
  is process-local `AtomicU64` (`redb_backend.rs:91`).
- **WHY 4A**: Phase 1 design treats `commit_index` as a "store cursor"
  (a single inherent accessor on `LocalIntentStore`, `redb_backend.rs:139`)
  — there was never a value-attached index in the trait surface; the
  wire field name pretended otherwise.
- **WHY 5A — ROOT CAUSE A**: **Trait-surface gap.** `IntentStore` exposes
  no `(value, index)` read primitive. The trait could not satisfy a
  per-entry contract even if the handler tried.

### Branch B — `submit_job::Inserted` reads outside the txn

- **WHY 1B**: Counter is read on the calling task after `put_if_absent`
  returns. *Evidence*: `handlers.rs:90-94`.
- **WHY 2B**: The `bump_commit` happens INSIDE the `spawn_blocking` body
  (`redb_backend.rs:238`), but `PutOutcome::Inserted` is unit-like
  (`intent_store.rs:75`), carrying no index back to the caller.
- **WHY 3B**: redb serializes write txns, so `put_if_absent` for *the
  same key* is safe; but two writes for *different keys* (A then B)
  interleave freely between commit-N's bump and the handler's read of
  `commit_index()`.
- **WHY 4B**: Atomicity contract was framed around "no double-write at
  the same key" (TOCTOU on the key). It was not framed around "the index
  returned to caller equals the index at which their bytes landed."
- **WHY 5B — ROOT CAUSE B**: **`PutOutcome::Inserted` carries no index.**
  The atomic compare-and-set returns a unit-like marker; the caller has
  no way to obtain the index assigned inside the same transaction.

### Branch C — Doc/code drift not caught by tests

- **WHY 1C**: Existing tests assert global monotonicity
  (`crates/overdrive-store-local/tests/acceptance/commit_index_monotonic.rs`)
  and TOCTOU-single-winner
  (`crates/overdrive-control-plane/tests/integration/concurrent_submit_toctou.rs`);
  none assert `describe(A).commit_index == submit(A).commit_index` after
  intervening writes.
- **WHY 2C**: The user-stories example documenting the exact failure
  shape — "submit A → 17, submit B → 18, describe A → still 17"
  (`docs/feature/phase-1-control-plane-core/discuss/user-stories.md:234`)
  was never lifted into a test fixture.
- **WHY 5C — ROOT CAUSE C**: **Missing regression test for per-entry
  semantics.** The contract is written down in user stories and
  `JobDescription`'s rustdoc (`api.rs:50-52` "the commit index at which
  it was written"); no test mechanically pins it.

## Proposed fix (single-cut, additive — user-approved)

1. **Trait additive change** in
   `crates/overdrive-core/src/traits/intent_store.rs`:
   - `PutOutcome::Inserted { commit_index: u64 }`
   - `PutOutcome::KeyExists { existing: Bytes, commit_index: u64 }`
     — commit_index of the prior write
   - `IntentStore::get -> Result<Option<(Bytes, u64)>, IntentStoreError>`
2. **Store impl** in
   `crates/overdrive-store-local/src/redb_backend.rs`:
   - Persist a per-entry index alongside the value (parallel
     `entry_index` table, OR a packed `(u64 LE prefix, value)` frame
     in the existing entries table — crafter chooses; document choice).
   - Bump-and-capture inside the same `spawn_blocking` / redb write
     txn — the index returned via `PutOutcome` is the one assigned in
     the transaction that wrote the bytes.
3. **Snapshot frame v2**
   (`crates/overdrive-store-local/src/snapshot_frame.rs`):
   - Carry per-entry indices through export/bootstrap so the single → HA
     migration story stays whole.
4. **Handlers** (`crates/overdrive-control-plane/src/handlers.rs`):
   - `submit_job` consumes `commit_index` from `PutOutcome::Inserted`
     and `PutOutcome::KeyExists` directly.
   - `describe_job` consumes `commit_index` from the new `get` tuple.
   - `cluster_status` keeps the global cursor (its semantics are correct
     — it documents the store-wide commit count, not a per-entry index).
5. **Tests**:
   - Lift the user-story scenario verbatim — submit A → idx_a; submit B
     → idx_b > idx_a; `describe(A).commit_index == idx_a`.
   - Tighten `concurrent_submit_toctou.rs:269` to assert the
     byte-identical-resubmit value equals the original insert's index.
   - Add per-entry semantics tests at the store layer.

## Files affected

- `crates/overdrive-core/src/traits/intent_store.rs` — trait surface
- `crates/overdrive-store-local/src/redb_backend.rs` — per-entry storage
- `crates/overdrive-store-local/src/snapshot_frame.rs` — frame v2
- `crates/overdrive-store-local/tests/acceptance/{put_if_absent,commit_index_monotonic}.rs` — pattern-match new variant shapes; assert per-entry
- `crates/overdrive-store-local/tests/acceptance/per_entry_commit_index.rs` (new)
- `crates/overdrive-control-plane/src/handlers.rs` — consume returned indices
- `crates/overdrive-control-plane/tests/integration/{submit_round_trip,describe_round_trip,idempotent_resubmit,concurrent_submit_toctou}.rs` — pin per-entry contract
- `crates/overdrive-control-plane/tests/integration/per_entry_commit_index.rs` (new) — user-story regression test

## Risk

- **OpenAPI**: field name + JSON shape unchanged (`u64`); semantics
  tighten — clients receive a *more stable* value. Wire-compatible.
- **CLI**: `overdrive job submit` / `describe` already render the field;
  printed number changes meaning but the type does not.
- **Single → HA migration**: snapshot frame v2 must carry per-entry
  indices forward. `RaftStore` (Phase 2) wires real Raft log indices
  through the same `PutOutcome` shape.
- **DST**: `LocalStore` is the sim intent store too; existing `commit_index`
  assertions need a quick review (none currently assert per-entry
  semantics, so the update is small).
- **Watch streams**: unaffected (carry `(key, value)`, not indices).

## Out of scope

- Removing `LocalIntentStore::commit_index()` — `cluster_status` is a
  legitimate consumer of the global cursor.
- Threading per-entry indices through watch events — additive change for
  reconciler resume; not blocking for this fix.
