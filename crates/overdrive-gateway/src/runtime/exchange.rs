//! The exchange driver of one public connection: two concurrent halves per
//! request over the `GatewayRelay` port, driven by `ExchangeMachine`.
//!
//! SCAFFOLD: true. The DESIGN pins the driver's behaviour (Kernel relay data
//! path, "Message-framing contract", "Exchange state machine") but no public
//! or cross-module item: the driver, its deadline composition and its
//! status mapping are crate-private structure. Its behaviour is specified
//! through the public listener by
//! `tests/integration/public_ingress_gateway_relay.rs` and replayed against
//! the pure owner by `http1::quint_exchange`.
