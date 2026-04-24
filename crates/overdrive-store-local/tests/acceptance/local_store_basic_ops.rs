//! Acceptance scenarios for US-03 §4.1 and §4.3 — `LocalIntentStore` basic
//! operations on real redb.
//!
//! Translates `docs/feature/phase-1-foundation/distill/test-scenarios.md`
//! §4.1 four happy-path scenarios (roundtrip, watch, delete, txn) plus
//! the §4.3 first error scenario (absence-read returns nothing without
//! error) into Rust `#[tokio::test]` bodies.
//!
//! Port-to-port discipline: every assertion drives the `IntentStore`
//! trait surface that `LocalIntentStore` implements. No internal types are
//! inspected.
//!
//! Strategy C per DWD-01: real redb, `tempfile::TempDir` backing path.

use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use overdrive_core::traits::intent_store::{IntentStore, TxnOp, TxnOutcome};
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;
use tokio::time::timeout;

// -----------------------------------------------------------------------------
// §4.1 scenario 1 — "A value written to LocalIntentStore can be read back on the
// same store"
// -----------------------------------------------------------------------------

#[tokio::test]
async fn a_value_written_can_be_read_back() {
    // Given a freshly constructed LocalIntentStore backed by real redb on a
    // temporary path.
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    // When Ana writes bytes B under key K.
    let key: &[u8] = b"jobs/payments";
    let value: &[u8] = b"spec-bytes-v1";
    store.put(key, value).await.expect("put");

    // And Ana reads key K from the same store.
    let read = store.get(key).await.expect("get");

    // Then the returned bytes equal B.
    assert_eq!(read, Some(Bytes::copy_from_slice(value)));
}

// -----------------------------------------------------------------------------
// §4.1 scenario 2 — "A watch subscription on a prefix fires exactly once
// per matching write"
// -----------------------------------------------------------------------------

#[tokio::test]
async fn watch_fires_once_per_prefix_matching_write_and_ignores_non_matching() {
    // Given a freshly constructed LocalIntentStore backed by real redb, and a
    // watch subscription for the prefix "jobs/".
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    let mut watch = store.watch(b"jobs/").await.expect("watch subscribe");

    // When another task writes a value under the key "jobs/payments".
    store.put(b"jobs/payments", b"spec-v1").await.expect("put matching");

    // And another task writes under a non-matching prefix.
    store.put(b"nodes/node-1", b"node-spec").await.expect("put non-matching");

    // Then the subscription yields one event whose key is "jobs/payments".
    let first = timeout(Duration::from_secs(2), watch.next())
        .await
        .expect("watch event arrives")
        .expect("stream yields a value");

    assert_eq!(first.0, Bytes::copy_from_slice(b"jobs/payments"));
    assert_eq!(first.1, Bytes::copy_from_slice(b"spec-v1"));

    // And no further events are delivered for the non-matching write.
    // (We use a short timeout — if a non-matching event leaks through,
    // next() returns before the timeout fires.)
    let tail = timeout(Duration::from_millis(100), watch.next()).await;
    assert!(tail.is_err(), "non-matching write must not wake the 'jobs/' subscription");
}

// -----------------------------------------------------------------------------
// §4.1 scenario 3 — "Deleting a key removes it from subsequent reads"
// -----------------------------------------------------------------------------

#[tokio::test]
async fn deleting_a_key_removes_it_from_subsequent_reads() {
    // Given a LocalIntentStore containing a value under key K.
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");
    let key: &[u8] = b"jobs/payments";
    store.put(key, b"spec-v1").await.expect("put");
    assert!(store.get(key).await.expect("get pre-delete").is_some());

    // When Ana deletes key K.
    store.delete(key).await.expect("delete");

    // And Ana reads key K.
    let read = store.get(key).await.expect("get post-delete");

    // Then the read returns nothing.
    assert_eq!(read, None);
}

// -----------------------------------------------------------------------------
// §4.1 scenario 4 — "A transaction commits all operations atomically on
// success"
// -----------------------------------------------------------------------------

#[tokio::test]
async fn a_transaction_commits_all_operations_atomically() {
    // Given a freshly constructed LocalIntentStore.
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    // Seed a key that the transaction will delete.
    store.put(b"jobs/legacy", b"legacy-spec").await.expect("seed");

    // When Ana submits a transaction containing two put operations and
    // one delete.
    let ops = vec![
        TxnOp::Put { key: Bytes::from_static(b"jobs/payments"), value: Bytes::from_static(b"v1") },
        TxnOp::Put { key: Bytes::from_static(b"jobs/auth"), value: Bytes::from_static(b"v1") },
        TxnOp::Delete { key: Bytes::from_static(b"jobs/legacy") },
    ];
    let outcome = store.txn(ops).await.expect("txn executes");

    // Then the transaction outcome is committed.
    assert!(matches!(outcome, TxnOutcome::Committed), "expected Committed, got {outcome:?}");

    // And every put is readable from the store.
    assert_eq!(
        store.get(b"jobs/payments").await.expect("get payments"),
        Some(Bytes::copy_from_slice(b"v1"))
    );
    assert_eq!(
        store.get(b"jobs/auth").await.expect("get auth"),
        Some(Bytes::copy_from_slice(b"v1"))
    );

    // And the deleted key returns nothing.
    assert_eq!(store.get(b"jobs/legacy").await.expect("get legacy"), None);
}

// -----------------------------------------------------------------------------
// §4.3 scenario 1 — "A read on an absent key returns nothing without
// error"
// -----------------------------------------------------------------------------

#[tokio::test]
async fn a_read_on_an_absent_key_returns_nothing_without_error() {
    // Given a freshly constructed LocalIntentStore with no entries.
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    // When Ana reads a key that has never been written.
    let result = store.get(b"nonexistent/key").await;

    // Then the read returns nothing, and no error is reported.
    assert!(result.is_ok(), "absent-key read must not be an error");
    assert_eq!(result.expect("ok"), None);
}
