//! redb-backed implementation of `IntentStore` for the single-node
//! `LocalIntentStore`.
//!
//! # Per-entry `commit_index` storage
//!
//! Each entry's `commit_index` is persisted **inline** with its value in
//! the existing `entries` table. The on-disk byte layout for a stored
//! row is:
//!
//! ```text
//! [u64 little-endian commit_index || raw value bytes]
//! ```
//!
//! `decode_entry` splits the prefix off; `encode_entry` prepends it.
//! Both helpers are private to this module — the trait surface only
//! sees `(Bytes, u64)` tuples.
//!
//! ## Why packed inline rather than a parallel index table
//!
//! Two layouts were considered for `fix-commit-index-per-entry`
//! Step 01-02:
//!
//! 1. **Parallel `entry_index: &[u8] -> u64` redb table** written in
//!    the same write transaction as `entries`.
//! 2. **Packed `[u64-LE-prefix || value]` frame** in the existing
//!    `entries` table.
//!
//! The packed layout was chosen because:
//!
//! * **Atomicity.** A single redb `insert` writes both bytes and
//!   index, eliminating the cross-table consistency window that an
//!   intermediate `RaftStore` adapter (or a partial-commit recovery
//!   path) would otherwise have to reason about. The `txn` path also
//!   stays a single-table loop instead of a two-table dance.
//! * **Read cost.** `get` is a single `table.get()` whose returned
//!   slice is sliced into prefix + value — no second table lookup.
//! * **Snapshot frame v2.** The on-disk row IS the rkyv-archived
//!   value, so the frame v2 payload becomes
//!   `Vec<(Vec<u8>, Vec<u8>, u64)>` — three obvious fields per entry,
//!   no implicit join. Forward compat with v1 just means projecting
//!   the missing index column to `0`.
//! * **Bootstrap atomicity.** `bootstrap_from`'s clear-and-replace is
//!   a single-table operation; a parallel-table layout would need to
//!   clear two tables in lockstep or risk a half-bootstrapped state.
//!
//! The downside of the packed layout — that a future `range scan with
//! commit_index >= N` query has to deserialise every value in range —
//! is not relevant to Phase 1 reconcilers (which read by exact key,
//! not range). When that query shape arrives, a secondary index table
//! is the standard answer; it does not require migrating the primary
//! layout.
//!
//! # Watch: Phase 1 substitute
//!
//! Per the roadmap note for step 03-01, the current `watch` implementation
//! is a `tokio::sync::broadcast` channel behind a `parking_lot::Mutex`.
//! Every `put` and `delete` writes to redb first and, once the redb commit
//! succeeds, emits a `(key, value)` event onto the broadcast channel.
//! Subscribers receive only their matching prefix through a per-stream
//! filter.
//!
//! This is an **in-process** notification surface: it is correct for a
//! single-node `LocalIntentStore` (the `mode = "single"` deployment per the
//! whitepaper §4), where every reader of `IntentStore` lives in the same
//! process as the writer. **Phase 2 replaces this with a Raft-log-driven
//! change notification** once `RaftStore` lands — at that point,
//! subscribers on any node pick up changes through the replicated log
//! rather than through an in-process channel.
//!
//! Trade-offs of the Phase 1 substitute:
//!
//! * Subscribers that lag past the broadcast capacity drop events
//!   (`RecvError::Lagged`). The current stream-wrapper layer silently
//!   drops the lagged notification and continues delivering subsequent
//!   events — the stream does not close, so a caller relying on
//!   end-of-stream as a catch-up trigger will miss the lost events. The
//!   Raft-driven replacement will recover via log catch-up.
//! * Events fire only after successful redb commit, so a subscriber
//!   never sees a phantom write that failed to persist.
//! * The table layout is deliberately minimal for Phase 1: a single
//!   `entries: &[u8] -> &[u8]` table holding packed
//!   `[u64-LE-prefix || value]` rows. Secondary indexes land when
//!   reconcilers need them (Phase 2).

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;
use overdrive_core::traits::intent_store::{
    IntentStore, IntentStoreError, PutOutcome, StateSnapshot, TxnOp, TxnOutcome,
};
use redb::{Database, ReadableTable, TableDefinition};
use tokio::sync::broadcast;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;

use crate::snapshot_frame;

/// Single redb table holding every key/value pair written by the store.
/// Secondary indexes are deliberately out of scope for Phase 1 — reconcilers
/// that need them will add them in Phase 2.
const ENTRIES_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("entries");

/// Capacity of the in-process change-notification broadcast channel.
/// Big enough to absorb a short-lived reader stall without dropping
/// events on a single-node workload; small enough that an infinite lag
/// in a subscriber doesn't balloon memory. Subscribers that lag past
/// this silently lose the dropped events and keep receiving subsequent
/// ones — the stream does not close on lag (see module docs).
const WATCH_CHANNEL_CAPACITY: usize = 1024;

#[derive(Debug, Clone)]
struct WatchEvent {
    key: Bytes,
    /// Empty for deletes, non-empty for puts — matching the
    /// `IntentStore::watch` trait docstring.
    value: Bytes,
}

/// Redb-backed `IntentStore`. Cheap to clone via `Arc`; safe to share
/// across tasks and threads.
pub struct LocalIntentStore {
    inner: Arc<Inner>,
}

struct Inner {
    /// `redb::Database` handles its own internal locking — `begin_read`
    /// and `begin_write` both take `&self`, and the crate is documented
    /// as safe to share across threads. No external mutex is required.
    db: Database,
    watch_tx: broadcast::Sender<WatchEvent>,
    /// Monotonically-increasing commit counter. Phase 1 handlers surface
    /// this as the `commit_index` field on `POST /v1/jobs` responses per
    /// US-03 AC — it is NOT a durable Raft log index (there is no Raft
    /// in single mode) and it resets to zero when the process restarts.
    /// Phase 2's `RaftStore` replaces this with the actual log index
    /// while keeping the accessor signature (`-> u64`) unchanged so the
    /// handler layer is mode-agnostic.
    commit_counter: AtomicU64,
}

impl LocalIntentStore {
    /// Open (or create) a redb-backed `LocalIntentStore` at `path`.
    ///
    /// The parent directory is created if missing — mirrors
    /// `LocalObservationStore::open` so the boot path does not depend
    /// on caller ordering or sibling-store side effects to satisfy a
    /// "parent must exist" precondition. Initializes the single
    /// `entries` table so that the first read doesn't need to take a
    /// write transaction.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, IntentStoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(IntentStoreError::Io)?;
        }
        let db = Database::create(path).map_err(map_database_error)?;

        // Materialize the table on open so the first read doesn't have
        // to open a write transaction to create it.
        {
            let write = db.begin_write().map_err(map_transaction_error)?;
            {
                let _ = write.open_table(ENTRIES_TABLE).map_err(map_table_error)?;
            }
            write.commit().map_err(map_commit_error)?;
        }

        let (watch_tx, _) = broadcast::channel(WATCH_CHANNEL_CAPACITY);

        Ok(Self { inner: Arc::new(Inner { db, watch_tx, commit_counter: AtomicU64::new(0) }) })
    }

    fn emit(&self, key: Bytes, value: Bytes) {
        // `send` returns `Err` only when there are no active
        // subscribers — that's not a failure for us.
        let _ = self.inner.watch_tx.send(WatchEvent { key, value });
    }

    /// Monotonically-increasing commit counter — advances on every
    /// successful `put`, `delete`, or `txn` commit. Handlers surface
    /// this value as the `commit_index` field on write responses per
    /// US-03 AC.
    ///
    /// Phase 1 semantics: in-memory process-local counter, resets to
    /// zero on restart. Phase 2's `RaftStore` replaces the
    /// implementation with the actual Raft log index while keeping this
    /// signature stable, so handler callers are mode-agnostic.
    ///
    /// Deliberately returns `u64` — not a `redb::WriteTransaction` or
    /// `redb::Savepoint` — per ADR-0015 / US-03 AC "no leakage of redb
    /// types through the accessor signature."
    #[must_use]
    pub fn commit_index(&self) -> u64 {
        self.inner.commit_counter.load(Ordering::Acquire)
    }

    /// Bump the commit counter and return the new value. Uses
    /// `AcqRel` ordering so any observation of the new counter
    /// happens-after the call site (which itself is inside a redb
    /// write transaction whose `commit()` provides the durability
    /// fence). The returned value IS the index assigned to the write
    /// transaction inside which `bump_commit_inside` is called — every
    /// caller of this helper does so **inside** the
    /// `tokio::task::spawn_blocking` body that owns the redb write
    /// transaction.
    fn bump_commit_inside(inner: &Inner) -> u64 {
        // `fetch_add` returns the prior value; the assigned index is
        // therefore prior + 1. Using AcqRel here pairs with the load
        // ordering on the inherent `commit_index()` accessor below.
        inner.commit_counter.fetch_add(1, Ordering::AcqRel) + 1
    }
}

/// Encode a value + per-entry `commit_index` into the on-disk row form.
///
/// The frame is `[u64 little-endian commit_index || value bytes]` —
/// the `commit_index` is a fixed 8-byte prefix; the remainder is the
/// caller's value verbatim. Two callers writing the same logical
/// `(value, idx)` produce byte-identical frames.
fn encode_entry(commit_index: u64, value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + value.len());
    out.extend_from_slice(&commit_index.to_le_bytes());
    out.extend_from_slice(value);
    out
}

/// Decode an on-disk row into `(value_bytes, commit_index)`. Returns
/// an `IntentStoreError::Io` (mapped through a synthetic `io::Error`)
/// if the slice is shorter than the 8-byte prefix — that shape can
/// only arise from a backing-file corruption since the encoder always
/// writes the prefix.
fn decode_entry(raw: &[u8]) -> Result<(Bytes, u64), IntentStoreError> {
    if raw.len() < 8 {
        return Err(IntentStoreError::Io(std::io::Error::other(
            "entry row corrupted: shorter than 8-byte commit_index prefix",
        )));
    }
    let mut idx_bytes = [0u8; 8];
    idx_bytes.copy_from_slice(&raw[..8]);
    let commit_index = u64::from_le_bytes(idx_bytes);
    let value = Bytes::copy_from_slice(&raw[8..]);
    Ok((value, commit_index))
}

#[async_trait]
impl IntentStore for LocalIntentStore {
    async fn get(&self, key: &[u8]) -> Result<Option<(Bytes, u64)>, IntentStoreError> {
        let inner = Arc::clone(&self.inner);
        let key = key.to_vec();

        tokio::task::spawn_blocking(move || {
            let read = inner.db.begin_read().map_err(map_transaction_error)?;
            let table = read.open_table(ENTRIES_TABLE).map_err(map_table_error)?;
            let got = table.get(key.as_slice()).map_err(map_storage_error)?;
            got.map_or(Ok(None), |v| decode_entry(v.value()).map(Some))
        })
        .await
        .map_err(map_join_error)?
    }

    async fn put(&self, key: &[u8], value: &[u8]) -> Result<(), IntentStoreError> {
        let key_vec = key.to_vec();
        let value_vec = value.to_vec();
        let inner = Arc::clone(&self.inner);

        let (emit_key, emit_value) = tokio::task::spawn_blocking(move || {
            let write = inner.db.begin_write().map_err(map_transaction_error)?;
            // Bump-and-capture INSIDE the write transaction. The
            // assigned commit_index lands on disk together with the
            // value — see the `commit_index` storage docstring at the
            // top of this module.
            let new_idx = Self::bump_commit_inside(&inner);
            let row = encode_entry(new_idx, value_vec.as_slice());
            {
                let mut table = write.open_table(ENTRIES_TABLE).map_err(map_table_error)?;
                table.insert(key_vec.as_slice(), row.as_slice()).map_err(map_storage_error)?;
            }
            write.commit().map_err(map_commit_error)?;
            Ok::<_, IntentStoreError>((key_vec, value_vec))
        })
        .await
        .map_err(map_join_error)??;

        self.emit(Bytes::from(emit_key), Bytes::from(emit_value));
        Ok(())
    }

    /// Atomic compare-and-set backed by a single redb write
    /// transaction. The `get` + `insert` pair executes inside one
    /// `begin_write` / `commit` cycle; redb serialises write
    /// transactions, so two concurrent `put_if_absent` calls for the
    /// same key cannot both observe the key as absent.
    ///
    /// This closes the TOCTOU window that opens when a caller does a
    /// separate `get` (read txn) followed by a `put` (write txn):
    /// another writer can interleave between the two and silently
    /// overwrite the first write.
    ///
    /// On the `Inserted` branch, the per-entry `commit_index` returned
    /// inside [`PutOutcome::Inserted`] is the index assigned by
    /// `bump_commit_inside` **inside** the write transaction — no
    /// race window with concurrent committers for *different* keys.
    /// On the `KeyExists` branch, `commit_index` carries the prior
    /// write's index, decoded from the stored row.
    async fn put_if_absent(
        &self,
        key: &[u8],
        value: &[u8],
    ) -> Result<PutOutcome, IntentStoreError> {
        let key_vec = key.to_vec();
        let value_vec = value.to_vec();
        let inner = Arc::clone(&self.inner);

        let (outcome, emit) = tokio::task::spawn_blocking(move || {
            let write = inner.db.begin_write().map_err(map_transaction_error)?;
            let (outcome, emit) = {
                let mut table = write.open_table(ENTRIES_TABLE).map_err(map_table_error)?;
                // `get` inside the write transaction sees a consistent
                // view with the subsequent `insert` — this is what makes
                // the check-and-set atomic.
                if let Some(existing) = table.get(key_vec.as_slice()).map_err(map_storage_error)? {
                    let (existing_bytes, existing_idx) = decode_entry(existing.value())?;
                    (
                        PutOutcome::KeyExists {
                            existing: existing_bytes,
                            commit_index: existing_idx,
                        },
                        None,
                    )
                } else {
                    // Bump-and-capture INSIDE the same write txn that
                    // commits the row. The returned `commit_index` is
                    // exactly the one stored on disk — no separate
                    // post-await read of the global counter.
                    let new_idx = Self::bump_commit_inside(&inner);
                    let row = encode_entry(new_idx, value_vec.as_slice());
                    table.insert(key_vec.as_slice(), row.as_slice()).map_err(map_storage_error)?;
                    (PutOutcome::Inserted { commit_index: new_idx }, Some((key_vec, value_vec)))
                }
            };
            write.commit().map_err(map_commit_error)?;
            Ok::<_, IntentStoreError>((outcome, emit))
        })
        .await
        .map_err(map_join_error)??;

        // Watch events only fire on the insert branch — a `KeyExists`
        // return is a no-op commit, semantically. The counter has
        // already been bumped inside the spawn_blocking body.
        if let Some((emit_key, emit_value)) = emit {
            self.emit(Bytes::from(emit_key), Bytes::from(emit_value));
        }
        Ok(outcome)
    }

    async fn delete(&self, key: &[u8]) -> Result<(), IntentStoreError> {
        let key_vec = key.to_vec();
        let inner = Arc::clone(&self.inner);

        // `delete` is idempotent for absent keys: `redb::Table::remove`
        // returns `Ok(None)` when the key is not present. Without
        // gating on that return, the counter bump and watch emit fire
        // for a row that never existed — `commit_index()` observers see
        // false monotone advancement, and `watch(prefix)` subscribers
        // see a phantom `(key, empty)` event. Both bump and emit are
        // therefore conditional on the remove having actually removed.
        let emit_key = tokio::task::spawn_blocking(move || {
            let write = inner.db.begin_write().map_err(map_transaction_error)?;
            let removed = {
                let mut table = write.open_table(ENTRIES_TABLE).map_err(map_table_error)?;
                table.remove(key_vec.as_slice()).map_err(map_storage_error)?.is_some()
            };
            if removed {
                // Bump inside the write transaction so the global
                // counter tracks committed deletes consistently with
                // puts. Delete does not store a per-entry index (the
                // row is gone), so we discard the returned index —
                // only the global cursor observers (e.g.
                // `cluster_status`) read this advance.
                let _new_idx = Self::bump_commit_inside(&inner);
            }
            write.commit().map_err(map_commit_error)?;
            Ok::<_, IntentStoreError>(removed.then_some(key_vec))
        })
        .await
        .map_err(map_join_error)??;

        // Delete events carry an empty value per the trait docstring.
        // Emit only when the row actually existed pre-delete — a
        // phantom event for an absent key is the bug this gate closes.
        if let Some(key) = emit_key {
            self.emit(Bytes::from(key), Bytes::new());
        }
        Ok(())
    }

    async fn txn(&self, ops: Vec<TxnOp>) -> Result<TxnOutcome, IntentStoreError> {
        // An empty `txn` has nothing to commit. Opening a write
        // transaction and bumping the counter for it would leak false
        // monotone advancement to `commit_index()` observers and waste
        // a redb commit on a no-op. Short-circuit before any I/O.
        if ops.is_empty() {
            return Ok(TxnOutcome::Committed);
        }

        let inner = Arc::clone(&self.inner);
        let ops_for_commit = ops.clone();

        // For each op, record whether it had observable effect: every
        // `Put` always does; a `Delete` only when `redb::Table::remove`
        // returned `Some(_)`. The post-commit emit loop reads this
        // mask so phantom delete events for absent keys never reach
        // `watch(prefix)` subscribers — the same gate as the standalone
        // `delete` path, applied per-op inside the transaction.
        let effective = tokio::task::spawn_blocking(move || {
            let write = inner.db.begin_write().map_err(map_transaction_error)?;
            // Bump-and-capture once for the whole transaction — every
            // put inside this txn shares the same per-entry index.
            // This matches the trait contract that `txn` advances the
            // commit cursor by exactly 1 regardless of op count, with
            // the natural exception of an empty `ops` vector handled
            // above.
            let txn_idx = Self::bump_commit_inside(&inner);
            let mut effective = Vec::with_capacity(ops_for_commit.len());
            {
                let mut table = write.open_table(ENTRIES_TABLE).map_err(map_table_error)?;
                for op in &ops_for_commit {
                    match op {
                        TxnOp::Put { key, value } => {
                            let row = encode_entry(txn_idx, value.as_ref());
                            table
                                .insert(key.as_ref(), row.as_slice())
                                .map_err(map_storage_error)?;
                            effective.push(true);
                        }
                        TxnOp::Delete { key } => {
                            let removed =
                                table.remove(key.as_ref()).map_err(map_storage_error)?.is_some();
                            effective.push(removed);
                        }
                    }
                }
            }
            write.commit().map_err(map_commit_error)?;
            Ok::<_, IntentStoreError>(effective)
        })
        .await
        .map_err(map_join_error)??;

        // Emit per-op events *after* the commit succeeds so subscribers
        // never see a phantom write — and only for ops that actually
        // changed state.
        for (op, effective) in ops.into_iter().zip(effective) {
            if !effective {
                continue;
            }
            match op {
                TxnOp::Put { key, value } => self.emit(key, value),
                TxnOp::Delete { key } => self.emit(key, Bytes::new()),
            }
        }

        Ok(TxnOutcome::Committed)
    }

    async fn watch(
        &self,
        prefix: &[u8],
    ) -> Result<Box<dyn Stream<Item = (Bytes, Bytes)> + Send + Unpin>, IntentStoreError> {
        let prefix = Bytes::copy_from_slice(prefix);
        let rx = self.inner.watch_tx.subscribe();

        // Drop `Lagged` / drain errors silently by filtering them out —
        // returning `None` from `filter_map` skips the lagged notification
        // and keeps the stream alive; subsequent events still arrive.
        // Phase 2 log-driven notification is the recovery path for the
        // lost events.
        let stream = BroadcastStream::new(rx).filter_map(move |evt| match evt {
            Ok(event) => {
                if event.key.starts_with(&prefix) {
                    Some((event.key, event.value))
                } else {
                    None
                }
            }
            Err(_lag) => None,
        });

        Ok(Box::new(Box::pin(PrefixWatchStream { inner: Box::pin(stream) })))
    }

    /// Export a full-state snapshot of this `LocalIntentStore`.
    ///
    /// Reads every `(key, value, commit_index)` triple in a single
    /// redb read transaction, sorts them by key via
    /// [`snapshot_frame::encode`], and returns a [`StateSnapshot`]
    /// whose `bytes()` slice is canonical — two semantically-equal
    /// stores produce byte-identical exports. The frame format is v2
    /// (carries per-entry `commit_index`); see `snapshot_frame.rs`
    /// for the byte layout. The same frame format is consumed by
    /// [`Self::bootstrap_from`] and will be consumed by
    /// `RaftStore::bootstrap_from` in Phase 2.
    async fn export_snapshot(&self) -> Result<StateSnapshot, IntentStoreError> {
        let inner = Arc::clone(&self.inner);

        tokio::task::spawn_blocking(move || {
            let read = inner.db.begin_read().map_err(map_transaction_error)?;
            let table = read.open_table(ENTRIES_TABLE).map_err(map_table_error)?;

            let mut entries: Vec<(Bytes, Bytes, u64)> = Vec::new();
            let iter = table.iter().map_err(map_storage_error)?;
            for item in iter {
                let (k, raw) = item.map_err(map_storage_error)?;
                let (value, idx) = decode_entry(raw.value())?;
                entries.push((Bytes::copy_from_slice(k.value()), value, idx));
            }

            let bytes = snapshot_frame::encode(&entries)
                .map_err(|e| IntentStoreError::SnapshotImport(e.to_string()))?;
            // `encode` sorts internally on an owned copy, so we sort
            // the caller-visible view here as well to keep the two
            // projections consistent.
            entries.sort_by(|a, b| a.0.as_ref().cmp(b.0.as_ref()));

            // The trait-level `StateSnapshot` view exposes
            // `Vec<(Bytes, Bytes)>` — drop the index column for the
            // logical view; the canonical byte slice still carries
            // them losslessly, and `bootstrap_from` reads from
            // `bytes()` not from `entries`.
            let logical: Vec<(Bytes, Bytes)> =
                entries.into_iter().map(|(k, v, _idx)| (k, v)).collect();

            Ok::<_, IntentStoreError>(StateSnapshot::from_parts(
                u32::from(snapshot_frame::VERSION),
                logical,
                bytes,
            ))
        })
        .await
        .map_err(map_join_error)?
    }

    /// Replay a snapshot as the initial state of this `LocalIntentStore`.
    ///
    /// Decodes the framed byte slice via [`snapshot_frame::decode`],
    /// then, inside a single redb write transaction, clears every
    /// pre-existing row before inserting the snapshot entries. Pre-
    /// existing rows do NOT survive — the trait docstring specifies
    /// that this replays a *full-state* snapshot as the initial state,
    /// and preserving leftover keys would silently violate that
    /// contract.
    ///
    /// The clear-then-insert sequence happens inside a single
    /// `begin_write` / `commit` pair so the operation remains atomic:
    /// concurrent readers observe either the pre-bootstrap state or
    /// the fully-replayed state, never an intermediate view. Returns a
    /// typed [`IntentStoreError::SnapshotImport`] on any frame-level
    /// corruption — step 03-03 covers the specific corruption
    /// scenarios.
    async fn bootstrap_from(&self, snapshot: StateSnapshot) -> Result<(), IntentStoreError> {
        let inner = Arc::clone(&self.inner);
        // Clone out of the snapshot so the spawn_blocking closure owns
        // its input — the frame bytes are the authoritative source,
        // not the decoded `entries` view.
        let frame = snapshot.bytes().to_vec();

        tokio::task::spawn_blocking(move || {
            // Decode BEFORE opening the write transaction. This is what
            // makes `bootstrap_from` atomic across corruption: a frame
            // that fails to decode never touches the target store, so
            // the post-failure `export_snapshot` is byte-identical to
            // the export of a fresh never-bootstrapped store.
            //
            // `decode` accepts both v1 frames (zero-padded indices)
            // and v2 frames (per-entry indices); see `snapshot_frame`
            // module docstring for the forward-compat behaviour.
            let entries = snapshot_frame::decode(&frame)
                .map_err(|e| IntentStoreError::SnapshotCorrupt { offset: e.offset() })?;

            // Compute the maximum per-entry commit_index in the
            // incoming snapshot — the global counter must be
            // >= every per-entry index after bootstrap so subsequent
            // writes do not assign a clashing index. Empty snapshots
            // keep the counter at its current value (typically 0 for
            // a freshly-opened target store).
            let max_idx = entries.iter().map(|(_, _, idx)| *idx).max().unwrap_or(0);

            let write = inner.db.begin_write().map_err(map_transaction_error)?;
            {
                let mut table = write.open_table(ENTRIES_TABLE).map_err(map_table_error)?;
                // Drop every pre-existing row so bootstrap replaces
                // state rather than merging into it. `retain` with a
                // `false` predicate is redb's idiomatic full-table
                // clear and keeps the whole operation inside the same
                // write transaction as the subsequent inserts.
                table.retain(|_, _| false).map_err(map_storage_error)?;
                for (k, v, idx) in &entries {
                    let row = encode_entry(*idx, v.as_ref());
                    table.insert(k.as_ref(), row.as_slice()).map_err(map_storage_error)?;
                }
            }
            write.commit().map_err(map_commit_error)?;

            // Advance the global counter to at least max_idx so the
            // next write does not collide with any imported entry's
            // commit_index. `store` with Release ordering is correct
            // here because the redb commit above is the durability
            // fence; this is a "raise to floor" not a bump.
            //
            // Use `fetch_max` to avoid clobbering a higher counter on
            // a future re-import scenario — though in practice the
            // target is freshly-opened with counter == 0.
            inner.commit_counter.fetch_max(max_idx, Ordering::AcqRel);

            Ok::<_, IntentStoreError>(())
        })
        .await
        .map_err(map_join_error)?
    }
}

/// Thin newtype wrapper so that `watch` can return a `Box<dyn Stream +
/// Unpin>` — `futures::stream::FilterMap` isn't `Unpin` on its own
/// because it holds a user-supplied `FnMut`.
struct PrefixWatchStream {
    inner: Pin<Box<dyn Stream<Item = (Bytes, Bytes)> + Send>>,
}

impl Stream for PrefixWatchStream {
    type Item = (Bytes, Bytes);

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

// -----------------------------------------------------------------------------
// Error mapping helpers — map every redb error class onto a single
// `IntentStoreError` variant. redb's error hierarchy is deeper than
// Phase 1 reconcilers need to distinguish; Phase 2 may split these out
// if a reconciler grows a branch on a specific redb error class.
// -----------------------------------------------------------------------------

fn map_database_error(err: redb::DatabaseError) -> IntentStoreError {
    IntentStoreError::Io(std::io::Error::other(err))
}

fn map_transaction_error(err: redb::TransactionError) -> IntentStoreError {
    IntentStoreError::Io(std::io::Error::other(err))
}

fn map_table_error(err: redb::TableError) -> IntentStoreError {
    IntentStoreError::Io(std::io::Error::other(err))
}

fn map_storage_error(err: redb::StorageError) -> IntentStoreError {
    IntentStoreError::Io(std::io::Error::other(err))
}

fn map_commit_error(err: redb::CommitError) -> IntentStoreError {
    IntentStoreError::Io(std::io::Error::other(err))
}

fn map_join_error(err: tokio::task::JoinError) -> IntentStoreError {
    IntentStoreError::Io(std::io::Error::other(err))
}
