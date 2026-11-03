//! Public-ingress real-store, production-composition and Sim-relay exchange
//! acceptance entrypoint.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod integration {
    mod public_ingress_gateway;
    mod public_ingress_gateway_relay;
}
