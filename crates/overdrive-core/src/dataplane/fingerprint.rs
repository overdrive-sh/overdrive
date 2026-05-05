//! `BackendSetFingerprint` — content-hash of a `(ServiceVip,
//! &[Backend])` pair.
//!
//! Identifies a unique backend-set state for convergence detection
//! in the `ServiceMapHydrator` reconciler and for LWW resolution in
//! the `service_hydration_results` ObservationStore table per
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md`
//! § 6 *Type aliases*.
//!
//! Type alias rather than STRICT newtype because the value is
//! derived (a hash), never operator-typed; it has no canonical
//! string form (no `Display` / `FromStr`); the existing
//! `CorrelationKey::derive` precedent in `crate::id` is the
//! project's pattern for content-derived numeric identifiers.
//!
//! The hashing-determinism rule
//! (`.claude/rules/development.md` § Hashing requires
//! deterministic serialization) governs how the value is
//! computed — rkyv-archived bytes, blake3 keyed hash, truncated
//! to u64 — not the type's wire shape.
//!
//! **RED scaffold** — the [`fingerprint`] body panics until
//! DELIVER fills it (Slice 04 / Slice 08 per the carpaccio plan).

use crate::traits::dataplane::Backend;
// `ServiceVip` is part of the additive `crate::id` extension that
// DELIVER lands as part of Slice 02 / Slice 04. The placeholder
// import path here is the same path the DESIGN locks in.
// SAFETY: this is a RED scaffold — DELIVER fills the import surface
// when `ServiceVip` lands.
use crate::id::ServiceVip;

/// Content-hash of a `(ServiceVip, &[Backend])` pair, truncated to
/// `u64`.
pub type BackendSetFingerprint = u64;

/// Compute the canonical content-hash of a backend set keyed by
/// VIP. Bit-identical across nodes given identical inputs (the
/// rkyv archive is canonical by construction).
///
/// Truncates blake3's 256-bit digest to the first 8 bytes
/// (little-endian) — the cluster-lifetime collision probability at
/// O(1k) services × O(1k) churn-per-service is negligible.
///
/// **RED scaffold** — DELIVER fills this body per Slice 04 / 08.
/// See test-scenarios.md S-2.2-12 (Maglev determinism), S-2.2-26
/// (hydrator convergence).
pub fn fingerprint(_vip: &ServiceVip, _backends: &[Backend]) -> BackendSetFingerprint {
    todo!("RED scaffold: dataplane::fingerprint — see Slice 04 / Slice 08")
}
