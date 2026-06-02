//! Tier-1 C3-guard scaffolds — proto provenance for the desired
//! projection (udp-service-support US-01; ADR-0060 site #8; ATLAS-1 b).
//!
//! **RED scaffolds** (`#[should_panic(expected = "RED scaffold")]`).
//! These pin the load-bearing C3 defense: the protocol carried into the
//! `ServiceFrontend` / `Action::DataplaneUpdateService` MUST be sourced
//! from a **listener-bearing fact** (`ListenerRow`,
//! `observation_store.rs:321` — carries `(port, protocol, vip)`; and/or
//! the `BackendDiscoveryBridge` per-listener projection), and an
//! unresolvable listener protocol MUST be a structured error, NEVER a
//! silent `Proto::Tcp` default (constraint C3).
//!
//! Scenario SSOT:
//! `docs/feature/udp-service-support/distill/test-scenarios.md`
//! - S-01-C: a udp listener's protocol reaches the dataplane as Udp,
//!   never defaulted to Tcp.
//! - S-01-D: the desired projection sources protocol from the listener
//!   fact, NOT from the proto-less `service_backends` row.
//! - S-01-E: NEGATIVE — an unresolvable listener protocol produces a
//!   structured Failed, NOT a silent `Tcp`-defaulted action.
//!
//! Why this is a first-class scenario (ATLAS-1 b, carried forward from
//! DESIGN review): `ServiceBackendRowV1` (`observation_store.rs:875`)
//! carries neither port nor proto, and `hydrate_desired`
//! (`reconciler_runtime.rs:1322-1348`) reads only `service_backends_rows`
//! today — so a crafter implementing site #8 against the literal "carried
//! from `service_backends`" text could synthesise a `Tcp` default (a C3
//! violation). S-01-E's negative arm makes that violation a failing test.
//!
//! Driving port: `ServiceMapHydrator.reconcile` (the desired→Action
//! emission seam; `service_map_hydrator.rs:40,235,263`). GREEN: DELIVER
//! adds the protocol dimension to `ServiceDesired` + the obs→desired
//! projection, drops `#[should_panic]`, and fills the assertions in the
//! single-cut US-01 PR.

#![allow(clippy::expect_used, clippy::unwrap_used)]

// S-01-C — a udp listener's protocol reaches the dataplane as Udp, never
// defaulted to Tcp. Assert the emitted ServiceFrontend / action proto is
// Udp for a service whose desired projection is sourced from a listener
// fact declaring protocol=udp on port 5353.
#[test]
#[should_panic(expected = "RED scaffold")]
fn udp_listener_protocol_reaches_dataplane_as_udp() {
    panic!(
        "Not yet implemented -- RED scaffold (S-01-C / a udp listener's proto \
         reaches update_service as Udp, no Tcp default on the path)"
    );
}

// S-01-D — the desired projection sources protocol from the
// listener-bearing fact (ListenerRow / BackendDiscoveryBridge per-listener
// projection), NOT from the proto-less service_backends row.
#[test]
#[should_panic(expected = "RED scaffold")]
fn proto_sourced_from_listener_fact_not_service_backends() {
    panic!(
        "Not yet implemented -- RED scaffold (S-01-D / desired projection reads \
         proto from the listener-bearing fact, not the proto-less service_backends row)"
    );
}

// S-01-E — NEGATIVE (the C3 load-bearing defense): a desired projection
// with NO resolvable listener protocol produces a structured Failed,
// and does NOT emit a ServiceFrontend / update_service action with a
// silently-defaulted Proto::Tcp. The test asserts BOTH the presence of
// the structured Failed AND the absence of a Tcp-defaulted action.
#[test]
#[should_panic(expected = "RED scaffold")]
fn unresolvable_listener_proto_is_structured_error_not_tcp_default() {
    panic!(
        "Not yet implemented -- RED scaffold (S-01-E / unresolvable listener proto \
         -> structured Failed; NEVER a silent Proto::Tcp-defaulted action [C3])"
    );
}
