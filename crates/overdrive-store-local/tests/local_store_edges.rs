//! Structural edge-case tests for `LocalIntentStore` that sit alongside the
//! §4 acceptance scenarios. These fill in trait-contract corners not
//! covered by §4.1/§4.3 but still observable through the `IntentStore`
//! surface:
//!
//! * watch must also fire on a prefix-matching delete (trait docstring:
//!   "deletes are reported as empty value");
//! * overwriting a key must return the latest value;
//! * an empty transaction must commit as a no-op;
//! * opening two `LocalIntentStore`s on the same path must not corrupt
//!   existing state (second open reads first writes).
//!
//! Per `.claude/rules/testing.md` Tier 3, all four use real redb backed
//! by `tempfile::TempDir`.

#![allow(clippy::expect_used)]

use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use overdrive_core::traits::intent_store::{IntentStore, TxnOutcome};
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;
use tokio::time::timeout;

#[tokio::test]
async fn watch_fires_on_delete_with_empty_value() {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

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
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    store.put(b"jobs/payments", b"v1").await.expect("first put");
    store.put(b"jobs/payments", b"v2").await.expect("second put");

    // Overwrite assigns a new per-entry index — the second put took
    // global counter slot 2.
    let read = store.get(b"jobs/payments").await.expect("get");
    assert_eq!(read, Some((Bytes::copy_from_slice(b"v2"), 2)));
}

#[tokio::test]
async fn empty_transaction_commits_as_a_noop() {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

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

    // Write through one LocalIntentStore and drop it.
    {
        let store = LocalIntentStore::open(&path).expect("first open");
        store.put(b"jobs/payments", b"durable").await.expect("put");
    }

    // Open a fresh LocalIntentStore on the same path. The per-entry
    // commit_index survives reopen — the first put assigned index 1
    // before the original handle dropped.
    let store = LocalIntentStore::open(&path).expect("second open");
    let read = store.get(b"jobs/payments").await.expect("get");
    assert_eq!(read, Some((Bytes::copy_from_slice(b"durable"), 1)));
}

#[tokio::test]
async fn bootstrap_from_replaces_rather_than_merges_into_existing_state() {
    // The trait docstring describes `bootstrap_from` as replaying a
    // *full-state* snapshot as the initial state of the target store.
    // A merge semantics would silently corrupt that contract: a key
    // that exists in the target but NOT in the snapshot would survive
    // bootstrap, even though the snapshot's producer believed it was
    // describing the complete cluster state.
    let tmp = TempDir::new().expect("temp dir");

    // Producer store: writes a single key we want preserved through
    // bootstrap.
    let producer_path = tmp.path().join("producer.redb");
    let producer = LocalIntentStore::open(&producer_path).expect("producer open");
    producer.put(b"jobs/payments", b"from-producer").await.expect("producer put");
    let snapshot = producer.export_snapshot().await.expect("export");

    // Target store: seeded with a DIFFERENT key that must not survive
    // bootstrap. Full-state semantics require this key be gone.
    let target_path = tmp.path().join("target.redb");
    let target = LocalIntentStore::open(&target_path).expect("target open");
    target.put(b"jobs/leftover", b"should-be-wiped").await.expect("target put");

    target.bootstrap_from(snapshot).await.expect("bootstrap_from");

    // The producer's key is visible at its original per-entry index
    // (1 — the producer's first put).
    let producer_value = target.get(b"jobs/payments").await.expect("get producer key");
    assert_eq!(producer_value, Some((Bytes::copy_from_slice(b"from-producer"), 1)));

    // The pre-existing target-only key is GONE. Without the clear
    // step inside `bootstrap_from` this assertion fails — the leftover
    // row would survive the snapshot replay.
    let leftover = target.get(b"jobs/leftover").await.expect("get leftover");
    assert_eq!(leftover, None, "bootstrap_from must replace, not merge");
}
