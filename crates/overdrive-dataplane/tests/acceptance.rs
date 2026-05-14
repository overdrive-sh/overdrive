//! Acceptance tests for the `service-vip-allocator` feature.
//!
//! Phase 1 step 01-03 (S-VIP-05, S-VIP-10, S-VIP-20) — covers
//! [`PersistentServiceVipAllocator`] persistence-roundtrip,
//! no-partial-state-on-exhaustion, and idempotent-release scenarios
//! against a real `LocalIntentStore` over a `tempfile::TempDir`.
//!
//! Real-infrastructure tests (redb file I/O) gated behind the
//! `integration-tests` feature per
//! `.claude/rules/testing.md` § "Integration vs unit gating".
//!
//! [`PersistentServiceVipAllocator`]:
//!   overdrive_dataplane::allocators::PersistentServiceVipAllocator

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, clippy::expect_fun_call)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use overdrive_dataplane::allocators::{
    PersistentAllocatorError, PersistentServiceVipAllocator, ServiceSpecDigest, ServiceVip,
    ServiceVipAllocatorError, VipRange,
};
use overdrive_store_local::{IntentStore, LocalIntentStore};

/// Build a fresh redb-backed `LocalIntentStore` rooted in a tempdir.
/// The returned `(Arc<dyn IntentStore>, TempDir)` pair keeps the
/// tempdir alive for the test's lifetime.
fn fresh_store() -> (Arc<dyn IntentStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("create tempdir");
    let path = dir.path().join("intent.redb");
    let store = LocalIntentStore::open(&path).expect("open LocalIntentStore");
    (Arc::new(store), dir)
}

/// Reopen the on-disk `LocalIntentStore` at the tempdir's path.
fn reopen_store(dir: &tempfile::TempDir) -> Arc<dyn IntentStore> {
    let path = dir.path().join("intent.redb");
    let store = LocalIntentStore::open(&path).expect("reopen LocalIntentStore");
    Arc::new(store)
}

/// Operator-configured range `10.96.0.0/30` with the network and
/// broadcast addresses reserved — yields a capacity of 2 (.1, .2).
fn small_range() -> VipRange {
    use std::collections::BTreeSet;
    let cidr: ipnet::Ipv4Net = "10.96.0.0/30".parse().expect("valid cidr");
    let reserved: BTreeSet<Ipv4Addr> =
        [Ipv4Addr::new(10, 96, 0, 0), Ipv4Addr::new(10, 96, 0, 3)].into_iter().collect();
    VipRange::new(vec![cidr], reserved).expect("valid VipRange")
}

/// Wider range `10.96.0.0/24` for tests that need many slots.
fn wide_range() -> VipRange {
    use std::collections::BTreeSet;
    let cidr: ipnet::Ipv4Net = "10.96.0.0/24".parse().expect("valid cidr");
    VipRange::new(vec![cidr], BTreeSet::new()).expect("valid VipRange")
}

const fn digest(seed: u8) -> ServiceSpecDigest {
    [seed; 32]
}

// ---------------------------------------------------------------------------
// S-VIP-05 — persistence_across_restart
// ---------------------------------------------------------------------------

/// Allocate `V` for digest `D`, persist to a real `LocalIntentStore`,
/// then construct a fresh `PersistentServiceVipAllocator` via
/// `bulk_load` against the same on-disk store. Assert that `get(&D)`
/// returns `Some(V)` and `allocate(D)` returns the same `V` (memo
/// hit; counter unchanged).
#[tokio::test]
async fn persistence_across_restart() {
    let (store, dir) = fresh_store();

    // First boot — allocate.
    let mut allocator = PersistentServiceVipAllocator::new(wide_range(), Arc::clone(&store));
    let d = digest(0xAB);
    let vip = allocator.allocate(d).await.expect("allocate succeeds");

    // Sanity: in-memory state reflects the allocation.
    assert_eq!(allocator.get(&d), Some(vip), "memo populated after allocate");

    // Drop the allocator AND the store — simulate process restart.
    drop(allocator);
    drop(store);

    // Reopen the store and bulk-load the allocator.
    let store_reopened = reopen_store(&dir);
    let mut reloaded =
        PersistentServiceVipAllocator::bulk_load(wide_range(), Arc::clone(&store_reopened))
            .await
            .expect("bulk_load succeeds");

    // Assertion 1: the in-memory state was reconstructed from disk.
    assert_eq!(
        reloaded.get(&d),
        Some(vip),
        "bulk_load reconstructs (digest, vip) memo from persisted entries",
    );

    // Assertion 2: allocate(same digest) is a memo-hit returning the
    // same VIP — counter was correctly reconstructed at
    // max(counter_idx) + 1 so this short-circuits without advancing.
    let vip_again = reloaded.allocate(d).await.expect("memo-hit allocate succeeds");
    assert_eq!(vip_again, vip, "memo-hit allocate returns the same VIP");
}

// ---------------------------------------------------------------------------
// S-VIP-10 — no_partial_state_on_exhaustion
// ---------------------------------------------------------------------------

/// Allocate to exhaustion against a small range, then attempt one
/// further allocation. Assert that:
///   1. The failed allocation returns `Err(Exhausted)`.
///   2. `get(&new_digest)` returns `None` — no partial in-memory state.
///   3. `get(&existing_digest)` still returns the prior VIP.
///   4. No partial entry was written to the store — the row count
///      under the allocator prefix equals the allocator's capacity,
///      NOT capacity + 1.
#[tokio::test]
async fn no_partial_state_on_exhaustion() {
    let (store, _dir) = fresh_store();
    let range = small_range();
    let capacity = range.capacity();
    assert_eq!(capacity, 2, "small_range has capacity 2");

    let mut allocator = PersistentServiceVipAllocator::new(range, Arc::clone(&store));

    let d_first = digest(0x01);
    let d_second = digest(0x02);
    let d_third = digest(0x03); // will fail — pool exhausted at 2

    let vip_first = allocator.allocate(d_first).await.expect("first allocate succeeds");
    let _vip_second = allocator.allocate(d_second).await.expect("second allocate succeeds");

    // Third allocation must fail — capacity is 2 and we've issued 2.
    let err = allocator.allocate(d_third).await.expect_err("third allocate must fail (exhausted)");
    match err {
        PersistentAllocatorError::Allocator(ServiceVipAllocatorError::Exhausted {
            allocated,
            capacity: cap,
        }) => {
            assert_eq!(allocated, 2, "Exhausted carries current memo size");
            assert_eq!(cap, 2, "Exhausted carries configured capacity");
        }
        other => panic!("expected Allocator(Exhausted); got {other:?}"),
    }

    // No partial in-memory state — the failed digest never made it
    // into the memo.
    assert_eq!(
        allocator.get(&d_third),
        None,
        "failed allocation must not populate the in-memory memo",
    );

    // The earlier successful allocation is preserved unchanged.
    assert_eq!(
        allocator.get(&d_first),
        Some(vip_first),
        "prior successful allocations survive an exhaustion failure",
    );

    // No partial on-disk state — exactly `capacity` rows under the
    // allocator prefix, NOT capacity + 1. This proves the fsync
    // didn't fire on the failed allocation path.
    let prefix_rows = store.scan_prefix(b"alloc/v\x01").await.expect("scan_prefix succeeds");
    assert_eq!(
        prefix_rows.len() as u64,
        capacity,
        "exactly `capacity` rows persisted — no partial write for the exhausted allocation",
    );
}

// ---------------------------------------------------------------------------
// S-VIP-20 — release_idempotent
// ---------------------------------------------------------------------------

/// Allocate digest `D`, release it twice, and confirm:
///   1. The double-release does not error.
///   2. `get(&D)` returns `None` post-release (both times).
///   3. The store no longer carries a row for `D`.
#[tokio::test]
async fn release_idempotent() {
    let (store, _dir) = fresh_store();
    let mut allocator = PersistentServiceVipAllocator::new(wide_range(), Arc::clone(&store));

    let d = digest(0xCD);
    let _vip = allocator.allocate(d).await.expect("allocate succeeds");
    assert!(allocator.get(&d).is_some(), "memo populated after allocate");

    // First release — removes from memo and store.
    allocator.release(&d).await.expect("first release succeeds");
    assert_eq!(allocator.get(&d), None, "memo empty after first release");

    // Second release — must not error and must not alter state.
    allocator.release(&d).await.expect("second release is idempotent (no error)");
    assert_eq!(allocator.get(&d), None, "memo still empty after second release");

    // The store no longer carries a row for this digest.
    let prefix_rows = store.scan_prefix(b"alloc/v\x01").await.expect("scan_prefix succeeds");
    assert_eq!(prefix_rows.len(), 0, "store has no rows after release");
}

// Compile-time witness that `ServiceVip` is `Copy + Eq`, so the
// assertions above can use `Some(vip)` equality directly.
const _: fn() = || {
    const fn assert_copy<T: Copy + Eq>() {}
    assert_copy::<ServiceVip>();
    let _ = IpAddr::V4(Ipv4Addr::UNSPECIFIED); // silence unused-import lint
};
