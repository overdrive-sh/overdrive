//! Regression tests for the "phantom write" defect class — operations
//! that observe no underlying state change must not emit watch events.
//!
//! The redb backend formerly fired a `(key, empty-value)` delete event
//! regardless of whether `redb::Table::remove` actually removed a row,
//! so `watch(prefix)` subscribers received a spurious delete event for
//! a key that was never present.
//!
//! Per ADR-0020 (drop `commit_index` from Phase 1) the previous
//! companion assertion — "no `commit_index` advancement" — has no
//! counter to assert against; only the watch-event property remains.
//! See `redesign-drop-commit-index/design/upstream-changes.md` §7.
//!
//! These tests pin the desired contract:
//!
//! * `delete(absent_key)` — no watch event.
//! * `txn([])` — returns `Committed`, no events.
//! * `txn([Delete{absent}, Put{...}])` — single put event, no
//!   delete event for the absent key.

#![allow(clippy::expect_used)]

use std::time::Duration;

use bytes::Bytes;
use overdrive_core::traits::intent_store::{IntentStore, TxnOp, TxnOutcome};
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;
use tokio::time::timeout;
use tokio_stream::StreamExt as _;

fn store() -> (LocalIntentStore, TempDir) {
    let tmp = TempDir::new().expect("TempDir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");
    (store, tmp)
}

// ---------------------------------------------------------------------------
// delete(absent_key) must NOT emit a watch event. We subscribe first,
// issue the delete, and assert the stream stays quiet for a short
// window. A phantom (key, empty) event would surface here.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_of_absent_key_emits_no_watch_event() {
    let (store, _tmp) = store();

    let mut watch = store.watch(b"jobs/").await.expect("watch subscribe");

    store.delete(b"jobs/never-existed").await.expect("delete absent");

    let next = timeout(Duration::from_millis(150), watch.next()).await;
    assert!(
        next.is_err(),
        "delete of an absent key must not emit a watch event; the \
         stream returned {next:?} within the quiet window",
    );
}

// ---------------------------------------------------------------------------
// Empty txn returns Committed (the txn is logically valid; it just had
// nothing to do) and emits no watch events.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn empty_txn_emits_no_watch_event() {
    let (store, _tmp) = store();

    let mut watch = store.watch(b"jobs/").await.expect("watch subscribe");

    let outcome = store.txn(Vec::new()).await.expect("empty txn");

    assert!(
        matches!(outcome, TxnOutcome::Committed),
        "empty txn must report Committed (no failure), got {outcome:?}",
    );
    let next = timeout(Duration::from_millis(150), watch.next()).await;
    assert!(next.is_err(), "empty txn must not emit any watch events; got {next:?}");
}

// ---------------------------------------------------------------------------
// txn containing a Delete of an absent key + a real Put must:
//   - emit a watch event for the put
//   - NOT emit a phantom delete event for the absent key
// ---------------------------------------------------------------------------

#[tokio::test]
async fn txn_with_absent_delete_and_real_put_emits_only_the_put_event() {
    let (store, _tmp) = store();

    let mut watch = store.watch(b"jobs/").await.expect("watch subscribe");

    let outcome = store
        .txn(vec![
            TxnOp::Delete { key: Bytes::from_static(b"jobs/never-existed") },
            TxnOp::Put {
                key: Bytes::from_static(b"jobs/payments"),
                value: Bytes::from_static(b"spec"),
            },
        ])
        .await
        .expect("mixed txn");

    assert!(matches!(outcome, TxnOutcome::Committed), "mixed txn must commit");

    // First event must be the put — not a phantom delete.
    let first = timeout(Duration::from_secs(2), watch.next())
        .await
        .expect("first event arrives within window")
        .expect("watch stream open");
    assert_eq!(first.0, Bytes::from_static(b"jobs/payments"));
    assert_eq!(
        first.1,
        Bytes::from_static(b"spec"),
        "first emitted event must be the put — a phantom delete event \
         for jobs/never-existed would arrive here as (key, empty)",
    );

    // No further events — specifically no phantom delete.
    let tail = timeout(Duration::from_millis(150), watch.next()).await;
    assert!(
        tail.is_err(),
        "txn with absent-key delete must NOT emit a phantom delete \
         event; got tail event {tail:?}",
    );
}
