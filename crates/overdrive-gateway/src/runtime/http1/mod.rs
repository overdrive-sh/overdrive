//! Crate-private HTTP/1.1 message-framing modules of the kernel relay.
//!
//! Data path of ADR-0245, ADR-0239, ADR-0247 and ADR-0254.
//!
//! SCAFFOLD: true. Only the shapes the DESIGN pins are present: the
//! [`exchange`] module (`ExchangeMachine` and its closed vocabulary, the
//! Quint conformance target of `specs/quint/public-ingress-gateway/
//! relay_exchange.qnt`) and its conformance driver. The pure head
//! validator, request/response parsers, framing decision and chunk-line
//! parser are crate-private implementation whose function signatures the
//! DESIGN leaves to the crafter; the behaviour they must deliver is
//! specified at the public listener by the relay integration file
//! (`tests/integration/public_ingress_gateway_relay.rs`).

#![allow(
    clippy::redundant_pub_crate,
    reason = "the DESIGN pins these items `pub(crate)`: crate-private, reached by the exchange driver and the Quint conformance driver"
)]

pub(crate) mod exchange;

#[cfg(all(test, feature = "integration-tests"))]
mod quint_exchange;
