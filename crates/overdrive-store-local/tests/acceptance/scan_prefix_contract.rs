//! Acceptance tests for `IntentStore::scan_prefix` on `LocalIntentStore`
//! and the companion `open()`-time recovery walk.
//!
//! Added by Phase 5 aggregate mutation gate (May 2026) to close three
//! surviving mutants on `crates/overdrive-store-local/src/redb_backend.rs`
//! introduced by this feature's persistence work:
//!
//! * `:397` — `scan_prefix` body replaced with `Ok(vec![])`. Every
//!   `scan_prefix` caller relies on rows being returned; the
//!   roundtrip happy-path test below kills the mutation.
//! * `:409` — `if !key_bytes.starts_with(prefix)` break condition
//!   inverted (`delete !`). Without it, the iterator runs off the
//!   end of the prefix and returns rows from adjacent namespaces.
//!   The cross-prefix-isolation test below kills the mutation.
//! * `:149` — `open()` recovery walk's
//!   `key.ends_with(STOP_SUFFIX) || key.ends_with(KIND_SUFFIX)`
//!   filter flipped from `||` to `&&`. The flipped form would
//!   route `/stop` and `/kind` sentinel-key bytes through
//!   `WorkloadIntent::from_store_bytes`, which rejects them as
//!   envelope-decode failures. The recovery-walk-skips-sentinel-keys
//!   test below kills the mutation.
//!
//! Port-to-port discipline: every assertion drives the `IntentStore`
//! trait surface that `LocalIntentStore` implements. Recovery-walk
//! coverage is observed at the `open()` boundary — a successful re-open
//! after seeding sentinel + aggregate rows is the observable signal
//! that the walk routed bytes through the right decode path.

use bytes::Bytes;
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, JobSpecInput, JobV1, ResourcesInput, WorkloadIntent,
    WorkloadKind,
};
use overdrive_core::id::WorkloadId;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn job_intent(id_str: &str) -> WorkloadIntent {
    let spec = JobSpecInput {
        id: id_str.to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    };
    WorkloadIntent::Job(JobV1::from_submit(spec).expect("canonical job spec must validate"))
}

fn workload_id(id_str: &str) -> WorkloadId {
    WorkloadId::new(id_str).expect("canonical id must validate")
}

// ---------------------------------------------------------------------------
// scan_prefix happy path — returns the rows under the prefix in order
// Kills the `Ok(vec![])` body-replacement mutation on :397.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scan_prefix_returns_all_rows_under_prefix_in_ascending_key_order() {
    // Given a LocalIntentStore with three rows under the "workloads/"
    // prefix written in non-monotonic order.
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    let intent_a = job_intent("alpha");
    let intent_b = job_intent("bravo");
    let intent_c = job_intent("charlie");

    let key_b = IntentKey::for_workload(&workload_id("bravo"));
    let key_c = IntentKey::for_workload(&workload_id("charlie"));
    let key_a = IntentKey::for_workload(&workload_id("alpha"));

    // Insertion order intentionally not ascending — scan_prefix must
    // return rows in key order regardless.
    store
        .put(key_b.as_bytes(), intent_b.archive_for_store().expect("archive b").as_ref())
        .await
        .expect("put bravo");
    store
        .put(key_c.as_bytes(), intent_c.archive_for_store().expect("archive c").as_ref())
        .await
        .expect("put charlie");
    store
        .put(key_a.as_bytes(), intent_a.archive_for_store().expect("archive a").as_ref())
        .await
        .expect("put alpha");

    // When Ana calls scan_prefix on b"workloads/".
    let rows = store.scan_prefix(b"workloads/").await.expect("scan_prefix");

    // Then all three rows are returned in ascending key order.
    assert_eq!(rows.len(), 3, "all three rows under prefix must be returned; got {rows:?}");
    let returned_keys: Vec<Bytes> = rows.into_iter().map(|(k, _)| k).collect();
    assert_eq!(returned_keys[0], Bytes::copy_from_slice(key_a.as_bytes()));
    assert_eq!(returned_keys[1], Bytes::copy_from_slice(key_b.as_bytes()));
    assert_eq!(returned_keys[2], Bytes::copy_from_slice(key_c.as_bytes()));
}

// ---------------------------------------------------------------------------
// scan_prefix isolation — rows outside the prefix are excluded.
// Kills the `delete !` mutation on :409 (the prefix-break condition).
// Without the break, the iterator walks past the prefix into the next
// namespace and returns its rows; without inverting it (the mutation),
// the prefix predicate is wrong and the rows are still returned because
// the loop never terminates correctly. Either way the row count is
// wrong, so the assertion below catches both shapes.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scan_prefix_excludes_rows_outside_the_prefix() {
    // Given a LocalIntentStore with rows in two distinct namespaces.
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    let inside = job_intent("payments");
    let key_inside = IntentKey::for_workload(&workload_id("payments"));
    store
        .put(key_inside.as_bytes(), inside.archive_for_store().expect("archive").as_ref())
        .await
        .expect("put workloads/payments");

    // Adjacent key OUTSIDE the workloads/ prefix that lexicographically
    // SORTS AFTER it ("z..." > "workloads/...") — exercises the break-on-
    // mismatch path because the iterator naturally reaches it after the
    // last matching row.
    store.put(b"zones/eu-west-1", b"some-zone-bytes").await.expect("put zones");

    // And another adjacent key BEFORE the prefix ("a..." < "workloads/").
    // scan_prefix uses range(prefix..) so this row should not be seen at
    // all; if it is, the range start is wrong (a different shape of bug,
    // not the one we're killing — but the assertion covers it).
    store.put(b"audit/2026/event-001", b"audit-bytes").await.expect("put audit");

    // When Ana calls scan_prefix on b"workloads/".
    let rows = store.scan_prefix(b"workloads/").await.expect("scan_prefix");

    // Then exactly one row is returned — the one under workloads/.
    assert_eq!(
        rows.len(),
        1,
        "scan_prefix must isolate to the prefix; got {} rows ({rows:?})",
        rows.len(),
    );
    assert_eq!(rows[0].0, Bytes::copy_from_slice(key_inside.as_bytes()));
}

// ---------------------------------------------------------------------------
// scan_prefix empty-result path — no rows match.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scan_prefix_returns_empty_vec_when_no_rows_match() {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    // Seed an unrelated namespace.
    store.put(b"audit/2026/event-001", b"audit-bytes").await.expect("put");

    let rows = store.scan_prefix(b"workloads/").await.expect("scan_prefix");
    assert!(rows.is_empty(), "no matching rows ⇒ empty Vec; got {rows:?}");
}

// ---------------------------------------------------------------------------
// open() recovery walk — sentinel keys (/stop, /kind) are SKIPPED by
// the `key.ends_with(STOP_SUFFIX) || key.ends_with(KIND_SUFFIX)` filter
// in `LocalIntentStore::open` at :149.
//
// Mutation kill: `||` → `&&` makes the filter trivially false (no key
// ends with BOTH suffixes), so the recovery walk would route sentinel
// bytes (`b""` for /stop, single-byte kind discriminator for /kind)
// through `WorkloadIntent::from_store_bytes`, which would reject with
// `IntentStoreError::Envelope`. Re-opening after seeding such rows
// would fail; the assertion that it SUCCEEDS kills the mutation.
//
// We use Job-kind (`b"j"`) for the kind discriminator byte per
// `WorkloadKind::Job::discriminator_byte()` — matches the byte the
// production submit handler writes.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn open_recovery_walk_skips_stop_and_kind_sentinel_keys() {
    // Given a freshly-seeded store with one valid aggregate body + the
    // matching /stop and /kind sentinel rows under the same workload id.
    let tmp = TempDir::new().expect("temp dir");
    let path = tmp.path().join("intent.redb");

    {
        let store = LocalIntentStore::open(&path).expect("first open");
        let id = workload_id("payments");
        let intent = job_intent("payments");

        let key_body = IntentKey::for_workload(&id);
        let key_stop = IntentKey::for_workload_stop(&id);
        let key_kind = IntentKey::for_workload_kind(&id);

        store
            .put(key_body.as_bytes(), intent.archive_for_store().expect("archive").as_ref())
            .await
            .expect("put body");
        // /stop sentinel — existence is the signal, value is empty bytes.
        store.put(key_stop.as_bytes(), b"").await.expect("put stop");
        // /kind sentinel — single ASCII discriminator byte.
        store
            .put(key_kind.as_bytes(), &[WorkloadKind::Job.discriminator_byte()])
            .await
            .expect("put kind");
        // Drop the store handle so the redb file is closed before reopen.
    }

    // When Ana re-opens the store, exercising the recovery walk on a
    // pre-existing file that contains BOTH aggregate envelope bytes AND
    // sentinel rows.
    let reopened = LocalIntentStore::open(&path);

    // Then the open succeeds — the recovery walk routed the aggregate
    // body row through `from_store_bytes` (decoded cleanly) and SKIPPED
    // both sentinel rows. If the `||` → `&&` mutation were applied,
    // `from_store_bytes` would receive `b""` or `b"j"` and fail with
    // `EnvelopeError::Malformed`, propagated as `IntentStoreError::Envelope`.
    assert!(
        reopened.is_ok(),
        "recovery walk must skip /stop and /kind sentinels; reopen failed: {:?}",
        reopened.err(),
    );
}
