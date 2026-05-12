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

/// Mirror of the production constant in
/// `crates/overdrive-store-local/src/redb_backend.rs`.
const ENTRIES_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("entries");

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

/// Write `raw_bytes` directly into the `entries` table (the intent
/// store's redb table) at the given `key`. Used by the intent
/// refuse-to-start integration test (S-EV-03.1, S-EV-03.2) to inject
/// bytes that would never be reachable through the typed
/// `IntentStore::put` surface — garbage that `Job::from_store_bytes`
/// must reject and surface as `IntentStoreError::Envelope`.
///
/// The redb file is opened in its own short-lived `Database`; the
/// caller's `LocalIntentStore` MUST be dropped or not yet opened when
/// this helper runs, since redb's single-writer lock would otherwise
/// contend. The recovery walk in `LocalIntentStore::open` then sees
/// the bytes when the test re-opens the store.
pub fn write_raw_bytes_to_entries_table(redb_path: &Path, key: &[u8], raw_bytes: &[u8]) {
    let db = Database::create(redb_path).expect("open redb back-door");
    let write = db.begin_write().expect("begin_write back-door");
    {
        let mut table = write.open_table(ENTRIES_TABLE).expect("open entries table");
        table.insert(key, raw_bytes).expect("insert raw bytes");
    }
    write.commit().expect("commit raw-bytes write");
}

/// Synthesise bytes that look like a `JobEnvelope` archive but carry
/// an unknown discriminant tag (`99`). Used by S-EV-03.2 to drive the
/// "unknown future variant" branch of `Job::from_store_bytes` — the
/// pre-decode probe (`probe_known_variant`) surfaces this as
/// `EnvelopeError::UnknownVersion { observed: 99, type_name:
/// "JobEnvelope", supported_max: 0 }`.
///
/// rkyv 0.8 places variable-length payload data (strings, vecs) in
/// a leading slab and the fixed-size "root" structure — including
/// the outer enum's discriminant byte — at the END of the archive
/// buffer. The distance from the buffer's end to the outer
/// discriminant is therefore stable across all archives of the same
/// envelope shape; the absolute offset from the start shifts with
/// every workload-id / command / args length change.
///
/// For `JobEnvelope` V1-only archives this distance is 64 bytes,
/// mirroring
/// `JobEnvelope::discriminant_offset_from_end() == Some(64)` in
/// `crates/overdrive-core/src/aggregate/mod.rs`.
///
/// The helper flips only that one byte (no surrounding padding) so
/// the resulting bytes:
/// * pass the pre-decode probe's structural sanity (the slice IS
///   long enough),
/// * fail the probe's known-discriminant check (99 is not in
///   `JobEnvelope::known_discriminants()`),
/// * therefore surface as `UnknownVersion` BEFORE rkyv decode.
///
/// **Version-bump invariant.** When `JobEnvelope::V2` lands, the
/// constant `JOB_ENVELOPE_DISCRIMINANT_OFFSET_FROM_END` below MUST
/// be re-pinned alongside the schema-evolution fixture per
/// `VersionedEnvelope::discriminant_offset_from_end`'s docstring —
/// if V2's archived footprint differs from V1's, rkyv shifts the
/// distance and this helper would silently target padding instead
/// of the tag, breaking the test.
pub fn synthesise_unknown_job_envelope_variant_tag(payload_archive: &[u8]) -> Vec<u8> {
    /// Empirically-pinned discriminant distance-from-end for
    /// `JobEnvelope` V1-only archives. Mirrors
    /// `JobEnvelope::discriminant_offset_from_end() == Some(64)`.
    const JOB_ENVELOPE_DISCRIMINANT_OFFSET_FROM_END: usize = 64;

    let mut bytes = payload_archive.to_vec();
    let n = bytes.len();
    assert!(
        n >= JOB_ENVELOPE_DISCRIMINANT_OFFSET_FROM_END,
        "test synthesis precondition: archive must be at least the discriminant offset long ({n} >= {JOB_ENVELOPE_DISCRIMINANT_OFFSET_FROM_END})",
    );
    let target = n - JOB_ENVELOPE_DISCRIMINANT_OFFSET_FROM_END;
    bytes[target] = 99;
    bytes
}
