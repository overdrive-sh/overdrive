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

use overdrive_core::dataplane::fingerprint::BackendSetFingerprint;
use overdrive_core::id::{AllocationId, NodeId, ServiceId};
use redb::{Database, TableDefinition};

/// Mirror of the production constant in
/// `crates/overdrive-store-local/src/observation_backend.rs`.
const ALLOC_STATUS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("observation_alloc_status");

/// Mirror of the production constant in
/// `crates/overdrive-store-local/src/observation_backend.rs`.
const NODE_HEALTH_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("observation_node_health");

/// Mirror of the production constant in
/// `crates/overdrive-store-local/src/observation_backend.rs`.
const SERVICE_HYDRATION_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("observation_service_hydration_results");

/// Mirror of the production constant in
/// `crates/overdrive-store-local/src/observation_backend.rs`.
const SERVICE_BACKENDS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("observation_service_backends");

/// Mirror of the production constant in
/// `crates/overdrive-store-local/src/redb_backend.rs`.
const ENTRIES_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("entries");

/// Write `raw_bytes` directly into the given redb `table_def` at the
/// given `key`. Shared back-door for the envelope-skip integration
/// tests across every observation + intent surface — each per-surface
/// helper below derives its own `(table_def, key)` pair and delegates
/// here for the redb plumbing.
///
/// The redb file is opened in its own short-lived `Database`; the
/// caller's `LocalIntentStore` / `LocalObservationStore` MUST be
/// dropped or not yet opened when this helper runs, since redb's
/// single-writer lock would otherwise contend.
fn write_raw_bytes_to_table(
    redb_path: &Path,
    table_def: TableDefinition<'_, &[u8], &[u8]>,
    key: &[u8],
    raw_bytes: &[u8],
) {
    let db = Database::create(redb_path).expect("open redb back-door");
    let write = db.begin_write().expect("begin_write back-door");
    {
        let mut table = write.open_table(table_def).expect("open back-door table");
        table.insert(key, raw_bytes).expect("insert raw bytes");
    }
    write.commit().expect("commit raw-bytes write");
}

/// Write `raw_bytes` directly into the `observation_alloc_status`
/// table at the canonical key for `alloc_id`. Used by the
/// envelope-skip integration test to inject bytes that would never be
/// reachable through the typed write surface (garbage that the
/// envelope decoder must reject).
pub fn write_raw_bytes_to_alloc_status_table(
    redb_path: &Path,
    alloc_id: &AllocationId,
    raw_bytes: &[u8],
) {
    write_raw_bytes_to_table(
        redb_path,
        ALLOC_STATUS_TABLE,
        alloc_id.as_str().as_bytes(),
        raw_bytes,
    );
}

/// Write `raw_bytes` directly into the `observation_node_health` table
/// at the canonical key for `node_id`. Mirrors
/// [`write_raw_bytes_to_alloc_status_table`] but for the node-health
/// surface — used by the envelope-skip integration test (S-EV-04.3)
/// to inject bytes that the typed write path would refuse to
/// construct.
pub fn write_raw_bytes_to_node_health_table(redb_path: &Path, node_id: &NodeId, raw_bytes: &[u8]) {
    write_raw_bytes_to_table(redb_path, NODE_HEALTH_TABLE, node_id.as_str().as_bytes(), raw_bytes);
}

/// Write `raw_bytes` directly into the
/// `observation_service_hydration_results` table at the canonical key
/// for `(service_id, fingerprint)`. Mirrors
/// [`write_raw_bytes_to_node_health_table`] but for the
/// service-hydration surface — used by the envelope-skip integration
/// test (S-EV-04.4) to inject bytes that the typed write path would
/// refuse to construct.
///
/// The 16-byte composite key layout
/// (`service_id` LE u64 || `fingerprint` LE u64) mirrors the
/// production `encode_service_hydration_key` in
/// `crates/overdrive-store-local/src/observation_backend.rs` — keeping
/// the layout in two places is the price of the bytes-injection
/// back-door; the structural pin is the production constant.
pub fn write_raw_bytes_to_service_hydration_results_table(
    redb_path: &Path,
    service_id: ServiceId,
    fingerprint: BackendSetFingerprint,
    raw_bytes: &[u8],
) {
    let sid = service_id.get().to_le_bytes();
    let fp = fingerprint.to_le_bytes();
    let key: [u8; 16] = [
        sid[0], sid[1], sid[2], sid[3], sid[4], sid[5], sid[6], sid[7], fp[0], fp[1], fp[2], fp[3],
        fp[4], fp[5], fp[6], fp[7],
    ];
    write_raw_bytes_to_table(redb_path, SERVICE_HYDRATION_TABLE, key.as_slice(), raw_bytes);
}

/// Write `raw_bytes` directly into the
/// `observation_service_backends` table at the canonical key for
/// `service_id`. Mirrors
/// [`write_raw_bytes_to_service_hydration_results_table`] but for the
/// service-backends surface — used by the envelope-skip integration
/// test (S-EV-04.5) to inject bytes that the typed write path would
/// refuse to construct.
///
/// The 8-byte key layout (`service_id` LE u64) mirrors the production
/// `encode_service_backends_key` in
/// `crates/overdrive-store-local/src/observation_backend.rs` — keeping
/// the layout in two places is the price of the bytes-injection
/// back-door; the structural pin is the production constant.
pub fn write_raw_bytes_to_service_backends_table(
    redb_path: &Path,
    service_id: ServiceId,
    raw_bytes: &[u8],
) {
    let key = service_id.get().to_le_bytes();
    write_raw_bytes_to_table(redb_path, SERVICE_BACKENDS_TABLE, key.as_slice(), raw_bytes);
}

/// Write `raw_bytes` directly into the `entries` table (the intent
/// store's redb table) at the given `key`. Used by the intent
/// refuse-to-start integration test (S-EV-03.1, S-EV-03.2) to inject
/// bytes that would never be reachable through the typed
/// `IntentStore::put` surface — garbage that `Job::from_store_bytes`
/// must reject and surface as `IntentStoreError::Envelope`.
///
/// The recovery walk in `LocalIntentStore::open` sees the bytes when
/// the test re-opens the store.
pub fn write_raw_bytes_to_entries_table(redb_path: &Path, key: &[u8], raw_bytes: &[u8]) {
    write_raw_bytes_to_table(redb_path, ENTRIES_TABLE, key, raw_bytes);
}

/// Synthesise bytes that look like a `WorkloadIntentEnvelope` archive but carry
/// an unknown discriminant tag (`99`). Used by S-EV-03.2 to drive the
/// "unknown future variant" branch of `Job::from_store_bytes` — the
/// pre-decode probe (`probe_known_variant`) surfaces this as
/// `EnvelopeError::UnknownVersion { observed: 99, type_name:
/// "WorkloadIntentEnvelope", supported_max: 0 }`.
///
/// rkyv 0.8 places variable-length payload data (strings, vecs) in
/// a leading slab and the fixed-size "root" structure — including
/// the outer enum's discriminant byte — at the END of the archive
/// buffer. The distance from the buffer's end to the outer
/// discriminant is therefore stable across all archives of the same
/// envelope shape; the absolute offset from the start shifts with
/// every workload-id / command / args length change.
///
/// For `WorkloadIntentEnvelope` V1-only archives this distance is 64 bytes,
/// mirroring
/// `WorkloadIntentEnvelope::discriminant_offset_from_end() == Some(64)` in
/// `crates/overdrive-core/src/aggregate/mod.rs`.
///
/// The helper flips only that one byte (no surrounding padding) so
/// the resulting bytes:
/// * pass the pre-decode probe's structural sanity (the slice IS
///   long enough),
/// * fail the probe's known-discriminant check (99 is not in
///   `WorkloadIntentEnvelope::known_discriminants()`),
/// * therefore surface as `UnknownVersion` BEFORE rkyv decode.
///
/// **Version-bump invariant.** When `WorkloadIntentEnvelope::V2` lands, the
/// constant `JOB_ENVELOPE_DISCRIMINANT_OFFSET_FROM_END` below MUST
/// be re-pinned alongside the schema-evolution fixture per
/// `VersionedEnvelope::discriminant_offset_from_end`'s docstring —
/// if V2's archived footprint differs from V1's, rkyv shifts the
/// distance and this helper would silently target padding instead
/// of the tag, breaking the test.
pub fn synthesise_unknown_job_envelope_variant_tag(payload_archive: &[u8]) -> Vec<u8> {
    /// Empirically-pinned discriminant distance-from-end for
    /// `WorkloadIntentEnvelope` V1-only archives. Mirrors
    /// `WorkloadIntentEnvelope::discriminant_offset_from_end() == Some(64)`.
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
