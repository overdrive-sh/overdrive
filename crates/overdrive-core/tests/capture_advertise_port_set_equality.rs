//! S-PORTSET — the inbound-capture port-set equals the advertise port-set
//! (DISTILL RED scaffold, GH #241, Tier-1 DST, PROPERTY — DELIVER obligation #1).
//!
//! `@us-portset @property`. For an N>=2-listener Service, the inbound-rule
//! port-set (`project_service_listen_ports(intent)` ->
//! `AllocationSpec.service_ports`) MUST EQUAL the advertise port-set (the bridge
//! reading `desired.listeners` ports). Same intent source, two code paths ->
//! latent drift risk; the AC asserts BYTE-SET EQUALITY (DELIVER obligation #1).
//!
//! Mandate 8 (Universe): `projection.service_ports_set` +
//! `advertise.listener_ports_set` with the invariant `projection == advertise`.
//! Mandate 9: Tier-1 `@property` -> PBT FULL. The crafter generates an arbitrary
//! non-empty set of `NonZeroU16` listener ports (N >= 2) and asserts set equality
//! across both read paths — the canonical "property over a domain-rich input
//! space" case the `@property` tag signals.
//!
//! Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-PORTSET.
//!
//! DELIVER replaces the panic body with a `proptest` over an arbitrary N>=2
//! listener-port set, reading both `project_service_listen_ports` and the bridge
//! advertise path off the same Service intent and asserting byte-set equality.

#[test]
#[should_panic(expected = "RED scaffold")]
fn every_captured_port_is_an_advertised_port_for_a_multi_listener_service() {
    panic!(
        "Not yet implemented -- RED scaffold (S-PORTSET @property / for an \
         N>=2-listener Service, project_service_listen_ports set EQUALS the \
         bridge advertise listener-port set -- byte-set equality, no captured \
         port missing from advertised and vice versa)"
    );
}
