//! Core workload-identity value types shared across the reconciler contract
//! and its adapters.
//!
//! [`HeldSvidFacts`] lives here (relocated out of
//! `reconcilers::svid_lifecycle` per ADR-0086 D6) because it crosses a **core**
//! read-port trait signature — [`HeldSvidView::held_snapshot`] returns
//! `BTreeMap<AllocationId, HeldSvidFacts>`. The general rule (ADR-0086 D6): any
//! value type appearing in a core read-trait signature stays in core;
//! reconciler-private projections do not. `IdentityMgr` (control-plane) and the
//! `svid-lifecycle` reconciler both import it from here.
//!
//! [`HeldSvidView::held_snapshot`]: crate::traits::HeldSvidView::held_snapshot

use crate::SpiffeId;
use crate::wall_clock::UnixInstant;

/// The per-allocation projection of a held workload SVID — the `actual` the
/// `SvidLifecycle` reconciler reads via
/// [`HeldSvidView::held_snapshot`](crate::traits::HeldSvidView::held_snapshot)
/// (ADR-0067 D1/D4; ADR-0086 D6 relocation).
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
