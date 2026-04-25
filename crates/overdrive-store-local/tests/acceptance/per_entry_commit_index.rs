//! Acceptance tests pinning the per-entry `commit_index` contract at
//! the `IntentStore` trait surface.
//!
//! The RCA (`docs/feature/fix-commit-index-per-entry/deliver/rca.md`)
//! identifies two trait-surface gaps:
//!
//! - WHY 5A — `IntentStore::get` returns `Option<Bytes>` only; no per-entry
//!   index is carried back to the caller, so handlers cannot return the
//!   index at which the entry was written.
//! - WHY 5B — `PutOutcome::Inserted` is unit-like; the index assigned
//!   inside the write transaction is not surfaced to the caller, so the
//!   submit handler reads `commit_index()` on the calling task *after*
//!   `put_if_absent` returns, racing with concurrent committers for
//!   different keys.
//!
//! These tests pin the FUTURE trait surface — Step 01-02 lands the
//! additive change. Per `.claude/rules/testing.md` §"RED scaffolds and
//! intentionally-failing commits", this file is intentionally
//! NON-COMPILING at this commit:
//!
//! - `PutOutcome::Inserted { commit_index }` is the future variant
//!   shape; current HEAD has unit-like `PutOutcome::Inserted`.
//! - `IntentStore::get -> Option<(Bytes, u64)>` is the future return
//!   type; current HEAD returns `Option<Bytes>`.
//!
//! `cargo check -p overdrive-store-local --features integration-tests
//! --tests` must fail with errors referencing these two future shapes.
//! That compile-time failure IS the RED signal — Step 01-02 makes it
//! compile and pass.

#![allow(clippy::expect_used)]

use overdrive_core::traits::intent_store::{IntentStore, PutOutcome};
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

fn store() -> (LocalIntentStore, TempDir) {
    let tmp = TempDir::new().expect("TempDir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");
    (store, tmp)
}

// ---------------------------------------------------------------------------
// `PutOutcome::Inserted` carries the index assigned inside the write
// transaction. Two consecutive distinct-key writes return DIFFERENT
// `commit_index` values via `Inserted` — the second is strictly greater
// than the first.
//
// This pins RCA WHY 5B — the atomic compare-and-set must surface the
// per-write index back to the caller, not leave the handler to read
// `commit_index()` separately on the calling task (which races).
//
// Future trait shape (Step 01-02):
//   PutOutcome::Inserted { commit_index: u64 }
//
// Current HEAD (Step 01-01):
//   PutOutcome::Inserted   ← unit-like, no field
//
// This test will fail to compile against current HEAD because the
// pattern destructures `commit_index` from a unit-like variant. That
// compile-time failure IS the RED signal.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn put_if_absent_returns_commit_index_assigned_in_txn() {
    let (store, _tmp) = store();

    // First put — distinct key.
    let outcome_a =
        store.put_if_absent(b"jobs/payments", b"spec-a").await.expect("put_if_absent A");
    let idx_a = match outcome_a {
        PutOutcome::Inserted { commit_index } => commit_index,
        PutOutcome::KeyExists { .. } => panic!("expected Inserted on empty key A"),
    };

    // Second put — DIFFERENT key. Both writes succeed via `Inserted`,
    // but each must carry the index assigned in its own write txn.
    let outcome_b =
        store.put_if_absent(b"jobs/frontend", b"spec-b").await.expect("put_if_absent B");
    let idx_b = match outcome_b {
        PutOutcome::Inserted { commit_index } => commit_index,
        PutOutcome::KeyExists { .. } => panic!("expected Inserted on empty key B"),
    };

    // The per-entry contract: each Inserted carries the index assigned
    // INSIDE its write txn. A handler that reads `commit_index()` on the
    // calling task after `put_if_absent` returns sees a counter that may
    // have been bumped by an interleaved write to a different key — it
    // does not see the index attached to ITS bytes. Returning the index
    // via `PutOutcome::Inserted` closes that race.
    assert!(
        idx_b > idx_a,
        "PutOutcome::Inserted must carry the index assigned inside its \
         write txn; consecutive distinct-key writes must produce strictly \
         increasing indices. Got idx_a={idx_a}, idx_b={idx_b}.",
    );
}

// ---------------------------------------------------------------------------
// `IntentStore::get` returns `(value, index)` — the index AT WHICH the
// entry was written, not the live store counter at read-time.
//
// This pins RCA WHY 5A — the trait must expose a `(value, index)` read
// primitive so `describe_job` can return the per-entry index per the
// `JobDescription` rustdoc contract ("the commit index at which it was
// written").
//
// Future trait shape (Step 01-02):
//   async fn get(&self, key: &[u8]) -> Result<Option<(Bytes, u64)>, _>
//
// Current HEAD (Step 01-01):
//   async fn get(&self, key: &[u8]) -> Result<Option<Bytes>, _>
//
// This test will fail to compile against current HEAD because the
// tuple destructure expects `Some((bytes, idx))` from a return of
// `Option<Bytes>`. That compile-time failure IS the RED signal.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_returns_index_at_which_entry_was_written() {
    let (store, _tmp) = store();

    // Write key1 first — capture the index assigned in its txn via
    // PutOutcome::Inserted.
    let outcome_1 = store.put_if_absent(b"jobs/payments", b"spec-1").await.expect("put 1");
    let idx_1 = match outcome_1 {
        PutOutcome::Inserted { commit_index } => commit_index,
        PutOutcome::KeyExists { .. } => panic!("expected Inserted on empty key 1"),
    };

    // Write key2 after — this advances the live store counter past
    // idx_1. The bug shape this test pins: `get(key1)` must NOT return
    // the live counter (which has advanced past idx_1); it must return
    // the index at which key1's bytes were committed.
    let outcome_2 = store.put_if_absent(b"jobs/frontend", b"spec-2").await.expect("put 2");
    let idx_2 = match outcome_2 {
        PutOutcome::Inserted { commit_index } => commit_index,
        PutOutcome::KeyExists { .. } => panic!("expected Inserted on empty key 2"),
    };
    assert!(idx_2 > idx_1, "idx_2 ({idx_2}) must exceed idx_1 ({idx_1})");

    // Read key1 back — the trait must surface `(value, index)` where
    // `index` is the per-entry index from key1's write txn, not the
    // live store counter.
    let read = store.get(b"jobs/payments").await.expect("get key1");
    let (bytes, idx_1_from_get) =
        read.expect("jobs/payments must be present after put_if_absent::Inserted");

    assert_eq!(
        bytes.as_ref(),
        b"spec-1".as_slice(),
        "get must return the bytes that were written at key1",
    );

    assert_eq!(
        idx_1_from_get, idx_1,
        "IntentStore::get must return the per-entry index assigned at \
         write-time ({idx_1}), NOT the live store counter ({idx_2}); a \
         handler that surfaces this index satisfies the JobDescription \
         rustdoc contract \"the commit index at which it was written\".",
    );
}
