//! `BackendDiscoveryBridge` core types — RED unit tests for step 01-01
//! of `backend-discovery-bridge-service-reachability`.
//!
//! Pins the State/View/Action surface introduced in this step. Tests
//! enter through the public `overdrive_core::reconcilers` driving surface
//! and assert observable construction + CBOR-roundtrip outcomes only —
//! no internal structure is inspected.
//!
//! Scope per `docs/feature/backend-discovery-bridge-service-reachability/
//! deliver/roadmap.json` § step 01-01:
//!
//! - `legacy_bridge_view_blob_decodes_to_empty_view` (T-BDB-VIEW-1,
//!   ADR-0079 § D3 / § D7) — the runtime owns CBOR persistence
//!   end-to-end (ADR-0035 § 3), and § D3 REMOVED the View's only
//!   field. This asserts the removal direction of serde's
//!   unknown-field tolerance against a payload that names the removed
//!   field deliberately, rather than arguing from serde's documented
//!   default.
//! - `action_write_service_backend_row_variant_constructs` — the
//!   `Action::WriteServiceBackendRow { row, correlation }` variant
//!   exists with the documented field shape per architecture.md § 4.3.
//! - `any_reconciler_backend_discovery_bridge_variant_constructs` —
//!   the `AnyReconciler::BackendDiscoveryBridge(_)` variant exists and
//!   carries the bridge struct per architecture.md § 4.2.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use overdrive_core::id::{CorrelationKey, NodeId, ServiceId};
use overdrive_core::reconcilers::Action;
use overdrive_core::traits::observation_store::{LogicalTimestamp, ServiceBackendRow};
use overdrive_reconcilers::AnyReconciler;
use overdrive_reconcilers::backend_discovery_bridge::{
    BackendDiscoveryBridge, BackendDiscoveryBridgeView,
};

/// T-BDB-VIEW-1 (ADR-0079 § D3 / § D7) — a CBOR blob written by a
/// PRE-ADR-0079 binary, carrying a populated `last_written_fingerprint`
/// map, still decodes into the now-field-less
/// `BackendDiscoveryBridgeView`.
///
/// This is the substrate-level proof of § D3's field-removal claim,
/// asserted rather than assumed: ciborium encodes a struct as a
/// string-keyed map and this type does not set `deny_unknown_fields`,
/// so the removed key is simply ignored on read. Legacy rows in the
/// bridge's redb table therefore stay inert instead of failing
/// `bulk_load` at boot.
///
/// It replaces `backend_discovery_bridge_view_cbor_roundtrip` (which
/// degenerates to a tautology once the type has no fields) and folds in
/// `backend_discovery_bridge_view_serde_default_tolerates_unknown_fields`
/// — that sibling still compiled and passed verbatim after the removal,
/// and THAT is the problem: it silently stopped testing what its name
/// claimed, because `last_written_fingerprint` had become just another
/// unknown key. This test names the removed field deliberately.
#[test]
fn legacy_bridge_view_blob_decodes_to_empty_view() {
    // GIVEN: the exact CBOR a pre-ADR-0079 binary persisted — a map
    // keyed by the now-deleted field name, carrying a real entry.
    let service_id = ServiceId::new(42).expect("any u64 is a valid ServiceId");
    let legacy: BTreeMap<&str, BTreeMap<ServiceId, u64>> = BTreeMap::from([(
        "last_written_fingerprint",
        BTreeMap::from([(service_id, 0xdead_beef_cafe_babe_u64)]),
    )]);
    let mut buf = Vec::<u8>::new();
    ciborium::into_writer(&legacy, &mut buf).expect("ciborium serialize the legacy blob");

    // WHEN: the current binary's `bulk_load` decodes it.
    let decoded: BackendDiscoveryBridgeView =
        ciborium::from_reader(buf.as_slice()).expect("legacy blob MUST still decode");

    // THEN: the removed field is dropped and the View is the empty
    // default — no error, no versioned envelope, no migration.
    assert_eq!(
        decoded,
        BackendDiscoveryBridgeView::default(),
        "a persisted `last_written_fingerprint` blob MUST decode into the \
         field-less View (ADR-0079 § D3 field-removal tolerance)",
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
    // GIVEN: a `BackendDiscoveryBridge` constructed with mandatory
    // `host_ipv4` and `writer_node_id` parameters per step 01-02
    // (`.claude/rules/development.md` § "Port-trait dependencies"
    // — required, not defaulted).
    let bridge = BackendDiscoveryBridge::new(
        Ipv4Addr::new(10, 0, 0, 1),
        NodeId::new("node-1").expect("'node-1' is a valid NodeId"),
    );

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
