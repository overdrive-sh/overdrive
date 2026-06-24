//! `wire` — the DNS wire codec for the dial-by-name responder (ADR-0072
//! DDN-3 / DDN-4 / DDN-8).
//!
//! `hickory-proto` IS the codec (DDN-3: do NOT hand-roll DNS). This module is
//! the anti-corruption boundary (DDN-4 / D-DBN-5): it is the ONLY place a
//! `hickory_proto` type is named. The pure domain core ([`NameAnswer`],
//! `answer_for`, [`MeshServiceName`]) stays hickory-free; the qtype
//! ([`RecordType`]) is the one hickory type that crosses into `answer_for`.
//!
//! Two halves:
//!
//! - [`decode`] — parse an inbound DNS query datagram into its
//!   `(MeshServiceName, RecordType)` question (the responder reads the name +
//!   qtype to dispatch to `answer_for`).
//! - [`encode`] — render a [`NameAnswer`] (plus the originating query name +
//!   qtype) into a DNS response datagram: an A-record answer for `Records`, a
//!   NOERROR/NODATA + SOA reply for `NoData`, and an NXDOMAIN + SOA reply for
//!   `NxDomain`. Both negative replies carry a synthetic SOA whose `MINIMUM`
//!   (RFC 2308 negative TTL) is `1` (DDN-8) and whose `SERIAL` is derived from
//!   an injected clock reading `T` — NEVER wall-clock (`development.md`
//!   § "Never call `SystemTime::now()` in core logic").
//!
//! # SERIAL granularity (ADR-0072 § Pinned signatures latitude)
//!
//! The SOA `SERIAL` is a deterministic function of the injected clock reading
//! `T` (a `Duration` since the UNIX epoch, as produced by `Clock::unix_now`).
//! DELIVER pins the granularity at **1 second**: `SERIAL = T.as_secs() as u32`.
//! `DnsResponder` (a later slice) reads `config.clock.unix_now()` and passes
//! the `Duration` in; the encoder takes the reading as a pure parameter, which
//! keeps it deterministic + DST-replayable and lets the S-DBN-WIRE-04 proptest
//! control `T` directly.

use std::str::FromStr;
use std::time::Duration;

use hickory_proto::op::{Message, MessageType, OpCode, ResponseCode};
use hickory_proto::rr::rdata::{A, SOA};
use hickory_proto::rr::{Name, RData, Record, RecordType};
use overdrive_core::id::{MeshServiceName, NameAnswer};
use thiserror::Error;

/// The SOA `MINIMUM` (RFC 2308 negative TTL) carried by every negative reply
/// (NODATA + NXDOMAIN). Pinned to 1 second (DDN-8) so a retrying dialer
/// re-resolves promptly once a backend reaches running-and-healthy.
pub const NEGATIVE_TTL_SECS: u32 = 1;

/// TTL (seconds) on a positive A-record answer. Short — a backend addr can
/// change as allocations cycle, so a dialer should not cache it long.
const A_RECORD_TTL_SECS: u32 = 1;

/// The SOA `refresh`/`retry`/`expire` timers. These are not load-bearing for a
/// headless single-source name layer (no secondary zone transfer); fixed to
/// sane non-zero values so the record is well-formed. The negative-caching
/// behaviour rides on `MINIMUM` (DDN-8), not these.
const SOA_REFRESH_SECS: i32 = 1;
const SOA_RETRY_SECS: i32 = 1;
const SOA_EXPIRE_SECS: i32 = 1;

/// The decoded question of an inbound dial-by-name query: the dialed
/// [`MeshServiceName`] and the queried [`RecordType`] (`A` / `AAAA` in v1).
///
/// `RecordType` is the one `hickory_proto` type that crosses the ACL boundary
/// into `answer_for` (DDN-4); `MeshServiceName` is the hickory-free domain
/// name the responder matches against its index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedQuery {
    /// The dialed mesh service name (parsed from the query's first question).
    pub name: MeshServiceName,
    /// The queried record type (`A` / `AAAA`).
    pub qtype: RecordType,
}

/// Typed decode errors. A malformed datagram, a question whose name is not a
/// valid `<job>.svc.overdrive.local`, or an empty question section each map to
/// a distinct variant (never a catch-all `Internal(String)`).
#[derive(Debug, Error)]
pub enum WireError {
    /// The datagram is not a parseable DNS message.
    #[error("malformed DNS datagram: {source}")]
    Malformed {
        /// The underlying hickory-proto decode error.
        source: hickory_proto::serialize::binary::DecodeError,
    },
    /// The message carried no question section.
    #[error("DNS query carried no question")]
    NoQuestion,
    /// The question name is not a valid `<job>.svc.overdrive.local`.
    #[error("query name is not a valid mesh service name: {name}")]
    NotMeshName {
        /// The offending name as decoded from the wire.
        name: String,
    },
}

/// Decode an inbound DNS query datagram into its `(MeshServiceName,
/// RecordType)` question.
///
/// Reads the FIRST question of the message: its name (which must parse as a
/// `<job>.svc.overdrive.local` via [`MeshServiceName::new`]) and its record
/// type. An empty question section, an unparseable datagram, or a non-mesh
/// name each return the matching [`WireError`] variant.
pub fn decode(datagram: &[u8]) -> Result<DecodedQuery, WireError> {
    let message = Message::from_vec(datagram).map_err(|source| WireError::Malformed { source })?;
    let question = message.queries.first().ok_or(WireError::NoQuestion)?;
    let wire_name = question.name().to_string();
    // `Name::to_string()` renders the FQDN with a trailing dot
    // (`server.svc.overdrive.local.`); strip it before handing to the
    // `MeshServiceName` grammar, which expects the dot-less suffix form.
    let canonical = wire_name.strip_suffix('.').unwrap_or(&wire_name);
    let name =
        MeshServiceName::new(canonical).map_err(|_| WireError::NotMeshName { name: wire_name })?;
    Ok(DecodedQuery { name, qtype: question.query_type() })
}

/// Encode a [`NameAnswer`] into a DNS response datagram for the query
/// `(name, qtype)`.
///
/// - [`NameAnswer::Records`] → `ResponseCode::NoError`, one `A` record per addr
///   in the answer section, `ANCOUNT == addrs.len()`.
/// - [`NameAnswer::NoData`] → `ResponseCode::NoError`, `ANCOUNT == 0`, exactly
///   one SOA in the AUTHORITY section with `MINIMUM == 1` (DDN-8).
/// - [`NameAnswer::NxDomain`] → `ResponseCode::NXDomain`, `ANCOUNT == 0`,
///   exactly one SOA in AUTHORITY with `MINIMUM == 1`.
///
/// `serial_reading` is the injected clock reading `T` (a `Duration` since the
/// UNIX epoch); the SOA `SERIAL` is `T.as_secs() as u32` (1-second
/// granularity). The encoder NEVER reads wall-clock — `T` is always a
/// parameter (DDN-8; `development.md` § "Never call `SystemTime::now()`").
pub fn encode(
    name: &MeshServiceName,
    qtype: RecordType,
    answer: &NameAnswer,
    serial_reading: Duration,
) -> Vec<u8> {
    // The query's FQDN owner name (`<job>.svc.overdrive.local`). `Display` on
    // `MeshServiceName` renders the full grammar; hickory's `Name` parser
    // accepts it (it appends the implicit root label).
    let owner = parse_name(&name.to_string());

    // A response carries the question echoed back (RFC 1035 § 4.1.1): build a
    // Response message, echo the query, and fill the matching section.
    let mut message = Message::new(0, MessageType::Response, OpCode::Query);
    message.add_query(hickory_proto::op::Query::query(owner.clone(), qtype));

    match answer {
        NameAnswer::Records(addrs) => {
            message.metadata.response_code = ResponseCode::NoError;
            for addr in addrs {
                let rdata = RData::A(A::from(*addr.ip()));
                message.add_answer(Record::from_rdata(owner.clone(), A_RECORD_TTL_SECS, rdata));
            }
        }
        NameAnswer::NoData => {
            // The name IS resolvable (NOERROR) but has no record of the queried
            // type; RFC 2308 negative caching rides on the AUTHORITY SOA.
            message.metadata.response_code = ResponseCode::NoError;
            message.add_authority(build_soa_record(&owner, serial_reading));
        }
        NameAnswer::NxDomain => {
            message.metadata.response_code = ResponseCode::NXDomain;
            message.add_authority(build_soa_record(&owner, serial_reading));
        }
    }

    // `to_vec` only fails on a malformed message we constructed ourselves
    // (e.g. an un-encodable name) — a logic error here, not a runtime
    // condition, so an unreachable on the `Err` arm communicates the invariant.
    message
        .to_vec()
        .unwrap_or_else(|err| unreachable!("self-constructed DNS Message must encode: {err}"))
}

/// Derive the SOA `SERIAL` from the injected clock reading at 1-second
/// granularity (the pinned mapping; ADR-0072 § Pinned signatures latitude).
///
/// The DNS SOA `SERIAL` is a `u32` (RFC 1035) whose comparison is mod-2^32
/// serial-number arithmetic (RFC 1982), so truncating the whole-seconds
/// reading into a `u32` is the DEFINED behaviour, not a lossy bug — a
/// reading past 2^32 seconds wraps exactly as the protocol intends. This is
/// the one place the truncation is correct-by-spec; the `expect` documents it
/// (an `as u32` here is the contract the S-DBN-WIRE-04 proptest pins).
#[expect(
    clippy::cast_possible_truncation,
    reason = "SOA SERIAL is u32 with RFC 1982 mod-2^32 arithmetic; wrap is defined behaviour"
)]
fn serial_from_reading(serial_reading: Duration) -> u32 {
    serial_reading.as_secs() as u32
}

/// Build the synthetic AUTHORITY SOA record carried by every negative reply
/// (NODATA + NXDOMAIN). `MINIMUM` is pinned to [`NEGATIVE_TTL_SECS`] (`1`,
/// DDN-8); `SERIAL` is [`serial_from_reading`] over the injected clock reading
/// (1-second granularity) — NEVER wall-clock.
fn build_soa_record(owner: &Name, serial_reading: Duration) -> Record {
    let serial = serial_from_reading(serial_reading);
    let soa = SOA::new(
        owner.clone(), // MNAME — the headless name is its own primary
        parse_name("hostmaster.svc.overdrive.local"), // RNAME — synthetic admin
        serial,
        SOA_REFRESH_SECS,
        SOA_RETRY_SECS,
        SOA_EXPIRE_SECS,
        NEGATIVE_TTL_SECS,
    );
    Record::from_rdata(owner.clone(), NEGATIVE_TTL_SECS, RData::SOA(soa))
}

/// Parse a DNS name from its canonical string form. The inputs here are
/// always either a validated [`MeshServiceName`] rendering or a fixed
/// in-tree constant, so a parse failure is a logic error (unreachable),
/// not a runtime condition.
fn parse_name(rendered: &str) -> Name {
    Name::from_str(rendered)
        .unwrap_or_else(|err| unreachable!("valid mesh name must parse as a DNS Name: {err}"))
}
