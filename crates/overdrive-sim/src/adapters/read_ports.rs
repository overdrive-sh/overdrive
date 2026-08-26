//! Sim doubles for the four ADR-0086 D5 hydration read-ports.
//!
//! ADR-0086 (reconcilers own their hydration) moves the intent + observation
//! hydration onto the `Reconciler::hydrate_*` trait methods, which read every
//! fact through a `HydrationContext` borrow-bundle of injected ports instead of
//! concrete `AppState` fields. Four of those ports are NEW narrow read-traits in
//! `overdrive-core`:
//!
//! | Core trait | Sim double (here) | Production impl (up) |
//! |---|---|---|
//! | [`ListenerFacts`] | [`SimListenerFacts`] | `ListenerFactStore` (control-plane) |
//! | [`ServiceVipView`] | [`SimServiceVipView`] | `PersistentServiceVipAllocator` (dataplane) |
//! | [`WorkflowLiveSet`] | [`SimWorkflowLiveSet`] | `WorkflowEngine` (control-plane) |
//! | [`HeldSvidView`] | [`SimHeldSvidView`] | `IdentityMgr` (control-plane) |
//!
//! These four doubles make the hydration boundary **DST-injectable for the first
//! time** (ADR-0086 D8): a scenario can seed a stale/empty [`SimWorkflowLiveSet`]
//! (crash-resume convergence), a missing [`SimServiceVipView`] memo (ADR-0049 §4
//! defer path), a drifted [`SimListenerFacts`] fact (ADR-0060 C3 skip-not-default),
//! or a global [`SimHeldSvidView`] set (ADR-0067 D5b hydrator filter) — none of
//! which the central concrete-`AppState` free functions allowed.
//!
//! # Earned-Trust posture (ADR-0086 D8 / Principle 13)
//!
//! Each double wraps an **in-process** snapshot ([`BTreeMap`]/[`BTreeSet`] per
//! `.claude/rules/development.md` § "Ordered-collection choice"). There is NO
//! external substrate (fs / network / subprocess / kernel) that could lie, so an
//! Earned-Trust `probe()` on these ports is **degenerate** — construction by the
//! composition root already guarantees presence. Consequently the four core
//! read-port traits declare NO `probe()` method, and these doubles add none: the
//! Earned-Trust value here is the *sim-injectability* of the hydration boundary,
//! not a substrate probe. The runtime's existing `ViewStore::probe` boot gate
//! (ADR-0035 §5) is a separate concern and is unchanged.
//!
//! # Dependency discipline
//!
//! Every double takes its preloaded snapshot as a **required constructor
//! parameter** — no builder, no production-binding default (`.claude/rules/
//! development.md` § "Port-trait dependencies"). A scenario that wants an empty
//! surface passes `BTreeMap::new()` / `BTreeSet::new()` explicitly (mirrors the
//! sibling `SimIdentityRead`). The snapshot IS the post-mutation state the
//! scenario wants to read against; there is no mutator surface the traits do not
//! name.
//!
//! # Determinism
//!
//! Every field is a [`BTreeMap`] / [`BTreeSet`] (not `HashMap` / `HashSet`), so
//! iteration order is deterministic across DST seeds (K5) — matching the
//! `BTreeMap`-backed production impls (`ListenerFactStore`, `IdentityMgr`,
//! `WorkflowEngine`'s `ClaimSet::snapshot`) bit-for-bit at the read surface.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use overdrive_core::id::{AllocationId, ContentHash, CorrelationKey, ServiceId, ServiceVip};
use overdrive_core::identity::HeldSvidFacts;
use overdrive_core::traits::observation_store::ListenerRow;
use overdrive_core::traits::{HeldSvidView, ListenerFacts, ServiceVipView, WorkflowLiveSet};

// ---------------------------------------------------------------------------
// SimListenerFacts — the per-ServiceId listener-fact read port (ADR-0086 D5).
// ---------------------------------------------------------------------------

/// In-memory [`ListenerFacts`] double for DST.
///
/// Serves the per-`ServiceId` listener fact from a preloaded
/// `BTreeMap<ServiceId, ListenerRow>`. A `ServiceId` absent from the map reads
/// `None` — the exact "skip this service, never default `Proto::Tcp`" edge the
/// production `ListenerFactStore` produces for an unknown key (ADR-0060 C3).
#[derive(Debug, Clone, Default)]
pub struct SimListenerFacts {
    /// Preloaded per-`ServiceId` facts. `BTreeMap` for deterministic order.
    facts: BTreeMap<ServiceId, ListenerRow>,
}

impl SimListenerFacts {
    /// Construct over a **required** preloaded per-`ServiceId` fact map.
    ///
    /// No builder, no production-binding default: a scenario that wants an empty
    /// surface passes `BTreeMap::new()` (`.claude/rules/development.md` §
    /// "Port-trait dependencies").
    #[must_use]
    pub const fn new(facts: BTreeMap<ServiceId, ListenerRow>) -> Self {
        Self { facts }
    }
}

#[async_trait]
impl ListenerFacts for SimListenerFacts {
    async fn fact_for(&self, service_id: ServiceId) -> Option<ListenerRow> {
        // Owned copy of the held fact (or explicit absence). Pure read; no
        // mutation, no default (C3). `ListenerRow` is `Copy`.
        self.facts.get(&service_id).copied()
    }
}

// ---------------------------------------------------------------------------
// SimServiceVipView — the assigned-VIP read port over the allocator memo.
// ---------------------------------------------------------------------------

/// In-memory [`ServiceVipView`] double for DST.
///
/// Serves the allocator-issued VIP from a preloaded
/// `BTreeMap<ContentHash, ServiceVip>` keyed by content-addressed spec digest. A
/// digest absent from the map reads `None` — the ADR-0049 §4
/// structural-invariant-violation "defer the tick" signal the hydrator treats as
/// "no `Action`, no default VIP".
#[derive(Debug, Clone, Default)]
pub struct SimServiceVipView {
    /// Preloaded VIP memo keyed by spec digest. `BTreeMap` for determinism.
    memo: BTreeMap<ContentHash, ServiceVip>,
}

impl SimServiceVipView {
    /// Construct over a **required** preloaded spec-digest → VIP memo.
    ///
    /// A scenario that wants a memo-absent surface passes `BTreeMap::new()`.
    #[must_use]
    pub const fn new(memo: BTreeMap<ContentHash, ServiceVip>) -> Self {
        Self { memo }
    }
}

#[async_trait]
impl ServiceVipView for SimServiceVipView {
    async fn assigned_vip(&self, spec_digest: &ContentHash) -> Option<ServiceVip> {
        // Owned VIP (or explicit absence). Pure read; allocates no VIP, mutates
        // no memo. The core `ContentHash` IS the allocator's 32-byte digest.
        self.memo.get(spec_digest).copied()
    }
}

// ---------------------------------------------------------------------------
// SimWorkflowLiveSet — the live-workflow-instance snapshot read port.
// ---------------------------------------------------------------------------

/// In-memory [`WorkflowLiveSet`] double for DST.
///
/// Serves the engine's live-task correlation-key set from a preloaded
/// `BTreeSet<CorrelationKey>`. An **empty** set is legitimate (not an error) —
/// most notably after a process restart, when every previously-running instance
/// reads as "no live task"; the injectable empty set is exactly the crash-resume
/// trigger the `WorkflowLifecycle` reconciler re-emits `StartWorkflow` for
/// (ADR-0064 §5).
#[derive(Debug, Clone, Default)]
pub struct SimWorkflowLiveSet {
    /// Preloaded live-task correlation keys. `BTreeSet` for determinism.
    live: BTreeSet<CorrelationKey>,
}

impl SimWorkflowLiveSet {
    /// Construct over a **required** preloaded live-task set.
    ///
    /// A scenario modelling a post-restart engine passes `BTreeSet::new()`.
    #[must_use]
    pub const fn new(live: BTreeSet<CorrelationKey>) -> Self {
        Self { live }
    }
}

impl WorkflowLiveSet for SimWorkflowLiveSet {
    fn live_instances(&self) -> BTreeSet<CorrelationKey> {
        // Owned clone of the snapshot. Pure read; never mutates the registry.
        self.live.clone()
    }
}

// ---------------------------------------------------------------------------
// SimHeldSvidView — the global node-held SVID snapshot read port.
// ---------------------------------------------------------------------------

/// In-memory [`HeldSvidView`] double for DST.
///
/// Serves the **GLOBAL** node-held SVID set from a preloaded
/// `BTreeMap<AllocationId, HeldSvidFacts>` — every workload's held leaf, keyed by
/// [`AllocationId`]. The map is the unfiltered global set **by contract**
/// (ADR-0067 D5b): filtering to a target workload (by `SpiffeId::for_allocation`
/// equality) is the HYDRATOR's job, not this port's, so the double over-returns
/// deliberately.
#[derive(Debug, Clone, Default)]
pub struct SimHeldSvidView {
    /// Preloaded global held set. `BTreeMap` for determinism (matches
    /// `IdentityMgr`'s own `BTreeMap` backing).
    held: BTreeMap<AllocationId, HeldSvidFacts>,
}

impl SimHeldSvidView {
    /// Construct over a **required** preloaded global held map.
    ///
    /// A scenario modelling a fresh restart (nothing held) passes
    /// `BTreeMap::new()`.
    #[must_use]
    pub const fn new(held: BTreeMap<AllocationId, HeldSvidFacts>) -> Self {
        Self { held }
    }
}

impl HeldSvidView for SimHeldSvidView {
    fn held_snapshot(&self) -> BTreeMap<AllocationId, HeldSvidFacts> {
        // Owned clone of the GLOBAL snapshot (the leaf private key is never
        // projected — K2). Pure read; never mutates the held set.
        self.held.clone()
    }
}
