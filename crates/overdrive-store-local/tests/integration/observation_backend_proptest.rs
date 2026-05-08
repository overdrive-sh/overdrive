//! Proptest harness for `LocalObservationStore`'s `service-hydration` /
//! `service-backends` encoding helpers and LWW-guarded inserts.
//!
//! Closes the 15 missed mutations the `QUALITY_GATE` wave's mutation run
//! flagged in `crates/overdrive-store-local/src/observation_backend.rs`:
//!
//! - `encode_service_hydration_key` body replacement (`[0; 16]` / `[1; 16]`)
//! - `encode_service_hydration_prefix` body replacement (`[0; 8]` / `[1; 8]`)
//! - `encode_service_backends_key` body replacement (`[0; 8]` / `[1; 8]`)
//! - `service_hydration_results_rows` body replacement (`Ok(vec![])`) and
//!   filter-logic flips (`||` -> `&&`, `!=` -> `==` ×2 sites at line 336)
//! - `service_backends_rows` body replacement (`Ok(vec![])`)
//! - `apply_service_backends_lww` body replacement (`Ok(true)` / `Ok(false)`)
//! - `apply_service_hydration_lww` body replacement (`Ok(true)` / `Ok(false)`)
//!
//! The encoding helpers (`encode_*`) are module-private; we exercise them
//! INDIRECTLY through `write` + `service_*_rows`. A `[0; N]` /
//! `[1; N]` body would collapse every distinct service onto the same key
//! bytes — write-then-read for distinct services would return mixed rows.
//!
//! The LWW-guarded inserts (`apply_*_lww`) are also module-private; we
//! exercise them through the `ObservationStore::write` driving port. The
//! `Ok(true)` mutation makes every write accepted (so an older-timestamp
//! write would clobber a newer one); the `Ok(false)` mutation makes every
//! write rejected (so a fresh row never lands). Read-after-write
//! discriminates both.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]
// row_a / row_b / row_v1 / row_v2 are intentionally similar — they are
// counterparts in the LWW-dominance and prefix-collision tests, and the
// suffix carries the discriminating information.
#![allow(clippy::similar_names)]
// Some helpers don't await but the tokio tests share a common async
// signature for store construction; pruning the `async` would force
// callers to thread through the runtime separately.
#![allow(clippy::unused_async)]

use std::net::Ipv4Addr;

use overdrive_core::dataplane::fingerprint::BackendSetFingerprint;
use overdrive_core::id::{NodeId, ServiceId, SpiffeId};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationRow, ObservationStore, ServiceBackendRow,
    ServiceHydrationResultRow, ServiceHydrationStatus,
};
use overdrive_store_local::LocalObservationStore;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Row helpers
// ---------------------------------------------------------------------------

fn make_writer() -> NodeId {
    NodeId::new("control-plane-0").expect("valid node id")
}

fn make_lts(counter: u64) -> LogicalTimestamp {
    LogicalTimestamp { counter, writer: make_writer() }
}

fn make_hydration_row(
    sid: u64,
    fp: BackendSetFingerprint,
    counter: u64,
) -> ServiceHydrationResultRow {
    ServiceHydrationResultRow {
        service_id: ServiceId::new(sid).expect("valid service id"),
        fingerprint: fp,
        status: ServiceHydrationStatus::Pending,
        updated_at: make_lts(counter),
    }
}

fn make_backend(addr_octet: u8) -> Backend {
    Backend {
        alloc: SpiffeId::new(&format!("spiffe://test/x/{addr_octet}")).expect("valid spiffe id"),
        addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(
            Ipv4Addr::new(10, 0, 0, addr_octet),
            8080,
        )),
        weight: 1,
        healthy: true,
    }
}

fn make_backends_row(sid: u64, vip_octet: u8, counter: u64) -> ServiceBackendRow {
    ServiceBackendRow {
        service_id: ServiceId::new(sid).expect("valid service id"),
        vip: Ipv4Addr::new(10, 1, 0, vip_octet),
        backends: vec![make_backend(vip_octet), make_backend(vip_octet.wrapping_add(1))],
        updated_at: make_lts(counter),
    }
}

async fn fresh_store() -> (TempDir, LocalObservationStore) {
    let tmp = TempDir::new().expect("tempdir");
    let store = LocalObservationStore::open(tmp.path().join("observation"))
        .expect("open observation store");
    (tmp, store)
}

// ---------------------------------------------------------------------------
// Smoke test — single write + single read MUST return the row.
// If this fails, every other test below fails too — regression isolation.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn single_hydration_write_then_read_returns_row() {
    let (_tmp, store) = fresh_store().await;
    let row = make_hydration_row(1, 0x1234, 100);
    store.write(ObservationRow::ServiceHydration(row.clone())).await.expect("write");
    let rows = store.service_hydration_results_rows(&row.service_id).await.expect("read");
    assert_eq!(rows, vec![row]);
}

#[tokio::test]
async fn single_backends_write_then_read_returns_row() {
    let (_tmp, store) = fresh_store().await;
    let row = make_backends_row(1, 1, 100);
    store.write(ObservationRow::ServiceBackend(row.clone())).await.expect("write");
    let rows = store.service_backends_rows(&row.service_id).await.expect("read");
    assert_eq!(rows, vec![row]);
}

// ---------------------------------------------------------------------------
// encode_service_hydration_key — distinct (service_id, fingerprint) pairs
// produce distinct keys, so write-then-read returns the matching row.
// ---------------------------------------------------------------------------
//
// If `encode_service_hydration_key` were `[0; 16]` (constant), every
// distinct `(sid, fp)` would write to the same key — the second write
// (LWW dominate) would overwrite the first, and reading service A would
// return service B's row. The assertion below catches this.

#[tokio::test]
async fn encode_service_hydration_key_is_collision_free_for_distinct_inputs() {
    let (_tmp, store) = fresh_store().await;

    // Two distinct (service_id, fingerprint) pairs. Different sid AND
    // different fp ensures both halves of the 16-byte key contribute.
    let row_a = make_hydration_row(0xaa_aa_aa_aa, 0x1111_1111, 100);
    let row_b = make_hydration_row(0xbb_bb_bb_bb, 0x2222_2222, 100);

    store.write(ObservationRow::ServiceHydration(row_a.clone())).await.expect("write row a");
    store.write(ObservationRow::ServiceHydration(row_b.clone())).await.expect("write row b");

    // Service A's prefix scan must return exactly row A.
    let rows_a = store
        .service_hydration_results_rows(&row_a.service_id)
        .await
        .expect("read service a hydration rows");
    assert_eq!(rows_a, vec![row_a.clone()], "service A's prefix scan must return exactly row A");

    // Service B's prefix scan must return exactly row B.
    let rows_b = store
        .service_hydration_results_rows(&row_b.service_id)
        .await
        .expect("read service b hydration rows");
    assert_eq!(rows_b, vec![row_b.clone()], "service B's prefix scan must return exactly row B");
}

#[tokio::test]
async fn distinct_fingerprints_for_same_service_id_produce_distinct_rows() {
    // If `encode_service_hydration_key` were `[0; 16]`, two distinct
    // fingerprints for the same service_id would collide — only one
    // row would survive. With the real encoding, both rows persist
    // and the prefix scan returns both.
    let (_tmp, store) = fresh_store().await;

    let sid = 0xdead_beef;
    let row_fp1 = make_hydration_row(sid, 0x1111_1111, 100);
    let row_fp2 = make_hydration_row(sid, 0x2222_2222, 100);

    store.write(ObservationRow::ServiceHydration(row_fp1.clone())).await.expect("write fp1");
    store.write(ObservationRow::ServiceHydration(row_fp2.clone())).await.expect("write fp2");

    let rows = store.service_hydration_results_rows(&row_fp1.service_id).await.expect("read rows");

    assert_eq!(rows.len(), 2, "both distinct-fingerprint rows must persist");
    assert!(rows.contains(&row_fp1), "row with fp1 must be present");
    assert!(rows.contains(&row_fp2), "row with fp2 must be present");
}

// ---------------------------------------------------------------------------
// encode_service_hydration_prefix — prefix-scan correctness
// ---------------------------------------------------------------------------
//
// Catches: line 336 filter `||` -> `&&` (would skip rows that DO match the
// prefix); `!=` -> `==` (twice — would invert which rows are rejected).
// Catches: line 322 body `Ok(vec![])` (would return empty regardless of
// what's in the table).

#[tokio::test]
async fn service_hydration_prefix_scan_filters_other_services() {
    // Write rows for two services. Querying service A must return ONLY
    // service A's rows — service B's rows must be filtered out by the
    // prefix check.
    let (_tmp, store) = fresh_store().await;

    let row_a1 = make_hydration_row(1, 0x1111, 100);
    let row_a2 = make_hydration_row(1, 0x2222, 100);
    let row_b = make_hydration_row(2, 0x3333, 100);

    for r in [&row_a1, &row_a2, &row_b] {
        store.write(ObservationRow::ServiceHydration(r.clone())).await.expect("write");
    }

    let rows_a =
        store.service_hydration_results_rows(&row_a1.service_id).await.expect("read service a");

    assert_eq!(rows_a.len(), 2, "service A has exactly 2 rows");
    assert!(rows_a.contains(&row_a1));
    assert!(rows_a.contains(&row_a2));
    assert!(!rows_a.contains(&row_b), "service B's row must NOT appear");
}

#[tokio::test]
async fn service_hydration_rows_returns_empty_for_unknown_service() {
    // Catches `Ok(vec![])` body mutation: if the read body is hardcoded
    // empty, this test passes. Pair with the previous test (which DOES
    // expect non-empty) — together they discriminate the mutation
    // (the `Ok(vec![])` mutation would make the previous test fail
    // because rows_a would also be empty).
    let (_tmp, store) = fresh_store().await;

    let unknown = ServiceId::new(999).expect("valid sid");
    let rows = store.service_hydration_results_rows(&unknown).await.expect("read");
    assert!(rows.is_empty(), "no rows written for service 999");
}

// ---------------------------------------------------------------------------
// encode_service_backends_key — distinct service_ids produce distinct rows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn encode_service_backends_key_is_collision_free_for_distinct_services() {
    let (_tmp, store) = fresh_store().await;

    let row_a = make_backends_row(0xaa, 1, 100);
    let row_b = make_backends_row(0xbb, 2, 100);

    store.write(ObservationRow::ServiceBackend(row_a.clone())).await.expect("write row a");
    store.write(ObservationRow::ServiceBackend(row_b.clone())).await.expect("write row b");

    let rows_a = store.service_backends_rows(&row_a.service_id).await.expect("read a");
    let rows_b = store.service_backends_rows(&row_b.service_id).await.expect("read b");

    assert_eq!(rows_a, vec![row_a.clone()], "service A's row");
    assert_eq!(rows_b, vec![row_b.clone()], "service B's row");
}

#[tokio::test]
async fn service_backends_rows_returns_empty_for_unknown_service() {
    let (_tmp, store) = fresh_store().await;
    let unknown = ServiceId::new(777).expect("valid sid");
    let rows = store.service_backends_rows(&unknown).await.expect("read");
    assert!(rows.is_empty());
}

// ---------------------------------------------------------------------------
// apply_service_backends_lww — LWW idempotence + dominance
// ---------------------------------------------------------------------------
//
// Catches: line 480 `apply_service_backends_lww -> Ok(true)` (every write
// dominates, so older clobbers newer) and `Ok(false)` (no write ever
// lands, so reads return empty even after writes).

#[tokio::test]
async fn service_backends_lww_higher_counter_dominates_lower() {
    let (_tmp, store) = fresh_store().await;
    let sid = 1;

    // First write: counter=10, vip=10.1.0.1.
    let row_v1 = make_backends_row(sid, 1, 10);
    store.write(ObservationRow::ServiceBackend(row_v1.clone())).await.expect("write v1");

    // Second write: higher counter (20), different vip (10.1.0.5) —
    // MUST dominate.
    let row_v2 = make_backends_row(sid, 5, 20);
    store.write(ObservationRow::ServiceBackend(row_v2.clone())).await.expect("write v2");

    let rows = store.service_backends_rows(&row_v1.service_id).await.expect("read");

    // Catches `Ok(false)` mutation: with always-false, neither write
    // would land, so rows would be empty.
    assert_eq!(rows.len(), 1);
    // Catches `Ok(true)` mutation: with always-true, the second write
    // dominates anyway in this case — but the discriminator is the
    // older-timestamp test below.
    assert_eq!(rows[0], row_v2, "newer (counter=20) dominates");
}

#[tokio::test]
async fn service_backends_lww_lower_counter_does_not_clobber_higher() {
    // This is the discriminator for the `Ok(true)` mutation: with
    // always-true, the third write (older counter=5) WOULD clobber the
    // second write (counter=20). With real LWW, the older write is
    // rejected and the newer row remains.
    let (_tmp, store) = fresh_store().await;
    let sid = 1;

    let row_v1 = make_backends_row(sid, 1, 10);
    store.write(ObservationRow::ServiceBackend(row_v1.clone())).await.expect("write v1");

    let row_v2 = make_backends_row(sid, 5, 20);
    store.write(ObservationRow::ServiceBackend(row_v2.clone())).await.expect("write v2");

    // Older-counter write — MUST be rejected by LWW.
    let row_v0 = make_backends_row(sid, 99, 5);
    store.write(ObservationRow::ServiceBackend(row_v0.clone())).await.expect("write v0 (older)");

    let rows = store.service_backends_rows(&row_v1.service_id).await.expect("read");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0], row_v2, "older-timestamp write must not clobber newer; got {:?}", rows[0]);
}

#[tokio::test]
async fn service_backends_lww_idempotent_under_replay() {
    // Re-applying the SAME row twice must leave exactly one row,
    // unchanged. (LWW idempotency case — equal timestamps return false
    // on the second call per LogicalTimestamp::dominates.)
    let (_tmp, store) = fresh_store().await;
    let sid = 1;

    let row = make_backends_row(sid, 1, 10);
    store.write(ObservationRow::ServiceBackend(row.clone())).await.expect("write 1");
    store.write(ObservationRow::ServiceBackend(row.clone())).await.expect("write 2 (replay)");

    let rows = store.service_backends_rows(&row.service_id).await.expect("read");
    assert_eq!(rows, vec![row]);
}

// ---------------------------------------------------------------------------
// apply_service_hydration_lww — same shape as backends LWW
// ---------------------------------------------------------------------------

#[tokio::test]
async fn service_hydration_lww_higher_counter_dominates_lower() {
    let (_tmp, store) = fresh_store().await;
    let sid = 1;
    let fp = 0x1234_5678;

    let row_v1 = make_hydration_row(sid, fp, 10);
    let row_v2 = make_hydration_row(sid, fp, 20);

    store.write(ObservationRow::ServiceHydration(row_v1.clone())).await.expect("write v1");
    store.write(ObservationRow::ServiceHydration(row_v2.clone())).await.expect("write v2");

    let rows = store.service_hydration_results_rows(&row_v1.service_id).await.expect("read");

    assert_eq!(rows.len(), 1, "same (sid, fp) -> single row, newest wins");
    assert_eq!(rows[0], row_v2, "newer counter dominates");
}

#[tokio::test]
async fn service_hydration_lww_lower_counter_does_not_clobber_higher() {
    let (_tmp, store) = fresh_store().await;
    let sid = 1;
    let fp = 0x1234_5678;

    let row_v2 = make_hydration_row(sid, fp, 20);
    store.write(ObservationRow::ServiceHydration(row_v2.clone())).await.expect("write v2");

    // Older write attempts to clobber newer.
    let row_v1 = make_hydration_row(sid, fp, 10);
    store
        .write(ObservationRow::ServiceHydration(row_v1.clone()))
        .await
        .expect("write v1 (older, should be rejected by LWW)");

    let rows = store.service_hydration_results_rows(&row_v2.service_id).await.expect("read");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0], row_v2, "newer must persist; older must not clobber");
}

#[tokio::test]
async fn service_hydration_lww_idempotent_under_replay() {
    let (_tmp, store) = fresh_store().await;
    let row = make_hydration_row(1, 0x1234_5678, 10);

    store.write(ObservationRow::ServiceHydration(row.clone())).await.expect("write 1");
    store.write(ObservationRow::ServiceHydration(row.clone())).await.expect("write 2 (replay)");

    let rows = store.service_hydration_results_rows(&row.service_id).await.expect("read");
    assert_eq!(rows, vec![row]);
}
