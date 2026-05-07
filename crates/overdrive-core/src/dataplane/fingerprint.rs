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
//! computed: internal data is canonicalised via rkyv archival —
//! rkyv's archived bytes are canonical by construction — and the
//! archived slice is fed directly into blake3. blake3 and the
//! truncate-to-`u64` little-endian shape are independent of the
//! canonicalisation step (architecture.md § 7).

use crate::id::ServiceVip;
use crate::traits::dataplane::Backend;

/// Content-hash of a `(ServiceVip, &[Backend])` pair, truncated to
/// `u64`.
pub type BackendSetFingerprint = u64;

/// rkyv envelope for the `(ServiceVip, &[Backend])` pair the
/// fingerprint covers. Owns clones of the inputs so a single
/// `rkyv::to_bytes` call sees one rooted aggregate. Cloning is
/// acceptable here: fingerprinting runs once per backend-set
/// change in the hydrator, not in the dataplane hot path.
#[derive(rkyv::Archive, rkyv::Serialize)]
struct FingerprintInput {
    vip: ServiceVip,
    backends: Vec<Backend>,
}

/// Compute the canonical content-hash of a backend set keyed by
/// VIP. Bit-identical across nodes given identical inputs.
///
/// Truncates blake3's 256-bit digest to the first 8 bytes
/// (little-endian) — the cluster-lifetime collision probability at
/// O(1k) services × O(1k) churn-per-service is negligible.
///
/// # Determinism
///
/// The value is the blake3 digest of the rkyv-archived
/// `FingerprintInput { vip, backends }` envelope. rkyv 0.8 archives
/// are canonical by construction (`.claude/rules/development.md`
/// § *Internal data → rkyv*), so the byte feed into blake3 is
/// deterministic across processes, runs, and seeds without any
/// hand-rolled field ordering.
///
/// `Backend` order is observed by the caller — the hydrator passes
/// backends in the deterministic `BTreeMap<BackendId, Backend>`
/// iteration order per architecture.md § 7. rkyv archives slices
/// in element order, so a reordered slice produces a different
/// fingerprint by construction.
#[must_use]
pub fn fingerprint(vip: &ServiceVip, backends: &[Backend]) -> BackendSetFingerprint {
    let input = FingerprintInput { vip: *vip, backends: backends.to_vec() };
    // rkyv archival of `FingerprintInput` is structurally infallible:
    // every field is an owned, sized value (ServiceVip is Copy; Vec<Backend>
    // owns its backends). The only path to `Err(rkyv::rancor::Error)` is
    // allocator failure during the archive scratch buffer, which on a
    // control-plane host means the process is already in OOM territory and
    // panicking is the correct response. `.expect` here documents that
    // contract; it is not a TODO.
    #[allow(clippy::expect_used)]
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&input)
        .expect("rkyv archival of FingerprintInput is infallible — fields are owned values");
    let digest = blake3::hash(&archived);
    let bytes = digest.as_bytes();
    let prefix: [u8; 8] =
        [bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]];
    u64::from_le_bytes(prefix)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use super::*;
    use crate::id::SpiffeId;

    fn vip_v4(a: u8, b: u8, c: u8, d: u8) -> ServiceVip {
        ServiceVip::new(IpAddr::V4(Ipv4Addr::new(a, b, c, d))).unwrap()
    }

    fn backend(spiffe: &str, addr: SocketAddr, weight: u16, healthy: bool) -> Backend {
        Backend { alloc: SpiffeId::new(spiffe).unwrap(), addr, weight, healthy }
    }

    fn sample_backends() -> Vec<Backend> {
        vec![
            backend(
                "spiffe://overdrive.local/job/payments/alloc/aaa",
                SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 8080),
                100,
                true,
            ),
            backend(
                "spiffe://overdrive.local/job/payments/alloc/bbb",
                SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 11)), 8080),
                100,
                true,
            ),
        ]
    }

    #[test]
    fn fingerprint_is_deterministic_across_calls() {
        let vip = vip_v4(10, 0, 0, 1);
        let backends = sample_backends();
        let a = fingerprint(&vip, &backends);
        let b = fingerprint(&vip, &backends);
        assert_eq!(a, b, "same inputs MUST produce same fingerprint");
    }

    #[test]
    fn fingerprint_is_sensitive_to_vip() {
        let backends = sample_backends();
        let a = fingerprint(&vip_v4(10, 0, 0, 1), &backends);
        let b = fingerprint(&vip_v4(10, 0, 0, 2), &backends);
        assert_ne!(a, b, "different VIPs must produce different fingerprints");
    }

    #[test]
    fn fingerprint_is_sensitive_to_backend_order() {
        // Per architecture.md § 7 the hydrator constructs `Vec<Backend>`
        // in deterministic `BTreeMap<BackendId, Backend>::iter()` order.
        // The fingerprint is responsible for hashing what it's given;
        // a reordered slice produces a different fingerprint by
        // construction.
        let vip = vip_v4(10, 0, 0, 1);
        let mut backends = sample_backends();
        let a = fingerprint(&vip, &backends);
        backends.reverse();
        let b = fingerprint(&vip, &backends);
        assert_ne!(a, b, "reordered backends must produce a different fingerprint");
    }

    #[test]
    fn fingerprint_is_sensitive_to_health_flag() {
        // Sanity check that rkyv archives the `bool` field — two
        // backend slices identical except for `healthy` must produce
        // different fingerprints.
        let vip = vip_v4(10, 0, 0, 1);
        let healthy_backends = sample_backends();
        let mut unhealthy_backends = sample_backends();
        unhealthy_backends[0].healthy = false;
        let a = fingerprint(&vip, &healthy_backends);
        let b = fingerprint(&vip, &unhealthy_backends);
        assert_ne!(a, b, "differing healthy flag must produce a different fingerprint");
    }
}
