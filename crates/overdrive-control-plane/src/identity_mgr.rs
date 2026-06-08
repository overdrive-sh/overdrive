//! `IdentityMgr` — the in-process held-SVID store (ADR-0067 D4).
//!
//! The workload-identity feature holds each Running allocation's minted
//! [`SvidMaterial`] (cert + node-held leaf key + validity end) in-process, keyed
//! by [`AllocationId`], alongside the boot [`TrustBundle`]. This held set is
//! **ephemeral runtime state** — neither intent nor observation (whitepaper §4
//! state-layer hygiene): it is NEVER persisted (the leaf [`CaKeyPem`] has no
//! `Serialize` and is non-reconstructable, ADR-0063 D9), and it is rebuilt on
//! restart by re-issuing for every still-Running alloc (ADR-0067 rev 2 D1). The
//! `issued_certificates` audit row is the observation; `IdentityMgr` holds the
//! live credential the node agent uses for the TLS 1.3 handshake.
//!
//! # Role in the reconciler loop (ADR-0067 D4)
//!
//! `IdentityMgr` is the `actual` source for the pure `SvidLifecycle` reconciler
//! (01-04): its [`held_snapshot`](IdentityMgr::held_snapshot) yields a
//! [`HeldSvidFacts`] projection per held alloc, exactly as
//! `WorkflowEngine::live_instances()` yields the live-task set the
//! `WorkflowLifecycle` reconciler reads. The runtime's `hydrate_actual` calls
//! `held_snapshot` (sync, in-process — no `.await`) to build the reconciler's
//! `actual`. The action-shim executors (01-06) drive the mutators:
//! [`hold`](IdentityMgr::hold) on `IssueSvid`, [`drop_svid`](IdentityMgr::drop_svid)
//! on `DropSvid`.
//!
//! # Concurrency (ADR-0067 A7)
//!
//! Interior state lives behind a [`parking_lot::RwLock`] — NOT a `tokio::sync`
//! lock: every method grabs the guard, mutates-or-clones, and drops the guard
//! WITHIN the call (no guard is ever held across an `.await`, per
//! `.claude/rules/development.md` § "Concurrency & async"). The held map is a
//! [`BTreeMap`] (not `HashMap`) so [`held_snapshot`] and the held-set invariant
//! iterate in deterministic [`AllocationId`] order across DST seeds (K5;
//! § "Ordered-collection choice").

use std::collections::BTreeMap;

use overdrive_core::AllocationId;
use overdrive_core::reconcilers::HeldSvidFacts;
use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::identity_read::IdentityRead;
use parking_lot::RwLock;

/// The in-process held-SVID set + boot trust bundle (ADR-0067 D4).
///
/// Holds each Running allocation's minted [`SvidMaterial`] keyed by
/// [`AllocationId`], plus the [`TrustBundle`] composed at boot. The leaf private
/// key inside each held `SvidMaterial` never leaves this type — readers see only
/// the [`HeldSvidFacts`] projection via [`held_snapshot`](IdentityMgr::held_snapshot)
/// (K2 — leak resistance).
#[derive(Debug)]
pub struct IdentityMgr {
    state: RwLock<IdentityState>,
}

/// The interior held state behind [`IdentityMgr`]'s lock.
#[derive(Debug)]
struct IdentityState {
    /// Per-allocation held SVID material (cert + node-held leaf key +
    /// validity end). `BTreeMap` for deterministic iteration order (K5).
    held: BTreeMap<AllocationId, SvidMaterial>,
    /// The trust bundle composed at boot — the relying-party verification
    /// material. `None` until a bundle is installed.
    bundle: Option<TrustBundle>,
}

impl IdentityMgr {
    /// Construct an `IdentityMgr` with the boot [`TrustBundle`] and an empty
    /// held set. On a fresh process boot the held set is empty — every
    /// still-Running alloc reads as `¬held`, which is exactly the restart
    /// re-issue trigger the `SvidLifecycle` reconciler needs (ADR-0067 rev 2 D1).
    #[must_use]
    pub fn new(bundle: Option<TrustBundle>) -> Self {
        Self { state: RwLock::new(IdentityState { held: BTreeMap::new(), bundle }) }
    }

    /// Hold the minted SVID material for `alloc` — called by the `IssueSvid`
    /// executor (01-06) after `ca_issuance::issue_and_audit` succeeds. Replaces
    /// any prior material for the same alloc (re-issue overwrites; ADR-0067 D2).
    pub fn hold(&self, alloc: AllocationId, svid: SvidMaterial) {
        // Write-lock → mutate → drop the guard within the call (never across an
        // `.await`; § "Concurrency & async").
        let mut state = self.state.write();
        state.held.insert(alloc, svid);
    }

    /// Drop the held SVID for `alloc` — called by the `DropSvid` executor
    /// (01-06) once the alloc is no longer Running. Removing the entry makes the
    /// node-held leaf private key unreachable in the held set (ADR-0067 O2 —
    /// leak resistance on stop).
    pub fn drop_svid(&self, alloc: &AllocationId) {
        let mut state = self.state.write();
        state.held.remove(alloc);
    }

    /// Install (replace) the boot trust bundle.
    pub fn set_bundle(&self, bundle: TrustBundle) {
        let mut state = self.state.write();
        state.bundle = Some(bundle);
    }

    /// Snapshot the held set as a per-allocation [`HeldSvidFacts`] projection —
    /// the `actual` the `SvidLifecycle` reconciler (01-04) reads.
    ///
    /// Returns the PROJECTION (`spiffe_id` + `not_after`), NEVER the held
    /// [`SvidMaterial`] itself: the leaf private key stays inside `IdentityMgr`
    /// (K2). Sync (read-lock → clone the projection → drop the guard) so the
    /// runtime's `hydrate_actual` reads it without `.await`, mirroring
    /// `WorkflowEngine::live_instances()`. `BTreeMap` iteration order is
    /// deterministic across DST seeds (K5).
    #[must_use]
    pub fn held_snapshot(&self) -> BTreeMap<AllocationId, HeldSvidFacts> {
        let state = self.state.read();
        state
            .held
            .iter()
            .map(|(alloc, svid)| {
                (
                    alloc.clone(),
                    HeldSvidFacts {
                        spiffe_id: svid.spiffe_id().clone(),
                        not_after: svid.not_after(),
                    },
                )
            })
            .collect()
    }
}

/// The in-process held-identity read surface (ADR-0067 D7).
///
/// Both getters take a read-lock, clone the value out, and drop the guard
/// WITHIN the read expression — the caller holds no lock after the read returns
/// (D7 clause 4; § "Concurrency & async"). Neither getter touches the `Ca` (D7
/// clause 1 — the O3 read-latency promise): `svid_for` reads the held map and
/// `current_bundle` reads the HYDRATED bundle (D6), never `Ca::issue_svid` /
/// `Ca::trust_bundle`. Neither mutates (clause 2); `None` is explicit absence
/// (clause 3); and a dropped alloc reads back `None` (clause 5 / K2).
impl IdentityRead for IdentityMgr {
    fn svid_for(&self, alloc: &AllocationId) -> Option<SvidMaterial> {
        // Read-lock → clone the held material out → drop the guard as the
        // temporary `state` falls out of the expression. No re-issue: the SVID
        // is served from the held map, never minted (D7 clause 1).
        self.state.read().held.get(alloc).cloned()
    }

    fn current_bundle(&self) -> Option<TrustBundle> {
        // Read-lock → clone the hydrated bundle out → drop the guard. Zero CA
        // I/O on the read path (D6 / D7 clause 1).
        self.state.read().bundle.clone()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::IdentityMgr;
    use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
    use overdrive_core::wall_clock::UnixInstant;
    use overdrive_core::{AllocationId, CertSerial, SpiffeId};
    use std::time::Duration;

    /// Build a `SvidMaterial` for `(spiffe, not_after_secs)` with placeholder
    /// cert/key bytes — the held-set tests assert on the `held_snapshot`
    /// PROJECTION (`spiffe_id` + `not_after`), not the opaque cert/key bytes.
    fn svid(spiffe: &str, not_after_secs: u64) -> SvidMaterial {
        SvidMaterial::new(
            CaCertPem::new("-----BEGIN CERTIFICATE-----\nLEAF\n-----END CERTIFICATE-----\n".into()),
            CaCertDer::new(vec![0xDE, 0xAD]),
            CertSerial::new("0badc0de").expect("serial parses"),
            SpiffeId::new(spiffe).expect("valid SpiffeId"),
            CaKeyPem::new("-----BEGIN PRIVATE KEY-----\nKEY\n-----END PRIVATE KEY-----\n".into()),
            UnixInstant::from_unix_duration(Duration::from_secs(not_after_secs)),
        )
    }

    fn alloc(id: &str) -> AllocationId {
        AllocationId::new(id).expect("valid AllocationId")
    }

    fn bundle() -> TrustBundle {
        TrustBundle::new(
            CaCertPem::new("-----BEGIN CERTIFICATE-----\nROOT\n-----END CERTIFICATE-----\n".into()),
            None,
        )
    }

    /// `new(Some(bundle))` constructs with an empty held set — a fresh boot
    /// holds nothing, so every still-Running alloc reads as `¬held` (the
    /// restart re-issue trigger).
    #[test]
    fn new_constructs_with_empty_held_set() {
        let mgr = IdentityMgr::new(Some(bundle()));
        assert!(mgr.held_snapshot().is_empty(), "a fresh IdentityMgr holds no SVIDs");
    }

    /// `hold` then `held_snapshot` exposes the alloc with the projected
    /// `spiffe_id` + `not_after` — the faithful projection the reconciler reads.
    #[test]
    fn hold_then_snapshot_projects_identity_and_validity_end() {
        let mgr = IdentityMgr::new(None);
        let spiffe = "spiffe://overdrive.local/job/payments/alloc/a1b2c3";
        let not_after = 1_700_003_600;

        mgr.hold(alloc("alloc-a1b2c3-0"), svid(spiffe, not_after));

        let snapshot = mgr.held_snapshot();
        let facts = snapshot.get(&alloc("alloc-a1b2c3-0")).expect("held alloc is in the snapshot");
        assert_eq!(
            facts.spiffe_id,
            SpiffeId::new(spiffe).expect("valid SpiffeId"),
            "snapshot projects the held SVID's identity"
        );
        assert_eq!(
            facts.not_after,
            UnixInstant::from_unix_duration(Duration::from_secs(not_after)),
            "snapshot projects the held SVID's validity-window end"
        );
    }

    /// `drop_svid` removes the entry — the held leaf key is no longer reachable
    /// in the snapshot (O2 — leak resistance on stop).
    #[test]
    fn drop_svid_removes_the_alloc_from_the_snapshot() {
        let mgr = IdentityMgr::new(None);
        let a = alloc("alloc-a1b2c3-0");
        mgr.hold(
            a.clone(),
            svid("spiffe://overdrive.local/job/payments/alloc/a1b2c3", 1_700_003_600),
        );
        assert!(mgr.held_snapshot().contains_key(&a), "alloc is held after hold()");

        mgr.drop_svid(&a);

        assert!(
            !mgr.held_snapshot().contains_key(&a),
            "drop_svid removes the alloc — held leaf key no longer reachable in the snapshot"
        );
    }

    /// `held_snapshot` iterates in deterministic sorted `AllocationId` order
    /// (BTreeMap, K5) — the load-bearing DST-determinism property for the
    /// invariant + reconciler that iterate the held set across seeds.
    #[test]
    fn held_snapshot_iterates_in_sorted_allocation_order() {
        let mgr = IdentityMgr::new(None);
        // Insert out of sorted order.
        mgr.hold(
            alloc("alloc-zzz-0"),
            svid("spiffe://overdrive.local/job/z/alloc/zzz", 1_700_000_001),
        );
        mgr.hold(
            alloc("alloc-aaa-0"),
            svid("spiffe://overdrive.local/job/a/alloc/aaa", 1_700_000_002),
        );
        mgr.hold(
            alloc("alloc-mmm-0"),
            svid("spiffe://overdrive.local/job/m/alloc/mmm", 1_700_000_003),
        );

        let keys: Vec<AllocationId> = mgr.held_snapshot().into_keys().collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(
            keys, sorted,
            "held_snapshot keys are in deterministic sorted AllocationId order"
        );
    }

    /// `set_bundle` replaces the held trust bundle. The bundle is not exposed by
    /// `held_snapshot` (that is the held-SVID projection); the observable effect
    /// of `set_bundle` is exercised by the 02-01 `IdentityRead` impl. Here we
    /// assert the mutator does not panic and leaves the held SVID set untouched
    /// — bundle and held set are independent slots.
    #[test]
    fn set_bundle_does_not_disturb_the_held_svid_set() {
        let mgr = IdentityMgr::new(None);
        let a = alloc("alloc-a1b2c3-0");
        mgr.hold(
            a.clone(),
            svid("spiffe://overdrive.local/job/payments/alloc/a1b2c3", 1_700_003_600),
        );

        mgr.set_bundle(bundle());

        assert!(
            mgr.held_snapshot().contains_key(&a),
            "set_bundle leaves the held SVID set unchanged (independent slot)"
        );
    }
}
