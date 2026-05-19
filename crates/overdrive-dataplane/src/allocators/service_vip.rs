//! `ServiceVip` newtype and `ServiceVipAllocator` — concrete scan-based
//! allocator for IPv4 service VIPs.
//!
//! Per ADR-0049 § 1 the allocator is purely in-memory, synchronous, no
//! I/O, no DB handle. State at construction: a validated [`VipRange`]
//! plus an empty memo. The persistence wrapper
//! [`super::PersistentServiceVipAllocator`] (step 01-03) wraps this with
//! redb-backed write-through and bulk-load reconstruction.
//!
//! # Reuse-on-release (ADR-0049 § Amendments → 2026-05-19)
//!
//! Released VIPs return to the pool. On allocate, the implementation
//! scans the configured [`VipRange`] in order and returns the first
//! address not currently bound by another memo entry. The trade-off is
//! O(capacity) per allocation on the miss path; in exchange the pool
//! does not exhaust after `capacity` total submissions regardless of
//! liveness — a /16 default pool can serve effectively-unbounded
//! lifetimes of distinct workloads as long as the **simultaneously-held
//! count** stays below capacity. This is the inverse of the original
//! monotonic-counter shape (rejected 2026-05-19 as operability-broken).
//!
//! [`BackendIdAllocator`] retains the monotonic-counter-no-reuse shape
//! (correct for its effectively-unbounded `u32` identifier space). The
//! two allocators no longer share a release policy.

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

use super::error::ServiceVipAllocatorError;
use super::vip_range::VipRange;

/// Service VIP token re-exported from the canonical
/// [`overdrive_core::id::ServiceVip`] (step 01-02 consolidation per
/// ADR-0049).
///
/// The newtype wraps [`std::net::IpAddr`] in the canonical declaration;
/// Phase 1 admits IPv4 only per ADR-0049 § 5, and the allocator only
/// ever constructs values via `ServiceVip::new(IpAddr::V4(addr))` from
/// the validated [`VipRange`].
pub use overdrive_core::id::ServiceVip;

/// Service-spec digest — 32-byte content hash that keys the allocator
/// memo. The spec digest is computed upstream (admission handler in
/// step 02-03) by SHA-256 over the canonicalised service spec.
pub type ServiceSpecDigest = [u8; 32];

/// Scan-based VIP-pool allocator with memo-table deduplication and
/// VIP reuse on release (ADR-0049 § Amendments → 2026-05-19).
///
/// Concrete (not generic) — there is no shared abstraction with
/// [`super::BackendIdAllocator`]. The two allocators operate over
/// different token domains with different reuse policies:
/// `BackendIdAllocator` is monotonic-no-reuse over a `u32` space;
/// `ServiceVipAllocator` is scan-based-with-reuse over a finite IPv4
/// pool bounded by an operator-configured `VipRange`.
///
/// # Invariants (S-VIP-P03 / S-VIP-P04 / S-VIP-21 / S-VIP-12)
///
/// - **No duplicate tokens among simultaneously-held entries**: two
///   distinct keys never receive the same `ServiceVip` while both are
///   present in the memo.
/// - **Memo-hit idempotency**: `allocate(K)` returning a memoised token
///   makes no further mutation.
/// - **Reserved-skipping**: the underlying [`VipRange`] excludes
///   reserved addresses; the allocator never observes them.
/// - **Reuse on release**: `release(K)` removes the memo entry. A
///   subsequent `allocate(K')` with a different key MAY receive the
///   released VIP if the scan reaches it first.
/// - **Exhaustion**: [`Self::allocate`] returns
///   [`ServiceVipAllocatorError::Exhausted`] iff every slot in the
///   range is currently held in the memo.
pub struct ServiceVipAllocator {
    range: VipRange,
    /// Memo table: spec-digest → assigned VIP. Iterated only via
    /// `values()` on the allocate-miss scan path (deterministic order
    /// per `BTreeMap`); never relied on for issuance ordering.
    by_digest: BTreeMap<ServiceSpecDigest, ServiceVip>,
}

impl ServiceVipAllocator {
    /// Construct an empty allocator bound to `range`.
    #[must_use]
    pub const fn new(range: VipRange) -> Self {
        Self { range, by_digest: BTreeMap::new() }
    }

    /// Allocate a [`ServiceVip`] for `digest`.
    ///
    /// - **Memo hit**: returns the previously-assigned VIP unchanged.
    /// - **Memo miss**: scans the configured [`VipRange`] in order and
    ///   returns the first allocatable address not currently held.
    ///   Inserts into the memo.
    /// - **Exhaustion**: every slot in the range is currently held in
    ///   the memo. Returns [`ServiceVipAllocatorError::Exhausted`].
    ///
    /// # Errors
    ///
    /// Returns [`ServiceVipAllocatorError::Exhausted`] when the pool
    /// has no available addresses. The `allocated` field is the current
    /// memo size; `capacity` is the configured capacity (after reserved
    /// exclusions).
    pub fn allocate(
        &mut self,
        digest: ServiceSpecDigest,
    ) -> Result<ServiceVip, ServiceVipAllocatorError> {
        if let Some(&existing) = self.by_digest.get(&digest) {
            return Ok(existing);
        }
        let vip = self.scan_for_available()?;
        self.by_digest.insert(digest, vip);
        Ok(vip)
    }

    /// Scan the configured range for the first allocatable address not
    /// currently bound in the memo. Shared by [`Self::allocate`] and
    /// [`Self::peek_next_allocation`].
    ///
    /// O(capacity) worst case. The held-set is materialised once per
    /// call into a `BTreeSet<ServiceVip>` so the per-index contains
    /// check is O(log N) rather than O(N).
    fn scan_for_available(&self) -> Result<ServiceVip, ServiceVipAllocatorError> {
        let held: BTreeSet<ServiceVip> = self.by_digest.values().copied().collect();
        let capacity = self.range.capacity();
        for i in 0..capacity {
            if let Some(addr) = self.range.nth_allocatable(i) {
                let vip = ServiceVip::new(IpAddr::V4(addr))?;
                if !held.contains(&vip) {
                    return Ok(vip);
                }
            }
        }
        Err(ServiceVipAllocatorError::Exhausted {
            allocated: self.by_digest.len() as u64,
            capacity,
        })
    }

    /// Return the VIP currently assigned to `digest`, if any.
    #[must_use]
    pub fn get(&self, digest: &ServiceSpecDigest) -> Option<ServiceVip> {
        self.by_digest.get(digest).copied()
    }

    /// Release the VIP bound to `digest`. Idempotent: a no-op if
    /// `digest` has no current allocation.
    ///
    /// After release, [`Self::get`] returns `None` for `digest`. The
    /// VIP is returned to the available pool per ADR-0049 § Amendments
    /// → 2026-05-19 — a subsequent [`Self::allocate`] with a different
    /// digest MAY receive the released address.
    pub fn release(&mut self, digest: &ServiceSpecDigest) {
        self.by_digest.remove(digest);
    }

    /// Number of entries in the memo table — i.e., the number of VIPs
    /// currently assigned.
    #[must_use]
    pub fn memo_len(&self) -> usize {
        self.by_digest.len()
    }

    /// The configured capacity of the underlying range.
    #[must_use]
    pub fn capacity(&self) -> u64 {
        self.range.capacity()
    }

    /// Returns `true` if `addr` projects within the bound
    /// [`VipRange`] (contained in some configured CIDR AND not in the
    /// reserved set).
    ///
    /// Used by [`super::PersistentServiceVipAllocator::bulk_load`]'s
    /// Earned Trust probe (ADR-0049 § Amendments) to assert every
    /// persisted VIP still falls inside the operator's currently-
    /// configured range. Boot-only — not used on the hot allocate
    /// path.
    #[must_use]
    pub fn range_contains(&self, addr: std::net::Ipv4Addr) -> bool {
        self.range.contains(addr)
    }

    /// Peek the VIP that would be issued for `digest` on the next
    /// [`Self::allocate`] call, WITHOUT mutating any in-memory state.
    ///
    /// Used by [`super::PersistentServiceVipAllocator::allocate`] to
    /// compute the candidate allocation BEFORE the fsync — the
    /// in-memory commit (memo insert) happens only after the
    /// persistence-layer write succeeds, per the fsync-then-memory
    /// ordering rule.
    ///
    /// # Preconditions
    ///
    /// `digest` MUST NOT already be in the memo. Callers must check
    /// [`Self::get`] first and short-circuit on hit.
    ///
    /// # Errors
    ///
    /// [`ServiceVipAllocatorError::Exhausted`] when every slot in the
    /// range is currently held, or
    /// [`ServiceVipAllocatorError::NewtypeRejected`] if the canonical
    /// [`ServiceVip`] constructor rejects the materialised address.
    pub fn peek_next_allocation(
        &self,
        digest: &ServiceSpecDigest,
    ) -> Result<ServiceVip, ServiceVipAllocatorError> {
        debug_assert!(
            !self.by_digest.contains_key(digest),
            "peek_next_allocation must not be called on a memo-hit digest"
        );
        let _ = digest; // suppress unused warning in non-debug builds
        self.scan_for_available()
    }

    /// Replay a persisted entry into the allocator. Used by
    /// [`super::PersistentServiceVipAllocator::bulk_load`] to
    /// reconstruct the in-memory state on restart, and by
    /// [`super::PersistentServiceVipAllocator::allocate`] to commit
    /// the in-memory state AFTER the fsync.
    ///
    /// Inserts the `(digest, vip)` pair into the memo.
    pub fn restore_entry(&mut self, digest: ServiceSpecDigest, vip: ServiceVip) {
        self.by_digest.insert(digest, vip);
    }
}
