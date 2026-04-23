//! `SimObservationStore` wired as the Phase 1 production observation-store impl.
//!
//! Per ADR-0012, the Phase 1 server reuses `overdrive-sim`'s
//! `SimObservationStore` (constructed via
//! [`SimObservationStore::single_peer`]) as the server's
//! `ObservationStore` implementation. Phase 2+ swaps in `CorrosionStore`
//! via a single `Box<dyn ObservationStore>` trait-object replacement —
//! no handler changes.
//!
//! This wiring module is the seam. Handlers depend on `&dyn
//! ObservationStore`, never on `SimObservationStore` directly.

use overdrive_core::id::NodeId;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::observation_store::SimObservationStore;

use crate::error::ControlPlaneError;

/// Deterministic seed used for the Phase 1 single-node
/// `SimObservationStore`. Per ADR-0012, Phase 1 is effectively a
/// single-writer peer with no gossip — the seed only governs internal
/// tiebreaks, so a fixed value (0) keeps the server boot deterministic.
const SINGLE_NODE_SEED: u64 = 0;

/// `NodeId` the Phase 1 server reports as its peer identity. Phase 1
/// runs a single control-plane peer and the identity is only used for
/// LWW writer tiebreaks; Phase 2 replaces this with the operator-
/// supplied node identity from the bootstrap config.
const SINGLE_NODE_PEER_ID: &str = "control-plane-0";

/// Construct the Phase 1 single-node observation store. Returns a
/// trait-object handle so handlers never name the concrete type.
pub fn wire_single_node_observation() -> Result<Box<dyn ObservationStore>, ControlPlaneError> {
    let node_id = NodeId::new(SINGLE_NODE_PEER_ID).map_err(|e| {
        ControlPlaneError::Internal(format!(
            "invariant: single-node peer id {SINGLE_NODE_PEER_ID:?} must parse: {e}"
        ))
    })?;
    let store = SimObservationStore::single_peer(node_id, SINGLE_NODE_SEED);
    Ok(Box::new(store))
}
