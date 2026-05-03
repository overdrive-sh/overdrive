//! Acceptance — `SimViewStore` is a lossless CBOR-byte cache for arbitrary
//! `View` values per ADR-0035 §2 / wave-decisions §D6
//! `ViewStoreRoundtripIsLossless`.
//!
//! Step 01-03 contract: `SimViewStore::write_through(name, t, &v)` followed
//! by `bulk_load::<V>(name).get(&t)` returns `Some(v)` byte-equal to the
//! original `v` after a CBOR encode → decode roundtrip. `delete(name, t)`
//! removes the row from subsequent `bulk_load`. `probe()` succeeds on a
//! fresh store and leaves no residual rows.
//!
//! The `SimViewStore` here is a port-to-port driving boundary: the
//! `Arc<dyn ViewStore>` surface is what the `ReconcilerRuntime` (step
//! 01-06) holds; tests exercise it through that surface, not through
//! private internals.

use std::collections::BTreeMap;
use std::sync::Arc;

use proptest::prelude::*;
use serde::{Deserialize, Serialize};

use overdrive_control_plane::view_store::{ViewStore, ViewStoreExt};
use overdrive_core::reconciler::{ReconcilerName, TargetResource};
use overdrive_sim::adapters::view_store::SimViewStore;

/// Test-local View shape — small enough to keep proptest cases fast,
/// rich enough to exercise CBOR encode/decode of nested data
/// (`Vec<u8>`, `BTreeMap`, optional fields).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct TestView {
    counter: u64,
    label: String,
    tags: BTreeMap<String, u32>,
    blob: Vec<u8>,
}

prop_compose! {
    fn arb_test_view()(
        counter in any::<u64>(),
        label in "[a-z]{0,16}",
        tag_keys in prop::collection::vec("[a-z]{1,8}", 0..4),
        tag_vals in prop::collection::vec(any::<u32>(), 0..4),
        blob in prop::collection::vec(any::<u8>(), 0..32),
    ) -> TestView {
        let n = tag_keys.len().min(tag_vals.len());
        let tags = tag_keys.into_iter().take(n).zip(tag_vals.into_iter().take(n)).collect();
        TestView { counter, label, tags, blob }
    }
}

prop_compose! {
    fn arb_reconciler_name()(s in "[a-z][a-z0-9-]{0,16}") -> ReconcilerName {
        ReconcilerName::new(&s).expect("valid by construction")
    }
}

prop_compose! {
    fn arb_target()(s in "[a-zA-Z0-9_-]{1,16}") -> TargetResource {
        TargetResource::new(&format!("job/{s}")).expect("valid by construction")
    }
}

proptest! {
    /// `SimViewStoreRoundtripIsLossless` (ADR-0035 §6 / wave-decisions §D6).
    /// Write-through then bulk-load returns a `View` byte-equal to the
    /// original under CBOR roundtrip.
    #[test]
    fn sim_view_store_roundtrip_is_lossless_under_proptest(
        name in arb_reconciler_name(),
        target in arb_target(),
        view in arb_test_view(),
    ) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("tokio runtime");
        let store: Arc<dyn ViewStore> = Arc::new(SimViewStore::new());

        rt.block_on(async {
            // Earned-Trust probe on a fresh store always succeeds and
            // leaves no residual state.
            store.probe().await.expect("probe must succeed on fresh store");

            // Write-through, then bulk-load, then assert byte-equal under
            // CBOR roundtrip.
            ViewStoreExt::write_through(&*store, &name, &target, &view)
                .await
                .expect("write_through must succeed on healthy store");

            let loaded: BTreeMap<TargetResource, TestView> =
                ViewStoreExt::bulk_load(&*store, &name)
                    .await
                    .expect("bulk_load must succeed on healthy store");

            prop_assert_eq!(loaded.get(&target), Some(&view),
                "bulk_load must return the value just written");

            // Delete removes the row from a subsequent bulk_load.
            store.delete(&name, &target).await
                .expect("delete must succeed");
            let loaded_after_delete: BTreeMap<TargetResource, TestView> =
                ViewStoreExt::bulk_load(&*store, &name)
                    .await
                    .expect("bulk_load must succeed after delete");
            prop_assert!(!loaded_after_delete.contains_key(&target),
                "deleted row must not appear in subsequent bulk_load");

            // Probe still succeeds after the round-trip / delete cycle
            // and leaves no residual rows under the same name.
            store.probe().await.expect("probe must still succeed");
            let after_probe: BTreeMap<TargetResource, TestView> =
                ViewStoreExt::bulk_load(&*store, &name)
                    .await
                    .expect("bulk_load must succeed after probe");
            prop_assert!(after_probe.is_empty(),
                "probe must leave no residual rows under user-visible names");

            Ok(())
        })?;
    }
}
