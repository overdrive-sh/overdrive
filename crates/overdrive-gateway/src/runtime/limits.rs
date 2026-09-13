//! Fixed first-slice gateway resource limits.
//!
//! SCAFFOLD: true. The constructor remains RED until DELIVER.
//! The source-local property implements S-PIG-17.

#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::time::Duration;

/// Finite resource and deadline policy for the first public-ingress slice.
#[derive(Debug, Clone)]
pub struct GatewayLimits {
    max_public_connections: NonZeroU32,
    max_inflight_connects: NonZeroU32,
    max_inflight_requests: NonZeroU32,
    max_cleanup_ledger_entries: NonZeroU32,
    max_request_head_bytes: NonZeroUsize,
    max_request_headers: NonZeroUsize,
    max_response_head_bytes: NonZeroUsize,
    max_response_headers: NonZeroUsize,
    max_requests_per_connection: NonZeroU32,
    max_request_body_bytes: NonZeroU64,
    max_response_body_bytes: NonZeroU64,
    application_gate_deadline: Duration,
    tls_handshake_deadline: Duration,
    request_head_deadline: Duration,
    upstream_connect_deadline: Duration,
    response_head_deadline: Duration,
    body_no_progress_deadline: Duration,
    keep_alive_idle_deadline: Duration,
    whole_request_deadline: Duration,
}

impl GatewayLimits {
    /// Construct the one fixed first-slice policy.
    pub fn first_slice() -> Self {
        todo!("SCAFFOLD: GatewayLimits::first_slice")
    }
    pub const fn max_public_connections(&self) -> NonZeroU32 {
        self.max_public_connections
    }
    pub const fn max_inflight_connects(&self) -> NonZeroU32 {
        self.max_inflight_connects
    }
    pub const fn max_inflight_requests(&self) -> NonZeroU32 {
        self.max_inflight_requests
    }
    pub const fn max_cleanup_ledger_entries(&self) -> NonZeroU32 {
        self.max_cleanup_ledger_entries
    }
    pub const fn max_request_head_bytes(&self) -> NonZeroUsize {
        self.max_request_head_bytes
    }
    pub const fn max_request_headers(&self) -> NonZeroUsize {
        self.max_request_headers
    }
    pub const fn max_response_head_bytes(&self) -> NonZeroUsize {
        self.max_response_head_bytes
    }
    pub const fn max_response_headers(&self) -> NonZeroUsize {
        self.max_response_headers
    }
    pub const fn max_requests_per_connection(&self) -> NonZeroU32 {
        self.max_requests_per_connection
    }
    pub const fn max_request_body_bytes(&self) -> NonZeroU64 {
        self.max_request_body_bytes
    }
    pub const fn max_response_body_bytes(&self) -> NonZeroU64 {
        self.max_response_body_bytes
    }
    pub const fn application_gate_deadline(&self) -> Duration {
        self.application_gate_deadline
    }
    pub const fn tls_handshake_deadline(&self) -> Duration {
        self.tls_handshake_deadline
    }
    pub const fn request_head_deadline(&self) -> Duration {
        self.request_head_deadline
    }
    pub const fn upstream_connect_deadline(&self) -> Duration {
        self.upstream_connect_deadline
    }
    pub const fn response_head_deadline(&self) -> Duration {
        self.response_head_deadline
    }
    pub const fn body_no_progress_deadline(&self) -> Duration {
        self.body_no_progress_deadline
    }
    pub const fn keep_alive_idle_deadline(&self) -> Duration {
        self.keep_alive_idle_deadline
    }
    pub const fn whole_request_deadline(&self) -> Duration {
        self.whole_request_deadline
    }
}

#[cfg(test)]
mod properties {
    use super::*;

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER fixed gateway limits step"]
    fn first_slice_limits_are_exact_finite_and_ordered() {
        let limits = GatewayLimits::first_slice();
        assert_eq!(limits.max_public_connections().get(), 128);
        assert_eq!(limits.max_inflight_connects().get(), 128);
        assert_eq!(limits.max_inflight_requests().get(), 128);
        assert_eq!(limits.max_cleanup_ledger_entries().get(), 128);
        assert_eq!(limits.max_request_head_bytes().get(), 32 * 1024);
        assert_eq!(limits.max_response_head_bytes().get(), 32 * 1024);
        assert_eq!(limits.max_request_headers().get(), 100);
        assert_eq!(limits.max_response_headers().get(), 100);
        assert_eq!(limits.max_requests_per_connection().get(), 100);
        assert_eq!(limits.max_request_body_bytes().get(), 16 * 1024 * 1024);
        assert_eq!(limits.max_response_body_bytes().get(), 16 * 1024 * 1024);
        assert_eq!(limits.application_gate_deadline(), Duration::from_secs(30));
        assert_eq!(limits.tls_handshake_deadline(), Duration::from_secs(5));
        assert_eq!(limits.request_head_deadline(), Duration::from_secs(5));
        assert_eq!(limits.upstream_connect_deadline(), Duration::from_secs(5));
        assert_eq!(limits.response_head_deadline(), Duration::from_secs(5));
        assert_eq!(limits.body_no_progress_deadline(), Duration::from_secs(30));
        assert_eq!(limits.keep_alive_idle_deadline(), Duration::from_secs(30));
        assert_eq!(limits.whole_request_deadline(), Duration::from_secs(120));
        assert!(limits.whole_request_deadline() > limits.body_no_progress_deadline());
    }
}
