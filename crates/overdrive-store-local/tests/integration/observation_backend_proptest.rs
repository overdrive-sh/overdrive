//! Proptest harness for `LocalObservationStore`'s `service-backends`
//! encoding helper and LWW-guarded insert.
//!
//! Closes the missed mutations the `QUALITY_GATE` wave's mutation run
//! flagged in `crates/overdrive-store-local/src/observation_backend.rs`:
//!
//! - `encode_service_backends_key` body replacement (`[0; 8]` / `[1; 8]`)
//! - `service_backends_rows` body replacement (`Ok(vec![])`)
//! - `apply_service_backends_lww` body replacement (`Ok(true)` / `Ok(false)`)
//!
//! The encoding helper (`encode_*`) is module-private; we exercise it
//! INDIRECTLY through `write` + `service_backends_rows`. A `[0; N]` /
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

use overdrive_core::id::{NodeId, ServiceId, SpiffeId};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationStore, ServiceBackendRow,
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

fn make_backend(addr_octet: u8) -> Backend {
    Backend {
        workload_identity: SpiffeId::new(&format!("spiffe://test/x/{addr_octet}")).expect("valid spiffe id"),
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
async fn single_backends_write_then_read_returns_row() {
    let (_tmp, store) = fresh_store().await;
    let row = make_backends_row(1, 1, 100);
    store
        .write(overdrive_core::traits::observation_store::ObservationWrite::ServiceBackend(
            row.clone(),
        ))
        .await
        .expect("write");
    let rows = store.service_backends_rows(&row.service_id).await.expect("read");
    assert_eq!(rows, vec![row]);
}

// ---------------------------------------------------------------------------
// encode_service_backends_key — distinct service_ids produce distinct rows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn encode_service_backends_key_is_collision_free_for_distinct_services() {
    let (_tmp, store) = fresh_store().await;

    let row_a = make_backends_row(0xaa, 1, 100);
    let row_b = make_backends_row(0xbb, 2, 100);

    store
        .write(overdrive_core::traits::observation_store::ObservationWrite::ServiceBackend(
            row_a.clone(),
        ))
        .await
        .expect("write row a");
    store
        .write(overdrive_core::traits::observation_store::ObservationWrite::ServiceBackend(
            row_b.clone(),
        ))
        .await
        .expect("write row b");

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
    store
        .write(overdrive_core::traits::observation_store::ObservationWrite::ServiceBackend(
            row_v1.clone(),
        ))
        .await
        .expect("write v1");

    // Second write: higher counter (20), different vip (10.1.0.5) —
    // MUST dominate.
    let row_v2 = make_backends_row(sid, 5, 20);
    store
        .write(overdrive_core::traits::observation_store::ObservationWrite::ServiceBackend(
            row_v2.clone(),
        ))
        .await
        .expect("write v2");

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
    store
        .write(overdrive_core::traits::observation_store::ObservationWrite::ServiceBackend(
            row_v1.clone(),
        ))
        .await
        .expect("write v1");

    let row_v2 = make_backends_row(sid, 5, 20);
    store
        .write(overdrive_core::traits::observation_store::ObservationWrite::ServiceBackend(
            row_v2.clone(),
        ))
        .await
        .expect("write v2");

    // Older-counter write — MUST be rejected by LWW.
    let row_v0 = make_backends_row(sid, 99, 5);
    store
        .write(overdrive_core::traits::observation_store::ObservationWrite::ServiceBackend(
            row_v0.clone(),
        ))
        .await
        .expect("write v0 (older)");

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
    store
        .write(overdrive_core::traits::observation_store::ObservationWrite::ServiceBackend(
            row.clone(),
        ))
        .await
        .expect("write 1");
    store
        .write(overdrive_core::traits::observation_store::ObservationWrite::ServiceBackend(
            row.clone(),
        ))
        .await
        .expect("write 2 (replay)");

    let rows = store.service_backends_rows(&row.service_id).await.expect("read");
    assert_eq!(rows, vec![row]);
}
