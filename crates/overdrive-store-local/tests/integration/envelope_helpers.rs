//! Shared helpers for `envelope_walking_skeleton` and
//! `envelope_observation_skip` integration tests.
//!
//! Per ADR-0048 § 3 (observation log + skip self-heal), the read path
//! decodes through `AllocStatusRowEnvelope::into_latest()`. To exercise
//! the malformed-row branch we need to write raw garbage bytes into the
//! redb table — bypassing the typed `ObservationStore::write` path,
//! which would refuse to construct a malformed envelope in the first
//! place. The back-door is `#[cfg(feature = "integration-tests")]` only.

use std::path::Path;

use overdrive_core::id::AllocationId;
use redb::{Database, TableDefinition};

/// Mirror of the production constant in
/// `crates/overdrive-store-local/src/observation_backend.rs`.
const ALLOC_STATUS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("observation_alloc_status");

/// Write `raw_bytes` directly into the `observation_alloc_status`
/// table at the canonical key for `alloc_id`. Used by the
/// envelope-skip integration test to inject bytes that would never be
/// reachable through the typed write surface (garbage that the
/// envelope decoder must reject).
///
/// The redb file is opened in its own short-lived `Database`; the
/// caller's `LocalObservationStore` MUST be dropped or not yet opened
/// when this helper runs, since redb's single-writer lock would
/// otherwise contend.
pub fn write_raw_bytes_to_alloc_status_table(
    redb_path: &Path,
    alloc_id: &AllocationId,
    raw_bytes: &[u8],
) {
    let db = Database::create(redb_path).expect("open redb back-door");
    let write = db.begin_write().expect("begin_write back-door");
    {
        let mut table = write.open_table(ALLOC_STATUS_TABLE).expect("open alloc_status table");
        let key = alloc_id.as_str().as_bytes();
        table.insert(key, raw_bytes).expect("insert raw bytes");
    }
    write.commit().expect("commit raw-bytes write");
}
