//! Trait-generic issuance-ordinal conformance harness invocation against
//! `SimObservationStore::single_peer` (ADR-0063 D6 rev 8 / fix-issuance-
//! ordinal-toctou step 01-01).
//!
//! The TOCTOU this pins is documented in
//! `docs/feature/fix-issuance-ordinal-toctou/deliver/rca.md`: the former
//! `issued_certificate_rows().len()` derivation let two concurrent
//! issuances stamp DUPLICATE ordinals. The additive
//! `ObservationStore::next_issuance_ordinal` port method allocates from a
//! durable atomic counter, making that collision unrepresentable.
//!
//! The trait-generic harness
//! `overdrive_core::testing::observation_store::run_issuance_ordinal_conformance`
//! exercises the contract against any `T: ObservationStore` — both
//! `SimObservationStore::single_peer` (this file) and
//! `LocalObservationStore` (the sibling file in `overdrive-store-local`)
//! drive it through the same allocation sequence (the DST-equivalence
//! requirement, architecture § 4.3).

use std::str::FromStr;

use overdrive_core::id::NodeId;
use overdrive_sim::adapters::observation_store::SimObservationStore;

/// Fixed seed for this harness invocation. Mirrors the seed-style used in
/// `lww_conformance.rs` so the run is reproducible.
const STEP_SEED: u64 = 0x01_01_C0_17_C0_17_C0_17;

#[tokio::test]
async fn sim_observation_store_satisfies_issuance_ordinal_conformance() {
    let peer_node = NodeId::from_str("node-a").expect("valid node id");
    let store = SimObservationStore::single_peer(peer_node, STEP_SEED);

    overdrive_core::testing::observation_store::run_issuance_ordinal_conformance(&store).await;
}
