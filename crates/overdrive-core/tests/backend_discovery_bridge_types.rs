//! `BackendDiscoveryBridge` core types — RED unit tests for step 01-01
//! of `backend-discovery-bridge-service-reachability`.
//!
//! Pins the State/View/Action surface introduced in this step. Tests
//! enter through the public `overdrive_core::reconciler` driving surface
//! and assert observable construction + CBOR-roundtrip outcomes only —
//! no internal structure is inspected.
//!
//! Scope per `docs/feature/backend-discovery-bridge-service-reachability/
//! deliver/roadmap.json` § step 01-01:
//!
//! - `backend_discovery_bridge_view_cbor_roundtrip` — runtime owns
//!   CBOR persistence end-to-end (ADR-0035 § 3); the View MUST
//!   round-trip via `ciborium` so the runtime can persist + reload.
//! - `backend_discovery_bridge_view_serde_default_tolerates_unknown_fields`
//!   — additive serde evolution per ADR-0035 § 6 / Reconciler I/O
//!   schema-evolution. The View MUST tolerate unknown fields without
//!   error so a forward-compat reader can accept new optional fields
//!   without a versioned-envelope bump.
//! - `action_write_service_backend_row_variant_constructs` — the
//!   `Action::WriteServiceBackendRow { row, correlation }` variant
//!   exists with the documented field shape per architecture.md § 4.3.
//! - `any_reconciler_backend_discovery_bridge_variant_constructs` —
//!   the `AnyReconciler::BackendDiscoveryBridge(_)` variant exists and
//!   carries the bridge struct per architecture.md § 4.2.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::net::Ipv4Addr;

use overdrive_core::id::{CorrelationKey, ServiceId};
use overdrive_core::reconciler::backend_discovery_bridge::{
    BackendDiscoveryBridge, BackendDiscoveryBridgeView,
};
use overdrive_core::reconciler::{Action, AnyReconciler};
use overdrive_core::traits::observation_store::{LogicalTimestamp, ServiceBackendRow};

#[test]
fn backend_discovery_bridge_view_cbor_roundtrip() {
    // GIVEN: a `BackendDiscoveryBridgeView` carrying a single
    // fingerprint entry — exercise the non-default path so the
    // BTreeMap field's serde shape is observed end-to-end.
    let service_id = ServiceId::new(42).expect("any u64 is a valid ServiceId");
    let mut view = BackendDiscoveryBridgeView::default();
    view.last_written_fingerprint.insert(service_id, 0xdead_beef_cafe_babe_u64);

    // WHEN: serialize via the runtime's CBOR codec (ciborium, per
    // ADR-0035 § 3) and deserialize back.
    let mut buf = Vec::<u8>::new();
    ciborium::into_writer(&view, &mut buf).expect("ciborium serialize");
    let decoded: BackendDiscoveryBridgeView =
        ciborium::from_reader(buf.as_slice()).expect("ciborium deserialize");

    // THEN: equal to the original — CBOR roundtrip preserves the
    // fingerprint map exactly. This is the load-bearing property the
    // runtime's per-tick persist-view path relies on.
    assert_eq!(view, decoded, "BackendDiscoveryBridgeView MUST CBOR-roundtrip exactly");
}

#[test]
fn backend_discovery_bridge_view_serde_default_tolerates_unknown_fields() {
    // GIVEN: a JSON shape carrying an unknown future field alongside
    // the canonical V1 surface. Forward-compat readers (V1 binary
    // reading a V2-written file) MUST decode without error rather
    // than reject — per ADR-0035 § 6 additive serde evolution.
    //
    // Use serde_json here (already a regular dep) — both ciborium
    // and serde_json honor the same `#[serde(default)]` /
    // ignore-unknown-fields semantics, and JSON is human-readable
    // for the test fixture.
    let json = serde_json::json!({
        "last_written_fingerprint": {},
        "future_optional_knob": "ignored",
    });

    // WHEN: deserialize the json-with-unknown-field into V1 shape.
    let decoded: BackendDiscoveryBridgeView =
        serde_json::from_value(json).expect("V1 reader tolerates unknown fields");

    // THEN: equal to the default — unknown fields are silently
    // dropped; known fields decode to their default when omitted.
    assert_eq!(
        decoded,
        BackendDiscoveryBridgeView::default(),
        "unknown fields MUST NOT block deserialization (additive evolution)",
    );
}

#[test]
fn action_write_service_backend_row_variant_constructs() {
    // GIVEN: a `ServiceBackendRow` (the persisted observation shape)
    // and a `CorrelationKey` (the cause-to-response link per the
    // existing reconciler I/O convention).
    let row = ServiceBackendRow {
        service_id: ServiceId::new(42).expect("any u64 is a valid ServiceId"),
        vip: Ipv4Addr::new(10, 0, 0, 1),
        backends: Vec::new(),
        updated_at: LogicalTimestamp {
            counter: 1,
            writer: overdrive_core::id::NodeId::new("local").expect("local node id"),
        },
    };
    let correlation = CorrelationKey::derive(
        "backend-discovery-bridge/test",
        &overdrive_core::id::ContentHash::of([0_u8; 32]),
        "write-service-backend-row",
    );

    // WHEN: construct the new `Action::WriteServiceBackendRow` variant.
    let action = Action::WriteServiceBackendRow { row, correlation };

    // THEN: the variant exists and matches positively. Per the
    // task's observable-outcomes mandate we assert through the
    // `matches!` driving surface rather than inspecting internal
    // fields — destructuring would couple the test to the variant's
    // field layout rather than to the public construction surface.
    assert!(
        matches!(action, Action::WriteServiceBackendRow { .. }),
        "Action::WriteServiceBackendRow variant MUST exist with the documented shape",
    );
}

#[test]
fn any_reconciler_backend_discovery_bridge_variant_constructs() {
    // GIVEN: the canonical `BackendDiscoveryBridge` reconciler.
    // Mirrors the construction pattern every other first-party
    // reconciler uses (`WorkloadLifecycle::canonical()`,
    // `ServiceMapHydrator::canonical()`).
    let bridge = BackendDiscoveryBridge::canonical();

    // WHEN: wrap into the runtime-dispatch `AnyReconciler` enum.
    let any = AnyReconciler::BackendDiscoveryBridge(bridge);

    // THEN: the variant exists, dispatches its canonical name, and
    // round-trips through `AnyReconciler::name()` without panic.
    // The static_name() surface is the load-bearing
    // `&'static str` accessor the runtime uses to key the
    // `ViewStore`'s redb table — observing it through `matches!`
    // guards against a future variant whose `static_name` panics.
    assert_eq!(
        any.static_name(),
        BackendDiscoveryBridge::NAME,
        "AnyReconciler::BackendDiscoveryBridge MUST dispatch static_name() \
         to the bridge's NAME const",
    );
    assert_eq!(
        any.name().as_str(),
        BackendDiscoveryBridge::NAME,
        "AnyReconciler::BackendDiscoveryBridge MUST dispatch name() to the \
         bridge's canonical ReconcilerName",
    );
}
