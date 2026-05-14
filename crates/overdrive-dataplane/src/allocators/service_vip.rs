//! `ServiceVip` newtype and `ServiceVipAllocator` — concrete monotonic
//! allocator for IPv4 service VIPs.
//!
//! Per ADR-0049 § 1: the allocator is purely in-memory, synchronous, no
//! I/O, no DB handle. State at construction: a validated [`VipRange`]
//! plus a `u64` monotonic counter at zero. The persistence wrapper
//! `IntentBackedAllocator` (step 01-03) wraps this with redb-backed
//! write-through and bulk-load reconstruction.
//!
//! **Shape mirrors [`super::BackendIdAllocator`] deliberately**: memo +
//! monotonic counter, memo-hit-returns-existing, no slot reclamation on
//! release. Released entries clear the memo but the counter does not
//! rewind — a released VIP is permanently lost to the pool. This keeps
//! the allocator trivially DST-replayable and removes a class of "did
//! we reuse the right slot?" reasoning. The trade-off is operator-
//! visible: a pool sized for `N` distinct workload lifetimes; once N
//! allocations have happened (across the boot lifetime, regardless of
//! intervening releases) the pool is exhausted and refuses. Phase 1 is
//! single-node, single boot; this is a deliberate Phase 1 simplification
//! and an operator note in the boot-time `health.startup.ready` event.
//!
//! `BackendIdAllocator` made the same choice (commit `allocator.rs:69`:
//! "Does NOT recycle the counter value — the counter is monotonic and
//! never wraps in practice"). Same shape; different token domain.

use std::collections::BTreeMap;
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

/// Monotonic VIP-pool allocator with memo-table deduplication.
///
/// Concrete (not generic) — there is no shared abstraction with
/// [`super::BackendIdAllocator`]. The two allocators happen to follow
/// the same memo-plus-counter shape but operate over different token
/// domains with different exhaustion semantics: BackendId has a `u32`
/// counter and effectively unbounded supply; ServiceVip is bounded by
/// the operator-configured `VipRange`.
///
/// # Invariants (S-VIP-P03 / S-VIP-P04 / S-VIP-21)
///
/// - **No duplicate tokens**: two distinct keys never receive the same
///   `ServiceVip` while both are present in the memo.
/// - **Memo-hit idempotency**: `allocate(K)` returning a memoised token
///   leaves the counter unchanged.
/// - **Reserved-skipping**: the underlying [`VipRange`] excludes
///   reserved addresses; the allocator never observes them.
/// - **Monotonic counter**: `release` removes the memo entry but does
///   not rewind the counter; a released VIP is not reused.
/// - **Exhaustion**: when the counter exceeds the range's effective
///   capacity (or wraps `u64`), [`Self::allocate`] returns
///   [`ServiceVipAllocatorError::Exhausted`].
pub struct ServiceVipAllocator {
    range: VipRange,
    /// Monotonic counter into the allocatable sequence. Advances on
    /// every memo miss; never rewinds on release.
    next_idx: u64,
    /// Memo table: spec-digest → assigned VIP.
    memo: BTreeMap<ServiceSpecDigest, ServiceVip>,
}

impl ServiceVipAllocator {
    /// Construct an empty allocator bound to `range`. Counter starts
    /// at zero (first allocation returns the 0th allocatable address
    /// in the range, skipping reserved entries).
    #[must_use]
    pub const fn new(range: VipRange) -> Self {
        Self { range, next_idx: 0, memo: BTreeMap::new() }
    }

    /// Allocate a [`ServiceVip`] for `digest`.
    ///
    /// - **Memo hit**: returns the previously-assigned VIP; counter is
    ///   unchanged.
    /// - **Memo miss**: materializes the next address via
    ///   [`VipRange::nth_allocatable`], advances the counter, inserts
    ///   into the memo, returns the VIP.
    /// - **Exhaustion**: [`VipRange::nth_allocatable`] returns `None`
    ///   for the current counter, or the counter wraps `u64::MAX`.
    ///   Returns [`ServiceVipAllocatorError::Exhausted`].
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
        if let Some(&existing) = self.memo.get(&digest) {
            return Ok(existing);
        }
        let addr = self.range.nth_allocatable(self.next_idx).ok_or_else(|| {
            ServiceVipAllocatorError::Exhausted {
                allocated: self.memo.len() as u64,
                capacity: self.range.capacity(),
            }
        })?;
        // `ServiceVip::new` is total over `IpAddr` today; the IPv4 wrap
        // is always valid. The `?` propagates as
        // `ServiceVipAllocatorError::NewtypeRejected` so a future range-
        // rejection on the canonical newtype surfaces as a typed error
        // rather than a panic — distinct failure modes get distinct
        // variants per `.claude/rules/development.md` § Errors.
        let vip = ServiceVip::new(IpAddr::V4(addr))?;
        // Saturating add on the off-chance of u64 overflow — at that
        // point the range is also long-exhausted, so the next call hits
        // the `nth_allocatable` exhaustion branch above.
        self.next_idx = self.next_idx.saturating_add(1);
        self.memo.insert(digest, vip);
        Ok(vip)
    }

    /// Return the VIP currently assigned to `digest`, if any.
    #[must_use]
    pub fn get(&self, digest: &ServiceSpecDigest) -> Option<ServiceVip> {
        self.memo.get(digest).copied()
    }

    /// Release the VIP bound to `digest`. Idempotent: a no-op if
    /// `digest` has no current allocation.
    ///
    /// After release, [`Self::get`] returns `None` for `digest`. The
    /// VIP is NOT returned to the pool — the counter is monotonic.
    pub fn release(&mut self, digest: &ServiceSpecDigest) {
        self.memo.remove(digest);
    }

    /// Number of entries in the memo table — i.e., the number of VIPs
    /// currently assigned (NOT the number of VIPs ever issued; the
    /// counter tracks that separately and is not exposed).
    #[must_use]
    pub fn memo_len(&self) -> usize {
        self.memo.len()
    }

    /// The configured capacity of the underlying range.
    #[must_use]
    pub fn capacity(&self) -> u64 {
        self.range.capacity()
    }
}
