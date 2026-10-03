//! [`GuestAttachmentView`] — the node guest-attachment occupancy read port
//! (D-295-R8, ADR-0134).
//!
//! The fifth narrow driven read-port the reconciler hydration boundary reads,
//! beside the four ADR-0086 read-ports. `WorkloadLifecycle` hydrates one
//! consistent occupancy snapshot through it for placement and restart gating;
//! the operator status derives cleanup-pending from the lease it reports
//! ([`GuestAttachmentLease::cleanup_pending`], D-295-R20). The production
//! implementation holds the server's guest-address pool and reads it under its
//! mutex; the DST implementation is `overdrive_sim::adapters::SimGuestAttachmentView`.
//!
//! Per `.claude/rules/development.md` § "Port-trait dependencies" the
//! reconciler reads through `&dyn GuestAttachmentView` on the
//! [`HydrationContext`], never a concrete `AppState` field.
//!
//! [`HydrationContext`]: crate::reconcilers::HydrationContext

use std::collections::BTreeMap;

use crate::guest_network::GuestAttachmentOccupancy;
use crate::id::AllocationId;
use crate::traits::observation_store::AllocState;

/// The guest-network lease an allocation holds in the server's
/// guest-address pool (ADR-0133).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestAttachmentLease {
    /// The lease was admitted and its cleanup has not begun.
    Admitted,
    /// The allocation's cleanup has begun and not finished; the lease still
    /// counts against the cap (D-295-R7).
    Retiring,
}

impl GuestAttachmentLease {
    /// True when the allocation still holds guest-network residue whose
    /// cleanup has not finished and is operator-visible as cleanup-pending:
    /// every Retiring lease (cleanup has begun), and an Admitted lease on an
    /// allocation whose row is terminal (its VMM exited; cleanup has not begun).
    #[must_use]
    pub const fn cleanup_pending(self, row_state: AllocState) -> bool {
        match self {
            Self::Retiring => true,
            Self::Admitted => row_state.is_terminal(),
        }
    }
}

/// One consistent occupancy snapshot plus the leases of the requested
/// allocations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestAttachmentObservation {
    /// Node-wide held and retiring occupancy.
    pub occupancy: GuestAttachmentOccupancy,
    /// The lease each requested allocation holds. An allocation holding no
    /// lease is absent from the map.
    pub leases: BTreeMap<AllocationId, GuestAttachmentLease>,
}

/// The node guest-attachment occupancy read port (D-295-R8, ADR-0134).
///
/// A **driven, read-only** projection: the reconciler calls out to read the
/// node's guest-attachment occupancy; there is no write method.
pub trait GuestAttachmentView: Send + Sync {
    /// Occupancy plus the leases of `allocs`, read as ONE snapshot under one
    /// acquisition of the owning lock. The snapshot is advisory. The
    /// authoritative decision is the pool's `assign` at dispatch, which may
    /// refuse a placement this snapshot allowed.
    fn observe(&self, allocs: &[AllocationId]) -> GuestAttachmentObservation;
}
