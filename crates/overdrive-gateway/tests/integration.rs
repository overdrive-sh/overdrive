//! Public-ingress real-store, production-composition and Sim-relay exchange
//! acceptance entrypoint.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod integration {
    #[path = "../../../overdrive-store-local/tests/integration/envelope_helpers.rs"]
    #[allow(
        dead_code,
        clippy::expect_used,
        reason = "shared raw-row back-door; only one helper is used here"
    )]
    mod envelope_helpers;
    #[path = "../../../overdrive-control-plane/tests/common/issue_fault_store.rs"]
    mod issue_fault_store;
    mod public_ingress_gateway;
    mod public_ingress_gateway_relay;
}
