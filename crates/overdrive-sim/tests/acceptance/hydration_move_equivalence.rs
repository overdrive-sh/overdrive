//! S-ROH-B-03 — `AnyReconciler::hydrate_*` forwarding wraps into the matching
//! `AnyState` variant (the post-move enum-dispatch equivalence bar).
//!
//! **CONTRACT_SHAPE: bounded-change (read-only forwarding; return-only).** The
//! ADR-0086 S3 move (step 02-04) collapses the central
//! `reconciler_runtime::hydrate_desired` / `hydrate_actual` free-fn `match`
//! dispatchers onto per-reconciler `Reconciler::hydrate_*` trait methods,
//! forwarded by `AnyReconciler::hydrate_desired` / `hydrate_actual` — one arm
//! per variant, wrapping the concrete `Self::State` into the MATCHING `AnyState`
//! variant, exactly as `AnyReconciler::reconcile` wraps `Self::View` into
//! `AnyReconcilerView`.
//!
//! This test drives the NEW port-driven forwarding surface directly (through the
//! production `hydrate_*_for_test` adapters, which build a `HydrationContext`
//! from `AppState` and call `AnyReconciler::hydrate_*`) for EVERY `AnyReconciler`
//! variant and asserts the returned `AnyState` discriminant is the variant that
//! reconciler owns — the same variant the deleted central `match reconciler { ..
//! } -> AnyState` free fn produced. The **byte-equality** half of the
//! equivalence bar (B-01, port-driven `AnyState` == the committed pre-move
//! golden) lives in the sibling `overdrive-control-plane`
//! `hydration_characterization_golden` test, which owns the golden fixtures; the
//! **trajectory** half (B-02, seeded reconcile trajectory == golden) lives in
//! `hydration_trajectory_golden`. This file is the STRUCTURAL forwarding bar:
//! the forwarding must dispatch to the right impl and wrap into the right
//! variant, on BOTH hydration sides, for ALL eight variants — a variant that
//! forwarded to the wrong arm (or failed to wrap) is caught here even where the
//! inner State is empty/default.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::uninlined_format_args)]
// rust-1.95.0 tightened these on this 02-03/02-04 equivalence-test file:
// `doc_markdown` (CONTRACT_SHAPE prose), `unused_async` (the `build_app_state`
// helper is awaited by async callers but has no interior `.await`), and
// `missing_const_for_fn` (`variant_name`). Match the sibling acceptance-test
// posture (e.g. `listener_fact_hydrate_equivalence.rs`) rather than churn them.
#![allow(clippy::doc_markdown, clippy::unused_async, clippy::missing_const_for_fn)]

use std::str::FromStr;
use std::sync::Arc;

use overdrive_control_plane::AppState;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, hydrate_actual_for_test, hydrate_desired_for_test,
};
use overdrive_core::id::NodeId;
use overdrive_core::reconcilers::TargetResource;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_reconcilers::{AnyReconciler, AnyState};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

fn node_id(name: &str) -> NodeId {
    NodeId::from_str(name).expect("valid NodeId")
}

fn target(raw: &str) -> TargetResource {
    TargetResource::new(raw).expect("valid target")
}

/// Same adapter wiring as the golden fixtures — `LocalIntentStore` +
/// `SimObservationStore` + `SimClock` + Exec `SimDriver` + default allocator +
/// `IdentityMgr::new(None)` + empty listener facts. The B-03 variant identity is
/// determined by WHICH reconciler forwards, not by the seeded data, so an empty
/// store suffices: every reconciler hydrates its own variant with an
/// empty/default inner State and never errors on absent intent/observation.
async fn build_app_state(tmp: &TempDir, obs: Arc<dyn ObservationStore>) -> AppState {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        Arc::new(SimClock::new()),
        Arc::new(SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        node_id("writer-1"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

/// Exhaustive `AnyState` → variant-name projection. The exhaustive match is
/// compile-forcing: a new `AnyState` variant breaks this until this test is
/// extended to cover its forwarding arm.
fn variant_name(s: &AnyState) -> &'static str {
    match s {
        AnyState::Unit => "Unit",
        AnyState::WorkloadLifecycle(_) => "WorkloadLifecycle",
        AnyState::WorkflowLifecycle(_) => "WorkflowLifecycle",
        AnyState::ServiceMapHydrator(_) => "ServiceMapHydrator",
        AnyState::BackendDiscoveryBridge(_) => "BackendDiscoveryBridge",
        AnyState::ServiceLifecycle(_) => "ServiceLifecycle",
        AnyState::SvidLifecycle(_) => "SvidLifecycle",
        AnyState::VmReclamation(_) => "VmReclamation",
    }
}

/// S-ROH-B-03 — for each `AnyReconciler` variant, the port-driven
/// `AnyReconciler::hydrate_desired` / `hydrate_actual` forwards to the concrete
/// impl and wraps `Self::State` into the matching `AnyState` variant, on BOTH
/// hydration sides.
#[tokio::test]
async fn any_reconciler_hydrate_forwarding_wraps_into_matching_anystate_variant() {
    let tmp = TempDir::new().expect("tmpdir");
    let obs =
        Arc::new(SimObservationStore::single_peer(node_id("local"), 0)) as Arc<dyn ObservationStore>;
    let state = build_app_state(&tmp, Arc::clone(&obs)).await;

    let cases: Vec<(&str, AnyReconciler, TargetResource)> = vec![
        ("Unit", overdrive_control_plane::noop_heartbeat(), target("workload/x")),
        (
            "WorkloadLifecycle",
            overdrive_control_plane::workload_lifecycle(),
            target("workload/x"),
        ),
        (
            "WorkflowLifecycle",
            overdrive_control_plane::workflow_lifecycle(),
            target("workflow/all"),
        ),
        (
            "ServiceMapHydrator",
            overdrive_control_plane::service_map_hydrator(std::net::Ipv4Addr::LOCALHOST),
            target("service/1"),
        ),
        (
            "BackendDiscoveryBridge",
            overdrive_control_plane::backend_discovery_bridge(
                std::net::Ipv4Addr::LOCALHOST,
                node_id("writer-1"),
            ),
            target("workload/x"),
        ),
        ("ServiceLifecycle", overdrive_control_plane::service_lifecycle(), target("workload/x")),
        ("SvidLifecycle", overdrive_control_plane::svid_lifecycle(), target("workload/x")),
        ("VmReclamation", overdrive_control_plane::vm_reclamation(), target("node/local")),
    ];

    assert_eq!(cases.len(), 8, "forwarding must cover all 8 AnyReconciler variants");

    for (expected_variant, reconciler, tgt) in &cases {
        let desired = hydrate_desired_for_test(reconciler, tgt, &state)
            .await
            .unwrap_or_else(|e| panic!("hydrate_desired forwarding for `{expected_variant}`: {e}"));
        let actual = hydrate_actual_for_test(reconciler, tgt, &state)
            .await
            .unwrap_or_else(|e| panic!("hydrate_actual forwarding for `{expected_variant}`: {e}"));

        assert_eq!(
            variant_name(&desired),
            *expected_variant,
            "AnyReconciler::hydrate_desired must wrap into AnyState::{expected_variant}",
        );
        assert_eq!(
            variant_name(&actual),
            *expected_variant,
            "AnyReconciler::hydrate_actual must wrap into AnyState::{expected_variant}",
        );
    }
}
