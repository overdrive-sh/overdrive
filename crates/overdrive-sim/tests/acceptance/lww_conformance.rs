//! Trait-generic LWW conformance harness invocation against
//! `SimObservationStore::single_peer`. Step 01-01 (RED scaffold).
//!
//! Per `docs/feature/fix-observation-lww-merge/deliver/rca.md` Cause C
//! (comparator in the wrong crate), the LWW comparator must live in
//! `overdrive-core` so both adapters of the `ObservationStore` trait
//! call the same primitive. The trait-generic conformance harness
//! `overdrive_core::testing::observation_store::run_lww_conformance`
//! exercises the LWW contract against any `T: ObservationStore` — both
//! `SimObservationStore::single_peer` (this file) and
//! `LocalObservationStore` (the sibling file in
//! `overdrive-store-local`) call it on the same input shape.
//!
//! # RED scaffold per `.claude/rules/testing.md` §"RED scaffolds and
//! # intentionally-failing commits"
//!
//! At step 01-01 commit time:
//!
//! - `overdrive_core::testing` does NOT yet exist — the import below
//!   fails to compile.
//! - `overdrive-core/test-utils` is NOT a dev-dep on this crate yet
//!   (per the step boundary; landing the dev-dep is part of 01-02).
//!
//! The compile error is the RED proof. GREEN counterpart is step
//! 01-02, which lands the harness, the dev-dep, and the production
//! LWW-guarded `LocalObservationStore::write`.

use std::str::FromStr;

use overdrive_core::id::NodeId;
use overdrive_sim::adapters::observation_store::SimObservationStore;

/// Fixed seed for this harness invocation. Mirrors the seed-style used
/// in `sim_observation_lww_converges.rs` so the conformance run is
/// reproducible.
const STEP_SEED: u64 = 0x01_01_BB_BB_BB_BB_BB_BB;

#[tokio::test]
async fn sim_observation_store_satisfies_lww_conformance() {
    let peer_node = NodeId::from_str("node-a").expect("valid node id");
    let store = SimObservationStore::single_peer(peer_node, STEP_SEED);

    overdrive_core::testing::observation_store::run_lww_conformance(&store).await;
}
