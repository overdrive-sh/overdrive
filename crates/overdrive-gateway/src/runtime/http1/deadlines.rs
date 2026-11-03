//! Pure deadline composition of the kernel relay (S-PIG-85; R-32, R-37).
//!
//! SCAFFOLD: true. Only the signatures the DESIGN pins are present; every body
//! is DELIVER's and fails with SCAFFOLD text. No function reads a clock: `now`
//! is an input.

#![allow(
    clippy::todo,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::redundant_pub_crate,
    dead_code,
    unused_variables,
    reason = "public-ingress DISTILL scaffold: bodies are DELIVER's"
)]

use std::time::Instant;

use crate::ports::ReadDeadlines;
use crate::runtime::GatewayLimits;

/// The whole-request ceiling of a request whose first octet arrived at
/// `first_octet_at`.
pub(crate) fn request_deadline(first_octet_at: Instant, limits: &GatewayLimits) -> Instant {
    todo!("SCAFFOLD: http1::request_deadline")
}

/// The `ReadDeadlines` the driver passes to `read_through` for each phase,
/// already reduced to `min(phase budget, request_deadline)`.
pub(crate) fn compose_read_deadlines(
    phase: ReadPhase,
    now: Instant,
    request_deadline: Instant,
    limits: &GatewayLimits,
) -> ReadDeadlines {
    todo!("SCAFFOLD: http1::compose_read_deadlines")
}

/// The read the deadlines are composed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadPhase {
    FirstRequestHead,
    LaterRequestHead,
    FramingLine,
    /// `request_complete_at`: `Some(t0)` once the upstream request is
    /// completely written (R-37), else `None`.
    ResponseHead {
        request_complete_at: Option<Instant>,
    },
}
