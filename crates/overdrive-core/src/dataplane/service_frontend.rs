//! `ServiceFrontend` — the per-service L4 frontend triple
//! `(ServiceVip [V4-by-construction], NonZeroU16 port, Proto)`.
//!
//! Per ADR-0060 § "The `ServiceFrontend` type" + DESIGN decisions
//! D1a/D1b/D2/D3. The type is an ephemeral call argument constructed at
//! the action-shim and consumed at the [`crate::traits::dataplane::Dataplane`]
//! adapter; it is neither a wire type nor a persisted type.
//!
//! Contract (ADR-0060):
//!
//! - `new(vip, port, proto)` validates the VIP is IPv4 **at the
//!   action-shim** (the existing operator-visible rejection site). On an
//!   IPv6 `vip` it returns an [`IdParseError`] that the action-shim maps
//!   to the existing operator-visible `Failed` observation row; IPv6 is
//!   **not** demoted to a late opaque `DataplaneError`.
//! - On success the embedded `ServiceVip` is guaranteed IPv4 by
//!   construction; [`ServiceFrontend::vip_v4`] narrows infallibly
//!   (`unreachable!` on the structurally-impossible V6 arm).
//! - Derives `Debug, Clone, Copy, PartialEq, Eq` only (D2) — no
//!   serde/utoipa/rkyv/Hash. Not a wire type, not a persisted type.

use std::net::Ipv4Addr;
use std::num::NonZeroU16;

use crate::dataplane::backend_key::Proto;
use crate::id::{IdParseError, ServiceVip};

/// Per-service L4 frontend: `(vip [V4-by-construction], port, proto)`.
///
/// See module docs + ADR-0060 for the full contract. The `vip` field is
/// **guaranteed IPv4 by construction** — [`ServiceFrontend::new`] rejects
/// IPv6; adapters may narrow `IpAddr → Ipv4Addr` infallibly via
/// [`ServiceFrontend::vip_v4`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceFrontend {
    vip: ServiceVip,
    port: NonZeroU16,
    proto: Proto,
}

impl ServiceFrontend {
    /// Fallible constructor — validates the VIP is IPv4. On an IPv6 `vip`
    /// returns the [`IdParseError`] that the action-shim maps to the
    /// existing operator-visible `Failed` observation row.
    ///
    /// # Errors
    ///
    /// Returns [`IdParseError::InvalidFormat`] when `vip` wraps an IPv6
    /// address (the IPv4-only precondition is enforced here, at the
    /// action-shim seam).
    pub const fn new(
        vip: ServiceVip,
        port: NonZeroU16,
        proto: Proto,
    ) -> Result<Self, IdParseError> {
        // V4-guaranteed-by-construction: reject IPv6 here so adapters may
        // narrow infallibly via `vip_v4()`. Per ADR-0060 D1a the rejection
        // is structured (`IdParseError`) and surfaces as the existing
        // operator-visible `Failed` row at the action-shim.
        match vip.try_as_ipv4() {
            Some(_) => Ok(Self { vip, port, proto }),
            None => Err(IdParseError::InvalidFormat {
                kind: "ServiceFrontend",
                expected: "an IPv4 service VIP (IPv6 dataplane is GH #155 deferral)",
            }),
        }
    }

    /// Infallible narrow to [`Ipv4Addr`] — the embedded `ServiceVip` is
    /// guaranteed IPv4 by construction ([`ServiceFrontend::new`] rejects
    /// IPv6).
    #[must_use]
    pub fn vip_v4(&self) -> Ipv4Addr {
        self.vip.try_as_ipv4().unwrap_or_else(|| {
            unreachable!("ServiceFrontend::new guarantees the embedded ServiceVip is IPv4")
        })
    }

    /// The embedded service VIP.
    #[must_use]
    pub const fn vip(&self) -> ServiceVip {
        self.vip
    }

    /// The service listener port.
    #[must_use]
    pub const fn port(&self) -> NonZeroU16 {
        self.port
    }

    /// The L4 protocol.
    #[must_use]
    pub const fn proto(&self) -> Proto {
        self.proto
    }
}
