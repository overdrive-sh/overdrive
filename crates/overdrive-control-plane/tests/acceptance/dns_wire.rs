//! S-DBN-WIRE-01..04 — `dns_responder::wire` codec proptests (Tier 1,
//! default unit lane, in-process; ADR-0072 DDN-3/4/8).
//!
//! These are the mandatory PBT coverage of the `wire.rs` encoder seam (the
//! irreducibly-Tier-1 half of the DNS surface; the socket loop is Tier-3).
//! The driving port is `wire::encode` / `wire::decode` — each property feeds
//! a `NameAnswer` through `encode`, then decodes the bytes with
//! `hickory_proto`'s OWN `Message` parser (the symmetric/roundtrip property,
//! Hebert ch.3) and asserts on the decoded message's observable surface
//! (response code, answer/authority sections, SOA MINIMUM/SERIAL). Asserting
//! through hickory's decoder — not on `wire.rs`'s internal `Message` build —
//! is the port-to-port discipline: a name-compression / RDATA-layout bug the
//! spike's lenient `dig` path masked still fails the round-trip.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::str::FromStr;
use std::time::Duration;

use hickory_proto::op::{Message, ResponseCode};
use hickory_proto::rr::{RData, RecordType};
use overdrive_control_plane::dns_responder::wire;
use overdrive_core::id::{MeshServiceName, NameAnswer};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategies — domain-specific generators for the WIRE input space.
// ---------------------------------------------------------------------------

/// A valid `<job>` label: DNS-1123, starts + ends alphanumeric, single label
/// (no interior `.` — the v1 single-label contract), within `LABEL_MAX`.
/// Kept short (≤ 16) to keep generation cheap; the boundary is covered by the
/// `MeshServiceName` validation suite, not here.
fn arb_job_label() -> impl Strategy<Value = String> {
    "[a-z0-9]([a-z0-9-]{0,14}[a-z0-9])?"
        .prop_filter("no trailing/leading hyphen", |s| !s.starts_with('-') && !s.ends_with('-'))
}

/// A valid `MeshServiceName` from a generated `<job>` label.
fn arb_mesh_name() -> impl Strategy<Value = MeshServiceName> {
    arb_job_label().prop_map(|label| {
        let full = format!("{label}.{}", MeshServiceName::SUFFIX);
        MeshServiceName::new(&full).expect("generated label is a valid mesh service name")
    })
}

/// A non-empty set of distinct `SocketAddrV4` (1..=8 addrs). The encoder
/// answers one A record per addr; distinctness keeps the decoded-set equality
/// unambiguous.
fn arb_addr_set() -> impl Strategy<Value = Vec<SocketAddrV4>> {
    proptest::collection::hash_set(
        (any::<u32>(), any::<u16>())
            .prop_map(|(ip, port)| SocketAddrV4::new(Ipv4Addr::from(ip), port)),
        1..=8,
    )
    .prop_map(|set| set.into_iter().collect())
}

/// A clock reading `T` (a `Duration` since the UNIX epoch). Bounded to a
/// realistic-but-wide range; the SOA SERIAL = `T.as_secs() as u32`.
fn arb_clock_reading() -> impl Strategy<Value = Duration> {
    (0u64..=4_000_000_000u64).prop_map(Duration::from_secs)
}

/// A fixed non-zero DNS message ID used by the section-shape tests
/// (WIRE-01..04). Picking a non-zero constant means those tests also
/// implicitly assert the response echoes the request ID (RFC 1035 §4.1.1) —
/// a hard-coded ID-0 response would fail the `msg.id() == ECHO_ID` checks.
const ECHO_ID: u16 = 0x1234;

// ---------------------------------------------------------------------------
// S-DBN-WIRE-05 — The DNS message ID round-trips: `decode` surfaces the
// query's ID and `encode` echoes it back into the response (RFC 1035 §4.1.1).
// Real stub resolvers (glibc, systemd-resolved) match responses to outstanding
// queries by ID and DISCARD a mismatched (ID-0) response, so the codec MUST
// preserve the ID — there is no post-`encode` seam to set it (wire is the
// DDN-4 anti-corruption boundary that owns the `hickory_proto::Message`).
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn wire_05_message_id_is_echoed_through_decode_and_encode(
        name in arb_mesh_name(),
        addrs in arb_addr_set(),
        id in any::<u16>(),
        t in arb_clock_reading(),
    ) {
        // A query datagram carrying an arbitrary (possibly zero) message ID.
        let mut query = Message::new(id, hickory_proto::op::MessageType::Query, hickory_proto::op::OpCode::Query);
        let owner = hickory_proto::rr::Name::from_str(&name.to_string())
            .expect("valid mesh name parses as a DNS Name");
        query.add_query(hickory_proto::op::Query::query(owner, RecordType::A));
        let query_bytes = query.to_vec().expect("self-constructed query encodes");

        // `decode` surfaces the query's ID on the DecodedQuery.
        let decoded = wire::decode(&query_bytes).expect("a well-formed mesh query decodes");
        prop_assert_eq!(decoded.id, id, "decode must surface the query message ID");

        // `encode` echoes the ID back into the response datagram. The ID lives
        // on `metadata` (same field-access idiom the WIRE-01..04 tests use for
        // `response_code`); avoids importing the `UpdateMessage` trait.
        let response_bytes = wire::encode(
            decoded.id,
            &name,
            RecordType::A,
            &NameAnswer::Records(addrs),
            t,
        );
        let response = Message::from_vec(&response_bytes)
            .expect("encoder output decodes as a DNS Message");
        prop_assert_eq!(response.metadata.id, id, "response must echo the request ID (RFC 1035 §4.1.1)");
    }
}

// ---------------------------------------------------------------------------
// S-DBN-WIRE-01 — Answered records survive a deterministic encode→decode
// round-trip (Hebert symmetric property).
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn wire_01_records_round_trip_through_hickory(
        name in arb_mesh_name(),
        addrs in arb_addr_set(),
        t in arb_clock_reading(),
    ) {
        let bytes = wire::encode(ECHO_ID, &name, RecordType::A, &NameAnswer::Records(addrs.clone()), t);

        prop_assert_eq!(
            Message::from_vec(&bytes).expect("decodes").metadata.id,
            ECHO_ID,
            "encode must echo the request ID",
        );

        let msg = Message::from_vec(&bytes).expect("encoder output decodes as a DNS Message");

        prop_assert_eq!(msg.metadata.response_code, ResponseCode::NoError);
        prop_assert_eq!(
            msg.answers.len(),
            addrs.len(),
            "ANCOUNT must equal the number of answered addrs",
        );

        let decoded: std::collections::HashSet<Ipv4Addr> = msg
            .answers
            .iter()
            .filter_map(|r| match &r.data {
                RData::A(a) => Some(a.0),
                _ => None,
            })
            .collect();
        prop_assert_eq!(decoded.len(), addrs.len(), "exactly one A record per addr");

        let expected: std::collections::HashSet<Ipv4Addr> =
            addrs.iter().map(|s| *s.ip()).collect();
        prop_assert_eq!(decoded, expected, "decoded A-record set equals the input addr set");
    }
}

// ---------------------------------------------------------------------------
// S-DBN-WIRE-02 — AAAA on a live name encodes NODATA (NOERROR, ANCOUNT==0,
// one SOA in AUTHORITY, SOA MINIMUM == 1).
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn wire_02_aaaa_on_live_name_is_nodata_with_soa(
        name in arb_mesh_name(),
        t in arb_clock_reading(),
    ) {
        let bytes = wire::encode(ECHO_ID, &name, RecordType::AAAA, &NameAnswer::NoData, t);

        let msg = Message::from_vec(&bytes).expect("encoder output decodes as a DNS Message");

        prop_assert_eq!(msg.metadata.id, ECHO_ID, "encode must echo the request ID");
        prop_assert_eq!(msg.metadata.response_code, ResponseCode::NoError, "NODATA is NOERROR");
        prop_assert_eq!(msg.answers.len(), 0, "NODATA carries no answer records");

        let soas: Vec<_> = msg
            .authorities
            .iter()
            .filter_map(|r| match &r.data {
                RData::SOA(soa) => Some(soa),
                _ => None,
            })
            .collect();
        prop_assert_eq!(soas.len(), 1, "exactly one SOA in the AUTHORITY section");
        prop_assert_eq!(
            soas[0].minimum,
            1,
            "SOA MINIMUM (RFC 2308 negative TTL) must be 1 (DDN-8)",
        );
    }
}

// ---------------------------------------------------------------------------
// S-DBN-WIRE-03 — A name with no running-and-healthy backend encodes NXDOMAIN
// (NXDomain, ANCOUNT==0, one SOA in AUTHORITY, SOA MINIMUM == 1), for both A
// and AAAA queries.
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn wire_03_nxdomain_with_soa_for_a_and_aaaa(
        name in arb_mesh_name(),
        qtype in prop_oneof![Just(RecordType::A), Just(RecordType::AAAA)],
        t in arb_clock_reading(),
    ) {
        let bytes = wire::encode(ECHO_ID, &name, qtype, &NameAnswer::NxDomain, t);

        let msg = Message::from_vec(&bytes).expect("encoder output decodes as a DNS Message");

        prop_assert_eq!(msg.metadata.id, ECHO_ID, "encode must echo the request ID");
        prop_assert_eq!(msg.metadata.response_code, ResponseCode::NXDomain);
        prop_assert_eq!(msg.answers.len(), 0, "NXDOMAIN carries no answer records");

        let soas: Vec<_> = msg
            .authorities
            .iter()
            .filter_map(|r| match &r.data {
                RData::SOA(soa) => Some(soa),
                _ => None,
            })
            .collect();
        prop_assert_eq!(soas.len(), 1, "exactly one SOA in the AUTHORITY section");
        prop_assert_eq!(soas[0].minimum, 1, "SOA MINIMUM must be 1 (DDN-8)");
    }
}

// ---------------------------------------------------------------------------
// S-DBN-WIRE-04 — SOA SERIAL is a deterministic function of the injected clock
// reading T: same T → byte-identical SERIAL; distinct T (≥ 1s apart, past the
// 1-second SERIAL granularity) → distinct SERIAL. The encoder never reads
// wall-clock — T is always a parameter.
//
// The `@example` pins two T values ≥ 1s apart (per DISTILL reviewer
// suggestion #3) so the "distinct T → distinct SERIAL" clause is not
// vacuously satisfiable by a coarser-than-1s mapping.
// ---------------------------------------------------------------------------

/// Extract the single SOA SERIAL from a negative-answer encoding.
fn soa_serial(bytes: &[u8]) -> u32 {
    let msg = Message::from_vec(bytes).expect("encoder output decodes as a DNS Message");
    let soa = msg
        .authorities
        .iter()
        .find_map(|r| match &r.data {
            RData::SOA(soa) => Some(soa),
            _ => None,
        })
        .expect("a negative answer carries one SOA");
    soa.serial
}

proptest! {
    #[test]
    fn wire_04_soa_serial_is_deterministic_per_clock_reading(
        name in arb_mesh_name(),
        // Two readings ≥ 1s apart: base T1, and T2 = T1 + delta (delta ≥ 1s)
        // so they straddle the 1-second SERIAL granularity boundary.
        secs1 in 0u64..=2_000_000_000u64,
        delta_secs in 1u64..=1_000_000_000u64,
        negative in prop_oneof![Just(NameAnswer::NoData), Just(NameAnswer::NxDomain)],
    ) {
        // @example: pin two T values 1s apart (DISTILL reviewer suggestion #3) —
        // the minimal pair that exercises "distinct past the 1s granularity".
        let t1 = Duration::from_secs(secs1);
        let t2 = Duration::from_secs(secs1 + delta_secs);
        let qtype = RecordType::AAAA;

        // Same T → byte-identical SERIAL (and byte-identical SOA encoding).
        let a = wire::encode(ECHO_ID, &name, qtype, &negative, t1);
        let b = wire::encode(ECHO_ID, &name, qtype, &negative, t1);
        prop_assert_eq!(soa_serial(&a), soa_serial(&b), "same T → identical SERIAL");

        // Distinct T (≥ 1s apart) → distinct SERIAL (1s granularity).
        let c = wire::encode(ECHO_ID, &name, qtype, &negative, t2);
        prop_assert_ne!(
            soa_serial(&a),
            soa_serial(&c),
            "distinct T past the 1s granularity → distinct SERIAL",
        );

        // The SERIAL is exactly the whole-seconds reading (the pinned 1s
        // granularity mapping): SERIAL == T.as_secs() as u32. Both T values
        // are generated < 2^32 s, so the `try_from` is infallible here.
        let serial1 = u32::try_from(t1.as_secs()).expect("generated T1 < 2^32 seconds");
        let serial2 = u32::try_from(t2.as_secs()).expect("generated T2 < 2^32 seconds");
        prop_assert_eq!(soa_serial(&a), serial1, "SERIAL == T.as_secs()");
        prop_assert_eq!(soa_serial(&c), serial2, "SERIAL == T.as_secs()");
    }
}
