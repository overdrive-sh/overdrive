//! [`WorkflowLiveSet`] — the live-workflow-instance snapshot read port
//! (ADR-0086 D5).
//!
//! One of the four narrow driven read-ports the reconciler hydration boundary
//! reads (ADR-0086 D5). The `WorkflowLifecycle` reconciler's hydration reads
//! WHICH workflow instances currently have a live task through this port — a
//! narrow read VIEW over the workflow engine, NOT the engine itself (ADR-0064
//! §5; the engine is a peer primitive and is deliberately not relocated). The
//! production impl is `WorkflowEngine` (control-plane); the DST impl is
//! `SimWorkflowLiveSet` (`overdrive-sim`, step 02-05), which makes the
//! empty-set crash-resume path injectable for the first time (ADR-0086 D8).
//!
//! **Sync**, not async: the read is an in-memory `ClaimSet::snapshot` (a
//! `parking_lot::Mutex` clone) with no `.await`, so the tick path pays no boxed
//! future per read. Per `.claude/rules/development.md` § "Trait definitions
//! specify behavior, not just signature" the method rustdoc below is the SSOT
//! the sim adapter and the DST equivalence test enforce against every impl.
//!
//! [`HydrationContext`]: crate::reconcilers::HydrationContext

use std::collections::BTreeSet;

use crate::id::CorrelationKey;

/// The live-workflow-instance snapshot read port (ADR-0086 D5).
///
/// A **driven, read-only** projection: the reconciler calls out to snapshot the
/// engine's live-task set; there is no start/stop method on this port (those
/// live on the engine — read/write split of Principle 12). Sync — the read is
/// an in-memory snapshot.
pub trait WorkflowLiveSet: Send + Sync {
    /// A point-in-time snapshot of the correlation keys of every workflow
    /// instance the engine currently has a **live task** for.
    ///
    /// # Preconditions
    /// None.
    ///
    /// # Postconditions
    /// Returns an **owned** [`BTreeSet`] cloned from the engine's live-task
    /// registry as of the moment of the call — ephemeral runtime state, NOT
    /// intent or observation. The registry is unchanged (pure read); no lock is
    /// held across the return. [`BTreeSet`] iteration order is deterministic
    /// across DST seeds (K5).
    ///
    /// # Edge cases
    /// An **empty set is legitimate**, not an error — most notably right after
    /// a process restart, when every previously-running instance reads as
    /// "no live task". An instance that is running-in-intent, absent from this
    /// set, AND has no terminal observation row IS the crash-resume trigger the
    /// `WorkflowLifecycle` reconciler re-emits `StartWorkflow` for (ADR-0064
    /// §5). The caller must never treat emptiness as a failure.
    ///
    /// # Observable invariants
    /// This is a **snapshot**: it is a consistent point-in-time view, and the
    /// interior claim-set lock is never held across an `.await` (the snapshot is
    /// sync). Membership reflects exactly the engine's current live tasks:
    /// a correlation key is present iff the engine holds a live task claim for
    /// it. It never mutates the registry.
    fn live_instances(&self) -> BTreeSet<CorrelationKey>;
}
