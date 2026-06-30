//! Acceptance scenarios for `TxnOp::IncrementU64` against
//! `LocalIntentStore` — the atomic-monotonic read-modify-write store
//! primitive that the `restart_workload` handler relies on to bump a
//! workload's desired-run generation and clear its stop sentinel in ONE
//! atomic transaction (ADR-0073 § "The six pinned signatures" item 4).
//!
//! The original draft had the handler `get` the generation, compute
//! `g+1`, blind-`Put` it, and retry on `Conflict`. That was a
//! correctness blocker: `TxnOp` has only blind `Put`/`Delete` and
//! `LocalIntentStore::txn` returns `Committed` unconditionally — two
//! concurrent restarts both read 0, both blind-Put 1, one bump silently
//! lost; a stale read can drive the generation BACKWARDS. The fix is a
//! read-modify variant performing the increment INSIDE the store's
//! write transaction (development.md § "Check-and-act must be atomic").
//!
//! Port-to-port discipline: every assertion drives the `IntentStore`
//! trait surface (`store.txn` / `store.get`). No internal redb types
//! are inspected. The value encoding observed at the port is the 8-byte
//! big-endian `u64` the contract pins.
//!
//! Strategy C per DWD-01: real redb, `tempfile::TempDir` backing path.

use std::sync::Arc;

use bytes::Bytes;
use overdrive_core::traits::intent_store::{IntentStore, TxnOp, TxnOutcome};
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

/// Decode a port-observed value as a big-endian `u64`, defending the
/// short-slice case per development.md § "Safe byte-slice access":
/// absent or non-8-byte values read as `0`. This is the SAME decode
/// the contract mandates the production arm perform — asserting on it
/// at the port is the observable surface for the generation value.
fn decode_be_u64(value: Option<Bytes>) -> u64 {
    value.and_then(|bytes| <[u8; 8]>::try_from(bytes.as_ref()).ok()).map_or(0, u64::from_be_bytes)
}

// -----------------------------------------------------------------------------
// S-BIR-TXN-01 — one atomic txn bumps the generation (absent ⇒ 1) AND
// clears the present stop sentinel. The atomicity contract: no observer
// sees the gen bumped without the stop cleared, or vice versa.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn single_txn_bumps_generation_and_clears_present_stop_sentinel() {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    let gen_key: &[u8] = b"workloads/payments/generation";
    let stop_key: &[u8] = b"workloads/payments/stop";

    // Precondition: no generation key, a present stop sentinel (the
    // stopped-origin restart shape).
    store.put(stop_key, b"").await.expect("seed stop sentinel");

    let outcome = store
        .txn(vec![
            TxnOp::IncrementU64 { key: Bytes::copy_from_slice(gen_key) },
            TxnOp::Delete { key: Bytes::copy_from_slice(stop_key) },
        ])
        .await
        .expect("txn");

    assert!(
        matches!(outcome, TxnOutcome::Committed),
        "the restart txn must commit unconditionally (no Conflict path); got {outcome:?}",
    );

    // The generation bumped from absent (0) to exactly 1.
    let generation = decode_be_u64(store.get(gen_key).await.expect("get gen"));
    assert_eq!(generation, 1, "absent generation must bump to exactly 1");

    // The stop sentinel is cleared — atomically with the bump.
    let stop = store.get(stop_key).await.expect("get stop");
    assert_eq!(stop, None, "the stop sentinel must be cleared by the same atomic txn as the bump");
}

// -----------------------------------------------------------------------------
// S-BIR-TXN-02 — THE load-bearing concurrency proof (ADR-0073 § item 4).
// N concurrent restart txns against the SAME store: every txn commits,
// the final generation is EXACTLY N (no lost bump — redb serialises
// writers so each read-modify-write sees the prior committed value), and
// the value never went backwards. This is the @property @concurrency
// proof, NOT a single example.
//
// This is the test the mutation gate hangs on: a `+1 → +0` mutation on
// the increment arm makes the final value 0 (or any value < N), which
// this assertion kills.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn concurrent_restart_txns_never_lose_a_bump_final_generation_equals_n() {
    const N: u64 = 32;

    let tmp = TempDir::new().expect("temp dir");
    let store = Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open"));

    let gen_key: &[u8] = b"workloads/payments/generation";
    let stop_key: &[u8] = b"workloads/payments/stop";

    let mut set = tokio::task::JoinSet::new();
    for _ in 0..N {
        let store = Arc::clone(&store);
        let gen_key = Bytes::copy_from_slice(gen_key);
        let stop_key = Bytes::copy_from_slice(stop_key);
        set.spawn(async move {
            store
                .txn(vec![TxnOp::IncrementU64 { key: gen_key }, TxnOp::Delete { key: stop_key }])
                .await
                .expect("txn")
        });
    }

    // Every concurrent txn must report Committed.
    let mut committed = 0u64;
    while let Some(res) = set.join_next().await {
        let outcome = res.expect("join task");
        assert!(
            matches!(outcome, TxnOutcome::Committed),
            "every concurrent restart txn must commit; got {outcome:?}",
        );
        committed += 1;
    }
    assert_eq!(committed, N, "all N txns must have committed");

    // The final generation equals N — no bump lost. A `+1 → +0`
    // mutation, a stale-snapshot read, or a non-serialised
    // read-modify-write would land here as a value < N.
    let generation = decode_be_u64(store.get(gen_key).await.expect("get gen"));
    assert_eq!(
        generation, N,
        "final generation must equal the count of committed restart txns ({N}) — \
         a value below {N} means a concurrent bump was lost (the non-atomic \
         get-then-Put failure shape this primitive exists to prevent)",
    );
}

// -----------------------------------------------------------------------------
// S-BIR-TXN-03 — the absent-key edge. Neither a generation key nor a
// stop sentinel exists; the restart txn commits, the gen bumps to 1
// (absent ⇒ 0 then +1), and the Delete of the already-absent stop_key
// is a no-op (Committed, not an error — the running-origin restart
// deletes a /stop that was never written).
// -----------------------------------------------------------------------------

#[tokio::test]
async fn absent_keys_bump_to_one_and_delete_of_absent_stop_is_a_noop() {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    let gen_key: &[u8] = b"workloads/payments/generation";
    let stop_key: &[u8] = b"workloads/payments/stop";

    // Precondition: NEITHER key exists (the running-origin restart).
    assert_eq!(store.get(gen_key).await.expect("get gen"), None, "precondition: gen absent");
    assert_eq!(store.get(stop_key).await.expect("get stop"), None, "precondition: stop absent");

    let outcome = store
        .txn(vec![
            TxnOp::IncrementU64 { key: Bytes::copy_from_slice(gen_key) },
            TxnOp::Delete { key: Bytes::copy_from_slice(stop_key) },
        ])
        .await
        .expect("txn");

    assert!(
        matches!(outcome, TxnOutcome::Committed),
        "deleting an already-absent stop sentinel is a no-op, not an error — the txn commits; \
         got {outcome:?}",
    );

    let generation = decode_be_u64(store.get(gen_key).await.expect("get gen"));
    assert_eq!(generation, 1, "absent generation must bump to exactly 1");
    assert_eq!(
        store.get(stop_key).await.expect("get stop"),
        None,
        "the absent stop key must remain absent after a no-op delete",
    );
}

// -----------------------------------------------------------------------------
// S-BIR-TXN-04 — the corrupt-row edge per development.md § "Safe
// byte-slice access". A generation key holding a 3-byte (non-8) value:
// the txn commits, the BE-u64 read defends against the short slice
// (decodes to 0 via a length-checked decode, NEVER `bytes[0..8]`
// indexing, NEVER a panic), and the post-state gen decodes to 1.
// -----------------------------------------------------------------------------

#[tokio::test]
async fn corrupt_short_row_decodes_as_zero_then_bumps_to_one() {
    let tmp = TempDir::new().expect("temp dir");
    let store = LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open");

    let gen_key: &[u8] = b"workloads/payments/generation";

    // Seed a corrupt 3-byte value at the generation key — not a valid
    // 8-byte BE u64. The read path must treat this as 0, never panic.
    store.put(gen_key, &[0xAA, 0xBB, 0xCC]).await.expect("seed corrupt gen");

    let outcome = store
        .txn(vec![TxnOp::IncrementU64 { key: Bytes::copy_from_slice(gen_key) }])
        .await
        .expect("txn");

    assert!(
        matches!(outcome, TxnOutcome::Committed),
        "a corrupt (short) generation row must not wedge the bump; the txn commits; got {outcome:?}",
    );

    // The short slice decoded as 0, then +1 = 1; the write path emits
    // canonical 8-byte BE so the post-state is now a valid 8-byte value.
    let read = store.get(gen_key).await.expect("get gen");
    assert_eq!(
        read.as_ref().map(bytes::Bytes::len),
        Some(8),
        "the write path must always emit a canonical 8-byte BE value, healing the corrupt row",
    );
    assert_eq!(
        decode_be_u64(read),
        1,
        "a corrupt short row decodes as 0; after the bump the generation is exactly 1",
    );
}
