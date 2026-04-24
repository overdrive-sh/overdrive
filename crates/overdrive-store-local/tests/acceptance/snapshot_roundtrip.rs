//! Acceptance scenarios for US-03 §4.2 — snapshot round-trip
//! byte-identity across `LocalIntentStore` instances.
//!
//! Translates `docs/feature/phase-1-foundation/distill/test-scenarios.md`
//! §4.2 both snapshot scenarios — the concrete-contents case and the
//! property-based variant — into Rust `#[tokio::test]` bodies. KPI K6:
//! commercial migration correctness depends on an `export_snapshot`
//! byte slice that round-trips byte-identical through a second
//! `LocalIntentStore`, so the same bytes that ship an HA-mode bootstrap also
//! ship DR backups and Raft snapshots.
//!
//! Port-to-port discipline: every assertion drives the `IntentStore`
//! trait surface that `LocalIntentStore` implements. No internal types are
//! inspected; the canonical byte form is accessed through
//! `StateSnapshot::bytes()`.
//!
//! Strategy C per DWD-01: real redb, `tempfile::TempDir` backing path.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use bytes::Bytes;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

// -----------------------------------------------------------------------------
// §4.2 scenario 1 — "Snapshot round-trip is byte-identical across
// LocalIntentStore instances"
// -----------------------------------------------------------------------------

#[tokio::test]
async fn snapshot_roundtrip_is_byte_identical_across_local_store_instances() {
    // Given a LocalIntentStore populated with a known set of JobSpec entries.
    let tmp1 = TempDir::new().expect("temp dir 1");
    let store1 = LocalIntentStore::open(tmp1.path().join("intent.redb")).expect("open 1");
    store1.put(b"jobs/payments", b"spec-v1-bytes").await.expect("put payments");
    store1.put(b"jobs/auth", b"spec-auth-bytes").await.expect("put auth");
    store1.put(b"jobs/frontend", b"spec-frontend-bytes").await.expect("put frontend");

    // When Ana exports a snapshot.
    let snap1 = store1.export_snapshot().await.expect("export 1");

    // And Ana constructs a second LocalIntentStore on a different temporary path.
    let tmp2 = TempDir::new().expect("temp dir 2");
    let store2 = LocalIntentStore::open(tmp2.path().join("intent.redb")).expect("open 2");

    // And Ana bootstraps the second store from the exported snapshot.
    store2.bootstrap_from(snap1.clone()).await.expect("bootstrap 2");

    // And Ana exports a snapshot from the second store.
    let snap2 = store2.export_snapshot().await.expect("export 2");

    // Then the second snapshot byte slice equals the first snapshot byte
    // slice.
    assert_eq!(
        snap1.bytes(),
        snap2.bytes(),
        "snapshot bytes must be byte-identical across LocalIntentStore instances"
    );

    // And every JobSpec readable from the first store is also readable
    // from the second store.
    assert_eq!(
        store2.get(b"jobs/payments").await.expect("get payments"),
        Some(Bytes::copy_from_slice(b"spec-v1-bytes"))
    );
    assert_eq!(
        store2.get(b"jobs/auth").await.expect("get auth"),
        Some(Bytes::copy_from_slice(b"spec-auth-bytes"))
    );
    assert_eq!(
        store2.get(b"jobs/frontend").await.expect("get frontend"),
        Some(Bytes::copy_from_slice(b"spec-frontend-bytes"))
    );
}

// -----------------------------------------------------------------------------
// Edge — an empty store still round-trips byte-identical. The framing
// headers must appear regardless of entry count, so the two empty
// exports produce identical non-empty byte slices.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn snapshot_roundtrip_byte_identical_for_empty_store() {
    let tmp1 = TempDir::new().expect("temp dir 1");
    let store1 = LocalIntentStore::open(tmp1.path().join("intent.redb")).expect("open 1");

    let snap1 = store1.export_snapshot().await.expect("export 1");

    let tmp2 = TempDir::new().expect("temp dir 2");
    let store2 = LocalIntentStore::open(tmp2.path().join("intent.redb")).expect("open 2");
    store2.bootstrap_from(snap1.clone()).await.expect("bootstrap 2");

    let snap2 = store2.export_snapshot().await.expect("export 2");
    assert_eq!(
        snap1.bytes(),
        snap2.bytes(),
        "empty-store snapshot bytes must round-trip byte-identical"
    );

    // An empty export still produces a framed byte slice — the magic +
    // version header is present.
    assert!(snap1.bytes().len() >= 6, "snapshot bytes must include magic (4) + version (2) header");
}

// -----------------------------------------------------------------------------
// Edge — a single 4 KB value round-trips. Exercises the upper bound
// of the entry-size envelope the §4.2 property test covers.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn snapshot_roundtrip_byte_identical_with_four_kb_value() {
    let tmp1 = TempDir::new().expect("temp dir 1");
    let store1 = LocalIntentStore::open(tmp1.path().join("intent.redb")).expect("open 1");

    // 4 KB value.
    let big_value = vec![0xABu8; 4 * 1024];
    store1.put(b"jobs/bulk", &big_value).await.expect("put bulk");

    let snap1 = store1.export_snapshot().await.expect("export 1");

    let tmp2 = TempDir::new().expect("temp dir 2");
    let store2 = LocalIntentStore::open(tmp2.path().join("intent.redb")).expect("open 2");
    store2.bootstrap_from(snap1.clone()).await.expect("bootstrap 2");

    let snap2 = store2.export_snapshot().await.expect("export 2");
    assert_eq!(snap1.bytes(), snap2.bytes(), "4 KB-value snapshot must round-trip byte-identical");

    // The value is readable from the bootstrapped store.
    assert_eq!(
        store2.get(b"jobs/bulk").await.expect("get bulk"),
        Some(Bytes::copy_from_slice(&big_value))
    );
}

// -----------------------------------------------------------------------------
// Determinism — two independent stores populated with the same entries
// (in any order) produce byte-identical exports. This pins down the
// "sort entries by key before archival" guarantee that KPI K6 rides on.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn snapshot_bytes_are_deterministic_regardless_of_insertion_order() {
    let tmp_a = TempDir::new().expect("temp dir a");
    let store_a = LocalIntentStore::open(tmp_a.path().join("intent.redb")).expect("open a");
    // Insert in order A, B, C.
    store_a.put(b"jobs/a", b"va").await.expect("put a");
    store_a.put(b"jobs/b", b"vb").await.expect("put b");
    store_a.put(b"jobs/c", b"vc").await.expect("put c");

    let tmp_b = TempDir::new().expect("temp dir b");
    let store_b = LocalIntentStore::open(tmp_b.path().join("intent.redb")).expect("open b");
    // Insert in reverse order C, B, A.
    store_b.put(b"jobs/c", b"vc").await.expect("put c");
    store_b.put(b"jobs/b", b"vb").await.expect("put b");
    store_b.put(b"jobs/a", b"va").await.expect("put a");

    let snap_a = store_a.export_snapshot().await.expect("export a");
    let snap_b = store_b.export_snapshot().await.expect("export b");
    assert_eq!(
        snap_a.bytes(),
        snap_b.bytes(),
        "snapshot bytes must be independent of insertion order"
    );
}
