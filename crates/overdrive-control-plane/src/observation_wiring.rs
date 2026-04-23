//! `SimObservationStore` wired as the Phase 1 production observation-store impl.
//!
//! SCAFFOLD: true — created by DISTILL wave for phase-1-control-plane-core.
//!
//! Per ADR-0012, the Phase 1 server reuses `overdrive-sim`'s
//! `SimObservationStore` (constructed with `GossipProfile::single_node()`)
//! as the server's `ObservationStore` implementation. Phase 2+ swaps
//! in `CorrosionStore` via a single `Box<dyn ObservationStore>` trait-
//! object replacement — no handler changes.
//!
//! This wiring module is the seam. Handlers depend on `&dyn
//! ObservationStore`, never on `SimObservationStore` directly.

use overdrive_core::traits::observation_store::ObservationStore;

use crate::error::ControlPlaneError;

/// Construct the Phase 1 single-node observation store. Returns a
/// trait-object handle so handlers never name the concrete type.
///
/// SCAFFOLD: true
pub fn wire_single_node_observation() -> Result<Box<dyn ObservationStore>, ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}
