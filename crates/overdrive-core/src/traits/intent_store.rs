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
    /// A snapshot byte slice handed to `bootstrap_from` failed
    /// validation at `offset`. Offsets are expressed in bytes from the
    /// start of the frame; `0` names the magic, `4..6` names the
    /// version word, and any offset ≥ `HEADER_LEN` names the rkyv
    /// payload. Callers rendering this error should print the offset
    /// alongside a hex dump of the surrounding bytes.
    #[error("snapshot frame is corrupted at byte offset {offset}")]
    SnapshotCorrupt {
        /// Byte offset into the snapshot frame where corruption was
        /// first detected.
        offset: usize,
    },
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

/// Portable full-state snapshot — used for `LocalIntentStore → RaftStore`
/// migration, routine Raft snapshots in HA mode, and DR backups.
///
/// The snapshot carries its canonical **framed byte slice** alongside
/// the decoded `entries` view. Byte identity is the migration contract:
/// `export_snapshot → bootstrap_from → export_snapshot` must produce a
/// bit-identical `bytes` slice regardless of insertion order, backing
/// path, or store instance. The decoded `entries` remain available for
/// callers that want to inspect contents without re-parsing the frame.
///
/// The concrete framing (magic `OSNP` + 2-byte LE version + rkyv
/// payload) is documented in the `overdrive-store-local` crate's
/// `snapshot_frame` module — it is shared with the future `RaftStore`
/// so that a single-mode export can be replayed as the initial Raft
/// log entry without re-encoding.
#[derive(Debug, Clone)]
pub struct StateSnapshot {
    pub version: u32,
    pub entries: Vec<(Bytes, Bytes)>,
    /// Canonical framed form of this snapshot. Two snapshots of
    /// semantically-equal store contents produce byte-identical slices
    /// here — this is what migration consumers compare.
    bytes: Vec<u8>,
}

impl StateSnapshot {
    /// Construct a `StateSnapshot` from its logical components plus its
    /// canonical framed byte slice.
    ///
    /// Callers are expected to have produced `bytes` via a framing
    /// routine that matches the single documented layout (magic +
    /// version + rkyv payload with entries sorted by key). No
    /// validation is performed here — the store that produced this
    /// snapshot is responsible for framing consistency.
    // `const fn` is vacuous here: `bytes::Bytes` carries an `Arc`
    // internally, so a caller who wanted a genuinely `const`
    // StateSnapshot would not be able to produce the `Vec<(Bytes,
    // Bytes)>` argument in a `const` context anyway. Keeping the
    // signature non-`const` makes that clear at the call site.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn from_parts(version: u32, entries: Vec<(Bytes, Bytes)>, bytes: Vec<u8>) -> Self {
        Self { version, entries, bytes }
    }

    /// Canonical framed byte slice. Migration consumers compare by
    /// value equality on this slice; it is also what gets written to
    /// Garage for DR backups and what `RaftStore` consumes to seed a
    /// new HA cluster.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
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
    /// bootstrapping a new HA cluster from a `LocalIntentStore` export.
    async fn bootstrap_from(&self, snapshot: StateSnapshot) -> Result<(), IntentStoreError>;
}

#[cfg(test)]
mod state_snapshot_tests {
    //! Unit witnesses for [`StateSnapshot`]'s component getters.
    //!
    //! These are mutation-testing seams: `cargo mutants` targets the
    //! `bytes()` getter independently from the storage-crate tests that
    //! drive it end-to-end, and without a same-crate test every
    //! getter-returns-`Vec::leak(Vec::new())` mutation is `MISSED`.
    use super::*;
    use bytes::Bytes;

    #[test]
    fn state_snapshot_bytes_returns_the_canonical_slice_supplied_to_from_parts() {
        let canonical: Vec<u8> = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01, 0x02];
        let snap = StateSnapshot::from_parts(
            1,
            vec![(Bytes::from_static(b"k"), Bytes::from_static(b"v"))],
            canonical.clone(),
        );
        assert_eq!(
            snap.bytes(),
            canonical.as_slice(),
            "bytes() must return the exact slice handed to from_parts"
        );
        // A distinct canonical slice must produce a distinct `bytes()`
        // projection — guards against a mutation that returns the same
        // static slice regardless of input.
        let other = StateSnapshot::from_parts(1, Vec::new(), vec![0x11, 0x22]);
        assert_ne!(snap.bytes(), other.bytes(), "bytes() must reflect per-instance state");
        assert_eq!(other.bytes(), &[0x11u8, 0x22u8][..]);
    }

    #[test]
    fn state_snapshot_bytes_is_non_empty_when_from_parts_receives_non_empty_bytes() {
        let snap = StateSnapshot::from_parts(1, Vec::new(), vec![0xAA]);
        assert!(!snap.bytes().is_empty(), "bytes() must reflect the non-empty input");
        assert_eq!(snap.bytes().len(), 1);
    }
}
