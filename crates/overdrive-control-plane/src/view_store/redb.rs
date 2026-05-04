//! `RedbViewStore` — production `ViewStore` adapter per ADR-0035 §4.
//!
//! One redb file per node at `<data_dir>/reconcilers/memory.redb`,
//! deliberately separate from the `IntentStore` redb file (`wave-decisions`
//! §O4 — different storage tier, different access pattern). One redb
//! table per `ReconcilerName` (`wave-decisions` §O2), keyed on
//! `TargetResource::display()` with CBOR-encoded `View` blobs as the
//! value. Tables are opened-or-created lazily on first
//! `write_through_bytes` for that reconciler.
//!
//! # Durability
//!
//! Per ADR-0035 §4, every `write_through_bytes` performs a single redb
//! write transaction with `Durability::Immediate` — the commit fsyncs
//! before returning. The runtime's in-memory map update is the
//! caller's responsibility (step 01-06) and follows the
//! `WriteThroughOrdering` invariant: persist first, mutate in-memory
//! second.
//!
//! # Earned Trust
//!
//! `probe()` per ADR-0035 § Earned Trust uses a dedicated
//! `__probe__` table with a fixed sentinel key/value pair. The
//! sequence is write → commit (fsync) → read back byte-equal → delete
//! → commit. Every leg is an opportunity to detect a degraded fs;
//! failure short-circuits boot via the runtime's `health.startup.refused`
//! emission (step 01-06).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use redb::{Database, Durability, ReadableTable, TableDefinition};

use overdrive_core::reconciler::TargetResource;

use super::{ProbeError, Result as VsResult, ViewStore, ViewStoreError};

/// Reserved table name for the Earned-Trust probe sentinel row. The
/// double-underscore prefix is outside the `ReconcilerName` validator's
/// `^[a-z][a-z0-9-]{0,62}$` grammar so no real reconciler can collide
/// with it.
const PROBE_TABLE_NAME: &str = "__probe__";

/// Sentinel key/value the probe writes; both fixed compile-time
/// constants so the byte-equal readback assertion is trivial.
const PROBE_KEY: &str = "earned-trust";
const PROBE_PAYLOAD: &[u8] = b"redb-view-store-probe-v1";

/// Production `ViewStore` adapter backed by a single redb file per node.
///
/// Cheap to clone via `Arc<Database>`; safe to share across tasks.
/// `redb::Database` itself handles internal locking — `begin_read` and
/// `begin_write` both take `&self`.
pub struct RedbViewStore {
    db: Arc<Database>,
}

impl RedbViewStore {
    /// Open (or create) a `RedbViewStore` rooted at `data_dir`. The
    /// actual redb file lives at `<data_dir>/reconcilers/memory.redb`
    /// per ADR-0035 §4. The intermediate `reconcilers/` directory is
    /// created if missing — matches `LocalIntentStore::open` so the
    /// boot path does not depend on caller ordering.
    ///
    /// redb's file lock makes concurrent opens fail with a
    /// `DatabaseAlreadyOpen` error class, which we surface as
    /// [`ViewStoreError::Io`].
    pub fn open(data_dir: impl AsRef<Path>) -> VsResult<Self> {
        let path = Self::resolve_path(data_dir.as_ref());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Database::create(&path).map_err(map_database_error)?;
        Ok(Self { db: Arc::new(db) })
    }

    /// Compute the canonical file path under `data_dir`. Public for
    /// step 01-06 wiring tests that want to assert on the path shape
    /// without opening the store.
    #[must_use]
    pub fn resolve_path(data_dir: &Path) -> PathBuf {
        data_dir.join("reconcilers").join("memory.redb")
    }

    /// Test-only convenience: byte-level `bulk_load` that does NOT
    /// CBOR-decode. Used by integration tests that want to inspect the
    /// raw stored bytes (e.g. asserting the probe table is empty under
    /// any user reconciler name without forcing a `V` type).
    #[doc(hidden)]
    pub async fn bulk_load_bytes_for_test(
        &self,
        reconciler: &'static str,
    ) -> VsResult<BTreeMap<TargetResource, Vec<u8>>> {
        self.bulk_load_bytes(reconciler).await
    }
}

/// Probe table definition — fixed `'static` literal, no leak.
const PROBE_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new(PROBE_TABLE_NAME);

#[async_trait]
impl ViewStore for RedbViewStore {
    async fn bulk_load_bytes(
        &self,
        reconciler: &'static str,
    ) -> VsResult<BTreeMap<TargetResource, Vec<u8>>> {
        // Synchronous redb call inside an async fn — see
        // `write_through_bytes` for the rationale.
        let read = self.db.begin_read().map_err(map_transaction_error)?;
        // `TableDefinition::new(reconciler)` is a free `const`
        // constructor — `reconciler: &'static str` is the static
        // lifetime `redb::TableDefinition` requires. Per the
        // `refactor-reconciler-static-name` RCA, inlining at the call
        // site is the load-bearing simplification: there is no helper,
        // no leak, no interner. Doc / regression test:
        // `tests/integration/redb_view_store_no_leak.rs`.
        let table_def: TableDefinition<&'static str, &'static [u8]> =
            TableDefinition::new(reconciler);
        let table = match read.open_table(table_def) {
            Ok(t) => t,
            // A reconciler with no persisted rows simply has no
            // table yet — return an empty map per the trait
            // contract. Other table errors propagate.
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(BTreeMap::new()),
            Err(e) => return Err(map_table_error(e)),
        };
        let mut out = BTreeMap::new();
        let iter = table.iter().map_err(map_storage_error)?;
        for entry in iter {
            let (k, v) = entry.map_err(map_storage_error)?;
            let key_str = k.value();
            let target = TargetResource::new(key_str).map_err(|e| {
                ViewStoreError::Io(std::io::Error::other(format!(
                    "stored target key {key_str:?} failed validation: {e}"
                )))
            })?;
            out.insert(target, v.value().to_vec());
        }
        Ok(out)
    }

    async fn write_through_bytes(
        &self,
        reconciler: &'static str,
        target: &TargetResource,
        cbor: &[u8],
    ) -> VsResult<()> {
        // Synchronous redb call inside an async fn. redb's
        // `begin_write` + `commit` are ~ms operations on a single-node
        // store; the cost of NOT routing through `spawn_blocking` is
        // a brief block on the current tokio worker, which is the
        // right tradeoff for Phase 1 single-node single-tenant
        // control planes. The cost of routing through
        // `spawn_blocking` is an additional `.await` round-trip per
        // tick, which throws off DST-style tests that count yields.
        let mut write = self.db.begin_write().map_err(map_transaction_error)?;
        // `Durability::Immediate` forces fsync on commit per
        // ADR-0035 §4 — the runtime's in-memory map update relies
        // on the row being on disk before this call returns Ok.
        write.set_durability(Durability::Immediate);
        {
            // See note on `bulk_load_bytes` — inlining the
            // `TableDefinition::new` call closes the door on the
            // `Box::leak`-per-call shape from before
            // `refactor-reconciler-static-name`.
            let table_def: TableDefinition<&'static str, &'static [u8]> =
                TableDefinition::new(reconciler);
            let mut table = write.open_table(table_def).map_err(map_table_error)?;
            table.insert(target.as_str(), cbor).map_err(map_storage_error)?;
        }
        write.commit().map_err(map_commit_error)?;
        Ok(())
    }

    async fn delete(&self, reconciler: &'static str, target: &TargetResource) -> VsResult<()> {
        // Synchronous redb call inside an async fn — see
        // `write_through_bytes` for the rationale.
        let mut write = self.db.begin_write().map_err(map_transaction_error)?;
        write.set_durability(Durability::Immediate);
        // Idempotent — a table that doesn't exist yet means the
        // row doesn't exist; succeed without creating the table.
        let table_def: TableDefinition<&'static str, &'static [u8]> =
            TableDefinition::new(reconciler);
        // See `bulk_load_bytes` for why this is inlined rather than
        // routed through a helper post-`refactor-reconciler-static-name`.
        match write.open_table(table_def) {
            Ok(mut table) => {
                let _ = table.remove(target.as_str()).map_err(map_storage_error)?;
            }
            Err(redb::TableError::TableDoesNotExist(_)) => {
                // Nothing to remove; commit a no-op txn so the
                // call shape stays uniform.
            }
            Err(e) => return Err(map_table_error(e)),
        }
        write.commit().map_err(map_commit_error)?;
        Ok(())
    }

    async fn probe(&self) -> std::result::Result<(), ProbeError> {
        // Synchronous redb calls inside an async fn — see
        // `write_through_bytes` for the rationale.

        // Step 1: write the sentinel + commit (fsync). Failure
        // surfaces as `WriteFailed`.
        {
            let mut write = self
                .db
                .begin_write()
                .map_err(|e| ProbeError::WriteFailed { source: map_transaction_error(e) })?;
            write.set_durability(Durability::Immediate);
            {
                let mut table = write
                    .open_table(PROBE_TABLE)
                    .map_err(|e| ProbeError::WriteFailed { source: map_table_error(e) })?;
                table
                    .insert(PROBE_KEY, PROBE_PAYLOAD)
                    .map_err(|e| ProbeError::WriteFailed { source: map_storage_error(e) })?;
            }
            write.commit().map_err(|e| ProbeError::CommitFailed { source: map_commit_error(e) })?;
        }

        // Step 2: read back byte-equal. Mismatch = engine
        // corruption — refuse boot.
        let got = {
            let read = self
                .db
                .begin_read()
                .map_err(|e| ProbeError::CommitFailed { source: map_transaction_error(e) })?;
            let table = read
                .open_table(PROBE_TABLE)
                .map_err(|e| ProbeError::CommitFailed { source: map_table_error(e) })?;
            let entry = table
                .get(PROBE_KEY)
                .map_err(|e| ProbeError::CommitFailed { source: map_storage_error(e) })?
                .ok_or_else(|| ProbeError::RoundTripMismatch {
                    wrote: PROBE_PAYLOAD.to_vec(),
                    got: Vec::new(),
                })?;
            entry.value().to_vec()
        };
        if got.as_slice() != PROBE_PAYLOAD {
            return Err(ProbeError::RoundTripMismatch { wrote: PROBE_PAYLOAD.to_vec(), got });
        }

        // Step 3: delete the sentinel + commit. Failure surfaces
        // as `CleanupFailed` so a store that can write/read but
        // not delete refuses boot rather than leaking sentinel
        // rows.
        {
            let mut write = self
                .db
                .begin_write()
                .map_err(|e| ProbeError::CleanupFailed { source: map_transaction_error(e) })?;
            write.set_durability(Durability::Immediate);
            {
                let mut table = write
                    .open_table(PROBE_TABLE)
                    .map_err(|e| ProbeError::CleanupFailed { source: map_table_error(e) })?;
                let _ = table
                    .remove(PROBE_KEY)
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
// Error mapping helpers — collapse redb's deep error hierarchy onto
// the single `ViewStoreError::Io` variant. ADR-0035 calls this out as
// acceptable in Phase 1; reconcilers that branch on a specific redb
// error class can split these out in a later phase.
// -----------------------------------------------------------------------------

fn map_database_error(err: redb::DatabaseError) -> ViewStoreError {
    ViewStoreError::Io(std::io::Error::other(err))
}

fn map_transaction_error(err: redb::TransactionError) -> ViewStoreError {
    ViewStoreError::Io(std::io::Error::other(err))
}

fn map_table_error(err: redb::TableError) -> ViewStoreError {
    ViewStoreError::Io(std::io::Error::other(err))
}

fn map_storage_error(err: redb::StorageError) -> ViewStoreError {
    ViewStoreError::Io(std::io::Error::other(err))
}

fn map_commit_error(err: redb::CommitError) -> ViewStoreError {
    ViewStoreError::Io(std::io::Error::other(err))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    use crate::view_store::ViewStoreExt;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
    struct Counter {
        n: u64,
    }

    /// Canonical kebab-case reconciler name as a `&'static str` literal.
    /// The `ViewStore` byte-level surface requires `&'static str` per the
    /// `refactor-reconciler-static-name` RCA; tests use the literal
    /// directly rather than constructing a `ReconcilerName` wrapper.
    const N: &str = "job-lifecycle";

    fn target(s: &str) -> TargetResource {
        TargetResource::new(s).expect("valid target resource")
    }

    #[test]
    fn resolve_path_appends_reconcilers_memory_redb() {
        let p = RedbViewStore::resolve_path(Path::new("/var/lib/overdrive"));
        assert_eq!(p, Path::new("/var/lib/overdrive/reconcilers/memory.redb"));
    }

    #[tokio::test]
    async fn open_creates_parent_dir_when_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Use a non-existent nested path under tempdir — `open` must
        // create `<tmp>/data/reconcilers/`.
        let nested = tmp.path().join("data");
        let _store = RedbViewStore::open(&nested).expect("open creates parent");
        assert!(
            nested.join("reconcilers").join("memory.redb").exists(),
            "open must materialise the redb file under reconcilers/"
        );
    }

    #[tokio::test]
    async fn bulk_load_returns_empty_when_table_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = RedbViewStore::open(tmp.path()).expect("open");
        let loaded: BTreeMap<TargetResource, Counter> =
            store.bulk_load(N).await.expect("bulk_load empty");
        assert!(loaded.is_empty(), "fresh store has no rows for any reconciler");
    }

    #[tokio::test]
    async fn write_through_creates_table_lazily() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = RedbViewStore::open(tmp.path()).expect("open");
        let t = target("job/payments");
        let v = Counter { n: 1 };

        // Before write_through, bulk_load returns empty (table absent).
        let pre: BTreeMap<TargetResource, Counter> = store.bulk_load(N).await.expect("empty");
        assert!(pre.is_empty());

        store.write_through(N, &t, &v).await.expect("write ok");

        let post: BTreeMap<TargetResource, Counter> = store.bulk_load(N).await.expect("read");
        assert_eq!(post.get(&t), Some(&v));
    }

    #[tokio::test]
    async fn bulk_load_iterates_targets_in_ord_order() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = RedbViewStore::open(tmp.path()).expect("open");

        // Insert in a deliberately non-sorted order to prove
        // BTreeMap-driven iteration.
        let order = ["job/zeta", "job/alpha", "job/middle"];
        for (idx, k) in order.iter().enumerate() {
            store.write_through(N, &target(k), &Counter { n: idx as u64 }).await.expect("write");
        }

        let loaded: BTreeMap<TargetResource, Counter> = store.bulk_load(N).await.expect("read");
        let keys: Vec<_> = loaded.keys().map(|t| t.as_str().to_string()).collect();
        // BTreeMap<TargetResource, _> iterates in TargetResource::Ord
        // order — String lexicographic given the inner type.
        assert_eq!(keys, vec!["job/alpha", "job/middle", "job/zeta"]);
    }

    #[tokio::test]
    async fn delete_removes_row_and_is_idempotent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = RedbViewStore::open(tmp.path()).expect("open");
        let t = target("job/payments");

        // Idempotent on a never-written key (table doesn't exist yet).
        store.delete(N, &t).await.expect("idempotent delete on missing table");

        store.write_through(N, &t, &Counter { n: 5 }).await.expect("write");
        store.delete(N, &t).await.expect("delete existing");

        let loaded: BTreeMap<TargetResource, Counter> = store.bulk_load(N).await.expect("read");
        assert!(!loaded.contains_key(&t));
    }

    #[tokio::test]
    async fn probe_succeeds_and_leaves_no_residual_rows() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = RedbViewStore::open(tmp.path()).expect("open");

        store.probe().await.expect("probe ok");

        // Probe table sentinel must be removed. Open it directly via
        // a read txn and assert the row is gone.
        let read = store.db.begin_read().expect("begin_read");
        let table = read.open_table(PROBE_TABLE).expect("probe table");
        let got = table.get(PROBE_KEY).expect("get").map(|g| g.value().to_vec());
        assert!(got.is_none(), "probe must remove its sentinel row, found {got:?}");
    }

    #[tokio::test]
    async fn probe_succeeds_repeatedly() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = RedbViewStore::open(tmp.path()).expect("open");
        store.probe().await.expect("probe 1");
        store.probe().await.expect("probe 2");
        store.probe().await.expect("probe 3");
    }
}
