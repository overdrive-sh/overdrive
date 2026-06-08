//! `svid-lifecycle` — the workload-SVID reconciler primitives (ADR-0067).
//!
//! This module is the home for the pure `SvidLifecycle` reconciler that
//! converges the in-process held-SVID set against the Running-allocation set
//! (`running ∧ ¬held → IssueSvid`, `¬running ∧ held → DropSvid`; ADR-0067 D2).
//! The reconciler, its `State` projection, and its retry-memory `View` land in
//! step 01-04; this step (01-03) defines ONLY the projection the held set
//! yields into the reconciler's `actual`:
//!
//! [`HeldSvidFacts`] — the per-allocation *projection* of a held
//! [`SvidMaterial`](crate::traits::ca::SvidMaterial) the
//! `IdentityMgr::held_snapshot` surface returns. It carries the two facts the
//! reconciler's `running ∧ ¬held` and near-expiry decisions read — the
//! `spiffe_id` and the `not_after` validity end — and DELIBERATELY NOT the leaf
//! private key: the key never leaves `IdentityMgr` (ADR-0067 K2 — leak
//! resistance; the held `SvidMaterial`'s `leaf_key` stays inside the holder).
//!
//! # Why a projection, not the full `SvidMaterial`
//!
//! The reconciler's `actual` must be a pure value the runtime can hold and
//! compare. Projecting to the two non-secret facts (a) keeps the node-held leaf
//! key off the reconciler's input surface entirely, and (b) gives the
//! near-expiry seam (ADR-0067 rev 3 D8) the `not_after` it compares against
//! `tick.now_unix` — a value that, post the ADR-0063 rev 2 amendment, equals
//! the `issued_certificates` audit row's `not_after` and derives from the same
//! injected clock, so the comparison is sound and DST-deterministic.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::SpiffeId;
use crate::id::{AllocationId, ContentHash, CorrelationKey, NodeId, WorkloadId};
use crate::wall_clock::UnixInstant;

use super::{Action, Reconciler, ReconcilerName, TickContext};

/// The per-allocation projection of a held workload SVID — the `actual` the
/// `SvidLifecycle` reconciler (01-04) reads via `IdentityMgr::held_snapshot`.
///
/// Carries the two non-secret facts the reconciler's decisions consume:
///
/// * `spiffe_id` — the identity the held leaf was minted for (the
///   `running ∧ ¬held` branch compares the desired identity against this).
/// * `not_after` — the held leaf's validity-window end (the near-expiry seam,
///   ADR-0067 rev 3 D8, compares this against `tick.now_unix`). An OBSERVED
///   FACT of the minted credential, equal to the `issued_certificates` row's
///   `not_after` by construction (ADR-0063 rev 2 amendment) — NOT a
///   recompute-from-policy deadline.
///
/// It DELIBERATELY does NOT carry the leaf private key: the
/// [`CaKeyPem`](crate::traits::ca::CaKeyPem) stays inside `IdentityMgr` (K2 —
/// the held secret is never projected into a reconciler input). `HeldSvidFacts`
/// derives `Debug`/`Clone`/`PartialEq`/`Eq` because the reconciler runtime
/// holds, clones, and diffs `actual` values; both fields are non-secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldSvidFacts {
    /// The identity the held leaf was minted for.
    pub spiffe_id: SpiffeId,
    /// The held leaf's validity-window end.
    pub not_after: UnixInstant,
}

/// The per-allocation `desired` fact the `SvidLifecycle` reconciler needs to
/// emit [`Action::IssueSvid`] for a Running allocation — the inputs to the pure
/// `SpiffeId::for_allocation` derivation plus the issuing node.
///
/// The runtime's hydrate-desired projects one of these per Running
/// `alloc_status` observation row (filtered to the target workload), exactly as
/// the `WorkloadLifecycle` / `BackendDiscoveryBridge` arms project the running
/// set. The `AllocationId` is the [`SvidLifecycleState::desired`] map key;
/// `RunningAlloc` carries the remaining two fields the issuance request names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningAlloc {
    /// The workload the allocation belongs to — the first component of the
    /// derived `spiffe://overdrive.local/job/<workload>/alloc/<alloc>` identity.
    pub workload_id: WorkloadId,
    /// The node the allocation runs on — carried on `IssueSvid` so the action is
    /// self-describing (the `issued_certificates` row's `node_id`, ADR-0067 D2).
    pub node_id: NodeId,
}

/// `desired` / `actual` projection for the `SvidLifecycle` reconciler
/// (ADR-0067 D1/D4).
///
/// As with [`WorkflowLifecycleState`](super::WorkflowLifecycleState), ONE
/// `State` type is instantiated by the runtime in two roles:
///
/// * **`desired`** — `desired` carries the currently-**Running** allocations
///   (`actual` is ignored on this value), keyed by [`AllocationId`].
/// * **`actual`** — `actual` carries the [`IdentityMgr`]-held snapshot
///   (`desired` is ignored on this value), keyed by [`AllocationId`].
///
/// `reconcile` reads `desired.desired` and `actual.actual` to converge the two
/// sets. Both maps are [`BTreeMap`] for deterministic iteration across DST seeds
/// per `.claude/rules/development.md` § "Ordered-collection choice" — the
/// reconcile body iterates them and the held-set invariant walks the result.
///
/// [`IdentityMgr`]: ../../../overdrive_control_plane/identity_mgr/struct.IdentityMgr.html
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SvidLifecycleState {
    /// The Running-allocation set (the `desired` role). Keyed by
    /// [`AllocationId`]; the value carries the issuance-request fields.
    pub desired: BTreeMap<AllocationId, RunningAlloc>,
    /// The held-SVID snapshot (the `actual` role) — presence of a key means the
    /// allocation is currently held; the value is the non-secret
    /// [`HeldSvidFacts`] projection (`IdentityMgr::held_snapshot`).
    pub actual: BTreeMap<AllocationId, HeldSvidFacts>,
}

/// Typed memory for the `SvidLifecycle` reconciler.
///
/// Slice 01 carries NO memory — the issue/drop decision is a pure function of
/// `desired` vs `actual` (running vs held). The struct exists for the
/// [`Reconciler::View`] associated-type contract and grows additively to the
/// retry-memory shape (`IssueRetry { attempts, last_failure_seen_at }`) in step
/// 03-01 per `development.md` § "Persist inputs, not derived state". The six
/// derives (incl. `Eq`) satisfy the trait bound and the runtime's NextView
/// Eq-diff.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SvidLifecycleView {}

/// The workload-SVID lifecycle reconciler (ADR-0067 D1).
pub struct SvidLifecycle {
    name: ReconcilerName,
}

impl SvidLifecycle {
    /// Construct the canonical `svid-lifecycle` instance.
    ///
    /// # Panics
    ///
    /// Never — `Self::NAME` is a compile-time string literal satisfying every
    /// `ReconcilerName` validation rule.
    #[must_use]
    pub fn canonical() -> Self {
        #[allow(clippy::expect_used)]
        let name = ReconcilerName::new(<Self as Reconciler>::NAME)
            .expect("'svid-lifecycle' is a valid ReconcilerName by construction");
        Self { name }
    }
}

/// Derive the deterministic [`CorrelationKey`] for an identity action against an
/// allocation: `target = "svid-lifecycle/<alloc>"`, `spec_hash =
/// ContentHash::of(<spiffe-uri bytes>)`, `purpose ∈ {"issue-svid",
/// "drop-svid"}` (ADR-0067 D2 — the ADR-0035 reconciler-I/O correlation
/// discipline; the content identity is the SVID's own SPIFFE URI, stable across
/// ticks for the same allocation, NOT a per-attempt request id).
fn identity_correlation(
    alloc: &AllocationId,
    spiffe_id: &SpiffeId,
    purpose: &str,
) -> CorrelationKey {
    let target = format!("svid-lifecycle/{alloc}");
    let spec_hash = ContentHash::of(spiffe_id.as_str().as_bytes());
    CorrelationKey::derive(&target, &spec_hash, purpose)
}

impl Reconciler for SvidLifecycle {
    /// Canonical kebab-case name; single compile-time anchor.
    const NAME: &'static str = "svid-lifecycle";

    type State = SvidLifecycleState;
    type View = SvidLifecycleView;

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    /// Pure-sync `reconcile` (ADR-0035 / ADR-0067 D1). Converges
    /// `desired = Running allocs` (`desired.desired`) against
    /// `actual = held set` (`actual.actual`):
    ///
    /// - **`running ∧ ¬held`** → emit [`Action::IssueSvid`] carrying the
    ///   pure-derived [`SpiffeId::for_allocation`] identity, the issuing
    ///   `node_id`, and the derived `issue-svid` correlation. On a control-plane
    ///   restart the held set is empty, so every Running alloc takes this branch
    ///   — restart recovery falls out for free (ADR-0067 D1, RECOVERY).
    /// - **`¬running ∧ held`** → emit [`Action::DropSvid`] so the executor drops
    ///   the held leaf key (ADR-0067 O2 — leak resistance on stop).
    /// - **`running ∧ held`** → no-op (the alloc is held and still desired;
    ///   near-expiry rotation is the gated #40 branch, step 03-02 — NOT here).
    ///
    /// The body holds no `.await`, reads no wall-clock (the issue/drop decision
    /// is time-independent in Slice 01 — `_tick` is unused), consults no RNG,
    /// and holds no CA / ObservationStore handle — it builds the `SpiffeId`
    /// purely and passes it in the action; CA I/O is the executor's (D3).
    /// dst-lint holds.
    fn reconcile(
        &self,
        desired: &Self::State,
        actual: &Self::State,
        _view: &Self::View,
        _tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        let mut actions: Vec<Action> = Vec::new();

        // running ∧ ¬held → IssueSvid.
        for (alloc_id, running) in &desired.desired {
            if actual.actual.contains_key(alloc_id) {
                // running ∧ held → no-op (gated near-expiry rotation is #40).
                continue;
            }
            let spiffe_id = SpiffeId::for_allocation(&running.workload_id, alloc_id);
            let correlation = identity_correlation(alloc_id, &spiffe_id, "issue-svid");
            actions.push(Action::IssueSvid {
                alloc_id: alloc_id.clone(),
                spiffe_id,
                node_id: running.node_id.clone(),
                correlation,
            });
        }

        // ¬running ∧ held → DropSvid.
        for (alloc_id, held) in &actual.actual {
            if desired.desired.contains_key(alloc_id) {
                continue;
            }
            let correlation = identity_correlation(alloc_id, &held.spiffe_id, "drop-svid");
            actions.push(Action::DropSvid { alloc_id: alloc_id.clone(), correlation });
        }

        // The §18 self-re-enqueue gate treats an all-Noop vector as "converged
        // this tick"; emit a single Noop when nothing needed doing so the gate
        // reads the converged shape (mirrors `WorkflowLifecycle::reconcile`).
        if actions.is_empty() {
            actions.push(Action::Noop);
        }

        (actions, SvidLifecycleView::default())
    }
}

#[cfg(test)]
mod tests {
    use super::HeldSvidFacts;
    use crate::SpiffeId;
    use crate::wall_clock::UnixInstant;
    use std::time::Duration;

    /// `HeldSvidFacts` is a faithful two-field projection: constructing it from a
    /// `spiffe_id` + `not_after` exposes exactly those two values back through
    /// its public fields. This pins the projection shape the
    /// `IdentityMgr::held_snapshot` surface produces (01-03) and the
    /// `SvidLifecycle` reconciler `actual` consumes (01-04) — a regression that
    /// dropped or swapped a field is caught here.
    #[test]
    fn held_svid_facts_carries_the_identity_and_validity_end() {
        let spiffe = SpiffeId::new("spiffe://overdrive.local/job/payments/alloc/a1b2c3")
            .expect("valid workload SpiffeId");
        let not_after = UnixInstant::from_unix_duration(Duration::from_secs(1_700_003_600));

        let facts = HeldSvidFacts { spiffe_id: spiffe.clone(), not_after };

        assert_eq!(facts.spiffe_id, spiffe, "projection preserves the held identity");
        assert_eq!(facts.not_after, not_after, "projection preserves the validity-window end");
    }
}
