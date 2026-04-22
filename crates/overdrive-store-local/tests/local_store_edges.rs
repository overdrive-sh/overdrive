//! Structural edge-case tests for `LocalStore` that sit alongside the
//! §4 acceptance scenarios. These fill in trait-contract corners not
//! covered by §4.1/§4.3 but still observable through the `IntentStore`
//! surface:
//!
//! * watch must also fire on a prefix-matching delete (trait docstring:
//!   "deletes are reported as empty value");
//! * overwriting a key must return the latest value;
//! * an empty transaction must commit as a no-op;
//! * opening two `LocalStore`s on the same path must not corrupt
//!   existing state (second open reads first writes).
//!
//! Per `.claude/rules/testing.md` Tier 3, all four use real redb backed
//! by `tempfile::TempDir`.

#![allow(clippy::expect_used)]

use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use overdrive_core::traits::intent_store::{IntentStore, TxnOutcome};
use overdrive_store_local::LocalStore;
use tempfile::TempDir;
use tokio::time::timeout;

#[tokio::test]
async fn watch_fires_on_delete_with_empty_value() {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalStore::open(tmp.path().join("intent.redb")).expect("open");

    // Seed a value so the delete has something to remove.
    store.put(b"jobs/payments", b"v1").await.expect("put");

    let mut watch = store.watch(b"jobs/").await.expect("watch");

    store.delete(b"jobs/payments").await.expect("delete");

    let event = timeout(Duration::from_secs(2), watch.next())
        .await
        .expect("delete event arrives")
        .expect("stream yields");

    // Key matches; value is empty to signal a delete per the trait
    // docstring.
    assert_eq!(event.0, Bytes::copy_from_slice(b"jobs/payments"));
    assert!(event.1.is_empty(), "delete event carries an empty value");
}

#[tokio::test]
async fn overwriting_a_key_returns_the_latest_value() {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalStore::open(tmp.path().join("intent.redb")).expect("open");

    store.put(b"jobs/payments", b"v1").await.expect("first put");
    store.put(b"jobs/payments", b"v2").await.expect("second put");

    let read = store.get(b"jobs/payments").await.expect("get");
    assert_eq!(read, Some(Bytes::copy_from_slice(b"v2")));
}

#[tokio::test]
async fn empty_transaction_commits_as_a_noop() {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalStore::open(tmp.path().join("intent.redb")).expect("open");

    let outcome = store.txn(Vec::new()).await.expect("empty txn");

    // No ops, but the transaction itself still commits — this matches
    // the trait contract that `txn` returns `TxnOutcome::Committed` on a
    // successful commit regardless of op count.
    assert!(matches!(outcome, TxnOutcome::Committed));
}

#[tokio::test]
async fn reopening_the_same_path_preserves_state() {
    let tmp = TempDir::new().expect("temp dir");
    let path = tmp.path().join("intent.redb");

    // Write through one LocalStore and drop it.
    {
        let store = LocalStore::open(&path).expect("first open");
        store.put(b"jobs/payments", b"durable").await.expect("put");
    }

    // Open a fresh LocalStore on the same path.
    let store = LocalStore::open(&path).expect("second open");
    let read = store.get(b"jobs/payments").await.expect("get");
    assert_eq!(read, Some(Bytes::copy_from_slice(b"durable")));
}
