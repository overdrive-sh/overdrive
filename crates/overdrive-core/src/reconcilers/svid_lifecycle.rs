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

use crate::SpiffeId;
use crate::wall_clock::UnixInstant;

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
