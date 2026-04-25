//! Acceptance scenarios for `IntentStore::put_if_absent` against
//! `LocalIntentStore` — the atomic compare-and-set primitive that the
//! control-plane `submit_job` handler relies on to close the TOCTOU
//! race between idempotent re-submit and conflicting-spec detection.
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
    let inserted_idx = match outcome {
        PutOutcome::Inserted { commit_index } => commit_index,
        PutOutcome::KeyExists { existing, commit_index } => panic!(
            "expected Inserted on empty key; got KeyExists {{ existing: {existing:?}, commit_index: {commit_index} }}"
        ),
    };
    assert!(inserted_idx >= 1, "Inserted on empty key must assign a commit_index >= 1");

    let read = store.get(key).await.expect("get");
    assert_eq!(read, Some((Bytes::copy_from_slice(value), inserted_idx)));
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

    // Capture the seed put's index via a follow-up `get` so we can
    // assert KeyExists carries the SAME index — the per-entry contract.
    let (_, seed_idx) = store.get(key).await.expect("get post-seed").expect("seed must be present");

    let outcome = store.put_if_absent(key, b"would-have-clobbered").await.expect("put_if_absent");
    match outcome {
        PutOutcome::KeyExists { existing, commit_index } => {
            assert_eq!(
                existing.as_ref(),
                first,
                "KeyExists must carry the incumbent bytes so the caller can compare",
            );
            assert_eq!(
                commit_index, seed_idx,
                "KeyExists.commit_index must carry the prior write's per-entry \
                 index ({seed_idx}); got {commit_index}",
            );
        }
        PutOutcome::Inserted { commit_index } => panic!(
            "expected KeyExists on populated key; got Inserted {{ commit_index: {commit_index} }}"
        ),
    }

    // The underlying store must still hold the original bytes — the
    // losing payload never touched the key. The per-entry index also
    // remains the seed's index.
    let read = store.get(key).await.expect("get");
    assert_eq!(
        read,
        Some((Bytes::copy_from_slice(first), seed_idx)),
        "`put_if_absent` on a populated key must be a no-op write; \
         the incumbent bytes AND per-entry index must be untouched",
    );
}

// -----------------------------------------------------------------------------
// Scenario 3 — `put_if_absent` does NOT bump `commit_index` on the
// `KeyExists` branch. This is what makes concurrent idempotent
// re-submissions through the handler return the SAME index N times.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn put_if_absent_does_not_advance_commit_index_on_key_exists() {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    let key: &[u8] = b"jobs/payments";
    store.put(key, b"seed").await.expect("seed put");
    let after_seed = store.commit_index();

    // Capture the seed's per-entry index so every KeyExists in the
    // loop can be asserted to carry it.
    let (_, seed_idx) = store.get(key).await.expect("get post-seed").expect("seed must be present");

    // Every no-op put_if_absent must leave the counter alone AND
    // return the seed's per-entry index.
    for _ in 0..5 {
        let outcome =
            store.put_if_absent(key, b"would-have-clobbered").await.expect("put_if_absent");
        match outcome {
            PutOutcome::KeyExists { commit_index, .. } => {
                assert_eq!(
                    commit_index, seed_idx,
                    "every KeyExists must carry the seed's per-entry index ({seed_idx}); \
                     got {commit_index} — drift here means the index is being recomputed \
                     rather than read from the stored row",
                );
            }
            PutOutcome::Inserted { commit_index } => panic!(
                "every attempt against a populated key must return KeyExists; got \
                 Inserted {{ commit_index: {commit_index} }}"
            ),
        }
    }
    assert_eq!(
        store.commit_index(),
        after_seed,
        "commit_index must NOT advance on KeyExists — a bump here would surface \
         to the handler as a spurious commit_index drift across idempotent submits",
    );
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

    let mut winners: Vec<(Vec<u8>, u64)> = Vec::new();
    let mut losers_saw_incumbent: Vec<(Bytes, u64)> = Vec::new();
    while let Some(res) = set.join_next().await {
        let (value, outcome) = res.expect("join task");
        match outcome {
            PutOutcome::Inserted { commit_index } => winners.push((value, commit_index)),
            PutOutcome::KeyExists { existing, commit_index } => {
                losers_saw_incumbent.push((existing, commit_index));
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

    // Every loser must see the winner's bytes AND the winner's
    // per-entry commit_index — the per-entry contract: KeyExists
    // reflects the prior Inserted's index.
    let (winning_value, winning_idx) = winners[0].clone();
    assert_eq!(
        losers_saw_incumbent.len(),
        (N as usize) - 1,
        "every non-winner must return KeyExists",
    );
    for (existing, loser_idx) in &losers_saw_incumbent {
        assert_eq!(
            existing.as_ref(),
            winning_value.as_slice(),
            "every loser must observe the winner's bytes via KeyExists",
        );
        assert_eq!(
            *loser_idx, winning_idx,
            "every loser's KeyExists.commit_index must equal the winner's \
             Inserted.commit_index ({winning_idx}); got {loser_idx} — this \
             is the per-entry-index contract under contention",
        );
    }

    // And the store must hold byte-exactly the winner's bytes at the
    // winner's per-entry index.
    let read = store.get(key).await.expect("get");
    assert_eq!(read, Some((Bytes::from(winning_value), winning_idx)));
}

// -----------------------------------------------------------------------------
// Scenario 5 — `Inserted` bumps `commit_index` exactly once. The
// counter-advance contract is the same as `put`; handlers that
// surface `commit_index` on write responses depend on it.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn put_if_absent_inserted_branch_advances_commit_index_once() {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    let before = store.commit_index();
    let outcome = store.put_if_absent(b"jobs/payments", b"v1").await.expect("put_if_absent");
    let inserted_idx = match outcome {
        PutOutcome::Inserted { commit_index } => commit_index,
        PutOutcome::KeyExists { .. } => panic!("expected Inserted on empty key"),
    };
    assert_eq!(
        store.commit_index(),
        before + 1,
        "Inserted branch must bump commit_index by exactly 1 (same contract as `put`)",
    );
    // The per-entry index returned by Inserted must equal the
    // post-bump global counter — both bump-and-capture happens inside
    // the same write transaction.
    assert_eq!(
        inserted_idx,
        before + 1,
        "Inserted.commit_index must equal the post-bump global counter ({}); got {inserted_idx}",
        before + 1,
    );
}
