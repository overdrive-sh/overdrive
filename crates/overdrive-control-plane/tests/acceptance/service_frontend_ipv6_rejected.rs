//! S-01-F — IPv6 VIP rejected at the action-shim with the existing
//! operator-visible `Failed` row (udp-service-support US-01; ADR-0060
//! D1a).
//!
//! **RED scaffold** (`#[should_panic(expected = "RED scaffold")]`).
//! Pins that `ServiceFrontend::new`'s IPv4 validation stays at the
//! **operator-visible** action-shim rejection site (`ipv4_from_vip` →
//! `ServiceHydrationStatus::Failed`, `dataplane_update_service.rs:110,160`)
//! and is NOT demoted to a late opaque `DataplaneError` deep in an
//! adapter.
//!
//! Scenario SSOT:
//! `docs/feature/udp-service-support/distill/test-scenarios.md` S-01-F.
//!
//! Driving port: the action-shim `dispatch`. Observable outcome (the
//! universe at this layer): the written `ServiceHydrationStatus::Failed`
//! observation row with reason `Ipv6Unsupported`. GREEN: DELIVER routes
//! the action-shim build of the frontend through `ServiceFrontend::new`,
//! drops `#[should_panic]`, and asserts the Failed row is written with
//! the existing reason — unchanged from today's `ipv4_from_vip` path.

#[test]
#[should_panic(expected = "RED scaffold")]
fn ipv6_vip_rejected_at_action_shim_as_operator_visible_failed() {
    panic!(
        "Not yet implemented -- RED scaffold (S-01-F / IPv6 VIP rejected at the \
         action-shim via ServiceFrontend::new as an operator-visible Failed row \
         [reason Ipv6Unsupported]; NOT a late opaque DataplaneError)"
    );
}
