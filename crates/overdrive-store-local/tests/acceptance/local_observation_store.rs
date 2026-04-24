//! Acceptance tests for `LocalObservationStore` — step 03-06.
//!
//! `LocalObservationStore` is the redb-backed `ObservationStore` impl that
//! replaces `SimObservationStore` in the Phase 1 production server wiring
//! per ADR-0012 (revised 2026-04-24). These tests prove:
//!
//! 1. Write-then-read works within a single process lifetime for both
//!    row classes (`AllocStatusRow`, `NodeHealthRow`).
//! 2. **Restart round-trip** — objection-(1) regression gate. Writes
//!    persist across store `drop` + `LocalObservationStore::open` reopen
//!    against the same redb file.
//! 3. Subscription delivery: `subscribe_all()` then `write` delivers the
//!    row on the broadcast stream within bounded tokio poll time.
//!    Future-only contract — subscribers opened AFTER a write do not see
//!    the historical row.
//! 4. Overwrite on same key: writing twice for the same `AllocationId`
//!    leaves exactly one row.
//! 5. The `overdrive-sim` crate is absent from `overdrive-control-plane`'s
//!    runtime dependency graph (read from Cargo.toml directly).
//!
//! Real redb file I/O via `tempfile::TempDir` — matches the existing
//! `local_store_basic_ops` convention in this crate.

use std::time::Duration;

use futures::StreamExt;
use overdrive_core::id::{AllocationId, JobId, NodeId, Region};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, NodeHealthRow, ObservationRow, ObservationStore,
};
use overdrive_store_local::LocalObservationStore;
use tempfile::TempDir;
use tokio::time::timeout;

// ---------------------------------------------------------------------------
// Row helpers
// ---------------------------------------------------------------------------

fn alloc_row(alloc_id: &str, state: AllocState, counter: u64) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::new(alloc_id).expect("valid alloc id"),
        job_id: JobId::new("payments").expect("valid job id"),
        node_id: NodeId::new("control-plane-0").expect("valid node id"),
        state,
        updated_at: LogicalTimestamp {
            counter,
            writer: NodeId::new("control-plane-0").expect("valid writer node id"),
        },
    }
}

fn node_row(node_id: &str, counter: u64) -> NodeHealthRow {
    NodeHealthRow {
        node_id: NodeId::new(node_id).expect("valid node id"),
        region: Region::new("local").expect("valid region"),
        last_heartbeat: LogicalTimestamp {
            counter,
            writer: NodeId::new(node_id).expect("valid writer node id"),
        },
    }
}

// ---------------------------------------------------------------------------
// AC 1 — write-then-read within a single process lifetime
// ---------------------------------------------------------------------------

#[tokio::test]
async fn write_then_read_alloc_status_within_lifetime() {
    let tmp = TempDir::new().expect("tempdir");
    let store = LocalObservationStore::open(tmp.path().join("observation"))
        .expect("open observation store");

    let row = alloc_row("alloc-1", AllocState::Running, 1);
    store.write(ObservationRow::AllocStatus(row.clone())).await.expect("write row");

    let rows = store.alloc_status_rows().await.expect("read alloc rows");
    assert_eq!(rows, vec![row], "single write must appear on read");
}

#[tokio::test]
async fn write_then_read_node_health_within_lifetime() {
    let tmp = TempDir::new().expect("tempdir");
    let store = LocalObservationStore::open(tmp.path().join("observation"))
        .expect("open observation store");

    let row = node_row("control-plane-0", 7);
    store.write(ObservationRow::NodeHealth(row.clone())).await.expect("write row");

    let rows = store.node_health_rows().await.expect("read node rows");
    assert_eq!(rows, vec![row], "single write must appear on read");
}

// ---------------------------------------------------------------------------
// AC 2 — restart round-trip (objection-(1) regression gate)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn restart_round_trip_alloc_status() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("observation");

    let row_a = alloc_row("alloc-a", AllocState::Running, 1);
    let row_b = alloc_row("alloc-b", AllocState::Pending, 2);

    {
        let store = LocalObservationStore::open(&path).expect("open (lifetime 1)");
        store.write(ObservationRow::AllocStatus(row_a.clone())).await.expect("write row_a");
        store.write(ObservationRow::AllocStatus(row_b.clone())).await.expect("write row_b");
        drop(store);
    }

    // New process lifetime — reopen the same redb file.
    let store2 = LocalObservationStore::open(&path).expect("open (lifetime 2)");
    let mut rows = store2.alloc_status_rows().await.expect("read after reopen");
    rows.sort_by(|a, b| a.alloc_id.as_str().cmp(b.alloc_id.as_str()));
    assert_eq!(
        rows,
        vec![row_a, row_b],
        "rows written before drop must survive reopen (objection-(1) gate)"
    );
}

#[tokio::test]
async fn restart_round_trip_node_health() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("observation");

    let row = node_row("control-plane-0", 42);

    {
        let store = LocalObservationStore::open(&path).expect("open (lifetime 1)");
        store.write(ObservationRow::NodeHealth(row.clone())).await.expect("write");
        drop(store);
    }

    let store2 = LocalObservationStore::open(&path).expect("open (lifetime 2)");
    let rows = store2.node_health_rows().await.expect("read after reopen");
    assert_eq!(rows, vec![row], "node health row must survive reopen");
}

// ---------------------------------------------------------------------------
// AC 3 — subscription delivery (future-only)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn subscribe_all_delivers_subsequent_writes() {
    let tmp = TempDir::new().expect("tempdir");
    let store = LocalObservationStore::open(tmp.path().join("observation"))
        .expect("open observation store");

    let mut sub = store.subscribe_all().await.expect("subscribe");

    let row = alloc_row("alloc-1", AllocState::Running, 1);
    let written = ObservationRow::AllocStatus(row.clone());
    store.write(written.clone()).await.expect("write after subscribe");

    let delivered = timeout(Duration::from_secs(2), sub.next())
        .await
        .expect("subscription delivers within timeout")
        .expect("stream yields the row");
    assert_eq!(delivered, written, "subscription must deliver the written row");
}

#[tokio::test]
async fn subscriber_opened_after_write_does_not_see_historical_row() {
    let tmp = TempDir::new().expect("tempdir");
    let store = LocalObservationStore::open(tmp.path().join("observation"))
        .expect("open observation store");

    // Write happens BEFORE the subscription — future-only contract.
    let historical = alloc_row("alloc-historical", AllocState::Terminated, 1);
    store.write(ObservationRow::AllocStatus(historical.clone())).await.expect("historical write");

    let mut sub = store.subscribe_all().await.expect("subscribe AFTER write");

    // Give the broadcast channel a tokio tick — if history were being
    // replayed, the timeout would fire with Some(row).
    let result = timeout(Duration::from_millis(200), sub.next()).await;
    assert!(result.is_err(), "subscription opened after write must not replay historical rows");
}

// ---------------------------------------------------------------------------
// AC 4 — overwrite on same key replaces first row
// ---------------------------------------------------------------------------

#[tokio::test]
async fn overwrite_same_key_replaces_first_row() {
    let tmp = TempDir::new().expect("tempdir");
    let store = LocalObservationStore::open(tmp.path().join("observation"))
        .expect("open observation store");

    let initial = alloc_row("alloc-1", AllocState::Pending, 1);
    let updated = alloc_row("alloc-1", AllocState::Running, 2);

    store.write(ObservationRow::AllocStatus(initial)).await.expect("write initial");
    store.write(ObservationRow::AllocStatus(updated.clone())).await.expect("write updated");

    let rows = store.alloc_status_rows().await.expect("read alloc rows");
    assert_eq!(rows.len(), 1, "same-key writes collapse to one row");
    assert_eq!(rows[0], updated, "second write must be the surviving row");
}

// ---------------------------------------------------------------------------
// AC 6 — overdrive-sim NOT in overdrive-control-plane runtime deps
// ---------------------------------------------------------------------------

#[tokio::test]
async fn overdrive_sim_not_in_control_plane_runtime_deps() {
    // Locate the overdrive-control-plane Cargo.toml by walking from this
    // test's compile-time manifest dir upward to the workspace root.
    // `CARGO_MANIFEST_DIR` is `.../crates/overdrive-store-local`.
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ws_root =
        manifest_dir.parent().and_then(std::path::Path::parent).expect("walk to workspace root");
    let cp_manifest = ws_root.join("crates").join("overdrive-control-plane").join("Cargo.toml");
    assert!(cp_manifest.exists(), "expected manifest at {}", cp_manifest.display());

    let raw = std::fs::read_to_string(&cp_manifest).expect("read control-plane Cargo.toml");

    // Split into top-level tables. We want the [dependencies] section
    // (runtime), NOT [dev-dependencies] or [build-dependencies].
    //
    // A trivial scan is sufficient: the crate's Cargo.toml is
    // hand-authored and flat — no target-conditional deps. We find the
    // [dependencies] header, take lines until the next [section] header
    // or EOF, and assert overdrive-sim is not referenced.
    let mut in_runtime_deps = false;
    let mut runtime_dep_lines = Vec::<String>::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_runtime_deps = trimmed == "[dependencies]";
            continue;
        }
        if in_runtime_deps {
            // Strip line comments before the dep-name match so doc
            // references to `overdrive-sim` (the dep we just removed)
            // do not give a false positive.
            let code = trimmed.find('#').map_or(trimmed, |i| &trimmed[..i]);
            let code = code.trim();
            if !code.is_empty() {
                runtime_dep_lines.push(code.to_string());
            }
        }
    }

    let runtime_block = runtime_dep_lines.join("\n");
    assert!(
        !runtime_block.contains("overdrive-sim") && !runtime_block.contains("overdrive_sim"),
        "overdrive-sim must not appear in [dependencies] of overdrive-control-plane \
         (runtime dep graph hygiene per ADR-0012 revision). Found:\n{runtime_block}"
    );
}
