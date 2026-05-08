//! Collision-free `BackendId` allocator per ADR-0046.
//!
//! Replaces the multiplicative-hash derivation at
//! `lib.rs:913-916` / `lib.rs:968-971` with a monotonic-counter
//! allocator + memo table, matching Cilium's `IDAllocator` pattern.
//!
//! The allocator is userspace-only; zero kernel-side changes.
//!
//! # Crash recovery
//!
//! On process restart the allocator starts fresh (counter at 1,
//! empty memo). All services are re-hydrated by the
//! `ServiceMapHydrator` reconciler (ADR-0042), which calls
//! `update_service` for every active service, rebuilding the memo
//! from scratch. Existing BACKEND_MAP entries from the prior
//! process are overwritten with new ids. This is correct because
//! the inner ARRAYs are also rebuilt atomically during
//! re-hydration.

use std::collections::BTreeMap;

use overdrive_core::id::BackendId;

/// Monotonic-counter `BackendId` allocator with memo-table
/// deduplication. Owned by `EbpfDataplane` behind
/// `parking_lot::Mutex`.
///
/// The counter starts at 1 (0 is reserved for empty-slot semantics
/// in the inner ARRAY). The memo table maps `(ip_host, port_host,
/// proto)` to the assigned `BackendId`.
pub struct BackendIdAllocator {
    next: u32,
    by_endpoint: BTreeMap<(u32, u16, u8), BackendId>,
}

impl BackendIdAllocator {
    /// Create a fresh allocator. Counter starts at 1.
    #[must_use]
    pub const fn new() -> Self {
        Self { next: 1, by_endpoint: BTreeMap::new() }
    }

    /// Return the existing `BackendId` for this endpoint (memo hit)
    /// or assign a new one from the monotonic counter.
    ///
    /// # Panics
    ///
    /// Panics if the counter overflows `u32::MAX`. In practice this
    /// requires 4,294,967,295 distinct endpoints over a single
    /// process lifetime — unreachable in production.
    #[allow(clippy::expect_used)]
    pub fn allocate(&mut self, ip: u32, port: u16, proto: u8) -> BackendId {
        if let Some(&id) = self.by_endpoint.get(&(ip, port, proto)) {
            return id;
        }
        // BackendId::new is infallible (every u32 is valid), but the
        // newtype-completeness shape returns Result. The expect is
        // structurally unreachable.
        let id = BackendId::new(self.next).expect("BackendId::new is infallible for any u32");
        self.next =
            self.next.checked_add(1).expect("BackendId counter exhausted (>4 billion endpoints)");
        self.by_endpoint.insert((ip, port, proto), id);
        id
    }

    /// Remove the memo entry whose value matches `id`. Called by
    /// orphan GC when a `BackendId` leaves the live set.
    ///
    /// Does NOT recycle the counter value — the counter is monotonic
    /// and never wraps in practice.
    pub fn release(&mut self, id: BackendId) {
        self.by_endpoint.retain(|_, v| *v != id);
    }

    /// Number of entries in the memo table. Diagnostic-only — used by
    /// integration tests to verify that release() was called after
    /// orphan-GC sweeps.
    #[must_use]
    pub fn memo_len(&self) -> usize {
        self.by_endpoint.len()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // ---- Test A: proptest — no duplicate BackendIds ----

    proptest! {
        #[test]
        fn allocator_never_assigns_duplicate_ids(
            endpoints in prop::collection::vec(
                (any::<u32>(), any::<u16>(), prop::sample::select(vec![6u8, 17u8])),
                1..1000
            )
        ) {
            let mut alloc = BackendIdAllocator::new();
            let mut seen: BTreeMap<BackendId, (u32, u16, u8)> = BTreeMap::new();
            for &(ip, port, proto) in &endpoints {
                let id = alloc.allocate(ip, port, proto);
                if let Some(prev) = seen.get(&id) {
                    // Same id must mean same endpoint
                    prop_assert_eq!(prev, &(ip, port, proto));
                }
                seen.insert(id, (ip, port, proto));
            }
        }
    }

    // ---- Test B: deterministic collision witness ----
    //
    // The pair (1_660_235_791, 37722) and (1_033_951_002, 57791)
    // collide under the OLD multiplicative hash. Found by random
    // sampling (seed 42, ~200k trials):
    //
    //   hash(ip, port) = ip.wrapping_mul(2_654_435_761).wrapping_add(port as u32)
    //   hash(1_660_235_791, 37722) = 3_832_997_049
    //   hash(1_033_951_002, 57791) = 3_832_997_049
    //
    // The allocator must assign distinct ids to this pair.

    #[test]
    fn old_hash_collision_pair_gets_distinct_ids() {
        // Verify the collision under the old hash exists.
        let hash_a = 1_660_235_791u32.wrapping_mul(2_654_435_761).wrapping_add(37_722u32);
        let hash_b = 1_033_951_002u32.wrapping_mul(2_654_435_761).wrapping_add(57_791u32);
        assert_eq!(hash_a, hash_b, "precondition: old hash must collide");

        // The new allocator must NOT collide.
        let mut alloc = BackendIdAllocator::new();
        let proto = 6u8; // TCP
        let id_a = alloc.allocate(1_660_235_791, 37_722, proto);
        let id_b = alloc.allocate(1_033_951_002, 57_791, proto);
        assert_ne!(id_a, id_b, "allocator must assign distinct BackendIds to distinct endpoints");
    }
}
