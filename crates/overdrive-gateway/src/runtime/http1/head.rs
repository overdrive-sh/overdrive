//! The pure head, framing and chunk-line functions of the kernel relay
//! (ADR-0245; R-32).
//!
//! SCAFFOLD: true. Only the signatures and closed vocabularies the DESIGN pins
//! ("Pure `http1` and relay-pipe functions") are present; every body is
//! DELIVER's and fails with SCAFFOLD text. No function reads a clock, a socket
//! or a global.

#![allow(
    clippy::todo,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_self,
    clippy::redundant_pub_crate,
    dead_code,
    unused_variables,
    reason = "public-ingress DISTILL scaffold: bodies are DELIVER's"
)]

use overdrive_core::public_ingress::PublicHostname;

use super::exchange::BodyFraming;
use crate::runtime::GatewayLimits;

/// Byte-level validator applied to every line the gateway reads: a request or
/// response head (the whole header block), a chunk-size line, a chunk
/// terminator, a trailer line. Runs before any parser.
pub(crate) fn validate_lines(bytes: &[u8]) -> Result<(), LineViolation> {
    todo!("SCAFFOLD: http1::validate_lines")
}

/// Why a line failed the byte-level validator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineViolation {
    BareCr,
    BareLf,
    Nul,
    OctetOutsideFieldValueRange,
    ObsoleteLineFold,
}

/// Request head -> typed value. `sni` is the handshake's server name
/// (Host/SNI equality, ADR-0236).
pub(crate) fn parse_request_head(
    bytes: &[u8],
    sni: &PublicHostname,
    limits: &GatewayLimits,
) -> Result<ParsedRequestHead, RequestHeadError> {
    todo!("SCAFFOLD: http1::parse_request_head")
}

/// A validated request head: method, origin-form target, original header map,
/// Host authority and framing.
pub(crate) struct ParsedRequestHead {
    _private: (),
}

impl ParsedRequestHead {
    pub(crate) fn framing(&self) -> BodyFraming {
        todo!("SCAFFOLD: ParsedRequestHead::framing")
    }
    pub(crate) fn method_is_head(&self) -> bool {
        todo!("SCAFFOLD: ParsedRequestHead::method_is_head")
    }
}

/// Why a request head was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RequestHeadError {
    Line(LineViolation),
    VersionNotHttp11,
    Http10,
    Connect,
    NonOriginFormTarget,
    HostMissing,
    HostDuplicate,
    HostInvalid,
    HostUserinfo,
    HostPortNot443,
    HostSniMismatch,
    Framing(FramingFault),
    DeclaredLengthAboveCeiling,
    HeadTooLarge,
    TooManyHeaders,
}

impl RequestHeadError {
    /// 400, 413, 421, 431 or 505 per the Request rules table.
    pub(crate) fn status(&self) -> http::StatusCode {
        todo!("SCAFFOLD: RequestHeadError::status")
    }
}

/// Response head -> typed value. `request_was_head` selects the no-body rule of
/// the response.
pub(crate) fn parse_response_head(
    bytes: &[u8],
    request_was_head: bool,
    limits: &GatewayLimits,
) -> Result<ParsedResponseHead, ResponseHeadError> {
    todo!("SCAFFOLD: http1::parse_response_head")
}

/// A validated response head.
pub(crate) enum ParsedResponseHead {
    /// An interim head: `100` (forwarded) or another `1xx` other than `101`
    /// (dropped).
    Interim { status: http::StatusCode, forward: bool },
    /// A final head: status, original header map, framing.
    Final(ParsedFinalResponseHead),
}

/// A validated final response head.
pub(crate) struct ParsedFinalResponseHead {
    _private: (),
}

impl ParsedFinalResponseHead {
    pub(crate) fn framing(&self) -> BodyFraming {
        todo!("SCAFFOLD: ParsedFinalResponseHead::framing")
    }
}

/// Why a response head was refused; every variant is a 502 before the public
/// head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResponseHeadError {
    Line(LineViolation),
    Switching101,
    VersionNotAccepted,
    Framing(FramingFault),
    DeclaredLengthAboveCeiling,
    HeadTooLarge,
    TooManyHeaders,
}

/// The one framing decision (RFC 9112 section 6.3), over the ORIGINAL headers:
/// nothing a `Connection` field nominates has been removed yet (S-PIG-81).
/// `role` selects the request rules (no framing = no body) or the response
/// rules.
pub(crate) fn derive_framing(
    headers: &http::HeaderMap,
    role: FramingRole,
) -> Result<BodyFraming, FramingFault> {
    todo!("SCAFFOLD: http1::derive_framing")
}

/// Which side of the exchange the framing decision is made for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FramingRole {
    Request,
    FinalResponse { request_was_head: bool, status: http::StatusCode },
}

/// Why the framing decision refused a header map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FramingFault {
    BothLengthAndCoding,
    RepeatedContentLength,
    ContentLengthNotDecimal,
    ContentLengthOverflow,
    RepeatedTransferEncoding,
    RepeatedChunked,
    FinalCodingNotChunked,
    UnsupportedCoding,
}

/// Hex chunk size of one chunk-size line as returned by `read_through`
/// (terminator included); extensions are discarded.
pub(crate) fn parse_chunk_size_line(line: &[u8]) -> Result<u64, ChunkLineError> {
    todo!("SCAFFOLD: http1::parse_chunk_size_line")
}

/// The chunk terminator is exactly CRLF.
pub(crate) fn check_chunk_terminator(line: &[u8]) -> Result<(), ChunkLineError> {
    todo!("SCAFFOLD: http1::check_chunk_terminator")
}

/// `true` for the empty line that ends the trailer; other lines are validated
/// and dropped.
pub(crate) fn is_trailer_end(line: &[u8]) -> Result<bool, ChunkLineError> {
    todo!("SCAFFOLD: http1::is_trailer_end")
}

/// Why a chunk framing line was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChunkLineError {
    Line(LineViolation),
    SizeNotHex,
    SizeOverflow,
    SizeLineMalformed,
    TerminatorNotCrlf,
}
