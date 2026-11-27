//! `owned_ranges` — the one list of Overdrive-owned address ranges and the
//! pure containment test (GPD-4, ADR-0359).
//!
//! DISTILL scaffold: the data shape is real so the composition root can
//! build it; the containment test is `todo!` until DELIVER lands it.

#![allow(
    clippy::todo,
    clippy::missing_const_for_fn,
    clippy::unused_self,
    clippy::needless_pass_by_value,
    clippy::doc_markdown,
    reason = "DISTILL scaffold: bodies are todo until DELIVER (ADR-0359)"
)]

use std::net::Ipv4Addr;

use overdrive_dataplane::allocators::VipRange;

/// Which owned range an address fell in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnedRangeKind {
    /// The mesh frontend range (`WORKLOAD_FRONTEND_BASE`).
    MeshFrontend,
    /// The configured service VIP pool.
    ServiceVip,
    /// The guest prefix (`GUEST_NODE_PREFIX`).
    GuestNode,
}

impl OwnedRangeKind {
    /// The canonical label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MeshFrontend => "mesh_frontend",
            Self::ServiceVip => "service_vip",
            Self::GuestNode => "guest_node",
        }
    }
}

/// The list of Overdrive-owned address ranges.
#[derive(Clone, Debug)]
pub struct OwnedAddressRanges {
    vip_range: VipRange,
}

impl OwnedAddressRanges {
    /// THE list. Adding an Overdrive-owned range is one line here.
    #[must_use]
    pub fn platform(vip_range: &VipRange) -> Self {
        Self { vip_range: vip_range.clone() }
    }

    /// The owned range containing `address`, if any.
    #[must_use]
    pub fn owning_range(&self, address: Ipv4Addr) -> Option<OwnedRangeKind> {
        let _ = (&self.vip_range, address);
        todo!(
            "owned-range guard: classify an address against the frontend range, the service VIP pool and the guest prefix"
        )
    }
}
