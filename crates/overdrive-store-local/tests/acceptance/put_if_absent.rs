//! Acceptance scenarios for `IntentStore::put_if_absent` against
//! `LocalIntentStore` — the atomic compare-and-set primitive that the
//! control-plane `submit_job` handler relies on to close the TOCTOU
//! race between idempotent re-submit and conflicting-spec detection.
//!
//! Per ADR-0020 (drop `commit_index` from Phase 1) the `PutOutcome`
//! variants are unit-like (`Inserted`) / single-field (`KeyExists {
//! existing }`); the per-entry index column was dropped. See
//! `redesign-drop-commit-index/design/upstream-changes.md` §7.
//!
//! Port-to-port discipline: every assertion drives the `IntentStore`
//! trait surface. No internal redb types are inspected.
//!
//! Strategy C per DWD-01: real redb, `tempfile::TempDir` backing path.

use std::sync::Arc;

use bytes::Bytes;
use overdrive_core::traits::intent_store::{IntentStore, PutOutcome};
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

// -----------------------------------------------------------------------------
// Scenario 1 — `put_if_absent` on an empty key inserts and reports
// `Inserted`, and the value is observable via `get`.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn put_if_absent_on_empty_key_inserts_and_reports_inserted() {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    let key: &[u8] = b"jobs/payments";
    let value: &[u8] = b"spec-bytes-v1";

    let outcome = store.put_if_absent(key, value).await.expect("put_if_absent");
    match outcome {
        PutOutcome::Inserted => {}
        PutOutcome::KeyExists { existing } => {
            panic!("expected Inserted on empty key; got KeyExists {{ existing: {existing:?} }}")
        }
    }

    let read = store.get(key).await.expect("get");
    assert_eq!(read, Some(Bytes::copy_from_slice(value)));
}

// -----------------------------------------------------------------------------
// Scenario 2 — `put_if_absent` on a populated key returns `KeyExists`
// carrying the incumbent bytes and does NOT overwrite.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn put_if_absent_on_populated_key_returns_existing_bytes_and_does_not_overwrite() {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    let key: &[u8] = b"jobs/payments";
    let first: &[u8] = b"incumbent-bytes";
    store.put(key, first).await.expect("seed put");

    let outcome = store.put_if_absent(key, b"would-have-clobbered").await.expect("put_if_absent");
    match outcome {
        PutOutcome::KeyExists { existing } => {
            assert_eq!(
                existing.as_ref(),
                first,
                "KeyExists must carry the incumbent bytes so the caller can compare",
            );
        }
        PutOutcome::Inserted => {
            panic!("expected KeyExists on populated key; got Inserted")
        }
    }

    // The underlying store must still hold the original bytes — the
    // losing payload never touched the key.
    let read = store.get(key).await.expect("get");
    assert_eq!(
        read,
        Some(Bytes::copy_from_slice(first)),
        "`put_if_absent` on a populated key must be a no-op write; \
         the incumbent bytes must be untouched",
    );
}

// -----------------------------------------------------------------------------
// Scenario 3 — repeated `put_if_absent` against a populated key always
// returns `KeyExists` with the same incumbent bytes — the idempotency
// witness handlers rely on (no overwrite, no drift).
// -----------------------------------------------------------------------------

#[tokio::test]
async fn put_if_absent_repeated_calls_against_populated_key_always_return_key_exists() {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    let key: &[u8] = b"jobs/payments";
    store.put(key, b"seed").await.expect("seed put");

    // Every no-op put_if_absent must return the seed's bytes.
    for _ in 0..5 {
        let outcome =
            store.put_if_absent(key, b"would-have-clobbered").await.expect("put_if_absent");
        match outcome {
            PutOutcome::KeyExists { existing } => {
                assert_eq!(
                    existing.as_ref(),
                    b"seed".as_slice(),
                    "KeyExists must carry the seed bytes verbatim — drift here means \
                     the incumbent is being overwritten or the read is racing the write",
                );
            }
            PutOutcome::Inserted => {
                panic!("every attempt against a populated key must return KeyExists")
            }
        }
    }

    // The store still holds the seed bytes after the loop.
    let read = store.get(key).await.expect("get");
    assert_eq!(read, Some(Bytes::copy_from_slice(b"seed")));
}

// -----------------------------------------------------------------------------
// Scenario 4 — concurrent `put_if_absent` for the same key with
// DIFFERENT values: exactly one winner, every other caller receives
// the winner's bytes via `KeyExists`. This is the atomicity property
// the TOCTOU fix depends on.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn concurrent_put_if_absent_same_key_different_values_yields_exactly_one_winner() {
    const N: u32 = 8;

    let tmp = TempDir::new().expect("temp dir");
    let store = Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open"));

    let key: &[u8] = b"jobs/payments";

    // Each task tries to put a distinct value under the same key.
    let mut set = tokio::task::JoinSet::new();
    for i in 0..N {
        let store = Arc::clone(&store);
        let value = format!("attempt-{i}").into_bytes();
        set.spawn(async move {
            let outcome = store.put_if_absent(key, &value).await.expect("put_if_absent");
            (value, outcome)
        });
    }

    let mut winners: Vec<Vec<u8>> = Vec::new();
    let mut losers_saw_incumbent: Vec<Bytes> = Vec::new();
    while let Some(res) = set.join_next().await {
        let (value, outcome) = res.expect("join task");
        match outcome {
            PutOutcome::Inserted => winners.push(value),
            PutOutcome::KeyExists { existing } => {
                losers_saw_incumbent.push(existing);
            }
        }
    }

    // Exactly one winner — this is the atomicity invariant. The test
    // would fail under a naive non-atomic `get + put` pair if two
    // tasks happened to read before either wrote.
    assert_eq!(
        winners.len(),
        1,
        "exactly one concurrent put_if_absent must report Inserted; got {} \
         winners — this is the TOCTOU failure shape the atomic primitive exists to prevent. \
         winners = {winners:?}",
        winners.len(),
    );

    // Every loser must see the winner's bytes.
    let winning_value = winners[0].clone();
    assert_eq!(
        losers_saw_incumbent.len(),
        (N as usize) - 1,
        "every non-winner must return KeyExists",
    );
    for existing in &losers_saw_incumbent {
        assert_eq!(
            existing.as_ref(),
            winning_value.as_slice(),
            "every loser must observe the winner's bytes via KeyExists",
        );
    }

    // And the store must hold byte-exactly the winner's bytes.
    let read = store.get(key).await.expect("get");
    assert_eq!(read, Some(Bytes::from(winning_value)));
}
