//! Regression tests for the "phantom write" defect class — operations
//! that observe no underlying state change must not advance
//! `commit_index` and must not emit watch events.
//!
//! The redb backend formerly bumped the commit counter and fired a
//! `(key, empty-value)` delete event regardless of whether
//! `redb::Table::remove` actually removed a row, and committed a no-op
//! write transaction (with a counter bump) when handed an empty `txn`
//! ops vector. Two consequences:
//!
//! 1. `commit_index()` / `cluster_status` observers saw false
//!    monotonic advancement.
//! 2. `watch(prefix)` subscribers received a spurious delete event for
//!    a key that was never present.
//!
//! These tests pin the desired contract:
//!
//! * `delete(absent_key)` — counter unchanged, no watch event.
//! * `txn([])` — counter unchanged, returns `Committed`, no events.
//! * `txn([Delete{absent}, Put{...}])` — single bump (the put), no
//!   delete event for the absent key, put event still fires.

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
// delete(absent_key) must NOT advance commit_index. The store starts at
// 0 and stays at 0 — a phantom bump would surface this immediately.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_of_absent_key_does_not_advance_commit_index() {
    let (store, _tmp) = store();
    assert_eq!(store.commit_index(), 0);

    store.delete(b"jobs/never-existed").await.expect("delete absent");

    assert_eq!(
        store.commit_index(),
        0,
        "delete of an absent key must be an idempotent no-op — no \
         counter advancement, no false monotone for cluster_status \
         observers",
    );
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
// delete(absent_key) followed by a real put must produce exactly one
// commit_index advance — pins the boundary where the bump is gated on
// the remove having effect.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_absent_then_put_advances_commit_index_by_exactly_one() {
    let (store, _tmp) = store();
    assert_eq!(store.commit_index(), 0);

    store.delete(b"jobs/never-existed").await.expect("delete absent");
    store.put(b"jobs/payments", b"spec").await.expect("put");

    assert_eq!(
        store.commit_index(),
        1,
        "the put advances by 1; the prior absent-delete must have been \
         a no-op — a phantom bump would land us at 2",
    );
}

// ---------------------------------------------------------------------------
// Empty txn must NOT advance commit_index. Returns Committed (the txn
// is logically valid; it just had nothing to do) but the counter is
// untouched.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn empty_txn_does_not_advance_commit_index() {
    let (store, _tmp) = store();
    assert_eq!(store.commit_index(), 0);

    let outcome = store.txn(Vec::new()).await.expect("empty txn");

    assert!(
        matches!(outcome, TxnOutcome::Committed),
        "empty txn must report Committed (no failure), got {outcome:?}",
    );
    assert_eq!(
        store.commit_index(),
        0,
        "an empty txn has no effect — bumping commit_index would leak \
         false monotone advancement to cluster_status observers",
    );
}

// ---------------------------------------------------------------------------
// Empty txn must NOT emit any watch events. There is nothing to
// announce.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn empty_txn_emits_no_watch_event() {
    let (store, _tmp) = store();

    let mut watch = store.watch(b"jobs/").await.expect("watch subscribe");

    store.txn(Vec::new()).await.expect("empty txn");

    let next = timeout(Duration::from_millis(150), watch.next()).await;
    assert!(next.is_err(), "empty txn must not emit any watch events; got {next:?}");
}

// ---------------------------------------------------------------------------
// txn containing a Delete of an absent key + a real Put must:
//   - bump commit_index by exactly 1 (the txn-level bump, unchanged)
//   - emit a watch event for the put
//   - NOT emit a phantom delete event for the absent key
// This is the "per-op effective" gating extension of the same fix.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn txn_with_absent_delete_and_real_put_emits_only_the_put_event() {
    let (store, _tmp) = store();
    assert_eq!(store.commit_index(), 0);

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
    assert_eq!(
        store.commit_index(),
        1,
        "mixed txn advances commit_index by exactly 1 (per-txn bump)",
    );

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
