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
/// "unknown future variant" branch of `Job::from_store_bytes`.
///
/// Per ADR-0048 § 1 the rkyv enum tag is at offset 0 of the archived
/// envelope bytes. This helper takes a valid `JobEnvelope::V1`
/// payload's archived bytes (passed in via `payload_archive`) and
/// flips the leading discriminant to 99. The resulting bytes fail
/// the rkyv bytecheck validator and surface as
/// `EnvelopeError::Malformed` per the implementation reality
/// documented in `JobV1::from_store_bytes`. (The variant
/// `EnvelopeError::UnknownVersion` is reserved for future
/// dynamic-variant designs per `crates/overdrive-core/src/codec/
/// envelope.rs::EnvelopeError::UnknownVersion`.)
///
/// The leading bytes of a real rkyv archive of a single-variant
/// enum are not literally `[0u8]` — rkyv uses a `#[repr(u8)]` tag
/// followed by per-variant inline body. Synthesis emits a fresh
/// byte buffer where the *last* byte is set to 99 (rkyv stores the
/// enum discriminator at the END of the archived envelope buffer,
/// after relative pointers — see `crates/overdrive-core/tests/
/// schema_evolution/alloc_status_row.rs::FIXTURE_V1` whose final
/// non-zero byte is the tag).
pub fn synthesise_unknown_job_envelope_variant_tag(payload_archive: &[u8]) -> Vec<u8> {
    // rkyv with `#[derive(Archive)]` on a `#[repr(u8)]`-implicit enum
    // serialises the discriminant byte at a position determined by
    // the archived layout — for `JobEnvelope` with one variant
    // `V1(JobV1)` the discriminant is the byte immediately preceding
    // the trailing zero-padding emitted to align the archive to a
    // multiple of `align_of::<Archived<Self>>()`. Inspecting the
    // canonical V1 fixture for `JobEnvelope` (see
    // `crates/overdrive-core/tests/schema_evolution/job.rs::
    // FIXTURE_V1`) the literal `01` discriminant sits 7 bytes from
    // the end. Flipping every byte across the trailing 8-byte
    // padding-and-discriminant region to `99` reliably corrupts the
    // tag regardless of micro-changes to the archive layout.
    let mut bytes = payload_archive.to_vec();
    let n = bytes.len();
    if n >= 8 {
        for slot in &mut bytes[n - 8..] {
            *slot = 99;
        }
    } else {
        for slot in &mut bytes[..] {
            *slot = 99;
        }
    }
    bytes
}
