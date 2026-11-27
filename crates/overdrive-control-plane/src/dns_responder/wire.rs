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
//! - [`decode`] — parse an inbound datagram into its single IN-class question
//!   (id, name, qtype) and classify the name once ([`classify`]).
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

use std::net::Ipv4Addr;
use std::str::FromStr;
use std::time::Duration;

use hickory_proto::op::{Edns, Message, MessageType, OpCode, ResponseCode};
use hickory_proto::rr::rdata::opt::EdnsOption;
use hickory_proto::rr::rdata::svcb::{SVCB, SvcParamValue};
use hickory_proto::rr::rdata::{A, SOA};
use hickory_proto::rr::{DNSClass, Name, RData, Record, RecordType};
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

/// The decoded question name of an inbound query: its labels, ASCII-case-folded,
/// the root label implicit. Hickory-free (DDN-4); classification compares these
/// labels, never a rendered string, so a label that itself contains a `.`
/// (`a\.svc.overdrive.local`) cannot pass for two labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryName {
    labels: Vec<Vec<u8>>,
}

impl QueryName {
    fn from_hickory(name: &Name) -> Self {
        Self { labels: name.iter().map(<[u8]>::to_ascii_lowercase).collect() }
    }

    fn to_hickory(&self) -> Name {
        Name::from_labels(self.labels.iter().map(Vec::as_slice))
            .unwrap_or_else(|err| unreachable!("labels decoded from a Name re-encode: {err}"))
    }

    /// Whether the trailing labels equal `suffix` (labels, not text).
    fn ends_with_labels(&self, suffix: &[&str]) -> bool {
        self.labels.len() >= suffix.len()
            && self
                .labels
                .iter()
                .rev()
                .zip(suffix.iter().rev())
                .all(|(label, expected)| label.as_slice() == expected.as_bytes())
    }
}

/// How the node treats a query name (ADR-0357).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryClass {
    /// A `<workload>.svc.overdrive.local` mesh service name, answered locally.
    Mesh(MeshServiceName),
    /// In the `overdrive.local` zone (apex included) but not a mesh name:
    /// NXDOMAIN, never forwarded.
    PlatformUnknown,
    /// Outside the platform zone: eligible for forwarding.
    Forwardable,
}

/// The ONE name classifier. Compares the trailing labels (`overdrive`,
/// `local`), case-insensitively; in zone a name is `Mesh` iff it is exactly one
/// single-label workload under `svc.overdrive.local` that `MeshServiceName`
/// accepts, otherwise `PlatformUnknown`; out of zone it is `Forwardable`.
#[must_use]
pub fn classify(name: &QueryName) -> QueryClass {
    let zone_labels = zone_labels();
    if !name.ends_with_labels(&zone_labels) {
        return QueryClass::Forwardable;
    }
    mesh_service_name(name).map_or(QueryClass::PlatformUnknown, QueryClass::Mesh)
}

fn zone_labels() -> Vec<&'static str> {
    MeshServiceName::PLATFORM_ZONE.split('.').collect()
}

/// The mesh name of an in-zone `name`, when it is `<workload>.svc.<zone>` with a
/// single, dot-free workload label the grammar accepts.
fn mesh_service_name(name: &QueryName) -> Option<MeshServiceName> {
    let mut expected = vec!["svc"];
    expected.extend(zone_labels());
    if name.labels.len() != expected.len() + 1 || !name.ends_with_labels(&expected) {
        return None;
    }
    let workload = std::str::from_utf8(name.labels.first()?).ok()?;
    if workload.contains('.') {
        return None;
    }
    MeshServiceName::new(&format!("{workload}.{}", MeshServiceName::SUFFIX)).ok()
}

/// The decoded question of an inbound query: the id, the question name, the
/// queried [`RecordType`] and the name's [`QueryClass`].
///
/// `RecordType` is the one `hickory_proto` type that crosses the ACL boundary
/// into `answer_for` (DDN-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedQuery {
    /// The DNS message ID, echoed verbatim into the response per RFC 1035
    /// § 4.1.1. Stub resolvers (glibc, systemd-resolved) match a response to
    /// its outstanding query by this ID and discard a mismatched one, so the
    /// responder MUST echo it back through [`encode`].
    pub id: u16,
    /// The question name.
    pub name: QueryName,
    /// The queried record type.
    pub qtype: RecordType,
    /// How the node treats the name.
    pub class: QueryClass,
}

/// A datagram that is not one IN-class QUERY with one question. It carries no
/// reply: the responder drops it.
#[derive(Debug, Error)]
pub enum WireReject {
    /// The datagram is not a parseable DNS message.
    #[error("malformed DNS datagram: {source}")]
    Undecodable {
        /// The underlying hickory-proto decode error.
        source: hickory_proto::serialize::binary::DecodeError,
    },
    /// The QR bit is set: a response, not a query.
    #[error("DNS datagram is a response")]
    NotAQuery,
    /// The opcode is not QUERY.
    #[error("DNS datagram opcode is not QUERY")]
    UnsupportedOpcode,
    /// The question count is not exactly one.
    #[error("DNS query carries {count} questions, not one")]
    QuestionCount {
        /// The number of questions the message carried.
        count: usize,
    },
    /// The question class is not IN.
    #[error("DNS question class is not IN")]
    NotInClass,
}

/// Decode an inbound datagram into its single IN-class question.
///
/// # Errors
///
/// A [`WireReject`] for every datagram that is not one IN-class QUERY with one
/// question.
pub fn decode(datagram: &[u8]) -> Result<DecodedQuery, WireReject> {
    let message =
        Message::from_vec(datagram).map_err(|source| WireReject::Undecodable { source })?;
    if message.metadata.message_type != MessageType::Query {
        return Err(WireReject::NotAQuery);
    }
    if message.metadata.op_code != OpCode::Query {
        return Err(WireReject::UnsupportedOpcode);
    }
    let [question] = message.queries.as_slice() else {
        return Err(WireReject::QuestionCount { count: message.queries.len() });
    };
    if question.query_class() != DNSClass::IN {
        return Err(WireReject::NotInClass);
    }
    let name = QueryName::from_hickory(question.name());
    let class = classify(&name);
    Ok(DecodedQuery { id: message.metadata.id, name, qtype: question.query_type(), class })
}

/// The NXDOMAIN reply for an unknown platform-zone name: question echoed, the
/// zone SOA in AUTHORITY with `MINIMUM == 1`.
pub(super) fn encode_platform_nxdomain(
    id: u16,
    name: &QueryName,
    qtype: RecordType,
    serial_reading: Duration,
) -> Vec<u8> {
    let owner = name.to_hickory();
    let mut message = Message::new(id, MessageType::Response, OpCode::Query);
    message.add_query(hickory_proto::op::Query::query(owner.clone(), qtype));
    message.metadata.response_code = ResponseCode::NXDomain;
    message.add_authority(build_soa_record(&owner, serial_reading));
    message
        .to_vec()
        .unwrap_or_else(|err| unreachable!("self-constructed DNS Message must encode: {err}"))
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
/// `id` is the originating query's DNS message ID, echoed verbatim into the
/// response header per RFC 1035 § 4.1.1 — a stub resolver matches the response
/// to its outstanding query by this ID and discards a mismatched one, so it
/// MUST be carried through ([`decode`] surfaces it on [`DecodedQuery::id`]).
///
/// `serial_reading` is the injected clock reading `T` (a `Duration` since the
/// UNIX epoch); the SOA `SERIAL` is `T.as_secs() as u32` (1-second
/// granularity). The encoder NEVER reads wall-clock — `T` is always a
/// parameter (DDN-8; `development.md` § "Never call `SystemTime::now()`").
pub fn encode(
    id: u16,
    name: &MeshServiceName,
    qtype: RecordType,
    answer: &NameAnswer,
    serial_reading: Duration,
) -> Vec<u8> {
    // The query's FQDN owner name (`<workload>.svc.overdrive.local`). `Display` on
    // `MeshServiceName` renders the full grammar; hickory's `Name` parser
    // accepts it (it appends the implicit root label).
    let owner = parse_name(&name.to_string());

    // A response carries the question echoed back (RFC 1035 § 4.1.1): build a
    // Response message echoing the query's ID, echo the question, and fill the
    // matching section. The ID MUST match the request or a stub resolver
    // discards the response.
    let mut message = Message::new(id, MessageType::Response, OpCode::Query);
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

/// A SERVFAIL for `query`: id and question echoed, QR set, RD copied from the
/// query, RA set, no records. A query that does not decode still gets a
/// header-only SERVFAIL carrying the id from its first two bytes (0 if short).
pub(super) fn servfail_reply(query: &[u8]) -> Vec<u8> {
    let decoded = Message::from_vec(query);
    let id = decoded.as_ref().map_or_else(
        |_| query.first_chunk::<2>().map_or(0, |bytes| u16::from_be_bytes(*bytes)),
        |message| message.metadata.id,
    );
    let mut reply = Message::new(id, MessageType::Response, OpCode::Query);
    reply.metadata.response_code = ResponseCode::ServFail;
    reply.metadata.recursion_available = true;
    if let Ok(message) = decoded {
        reply.metadata.recursion_desired = message.metadata.recursion_desired;
        for question in message.queries {
            reply.add_query(question);
        }
    }
    reply
        .to_vec()
        .unwrap_or_else(|err| unreachable!("self-constructed SERVFAIL must encode: {err}"))
}

/// The UDP payload size a blocked reply's OPT advertises (the DNS Flag Day
/// 2020 value).
const BLOCKED_REPLY_UDP_PAYLOAD: u16 = 1232;

/// Extended DNS Error INFO-CODE 15, "Blocked" (RFC 8914), with empty EXTRA-TEXT.
const EDE_BLOCKED: [u8; 2] = [0, 15];

/// The first question's name of `message` bytes, for logging; empty when the
/// bytes do not decode or carry no question.
pub(super) fn question_name(message: &[u8]) -> String {
    Message::from_vec(message)
        .ok()
        .and_then(|decoded| decoded.queries.first().map(|q| q.name().to_string()))
        .unwrap_or_default()
}

/// The blocked reply for an upstream reply that carried an owned address:
/// answer, authority and additional sections emptied, the upstream's id,
/// rcode and question kept, AD and TC cleared. Only when `query` carried an
/// OPT record does the reply carry one (payload 1232, DO copied
/// from the query, one Extended DNS Error option, code 15 "Blocked").
pub(super) fn blocked_reply(upstream_reply: &[u8], query: &[u8]) -> Vec<u8> {
    let mut message = Message::from_vec(upstream_reply)
        .unwrap_or_else(|err| unreachable!("the reply was inspected before it was blocked: {err}"));
    message.answers.clear();
    message.authorities.clear();
    message.additionals.clear();
    message.metadata.authentic_data = false;
    message.metadata.truncation = false;
    message.edns = Message::from_vec(query)
        .ok()
        .and_then(|decoded| decoded.edns)
        .map(|query_edns| blocked_opt(query_edns.flags().dnssec_ok));
    message
        .to_vec()
        .unwrap_or_else(|err| unreachable!("self-constructed blocked reply must encode: {err}"))
}

fn blocked_opt(dnssec_ok: bool) -> Edns {
    let mut edns = Edns::new();
    edns.set_max_payload(BLOCKED_REPLY_UDP_PAYLOAD);
    edns.set_dnssec_ok(dnssec_ok);
    edns.options_mut().insert(EdnsOption::Unknown(15, EDE_BLOCKED.to_vec()));
    edns
}

/// Whether `reply` answers `query`: a response (QR set) with the same id and
/// the same questions (name compared case-insensitively, type and class equal).
/// Bytes that do not decode never match.
pub(super) fn reply_matches(query: &[u8], reply: &[u8]) -> bool {
    let (Ok(query), Ok(reply)) = (Message::from_vec(query), Message::from_vec(reply)) else {
        return false;
    };
    reply.metadata.message_type == MessageType::Response
        && reply.metadata.id == query.metadata.id
        && reply.queries.len() == query.queries.len()
        && reply.queries.iter().zip(&query.queries).all(|(got, sent)| {
            got.query_type() == sent.query_type()
                && got.query_class() == sent.query_class()
                && got.name().to_string().eq_ignore_ascii_case(&sent.name().to_string())
        })
}

/// What the forwarder needs to know about an upstream reply, with no
/// `hickory_proto` type in sight (DDN-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReplyFacts {
    /// Every IPv4 address in the answer section: A rdata and the `ipv4hint`
    /// of SVCB / HTTPS records, in wire order.
    pub(super) answer_addresses: Vec<Ipv4Addr>,
}

/// The upstream reply bytes are not a decodable DNS message.
#[derive(Debug, Error)]
#[error("upstream reply is not a decodable DNS message: {source}")]
pub(super) struct UndecodableReply {
    source: hickory_proto::serialize::binary::DecodeError,
}

/// Project an upstream reply into [`ReplyFacts`]. A reply that does not decode
/// is an error; the forwarder never relays bytes it could not inspect.
pub(super) fn inspect_reply(reply: &[u8]) -> Result<ReplyFacts, UndecodableReply> {
    let message = Message::from_vec(reply).map_err(|source| UndecodableReply { source })?;
    let mut answer_addresses = Vec::with_capacity(message.answers.len());
    for record in &message.answers {
        match &record.data {
            RData::A(a) => answer_addresses.push(a.0),
            RData::SVCB(svcb) => answer_addresses.extend(hint_addresses(svcb)),
            RData::HTTPS(https) => answer_addresses.extend(hint_addresses(&https.0)),
            _ => {}
        }
    }
    Ok(ReplyFacts { answer_addresses })
}

fn hint_addresses(svcb: &SVCB) -> impl Iterator<Item = Ipv4Addr> + '_ {
    svcb.svc_params
        .iter()
        .filter_map(|(_, value)| match value {
            SvcParamValue::Ipv4Hint(hint) => Some(hint.0.iter().map(|a| a.0)),
            _ => None,
        })
        .flatten()
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
