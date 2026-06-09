//! `SimIdentityRead` — in-memory [`IdentityRead`] double for DST (ADR-0067 D7).
//!
//! The sim counterpart to the host `IdentityMgr` (`overdrive-control-plane`):
//! it serves the held-identity read surface from a **preloaded**
//! `BTreeMap<AllocationId, SvidMaterial>` + `Option<TrustBundle>`, so a DST
//! consumer (or the `identity_read_equivalence` structural guard) can drive the
//! same `IdentityRead` calls against both adapters and assert identical
//! observable reads.
//!
//! # Why a preloaded double (not a live holder)
//!
//! `IdentityMgr` is the live, mutable holder (`hold` / `drop_svid` /
//! `set_bundle`) the action-shim executors drive during convergence. The sim
//! double does NOT model those mutators — under DST the held set's *content* is
//! whatever the scenario preloads; the read surface (`svid_for` /
//! `current_bundle`) is the only behaviour the consumer-facing port exposes
//! (D7). A "drop" is modelled by constructing a double WITHOUT that entry — the
//! preloaded map IS the post-mutation snapshot the scenario wants to read
//! against. This keeps the double to exactly the two-getter `IdentityRead`
//! surface with no invented mutator surface the trait does not name.
//!
//! # Determinism
//!
//! The held map is a [`BTreeMap`] (not `HashMap`) — the equivalence guard walks
//! both adapters' held sets, and `IdentityMgr` is itself `BTreeMap`-backed
//! (ADR-0067 A7; § "Ordered-collection choice"), so the iteration order matches
//! across adapters bit-for-bit. `SimIdentityRead` carries no entropy / clock /
//! crypto — it holds the fixture material opaquely (research Finding 11), so the
//! read surface is deterministic by construction.
//!
//! # Dependency discipline
//!
//! `SimIdentityRead::new` takes the preloaded map + bundle as **required
//! constructor parameters** — no builder, no production-binding default
//! (`.claude/rules/development.md` § "Port-trait dependencies").

use std::collections::BTreeMap;

use overdrive_core::AllocationId;
use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::identity_read::IdentityRead;

/// In-memory [`IdentityRead`] double for DST.
///
/// Holds a preloaded per-allocation [`SvidMaterial`] set keyed by
/// [`AllocationId`] and an optional [`TrustBundle`]. `Send + Sync` (both fields
/// are `Send + Sync`), matching the sibling sim adapters, so it can be shared
/// across async tasks as `Arc<dyn IdentityRead>`.
#[derive(Debug, Clone)]
pub struct SimIdentityRead {
    /// Preloaded held set. `BTreeMap` for deterministic iteration order — the
    /// equivalence guard walks this against `IdentityMgr`'s own `BTreeMap`.
    held: BTreeMap<AllocationId, SvidMaterial>,
    /// Preloaded hydrated trust bundle. `None` models explicit absence (D7
    /// clause 3) — no bundle has been installed.
    bundle: Option<TrustBundle>,
}

impl SimIdentityRead {
    /// Construct a `SimIdentityRead` over a **required** preloaded held set and
    /// optional trust bundle.
    ///
    /// No builder, no default — both the held map and the bundle are mandatory
    /// constructor inputs so a scenario that forgets to preload one fails to
    /// compile (`.claude/rules/development.md` § "Port-trait dependencies"). A
    /// scenario that wants an empty held set passes `BTreeMap::new()`; one that
    /// wants no bundle passes `None`.
    #[must_use]
    pub const fn new(
        held: BTreeMap<AllocationId, SvidMaterial>,
        bundle: Option<TrustBundle>,
    ) -> Self {
        Self { held, bundle }
    }
}

/// The in-process held-identity read surface (ADR-0067 D7), served from the
/// preloaded snapshot.
///
/// Both getters return owned clones of the preloaded material (D7 clause 4 — no
/// lock crosses the call; the sim holds no lock at all). Neither touches a `Ca`
/// (clause 1 — there is none) and neither mutates the held set or bundle (clause
/// 2). `None` is explicit absence (clause 3). A "dropped" alloc is one absent
/// from the preloaded map, so it reads `None` (clause 5 / K2) — exactly the
/// post-drop observable the host `IdentityMgr::drop_svid` produces.
impl IdentityRead for SimIdentityRead {
    fn svid_for(&self, alloc: &AllocationId) -> Option<SvidMaterial> {
        // Owned clone of the held material, or explicit absence. No re-issue,
        // no mutation — the SVID is served from the preloaded map (clause 1/2).
        self.held.get(alloc).cloned()
    }

    fn current_bundle(&self) -> Option<TrustBundle> {
        // Owned clone of the hydrated bundle, or explicit absence (clause 3/4).
        self.bundle.clone()
    }
}
