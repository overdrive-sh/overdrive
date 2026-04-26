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
//! contract from the `ObservationStore` trait. Subscribers that lag
//! past the broadcast capacity drop the lagged notifications silently
//! and continue delivering subsequent events; the stream does not
//! close, so a caller relying on end-of-stream as a catch-up trigger
//! will miss the lost events. Phase 2's Corrosion replacement
//! recovers via CR-SQLite gossip catch-up.
//!
//! # LWW guard on `write`
//!
//! Per the `ObservationStore::write` trait contract codified in
//! `overdrive-core`, a write whose `updated_at` (alloc-status) or
//! `last_heartbeat` (node-health) does not dominate the existing row
//! at the same primary key MUST NOT mutate state and MUST NOT be
//! emitted on subscriptions. This implementation runs the comparison
//! INSIDE the redb `begin_write` transaction (no TOCTOU window) and
//! suppresses the post-commit broadcast on loss. Comparator:
//! [`overdrive_core::traits::observation_store::LogicalTimestamp::dominates`].
//! See `docs/feature/fix-observation-lww-merge/deliver/rca.md` for the
//! bug RCA and `docs/product/architecture/adr-0012-observation-store-server-impl.md`
//! for the third-revision rationale.

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
use redb::{Database, ReadableTable, Table, TableDefinition};
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
/// single-node workload without backing memory to the moon. Subscribers
/// that lag past this silently lose the dropped notifications and keep
/// receiving subsequent ones — the stream does not close on lag (see
/// module docs).
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
        let db = Database::create(path).map_err(map_to_io)?;

        // Materialize both tables up-front.
        {
            let write = db.begin_write().map_err(map_to_io)?;
            {
                let _ = write.open_table(ALLOC_STATUS_TABLE).map_err(map_to_io)?;
                let _ = write.open_table(NODE_HEALTH_TABLE).map_err(map_to_io)?;
            }
            write.commit().map_err(map_to_io)?;
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

        // The LWW comparison runs INSIDE the `begin_write` transaction
        // — a TOCTOU between read and insert is impossible because
        // redb's serializable isolation already linearises writers.
        // The closure returns whether the write was accepted; the
        // post-await branch suppresses `self.emit` on LWW reject.
        // See `ObservationStore::write`'s trait docstring in
        // `overdrive-core` for the trait-level contract; see
        // `docs/feature/fix-observation-lww-merge/deliver/rca.md` for
        // the bug RCA that motivated this guard.
        let accepted: bool = tokio::task::spawn_blocking(move || {
            let write = inner.db.begin_write().map_err(map_to_io)?;
            let accepted = match &row_for_commit {
                ObservationRow::AllocStatus(incoming) => {
                    let mut table = write.open_table(ALLOC_STATUS_TABLE).map_err(map_to_io)?;
                    apply_alloc_status_lww(&mut table, incoming)?
                }
                ObservationRow::NodeHealth(incoming) => {
                    let mut table = write.open_table(NODE_HEALTH_TABLE).map_err(map_to_io)?;
                    apply_node_health_lww(&mut table, incoming)?
                }
            };
            // Commit unconditionally — a rejected write performed only
            // a read inside the transaction; redb handles the no-op
            // commit cleanly.
            write.commit().map_err(map_to_io)?;
            Ok::<_, ObservationStoreError>(accepted)
        })
        .await
        .map_err(map_to_io)??;

        // Suppress emit on LWW reject — subscribers must NEVER observe
        // a row the store will then refuse to return on read. Matches
        // `SimObservationStore::apply_alloc_status` /
        // `apply_node_health` semantics: the broadcast `send` happens
        // only inside the dominate branch.
        if accepted {
            self.emit(row);
        }
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
            let read = inner.db.begin_read().map_err(map_to_io)?;
            let table = read.open_table(ALLOC_STATUS_TABLE).map_err(map_to_io)?;
            let mut out: Vec<AllocStatusRow> = Vec::new();
            let iter = table.iter().map_err(map_to_io)?;
            for item in iter {
                let (_k, v) = item.map_err(map_to_io)?;
                // redb returns a byte slice with unknown alignment; rkyv
                // requires 8-byte-aligned access. Copy into an AlignedVec
                // before deserialising.
                let mut aligned = rkyv::util::AlignedVec::<8>::new();
                aligned.extend_from_slice(v.value());
                let row: AllocStatusRow =
                    rkyv::from_bytes::<AllocStatusRow, rkyv::rancor::Error>(&aligned)
                        .map_err(map_to_io)?;
                out.push(row);
            }
            Ok::<_, ObservationStoreError>(out)
        })
        .await
        .map_err(map_to_io)?
    }

    async fn node_health_rows(&self) -> Result<Vec<NodeHealthRow>, ObservationStoreError> {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let read = inner.db.begin_read().map_err(map_to_io)?;
            let table = read.open_table(NODE_HEALTH_TABLE).map_err(map_to_io)?;
            let mut out: Vec<NodeHealthRow> = Vec::new();
            let iter = table.iter().map_err(map_to_io)?;
            for item in iter {
                let (_k, v) = item.map_err(map_to_io)?;
                // redb returns a byte slice with unknown alignment; rkyv
                // requires 8-byte-aligned access. Copy into an AlignedVec
                // before deserialising.
                let mut aligned = rkyv::util::AlignedVec::<8>::new();
                aligned.extend_from_slice(v.value());
                let row: NodeHealthRow =
                    rkyv::from_bytes::<NodeHealthRow, rkyv::rancor::Error>(&aligned)
                        .map_err(map_to_io)?;
                out.push(row);
            }
            Ok::<_, ObservationStoreError>(out)
        })
        .await
        .map_err(map_to_io)?
    }
}

// -----------------------------------------------------------------------------
// LWW-guarded inserts — read-then-conditional-insert inside the open
// `begin_write` transaction. The `ObservationStore::write` trait
// docstring in `overdrive-core` codifies the contract: an incoming row
// whose `updated_at` does not dominate the existing row at the same
// primary key MUST NOT mutate state.
//
// Returns `true` when the write was accepted (the row dominates a
// prior, or there is no prior); `false` when the write loses to an
// existing row. The caller (`LocalObservationStore::write`) gates the
// post-commit emit on the returned bool — losers must never be emitted
// on subscriptions.
// -----------------------------------------------------------------------------

/// Decode a prior rkyv-archived `AllocStatusRow` from redb-returned
/// bytes. Mirrors the alignment-aware decoding pattern at the top of
/// [`LocalObservationStore::alloc_status_rows`] — redb returns slices
/// with unknown alignment; rkyv requires 8-byte alignment.
fn decode_alloc_status(bytes: &[u8]) -> Result<AllocStatusRow, ObservationStoreError> {
    let mut aligned = rkyv::util::AlignedVec::<8>::new();
    aligned.extend_from_slice(bytes);
    rkyv::from_bytes::<AllocStatusRow, rkyv::rancor::Error>(&aligned).map_err(map_to_io)
}

/// Decode a prior rkyv-archived `NodeHealthRow` from redb-returned
/// bytes. See [`decode_alloc_status`] for the alignment rationale.
fn decode_node_health(bytes: &[u8]) -> Result<NodeHealthRow, ObservationStoreError> {
    let mut aligned = rkyv::util::AlignedVec::<8>::new();
    aligned.extend_from_slice(bytes);
    rkyv::from_bytes::<NodeHealthRow, rkyv::rancor::Error>(&aligned).map_err(map_to_io)
}

/// LWW-guarded insert for `AllocStatusRow`. Reads the prior row at
/// `incoming.alloc_id` (if any), compares via
/// [`overdrive_core::traits::observation_store::LogicalTimestamp::dominates`],
/// and inserts only on dominate. Returns `true` if the row was inserted.
fn apply_alloc_status_lww(
    table: &mut Table<'_, &[u8], &[u8]>,
    incoming: &AllocStatusRow,
) -> Result<bool, ObservationStoreError> {
    let key = incoming.alloc_id.as_str().as_bytes();
    let dominates = match table.get(key).map_err(map_to_io)? {
        None => true,
        Some(prior) => {
            let prior_row = decode_alloc_status(prior.value())?;
            incoming.updated_at.dominates(&prior_row.updated_at)
        }
    };
    if dominates {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(incoming).map_err(map_to_io)?;
        table.insert(key, bytes.as_ref()).map_err(map_to_io)?;
    }
    Ok(dominates)
}

/// LWW-guarded insert for `NodeHealthRow`. Mirrors
/// [`apply_alloc_status_lww`] — keyed by `incoming.node_id`, compares
/// `incoming.last_heartbeat` via `LogicalTimestamp::dominates`.
fn apply_node_health_lww(
    table: &mut Table<'_, &[u8], &[u8]>,
    incoming: &NodeHealthRow,
) -> Result<bool, ObservationStoreError> {
    let key = incoming.node_id.as_str().as_bytes();
    let dominates = match table.get(key).map_err(map_to_io)? {
        None => true,
        Some(prior) => {
            let prior_row = decode_node_health(prior.value())?;
            incoming.last_heartbeat.dominates(&prior_row.last_heartbeat)
        }
    };
    if dominates {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(incoming).map_err(map_to_io)?;
        table.insert(key, bytes.as_ref()).map_err(map_to_io)?;
    }
    Ok(dominates)
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
// Error mapping helper — collapses every redb / rkyv / tokio error class
// onto `ObservationStoreError::Io`. The outer trait only distinguishes
// `Unreachable` (gossip) from `Io` — Phase 2's Corrosion impl will grow
// the error surface as needed; Phase 1 folds low-level failures into
// `Io`.
//
// Generic over any `std::error::Error + Send + Sync + 'static` source
// so the eight distinct concrete error types (`redb::DatabaseError`,
// `redb::TransactionError`, `redb::TableError`, `redb::StorageError`,
// `redb::CommitError`, `tokio::task::JoinError`, and the two
// `rkyv::rancor::Error` lanes) route through one definition instead of
// eight type-specialised stubs. The function-pointer coercion
// (`map_err(map_to_io)`) requires a concrete fn type at each call
// site; turbofishing the generic parameter pins it.
// -----------------------------------------------------------------------------

fn map_to_io<E>(err: E) -> ObservationStoreError
where
    E: std::error::Error + Send + Sync + 'static,
{
    ObservationStoreError::Io(std::io::Error::other(err))
}
