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
//! computed — a field-by-field byte feed into blake3 in a fixed
//! canonical order, truncated to u64 LE. The architecture (§ 6)
//! cites rkyv as the example mechanism; an equivalent hand-rolled
//! deterministic serialization preserves the bit-identical-across-
//! nodes property without forcing rkyv derives onto every
//! transitive type in [`Backend`] (notably `SpiffeId`, which is
//! not yet rkyv-derived). The property the rule pins is canonical
//! determinism, not the mechanism.

use crate::id::ServiceVip;
use crate::traits::dataplane::Backend;

/// Content-hash of a `(ServiceVip, &[Backend])` pair, truncated to
/// `u64`.
pub type BackendSetFingerprint = u64;

/// Compute the canonical content-hash of a backend set keyed by
/// VIP. Bit-identical across nodes given identical inputs.
///
/// Truncates blake3's 256-bit digest to the first 8 bytes
/// (little-endian) — the cluster-lifetime collision probability at
/// O(1k) services × O(1k) churn-per-service is negligible.
///
/// # Determinism
///
/// The byte feed is field-by-field, in fixed order, with each
/// variable-length field length-prefixed (`u32` LE). No
/// `serde_json::to_string` (which has non-deterministic field
/// ordering on `serde_json::Value`); no `format!` for any addr or
/// `SpiffeId` — both have stable canonical forms emitted via
/// `Display` / explicit byte accessors.
///
/// `Backend` order is observed by the caller — the hydrator passes
/// backends in the deterministic `BTreeMap<BackendId, Backend>`
/// iteration order per architecture.md § 7. A reordered slice
/// produces a different fingerprint by construction.
#[must_use]
pub fn fingerprint(vip: &ServiceVip, backends: &[Backend]) -> BackendSetFingerprint {
    let mut hasher = blake3::Hasher::new();

    // VIP — IpAddr canonical form.
    feed_ip_addr(&mut hasher, vip.get());

    // Backend set — length-prefixed, then each backend in order.
    feed_u32(&mut hasher, u32::try_from(backends.len()).unwrap_or(u32::MAX));
    for backend in backends {
        // SpiffeId — canonical lowercased string (the SpiffeId
        // newtype guarantees this on construction). Length-prefix
        // the bytes so two Spiffe IDs whose byte concatenation
        // would alias cannot collide.
        let spiffe = backend.alloc.to_string();
        feed_bytes(&mut hasher, spiffe.as_bytes());

        // SocketAddr — IP + port. IP is fed via the same canonical
        // accessor as VIP; port is a fixed-width u16 LE.
        feed_ip_addr(&mut hasher, backend.addr.ip());
        hasher.update(&backend.addr.port().to_le_bytes());

        // Weight — fixed-width u16 LE.
        hasher.update(&backend.weight.to_le_bytes());

        // Health flag — single byte.
        hasher.update(&[u8::from(backend.healthy)]);
    }

    let digest = hasher.finalize();
    let bytes = digest.as_bytes();
    let prefix: [u8; 8] =
        [bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]];
    u64::from_le_bytes(prefix)
}

/// Feed an IP address as a discriminator byte (4 = v4, 6 = v6) plus
/// the canonical octets. Distinguishes `0.0.0.0` from `::` even
/// though both serialize to four/sixteen zero bytes.
fn feed_ip_addr(hasher: &mut blake3::Hasher, addr: std::net::IpAddr) {
    match addr {
        std::net::IpAddr::V4(v4) => {
            hasher.update(&[4_u8]);
            hasher.update(&v4.octets());
        }
        std::net::IpAddr::V6(v6) => {
            hasher.update(&[6_u8]);
            hasher.update(&v6.octets());
        }
    }
}

/// Length-prefixed byte feed (u32 LE).
fn feed_bytes(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    feed_u32(hasher, u32::try_from(bytes.len()).unwrap_or(u32::MAX));
    hasher.update(bytes);
}

/// Feed a u32 in little-endian.
fn feed_u32(hasher: &mut blake3::Hasher, v: u32) {
    hasher.update(&v.to_le_bytes());
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
}
