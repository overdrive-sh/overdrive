//! [`HeldSvidView`] — the global node-held SVID snapshot read port
//! (ADR-0086 D5).
//!
//! One of the four narrow driven read-ports the reconciler hydration boundary
//! reads (ADR-0086 D5). The `SvidLifecycle` reconciler's hydration reads the
//! node's held-SVID set — its `actual` half — through this port. The production
//! impl is `IdentityMgr` (control-plane); the DST impl is `SimHeldSvidView`
//! (`overdrive-sim`, step 02-05), which makes the global-set / target-filter
//! path injectable for the first time (ADR-0086 D8).
//!
//! **Sync**, not async: the read is a `parking_lot::RwLock` read-lock → clone →
//! drop, with no `.await`, so the tick path pays no boxed future per read
//! (mirrors [`WorkflowLiveSet`](crate::traits::WorkflowLiveSet)). Per
//! `.claude/rules/development.md` § "Trait definitions specify behavior, not
//! just signature" the method rustdoc below is the SSOT the sim adapter and the
//! DST equivalence test enforce against every impl.
//!
//! [`HydrationContext`]: crate::reconcilers::HydrationContext

use std::collections::BTreeMap;

use crate::id::AllocationId;
use crate::identity::HeldSvidFacts;

/// The global node-held SVID snapshot read port (ADR-0086 D5).
///
/// A **driven, read-only** projection: the reconciler calls out to snapshot the
/// node's held-SVID set; there is no issue/drop method on this port (those are
/// the `IssueSvid` / `DropSvid` action executors — read/write split of
/// Principle 12). Sync — the read is an in-memory snapshot.
pub trait HeldSvidView: Send + Sync {
    /// A snapshot of the **GLOBAL** node-held SVID set — every workload's held
    /// leaf projected to its non-secret [`HeldSvidFacts`], keyed by
    /// [`AllocationId`]; presence of a key means "currently held".
    ///
    /// # Preconditions
    /// None.
    ///
    /// # Postconditions
    /// Returns an **owned** [`BTreeMap`] cloned from the holder as of the moment
    /// of the call — ephemeral runtime state, rebuilt on restart. The held set
    /// is unchanged (pure read; the leaf private key is NEVER projected — K2);
    /// no lock is held across the return. [`BTreeMap`] iteration order is
    /// deterministic across DST seeds (K5).
    ///
    /// # Edge cases
    /// The map is the **unfiltered global set by contract** — it carries EVERY
    /// workload's held facts, not just one target's. Filtering to the target
    /// workload (by `SpiffeId::for_allocation` equality) is the HYDRATOR's job,
    /// NOT this port's (ADR-0067 D5b): the port over-returns deliberately so the
    /// filter policy lives in one place. An empty map means nothing is held
    /// (legitimate after a fresh restart), not an error.
    ///
    /// # Observable invariants
    /// This is a **snapshot**: a consistent point-in-time view with no lock held
    /// across the return. Presence reflects exactly the holder's current held
    /// set: an `AllocationId` is present iff a leaf is currently held for it —
    /// `Some` facts after a hold, absent after a drop. It never mutates the
    /// held set and never re-issues.
    fn held_snapshot(&self) -> BTreeMap<AllocationId, HeldSvidFacts>;
}
