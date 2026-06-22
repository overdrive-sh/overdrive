//! S-BRIDGE — `BackendDiscoveryBridge` advertises the canonical workload address
//! when present, the host address otherwise (DISTILL RED scaffold, GH #241,
//! Tier-1 DST / reconciler-logic, default-lane).
//!
//! D-B2 / `@us-B2`. The driving port is `BackendDiscoveryBridge::reconcile`
//! (the function signature IS the port — port-to-port at the domain layer).
//! Observable outcomes on the returned `(Vec<Action>, View)`:
//!
//!   - a Running alloc with `Some(workload_addr)` -> advertised
//!     `Backend.addr == workload_addr:listener_port`;
//!   - with `None` -> `host_ipv4:listener_port` (fallback UNCHANGED);
//!   - `ServiceBackendRow.vip` UNCHANGED in BOTH arms (the dialable-VIP path is
//!     #61 territory, orthogonal).
//!
//! The `None`-fallback arm is the error/edge coverage (host-netns workload).
//!
//! Mandate 8 (Universe — port-exposed names only): the reconcile-returned
//! actions' `backend_addr` + `service_vip` + the `View`'s advertised
//! fingerprint; NEVER the bridge's private fields.
//! Mandate 9: Tier-1 in-memory acceptance -> PBT-eligible (proptest over
//! `{Some(addr) | None} x listener_port`), with an `@example`-pinned canonical
//! mesh row + host row preserved for the reviewer.
//!
//! Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-BRIDGE.
//!
//! DELIVER replaces the panic body with `BackendDiscoveryBridge::reconcile`
//! driven over the two arms + `assert_state_delta`-shaped universe assertions.

#[test]
#[should_panic(expected = "RED scaffold")]
fn bridge_advertises_canonical_workload_address_when_present() {
    panic!(
        "Not yet implemented -- RED scaffold (S-BRIDGE / a Running alloc with \
         Some(workload_addr) -> Backend.addr == workload_addr:listener_port; \
         ServiceBackendRow.vip UNCHANGED)"
    );
}

#[test]
#[should_panic(expected = "RED scaffold")]
fn bridge_falls_back_to_host_address_for_host_netns_workload() {
    panic!(
        "Not yet implemented -- RED scaffold (S-BRIDGE error/edge / a Running \
         alloc with None workload_addr -> Backend.addr == host_ipv4:listener_port \
         (fallback UNCHANGED); ServiceBackendRow.vip UNCHANGED)"
    );
}
