//! `owned_ranges` — the one list of Overdrive-owned address ranges and the
//! pure containment test (GPD-4, ADR-0359).

use std::net::Ipv4Addr;

use ipnet::Ipv4Net;
use overdrive_core::guest_transport::GUEST_NODE_PREFIX;
use overdrive_dataplane::allocators::VipRange;

use super::frontend_addr_allocator::WORKLOAD_FRONTEND_BASE;

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
    fixed: Vec<(Ipv4Net, OwnedRangeKind)>,
    vip_range: VipRange,
}

impl OwnedAddressRanges {
    /// THE list. Adding an Overdrive-owned range is one line here.
    #[must_use]
    pub fn platform(vip_range: &VipRange) -> Self {
        Self {
            fixed: vec![
                (WORKLOAD_FRONTEND_BASE, OwnedRangeKind::MeshFrontend),
                (GUEST_NODE_PREFIX, OwnedRangeKind::GuestNode),
            ],
            vip_range: vip_range.clone(),
        }
    }

    /// The owned range containing `address`, if any. Reserved VIP addresses
    /// count as owned: the pool is tested by overlap, not by allocatability.
    #[must_use]
    pub fn owning_range(&self, address: Ipv4Addr) -> Option<OwnedRangeKind> {
        if let Some((_, kind)) = self.fixed.iter().find(|(net, _)| net.contains(&address)) {
            return Some(*kind);
        }
        self.vip_range.first_overlapping_range(Ipv4Net::from(address)).map(|_| OwnedRangeKind::ServiceVip)
    }
}
