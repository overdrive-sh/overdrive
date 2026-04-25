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
    // from the second store. The per-entry commit_index also survives
    // round-trip — payments was the first put (index 1), auth the
    // second (index 2), frontend the third (index 3).
    assert_eq!(
        store2.get(b"jobs/payments").await.expect("get payments"),
        Some((Bytes::copy_from_slice(b"spec-v1-bytes"), 1))
    );
    assert_eq!(
        store2.get(b"jobs/auth").await.expect("get auth"),
        Some((Bytes::copy_from_slice(b"spec-auth-bytes"), 2))
    );
    assert_eq!(
        store2.get(b"jobs/frontend").await.expect("get frontend"),
        Some((Bytes::copy_from_slice(b"spec-frontend-bytes"), 3))
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

    // The value is readable from the bootstrapped store at its
    // original per-entry index (1 — the only put).
    assert_eq!(
        store2.get(b"jobs/bulk").await.expect("get bulk"),
        Some((Bytes::copy_from_slice(&big_value), 1))
    );
}

// -----------------------------------------------------------------------------
// Frame v2 — round-trip carries per-entry commit_index losslessly.
//
// `fix-commit-index-per-entry` Step 01-02 contract: the v2 frame must
// preserve every entry's per-entry commit_index across
// `export → bootstrap_from → export`. A handler that consumed the
// bootstrapped store via `IntentStore::get` must read the same
// (value, commit_index) tuple it would have read on the source store.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn snapshot_v2_round_trip_preserves_per_entry_commit_indices_losslessly() {
    let tmp1 = TempDir::new().expect("temp dir 1");
    let store1 = LocalIntentStore::open(tmp1.path().join("intent.redb")).expect("open 1");

    // Three sequential puts assign commit_index 1, 2, 3 respectively.
    store1.put(b"jobs/a", b"va").await.expect("put a");
    store1.put(b"jobs/b", b"vb").await.expect("put b");
    store1.put(b"jobs/c", b"vc").await.expect("put c");

    let snap1 = store1.export_snapshot().await.expect("export 1");

    let tmp2 = TempDir::new().expect("temp dir 2");
    let store2 = LocalIntentStore::open(tmp2.path().join("intent.redb")).expect("open 2");
    store2.bootstrap_from(snap1.clone()).await.expect("bootstrap 2");

    // Per-entry indices survive bootstrap — every key reads its
    // original index, not the global counter at bootstrap time.
    assert_eq!(
        store2.get(b"jobs/a").await.expect("get a"),
        Some((Bytes::copy_from_slice(b"va"), 1)),
        "jobs/a must keep its source per-entry index (1) through bootstrap",
    );
    assert_eq!(
        store2.get(b"jobs/b").await.expect("get b"),
        Some((Bytes::copy_from_slice(b"vb"), 2)),
        "jobs/b must keep its source per-entry index (2) through bootstrap",
    );
    assert_eq!(
        store2.get(b"jobs/c").await.expect("get c"),
        Some((Bytes::copy_from_slice(b"vc"), 3)),
        "jobs/c must keep its source per-entry index (3) through bootstrap",
    );

    // Re-export from the bootstrapped store — bytes must be
    // byte-identical to the source export.
    let snap2 = store2.export_snapshot().await.expect("export 2");
    assert_eq!(
        snap1.bytes(),
        snap2.bytes(),
        "frame v2 round-trip must be byte-identical when per-entry indices match",
    );
}

// -----------------------------------------------------------------------------
// Frame v1 forward compatibility — DR backups produced before
// Step 01-02 landed (frame v1, no per-entry indices) decode cleanly,
// every entry lands at commit_index = 0, and a re-export emits v2.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn bootstrap_from_v1_frame_assigns_zero_index_and_re_exports_as_v2() {
    use overdrive_core::traits::intent_store::StateSnapshot;
    use overdrive_store_local::snapshot_frame;

    // Hand-craft a v1 frame: magic + version=1 LE + rkyv-archived
    // Vec<(Vec<u8>, Vec<u8>)>. This is what a pre-Step-01-02 export
    // would have produced.
    let mut v1_payload: Vec<(Vec<u8>, Vec<u8>)> =
        vec![(b"jobs/a".to_vec(), b"va".to_vec()), (b"jobs/b".to_vec(), b"vb".to_vec())];
    v1_payload.sort_by(|a, b| a.0.cmp(&b.0));

    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&v1_payload).expect("rkyv archive v1");

    let mut v1_bytes = Vec::with_capacity(snapshot_frame::HEADER_LEN + archived.len());
    v1_bytes.extend_from_slice(&snapshot_frame::MAGIC);
    v1_bytes.extend_from_slice(&snapshot_frame::VERSION_V1.to_le_bytes());
    v1_bytes.extend_from_slice(&archived);

    let v1_snapshot = StateSnapshot::from_parts(
        u32::from(snapshot_frame::VERSION_V1),
        v1_payload
            .iter()
            .map(|(k, v)| (Bytes::copy_from_slice(k), Bytes::copy_from_slice(v)))
            .collect(),
        v1_bytes,
    );

    // Bootstrap a fresh target from the v1 frame — must succeed.
    let tmp = TempDir::new().expect("temp dir target");
    let target = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open target");
    target.bootstrap_from(v1_snapshot).await.expect("bootstrap from v1 frame must succeed");

    // Every entry lands at commit_index = 0 — the v1-projection
    // contract documented in `snapshot_frame.rs`.
    assert_eq!(
        target.get(b"jobs/a").await.expect("get a"),
        Some((Bytes::copy_from_slice(b"va"), 0)),
        "v1-frame entries must read as commit_index = 0",
    );
    assert_eq!(
        target.get(b"jobs/b").await.expect("get b"),
        Some((Bytes::copy_from_slice(b"vb"), 0)),
        "v1-frame entries must read as commit_index = 0",
    );

    // The store now writes v2 — the next put assigns a fresh index
    // (>= 1; the v1-projection floors the counter at 0).
    target.put(b"jobs/c", b"vc").await.expect("post-bootstrap put");
    let (_, new_idx) =
        target.get(b"jobs/c").await.expect("get c").expect("jobs/c present after put");
    assert!(
        new_idx >= 1,
        "post-v1-bootstrap puts must assign a fresh commit_index >= 1; got {new_idx}",
    );

    // Re-exporting produces a v2 frame — assert via the version field.
    let snap_re = target.export_snapshot().await.expect("re-export");
    assert_eq!(
        snap_re.version,
        u32::from(snapshot_frame::VERSION),
        "re-export after v1 bootstrap must emit v2",
    );
}

// -----------------------------------------------------------------------------
// Determinism — two independent stores populated with the same entries
// (with the same per-entry indices) produce byte-identical exports
// regardless of the order rows are inserted into the underlying redb.
// This pins down the "sort entries by key before archival" guarantee
// that KPI K6 rides on, while honouring the per-entry commit_index
// contract from `fix-commit-index-per-entry`.
//
// To exercise distinct insertion order while keeping per-entry indices
// equal, both stores write all three entries in a single `txn` — the
// store contract assigns one shared per-entry index per committed
// transaction. The redb iteration order is then the only remaining
// non-determinism, and the snapshot frame's "sort by key before
// archival" step normalises that out.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn snapshot_bytes_are_deterministic_regardless_of_insertion_order() {
    use overdrive_core::traits::intent_store::TxnOp;

    let tmp_a = TempDir::new().expect("temp dir a");
    let store_a = LocalIntentStore::open(tmp_a.path().join("intent.redb")).expect("open a");
    // Insert in order A, B, C — single txn so all three share one
    // per-entry index.
    store_a
        .txn(vec![
            TxnOp::Put { key: Bytes::from_static(b"jobs/a"), value: Bytes::from_static(b"va") },
            TxnOp::Put { key: Bytes::from_static(b"jobs/b"), value: Bytes::from_static(b"vb") },
            TxnOp::Put { key: Bytes::from_static(b"jobs/c"), value: Bytes::from_static(b"vc") },
        ])
        .await
        .expect("txn a");

    let tmp_b = TempDir::new().expect("temp dir b");
    let store_b = LocalIntentStore::open(tmp_b.path().join("intent.redb")).expect("open b");
    // Insert in reverse order C, B, A — single txn so all three share
    // one per-entry index (same as store_a's, since both stores took
    // the first txn after open).
    store_b
        .txn(vec![
            TxnOp::Put { key: Bytes::from_static(b"jobs/c"), value: Bytes::from_static(b"vc") },
            TxnOp::Put { key: Bytes::from_static(b"jobs/b"), value: Bytes::from_static(b"vb") },
            TxnOp::Put { key: Bytes::from_static(b"jobs/a"), value: Bytes::from_static(b"va") },
        ])
        .await
        .expect("txn b");

    let snap_a = store_a.export_snapshot().await.expect("export a");
    let snap_b = store_b.export_snapshot().await.expect("export b");
    assert_eq!(
        snap_a.bytes(),
        snap_b.bytes(),
        "snapshot bytes must be independent of insertion order when \
         per-entry indices match (both stores wrote in one txn)",
    );
}
