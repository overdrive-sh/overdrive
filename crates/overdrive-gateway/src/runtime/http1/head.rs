//! The pure head, framing and chunk-line functions of the kernel relay
//! (ADR-0245; R-32).
//!
//! No function reads a clock, a socket or a global.

use http::header::{CONTENT_LENGTH, HeaderName, HeaderValue, TRANSFER_ENCODING};
use http::{HeaderMap, Method, StatusCode};
use overdrive_core::public_ingress::PublicHostname;

use super::exchange::BodyFraming;
use crate::runtime::GatewayLimits;
use crate::runtime::routing::{RouteAuthorityError, normalize_route_authority};

const CRLF: &[u8] = b"\r\n";
const HEADER_BLOCK_END: &[u8] = b"\r\n\r\n";
const HTTP_11: &[u8] = b"HTTP/1.1";
const HTTP_10: &[u8] = b"HTTP/1.0";

/// Byte-level validator applied to every line the gateway reads: a request or
/// response head (the whole header block), a chunk-size line, a chunk
/// terminator, a trailer line. Runs before any parser.
pub(crate) fn validate_lines(bytes: &[u8]) -> Result<(), LineViolation> {
    let mut line_start = true;
    let mut first_line = true;
    let mut index = 0;
    while let Some(&octet) = bytes.get(index) {
        match octet {
            b'\r' => {
                if bytes.get(index + 1) != Some(&b'\n') {
                    return Err(LineViolation::BareCr);
                }
                index += 2;
                line_start = true;
                first_line = false;
                continue;
            }
            b'\n' => return Err(LineViolation::BareLf),
            0 => return Err(LineViolation::Nul),
            b'\t' | b' ' if line_start && !first_line => {
                return Err(LineViolation::ObsoleteLineFold);
            }
            b'\t' | b' ' => {}
            0x01..=0x1f | 0x7f => return Err(LineViolation::OctetOutsideFieldValueRange),
            _ => {}
        }
        line_start = false;
        index += 1;
    }
    Ok(())
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
    validate_lines(bytes).map_err(RequestHeadError::Line)?;
    if bytes.len() > limits.max_request_head_bytes().get() {
        return Err(RequestHeadError::HeadTooLarge);
    }
    let lines = head_lines(bytes);
    let (start_line, field_lines) =
        lines.split_first().ok_or(RequestHeadError::NonOriginFormTarget)?;
    let (method, target) = parse_request_line(start_line)?;
    let headers = parse_fields(field_lines, limits.max_request_headers().get()).map_err(
        |error| match error {
            FieldsError::TooMany => RequestHeadError::TooManyHeaders,
            FieldsError::Malformed => {
                RequestHeadError::Line(LineViolation::OctetOutsideFieldValueRange)
            }
        },
    )?;
    let authority =
        normalize_route_authority(&headers, sni).map_err(RequestHeadError::Authority)?;
    let framing =
        derive_framing(&headers, FramingRole::Request).map_err(RequestHeadError::Framing)?;
    if let BodyFraming::Length(declared) = framing
        && declared > limits.max_request_body_bytes().get()
    {
        return Err(RequestHeadError::DeclaredLengthAboveCeiling);
    }
    Ok(ParsedRequestHead { method, target, headers, authority, framing })
}

/// A validated request head: method, origin-form target, original header map,
/// Host authority and framing.
pub(crate) struct ParsedRequestHead {
    method: Method,
    target: String,
    headers: HeaderMap,
    authority: PublicHostname,
    framing: BodyFraming,
}

impl ParsedRequestHead {
    pub(crate) const fn framing(&self) -> BodyFraming {
        self.framing
    }
    pub(crate) fn method_is_head(&self) -> bool {
        self.method == Method::HEAD
    }
    pub(crate) const fn method(&self) -> &Method {
        &self.method
    }
    pub(crate) fn target(&self) -> &str {
        &self.target
    }
    /// The validated Host authority (equal to the handshake's server name).
    pub(crate) const fn authority(&self) -> &PublicHostname {
        &self.authority
    }
    /// The ORIGINAL header map, nothing removed.
    pub(crate) fn into_headers(self) -> HeaderMap {
        self.headers
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
    Authority(RouteAuthorityError),
    Framing(FramingFault),
    DeclaredLengthAboveCeiling,
    HeadTooLarge,
    TooManyHeaders,
}

impl RequestHeadError {
    /// 400, 413, 421, 431 or 505 per the Request rules table.
    pub(crate) const fn status(&self) -> http::StatusCode {
        match self {
            Self::Line(_) | Self::Connect | Self::NonOriginFormTarget | Self::Framing(_) => {
                StatusCode::BAD_REQUEST
            }
            Self::Authority(RouteAuthorityError::SniMismatch) => StatusCode::MISDIRECTED_REQUEST,
            Self::Authority(_) => StatusCode::BAD_REQUEST,
            Self::DeclaredLengthAboveCeiling => StatusCode::PAYLOAD_TOO_LARGE,
            Self::HeadTooLarge | Self::TooManyHeaders => {
                StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
            }
            Self::VersionNotHttp11 | Self::Http10 => StatusCode::HTTP_VERSION_NOT_SUPPORTED,
        }
    }
}

fn parse_request_line(line: &[u8]) -> Result<(Method, String), RequestHeadError> {
    let mut parts = line.split(|octet| *octet == b' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(RequestHeadError::NonOriginFormTarget);
    };
    if version == HTTP_10 {
        return Err(RequestHeadError::Http10);
    }
    if version != HTTP_11 {
        return Err(RequestHeadError::VersionNotHttp11);
    }
    let method = Method::from_bytes(method).map_err(|_| RequestHeadError::NonOriginFormTarget)?;
    if method == Method::CONNECT {
        return Err(RequestHeadError::Connect);
    }
    if target.first() != Some(&b'/') {
        return Err(RequestHeadError::NonOriginFormTarget);
    }
    let target =
        String::from_utf8(target.to_vec()).map_err(|_| RequestHeadError::NonOriginFormTarget)?;
    Ok((method, target))
}

/// Response head -> typed value. `request_was_head` selects the no-body rule of
/// the response.
pub(crate) fn parse_response_head(
    bytes: &[u8],
    request_was_head: bool,
    limits: &GatewayLimits,
) -> Result<ParsedResponseHead, ResponseHeadError> {
    validate_lines(bytes).map_err(ResponseHeadError::Line)?;
    if bytes.len() > limits.max_response_head_bytes().get() {
        return Err(ResponseHeadError::HeadTooLarge);
    }
    let lines = head_lines(bytes);
    let (status_line, field_lines) =
        lines.split_first().ok_or(ResponseHeadError::VersionNotAccepted)?;
    let (version, status) = parse_status_line(status_line)?;
    if status == StatusCode::SWITCHING_PROTOCOLS {
        return Err(ResponseHeadError::Switching101);
    }
    let headers = parse_fields(field_lines, limits.max_response_headers().get()).map_err(
        |error| match error {
            FieldsError::TooMany => ResponseHeadError::TooManyHeaders,
            FieldsError::Malformed => {
                ResponseHeadError::Line(LineViolation::OctetOutsideFieldValueRange)
            }
        },
    )?;
    if status.is_informational() {
        return Ok(ParsedResponseHead::Interim { status, forward: status == StatusCode::CONTINUE });
    }
    let framing = derive_framing(&headers, FramingRole::FinalResponse { request_was_head, status })
        .map_err(ResponseHeadError::Framing)?;
    let explicit_length_needed = version == ResponseVersion::Http10
        && matches!(framing, BodyFraming::UntilClose | BodyFraming::Chunked);
    if explicit_length_needed {
        return Err(ResponseHeadError::VersionNotAccepted);
    }
    if let BodyFraming::Length(declared) = framing
        && declared > limits.max_response_body_bytes().get()
    {
        return Err(ResponseHeadError::DeclaredLengthAboveCeiling);
    }
    Ok(ParsedResponseHead::Final(ParsedFinalResponseHead { status, headers, framing }))
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
    status: StatusCode,
    headers: HeaderMap,
    framing: BodyFraming,
}

impl ParsedFinalResponseHead {
    pub(crate) const fn framing(&self) -> BodyFraming {
        self.framing
    }
    pub(crate) const fn status(&self) -> StatusCode {
        self.status
    }
    /// The ORIGINAL header map, nothing removed.
    pub(crate) fn into_headers(self) -> HeaderMap {
        self.headers
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseVersion {
    Http10,
    Http11,
}

fn parse_status_line(line: &[u8]) -> Result<(ResponseVersion, StatusCode), ResponseHeadError> {
    let mut parts = line.splitn(3, |octet| *octet == b' ');
    let (Some(version), Some(code)) = (parts.next(), parts.next()) else {
        return Err(ResponseHeadError::VersionNotAccepted);
    };
    let version = match version {
        v if v == HTTP_11 => ResponseVersion::Http11,
        v if v == HTTP_10 => ResponseVersion::Http10,
        _ => return Err(ResponseHeadError::VersionNotAccepted),
    };
    let status = StatusCode::from_bytes(code).map_err(|_| ResponseHeadError::VersionNotAccepted)?;
    if code.len() != 3 {
        return Err(ResponseHeadError::VersionNotAccepted);
    }
    Ok((version, status))
}

/// The one framing decision (RFC 9112 section 6.3), over the ORIGINAL headers:
/// nothing a `Connection` field nominates has been removed yet (S-PIG-81).
/// `role` selects the request rules (no framing = no body) or the response
/// rules.
pub(crate) fn derive_framing(
    headers: &http::HeaderMap,
    role: FramingRole,
) -> Result<BodyFraming, FramingFault> {
    if let FramingRole::FinalResponse { request_was_head, status } = role
        && (request_was_head
            || status.is_informational()
            || matches!(status.as_u16(), 204 | 205 | 304))
    {
        return Ok(BodyFraming::NoBody);
    }
    let lengths = headers.get_all(CONTENT_LENGTH);
    let codings = headers.get_all(TRANSFER_ENCODING);
    let has_length = lengths.iter().next().is_some();
    let has_coding = codings.iter().next().is_some();
    if has_length && has_coding {
        return Err(FramingFault::BothLengthAndCoding);
    }
    if has_length {
        return declared_length(headers);
    }
    if has_coding {
        return declared_coding(headers);
    }
    Ok(match role {
        FramingRole::Request => BodyFraming::NoBody,
        FramingRole::FinalResponse { .. } => BodyFraming::UntilClose,
    })
}

fn declared_length(headers: &HeaderMap) -> Result<BodyFraming, FramingFault> {
    let mut lines = headers.get_all(CONTENT_LENGTH).iter();
    let (Some(line), None) = (lines.next(), lines.next()) else {
        return Err(FramingFault::RepeatedContentLength);
    };
    let digits = line.as_bytes();
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Err(FramingFault::ContentLengthNotDecimal);
    }
    let text = std::str::from_utf8(digits).map_err(|_| FramingFault::ContentLengthNotDecimal)?;
    text.parse::<u64>().map(BodyFraming::Length).map_err(|_| FramingFault::ContentLengthOverflow)
}

fn declared_coding(headers: &HeaderMap) -> Result<BodyFraming, FramingFault> {
    let mut lines = headers.get_all(TRANSFER_ENCODING).iter();
    let (Some(line), None) = (lines.next(), lines.next()) else {
        return Err(FramingFault::RepeatedTransferEncoding);
    };
    let text = line.to_str().map_err(|_| FramingFault::FinalCodingNotChunked)?;
    let codings: Vec<String> =
        text.split(',').map(|coding| coding.trim().to_ascii_lowercase()).collect();
    let chunked = codings.iter().filter(|coding| *coding == "chunked").count();
    if chunked > 1 {
        return Err(FramingFault::RepeatedChunked);
    }
    if codings.last().map(String::as_str) != Some("chunked") {
        return Err(FramingFault::FinalCodingNotChunked);
    }
    if codings.len() > 1 {
        return Err(FramingFault::UnsupportedCoding);
    }
    Ok(BodyFraming::Chunked)
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
    validate_lines(line).map_err(ChunkLineError::Line)?;
    let body = line.strip_suffix(CRLF).ok_or(ChunkLineError::SizeLineMalformed)?;
    let size = body.split(|octet| *octet == b';').next().unwrap_or_default();
    let size = size.trim_ascii_end();
    if size.is_empty() {
        return Err(ChunkLineError::SizeLineMalformed);
    }
    if !size.iter().all(u8::is_ascii_hexdigit) {
        return Err(ChunkLineError::SizeNotHex);
    }
    let text = std::str::from_utf8(size).map_err(|_| ChunkLineError::SizeNotHex)?;
    u64::from_str_radix(text, 16).map_err(|_| ChunkLineError::SizeOverflow)
}

/// The chunk terminator is exactly CRLF.
pub(crate) fn check_chunk_terminator(line: &[u8]) -> Result<(), ChunkLineError> {
    validate_lines(line).map_err(ChunkLineError::Line)?;
    if line == CRLF { Ok(()) } else { Err(ChunkLineError::TerminatorNotCrlf) }
}

/// `true` for the empty line that ends the trailer; other lines are validated
/// and dropped.
pub(crate) fn is_trailer_end(line: &[u8]) -> Result<bool, ChunkLineError> {
    validate_lines(line).map_err(ChunkLineError::Line)?;
    if line == CRLF {
        return Ok(true);
    }
    line.strip_suffix(CRLF).map(|_| false).ok_or(ChunkLineError::TerminatorNotCrlf)
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

// ---------------------------------------------------------------------------
// Shared line and field helpers
// ---------------------------------------------------------------------------

/// The lines of a validated header block, the terminating empty line removed.
fn head_lines(bytes: &[u8]) -> Vec<&[u8]> {
    let block = bytes.strip_suffix(HEADER_BLOCK_END).or_else(|| bytes.strip_suffix(CRLF));
    let block = block.unwrap_or(bytes);
    let mut lines = Vec::new();
    let mut rest = block;
    while let Some(at) = rest.windows(CRLF.len()).position(|window| window == CRLF) {
        lines.push(&rest[..at]);
        rest = &rest[at + CRLF.len()..];
    }
    lines.push(rest);
    lines
}

enum FieldsError {
    TooMany,
    Malformed,
}

/// Parse field lines into the original map; more than `max` fields is refused
/// before any of them is interpreted.
fn parse_fields(lines: &[&[u8]], max: usize) -> Result<HeaderMap, FieldsError> {
    if lines.len() > max {
        return Err(FieldsError::TooMany);
    }
    let mut headers = HeaderMap::with_capacity(lines.len());
    for line in lines {
        let colon = line.iter().position(|octet| *octet == b':').ok_or(FieldsError::Malformed)?;
        let name = HeaderName::from_bytes(&line[..colon]).map_err(|_| FieldsError::Malformed)?;
        let value = line[colon + 1..].trim_ascii();
        let value = HeaderValue::from_bytes(value).map_err(|_| FieldsError::Malformed)?;
        headers.append(name, value);
    }
    Ok(headers)
}

#[cfg(test)]
mod laws {
    #![allow(
        clippy::doc_markdown,
        clippy::too_many_lines,
        clippy::type_complexity,
        reason = "CONTRACT_SHAPE is required metadata; the tables are long finite sets"
    )]

    use std::net::Ipv4Addr;

    use http::header::{CONNECTION, CONTENT_LENGTH, HeaderName, HeaderValue, TRANSFER_ENCODING};
    use http::{HeaderMap, StatusCode};
    use proptest::prelude::*;

    use overdrive_core::public_ingress::PublicHostnameError;

    use super::*;
    use crate::runtime::proxy_headers::rewrite_request_headers;

    // -----------------------------------------------------------------
    // Wire builders and thin adapters over the pinned functions
    // -----------------------------------------------------------------

    const HOSTNAME: &str = "api.example.com";

    fn limits() -> GatewayLimits {
        GatewayLimits::first_slice()
    }

    fn sni() -> PublicHostname {
        PublicHostname::new(HOSTNAME).expect("valid SNI")
    }

    /// A header block: `start_line`, every `lines` entry, then the empty line.
    fn block(start_line: &str, lines: &[String]) -> Vec<u8> {
        let mut out = format!("{start_line}\r\n");
        for line in lines {
            out.push_str(line);
            out.push_str("\r\n");
        }
        out.push_str("\r\n");
        out.into_bytes()
    }

    fn field_lines(fields: &[(&str, &str)]) -> Vec<String> {
        fields.iter().map(|(name, value)| format!("{name}: {value}")).collect()
    }

    /// A request head with the canonical `Host` followed by `fields`.
    fn request_with(start_line: &str, fields: &[(&str, &str)]) -> Vec<u8> {
        let mut lines = vec![format!("Host: {HOSTNAME}")];
        lines.extend(field_lines(fields));
        block(start_line, &lines)
    }

    fn post_with(fields: &[(&str, &str)]) -> Vec<u8> {
        request_with("POST /a HTTP/1.1", fields)
    }

    fn response_with(status_line: &str, fields: &[(&str, &str)]) -> Vec<u8> {
        block(status_line, &field_lines(fields))
    }

    fn header_map(fields: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in fields {
            map.append(
                HeaderName::from_bytes(name.as_bytes()).expect("header name"),
                HeaderValue::from_str(value).expect("header value"),
            );
        }
        map
    }

    fn request_result(bytes: &[u8]) -> Result<BodyFraming, RequestHeadError> {
        parse_request_head(bytes, &sni(), &limits()).map(|head| head.framing())
    }

    fn request_error(bytes: &[u8]) -> Option<RequestHeadError> {
        request_result(bytes).err()
    }

    /// What a parsed response head reduces to for assertions.
    #[derive(Debug, PartialEq, Eq)]
    enum ResponseShape {
        Interim { status: u16, forward: bool },
        Final(BodyFraming),
    }

    fn response_result(
        bytes: &[u8],
        request_was_head: bool,
    ) -> Result<ResponseShape, ResponseHeadError> {
        parse_response_head(bytes, request_was_head, &limits()).map(|parsed| match parsed {
            ParsedResponseHead::Interim { status, forward } => {
                ResponseShape::Interim { status: status.as_u16(), forward }
            }
            ParsedResponseHead::Final(head) => ResponseShape::Final(head.framing()),
        })
    }

    fn final_response(status: u16) -> FramingRole {
        FramingRole::FinalResponse {
            request_was_head: false,
            status: StatusCode::from_u16(status).expect("status"),
        }
    }

    /// `NoBody` and `Length(0)` are the same framing (a zero-length body).
    fn is_empty_body(framing: &BodyFraming) -> bool {
        matches!(framing, BodyFraming::NoBody | BodyFraming::Length(0))
    }

    // -----------------------------------------------------------------
    // S-PIG-76: the one framing decision
    // -----------------------------------------------------------------

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// S-PIG-76 (request): exactly `Content-Length` octets for any decimal
        /// length that fits `u64`; chunked for `Transfer-Encoding: chunked` in
        /// any letter case; no body for a request with neither; a request is
        /// never close-delimited.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn a_request_body_is_delimited_by_its_length_or_chunking_or_is_absent(
            n in 1u64..=u64::MAX,
            coding in prop_oneof![Just("chunked"), Just("Chunked"), Just("CHUNKED")],
        ) {
            let by_length = derive_framing(
                &header_map(&[("Content-Length", n.to_string().as_str())]),
                FramingRole::Request,
            );
            prop_assert_eq!(by_length, Ok(BodyFraming::Length(n)));

            let chunked = derive_framing(
                &header_map(&[("Transfer-Encoding", coding)]),
                FramingRole::Request,
            );
            prop_assert_eq!(chunked, Ok(BodyFraming::Chunked));
        }

        /// S-PIG-76 (request): a request is never close-delimited, whatever the
        /// header combination: every accepted result is a request framing.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn no_request_header_combination_is_close_delimited(
            lengths in proptest::collection::vec("[0-9a-z+-]{1,8}", 0..3),
            codings in proptest::collection::vec(
                prop_oneof![Just("chunked"), Just("gzip"), Just("identity"), Just("chunked, gzip")],
                0..3,
            ),
        ) {
            let mut fields: Vec<(&str, &str)> = Vec::new();
            for length in &lengths {
                fields.push(("Content-Length", length.as_str()));
            }
            for coding in &codings {
                fields.push(("Transfer-Encoding", *coding));
            }
            let outcome = derive_framing(&header_map(&fields), FramingRole::Request);
            prop_assert_ne!(outcome, Ok(BodyFraming::UntilClose));
        }

        /// S-PIG-76 (response): none for `HEAD` responses and for `1xx`, `204`,
        /// `205` and `304`, even when the head declares a length or a coding;
        /// otherwise exactly the declared length, chunked, or close-delimited
        /// when the response declares neither (R-12).
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn a_response_body_follows_the_status_the_request_method_and_the_declared_framing(
            body_status in (200u16..=599).prop_filter("statuses that may carry a body", |s| {
                !matches!(*s, 204 | 205 | 304)
            }),
            no_body_status in prop_oneof![100u16..=199, Just(204u16), Just(205u16), Just(304u16)],
            n in 1u64..=1_000_000,
        ) {
            let length = n.to_string();
            let with_length = header_map(&[("Content-Length", length.as_str())]);
            let with_coding = header_map(&[("Transfer-Encoding", "chunked")]);
            let with_nothing = HeaderMap::new();

            // Statuses without a body ignore a declared length or coding.
            let status = StatusCode::from_u16(no_body_status).expect("status");
            for headers in [&with_length, &with_coding, &with_nothing] {
                let framing = derive_framing(
                    headers,
                    FramingRole::FinalResponse { request_was_head: false, status },
                );
                prop_assert_eq!(framing, Ok(BodyFraming::NoBody), "status {} has no body", no_body_status);
            }

            // A response to HEAD never has a body, whatever it declares.
            let status = StatusCode::from_u16(body_status).expect("status");
            for headers in [&with_length, &with_coding, &with_nothing] {
                let framing = derive_framing(
                    headers,
                    FramingRole::FinalResponse { request_was_head: true, status },
                );
                prop_assert_eq!(framing, Ok(BodyFraming::NoBody), "a HEAD response has no body");
            }

            // Otherwise the declared framing, and close-delimited when absent.
            let role = FramingRole::FinalResponse { request_was_head: false, status };
            prop_assert_eq!(derive_framing(&with_length, role), Ok(BodyFraming::Length(n)));
            prop_assert_eq!(derive_framing(&with_coding, role), Ok(BodyFraming::Chunked));
            prop_assert_eq!(derive_framing(&with_nothing, role), Ok(BodyFraming::UntilClose));
        }
    }

    /// S-PIG-76 (examples): a request with neither length nor coding has no
    /// body, a zero length is an empty body, and the framing is the same
    /// whether the rest of the head is a request or a response head.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn a_request_without_length_or_coding_has_no_body_and_a_zero_length_is_empty() {
        let none = derive_framing(&HeaderMap::new(), FramingRole::Request).expect("no framing");
        assert_eq!(none, BodyFraming::NoBody, "no length and no coding: no body");

        let zero = derive_framing(&header_map(&[("Content-Length", "0")]), FramingRole::Request)
            .expect("a zero length");
        assert!(is_empty_body(&zero), "a zero length is an empty body, got {zero:?}");

        assert_eq!(
            request_result(&post_with(&[("Content-Length", "5")])),
            Ok(BodyFraming::Length(5)),
            "the head parser reports the same framing the decision derives",
        );
        assert_eq!(request_result(&request_with("GET /a HTTP/1.1", &[])), Ok(BodyFraming::NoBody),);
        assert_eq!(
            response_result(&response_with("HTTP/1.1 200 OK", &[]), false),
            Ok(ResponseShape::Final(BodyFraming::UntilClose)),
            "a response without length or coding is close-delimited",
        );
    }

    // -----------------------------------------------------------------
    // S-PIG-77 / S-PIG-78: ambiguous framing
    // -----------------------------------------------------------------

    /// One ambiguous header set and the faults the strict rejection table lets
    /// it be reported as (the table pins the rejection, and for overlapping
    /// rows the fault is one of the rows' variants).
    struct Ambiguity {
        name: &'static str,
        fields: &'static [(&'static str, &'static str)],
        faults: &'static [FramingFault],
    }

    const AMBIGUITIES: &[Ambiguity] = &[
        Ambiguity {
            name: "both Content-Length and Transfer-Encoding",
            fields: &[("Content-Length", "5"), ("Transfer-Encoding", "chunked")],
            faults: &[FramingFault::BothLengthAndCoding],
        },
        Ambiguity {
            name: "repeated Content-Length with equal values",
            fields: &[("Content-Length", "5"), ("Content-Length", "5")],
            faults: &[FramingFault::RepeatedContentLength],
        },
        Ambiguity {
            name: "repeated Content-Length with different values",
            fields: &[("Content-Length", "5"), ("Content-Length", "6")],
            faults: &[FramingFault::RepeatedContentLength],
        },
        Ambiguity {
            name: "Content-Length list in one line",
            fields: &[("Content-Length", "5, 5")],
            faults: &[FramingFault::RepeatedContentLength, FramingFault::ContentLengthNotDecimal],
        },
        Ambiguity {
            name: "Content-Length with a letter",
            fields: &[("Content-Length", "12a")],
            faults: &[FramingFault::ContentLengthNotDecimal],
        },
        Ambiguity {
            name: "Content-Length with a plus sign",
            fields: &[("Content-Length", "+5")],
            faults: &[FramingFault::ContentLengthNotDecimal],
        },
        Ambiguity {
            name: "negative Content-Length",
            fields: &[("Content-Length", "-1")],
            faults: &[FramingFault::ContentLengthNotDecimal],
        },
        Ambiguity {
            name: "hexadecimal Content-Length",
            fields: &[("Content-Length", "0x10")],
            faults: &[FramingFault::ContentLengthNotDecimal],
        },
        Ambiguity {
            name: "Content-Length one above u64::MAX",
            fields: &[("Content-Length", "18446744073709551616")],
            faults: &[FramingFault::ContentLengthOverflow],
        },
        Ambiguity {
            name: "Content-Length far above u64::MAX",
            fields: &[("Content-Length", "99999999999999999999999999")],
            faults: &[FramingFault::ContentLengthOverflow],
        },
        Ambiguity {
            name: "repeated Transfer-Encoding lines",
            fields: &[("Transfer-Encoding", "chunked"), ("Transfer-Encoding", "chunked")],
            faults: &[FramingFault::RepeatedTransferEncoding],
        },
        Ambiguity {
            name: "chunked, chunked",
            fields: &[("Transfer-Encoding", "chunked, chunked")],
            faults: &[FramingFault::RepeatedChunked],
        },
        Ambiguity {
            name: "another coding before chunked",
            fields: &[("Transfer-Encoding", "gzip, chunked")],
            faults: &[FramingFault::UnsupportedCoding],
        },
        Ambiguity {
            name: "a final coding that is not chunked",
            fields: &[("Transfer-Encoding", "chunked, gzip")],
            faults: &[FramingFault::FinalCodingNotChunked, FramingFault::UnsupportedCoding],
        },
        Ambiguity {
            name: "gzip alone",
            fields: &[("Transfer-Encoding", "gzip")],
            faults: &[FramingFault::FinalCodingNotChunked, FramingFault::UnsupportedCoding],
        },
        Ambiguity {
            name: "identity alone",
            fields: &[("Transfer-Encoding", "identity")],
            faults: &[FramingFault::FinalCodingNotChunked, FramingFault::UnsupportedCoding],
        },
    ];

    /// S-PIG-77: every ambiguous framing of a request is refused, as the
    /// `Framing` rejection with status 400, before any upstream octet (the
    /// parser is the only code that ran); the framing decision refuses the
    /// same header map with the same fault. An `HTTP/1.0` request is a 505.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn ambiguous_request_framing_is_a_400_and_an_http10_request_is_a_505() {
        for case in AMBIGUITIES {
            let Some(error) = request_error(&post_with(case.fields)) else {
                panic!("{}: the ambiguous request was accepted", case.name);
            };
            let RequestHeadError::Framing(fault) = error.clone() else {
                panic!("{}: expected a framing rejection, got {error:?}", case.name);
            };
            assert!(case.faults.contains(&fault), "{}: unexpected fault {fault:?}", case.name);
            assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{}", case.name);
            assert_eq!(
                derive_framing(&header_map(case.fields), FramingRole::Request),
                Err(fault),
                "{}: the framing decision reports the fault the head parser reported",
                case.name,
            );
        }

        let http10 = request_with("GET /a HTTP/1.0", &[]);
        let error = request_error(&http10).expect("an HTTP/1.0 request is refused");
        assert_eq!(error, RequestHeadError::Http10);
        assert_eq!(error.status(), StatusCode::HTTP_VERSION_NOT_SUPPORTED);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// S-PIG-77: any decimal `Content-Length` one octet or more above
        /// `u64::MAX` overflows, and any value that is not decimal digits is
        /// not decimal; both are refused as 400 framing rejections, while any
        /// length up to the request body ceiling is accepted as given.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn content_length_values_are_classified_by_what_they_are(
            overflowing in "[1-9][0-9]{20,30}",
            not_decimal in "[0-9]{0,3}[a-zA-Z+-][0-9a-z]{0,3}",
            within in 1u64..=16 * 1024 * 1024,
        ) {
            prop_assert_eq!(
                derive_framing(&header_map(&[("Content-Length", overflowing.as_str())]), FramingRole::Request),
                Err(FramingFault::ContentLengthOverflow),
            );
            prop_assert_eq!(
                derive_framing(&header_map(&[("Content-Length", not_decimal.as_str())]), FramingRole::Request),
                Err(FramingFault::ContentLengthNotDecimal),
            );
            prop_assert_eq!(
                request_result(&post_with(&[("Content-Length", within.to_string().as_str())])),
                Ok(BodyFraming::Length(within)),
            );
            let refused = request_error(&post_with(&[("Content-Length", overflowing.as_str())]))
                .expect("an overflowing length is refused");
            prop_assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
        }
    }

    /// S-PIG-78: the same ambiguity on an origin response is a
    /// `ResponseHeadError::Framing` (a 502 before the public head); a status
    /// line that is not `HTTP/1.1` and `101` are refused; an `HTTP/1.0`
    /// response with an explicit length is accepted (R-12) and without one is
    /// not.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn ambiguous_origin_framing_is_refused_and_http10_needs_an_explicit_length() {
        for case in AMBIGUITIES {
            let bytes = response_with("HTTP/1.1 200 OK", case.fields);
            let Err(error) = response_result(&bytes, false) else {
                panic!("{}: the ambiguous response was accepted", case.name);
            };
            let ResponseHeadError::Framing(fault) = error.clone() else {
                panic!("{}: expected a framing rejection, got {error:?}", case.name);
            };
            assert!(case.faults.contains(&fault), "{}: unexpected fault {fault:?}", case.name);
            assert_eq!(
                derive_framing(&header_map(case.fields), final_response(200)),
                Err(fault),
                "{}: the framing decision reports the fault the head parser reported",
                case.name,
            );
        }

        assert_eq!(
            response_result(
                &response_with("HTTP/1.1 101 Switching Protocols", &[("Upgrade", "websocket")]),
                false,
            )
            .err(),
            Some(ResponseHeadError::Switching101),
        );
        assert_eq!(
            response_result(&response_with("HTTP/1.0 200 OK", &[("Content-Length", "4")]), false),
            Ok(ResponseShape::Final(BodyFraming::Length(4))),
            "an HTTP/1.0 response with an explicit length is accepted",
        );
        assert_eq!(
            response_result(&response_with("HTTP/1.0 200 OK", &[]), false).err(),
            Some(ResponseHeadError::VersionNotAccepted),
            "an HTTP/1.0 response without a length cannot be told from a truncated one",
        );
    }

    // -----------------------------------------------------------------
    // S-PIG-79: the byte-level line validator
    // -----------------------------------------------------------------

    /// One octet the validator refuses inside a field value, and the violation
    /// that names it.
    fn refused_octet(class: usize, which: u8) -> (u8, LineViolation) {
        match class {
            0 => (b'\r', LineViolation::BareCr),
            1 => (b'\n', LineViolation::BareLf),
            2 => (0, LineViolation::Nul),
            _ => {
                // Control octets other than HTAB, CR, LF and NUL, and DEL.
                const CONTROLS: [u8; 28] = [
                    1, 2, 3, 4, 5, 6, 7, 8, 11, 12, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
                    26, 27, 28, 29, 30, 127,
                ];
                (
                    CONTROLS[usize::from(which) % CONTROLS.len()],
                    LineViolation::OctetOutsideFieldValueRange,
                )
            }
        }
    }

    fn head_with_value(value: &[u8]) -> Vec<u8> {
        let mut bytes = b"GET /a HTTP/1.1\r\nHost: api.example.com\r\nX-A: ".to_vec();
        bytes.extend_from_slice(value);
        bytes.extend_from_slice(b"\r\n\r\n");
        bytes
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// S-PIG-79: a header block whose values are visible octets, spaces,
        /// tabs and obs-text is valid; inserting one bare CR, bare LF, NUL or
        /// other control octet anywhere in a value is refused with the
        /// violation that names that octet, before the head parser runs: the
        /// request and response parsers return the line violation (status 400
        /// for a request) even when the head is also wrong in another way.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn bare_control_octets_in_a_head_are_refused_before_the_parser_runs(
            value in proptest::collection::vec(
                prop_oneof![0x21u8..=0x7e, 0x80u8..=0xff],
                0..24,
            ),
            at in 0usize..24,
            class in 0usize..4,
            which in any::<u8>(),
        ) {
            prop_assert_eq!(validate_lines(&head_with_value(&value)), Ok(()));
            prop_assert_eq!(
                validate_lines(&head_with_value(b"two words\tand a tab")),
                Ok(()),
                "spaces and tabs inside a value are valid",
            );

            let (octet, violation) = refused_octet(class, which);
            let mut mutated = value.clone();
            mutated.insert(at.min(value.len()), octet);
            let bytes = head_with_value(&mutated);
            prop_assert_eq!(validate_lines(&bytes), Err(violation));

            let error = request_error(&bytes).expect("a head with a bare control octet is refused");
            prop_assert_eq!(&error, &RequestHeadError::Line(violation));
            prop_assert_eq!(error.status(), StatusCode::BAD_REQUEST);

            // The validator runs before any parser: an HTTP/1.0 request line
            // does not outrank the line violation.
            let mut other_fault = b"GET /a HTTP/1.0\r\nHost: api.example.com\r\nX-A: ".to_vec();
            other_fault.extend_from_slice(&mutated);
            other_fault.extend_from_slice(b"\r\n\r\n");
            prop_assert_eq!(
                request_error(&other_fault),
                Some(RequestHeadError::Line(violation)),
            );

            let mut response = b"HTTP/1.1 200 OK\r\nX-A: ".to_vec();
            response.extend_from_slice(&mutated);
            response.extend_from_slice(b"\r\n\r\n");
            prop_assert_eq!(
                response_result(&response, false).err(),
                Some(ResponseHeadError::Line(violation)),
            );
        }
    }

    /// S-PIG-79 (other callers and the fold): the same validator guards one
    /// chunk-size line, a chunk terminator and a trailer line; an obsolete line
    /// fold is refused in a header block.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn single_lines_and_the_obsolete_fold_are_validated_by_the_same_rules() {
        assert_eq!(validate_lines(b"1a\r\n"), Ok(()));
        assert_eq!(validate_lines(b"1a\n"), Err(LineViolation::BareLf));
        assert_eq!(validate_lines(b"1a\rx\r\n"), Err(LineViolation::BareCr));
        assert_eq!(validate_lines(b"1a\0\r\n"), Err(LineViolation::Nul));

        for fold in [
            b"GET /a HTTP/1.1\r\nHost: api.example.com\r\nX-A: one\r\n two\r\n\r\n".as_slice(),
            b"GET /a HTTP/1.1\r\nHost: api.example.com\r\nX-A: one\r\n\ttwo\r\n\r\n".as_slice(),
        ] {
            assert_eq!(validate_lines(fold), Err(LineViolation::ObsoleteLineFold));
            assert_eq!(
                request_error(fold),
                Some(RequestHeadError::Line(LineViolation::ObsoleteLineFold)),
            );
        }
        assert_eq!(
            response_result(b"HTTP/1.1 200 OK\r\nX-A: one\r\n two\r\n\r\n", false).err(),
            Some(ResponseHeadError::Line(LineViolation::ObsoleteLineFold)),
        );
    }

    // -----------------------------------------------------------------
    // S-PIG-80: chunk framing lines
    // -----------------------------------------------------------------

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// S-PIG-80: a chunk-size line parses to its hexadecimal value in either
        /// letter case for every `u64`, with or without chunk extensions
        /// (discarded); a size above `u64::MAX` overflows; a size that is not
        /// hexadecimal is refused.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn chunk_size_lines_parse_every_u64_and_refuse_overflow_and_non_hex(
            size in any::<u64>(),
            extension in prop_oneof![
                Just(String::new()),
                Just(";name=value".to_owned()),
                Just(";a;b=\"quoted\"".to_owned()),
            ],
            overflowing in "[1-9a-f][0-9a-f]{16,24}",
            not_hex in "[g-zG-Z][0-9a-z]{0,4}",
        ) {
            prop_assert_eq!(parse_chunk_size_line(format!("{size:x}{extension}\r\n").as_bytes()), Ok(size));
            prop_assert_eq!(parse_chunk_size_line(format!("{size:X}{extension}\r\n").as_bytes()), Ok(size));
            prop_assert_eq!(
                parse_chunk_size_line(format!("{overflowing}{extension}\r\n").as_bytes()),
                Err(ChunkLineError::SizeOverflow),
            );
            prop_assert_eq!(
                parse_chunk_size_line(format!("{not_hex}\r\n").as_bytes()),
                Err(ChunkLineError::SizeNotHex),
            );
        }
    }

    /// S-PIG-80 (examples): the maximum size, the empty and unterminated
    /// lines, a control octet in an extension, the chunk terminator and the
    /// trailer-end line.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn malformed_chunk_lines_the_terminator_and_the_trailer_end_follow_their_rules() {
        assert_eq!(parse_chunk_size_line(b"ffffffffffffffff\r\n"), Ok(u64::MAX));
        assert_eq!(parse_chunk_size_line(b"0\r\n"), Ok(0));
        assert!(
            matches!(
                parse_chunk_size_line(b"\r\n"),
                Err(ChunkLineError::SizeNotHex | ChunkLineError::SizeLineMalformed)
            ),
            "an empty size line is refused",
        );
        assert!(parse_chunk_size_line(b"1a").is_err(), "an unterminated line is refused");
        assert_eq!(
            parse_chunk_size_line(b"1a;x\ry\r\n"),
            Err(ChunkLineError::Line(LineViolation::BareCr)),
            "the validator runs before the size parser",
        );

        assert_eq!(check_chunk_terminator(b"\r\n"), Ok(()));
        assert_eq!(check_chunk_terminator(b"ab\r\n"), Err(ChunkLineError::TerminatorNotCrlf));
        assert!(check_chunk_terminator(b"\n").is_err(), "a bare LF is not a CRLF terminator");

        assert_eq!(is_trailer_end(b"\r\n"), Ok(true), "the empty line ends the trailer");
        assert_eq!(is_trailer_end(b"X-Sum: abc\r\n"), Ok(false), "a trailer line is dropped");
        assert_eq!(
            is_trailer_end(b"X-Sum: a\0c\r\n"),
            Err(ChunkLineError::Line(LineViolation::Nul)),
        );
    }

    // -----------------------------------------------------------------
    // S-PIG-81: framing is read from the original headers
    // -----------------------------------------------------------------

    /// S-PIG-81: a `Connection` field nominating `Content-Length` or
    /// `Transfer-Encoding` removes nothing before the framing decision (the
    /// decision and the head parser still see the declared framing), and the
    /// forwarded head then carries none of the client's framing headers: the
    /// gateway alone emits them.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn a_connection_nomination_removes_nothing_before_the_framing_decision() {
        let cases: [(&str, [(&str, &str); 2], BodyFraming); 4] = [
            (
                "Content-Length nominated",
                [("Connection", "Content-Length"), ("Content-Length", "5")],
                BodyFraming::Length(5),
            ),
            (
                "Transfer-Encoding nominated",
                [("Connection", "Transfer-Encoding"), ("Transfer-Encoding", "chunked")],
                BodyFraming::Chunked,
            ),
            (
                "both nominated through a list",
                [("Connection", "close, Content-Length, x-hop"), ("Content-Length", "7")],
                BodyFraming::Length(7),
            ),
            (
                "nominated in another letter case",
                [("Connection", "TRANSFER-ENCODING"), ("Transfer-Encoding", "Chunked")],
                BodyFraming::Chunked,
            ),
        ];
        for (name, fields, expected) in cases {
            assert_eq!(
                derive_framing(&header_map(&fields), FramingRole::Request),
                Ok(expected),
                "{name}: the decision reads the original headers",
            );
            assert_eq!(request_result(&post_with(&fields)), Ok(expected), "{name}: head parser");

            let authority = PublicHostname::new(HOSTNAME).expect("authority");
            let forwarded = rewrite_request_headers(
                header_map(&fields),
                Ipv4Addr::new(203, 0, 113, 9),
                &authority,
            );
            assert!(
                forwarded.get(CONTENT_LENGTH).is_none(),
                "{name}: the forwarded head carries no client Content-Length",
            );
            assert!(
                forwarded.get(TRANSFER_ENCODING).is_none(),
                "{name}: the forwarded head carries no client Transfer-Encoding",
            );
            assert!(forwarded.get(CONNECTION).is_none(), "{name}: the nomination is consumed");
        }
    }

    // -----------------------------------------------------------------
    // S-PIG-82: request head policy at its boundaries
    // -----------------------------------------------------------------

    /// S-PIG-82 (R-67): `RequestHeadError::status` is the Request-rules table. Line
    /// violations, framing faults, a non-origin-form target, `CONNECT` and every
    /// `Authority(_)` error except the SNI mismatch are 400; `Authority(SniMismatch)`
    /// is 421; a declared length above the ceiling is 413; an oversized head or too
    /// many fields is 431; a version other than HTTP/1.1 is 505. One row per
    /// `RouteAuthorityError` variant.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_request_head_error_maps_to_its_status() {
        let table: [(RequestHeadError, u16); 19] = [
            (RequestHeadError::Line(LineViolation::Nul), 400),
            (RequestHeadError::Connect, 400),
            (RequestHeadError::NonOriginFormTarget, 400),
            (RequestHeadError::Authority(RouteAuthorityError::MissingHost), 400),
            (RequestHeadError::Authority(RouteAuthorityError::DuplicateHost), 400),
            (RequestHeadError::Authority(RouteAuthorityError::NonAsciiHost), 400),
            (RequestHeadError::Authority(RouteAuthorityError::InvalidAuthority), 400),
            (RequestHeadError::Authority(RouteAuthorityError::UnsupportedPort { got: 8443 }), 400),
            (
                RequestHeadError::Authority(RouteAuthorityError::InvalidHostname(
                    PublicHostnameError::InvalidLabel,
                )),
                400,
            ),
            (RequestHeadError::Authority(RouteAuthorityError::SniMismatch), 421),
            (RequestHeadError::Framing(FramingFault::BothLengthAndCoding), 400),
            (RequestHeadError::Framing(FramingFault::ContentLengthOverflow), 400),
            (RequestHeadError::DeclaredLengthAboveCeiling, 413),
            (RequestHeadError::HeadTooLarge, 431),
            (RequestHeadError::TooManyHeaders, 431),
            (RequestHeadError::VersionNotHttp11, 505),
            (RequestHeadError::Http10, 505),
            (RequestHeadError::Line(LineViolation::BareLf), 400),
            (RequestHeadError::Framing(FramingFault::UnsupportedCoding), 400),
        ];
        for (error, status) in table {
            assert_eq!(error.status().as_u16(), status, "{error:?}");
        }
    }

    /// S-PIG-82 (target and Host, R-67): a non-origin-form target and `CONNECT` are
    /// refused; every Host row is the `Authority(<RouteAuthorityError>)` the one
    /// `normalize_route_authority` produces: missing, duplicated, invalid, with
    /// userinfo, with a port other than 443 and with a non-canonical port spelling
    /// are 400; an explicit `:443` and the canonical Host are accepted; a Host that
    /// differs from the SNI is a 421.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn target_and_host_rules_hold_at_their_boundaries() {
        let host = |value: &str| block("GET /a HTTP/1.1", &[format!("Host: {value}")]);
        let authority = |value: &str| match request_error(&host(value)) {
            Some(RequestHeadError::Authority(error)) => error,
            other => panic!("Host {value:?}: expected an Authority error, got {other:?}"),
        };

        assert_eq!(request_result(&request_with("GET /a HTTP/1.1", &[])), Ok(BodyFraming::NoBody));
        assert_eq!(request_result(&host("api.example.com:443")), Ok(BodyFraming::NoBody));

        assert_eq!(
            request_error(&request_with("GET http://api.example.com/a HTTP/1.1", &[])),
            Some(RequestHeadError::NonOriginFormTarget),
        );
        assert_eq!(
            request_error(&request_with("GET * HTTP/1.1", &[])),
            Some(RequestHeadError::NonOriginFormTarget),
        );
        let connect = request_error(&block(
            "CONNECT api.example.com:443 HTTP/1.1",
            &["Host: api.example.com:443".to_owned()],
        ))
        .expect("CONNECT is refused");
        assert_eq!(connect.status(), StatusCode::BAD_REQUEST);

        assert_eq!(
            request_error(&block("GET /a HTTP/1.1", &[])),
            Some(RequestHeadError::Authority(RouteAuthorityError::MissingHost)),
        );
        assert_eq!(
            request_error(&block(
                "GET /a HTTP/1.1",
                &["Host: api.example.com".to_owned(), "Host: api.example.com".to_owned()],
            )),
            Some(RequestHeadError::Authority(RouteAuthorityError::DuplicateHost)),
        );
        assert!(matches!(authority("bad host!"), RouteAuthorityError::InvalidHostname(_)));
        assert_eq!(authority("user@api.example.com"), RouteAuthorityError::InvalidAuthority);
        assert_eq!(
            authority("api.example.com:8443"),
            RouteAuthorityError::UnsupportedPort { got: 8443 },
        );
        for spelling in [":+443", ":0443", ":00443", ":", ":4a3", ":65536"] {
            assert_eq!(
                authority(&format!("api.example.com{spelling}")),
                RouteAuthorityError::InvalidAuthority,
                "{spelling:?}",
            );
        }

        let mismatch = request_error(&host("other.example.com")).expect("Host differs from SNI");
        assert_eq!(mismatch, RequestHeadError::Authority(RouteAuthorityError::SniMismatch));
        assert_eq!(mismatch.status(), StatusCode::MISDIRECTED_REQUEST);
    }

    /// S-PIG-82 (ceilings): a declared length at `max_request_body_bytes` is
    /// accepted and one above is refused (413); a head of exactly
    /// `max_request_head_bytes` is accepted and one octet more is refused
    /// (431); 100 fields are accepted and 101 refused (431); `Upgrade` is
    /// removed as hop-by-hop from the forwarded head; `Expect` is accepted
    /// with any value and changes no framing.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn request_ceilings_are_exact_and_expect_and_upgrade_are_handled_as_pinned() {
        let limits = limits();

        // Declared length: at the ceiling and one above.
        let ceiling = limits.max_request_body_bytes().get();
        assert_eq!(
            request_result(&post_with(&[("Content-Length", ceiling.to_string().as_str())])),
            Ok(BodyFraming::Length(ceiling)),
        );
        let above =
            request_error(&post_with(&[("Content-Length", (ceiling + 1).to_string().as_str())]))
                .expect("a length above the ceiling is refused");
        assert_eq!(above, RequestHeadError::DeclaredLengthAboveCeiling);
        assert_eq!(above.status(), StatusCode::PAYLOAD_TOO_LARGE);

        // Head size: exactly the ceiling, then one octet more.
        let padded = |total: usize| -> Vec<u8> {
            let empty = request_with("GET /a HTTP/1.1", &[("X-Pad", "")]).len();
            let pad = "a".repeat(total - empty);
            request_with("GET /a HTTP/1.1", &[("X-Pad", pad.as_str())])
        };
        let max_head = limits.max_request_head_bytes().get();
        assert_eq!(padded(max_head).len(), max_head);
        assert_eq!(request_result(&padded(max_head)), Ok(BodyFraming::NoBody));
        let too_large = request_error(&padded(max_head + 1)).expect("one octet over is refused");
        assert_eq!(too_large, RequestHeadError::HeadTooLarge);
        assert_eq!(too_large.status(), StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE);

        // Field count: Host plus extra fields, 100 then 101.
        let with_fields = |count: usize| -> Vec<u8> {
            let names: Vec<String> = (1..count).map(|i| format!("X-N{i}")).collect();
            let fields: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "v")).collect();
            request_with("GET /a HTTP/1.1", &fields)
        };
        let max_fields = limits.max_request_headers().get();
        assert_eq!(request_result(&with_fields(max_fields)), Ok(BodyFraming::NoBody));
        assert_eq!(
            request_error(&with_fields(max_fields + 1)),
            Some(RequestHeadError::TooManyHeaders),
        );

        // `Expect` is the origin's business: any value parses, framing intact.
        for value in ["100-continue", "something-else", ""] {
            assert_eq!(
                request_result(&post_with(&[("Content-Length", "3"), ("Expect", value)])),
                Ok(BodyFraming::Length(3)),
                "Expect: {value:?} is accepted and decides nothing",
            );
        }

        // `Upgrade` is hop-by-hop and does not reach the origin.
        let authority = PublicHostname::new(HOSTNAME).expect("authority");
        let forwarded = rewrite_request_headers(
            header_map(&[("Upgrade", "websocket"), ("X-Keep", "yes")]),
            Ipv4Addr::new(203, 0, 113, 9),
            &authority,
        );
        assert!(forwarded.get("upgrade").is_none(), "Upgrade is removed as hop-by-hop");
        assert!(forwarded.get("x-keep").is_some(), "end-to-end headers survive");
    }

    // -----------------------------------------------------------------
    // S-PIG-83 (U half): interim heads
    // -----------------------------------------------------------------

    /// S-PIG-83: an origin `100` is an interim head the gateway forwards; every
    /// other `1xx` except `101` is an interim head that is read and dropped;
    /// `101` is refused; a final head is final whatever interim heads preceded
    /// it.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn interim_heads_are_forwarded_only_for_100_and_never_for_101() {
        let interim = |status: u16, reason: &str| {
            response_with(&format!("HTTP/1.1 {status} {reason}"), &[("Link", "</s.css>")])
        };
        assert_eq!(
            response_result(&interim(100, "Continue"), false),
            Ok(ResponseShape::Interim { status: 100, forward: true }),
        );
        for (status, reason) in
            [(102u16, "Processing"), (103, "Early Hints"), (110, "Other"), (199, "Other")]
        {
            assert_eq!(
                response_result(&interim(status, reason), false),
                Ok(ResponseShape::Interim { status, forward: false }),
                "{status} is read and dropped",
            );
        }
        assert_eq!(
            response_result(&interim(101, "Switching Protocols"), false).err(),
            Some(ResponseHeadError::Switching101),
        );
        assert_eq!(
            response_result(&response_with("HTTP/1.1 200 OK", &[("Content-Length", "2")]), false),
            Ok(ResponseShape::Final(BodyFraming::Length(2))),
        );
        assert_eq!(
            response_result(&response_with("HTTP/1.1 204 No Content", &[]), false),
            Ok(ResponseShape::Final(BodyFraming::NoBody)),
        );
        assert_eq!(
            response_result(&response_with("HTTP/1.1 200 OK", &[("Content-Length", "9")]), true,),
            Ok(ResponseShape::Final(BodyFraming::NoBody)),
            "a response to HEAD has no body whatever it declares",
        );
    }
}
