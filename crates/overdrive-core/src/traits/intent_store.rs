//! [`IntentStore`] — linearizable authoritative storage.
//!
//! Jobs, policies, certificates, scheduler decisions — every declaration
//! of *what should be*. Single mode is backed by redb; HA mode by
//! openraft + redb. Simulation uses the single-mode path since Raft itself
//! is tested by dedicated consensus tests.
//!
//! Every mutation from a reconciler or workflow arrives here as a typed
//! action; this trait does not expose a raw `put(key, value)` surface.

use std::path::PathBuf;

use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;
use thiserror::Error;

use crate::codec::EnvelopeError;

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
    // SCAFFOLD: true — RED scaffold per ADR-0048 § 3 (intent fail-fast
    // policy) + § 6 (operator remediation). The `Display` form names
    // the redb path twice — once in "decode failed for {redb_path}"
    // and once in the remediation hint "delete {redb_path}" — so
    // operators see the path on the failure line and the recovery
    // command on the same render. Lands GREEN in DELIVER step 01-04
    // when `LocalStore::open` wires the envelope decode path.
    #[error(
        "intent envelope decode failed for {redb_path}: {source}. Remediation: delete {redb_path} and restart the control-plane",
        redb_path = redb_path.display()
    )]
    Envelope {
        redb_path: PathBuf,
        #[source]
        source: EnvelopeError,
    },
}

#[derive(Debug, Clone)]
pub enum TxnOp {
    Put {
        key: Bytes,
        value: Bytes,
    },
    Delete {
        key: Bytes,
    },
    /// Read the big-endian `u64` at `key` (absent ⇒ `0`), write
    /// `current + 1` (saturating at `u64::MAX`). The read and the write
    /// happen inside the SAME store write transaction as every other op
    /// in the [`txn`](IntentStore::txn) batch — see the [`txn`] method
    /// contract for the full preconditions / postconditions / edge
    /// cases / observable invariant.
    ///
    /// This is the atomic-monotonic-increment primitive. It cannot be
    /// expressed by [`Put`](TxnOp::Put) (a blind write that loses
    /// concurrent bumps and can drive the value backwards on a stale
    /// read) or by [`put_if_absent`](IntentStore::put_if_absent)
    /// (insert-if-absent, no increment). The read-modify-write inside
    /// the store's exclusive write transaction is what makes concurrent
    /// bumps serialise with no lost increment.
    ///
    /// # Preconditions
    ///
    /// `key` may name an absent row (treated as the `u64` `0`) or a row
    /// holding exactly 8 big-endian bytes. A row at `key` whose length
    /// is not 8 is decoded as `0` per `development.md` § "Safe
    /// byte-slice access" (length-guarded decode) — never a panic.
    ///
    /// # Postconditions
    ///
    /// After a [`txn`] containing `IncrementU64 { key }` returns
    /// `Ok(TxnOutcome::Committed)`, a subsequent [`get`](IntentStore::get)
    /// of `key` returns the 8-byte BE encoding of `prev + 1` (saturating
    /// at `u64::MAX`), where `prev` is the value visible at the instant
    /// the batch's write transaction began. The increment and every
    /// sibling op in the same batch commit atomically — there is no
    /// observable state in which the increment landed but a sibling
    /// [`Delete`](TxnOp::Delete) did not, or vice versa.
    ///
    /// # Edge cases
    ///
    /// * Absent key ⇒ post-state is `1`.
    /// * Row of `< 8` or `> 8` bytes ⇒ decoded as `0`, post-state `1`
    ///   (the read path is corruption-tolerant; the write path always
    ///   emits canonical 8-byte BE).
    /// * `u64::MAX` ⇒ saturates, stays `u64::MAX` (the monotonic-advance
    ///   contract degrades to "no further advance" at the ceiling, never
    ///   wraps to a lower value that would wedge a reconciler comparing
    ///   `observed < desired`).
    ///
    /// # Observable invariant
    ///
    /// Across any number of concurrent [`txn`]s each carrying one
    /// `IncrementU64 { key }`, the final value equals the count of those
    /// `txn`s that committed (modulo the `u64::MAX` saturation ceiling).
    /// The sequence of values a serial reader would observe is strictly
    /// non-decreasing. No committed increment is lost; the value never
    /// goes backwards.
    ///
    /// [`txn`]: IntentStore::txn
    IncrementU64 {
        key: Bytes,
    },
}

#[derive(Debug, Clone)]
pub enum TxnOutcome {
    Committed,
    Conflict,
}

/// Outcome of an atomic compare-and-set `put_if_absent` against the
/// [`IntentStore`].
///
/// The existence check and the write happen inside a single store
/// transaction — a concurrent caller racing on the same key cannot
/// observe an intermediate state where both callers see `None` on the
/// read and both fall through to a blind `put`. Exactly one caller
/// wins with [`Inserted`]; every other caller loses with
/// [`KeyExists`], receiving the bytes that actually occupy the key.
///
/// Handlers use the returned bytes to distinguish idempotent
/// re-submission (byte-identical to the losing caller's payload) from
/// genuine conflict (different payload at the same key) — see
/// `overdrive_control_plane::handlers::submit_workload`.
///
/// [`Inserted`]: PutOutcome::Inserted
/// [`KeyExists`]: PutOutcome::KeyExists
#[derive(Debug, Clone)]
pub enum PutOutcome {
    /// The key was absent when the transaction began; the new value
    /// was written.
    Inserted,
    /// The key was already populated when the transaction began; no
    /// write occurred. `existing` carries the bytes that currently
    /// occupy the key so the caller can compare byte-for-byte before
    /// deciding whether to return 200 (idempotent) or 409 (conflict).
    KeyExists {
        /// The bytes currently occupying the key.
        existing: Bytes,
    },
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
    /// Read the bytes at `key`.
    ///
    /// Returns `Ok(None)` when the key is absent, `Ok(Some(bytes))`
    /// when populated. The bytes returned are the caller-provided
    /// bytes as passed to [`put`], [`put_if_absent`], or
    /// [`TxnOp::Put`] — implementations do not surface any internal
    /// row encoding.
    ///
    /// [`put`]: Self::put
    /// [`put_if_absent`]: Self::put_if_absent
    async fn get(&self, key: &[u8]) -> Result<Option<Bytes>, IntentStoreError>;

    async fn put(&self, key: &[u8], value: &[u8]) -> Result<(), IntentStoreError>;

    /// Atomic compare-and-set — insert `value` at `key` only if `key`
    /// is currently absent. The existence check and the write happen
    /// inside a single store transaction, so two concurrent callers
    /// racing on the same key cannot both see the key as absent and
    /// both commit a write: exactly one wins with
    /// [`PutOutcome::Inserted`]; the other observes
    /// [`PutOutcome::KeyExists`] carrying the bytes that actually
    /// occupy the key.
    ///
    /// This is the correct primitive for handlers that need to
    /// implement an idempotent create-or-conflict HTTP contract (`POST
    /// /v1/jobs`); a naive `get` followed by a separate `put` has a
    /// TOCTOU window and silently loses writes under concurrency.
    async fn put_if_absent(&self, key: &[u8], value: &[u8])
    -> Result<PutOutcome, IntentStoreError>;

    async fn delete(&self, key: &[u8]) -> Result<(), IntentStoreError>;

    /// Apply a batch of [`TxnOp`]s atomically inside a single store write
    /// transaction. Either every op in `ops` commits or none does; a
    /// concurrent reader observes the pre-batch state or the fully-applied
    /// post-batch state, never an intermediate view.
    ///
    /// The supported ops are [`Put`](TxnOp::Put) (blind write),
    /// [`Delete`](TxnOp::Delete) (idempotent remove — deleting an absent
    /// key is a no-op, not an error), and [`IncrementU64`](TxnOp::IncrementU64)
    /// (atomic-monotonic read-modify-write of a big-endian `u64`).
    ///
    /// # `IncrementU64` contract
    ///
    /// * **Preconditions.** A targeted `key` may name an absent row
    ///   (treated as the `u64` `0`) or a row holding exactly 8 big-endian
    ///   bytes. A row whose length is not 8 is decoded as `0` per
    ///   `development.md` § "Safe byte-slice access" — never a panic.
    /// * **Postconditions.** After this method returns
    ///   `Ok(TxnOutcome::Committed)`, a subsequent [`get`](Self::get) of
    ///   the incremented `key` returns the 8-byte BE encoding of
    ///   `prev + 1` (saturating at `u64::MAX`), where `prev` is the value
    ///   visible at the instant the batch's write transaction began. The
    ///   increment and every sibling op commit atomically — no observable
    ///   state in which the increment landed but a sibling delete did not.
    /// * **Edge cases.** Absent key ⇒ post-state `1`. Row of `< 8` or
    ///   `> 8` bytes ⇒ decoded `0`, post-state `1`. `u64::MAX` ⇒
    ///   saturates, never wraps to a lower value.
    /// * **Observable invariant.** Across any number of concurrent `txn`s
    ///   each carrying one `IncrementU64 { key }`, the final value equals
    ///   the count of committed `txn`s (modulo the `u64::MAX` ceiling); a
    ///   serial reader observes a strictly non-decreasing sequence. No
    ///   committed increment is lost; the value never goes backwards.
    async fn txn(&self, ops: Vec<TxnOp>) -> Result<TxnOutcome, IntentStoreError>;

    /// Watch for changes under a key prefix. Each item is `(key, value)`;
    /// deletes are reported as empty `value`.
    ///
    /// `value` is the **caller-provided bytes** as passed to [`put`],
    /// [`put_if_absent`], or [`TxnOp::Put`]. Subscribers that
    /// subsequently call [`get`] on the same key receive the same
    /// logical value.
    ///
    /// [`put`]: Self::put
    /// [`put_if_absent`]: Self::put_if_absent
    /// [`get`]: Self::get
    async fn watch(
        &self,
        prefix: &[u8],
    ) -> Result<Box<dyn Stream<Item = (Bytes, Bytes)> + Send + Unpin>, IntentStoreError>;

    /// Scan every `(key, value)` pair whose `key` begins with
    /// `prefix`, returning them as an owned `Vec` in ascending
    /// (lexicographic) key order.
    ///
    /// # Preconditions
    ///
    /// `prefix` may be empty (returns every row in the store) or any
    /// byte sequence (returns only rows whose key starts with those
    /// bytes). The prefix does not need to align to a structured key
    /// boundary — the operation is a byte-level prefix match.
    ///
    /// # Postconditions
    ///
    /// On `Ok(rows)`:
    /// * Every returned `(key, value)` satisfies `key.starts_with(prefix)`.
    /// * `value` is the caller-provided bytes from the original
    ///   [`put`] / [`put_if_absent`] / [`TxnOp::Put`] — implementations
    ///   do not transform the row payload.
    /// * Rows are returned in ascending lexicographic key order
    ///   (deterministic; what `BTreeMap`-style iteration provides).
    /// * The empty prefix returns every row in the store; an
    ///   unmatched prefix returns an empty `Vec`.
    ///
    /// # Edge cases
    ///
    /// * Empty `prefix` → returns every row.
    /// * `prefix` matches no rows → returns `Ok(vec![])`.
    /// * Concurrent writers → the returned snapshot is consistent
    ///   with a single point-in-time read transaction; subsequent
    ///   writes do not affect the already-returned `Vec`.
    ///
    /// # Use cases
    ///
    /// The primary caller today is
    /// `overdrive_dataplane::allocators::PersistentServiceVipAllocator::bulk_load`,
    /// which reconstructs the allocator's in-memory state by scanning
    /// every persisted entry under the allocator's namespace prefix.
    /// Future callers (additional typed namespaces, reconciler
    /// bootstrap hydration) follow the same pattern.
    ///
    /// [`put`]: Self::put
    /// [`put_if_absent`]: Self::put_if_absent
    async fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Bytes, Bytes)>, IntentStoreError>;

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
