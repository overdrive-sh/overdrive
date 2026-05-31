//! Tier 3 integration — `LocalObservationStore::write_probe_result`
//! / `list_probe_results_for_alloc` roundtrip across redb reopens
//! plus LWW semantics on `(alloc_id, probe_idx)`.
//!
//! Per ADR-0054 §5 (probe-results table) and ADR-0048 (versioned
//! envelope evolution) — every persisted `ProbeResultRow` round-trips
//! bit-identical through the rkyv envelope, and stale writes are
//! no-ops under the strict-dominate LWW rule.

#![allow(clippy::expect_used)]

use overdrive_core::id::AllocationId;
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeStatus};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_store_local::LocalObservationStore;
use tempfile::TempDir;

fn alloc(id: &str) -> AllocationId {
    AllocationId::new(id).expect("alloc id parses")
}

/// S-SHCP-INT-01-04 (US-01 / AC #6) — write a probe-result row,
/// reopen the redb file, read it back bit-identical. Asserts the
/// envelope codec survives an fsync + reopen.
#[tokio::test]
async fn probe_result_row_survives_redb_reopen_bit_identical() {
    let tmp = TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("obs.redb");

    let alloc_id = alloc("alloc-roundtrip-1");
    let row = ProbeResultRow {
        alloc_id: alloc_id.clone(),
        probe_idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        status: ProbeStatus::Pass,
        last_observed_at_unix_ms: 42_000,
        inferred: false,
    };

    {
        let store = LocalObservationStore::open(&db_path).expect("open store");
        store.write_probe_result(row.clone()).await.expect("write probe result");
    }

    let store = LocalObservationStore::open(&db_path).expect("reopen store");
    let read = store
        .list_probe_results_for_alloc(&alloc_id)
        .await
        .expect("list probe results after reopen");
    assert_eq!(read.len(), 1, "exactly one row survives reopen");
    assert_eq!(read[0], row, "row content bit-identical across reopen");
}

/// S-SHCP-INT-01-05 (US-01 / AC #6) — LWW on `(alloc_id,
/// probe_idx)`: a newer-timestamp write dominates; a stale write
/// at the same key is a no-op.
#[tokio::test]
async fn probe_result_lww_strict_dominate_on_last_observed_ms() {
    let tmp = TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("obs.redb");

    let alloc_id = alloc("alloc-lww-1");
    let initial = ProbeResultRow {
        alloc_id: alloc_id.clone(),
        probe_idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        status: ProbeStatus::Pass,
        last_observed_at_unix_ms: 42_000,
        inferred: false,
    };

    let store = LocalObservationStore::open(&db_path).expect("open store");
    store.write_probe_result(initial.clone()).await.expect("write initial");

    // Newer timestamp + new status — dominates.
    let newer = ProbeResultRow {
        last_observed_at_unix_ms: 50_000,
        status: ProbeStatus::Fail { last_fail_reason: "connection refused".to_owned() },
        ..initial.clone()
    };
    store.write_probe_result(newer.clone()).await.expect("write newer");

    // Stale write — strict-dominate rule rejects.
    let stale = ProbeResultRow { last_observed_at_unix_ms: 30_000, ..initial.clone() };
    store.write_probe_result(stale).await.expect("stale write returns Ok (no-op)");

    // Equal-timestamp re-write — also a no-op (strict-dominate,
    // not >=).
    let equal_ts = ProbeResultRow {
        last_observed_at_unix_ms: 50_000,
        status: ProbeStatus::Fail { last_fail_reason: "different reason".to_owned() },
        ..initial.clone()
    };
    store.write_probe_result(equal_ts).await.expect("equal-ts write returns Ok (no-op)");

    let read = store.list_probe_results_for_alloc(&alloc_id).await.expect("list after LWW writes");
    assert_eq!(read.len(), 1, "single row at composite key");
    assert_eq!(read[0], newer, "LWW winner is the newer row");
}

/// S-SHCP-INT-01-06 (US-01 / AC #6) — multiple `probe_idx` values
/// for the same alloc are independently keyed; `list_probe_results_for_alloc`
/// returns all rows for the alloc sorted ascending by `probe_idx`.
#[tokio::test]
async fn probe_results_per_alloc_per_probe_idx_independent_keys() {
    let tmp = TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("obs.redb");
    let store = LocalObservationStore::open(&db_path).expect("open store");

    let alloc_id = alloc("alloc-multi-probe-1");
    let other_alloc = alloc("alloc-multi-probe-2");

    let mk = |idx: u32, alloc: &AllocationId, status: ProbeStatus| ProbeResultRow {
        alloc_id: alloc.clone(),
        probe_idx: ProbeIdx::new(idx),
        role: ProbeRole::Startup,
        status,
        last_observed_at_unix_ms: 10_000 + u64::from(idx),
        inferred: false,
    };

    // Insert in non-monotonic order to verify sort-by-probe_idx.
    store.write_probe_result(mk(2, &alloc_id, ProbeStatus::Pass)).await.expect("write idx 2");
    store.write_probe_result(mk(0, &alloc_id, ProbeStatus::Pass)).await.expect("write idx 0");
    store
        .write_probe_result(mk(
            1,
            &alloc_id,
            ProbeStatus::Fail { last_fail_reason: "transient".to_owned() },
        ))
        .await
        .expect("write idx 1");

    // Different-alloc row at probe_idx 0 — must NOT appear in the
    // first alloc's read.
    store
        .write_probe_result(mk(0, &other_alloc, ProbeStatus::Pass))
        .await
        .expect("write other-alloc row");

    let read = store.list_probe_results_for_alloc(&alloc_id).await.expect("list probe results");
    assert_eq!(read.len(), 3, "three rows for the alloc");
    assert_eq!(read[0].probe_idx, ProbeIdx::new(0), "sorted by probe_idx ascending");
    assert_eq!(read[1].probe_idx, ProbeIdx::new(1));
    assert_eq!(read[2].probe_idx, ProbeIdx::new(2));
    for r in &read {
        assert_eq!(r.alloc_id, alloc_id, "no cross-alloc leakage");
    }

    let other_read =
        store.list_probe_results_for_alloc(&other_alloc).await.expect("list other-alloc rows");
    assert_eq!(other_read.len(), 1, "other alloc has its own row");
    assert_eq!(other_read[0].probe_idx, ProbeIdx::new(0));
}

/// S-SHCP-INT-01-07 (US-01 / AC #6) — listing an alloc with no
/// probe-result rows returns `Ok(vec![])`, never an error.
#[tokio::test]
async fn probe_results_for_unknown_alloc_returns_empty() {
    let tmp = TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("obs.redb");
    let store = LocalObservationStore::open(&db_path).expect("open store");

    let alloc_id = alloc("alloc-nonexistent");
    let read = store
        .list_probe_results_for_alloc(&alloc_id)
        .await
        .expect("list returns Ok even for unknown alloc");
    assert!(read.is_empty(), "no rows for an alloc that has never been observed");
}
