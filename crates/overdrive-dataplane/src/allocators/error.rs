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

    /// The TOML config does not declare the named `[dataplane.vip_allocator]`
    /// subsection. Surfaces from the boot-time parser (step 02-02)
    /// when the operator-supplied config is missing the required
    /// section entirely. The `section` field names the missing path
    /// verbatim so the operator-facing message is unambiguous.
    ///
    /// Type-level callers (`VipRange::new` direct invocations) never
    /// construct this variant — it is reserved for the config-parser
    /// surface in `overdrive-control-plane::vip_allocator_config`.
    #[error("required config section [{section}] is missing")]
    Missing {
        /// Dotted path of the missing TOML section
        /// (e.g. `"dataplane.vip_allocator"`).
        section: &'static str,
    },
}

/// Result alias for [`VipAllocatorConfigError`]-returning constructors.
pub type Result<T, E = VipAllocatorConfigError> = std::result::Result<T, E>;

/// Runtime allocation errors from
/// [`super::ServiceVipAllocator::allocate`].
///
/// The allocator is monotonic — released entries are not reclaimed; the
/// counter advances on every miss until the configured range is
/// exhausted. `Exhausted` surfaces when the next index has no
/// allocatable address available.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ServiceVipAllocatorError {
    /// The pool has no more available addresses. `allocated` is the
    /// current memo size; `capacity` is the configured maximum.
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
