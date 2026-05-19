//! Property and unit tests for `ServiceVipAllocator` + `VipRange`.
//!
//! Step 01-01 of the service-vip-allocator feature (ADR-0049). Tests
//! the public module API exported from `overdrive_dataplane::allocators`.
//!
//! Scope:
//!
//! - **S-VIP-P03** — `ServiceVipAllocator` never assigns duplicate
//!   tokens to simultaneously-held memo entries; memo size matches
//!   the distinct-key call count. Released VIPs return to the
//!   available pool per ADR-0049 § Amendments → 2026-05-19 — a
//!   subsequent allocate MAY reuse the released address.
//! - **S-VIP-P04** — `VipRange::capacity()` == CIDR size − reserved
//!   count for all valid `(cidr, reserved)` inputs.
//! - **S-VIP-21** — Reserved addresses skipped during allocation; pool
//!   exhausts after 2 allocations on a /30 with 2 reserved.
//! - **S-VIP-16** — `VipRange::new` rejects overlapping CIDR ranges.
//! - **S-VIP-17** — `VipRange::new` rejects reserved addresses outside
//!   the configured range.
//! - **S-VIP-18** — `VipRange::new` rejects zero effective capacity.
//! - **S-VIP-12** — `ServiceVipAllocator` constructed from a `/24`
//!   range allocates a `ServiceVip` within range, returns the same
//!   VIP on memo-hit for the same `spec_digest`, and `get` returns
//!   `None` after `release`. Pins the post-consolidation behaviour:
//!   the allocator's returned token IS the canonical
//!   `overdrive_core::id::ServiceVip` (step 01-02 consolidation).
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

        // Release clears the memo entry; per ADR-0049 § Amendments
        // → 2026-05-19, released VIPs return to the available pool.
        // Re-allocate of the same digest hits the scan path and (for
        // a /24 pool with the scan starting at index 0) returns the
        // FIRST available address — which is the lowest unallocated
        // address in the held set's complement. The released VIP IS
        // available; whether the scan returns it depends on whether
        // it sits earlier in the range than the other unallocated
        // addresses.
        if let Some(first) = keys.first().copied() {
            let d = digest_from_u64(first);
            let pre_release = alloc.get(&d).expect("memoised");
            alloc.release(&d);
            prop_assert_eq!(alloc.get(&d), None);

            // If the pool still has capacity, re-allocate must succeed
            // and stay within the /24 range. The reuse property is the
            // inverse of the prior monotonic shape.
            if alloc.memo_len() < usize::try_from(alloc.capacity()).unwrap_or(usize::MAX) {
                let realloc = alloc.allocate(d).expect("re-allocate after release");
                let realloc_v4 = realloc
                    .try_as_ipv4()
                    .expect("allocator always produces IPv4 tokens (ADR-0049 § 5)");
                prop_assert!(realloc_v4 >= Ipv4Addr::new(10, 96, 0, 0));
                prop_assert!(realloc_v4 <= Ipv4Addr::new(10, 96, 0, 255));
                // No assert_ne — the scan-based allocator MAY return the
                // released VIP. The simultaneously-held no-duplicate
                // invariant is asserted globally above (`tokens.insert`).
                let _ = pre_release;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// S-VIP-P03 (additional) — capacity=1 release-then-realloc returns same VIP
// ---------------------------------------------------------------------------

/// Mechanical reuse property: with `capacity = 1` and the sole VIP
/// held, releasing and then allocating a byte-different digest MUST
/// return the original VIP (there is no other address available). This
/// is the property the integration test `vip_allocator_lifecycle`
/// pins end-to-end; this unit-level test pins the allocator-level
/// contract in isolation per `.claude/rules/development.md` §
/// "Trait definitions specify behavior, not just signature".
#[test]
fn release_then_reallocate_reuses_address_under_capacity_one() {
    use std::net::Ipv4Addr;

    let cidr = Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 1), 32).expect("/32 valid");
    let range = VipRange::new(vec![cidr], BTreeSet::new()).expect("single-address pool");
    assert_eq!(range.capacity(), 1);
    let mut alloc = ServiceVipAllocator::new(range);

    let digest_a = digest_from_u64(0x1111_1111_1111_1111);
    let digest_b = digest_from_u64(0x2222_2222_2222_2222);

    let vip_a = alloc.allocate(digest_a).expect("first allocation");
    assert_eq!(
        vip_a.try_as_ipv4().expect("v4"),
        Ipv4Addr::new(10, 96, 0, 1),
        "capacity=1 pool must issue the sole address",
    );

    // Without release, a different digest exhausts.
    assert!(matches!(alloc.allocate(digest_b), Err(ServiceVipAllocatorError::Exhausted { .. }),));

    // Release A; the VIP is now available again.
    alloc.release(&digest_a);

    // Allocate B — MUST return the same VIP since the pool has exactly
    // one address.
    let vip_b = alloc.allocate(digest_b).expect("post-release alloc");
    assert_eq!(
        vip_b, vip_a,
        "capacity=1 reuse: released VIP MUST be reissued on the next allocation",
    );
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
    let t1_v4 = t1.try_as_ipv4().expect("allocator emits IPv4");
    let t2_v4 = t2.try_as_ipv4().expect("allocator emits IPv4");
    assert!(allowed.contains(&t1_v4), "t1 = {t1:?} not in allowed set");
    assert!(allowed.contains(&t2_v4), "t2 = {t2:?} not in allowed set");
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

// ---------------------------------------------------------------------------
// S-VIP-12 — Allocator serves the canonical `overdrive_core::id::ServiceVip`
//
// Pins the step 01-02 consolidation: the local `ServiceVip` declaration
// in `crates/overdrive-dataplane/src/allocators/service_vip.rs` is
// replaced by a re-export of the canonical newtype, and the allocator's
// `allocate(...)` / `get(...)` return that canonical token. The test
// names the canonical newtype via its `overdrive_core::id` path
// explicitly and equates against the re-export from
// `overdrive_dataplane::allocators` — both must resolve to the same
// type at compile time, which is the structural defense that exactly
// one declaration exists post-commit.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Mutation-coverage tests — `capacity()` accessors + envelope diagnostics
// ---------------------------------------------------------------------------

/// `ServiceVipAllocator::capacity()` MUST reflect the underlying
/// `VipRange::capacity()` — not a hardcoded 0, 1, or any other constant.
/// Three distinct ranges with three distinct capacities pin the mapping.
#[test]
fn service_vip_allocator_capacity_reflects_range() {
    use std::net::Ipv4Addr;

    // /30 with one reserved → capacity 3.
    let r30 = VipRange::new(
        vec![Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 0), 30).unwrap()],
        std::iter::once(Ipv4Addr::new(10, 96, 0, 0)).collect(),
    )
    .expect("/30 with 1 reserved");
    assert_eq!(ServiceVipAllocator::new(r30).capacity(), 3);

    // /30 no reserved → capacity 4.
    let r30_full = VipRange::new(
        vec![Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 0), 30).unwrap()],
        BTreeSet::new(),
    )
    .expect("/30 no reserved");
    assert_eq!(ServiceVipAllocator::new(r30_full).capacity(), 4);

    // /24 no reserved → capacity 256.
    let r24 = VipRange::new(
        vec![Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 0), 24).unwrap()],
        BTreeSet::new(),
    )
    .expect("/24 no reserved");
    assert_eq!(ServiceVipAllocator::new(r24).capacity(), 256);
}

/// `PersistentServiceVipAllocator::capacity()` MUST delegate to the
/// underlying allocator's capacity. Pin via a wrapper construction
/// against a tempdir-backed store and a non-default range — a hardcoded
/// 0 or 1 in the pass-through would diverge from the inner capacity.
#[tokio::test]
async fn persistent_service_vip_allocator_capacity_reflects_range() {
    use std::net::Ipv4Addr;
    use std::sync::Arc;

    use overdrive_core::traits::intent_store::IntentStore;
    use overdrive_dataplane::allocators::PersistentServiceVipAllocator;
    use overdrive_store_local::LocalIntentStore;
    use tempfile::TempDir;

    let tmp = TempDir::new().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let store: Arc<dyn IntentStore> =
        Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));

    // /28 with 0 reserved → capacity 16; distinct from the inner
    // allocator's defaults and from 0 / 1 mutations.
    let range = VipRange::new(
        vec![Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 0), 28).unwrap()],
        BTreeSet::new(),
    )
    .expect("/28 range");

    let persistent = PersistentServiceVipAllocator::new(range, store);
    assert_eq!(
        persistent.capacity(),
        16,
        "PersistentServiceVipAllocator::capacity must delegate to the underlying VipRange",
    );
}

/// `ServiceVipAllocatorEntryEnvelope::known_discriminants()` and
/// `type_name()` are the diagnostic surface the
/// `VersionedEnvelope::probe_unknown_discriminant` decoder consults
/// when emitting `EnvelopeError::UnknownVersion`. The exact values
/// matter — a `&[]` or `&[1]` discriminant set would silently classify
/// every persisted V1 / V2 byte as "unknown version" and the
/// `bulk_load` decoder would refuse every Earned Trust probe under
/// production load.
#[test]
fn service_vip_allocator_entry_envelope_diagnostics() {
    use overdrive_core::codec::VersionedEnvelope;
    use overdrive_dataplane::allocators::entry::ServiceVipAllocatorEntryEnvelope;

    // V1 + V2 currently shipped — discriminant set must name exactly
    // these two rkyv tags (declaration order: V1 = 0, V2 = 1).
    assert_eq!(
        ServiceVipAllocatorEntryEnvelope::known_discriminants(),
        &[0u8, 1u8],
        "known_discriminants must enumerate the V1 + V2 rkyv discriminant tags exactly",
    );

    // The type name is structured-diagnostic — it surfaces in
    // `health.startup.refused` events and operator-facing errors. The
    // canonical name is the envelope's Rust type ident.
    assert_eq!(
        ServiceVipAllocatorEntryEnvelope::type_name(),
        "ServiceVipAllocatorEntryEnvelope",
        "type_name must match the envelope's Rust type identifier for operator diagnostics",
    );
}

#[test]
fn service_vip_allocator_serves_canonical_newtype() {
    // Constructor: /24 range, no reserved addresses.
    let range = VipRange::new(
        vec![Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 0), 24).unwrap()],
        BTreeSet::new(),
    )
    .expect("valid /24 range");
    let mut alloc = ServiceVipAllocator::new(range);

    let digest_a = digest_from_u64(0xAAAA_AAAA_AAAA_AAAA);
    let digest_b = digest_from_u64(0xBBBB_BBBB_BBBB_BBBB);

    // First allocation lands inside the range.
    let vip_a = alloc.allocate(digest_a).expect("first allocation within capacity");
    // The returned token IS the canonical `overdrive_core::id::ServiceVip` —
    // this assignment fails to compile if the consolidation has not happened
    // (e.g. if a local newtype distinct from the canonical re-emerges).
    let _: overdrive_core::id::ServiceVip = vip_a;

    // Distinct digest yields a distinct VIP.
    let vip_b = alloc.allocate(digest_b).expect("second allocation within capacity");
    assert_ne!(vip_a, vip_b, "distinct digests must yield distinct VIPs");

    // Memo hit: re-allocating digest_a returns the same VIP.
    let vip_a_again = alloc.allocate(digest_a).expect("memo hit");
    assert_eq!(vip_a, vip_a_again, "memo-hit must return the prior token");

    // `get(&digest_a)` reflects the memo before release.
    assert_eq!(alloc.get(&digest_a), Some(vip_a));

    // After release, `get` returns `None`.
    alloc.release(&digest_a);
    assert_eq!(alloc.get(&digest_a), None, "post-release lookup must be None");
}
