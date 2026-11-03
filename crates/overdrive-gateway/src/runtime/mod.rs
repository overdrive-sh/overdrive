//! Gateway runtime modules.
//!
//! Fixed limits, public authority normalization, proxy-header rewriting and
//! the kernel-relay `http1` exchange state machine. The head validator,
//! parsers, framing decision, chunk-line parser and the exchange driver are
//! crate-private implementation of the DESIGN contracts and are DELIVER's.

pub mod limits;
pub mod proxy_headers;
pub mod routing;

pub(crate) mod exchange;
pub(crate) mod http1;

pub use limits::GatewayLimits;
