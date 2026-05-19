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
    // same VIP — the bulk_load replay reconstructed the memo, so the
    // re-allocate short-circuits without rescanning.
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

// ---------------------------------------------------------------------------
// Step 02-04 — Earned Trust boot probe scenarios.
//
// The probe runs inside `PersistentServiceVipAllocator::bulk_load`. For
// each persisted `(digest, vip)` entry, the probe asserts
// `vip` projects back within the configured `VipRange` (i.e. is contained
// in some configured CIDR AND not in the reserved set). Inconsistency
// (typically caused by an operator narrowing the configured range AFTER
// allocations were persisted under a wider range) surfaces as
// `PersistentAllocatorError::PersistedStateInconsistent`, naming the
// offending VIP.
//
// Driving port: `PersistentServiceVipAllocator::bulk_load(range, store)`.
// Inconsistency is seeded by allocating against a wide range, then
// re-opening the same on-disk store with a narrower range that excludes
// some persisted VIPs.
// ---------------------------------------------------------------------------

/// Range covering `10.96.0.0/24` (256 addresses, none reserved). Wide
/// enough to allocate VIPs that fall OUTSIDE the narrower 02-04 range.
fn wide_24_range() -> VipRange {
    use std::collections::BTreeSet;
    let cidr: ipnet::Ipv4Net = "10.96.0.0/24".parse().expect("valid cidr");
    VipRange::new(vec![cidr], BTreeSet::new()).expect("valid VipRange")
}

/// Narrow range — `10.96.0.0/29` covers `10.96.0.0`..`10.96.0.7` only.
/// Used to project the wide-range allocations after restart; VIPs at
/// `10.96.0.8`+ will be outside this range and trip the probe.
fn narrow_29_range() -> VipRange {
    use std::collections::BTreeSet;
    let cidr: ipnet::Ipv4Net = "10.96.0.0/29".parse().expect("valid cidr");
    VipRange::new(vec![cidr], BTreeSet::new()).expect("valid VipRange")
}

// ---------------------------------------------------------------------------
// S-VIP-19 — boot_probe_refuses_inconsistent_state
// ---------------------------------------------------------------------------

/// Allocate enough VIPs against a wide range that some land outside a
/// narrower range, restart against the narrow range, and assert that
/// `bulk_load` refuses with `PersistedStateInconsistent` naming the
/// first VIP that fails to project back into the active range.
#[tokio::test]
async fn boot_probe_refuses_inconsistent_state() {
    let (store, dir) = fresh_store();

    // First boot — allocate against the wide /24 range; counter
    // walks from .0 upward, so the 10th allocation lands at .9 (well
    // outside the narrower /29).
    let mut allocator = PersistentServiceVipAllocator::new(wide_24_range(), Arc::clone(&store));
    for seed in 0..10 {
        let _ = allocator.allocate(digest(seed)).await.expect("allocate succeeds");
    }
    drop(allocator);
    drop(store);

    // Operator-misconfiguration scenario: restart with a narrower
    // range that excludes the persisted VIPs at .8 and beyond.
    let store_reopened = reopen_store(&dir);
    let result = PersistentServiceVipAllocator::bulk_load(narrow_29_range(), store_reopened).await;
    let Err(err) = result else {
        panic!("bulk_load must refuse when persisted VIPs project outside the active range");
    };

    match err {
        PersistentAllocatorError::PersistedStateInconsistent { vip } => {
            // VIP must be a real IPv4 that is OUTSIDE the narrow range.
            let v4 = vip.try_as_ipv4().expect("ServiceVip is IPv4 in Phase 1");
            assert!(
                !narrow_29_range().contains(v4),
                "PersistedStateInconsistent must name a VIP outside the active range; got {v4}",
            );
        }
        other => {
            panic!("expected PersistedStateInconsistent on narrowed-range restart; got {other:?}")
        }
    }
}

// ---------------------------------------------------------------------------
// S-VIP-passes-consistent — boot_probe_passes_consistent_state
// ---------------------------------------------------------------------------

/// Allocate VIPs against a wide range, restart against the same wide
/// range, and assert that `bulk_load` succeeds (every persisted VIP
/// projects back within the active range).
#[tokio::test]
async fn boot_probe_passes_consistent_state() {
    let (store, dir) = fresh_store();

    let mut allocator = PersistentServiceVipAllocator::new(wide_24_range(), Arc::clone(&store));
    let d_first = digest(0xA0);
    let d_second = digest(0xA1);
    let vip_first = allocator.allocate(d_first).await.expect("first allocate succeeds");
    let _ = allocator.allocate(d_second).await.expect("second allocate succeeds");
    drop(allocator);
    drop(store);

    // Reopen with the SAME wide range — every persisted VIP still
    // projects back within the active range.
    let store_reopened = reopen_store(&dir);
    let reloaded = PersistentServiceVipAllocator::bulk_load(wide_24_range(), store_reopened)
        .await
        .expect("bulk_load succeeds when persisted VIPs project back within active range");

    assert_eq!(
        reloaded.get(&d_first),
        Some(vip_first),
        "consistent-state bulk_load reconstructs the memo",
    );
    assert_eq!(reloaded.memo_len(), 2, "memo carries both persisted allocations");
}

// ---------------------------------------------------------------------------
// S-VIP-empty — boot_probe_passes_empty_store
// ---------------------------------------------------------------------------

/// First-boot scenario: empty `IntentStore`, narrow `VipRange`,
/// `bulk_load` returns Ok with empty memo (the per-VIP projection
/// check is vacuously true with zero persisted entries).
#[tokio::test]
async fn boot_probe_passes_empty_store() {
    let (store, _dir) = fresh_store();

    let allocator = PersistentServiceVipAllocator::bulk_load(narrow_29_range(), store)
        .await
        .expect("bulk_load succeeds on empty store (first boot)");

    assert_eq!(allocator.memo_len(), 0, "first-boot memo is empty");
}
