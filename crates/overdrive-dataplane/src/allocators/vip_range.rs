//! `VipRange` — IPv4 CIDR ranges plus an exclusion set of reserved
//! addresses, validated at construction time.
//!
//! Per ADR-0049 § 5b the operator-facing config shape is:
//!
//! ```toml
//! [dataplane.vip_allocator]
//! ranges   = ["10.96.0.0/24"]
//! reserved = ["10.96.0.0", "10.96.0.255"]
//! ```
//!
//! The TOML parse surface lands in step 02-02; this module owns the
//! TYPE-LEVEL constructor + invariant. Multi-range support: every input
//! CIDR is checked against every other input for overlap; every
//! reserved address must lie within at least one configured range; the
//! capacity (sum of `range.size()` minus reserved cardinality) must be
//! positive.
//!
//! See `docs/feature/service-vip-allocator/distill/test-scenarios.md`
//! S-VIP-16/17/18/P04 for the spec.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

use ipnet::Ipv4Net;

use super::error::{Result, VipAllocatorConfigError};

/// Validated VIP-pool address space.
///
/// Construct via [`Self::new`], which enforces three invariants:
///
/// 1. No two CIDR ranges overlap.
/// 2. Every reserved address lies within the union of `ranges`.
/// 3. After excluding reserved addresses, the effective capacity is
///    positive.
///
/// Order-stable iteration is preserved by storing `ranges` in a
/// `Vec<Ipv4Net>` (operator-supplied order) and `reserved` in a
/// `BTreeSet<Ipv4Addr>` (sorted). Per `.claude/rules/development.md`
/// § Ordered-collection choice — the allocator iterates the range
/// indices when skipping reserved addresses, and deterministic order
/// is the right default.
#[derive(Debug, Clone)]
pub struct VipRange {
    ranges: Vec<Ipv4Net>,
    reserved: BTreeSet<Ipv4Addr>,
    /// Pre-computed total address count across `ranges` (i.e., the
    /// sum of each range's size). Cached at construction; never
    /// mutated.
    total: u64,
}

impl VipRange {
    /// Construct a validated `VipRange`.
    ///
    /// # Errors
    ///
    /// - [`VipAllocatorConfigError::OverlappingRanges`] — two input
    ///   CIDRs overlap. The first overlapping pair encountered is
    ///   reported.
    /// - [`VipAllocatorConfigError::ReservedOutsideRange`] — a
    ///   reserved address is not contained in any range.
    /// - [`VipAllocatorConfigError::ZeroCapacity`] — after exclusions,
    ///   no addresses remain allocatable.
    pub fn new(ranges: Vec<Ipv4Net>, reserved: BTreeSet<Ipv4Addr>) -> Result<Self> {
        // Pairwise overlap check. `Vec` not `BTreeSet`-of-ranges
        // because `Ipv4Net` does not implement `Ord` in a way that
        // groups overlaps; an O(n²) scan is acceptable for the
        // operator-config cardinality (handful of ranges).
        for (i, a) in ranges.iter().enumerate() {
            for b in ranges.iter().skip(i + 1) {
                if a.contains(&b.network())
                    || a.contains(&b.broadcast())
                    || b.contains(&a.network())
                    || b.contains(&a.broadcast())
                {
                    return Err(VipAllocatorConfigError::OverlappingRanges { a: *a, b: *b });
                }
            }
        }

        // Every reserved address must be inside the union.
        for addr in &reserved {
            if !ranges.iter().any(|net| net.contains(addr)) {
                return Err(VipAllocatorConfigError::ReservedOutsideRange { addr: *addr });
            }
        }

        // Compute total span across all ranges. `u64` because four
        // /0 ranges would overflow `u32`; in practice the operator
        // configures at most a few /24..32 ranges.
        let total: u64 = ranges.iter().map(|net| 1u64 << (32 - u32::from(net.prefix_len()))).sum();

        let reserved_count = reserved.len() as u64;
        if reserved_count >= total {
            return Err(VipAllocatorConfigError::ZeroCapacity);
        }

        Ok(Self { ranges, reserved, total })
    }

    /// Effective capacity — total addresses in `ranges` minus the
    /// number of reserved addresses.
    ///
    /// Equivalent to: how many distinct tokens the underlying
    /// [`super::PoolAllocator`] can hand out before exhaustion.
    #[must_use]
    pub fn capacity(&self) -> u64 {
        self.total - self.reserved.len() as u64
    }

    /// Returns the `n`-th allocatable IPv4 address, skipping reserved
    /// entries.
    ///
    /// `n` is a zero-based index into the *allocatable* sequence
    /// (i.e., the sequence with reserved addresses already removed).
    /// Returns `None` if `n >= capacity()`.
    ///
    /// Iteration order: `ranges` in the operator-supplied order; each
    /// range walked from `network()` to `broadcast()` inclusive,
    /// skipping any address in the reserved set.
    #[must_use]
    pub fn nth_allocatable(&self, n: u64) -> Option<Ipv4Addr> {
        let mut remaining = n;
        for net in &self.ranges {
            let net_u32 = u32::from(net.network());
            let span: u64 = 1u64 << (32 - u32::from(net.prefix_len()));
            for offset in 0..span {
                // span is bounded by 2^32 for a /0 range; the cast
                // discards the upper bit only when (offset + net_u32)
                // wraps, which it cannot here because net_u32 is the
                // base and offset < span ≤ 2^32 - net_u32.
                #[allow(clippy::cast_possible_truncation)]
                let addr = Ipv4Addr::from(net_u32.wrapping_add(offset as u32));
                if self.reserved.contains(&addr) {
                    continue;
                }
                if remaining == 0 {
                    return Some(addr);
                }
                remaining -= 1;
            }
        }
        None
    }

    /// Returns `true` if `addr` is contained in any configured range
    /// AND not reserved. Used by Earned Trust probes (step 01-03) to
    /// verify persisted entries still project into the live range.
    #[must_use]
    pub fn contains(&self, addr: Ipv4Addr) -> bool {
        if self.reserved.contains(&addr) {
            return false;
        }
        self.ranges.iter().any(|net| net.contains(&addr))
    }
}

impl Default for VipRange {
    /// Phase 1 single-node default per ADR-0049 § Amendments → 2026-05-15
    /// (Alt-E accepted).
    ///
    /// Returns a `VipRange` covering `10.96.0.0/16` with reserved set
    /// `[10.96.0.0, 10.96.0.1, 10.96.255.255]` — matching the pinned
    /// defaults Kubernetes (`--service-cluster-ip-range`), MetalLB
    /// (`addresses`), and kube-vip / Calico-CNI ship. Operators may
    /// override via the `[dataplane.vip_allocator]` TOML section.
    ///
    /// # Panics
    ///
    /// Never. The default inputs are statically known to satisfy every
    /// invariant `VipRange::new` enforces (single non-overlapping CIDR,
    /// reserved addresses all inside the CIDR, capacity = 65536 - 3 > 0).
    /// The `expect` is therefore a static-guarantee assertion, not a
    /// runtime failure path. If you change the constants below, re-prove
    /// the invariants hold. The `expect` lint is scoped here because the
    /// inputs are infallible by construction; the surrounding lib does
    /// not relax this lint generally.
    #[allow(
        clippy::expect_used,
        reason = "static-guarantee assertion on hard-coded ADR-0049 default inputs; \
                  the prefix is constant (/16 ≤ /32) and the reserved addresses are \
                  inside the network, so the constructor cannot return Err. See the \
                  `vip_range_default_value` unit test for the invariant pin."
    )]
    fn default() -> Self {
        let network = Ipv4Addr::new(10, 96, 0, 0);
        let range = Ipv4Net::new(network, 16)
            .expect("/16 prefix is a valid prefix length (≤32) per Ipv4Net::new contract");
        let reserved: BTreeSet<Ipv4Addr> = [
            Ipv4Addr::new(10, 96, 0, 0),
            Ipv4Addr::new(10, 96, 0, 1),
            Ipv4Addr::new(10, 96, 255, 255),
        ]
        .into_iter()
        .collect();
        Self::new(vec![range], reserved).expect(
            "ADR-0049 default VipRange inputs statically satisfy every VipRange::new invariant",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR-0049 amendment 2026-05-15 (Alt-E): the Phase 1 default
    /// `VipRange` covers `10.96.0.0/16` with reserved set
    /// `[10.96.0.0, 10.96.0.1, 10.96.255.255]`. The boot-time refusal
    /// for "missing `[dataplane.vip_allocator]` section" is walked back
    /// to "default and emit `health.startup.warn` under HA". This unit
    /// test pins the default's effective shape so a future
    /// behaviour-changing edit to `Default::default()` fails loud at
    /// PR time rather than silently shipping a different default to
    /// every single-node operator.
    #[test]
    fn vip_range_default_value() {
        let range = VipRange::default();

        // /16 has 65_536 addresses; 3 reserved → 65_533 allocatable.
        assert_eq!(
            range.capacity(),
            65_536 - 3,
            "default /16 minus 3 reserved should be 65533 allocatable",
        );

        // Reserved addresses are NOT contained.
        assert!(!range.contains(Ipv4Addr::new(10, 96, 0, 0)));
        assert!(!range.contains(Ipv4Addr::new(10, 96, 0, 1)));
        assert!(!range.contains(Ipv4Addr::new(10, 96, 255, 255)));

        // First allocatable address is 10.96.0.2 (after skipping
        // .0 and .1).
        assert_eq!(
            range.nth_allocatable(0),
            Some(Ipv4Addr::new(10, 96, 0, 2)),
            "first allocatable after reserved skip is 10.96.0.2",
        );

        // Addresses inside the /16 but outside the reserved set ARE
        // contained.
        assert!(range.contains(Ipv4Addr::new(10, 96, 0, 2)));
        assert!(range.contains(Ipv4Addr::new(10, 96, 128, 0)));

        // Addresses outside the /16 are NOT contained.
        assert!(!range.contains(Ipv4Addr::new(10, 97, 0, 0)));
    }
}
