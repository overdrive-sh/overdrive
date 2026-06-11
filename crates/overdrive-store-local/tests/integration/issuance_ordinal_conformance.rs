//! Trait-generic issuance-ordinal conformance harness invocation against
//! `LocalObservationStore` (ADR-0063 D6 rev 8 / fix-issuance-ordinal-
//! toctou step 01-01).
//!
//! The TOCTOU this pins is documented in
//! `docs/feature/fix-issuance-ordinal-toctou/deliver/rca.md`: the former
//! `issued_certificate_rows().len()` derivation let two concurrent
//! issuances stamp DUPLICATE ordinals. The host adapter allocates the
//! ordinal atomically inside one redb `begin_write`/`commit` under
//! serializable isolation, making the collision unrepresentable.
//!
//! The trait-generic harness
//! `overdrive_core::testing::observation_store::run_issuance_ordinal_conformance`
//! exercises the contract against any `T: ObservationStore` — both
//! `LocalObservationStore` (this file) and `SimObservationStore`
//! (the sibling file in `overdrive-sim`) drive it through the same
//! allocation sequence (the DST-equivalence requirement, architecture
//! § 4.3 #1 and #2). The host-only durable-across-reopen sub-case
//! (§ 4.3 #3) lives in the sibling `issuance_ordinal_durable_reopen.rs`.

use overdrive_store_local::LocalObservationStore;
use tempfile::TempDir;

#[tokio::test]
async fn local_observation_store_satisfies_issuance_ordinal_conformance() {
    let tmp = TempDir::new().expect("tempdir");
    let store = LocalObservationStore::open(tmp.path().join("observation"))
        .expect("open observation store");

    overdrive_core::testing::observation_store::run_issuance_ordinal_conformance(&store).await;
}
