//! redb-backed implementation of [`ObservationStore`] for the single-node
//! `LocalObservationStore`.
//!
//! Per ADR-0012 (revised 2026-04-24), Phase 1 observation state lives in
//! a dedicated redb database co-resident with the intent store. The
//! previous wiring routed production observation through
//! `SimObservationStore` (an in-memory CR-SQLite-shaped fixture) — that
//! reversal traded persistence for purity and was reversed once the
//! objection surfaced: observation rows must survive a server restart,
//! and the "all production impls live under real adapters" rule in
//! ADR-0003 must not be perforated for convenience.
//!
//! # Durability shape
//!
//! Two redb tables:
//!
//! * `alloc_status` — keyed by canonical `AllocationId` bytes, value is
//!   the rkyv-archived `AllocStatusRow`. Overwrite semantics on the key
//!   (second write for same id replaces the first).
//! * `node_health` — keyed by canonical `NodeId` bytes, value is the
//!   rkyv-archived `NodeHealthRow`. Same overwrite semantics.
//!
//! Phase 1 has NO on-disk schema versioning for observation rows — the
//! format is the rkyv layout of the Rust types at build time. A Phase 2
//! migration (new row variants, field additions) ships its own
//! schema-migration reconciler; the Phase 1 file is considered
//! rebuild-on-upgrade until then.
//!
//! # Subscription shape
//!
//! Subscribers receive a `tokio::sync::broadcast` stream of every row
//! written to this peer AFTER the subscription opens — the future-only
//! contract from the `ObservationStore` trait. A subscriber that lags
//! past the broadcast capacity is signalled end-of-stream; Phase 2's
//! Corrosion replacement recovers via CR-SQLite gossip catch-up.

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures::Stream;
use overdrive_core::traits::observation_store::{
    AllocStatusRow, NodeHealthRow, ObservationRow, ObservationStore, ObservationStoreError,
    ObservationSubscription,
};
use redb::{Database, ReadableTable, TableDefinition};
use tokio::sync::broadcast;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;

/// Holds the rkyv-archived bytes of every `AllocStatusRow`, keyed by
/// canonical `AllocationId` bytes.
const ALLOC_STATUS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("observation_alloc_status");

/// Holds the rkyv-archived bytes of every `NodeHealthRow`, keyed by
/// canonical `NodeId` bytes.
const NODE_HEALTH_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("observation_node_health");

/// Capacity of the in-process broadcast channel used for
/// `subscribe_all`. Sized to absorb a short-lived reader stall on a
/// single-node workload without backing memory to the moon. Lag past
/// this is signalled as end-of-stream (see module docs).
const SUBSCRIPTION_CHANNEL_CAPACITY: usize = 1024;

/// Redb-backed `ObservationStore`. Cheap to clone via `Arc`; safe to
/// share across tasks and threads.
pub struct LocalObservationStore {
    inner: Arc<Inner>,
}

struct Inner {
    /// `redb::Database` handles its own internal locking.
    db: Database,
    /// Fan-out channel for `subscribe_all` subscribers. Every
    /// successful `write` emits the row on this channel after the redb
    /// commit succeeds — subscribers never observe a phantom row that
    /// failed to persist.
    subscription_tx: broadcast::Sender<ObservationRow>,
}

impl LocalObservationStore {
    /// Open (or create) a redb-backed `LocalObservationStore` at `path`.
    ///
    /// The parent directory is created if missing. Both observation
    /// tables are materialised on open so the first read does not need
    /// to take a write transaction to create them.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ObservationStoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(ObservationStoreError::Io)?;
        }
        let db = Database::create(path).map_err(map_database_error)?;

        // Materialize both tables up-front.
        {
            let write = db.begin_write().map_err(map_transaction_error)?;
            {
                let _ = write.open_table(ALLOC_STATUS_TABLE).map_err(map_table_error)?;
                let _ = write.open_table(NODE_HEALTH_TABLE).map_err(map_table_error)?;
            }
            write.commit().map_err(map_commit_error)?;
        }

        let (subscription_tx, _) = broadcast::channel(SUBSCRIPTION_CHANNEL_CAPACITY);

        Ok(Self { inner: Arc::new(Inner { db, subscription_tx }) })
    }

    fn emit(&self, row: ObservationRow) {
        // `send` returns `Err` only when there are no active
        // subscribers — that's not a failure.
        let _ = self.inner.subscription_tx.send(row);
    }
}

#[async_trait]
impl ObservationStore for LocalObservationStore {
    async fn write(&self, row: ObservationRow) -> Result<(), ObservationStoreError> {
        let inner = Arc::clone(&self.inner);
        let row_for_commit = row.clone();

        tokio::task::spawn_blocking(move || {
            let write = inner.db.begin_write().map_err(map_transaction_error)?;
            {
                match &row_for_commit {
                    ObservationRow::AllocStatus(r) => {
                        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(r)
                            .map_err(map_rkyv_serialize_error)?;
                        let mut table =
                            write.open_table(ALLOC_STATUS_TABLE).map_err(map_table_error)?;
                        table
                            .insert(r.alloc_id.as_str().as_bytes(), bytes.as_ref())
                            .map_err(map_storage_error)?;
                    }
                    ObservationRow::NodeHealth(r) => {
                        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(r)
                            .map_err(map_rkyv_serialize_error)?;
                        let mut table =
                            write.open_table(NODE_HEALTH_TABLE).map_err(map_table_error)?;
                        table
                            .insert(r.node_id.as_str().as_bytes(), bytes.as_ref())
                            .map_err(map_storage_error)?;
                    }
                }
            }
            write.commit().map_err(map_commit_error)?;
            Ok::<_, ObservationStoreError>(())
        })
        .await
        .map_err(map_join_error)??;

        // Emit after the redb commit succeeds — subscribers never see a
        // row that failed to persist.
        self.emit(row);
        Ok(())
    }

    async fn subscribe_all(&self) -> Result<ObservationSubscription, ObservationStoreError> {
        let rx = self.inner.subscription_tx.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(Result::ok);
        Ok(Box::new(SubscriptionStream { inner: Box::pin(stream) }))
    }

    async fn alloc_status_rows(&self) -> Result<Vec<AllocStatusRow>, ObservationStoreError> {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let read = inner.db.begin_read().map_err(map_transaction_error)?;
            let table = read.open_table(ALLOC_STATUS_TABLE).map_err(map_table_error)?;
            let mut out: Vec<AllocStatusRow> = Vec::new();
            let iter = table.iter().map_err(map_storage_error)?;
            for item in iter {
                let (_k, v) = item.map_err(map_storage_error)?;
                // redb returns a byte slice with unknown alignment; rkyv
                // requires 8-byte-aligned access. Copy into an AlignedVec
                // before deserialising.
                let mut aligned = rkyv::util::AlignedVec::<8>::new();
                aligned.extend_from_slice(v.value());
                let row: AllocStatusRow =
                    rkyv::from_bytes::<AllocStatusRow, rkyv::rancor::Error>(&aligned)
                        .map_err(map_rkyv_deserialize_error)?;
                out.push(row);
            }
            Ok::<_, ObservationStoreError>(out)
        })
        .await
        .map_err(map_join_error)?
    }

    async fn node_health_rows(&self) -> Result<Vec<NodeHealthRow>, ObservationStoreError> {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let read = inner.db.begin_read().map_err(map_transaction_error)?;
            let table = read.open_table(NODE_HEALTH_TABLE).map_err(map_table_error)?;
            let mut out: Vec<NodeHealthRow> = Vec::new();
            let iter = table.iter().map_err(map_storage_error)?;
            for item in iter {
                let (_k, v) = item.map_err(map_storage_error)?;
                // redb returns a byte slice with unknown alignment; rkyv
                // requires 8-byte-aligned access. Copy into an AlignedVec
                // before deserialising.
                let mut aligned = rkyv::util::AlignedVec::<8>::new();
                aligned.extend_from_slice(v.value());
                let row: NodeHealthRow =
                    rkyv::from_bytes::<NodeHealthRow, rkyv::rancor::Error>(&aligned)
                        .map_err(map_rkyv_deserialize_error)?;
                out.push(row);
            }
            Ok::<_, ObservationStoreError>(out)
        })
        .await
        .map_err(map_join_error)?
    }
}

/// Thin `Unpin` wrapper so we can return a `Box<dyn Stream + Unpin>`.
struct SubscriptionStream {
    inner: Pin<Box<dyn Stream<Item = ObservationRow> + Send>>,
}

impl Stream for SubscriptionStream {
    type Item = ObservationRow;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

// -----------------------------------------------------------------------------
// Error mapping helpers — collapse every redb / rkyv / tokio error class
// onto `ObservationStoreError::Io`. The outer trait only distinguishes
// `Unreachable` (gossip) from `Io` — Phase 2's Corrosion impl will grow
// the error surface as needed; Phase 1 folds low-level failures into
// `Io`.
// -----------------------------------------------------------------------------

fn map_database_error(err: redb::DatabaseError) -> ObservationStoreError {
    ObservationStoreError::Io(std::io::Error::other(err))
}

fn map_transaction_error(err: redb::TransactionError) -> ObservationStoreError {
    ObservationStoreError::Io(std::io::Error::other(err))
}

fn map_table_error(err: redb::TableError) -> ObservationStoreError {
    ObservationStoreError::Io(std::io::Error::other(err))
}

fn map_storage_error(err: redb::StorageError) -> ObservationStoreError {
    ObservationStoreError::Io(std::io::Error::other(err))
}

fn map_commit_error(err: redb::CommitError) -> ObservationStoreError {
    ObservationStoreError::Io(std::io::Error::other(err))
}

fn map_join_error(err: tokio::task::JoinError) -> ObservationStoreError {
    ObservationStoreError::Io(std::io::Error::other(err))
}

fn map_rkyv_serialize_error(err: rkyv::rancor::Error) -> ObservationStoreError {
    ObservationStoreError::Io(std::io::Error::other(err))
}

fn map_rkyv_deserialize_error(err: rkyv::rancor::Error) -> ObservationStoreError {
    ObservationStoreError::Io(std::io::Error::other(err))
}
