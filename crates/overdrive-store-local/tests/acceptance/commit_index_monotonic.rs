//! Acceptance tests for `LocalIntentStore::commit_index` — the
//! monotonic in-memory counter bumped on every successful write.
//!
//! Per ADR-0015 the value flows onto `POST /v1/jobs` responses as
//! `commit_index`, and `GET /v1/jobs/{id}` renders it inside
//! `JobDescription.commit_index`. A mutation that makes `commit_index`
//! return a constant (`0` or `1`) — or that turns `bump_commit` into
//! `()` — would leak onto the wire as a frozen counter, breaking
//! idempotency / conflict-detection contracts downstream.
//!
//! The integration suite catches this through full HTTP round-trips,
//! but that lane is gated by `integration-tests`. These acceptance
//! tests exercise the same property directly against `LocalIntentStore`
//! (`TempDir` + redb, no network) so the counter contract is pinned in
//! the default lane.

#![allow(clippy::expect_used)]

use bytes::Bytes;
use overdrive_core::traits::intent_store::{IntentStore, TxnOp, TxnOutcome};
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

fn store() -> (LocalIntentStore, TempDir) {
    let tmp = TempDir::new().expect("TempDir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");
    (store, tmp)
}

// ---------------------------------------------------------------------------
// Fresh store: commit_index is zero. A mutation that hard-codes `1` is
// immediately caught.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fresh_store_has_commit_index_zero() {
    let (store, _tmp) = store();
    assert_eq!(
        store.commit_index(),
        0,
        "a freshly-opened LocalIntentStore must have commit_index 0 — a \
         mutation that returns `1` as a constant is caught here",
    );
}

// ---------------------------------------------------------------------------
// Single put advances commit_index by exactly 1. Kills the mutation
// that returns `0` as a constant AND the mutation that turns
// `bump_commit` into `()`.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn put_advances_commit_index_by_exactly_one() {
    let (store, _tmp) = store();
    assert_eq!(store.commit_index(), 0);

    store.put(b"jobs/payments", b"spec-v1").await.expect("put");
    assert_eq!(
        store.commit_index(),
        1,
        "after a single successful put the counter must be exactly 1; \
         a mutation of bump_commit to `()` would leave it at 0",
    );
}

// ---------------------------------------------------------------------------
// N puts strictly increment the counter by N — the canonical signal
// that the counter is monotonic and that every put calls bump_commit
// once. Mutation `commit_index -> 0/1` fails. Mutation `bump_commit ->
// ()` fails.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn n_puts_advance_commit_index_by_exactly_n() {
    const N: u64 = 5;

    let (store, _tmp) = store();
    let starting = store.commit_index();
    assert_eq!(starting, 0);

    for i in 0..N {
        let key = format!("jobs/payments-{i}");
        let value = format!("spec-{i}");
        store.put(key.as_bytes(), value.as_bytes()).await.expect("put");
    }
    assert_eq!(
        store.commit_index(),
        starting + N,
        "N successive puts must advance commit_index by exactly N; \
         got {} after {N} puts — a constant-return mutation or a \
         bump_commit-to-nop mutation would drift this",
        store.commit_index(),
    );
}

// ---------------------------------------------------------------------------
// Each put advances commit_index by one at a time — not "batch" and
// not "all at once at the end." This distinguishes a mutation where
// bump_commit does something wrong (e.g. add 2, add N) from the
// original behaviour.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn commit_index_advances_once_per_put_stepwise() {
    let (store, _tmp) = store();

    let mut expected = store.commit_index();
    for i in 0..4_u64 {
        let key = format!("jobs/payments-{i}");
        store.put(key.as_bytes(), b"spec").await.expect("put");
        expected += 1;
        assert_eq!(
            store.commit_index(),
            expected,
            "after put #{i} the commit_index must be {expected}; got {} \
             — a bump_commit that fires multiple times or not at all \
             would drift from the stepwise expectation",
            store.commit_index(),
        );
    }
}

// ---------------------------------------------------------------------------
// delete() also bumps commit_index.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_advances_commit_index() {
    let (store, _tmp) = store();
    store.put(b"jobs/payments", b"spec").await.expect("put");
    let before_delete = store.commit_index();
    assert_eq!(before_delete, 1);

    store.delete(b"jobs/payments").await.expect("delete");
    assert_eq!(
        store.commit_index(),
        before_delete + 1,
        "delete must also bump commit_index — a mutation in bump_commit \
         would surface here even if the put path remained correct",
    );
}

// ---------------------------------------------------------------------------
// txn() bumps commit_index once per successful transaction — not
// once per op inside the transaction.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn txn_commit_bumps_commit_index_exactly_once() {
    let (store, _tmp) = store();
    let starting = store.commit_index();

    let outcome = store
        .txn(vec![
            TxnOp::Put { key: Bytes::from_static(b"jobs/a"), value: Bytes::from_static(b"spec-a") },
            TxnOp::Put { key: Bytes::from_static(b"jobs/b"), value: Bytes::from_static(b"spec-b") },
            TxnOp::Put { key: Bytes::from_static(b"jobs/c"), value: Bytes::from_static(b"spec-c") },
        ])
        .await
        .expect("txn");

    assert!(matches!(outcome, TxnOutcome::Committed), "txn must commit — got {outcome:?}");
    assert_eq!(
        store.commit_index(),
        starting + 1,
        "a single successful txn must bump commit_index by exactly 1 \
         regardless of how many ops it contains",
    );
}
