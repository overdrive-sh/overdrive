//! Public-ingress real-store and production-composition acceptance entrypoint.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod integration {
    mod public_ingress_gateway;
}
