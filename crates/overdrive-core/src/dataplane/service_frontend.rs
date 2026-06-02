//! `ServiceFrontend` — the per-service L4 frontend triple
//! `(ServiceVip [V4-by-construction], NonZeroU16 port, Proto)`.
//!
//! **RED scaffold (DISTILL, udp-service-support US-01).** The bodies are
//! `todo!("RED scaffold: …")` until DELIVER lands US-01 per ADR-0060 §
//! "The `ServiceFrontend` type" + DESIGN decisions D1a/D1b/D2/D3. This
//! file creates the type so the DISTILL acceptance scaffolds compile and
//! go RED for the right reason (missing implementation), NOT BROKEN
//! (missing type). DISTILL does **not** migrate the 8 production sites
//! (trait, both adapters, action-shim, lockstep invariant, Action,
//! ServiceDesired + projection) — that is DELIVER's job.
//!
//! Contract (per ADR-0060 — pinned here so DELIVER implements to it):
//!
//! - `new(vip, port, proto)` validates the VIP is IPv4 **at the
//!   action-shim** (the existing operator-visible rejection site). On an
//!   IPv6 `vip` it returns the error that maps to the existing
//!   operator-visible `Failed` observation row; IPv6 is **not** demoted
//!   to a late opaque `DataplaneError`.
//! - On success the embedded `ServiceVip` is guaranteed IPv4 by
//!   construction; `vip_v4()` narrows infallibly (`unreachable!` on the
//!   structurally-impossible V6 arm).
//! - Derives `Debug, Clone, Copy, PartialEq, Eq` only (D2) — no
//!   serde/utoipa/rkyv/Hash. Not a wire type, not a persisted type.

use std::net::Ipv4Addr;
use std::num::NonZeroU16;

use crate::dataplane::backend_key::Proto;
use crate::id::{IdParseError, ServiceVip};

/// Per-service L4 frontend: `(vip [V4-by-construction], port, proto)`.
///
/// See module docs + ADR-0060 for the full contract. The `vip` field is
/// **guaranteed IPv4 by construction** — `ServiceFrontend::new` rejects
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
    /// returns the error that the action-shim maps to the existing
    /// operator-visible `Failed` observation row.
    ///
    /// # Errors
    ///
    /// Returns `Err` when `vip` wraps an IPv6 address (the IPv4-only
    /// precondition is enforced here, at the action-shim seam).
    #[expect(
        clippy::todo,
        reason = "RED scaffold; lands GREEN in udp-service-support US-01 (slice 01)"
    )]
    pub fn new(vip: ServiceVip, port: NonZeroU16, proto: Proto) -> Result<Self, IdParseError> {
        let _ = (vip, port, proto);
        todo!("RED scaffold: validate vip is IPv4, build ServiceFrontend (US-01)")
    }

    /// Infallible narrow to `Ipv4Addr` — the embedded `ServiceVip` is
    /// guaranteed IPv4 by construction (`new` rejects IPv6).
    #[expect(
        clippy::todo,
        reason = "RED scaffold; lands GREEN in udp-service-support US-01 (slice 01)"
    )]
    #[must_use]
    pub fn vip_v4(&self) -> Ipv4Addr {
        let _ = self.vip;
        todo!("RED scaffold: narrow self.vip to Ipv4Addr via documented invariant (US-01)")
    }

    /// The embedded service VIP.
    #[expect(
        clippy::todo,
        reason = "RED scaffold; lands GREEN in udp-service-support US-01 (slice 01)"
    )]
    #[must_use]
    pub fn vip(&self) -> ServiceVip {
        let _ = self.vip;
        todo!("RED scaffold: return self.vip (US-01)")
    }

    /// The service listener port.
    #[expect(
        clippy::todo,
        reason = "RED scaffold; lands GREEN in udp-service-support US-01 (slice 01)"
    )]
    #[must_use]
    pub fn port(&self) -> NonZeroU16 {
        let _ = self.port;
        todo!("RED scaffold: return self.port (US-01)")
    }

    /// The L4 protocol.
    #[expect(
        clippy::todo,
        reason = "RED scaffold; lands GREEN in udp-service-support US-01 (slice 01)"
    )]
    #[must_use]
    pub fn proto(&self) -> Proto {
        let _ = self.proto;
        todo!("RED scaffold: return self.proto (US-01)")
    }
}
