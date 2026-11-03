//! Crate-private HTTP/1.1 message-framing modules of the kernel relay.
//!
//! Data path of ADR-0245, ADR-0239, ADR-0247 and ADR-0254.
//!
//! SCAFFOLD: true. Only the shapes the DESIGN pins are present: the
//! [`exchange`] module (`ExchangeMachine` and its closed vocabulary, the
//! Quint conformance target of `specs/quint/public-ingress-gateway/
//! relay_exchange.qnt`) and its conformance driver, the pure [`head`]
//! functions (head validator, request/response parsers, framing decision,
//! chunk-line parser) and the pure [`deadlines`] composition, whose
//! signatures R-32 pins for the source-local tests.

#![allow(
    clippy::redundant_pub_crate,
    reason = "the DESIGN pins these items `pub(crate)`: crate-private, reached by the exchange driver and the Quint conformance driver"
)]

pub(crate) mod deadlines;
pub(crate) mod exchange;
pub(crate) mod head;

#[cfg(all(test, feature = "integration-tests"))]
mod quint_exchange;
