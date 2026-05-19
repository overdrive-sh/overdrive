//! Typed errors for the `allocators` module.
//!
//! Two error families, distinct concerns:
//!
//! - [`VipAllocatorConfigError`] — surfaces from [`super::VipRange::new`].
//!   Construction-time validation: overlapping CIDRs, reserved addresses
//!   outside the configured range, zero effective capacity. Per ADR-0049
//!   these are operator-config errors that refuse boot.
//! - [`ServiceVipAllocatorError`] — surfaces from
//!   [`super::ServiceVipAllocator::allocate`]. Runtime allocation
//!   errors: pool exhaustion.
//!
//! Both follow the project's `thiserror`-typed pattern; binary boundaries
//! flatten via `eyre`. Per `.claude/rules/development.md` § Errors.

use std::net::Ipv4Addr;

use ipnet::Ipv4Net;
use overdrive_core::id::IdParseError;
use thiserror::Error;

/// Construction-time errors from [`super::VipRange::new`].
///
/// Each variant carries the offending input(s) so the boot-time
/// `health.startup.refused` event can name the precise misconfiguration
/// the operator must fix.
#[derive(Debug, Error)]
pub enum VipAllocatorConfigError {
    /// Two configured CIDR ranges overlap. Refusing prevents the same
    /// IPv4 address from being allocated twice under two different
    /// `VipRange` entries.
    #[error("overlapping VIP ranges: {a} overlaps {b}")]
    OverlappingRanges {
        /// First overlapping CIDR.
        a: Ipv4Net,
        /// Second overlapping CIDR.
        b: Ipv4Net,
    },

    /// A reserved address falls outside every configured range. The
    /// reserved set must be a subset of the union of `ranges`.
    #[error("reserved address {addr} is outside every configured range")]
    ReservedOutsideRange {
        /// The offending reserved address.
        addr: Ipv4Addr,
    },

    /// After exclusions, the range has zero allocatable addresses.
    /// Either the CIDR is /32 with that address reserved, or every
    /// address in the union is reserved.
    #[error("VIP range has zero effective capacity after exclusions")]
    ZeroCapacity,
}

/// Result alias for [`VipAllocatorConfigError`]-returning constructors.
pub type Result<T, E = VipAllocatorConfigError> = std::result::Result<T, E>;

/// Runtime allocation errors from
/// [`super::ServiceVipAllocator::allocate`].
///
/// The allocator is scan-based with reuse-on-release (ADR-0049 §
/// Amendments → 2026-05-19); released entries return to the pool.
/// `Exhausted` surfaces only when every slot in the configured range
/// is currently held in the memo — a finite-but-large pool can serve
/// effectively-unbounded lifetimes as long as the
/// simultaneously-held count stays below capacity.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ServiceVipAllocatorError {
    /// The pool has no more available addresses — every slot in the
    /// configured range is currently held. `allocated` is the current
    /// memo size; `capacity` is the configured maximum.
    #[error("ServiceVip pool exhausted: allocated {allocated} of {capacity}")]
    Exhausted {
        /// Number of VIPs currently allocated.
        allocated: u64,
        /// Configured pool capacity (after reserved exclusions).
        capacity: u64,
    },

    /// The canonical [`overdrive_core::id::ServiceVip`] constructor
    /// rejected the IPv4 address produced by
    /// [`super::VipRange::nth_allocatable`]. Currently unreachable —
    /// `ServiceVip::new` is total over `IpAddr` today — but the
    /// variant exists so a future range-rejection (multicast /
    /// unspecified / reserved-by-IANA) on the canonical newtype
    /// surfaces here as a typed error rather than a panic.
    #[error("ServiceVip newtype rejected allocated address: {source}")]
    NewtypeRejected {
        /// Underlying parse error from the canonical newtype.
        #[from]
        source: IdParseError,
    },
}
