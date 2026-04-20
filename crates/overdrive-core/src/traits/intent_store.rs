//! [`IntentStore`] — linearizable authoritative storage.
//!
//! Jobs, policies, certificates, scheduler decisions — every declaration
//! of *what should be*. Single mode is backed by redb; HA mode by
//! openraft + redb. Simulation uses the single-mode path since Raft itself
//! is tested by dedicated consensus tests.
//!
//! Every mutation from a reconciler or workflow arrives here as a typed
//! action; this trait does not expose a raw `put(key, value)` surface.

use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IntentStoreError {
    #[error("intent store busy (retry)")]
    Busy,
    #[error("key not found")]
    NotFound,
    #[error("transaction conflict")]
    Conflict,
    #[error("snapshot import failed: {0}")]
    SnapshotImport(String),
    #[error("intent store I/O: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub enum TxnOp {
    Put { key: Bytes, value: Bytes },
    Delete { key: Bytes },
}

#[derive(Debug, Clone)]
pub enum TxnOutcome {
    Committed,
    Conflict,
}

/// Portable full-state snapshot — used for `LocalStore → RaftStore`
/// migration, routine Raft snapshots in HA mode, and DR backups.
#[derive(Debug, Clone)]
pub struct StateSnapshot {
    pub version: u32,
    pub entries: Vec<(Bytes, Bytes)>,
}

#[async_trait]
pub trait IntentStore: Send + Sync + 'static {
    async fn get(&self, key: &[u8]) -> Result<Option<Bytes>, IntentStoreError>;

    async fn put(&self, key: &[u8], value: &[u8]) -> Result<(), IntentStoreError>;

    async fn delete(&self, key: &[u8]) -> Result<(), IntentStoreError>;

    async fn txn(&self, ops: Vec<TxnOp>) -> Result<TxnOutcome, IntentStoreError>;

    /// Watch for changes under a key prefix. Each item is `(key, value)`;
    /// deletes are reported as empty `value`.
    async fn watch(
        &self,
        prefix: &[u8],
    ) -> Result<Box<dyn Stream<Item = (Bytes, Bytes)> + Send + Unpin>, IntentStoreError>;

    /// Export full state. Used for migration, DR, and Raft snapshots.
    async fn export_snapshot(&self) -> Result<StateSnapshot, IntentStoreError>;

    /// Replay a snapshot as the initial state — used by `RaftStore` when
    /// bootstrapping a new HA cluster from a `LocalStore` export.
    async fn bootstrap_from(&self, snapshot: StateSnapshot) -> Result<(), IntentStoreError>;
}
