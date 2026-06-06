//! `RedbJournalStore` — production `JournalStore` adapter per ADR-0063 §1.
//!
//! The workflow journal rides the SAME runtime-owned redb substrate as
//! [`RedbViewStore`](crate::view_store::redb::RedbViewStore): one redb
//! file per node, one shared `Arc<Database>` (ADR-0063 §1
//! one-file-two-layouts). `redb::Database::begin_read` / `begin_write`
//! both take `&self`, so the same `Arc<Database>` is safe to share across
//! the ViewStore and the JournalStore without further locking.
//!
//! The journal's table layout is DISTINCT from the ViewStore's
//! single-blob-overwrite-per-target shape: ONE append-only table
//! `__wf_journal__` keyed `(WorkflowId-bytes, u32)` with CBOR-encoded
//! [`LoadedEntry`] blobs as values (ADR-0063 §3). The reserved
//! `__wf_journal__` name uses the double-underscore prefix outside the
//! `ReconcilerName` validator grammar so it cannot collide with a
//! per-reconciler ViewStore table sharing the same file.
//!
//! # Durability
//!
//! Per ADR-0063 §4 (reusing ADR-0035 §6 `WriteThroughOrdering`), every
//! [`append`](RedbJournalStore::append) performs ONE redb write
//! transaction with `Durability::Immediate` — the commit fsyncs before
//! returning, so a crash after `Ok(())` preserves the entry across the
//! next boot's `load_journal`.
//!
//! # Earned Trust
//!
//! [`probe`](RedbJournalStore::probe) per ADR-0063 §4 writes a sentinel
//! entry under a reserved probe key, fsyncs, reads it back byte-equal,
//! and deletes it — leaving no residue on success. Any failure
//! short-circuits boot via the composition root's
//! `health.startup.refused` emission (wired in step 01-05).
//!
//! # Codec
//!
//! CBOR (`ciborium`), NOT rkyv — the journal is mutable, runtime-owned,
//! additively-evolving memory (ADR-0063 §2), the same codec
//! [`SimJournalStore`](../../../overdrive_sim/adapters/journal/index.html)
//! uses, so the two adapters observe one contract.

use std::sync::Arc;

use async_trait::async_trait;
use redb::{Database, Durability, ReadableTable, TableDefinition};

use super::{
    JournalStore, JournalStoreError, LoadedEntry, ProbeError, Result as JsResult, WorkflowId,
};

/// Reserved append-only table for the workflow journal. The
/// double-underscore prefix is outside the `ReconcilerName` validator's
/// `^[a-z][a-z0-9-]{0,62}$` grammar, so no per-reconciler `ViewStore`
/// table sharing this redb file can collide with it (ADR-0063 §1).
const JOURNAL_TABLE_NAME: &str = "__wf_journal__";

/// The journal table definition: key is `(WorkflowId-as-str, step)`,
/// value is the CBOR-encoded [`LoadedEntry`] blob. The string key
/// component is the `WorkflowId`'s canonical form; the `u32` is the
/// monotonic await-point step index — the tuple gives the ascending
/// `(id, step)` range-scan ordering for free (ADR-0063 §3), mirroring
/// the `SimJournalStore` `BTreeMap<(WorkflowId, u32), _>` key shape.
const JOURNAL_TABLE: TableDefinition<(&str, u32), &[u8]> = TableDefinition::new(JOURNAL_TABLE_NAME);

/// Reserved probe sentinel — a `WorkflowId` outside any real-instance
/// minting scheme, written/read/deleted by [`RedbJournalStore::probe`].
/// Validated by `WorkflowId::new` at use; the
/// `probe_sentinel_id_is_valid` unit test guards against a future
/// validator tightening silently breaking the probe.
const PROBE_WORKFLOW_ID: &str = "probe-wf-earned-trust";
/// Fixed sentinel payload the probe writes so the byte-equal readback is
/// trivial and self-describing.
const PROBE_PAYLOAD: &[u8] = b"redb-journal-store-probe-v1";

/// Production `JournalStore` adapter backed by the shared per-node redb
/// file (ADR-0063 §1).
///
/// Cheap to clone via `Arc<Database>`; safe to share across tasks and
/// alongside [`RedbViewStore`](crate::view_store::redb::RedbViewStore) —
/// `redb::Database` handles internal locking and both `begin_read` /
/// `begin_write` take `&self`.
pub struct RedbJournalStore {
    db: Arc<Database>,
}

impl RedbJournalStore {
    /// Construct over the shared `Arc<Database>` — the SAME handle /
    /// redb file the `RedbViewStore` uses (ADR-0063 §1). The composition
    /// root (step 01-05) opens the database once and hands an
    /// `Arc::clone` to each store.
    #[must_use]
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Encode an entry to CBOR via `ciborium` — the same codec the sim
    /// adapter uses, so any codec skew is caught at the contract boundary.
    fn encode(entry: &LoadedEntry) -> JsResult<Vec<u8>> {
        let mut buf: Vec<u8> = Vec::new();
        ciborium::into_writer(entry, &mut buf)
            .map_err(|e| JournalStoreError::Encode(e.to_string()))?;
        Ok(buf)
    }

    /// Decode CBOR bytes back into a [`LoadedEntry`].
    fn decode(bytes: &[u8]) -> JsResult<LoadedEntry> {
        ciborium::from_reader(bytes).map_err(|e| JournalStoreError::Decode(e.to_string()))
    }

    /// The next step index for `workflow_id`. Append position is 1:1 with
    /// ascending step per the [`JournalStore::append`] contract, and an
    /// instance's steps are contiguous from 0 (`append` is the sole writer
    /// and nothing deletes a real instance's entries), so the next step is
    /// `last_step + 1`. Derived by a reverse peek at the back of the
    /// `(id, 0)..=(id, u32::MAX)` range — `redb::Range` is a
    /// `DoubleEndedIterator`, so `.next_back()` is an O(log N) reverse
    /// B-tree cursor seek, NOT an O(N) forward walk. An empty range (no
    /// entries yet) yields step 0.
    fn next_step(
        table: &impl ReadableTable<(&'static str, u32), &'static [u8]>,
        workflow_id: &WorkflowId,
    ) -> JsResult<u32> {
        let id = workflow_id.as_str();
        match table.range((id, 0u32)..=(id, u32::MAX)).map_err(map_storage_error)?.next_back() {
            None => Ok(0),
            Some(entry) => {
                let (key, _value) = entry.map_err(map_storage_error)?;
                let (_id, last_step) = key.value();
                last_step.checked_add(1).ok_or_else(|| {
                    JournalStoreError::Io(std::io::Error::other(
                        "a single workflow instance cannot exceed u32::MAX journal entries",
                    ))
                })
            }
        }
    }
}

#[async_trait]
impl JournalStore for RedbJournalStore {
    async fn append(&self, workflow_id: &WorkflowId, entry: &LoadedEntry) -> JsResult<()> {
        // Encode BEFORE the write txn so an encode failure surfaces
        // cleanly without touching the store.
        let bytes = Self::encode(entry)?;

        // Synchronous redb call inside an async fn — same rationale as
        // `RedbViewStore::write_through_bytes`: redb's begin_write +
        // commit are ~ms on a single-node store; routing through
        // `spawn_blocking` would add an `.await` round-trip per append
        // that throws off DST-style yield-counting tests.
        let mut write = self.db.begin_write().map_err(map_transaction_error)?;
        // `Durability::Immediate` forces fsync on commit per ADR-0063 §4
        // — the entry is on disk before this returns Ok.
        write.set_durability(Durability::Immediate);
        {
            let mut table = write.open_table(JOURNAL_TABLE).map_err(map_table_error)?;
            // Assign the step by append position within the SAME txn so
            // the count and the insert see one consistent view.
            let step = Self::next_step(&table, workflow_id)?;
            table
                .insert((workflow_id.as_str(), step), bytes.as_slice())
                .map_err(map_storage_error)?;
        }
        write.commit().map_err(map_commit_error)?;
        Ok(())
    }

    async fn load_journal(&self, workflow_id: &WorkflowId) -> JsResult<Vec<LoadedEntry>> {
        let read = self.db.begin_read().map_err(map_transaction_error)?;
        let table = match read.open_table(JOURNAL_TABLE) {
            Ok(t) => t,
            // No journal table yet means no instance has ever appended —
            // an unknown / fresh instance loads as an empty run per the
            // trait contract, never an error.
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(e) => return Err(map_table_error(e)),
        };
        let id = workflow_id.as_str();
        // Range scan `(id, 0)..=(id, u32::MAX)` yields keys in ascending
        // `(id, step)` order; filtered to this instance, that IS ascending
        // step order (ADR-0063 §3).
        let iter = table.range((id, 0u32)..=(id, u32::MAX)).map_err(map_storage_error)?;
        let mut out = Vec::new();
        for entry in iter {
            let (_key, value) = entry.map_err(map_storage_error)?;
            out.push(Self::decode(value.value())?);
        }
        Ok(out)
    }

    async fn probe(&self) -> std::result::Result<(), ProbeError> {
        let probe_id = WorkflowId::new(PROBE_WORKFLOW_ID)
            .unwrap_or_else(|_| unreachable!("PROBE_WORKFLOW_ID is a valid instance id"));
        let key = (probe_id.as_str(), 0u32);

        // Step 1: write the sentinel + commit (fsync). Failure surfaces
        // as `WriteFailed`.
        {
            let mut write = self
                .db
                .begin_write()
                .map_err(|e| ProbeError::WriteFailed { source: map_transaction_error(e) })?;
            write.set_durability(Durability::Immediate);
            {
                let mut table = write
                    .open_table(JOURNAL_TABLE)
                    .map_err(|e| ProbeError::WriteFailed { source: map_table_error(e) })?;
                table
                    .insert(key, PROBE_PAYLOAD)
                    .map_err(|e| ProbeError::WriteFailed { source: map_storage_error(e) })?;
            }
            write.commit().map_err(|e| ProbeError::CommitFailed { source: map_commit_error(e) })?;
        }

        // Step 2: read back byte-equal. Mismatch = engine corruption.
        let got = {
            let read = self
                .db
                .begin_read()
                .map_err(|e| ProbeError::ReadFailed { source: map_transaction_error(e) })?;
            let table = read
                .open_table(JOURNAL_TABLE)
                .map_err(|e| ProbeError::ReadFailed { source: map_table_error(e) })?;
            let entry = table
                .get(key)
                .map_err(|e| ProbeError::ReadFailed { source: map_storage_error(e) })?
                .ok_or_else(|| ProbeError::RoundTripMismatch {
                    wrote: PROBE_PAYLOAD.to_vec(),
                    got: Vec::new(),
                })?;
            entry.value().to_vec()
        };
        if got.as_slice() != PROBE_PAYLOAD {
            return Err(ProbeError::RoundTripMismatch { wrote: PROBE_PAYLOAD.to_vec(), got });
        }

        // Step 3: delete the sentinel + commit so the probe leaves no
        // residue. Failure surfaces as `CleanupFailed`.
        {
            let mut write = self
                .db
                .begin_write()
                .map_err(|e| ProbeError::CleanupFailed { source: map_transaction_error(e) })?;
            write.set_durability(Durability::Immediate);
            {
                let mut table = write
                    .open_table(JOURNAL_TABLE)
                    .map_err(|e| ProbeError::CleanupFailed { source: map_table_error(e) })?;
                let _ = table
                    .remove(key)
                    .map_err(|e| ProbeError::CleanupFailed { source: map_storage_error(e) })?;
            }
            write
                .commit()
                .map_err(|e| ProbeError::CleanupFailed { source: map_commit_error(e) })?;
        }

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Error mapping — collapse redb's error hierarchy onto the
// `JournalStoreError::Io` variant, mirroring `RedbViewStore`. ADR-0063
// reuses ADR-0035's "acceptable in Phase 1" stance; reconcilers/engines
// that branch on a specific redb class can split these later.
// -----------------------------------------------------------------------------

fn map_transaction_error(err: redb::TransactionError) -> JournalStoreError {
    JournalStoreError::Io(std::io::Error::other(err))
}

fn map_table_error(err: redb::TableError) -> JournalStoreError {
    JournalStoreError::Io(std::io::Error::other(err))
}

fn map_storage_error(err: redb::StorageError) -> JournalStoreError {
    JournalStoreError::Io(std::io::Error::other(err))
}

fn map_commit_error(err: redb::CommitError) -> JournalStoreError {
    JournalStoreError::Io(std::io::Error::other(err))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    use overdrive_core::id::ContentHash;

    fn db() -> (tempfile::TempDir, Arc<Database>) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("memory.redb");
        let db = Arc::new(Database::create(path).expect("create redb"));
        (tmp, db)
    }

    use super::super::{JournalCommand, JournalNotification};
    use overdrive_core::workflow::{SignalKey, SignalValue};

    fn workflow_id() -> WorkflowId {
        WorkflowId::new("wf-test-0001").expect("valid id")
    }

    fn started() -> LoadedEntry {
        LoadedEntry::Command(JournalCommand::Started {
            spec_digest: ContentHash::of(b"provision-record"),
            input_digest: ContentHash::of(b"provision-record"),
        })
    }

    /// A `RunResult` command — `step` is no longer an in-entry field
    /// (identity is positional, D5); the `nonce` distinguishes successive
    /// `RunResult`s in a run so a multi-step fixture round-trips distinctly.
    fn run_result(nonce: &str) -> LoadedEntry {
        LoadedEntry::Command(JournalCommand::RunResult {
            name: "provision-write".to_string(),
            result_digest: ContentHash::of(nonce.as_bytes()),
            result_bytes: nonce.as_bytes().to_vec(),
        })
    }

    /// A `SignalSeen` notification — exercises the interleaved
    /// command/notification on-disk stream the dumb store must preserve.
    fn signal_seen() -> LoadedEntry {
        LoadedEntry::Notification(JournalNotification::SignalSeen {
            signal_key: SignalKey::new("provision-ready").expect("valid signal key"),
            value_digest: ContentHash::of(b"signal-value"),
            value: SignalValue::new("signal-value"),
        })
    }

    #[tokio::test]
    async fn append_then_load_round_trips_in_step_order() {
        let (_tmp, db) = db();
        let store = RedbJournalStore::new(db);
        let id = workflow_id();

        // Interleave a notification between commands — the store is a dumb
        // ordered log and must preserve the interleave verbatim (D2).
        let entries = vec![started(), run_result("a"), signal_seen(), run_result("b")];
        for e in &entries {
            store.append(&id, e).await.expect("append");
        }

        let loaded = store.load_journal(&id).await.expect("load");
        assert_eq!(loaded, entries, "real redb run round-trips byte-equal in append order");
    }

    #[tokio::test]
    async fn load_journal_for_unknown_instance_is_empty_not_error() {
        let (_tmp, db) = db();
        let store = RedbJournalStore::new(db);
        let id = WorkflowId::new("wf-never-started").expect("valid id");
        let loaded = store.load_journal(&id).await.expect("load empty");
        assert!(loaded.is_empty(), "an instance with no entries loads as an empty run");
    }

    /// Regression guard for the O(1) `next_step` reverse-peek derivation.
    /// `next_step` must assign a fresh, non-colliding step on every append
    /// across a long contiguous run. A broken `next_step` (one that returns
    /// a step colliding with an existing key) would make the subsequent
    /// `insert` OVERWRITE a prior entry, so `load_journal` would return
    /// fewer distinct entries than were appended — observable here through
    /// the public API. K = 64 exercises the `next_back` path well past a
    /// single step.
    #[tokio::test]
    async fn next_step_assigns_contiguous_non_colliding_steps_across_a_long_run() {
        const K: u32 = 64;
        let (_tmp, db) = db();
        let store = RedbJournalStore::new(Arc::clone(&db));
        let id = workflow_id();

        // Each entry is made unique by varying the nonce, so a collision /
        // overwrite is observable as a missing or duplicated entry.
        let expected: Vec<LoadedEntry> =
            (0..K).map(|n| run_result(&format!("entry-{n}"))).collect();
        for entry in &expected {
            store.append(&id, entry).await.expect("append");
        }

        // Public-API observation: exactly K distinct entries, in append
        // order, no collision / overwrite / drop.
        let loaded = store.load_journal(&id).await.expect("load");
        assert_eq!(loaded.len(), K as usize, "every append must yield a distinct stored entry");
        assert_eq!(loaded, expected, "no collision, no overwrite, preserved append order");

        // Tighter pin of the contiguity invariant `next_step` depends on:
        // the stored step components are exactly the contiguous 0..K.
        let read = db.begin_read().expect("read txn");
        let table = read.open_table(JOURNAL_TABLE).expect("journal table");
        let stored_steps: Vec<u32> = table
            .range((id.as_str(), 0u32)..=(id.as_str(), u32::MAX))
            .expect("range scan")
            .map(|entry| {
                let (key, _value) = entry.expect("range entry");
                let (_id, step) = key.value();
                step
            })
            .collect();
        assert_eq!(
            stored_steps,
            (0..K).collect::<Vec<u32>>(),
            "next_step must produce the contiguous step sequence 0..K"
        );
    }

    #[tokio::test]
    async fn per_instance_isolation_in_shared_table() {
        let (_tmp, db) = db();
        let store = RedbJournalStore::new(db);
        let a = WorkflowId::new("wf-aaaa").expect("id a");
        let b = WorkflowId::new("wf-bbbb").expect("id b");

        store.append(&a, &started()).await.expect("append a");
        store.append(&b, &run_result("a")).await.expect("append b");

        let loaded_a = store.load_journal(&a).await.expect("load a");
        let loaded_b = store.load_journal(&b).await.expect("load b");
        assert_eq!(loaded_a, vec![started()], "instance a sees only its own run");
        assert_eq!(loaded_b, vec![run_result("a")], "instance b sees only its own run");
    }

    #[tokio::test]
    async fn probe_succeeds_and_leaves_no_residue() {
        let (_tmp, db) = db();
        let store = RedbJournalStore::new(db);

        store.probe().await.expect("probe ok on healthy fs");

        // The sentinel must be deleted — its probe id loads as empty.
        let probe_id = WorkflowId::new(PROBE_WORKFLOW_ID).expect("valid probe id");
        let residue = store.load_journal(&probe_id).await.expect("load probe id");
        assert!(residue.is_empty(), "probe must leave no sentinel residue, found {residue:?}");
    }

    /// The probe is NOT a no-op — it actually writes, reads back, and
    /// DELETES the reserved sentinel key. Pre-seeding that key with a
    /// foreign entry distinguishes a real probe (which overwrites then
    /// deletes the key, leaving it empty) from a `probe -> Ok(())`
    /// no-op (which never touches the store, so the foreign entry
    /// survives). The sibling `probe_succeeds_and_leaves_no_residue`
    /// cannot catch the no-op because a clean store is empty both before
    /// and after — the pre-seed is what makes the side effect observable.
    #[tokio::test]
    async fn probe_writes_reads_and_deletes_the_sentinel_not_a_noop() {
        let (_tmp, db) = db();
        let store = RedbJournalStore::new(db);
        let probe_id = WorkflowId::new(PROBE_WORKFLOW_ID).expect("valid probe id");

        // Pre-seed the reserved probe-sentinel key (first append lands at
        // step 0 — the exact key the probe uses) with a foreign entry.
        store.append(&probe_id, &run_result("pre-seed")).await.expect("seed probe key");
        assert!(
            !store.load_journal(&probe_id).await.expect("load seeded").is_empty(),
            "precondition: the probe sentinel key is seeded with a foreign entry"
        );

        store.probe().await.expect("probe ok on healthy fs");

        let after = store.load_journal(&probe_id).await.expect("load after probe");
        assert!(
            after.is_empty(),
            "a real probe writes-then-deletes the sentinel key, leaving it empty; \
             a no-op probe leaves the pre-seeded entry behind, found {after:?}"
        );
    }

    #[tokio::test]
    async fn probe_succeeds_repeatedly() {
        let (_tmp, db) = db();
        let store = RedbJournalStore::new(db);
        store.probe().await.expect("probe 1");
        store.probe().await.expect("probe 2");
        store.probe().await.expect("probe 3");
    }

    #[test]
    fn probe_sentinel_id_is_valid() {
        assert!(WorkflowId::new(PROBE_WORKFLOW_ID).is_ok());
    }
}
