//! [`ServiceVipView`] — the assigned-VIP read port over the service-VIP
//! allocator memo (ADR-0086 D5).
//!
//! One of the four narrow driven read-ports the reconciler hydration boundary
//! reads (ADR-0086 D5). A Service reconciler's hydration reads the
//! allocator-issued VIP for a content-addressed spec digest through this port.
//! The production impl is `PersistentServiceVipAllocator` (dataplane); the DST
//! impl is `SimServiceVipView` (`overdrive-sim`, step 02-05), which makes the
//! memo-absent path injectable for the first time (ADR-0086 D8).
//!
//! Per `.claude/rules/development.md` § "Port-trait dependencies" the reconciler
//! reads through `&dyn ServiceVipView` on the [`HydrationContext`], never a
//! concrete `AppState` field. Per § "Trait definitions specify behavior, not
//! just signature" the method rustdoc below is the SSOT the sim adapter and the
//! DST equivalence test enforce against every impl.
//!
//! [`HydrationContext`]: crate::reconcilers::HydrationContext

use async_trait::async_trait;

use crate::id::{ContentHash, ServiceVip};

/// The assigned-VIP read port over the service-VIP allocator memo (ADR-0086 D5).
///
/// A **driven, read-only** projection: the reconciler calls out to fetch the
/// memoised VIP for a spec digest; there is no allocate/release method on this
/// port (the mutating allocator surface is separate — read/write split of
/// Principle 12). Async because the production impl
/// (`PersistentServiceVipAllocator`) is held behind a `tokio::sync::Mutex` on
/// `AppState`, so the read crosses an `.await`.
#[async_trait]
pub trait ServiceVipView: Send + Sync {
    /// The allocator-issued VIP for the content-addressed spec digest, or
    /// `None` when no VIP is memoised for it.
    ///
    /// # Preconditions
    /// None. Any [`ContentHash`] is a valid query — a digest the allocator has
    /// never issued a VIP for reads as `None`.
    ///
    /// # Postconditions
    /// On `Some`, returns an **owned** [`ServiceVip`] the allocator has memoised
    /// for `spec_digest`. The allocator memo is unchanged — this is a pure read,
    /// allocates no VIP and mutates no memo. The caller holds no lock after the
    /// call returns.
    ///
    /// # Edge cases
    /// `None` on a **persisted** Service intent is the ADR-0049 §4
    /// structural-invariant-violation signal: the caller MUST DEFER the tick —
    /// not hydrate the service's `State`, emit NO `Action` for it, and in
    /// particular NEVER panic and NEVER default a VIP. It is not an error return
    /// and not an empty-but-present VIP. The adapter maps the core
    /// [`ContentHash`] to the allocator's own `ServiceSpecDigest` (a `[u8; 32]`);
    /// the mapping is byte-identity on the 32-byte digest.
    ///
    /// # Observable invariants
    /// This is a **point read** keyed by content: two reads of the same
    /// `spec_digest` with no intervening allocate/release return equal values.
    /// `assigned_vip(spec_digest)` reflects exactly the allocator's current
    /// memo for that digest: `Some(vip)` after an allocate, `None` after a
    /// release (or before any allocate). It never mutates the memo.
    async fn assigned_vip(&self, spec_digest: &ContentHash) -> Option<ServiceVip>;
}
