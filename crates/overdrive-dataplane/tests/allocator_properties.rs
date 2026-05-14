//! Property and unit tests for `ServiceVipAllocator` + `VipRange`.
//!
//! Step 01-01 of the service-vip-allocator feature (ADR-0049). Tests
//! the public module API exported from `overdrive_dataplane::allocators`.
//!
//! Scope:
//!
//! - **S-VIP-P03** — `ServiceVipAllocator` never assigns duplicate
//!   tokens; memo size matches the distinct-key call count.
//! - **S-VIP-P04** — `VipRange::capacity()` == CIDR size − reserved
//!   count for all valid `(cidr, reserved)` inputs.
//! - **S-VIP-21** — Reserved addresses skipped during allocation; pool
//!   exhausts after 2 allocations on a /30 with 2 reserved.
//! - **S-VIP-16** — `VipRange::new` rejects overlapping CIDR ranges.
//! - **S-VIP-17** — `VipRange::new` rejects reserved addresses outside
//!   the configured range.
//! - **S-VIP-18** — `VipRange::new` rejects zero effective capacity.
//!
//! Unit-level tests, default lane per DWD-03 — no `integration-tests`
//! feature gate; pure in-memory; no I/O.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

use ipnet::Ipv4Net;
use overdrive_dataplane::allocators::{
    ServiceSpecDigest, ServiceVip, ServiceVipAllocator, ServiceVipAllocatorError,
    VipAllocatorConfigError, VipRange,
};
use proptest::prelude::*;

/// Build a `ServiceSpecDigest` from a `u64` for proptest convenience.
/// The 8-byte big-endian representation of `n` lives in the first 8
/// bytes; the rest is zero. Distinct `n` produce distinct digests.
fn digest_from_u64(n: u64) -> ServiceSpecDigest {
    let mut d = [0u8; 32];
    d[..8].copy_from_slice(&n.to_be_bytes());
    d
}

// ---------------------------------------------------------------------------
// S-VIP-P03 — ServiceVipAllocator never assigns duplicate tokens
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn pool_allocator_no_duplicate_tokens(
        // Up to 200 distinct keys; bounded so capacity is not exceeded
        // on the /24 range. Distinct via BTreeSet collection.
        key_set in proptest::collection::btree_set(any::<u64>(), 1..200),
    ) {
        let range = VipRange::new(
            vec![Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 0), 24).unwrap()],
            BTreeSet::new(),
        ).expect("valid range");
        let mut alloc = ServiceVipAllocator::new(range);

        let mut tokens: BTreeSet<ServiceVip> = BTreeSet::new();
        let keys: Vec<u64> = key_set.into_iter().collect();
        for key in &keys {
            let token = alloc.allocate(digest_from_u64(*key)).expect("within capacity");
            prop_assert!(tokens.insert(token), "duplicate token assigned for key {key}");
        }
        prop_assert_eq!(alloc.memo_len(), keys.len());

        // Idempotency on memo hit — same digest returns same VIP.
        if let Some(first) = keys.first().copied() {
            let d = digest_from_u64(first);
            let token_again = alloc.allocate(d).expect("memo hit");
            prop_assert!(tokens.contains(&token_again));
        }

        // Release clears the memo entry; counter does NOT rewind, so a
        // re-allocate of the same digest yields a fresh VIP (different
        // from the released one), proving monotonic semantics.
        if let Some(first) = keys.first().copied() {
            let d = digest_from_u64(first);
            let pre_release = alloc.get(&d).expect("memoised");
            alloc.release(&d);
            prop_assert_eq!(alloc.get(&d), None);

            // If the pool still has capacity, re-allocate must succeed
            // and yield a distinct VIP (counter moved on).
            if alloc.memo_len() < usize::try_from(alloc.capacity()).unwrap_or(usize::MAX) {
                let realloc = alloc.allocate(d).expect("re-allocate after release");
                prop_assert!(realloc.as_ipv4() >= Ipv4Addr::new(10, 96, 0, 0));
                prop_assert!(realloc.as_ipv4() <= Ipv4Addr::new(10, 96, 0, 255));
                prop_assert_ne!(
                    realloc, pre_release,
                    "monotonic counter must NOT reuse the released VIP"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// S-VIP-P04 — VipRange::capacity() == CIDR size − reserved count
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn vip_range_capacity_invariant(
        // /24 — /30 produce a manageable address space for proptest.
        prefix in 24u8..=30u8,
        reserved_pick in proptest::collection::vec(0u8..=255u8, 0..10),
    ) {
        let base = Ipv4Addr::new(10, 96, 0, 0);
        let cidr = Ipv4Net::new(base, prefix).unwrap();
        let total: u64 = 1u64 << (32 - u32::from(prefix));

        let span: u32 = 1u32 << (32 - u32::from(prefix));
        let mut reserved: BTreeSet<Ipv4Addr> = BTreeSet::new();
        for offset in reserved_pick {
            let off = u32::from(offset);
            if off < span {
                let net = u32::from(cidr.network());
                reserved.insert(Ipv4Addr::from(net + off));
            }
        }

        let range = VipRange::new(vec![cidr], reserved.clone());
        if total == reserved.len() as u64 {
            prop_assert!(matches!(range, Err(VipAllocatorConfigError::ZeroCapacity)));
        } else {
            let range = range.expect("valid range");
            prop_assert_eq!(
                range.capacity(),
                total - reserved.len() as u64,
                "capacity must equal CIDR size minus reserved count"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// S-VIP-21 — Reserved addresses are skipped during allocation
// ---------------------------------------------------------------------------

#[test]
fn reserved_addresses_skipped_during_allocation() {
    let cidr = Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 0), 30).unwrap(); // 4 addrs
    let reserved: BTreeSet<Ipv4Addr> =
        [Ipv4Addr::new(10, 96, 0, 0), Ipv4Addr::new(10, 96, 0, 3)].into_iter().collect();
    let range = VipRange::new(vec![cidr], reserved).expect("valid range");
    assert_eq!(range.capacity(), 2);

    let mut alloc = ServiceVipAllocator::new(range);

    let t1 = alloc.allocate(digest_from_u64(1)).expect("first allocation");
    let t2 = alloc.allocate(digest_from_u64(2)).expect("second allocation");

    let allowed: BTreeSet<Ipv4Addr> =
        [Ipv4Addr::new(10, 96, 0, 1), Ipv4Addr::new(10, 96, 0, 2)].into_iter().collect();
    assert!(allowed.contains(&t1.as_ipv4()), "t1 = {t1:?} not in allowed set");
    assert!(allowed.contains(&t2.as_ipv4()), "t2 = {t2:?} not in allowed set");
    assert_ne!(t1, t2, "first and second allocations must be distinct");

    // Third allocation must fail with Exhausted { allocated: 2, capacity: 2 }
    let third = alloc.allocate(digest_from_u64(3));
    match third {
        Err(ServiceVipAllocatorError::Exhausted { allocated, capacity }) => {
            assert_eq!(allocated, 2);
            assert_eq!(capacity, 2);
        }
        other => panic!("expected ServiceVipAllocatorError::Exhausted{{2,2}}, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// S-VIP-16 — Overlapping CIDR ranges rejected
// ---------------------------------------------------------------------------

#[test]
fn vip_range_rejects_overlapping_cidrs() {
    let a = Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 0), 24).unwrap();
    let b = Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 0), 16).unwrap();
    let result = VipRange::new(vec![a, b], BTreeSet::new());
    match result {
        Err(VipAllocatorConfigError::OverlappingRanges { a: ra, b: rb }) => {
            let names: BTreeSet<String> = [ra.to_string(), rb.to_string()].into_iter().collect();
            assert!(names.contains(&a.to_string()));
            assert!(names.contains(&b.to_string()));
        }
        other => panic!("expected OverlappingRanges, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// S-VIP-17 — Reserved address outside configured range rejected
// ---------------------------------------------------------------------------

#[test]
fn vip_range_rejects_reserved_outside_range() {
    let cidr = Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 0), 24).unwrap();
    let mut reserved = BTreeSet::new();
    let outside = Ipv4Addr::new(192, 168, 1, 1);
    reserved.insert(outside);
    let result = VipRange::new(vec![cidr], reserved);
    match result {
        Err(VipAllocatorConfigError::ReservedOutsideRange { addr }) => {
            assert_eq!(addr, outside);
        }
        other => panic!("expected ReservedOutsideRange, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// S-VIP-18 — Zero effective capacity rejected
// ---------------------------------------------------------------------------

#[test]
fn vip_range_rejects_zero_capacity() {
    let cidr = Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 1), 32).unwrap();
    let mut reserved = BTreeSet::new();
    reserved.insert(Ipv4Addr::new(10, 96, 0, 1));
    let result = VipRange::new(vec![cidr], reserved);
    assert!(
        matches!(result, Err(VipAllocatorConfigError::ZeroCapacity)),
        "expected ZeroCapacity, got {result:?}"
    );
}
