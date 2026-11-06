//! Structural edge-case tests for `LocalIntentStore` that sit alongside the
//! §4 acceptance scenarios. These fill in trait-contract corners not
//! covered by §4.1/§4.3 but still observable through the `IntentStore`
//! surface:
//!
//! * watch must also fire on a prefix-matching delete (trait docstring:
//!   a committed delete is reported as `Changed { value: None }`);
//! * overwriting a key must return the latest value;
//! * an empty transaction must commit as a no-op;
//! * opening two `LocalIntentStore`s on the same path must not corrupt
//!   existing state (second open reads first writes).
//!
//! * a subscriber that falls behind is told how many events it missed
//!   and then resumes with the next committed change (S-PIG-48).
//!
//! Per `.claude/rules/testing.md` Tier 3, all of them use real redb backed
//! by `tempfile::TempDir`.

#![allow(clippy::expect_used)]
#![allow(clippy::doc_markdown, reason = "per-test CONTRACT_SHAPE declarations")]

use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use overdrive_core::traits::intent_store::{
    IntentStore, IntentSubscriptionEvent, TxnOp, TxnOutcome,
};
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;
use tokio::time::timeout;

#[tokio::test]
async fn watch_fires_on_delete_with_no_value() {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    // Seed a value so the delete has something to remove.
    store.put(b"kv/payments", b"v1").await.expect("put");

    let mut watch = store.watch(b"kv/").await.expect("watch");

    store.delete(b"kv/payments").await.expect("delete");

    let event = timeout(Duration::from_secs(2), watch.next())
        .await
        .expect("delete event arrives")
        .expect("stream yields");

    // Key matches; `value: None` signals a committed delete per the trait
    // docstring.
    assert_eq!(
        event,
        IntentSubscriptionEvent::Changed {
            key: Bytes::copy_from_slice(b"kv/payments"),
            value: None,
        },
    );
}

#[tokio::test]
async fn overwriting_a_key_returns_the_latest_value() {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    store.put(b"kv/payments", b"v1").await.expect("first put");
    store.put(b"kv/payments", b"v2").await.expect("second put");

    // Per ADR-0020 the read returns bytes only — the latest value
    // (v2) is what surfaces.
    let read = store.get(b"kv/payments").await.expect("get");
    assert_eq!(read, Some(Bytes::copy_from_slice(b"v2")));
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
        store.put(b"kv/payments", b"durable").await.expect("put");
    }

    // Open a fresh LocalIntentStore on the same path. Per ADR-0020 the
    // read returns bytes only — there is no per-entry index column.
    let store = LocalIntentStore::open(&path).expect("second open");
    let read = store.get(b"kv/payments").await.expect("get");
    assert_eq!(read, Some(Bytes::copy_from_slice(b"durable")));
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
    producer.put(b"kv/payments", b"from-producer").await.expect("producer put");
    let snapshot = producer.export_snapshot().await.expect("export");

    // Target store: seeded with a DIFFERENT key that must not survive
    // bootstrap. Full-state semantics require this key be gone.
    let target_path = tmp.path().join("target.redb");
    let target = LocalIntentStore::open(&target_path).expect("target open");
    target.put(b"kv/leftover", b"should-be-wiped").await.expect("target put");

    target.bootstrap_from(snapshot).await.expect("bootstrap_from");

    // The producer's key is visible after bootstrap. Per ADR-0020 the
    // read returns bytes only.
    let producer_value = target.get(b"kv/payments").await.expect("get producer key");
    assert_eq!(producer_value, Some(Bytes::copy_from_slice(b"from-producer")));

    // The pre-existing target-only key is GONE. Without the clear
    // step inside `bootstrap_from` this assertion fails — the leftover
    // row would survive the snapshot replay.
    let leftover = target.get(b"kv/leftover").await.expect("get leftover");
    assert_eq!(leftover, None, "bootstrap_from must replace, not merge");
}

#[tokio::test]
async fn open_creates_missing_parent_directory() {
    // On a fresh production install the XDG-default `data_dir`
    // (`~/.local/share/overdrive`) does not yet exist. Boot paths that
    // open `LocalIntentStore` first — i.e. without relying on a sibling
    // store's directory-creation side effect — must still succeed.
    // Pinning this contract here keeps the store self-sufficient and
    // independent of caller ordering, mirroring the same shape
    // `LocalObservationStore::open` already implements.
    let tmp = TempDir::new().expect("temp dir");
    let missing_parent = tmp.path().join("does-not-exist-yet");
    let path = missing_parent.join("intent.redb");

    assert!(!missing_parent.exists(), "precondition: parent must be absent");

    let store = LocalIntentStore::open(&path).expect("open creates parent");

    // The store is usable end-to-end after open — not just a no-op
    // success that papered over a half-initialised database.
    store.put(b"kv/payments", b"v1").await.expect("put after open");
    let read = store.get(b"kv/payments").await.expect("get after open");
    assert_eq!(read, Some(Bytes::copy_from_slice(b"v1")));
}

/// S-PIG-48: a subscriber that falls behind the store's change notifications
/// is told exactly how many events it missed, and then resumes with the next
/// committed change; no event is silently dropped.
///
/// The accounting is stated at the port only: every committed write is
/// either delivered as `Changed` or counted in a `Lagged.skipped`, the
/// delivered keys are the contiguous suffix after the gap, and the stream
/// keeps delivering afterwards.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn lagged_subscription_reports_the_exact_missed_count_and_resumes_with_the_next_change() {
    // Far more matching writes than any in-process notification buffer holds.
    const WRITES: usize = 4_096;

    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");
    let mut watch = store.watch(b"jobs/").await.expect("watch");

    // Given the subscriber has read nothing while one batch of WRITES
    // matching puts commits.
    let ops = (0..WRITES)
        .map(|i| TxnOp::Put {
            key: Bytes::from(format!("jobs/{i:05}")),
            value: Bytes::from(format!("spec-{i}")),
        })
        .collect();
    let outcome = store.txn(ops).await.expect("batch commits");
    assert!(matches!(outcome, TxnOutcome::Committed));

    // When the subscriber finally reads until the stream goes quiet.
    let mut events = Vec::new();
    while let Ok(Some(event)) = timeout(Duration::from_millis(300), watch.next()).await {
        events.push(event);
    }

    // Then the gap is announced before any delivered change.
    assert!(
        matches!(events.first(), Some(IntentSubscriptionEvent::Lagged { .. })),
        "the first item must announce the gap, got {:?}",
        events.first(),
    );
    let skipped_total: u64 = events
        .iter()
        .filter_map(|event| match event {
            IntentSubscriptionEvent::Lagged { skipped } => Some(skipped.get()),
            IntentSubscriptionEvent::Changed { .. } => None,
        })
        .sum();
    let delivered: Vec<&Bytes> = events
        .iter()
        .filter_map(|event| match event {
            IntentSubscriptionEvent::Changed { key, .. } => Some(key),
            IntentSubscriptionEvent::Lagged { .. } => None,
        })
        .collect();

    // And every committed write is either delivered or counted as missed.
    assert_eq!(
        skipped_total + delivered.len() as u64,
        WRITES as u64,
        "missed count plus delivered changes must equal the committed writes",
    );
    // And the delivered changes resume exactly after the missed ones.
    let first_expected = Bytes::from(format!("jobs/{skipped_total:05}"));
    assert_eq!(delivered.first().copied(), Some(&first_expected));
    let last_expected = Bytes::from(format!("jobs/{:05}", WRITES - 1));
    assert_eq!(delivered.last().copied(), Some(&last_expected));

    // And the next committed change still arrives as a normal `Changed`.
    store.put(b"jobs/after-the-gap", b"spec-after").await.expect("put after the gap");
    let next = timeout(Duration::from_secs(2), watch.next())
        .await
        .expect("the stream stays alive after a gap")
        .expect("stream yields");
    assert_eq!(
        next,
        IntentSubscriptionEvent::Changed {
            key: Bytes::from_static(b"jobs/after-the-gap"),
            value: Some(Bytes::from_static(b"spec-after")),
        },
    );
}
