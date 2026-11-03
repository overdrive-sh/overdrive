//! Pure gateway runtime modules: fixed limits, public authority
//! normalization and proxy-header rewriting. The kernel-relay `http1`
//! framing modules and the exchange driver are added by the relay batch.

pub mod limits;
pub mod proxy_headers;
pub mod routing;

pub use limits::GatewayLimits;
