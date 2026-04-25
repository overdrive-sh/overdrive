//! Trait-generic LWW conformance harness invocation against
//! `LocalObservationStore`. Step 01-01 (RED scaffold).
//!
//! Per `docs/feature/fix-observation-lww-merge/deliver/rca.md` Cause C
//! (comparator in the wrong crate), the LWW comparator must live in
//! `overdrive-core` so both adapters of the `ObservationStore` trait
//! call the same primitive. The trait-generic conformance harness
//! `overdrive_core::testing::observation_store::run_lww_conformance`
//! exercises the LWW contract against any `T: ObservationStore` — both
//! `LocalObservationStore` (this file) and
//! `SimObservationStore::single_peer` (the sibling file in
//! `overdrive-sim`) call it on the same input shape.
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
//! 01-02, which lands:
//!
//! 1. `LogicalTimestamp::dominates(&self, &Self) -> bool` in
//!    `overdrive-core` (promoting the comparator out of `overdrive-sim`).
//! 2. The trait-generic harness at
//!    `overdrive_core::testing::observation_store::run_lww_conformance`.
//! 3. The `overdrive-core/test-utils` dev-dep on this crate.
//! 4. LWW-guarded `LocalObservationStore::write` with suppressed emit
//!    on loss.

use overdrive_store_local::LocalObservationStore;
use tempfile::TempDir;

#[tokio::test]
async fn local_observation_store_satisfies_lww_conformance() {
    let tmp = TempDir::new().expect("tempdir");
    let store = LocalObservationStore::open(tmp.path().join("observation"))
        .expect("open observation store");

    overdrive_core::testing::observation_store::run_lww_conformance(&store).await;
}
