//! redb-backed implementation of `IntentStore` for the single-node
//! `LocalIntentStore`.
//!
//! # Storage layout
//!
//! A single `entries: &[u8] -> &[u8]` table holds every key/value pair.
//! The stored row IS the caller-provided value — no inline framing, no
//! prefix, no transformation. `IntentStore::put(k, v)` followed by
//! `IntentStore::get(k)` returns `v` byte-for-byte.
//!
//! Secondary indexes are deliberately out of scope for Phase 1 —
//! reconcilers that need them will add them in Phase 2 alongside
//! `RaftStore`.
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

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
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

        Ok(Self { inner: Arc::new(Inner { db, watch_tx }) })
    }

    fn emit(&self, key: Bytes, value: Bytes) {
        // `send` returns `Err` only when there are no active
        // subscribers — that's not a failure for us.
        let _ = self.inner.watch_tx.send(WatchEvent { key, value });
    }
}

#[async_trait]
impl IntentStore for LocalIntentStore {
    async fn get(&self, key: &[u8]) -> Result<Option<Bytes>, IntentStoreError> {
        let inner = Arc::clone(&self.inner);
        let key = key.to_vec();

        tokio::task::spawn_blocking(move || {
            let read = inner.db.begin_read().map_err(map_transaction_error)?;
            let table = read.open_table(ENTRIES_TABLE).map_err(map_table_error)?;
            let got = table.get(key.as_slice()).map_err(map_storage_error)?;
            // The stored row IS the caller-provided value — no
            // framing, no prefix, no transformation (ADR-0020 §Decision §2).
            Ok(got.map(|v| Bytes::copy_from_slice(v.value())))
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
            {
                let mut table = write.open_table(ENTRIES_TABLE).map_err(map_table_error)?;
                // Store the caller-provided bytes verbatim. ADR-0020
                // §Decision §2: `IntentStore::put(k, v)` followed by
                // `IntentStore::get(k)` returns `v` byte-for-byte.
                table
                    .insert(key_vec.as_slice(), value_vec.as_slice())
                    .map_err(map_storage_error)?;
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
    /// On the `KeyExists` branch, `existing` carries the bytes that
    /// currently occupy the key — used by handlers to distinguish
    /// idempotent re-submission from genuine conflict.
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
                    let existing_bytes = Bytes::copy_from_slice(existing.value());
                    (PutOutcome::KeyExists { existing: existing_bytes }, None)
                } else {
                    table
                        .insert(key_vec.as_slice(), value_vec.as_slice())
                        .map_err(map_storage_error)?;
                    (PutOutcome::Inserted, Some((key_vec, value_vec)))
                }
            };
            write.commit().map_err(map_commit_error)?;
            Ok::<_, IntentStoreError>((outcome, emit))
        })
        .await
        .map_err(map_join_error)??;

        // Watch events only fire on the insert branch — a `KeyExists`
        // return is a no-op commit, semantically.
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
        // gating on that return, the watch emit fires for a row that
        // never existed — `watch(prefix)` subscribers would see a
        // phantom `(key, empty)` event. The emit is therefore
        // conditional on the remove having actually removed.
        let emit_key = tokio::task::spawn_blocking(move || {
            let write = inner.db.begin_write().map_err(map_transaction_error)?;
            let removed = {
                let mut table = write.open_table(ENTRIES_TABLE).map_err(map_table_error)?;
                table.remove(key_vec.as_slice()).map_err(map_storage_error)?.is_some()
            };
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
        // An empty `txn` has nothing to commit. Short-circuit before
        // any I/O — opening a write transaction for it is wasted work.
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
            let mut effective = Vec::with_capacity(ops_for_commit.len());
            {
                let mut table = write.open_table(ENTRIES_TABLE).map_err(map_table_error)?;
                for op in &ops_for_commit {
                    match op {
                        TxnOp::Put { key, value } => {
                            table
                                .insert(key.as_ref(), value.as_ref())
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
    /// Reads every `(key, value)` pair in a single redb read
    /// transaction, sorts them by key via [`snapshot_frame::encode`],
    /// and returns a [`StateSnapshot`] whose `bytes()` slice is
    /// canonical — two semantically-equal stores produce byte-identical
    /// exports. The frame format is v1 (key + value, no per-entry
    /// `commit_index` per ADR-0020 §Decision §3); see `snapshot_frame.rs`
    /// for the byte layout. The same frame format is consumed by
    /// [`Self::bootstrap_from`] and will be consumed by
    /// `RaftStore::bootstrap_from` in Phase 2.
    async fn export_snapshot(&self) -> Result<StateSnapshot, IntentStoreError> {
        let inner = Arc::clone(&self.inner);

        tokio::task::spawn_blocking(move || {
            let read = inner.db.begin_read().map_err(map_transaction_error)?;
            let table = read.open_table(ENTRIES_TABLE).map_err(map_table_error)?;

            let mut entries: Vec<(Bytes, Bytes)> = Vec::new();
            let iter = table.iter().map_err(map_storage_error)?;
            for item in iter {
                let (k, v) = item.map_err(map_storage_error)?;
                entries
                    .push((Bytes::copy_from_slice(k.value()), Bytes::copy_from_slice(v.value())));
            }

            let bytes = snapshot_frame::encode(&entries)
                .map_err(|e| IntentStoreError::SnapshotImport(e.to_string()))?;
            // `encode` sorts internally on an owned copy, so we sort
            // the caller-visible view here as well to keep the two
            // projections consistent.
            entries.sort_by(|a, b| a.0.as_ref().cmp(b.0.as_ref()));

            Ok::<_, IntentStoreError>(StateSnapshot::from_parts(
                u32::from(snapshot_frame::VERSION),
                entries,
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
            let entries = snapshot_frame::decode(&frame)
                .map_err(|e| IntentStoreError::SnapshotCorrupt { offset: e.offset() })?;

            let write = inner.db.begin_write().map_err(map_transaction_error)?;
            {
                let mut table = write.open_table(ENTRIES_TABLE).map_err(map_table_error)?;
                // Drop every pre-existing row so bootstrap replaces
                // state rather than merging into it. `retain` with a
                // `false` predicate is redb's idiomatic full-table
                // clear and keeps the whole operation inside the same
                // write transaction as the subsequent inserts.
                table.retain(|_, _| false).map_err(map_storage_error)?;
                for (k, v) in &entries {
                    table.insert(k.as_ref(), v.as_ref()).map_err(map_storage_error)?;
                }
            }
            write.commit().map_err(map_commit_error)?;

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
