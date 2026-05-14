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
