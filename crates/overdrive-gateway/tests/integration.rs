//! Public-ingress real-store, production-composition and Sim-relay exchange
//! acceptance entrypoint.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod integration {
    #[path = "../../../overdrive-control-plane/tests/common/issue_fault_store.rs"]
    mod issue_fault_store;
    mod public_ingress_gateway;
    mod public_ingress_gateway_relay;
}
