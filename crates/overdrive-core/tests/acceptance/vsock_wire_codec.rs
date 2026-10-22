//! P2 — guest-transport wire codec: flow request, control open, control
//! message, the errno newtype, READY's launch field (GH #295, vsock
//! attachment replacement; EVMOD data path).
//!
//! Scenarios: P2-S1 (request / control open / launch-token codec, all-zero
//! token refused; request kind `3` undefined under EVMOD-DGRAM; READY's
//! `launch=` field, DG-7) and P2-S2 (control messages `Paired`,
//! `Refused { errno }`, `TimedOut { errno }`, `Abort` — INB-1; no
//! end-of-direction control message — R2-END). Contract:
//! `docs/feature/netns-density-295/feature-delta.md` § "[REF] Wire contracts —
//! flow request, control message, datagram frame" (HOST-NEUTRAL: the host
//! side decodes the same bytes whatever guest sent them) and DG-7.
//!
//! Driving ports: `overdrive_core::guest_transport::wire` and
//! `overdrive_core::vm::beacon::BeaconMessage` (`FromStr` / `Display`) —
//! layer 1, pure: `proptest` full (Mandate 9). `decode` accepts only
//! canonical encodings, so two properties pin each codec from both sides:
//! every valid value round-trips, and every byte string `decode` accepts
//! re-encodes to itself. The golden bytes are the Rust half of the one golden
//! set every guest's implementation of the host-side protocol must match.
//!
//! Not here: the datagram-frame codec (DG-8, R5-51; `vsock_datagram_frame.rs`),
//! the forwarder's end record (EVMOD-REC: it never leaves the machine and is
//! decoded by the `overdrive-fwd` adapter) and the non-READY beacon messages
//! (`vm_beacon_roundtrip.rs`).

#![allow(
    clippy::missing_const_for_fn,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    reason = "test bodies unwrap fixtures they construct; the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::str::FromStr as _;
use std::time::Duration;

use overdrive_core::guest_transport::wire::{
    CONTROL_MESSAGE_LEN, CONTROL_OPEN_LEN, CONTROL_RECONNECT_BACKOFF, ControlMessage, ControlOpen,
    FLOW_PAIRING_DEADLINE, FLOW_REQUEST_DEADLINE, FLOW_REQUEST_LEN, FlowId, FlowKind, FlowOrigin,
    FlowRefusal, FlowRequest, FlowWireError, GUEST_DIAL_PAIRING_DEADLINE, GUEST_FLOW_MAX_PAIRS,
    GUEST_INBOUND_CONNECT_TIMEOUT, KernelErrno, WireField,
};
use overdrive_core::guest_transport::{
    GUEST_NODE_GATEWAY, LaunchToken, LaunchTokenError, guest_reserved_ports,
};
use overdrive_core::net::ListenPort;
use overdrive_core::vm::beacon::{BeaconMessage, BeaconParseError};
use overdrive_core::vm::config::VsockPort;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Vocabulary
// ---------------------------------------------------------------------------

const TOKEN_BYTES: [u8; 16] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
];
const TOKEN_HEX: &str = "0102030405060708090a0b0c0d0e0f10";

/// The pinned refusal-code table (feature-delta § Wire contracts, control
/// message refusal codes 1..=12). Code 11 carries the guest's errno.
fn refusal_table() -> [(FlowRefusal, u8); 12] {
    [
        (FlowRefusal::NotActive, 1),
        (FlowRefusal::Policy, 2),
        (FlowRefusal::IntakeUnavailable, 3),
        (FlowRefusal::Capacity, 4),
        (FlowRefusal::Malformed, 5),
        (FlowRefusal::Unsupported, 6),
        (FlowRefusal::Quiesced, 7),
        (FlowRefusal::MeshUnreachable, 8),
        (FlowRefusal::ResolveFailed, 9),
        (FlowRefusal::DestinationUnreachable, 10),
        (FlowRefusal::ApplicationRefused { errno: KernelErrno::ECONNREFUSED }, 11),
        (FlowRefusal::HostInternal, 12),
    ]
}

fn token() -> LaunchToken {
    LaunchToken::from_bytes(TOKEN_BYTES).expect("non-zero token")
}

fn listen_port(port: u16) -> ListenPort {
    ListenPort::new(port).expect("a non-zero listen port")
}

fn guest_flow(seq: u32) -> FlowId {
    FlowId::new(FlowOrigin::Guest, seq).expect("seq < 2^31")
}

fn host_flow(seq: u32) -> FlowId {
    FlowId::new(FlowOrigin::Host, seq).expect("seq < 2^31")
}

fn errno(raw: u16) -> KernelErrno {
    KernelErrno::new(raw).expect("a non-zero errno")
}

/// The flow request layout: magic `OF`, version 2, kind, BE flow id, address,
/// BE port, BE listen port, launch token.
fn request_bytes(kind: u8, flow: [u8; 4], addr: [u8; 4], port: u16, listen: u16) -> [u8; 32] {
    let mut bytes = [0u8; FLOW_REQUEST_LEN];
    bytes[0..2].copy_from_slice(b"OF");
    bytes[2] = 2;
    bytes[3] = kind;
    bytes[4..8].copy_from_slice(&flow);
    bytes[8..12].copy_from_slice(&addr);
    bytes[12..14].copy_from_slice(&port.to_be_bytes());
    bytes[14..16].copy_from_slice(&listen.to_be_bytes());
    bytes[16..32].copy_from_slice(&TOKEN_BYTES);
    bytes
}

/// The control message layout: type, detail (refusal code), BE errno, BE
/// flow id.
fn control_bytes(ty: u8, detail: u8, errno: u16, flow: u32) -> [u8; CONTROL_MESSAGE_LEN] {
    let mut bytes = [0u8; CONTROL_MESSAGE_LEN];
    bytes[0] = ty;
    bytes[1] = detail;
    bytes[2..4].copy_from_slice(&errno.to_be_bytes());
    bytes[4..8].copy_from_slice(&flow.to_be_bytes());
    bytes
}

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn a_token() -> impl Strategy<Value = LaunchToken> {
    any::<[u8; 16]>()
        .prop_filter_map("all zero is not a token", |bytes| LaunchToken::from_bytes(bytes).ok())
}

fn a_guest_flow() -> impl Strategy<Value = FlowId> {
    (0u32..(1 << 31)).prop_map(guest_flow)
}

fn a_host_flow() -> impl Strategy<Value = FlowId> {
    (0u32..(1 << 31)).prop_map(host_flow)
}

fn any_flow() -> impl Strategy<Value = FlowId> {
    prop_oneof![a_guest_flow(), a_host_flow()]
}

fn a_destination() -> impl Strategy<Value = SocketAddrV4> {
    (any::<[u8; 4]>(), 1u16..=u16::MAX)
        .prop_map(|(octets, port)| SocketAddrV4::new(Ipv4Addr::from(octets), port))
}

fn an_errno() -> impl Strategy<Value = KernelErrno> {
    (1u16..=u16::MAX).prop_map(errno)
}

/// A declared listen port. No spec declaring a port of the guest's
/// reservation (`guest_reserved_ports()`, 61,000..=62,024) deploys (D25), so
/// no valid dial targets one: the round-trip property ranges over the ports
/// a `TcpAccept` is written for.
fn a_declared_listen_port() -> impl Strategy<Value = ListenPort> {
    prop_oneof![1u16..61_000, 62_025u16..=u16::MAX].prop_map(listen_port)
}

fn a_flow_request() -> impl Strategy<Value = FlowRequest> {
    prop_oneof![
        (a_guest_flow(), a_destination(), a_token()).prop_map(|(flow, destination, launch)| {
            FlowRequest::TcpConnect { flow, destination, launch }
        }),
        (a_guest_flow(), a_destination(), a_token()).prop_map(|(flow, destination, launch)| {
            FlowRequest::DatagramStream { flow, destination, launch }
        }),
        (a_host_flow(), any::<[u8; 4]>(), a_declared_listen_port(), a_token()).prop_map(
            |(flow, peer, listen_port, launch)| FlowRequest::TcpAccept {
                flow,
                peer: Ipv4Addr::from(peer),
                listen_port,
                launch,
            }
        ),
    ]
}

fn a_refusal() -> impl Strategy<Value = FlowRefusal> {
    prop_oneof![
        proptest::sample::select(
            refusal_table()
                .into_iter()
                .map(|(refusal, _)| refusal)
                .filter(|refusal| !matches!(refusal, FlowRefusal::ApplicationRefused { .. }))
                .collect::<Vec<_>>()
        ),
        an_errno().prop_map(|errno| FlowRefusal::ApplicationRefused { errno }),
    ]
}

/// Every message a valid sender writes: `Paired` / `Refused` / `Abort` on any
/// flow; the replies only a guest sends to a host-opened `TcpAccept`
/// (`Refused { ApplicationRefused }`, `TimedOut`) on host-origin flows.
fn a_control_message() -> impl Strategy<Value = ControlMessage> {
    prop_oneof![
        any_flow().prop_map(|flow| ControlMessage::Paired { flow }),
        (any_flow(), a_refusal()).prop_filter_map(
            "ApplicationRefused answers a host-opened flow",
            |(flow, refusal)| {
                let host_only = matches!(refusal, FlowRefusal::ApplicationRefused { .. });
                (!host_only || flow.origin() == FlowOrigin::Host)
                    .then_some(ControlMessage::Refused { flow, refusal })
            }
        ),
        any_flow().prop_map(|flow| ControlMessage::Abort { flow }),
        (a_host_flow(), an_errno())
            .prop_map(|(flow, errno)| ControlMessage::TimedOut { flow, errno }),
    ]
}

// ===========================================================================
// P2-S1 — flow request and control open
// ===========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Every request kind round-trips; its kind, flow and launch token survive
    /// bit-exact (ATTR-TOKEN), and the flow id's origin matches the kind.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_flow_request_round_trips(req in a_flow_request()) {
        let bytes = req.encode();
        prop_assert_eq!(bytes.len(), FLOW_REQUEST_LEN);
        let decoded = FlowRequest::decode(&bytes).expect("a canonical encoding decodes");
        prop_assert_eq!(decoded, req);
        prop_assert_eq!(decoded.kind(), req.kind());
        prop_assert_eq!(decoded.flow(), req.flow());
        prop_assert_eq!(decoded.launch(), req.launch());
        let host_opened = matches!(req.kind(), FlowKind::TcpAccept);
        prop_assert_eq!(req.flow().origin() == FlowOrigin::Host, host_opened);
        // INB-1: a TcpAccept carries port 0 on the wire (the guest kernel
        // picks the source port).
        if host_opened {
            prop_assert_eq!(&bytes[12..14], &[0u8, 0]);
        }
    }

    /// `decode` accepts only canonical encodings: any 32 bytes it accepts
    /// re-encode to exactly those bytes.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_accepted_request_is_canonical(bytes in any::<[u8; FLOW_REQUEST_LEN]>()) {
        if let Ok(req) = FlowRequest::decode(&bytes) {
            prop_assert_eq!(req.encode(), bytes);
        }
    }

    /// A request near the canonical form (valid header, one byte mutated)
    /// either decodes to a value that re-encodes to the same bytes or is
    /// refused — the arbitrary-bytes property rarely reaches past the magic.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_mutated_request_is_canonical_or_refused(
        req in a_flow_request(),
        index in 2usize..FLOW_REQUEST_LEN,
        value in any::<u8>(),
    ) {
        let mut bytes = req.encode();
        bytes[index] = value;
        if let Ok(decoded) = FlowRequest::decode(&bytes) {
            prop_assert_eq!(decoded.encode(), bytes);
        }
    }

    /// `ControlOpen` round-trips carrying the guest's launch token, and any
    /// 24 bytes it accepts are canonical.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn control_open_round_trips_and_is_canonical(
        launch in a_token(),
        bytes in any::<[u8; CONTROL_OPEN_LEN]>(),
    ) {
        let open = ControlOpen { launch };
        let encoded = open.encode();
        prop_assert_eq!(encoded.len(), CONTROL_OPEN_LEN);
        prop_assert_eq!(ControlOpen::decode(&encoded).expect("decodes"), open);
        if let Ok(decoded) = ControlOpen::decode(&bytes) {
            prop_assert_eq!(decoded.encode(), bytes);
        }
    }
}

/// Golden bytes of each request kind — `1` TcpConnect, `2` DatagramStream
/// (the association's one connection, EVMOD-DGRAM), `4` TcpAccept — its
/// launch token included. A TcpAccept names the guest gateway as the peer,
/// port 0 and the declared port (INB-1, ADR-0174).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn golden_flow_requests_pin_every_kind() {
    let destination = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 7), 443);
    let guest_one = [0x00, 0x00, 0x00, 0x01];
    let cases = [
        (
            FlowRequest::TcpConnect { flow: guest_flow(1), destination, launch: token() },
            request_bytes(1, guest_one, [192, 0, 2, 7], 443, 0),
            FlowKind::TcpConnect,
        ),
        (
            FlowRequest::DatagramStream { flow: guest_flow(1), destination, launch: token() },
            request_bytes(2, guest_one, [192, 0, 2, 7], 443, 0),
            FlowKind::DatagramStream,
        ),
        (
            FlowRequest::TcpAccept {
                flow: host_flow(2),
                peer: GUEST_NODE_GATEWAY,
                listen_port: listen_port(8080),
                launch: token(),
            },
            request_bytes(4, [0x80, 0x00, 0x00, 0x02], [100, 95, 0, 1], 0, 8080),
            FlowKind::TcpAccept,
        ),
    ];
    for (req, golden, kind) in cases {
        assert_eq!(req.encode(), golden, "{kind:?} encodes to its golden bytes");
        assert_eq!(FlowRequest::decode(&golden), Ok(req), "{kind:?} golden bytes decode");
        assert_eq!(req.kind(), kind);
        assert_eq!(req.launch(), token());
    }
    assert_eq!(token().to_string(), TOKEN_HEX);
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn golden_control_open() {
    let mut golden = [0u8; CONTROL_OPEN_LEN];
    golden[0..2].copy_from_slice(b"OC");
    golden[2] = 1;
    golden[8..24].copy_from_slice(&TOKEN_BYTES);
    let open = ControlOpen { launch: token() };
    assert_eq!(open.encode(), golden);
    assert_eq!(ControlOpen::decode(&golden), Ok(open));
}

/// ATTR-TOKEN: the all-zero token is the invalid encoding in every record
/// that carries one.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn an_all_zero_launch_token_is_refused_everywhere() {
    assert_eq!(LaunchToken::from_bytes([0u8; 16]), Err(LaunchTokenError::Zero));
    assert_eq!(LaunchToken::from_str(&"0".repeat(32)), Err(LaunchTokenError::Zero));

    for kind in [1u8, 2] {
        let mut request = request_bytes(kind, [0, 0, 0, 1], [192, 0, 2, 7], 443, 0);
        request[16..32].fill(0);
        assert_eq!(
            FlowRequest::decode(&request),
            Err(FlowWireError::InvalidField { field: WireField::LaunchToken }),
            "kind {kind}"
        );
    }
    let mut accept = request_bytes(4, [0x80, 0, 0, 2], [100, 95, 0, 1], 0, 8080);
    accept[16..32].fill(0);
    assert_eq!(
        FlowRequest::decode(&accept),
        Err(FlowWireError::InvalidField { field: WireField::LaunchToken })
    );

    let mut open = ControlOpen { launch: token() }.encode();
    open[8..24].fill(0);
    assert_eq!(
        ControlOpen::decode(&open),
        Err(FlowWireError::InvalidField { field: WireField::LaunchToken })
    );
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_flow_id_carries_its_origin_in_the_top_bit() {
    let max = guest_flow((1 << 31) - 1);
    assert_eq!(max.origin(), FlowOrigin::Guest);
    assert_eq!(max.get(), (1 << 31) - 1);
    assert_eq!(
        FlowId::new(FlowOrigin::Guest, 1 << 31),
        Err(FlowWireError::InvalidField { field: WireField::FlowSeq })
    );
    assert_eq!(
        FlowId::new(FlowOrigin::Host, u32::MAX),
        Err(FlowWireError::InvalidField { field: WireField::FlowSeq })
    );
    assert_eq!(host_flow(1).origin(), FlowOrigin::Host);
    assert_eq!(host_flow(1).get(), guest_flow(1).get() | (1 << 31));
}

/// Every malformed request is refused with its typed cause (C6b).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn malformed_flow_requests_are_refused_with_their_cause() {
    let connect = || request_bytes(1, [0, 0, 0, 1], [192, 0, 2, 7], 443, 0);
    let accept = || request_bytes(4, [0x80, 0, 0, 2], [100, 95, 0, 1], 0, 8080);

    let mut bytes = connect();
    bytes[0] = b'X';
    assert_eq!(FlowRequest::decode(&bytes), Err(FlowWireError::BadMagic { observed: *b"XF" }));

    for version in [1u8, 3] {
        let mut bytes = connect();
        bytes[2] = version;
        assert_eq!(FlowRequest::decode(&bytes), Err(FlowWireError::UnsupportedVersion { version }));
    }

    // Kind 3 (the removed SEQPACKET record) is not defined (EVMOD-DGRAM).
    for kind in [0u8, 3, 5, 255] {
        let mut bytes = connect();
        bytes[3] = kind;
        assert_eq!(FlowRequest::decode(&bytes), Err(FlowWireError::UnknownKind { kind }));
    }

    // Kinds 1 and 2 must be guest-opened; kind 4 host-opened.
    for (kind, flow_kind) in [(1u8, FlowKind::TcpConnect), (2, FlowKind::DatagramStream)] {
        let mut bytes = request_bytes(kind, [0, 0, 0, 1], [192, 0, 2, 7], 443, 0);
        bytes[4] |= 0x80;
        assert_eq!(
            FlowRequest::decode(&bytes),
            Err(FlowWireError::OriginMismatch { kind: flow_kind })
        );
    }
    let mut bytes = accept();
    bytes[4] &= 0x7F;
    assert_eq!(
        FlowRequest::decode(&bytes),
        Err(FlowWireError::OriginMismatch { kind: FlowKind::TcpAccept })
    );

    // Kinds 1 and 2: the original destination port is never 0.
    for kind in [1u8, 2] {
        let bytes = request_bytes(kind, [0, 0, 0, 1], [192, 0, 2, 7], 0, 0);
        assert_eq!(
            FlowRequest::decode(&bytes),
            Err(FlowWireError::InvalidField { field: WireField::Port }),
            "kind {kind}"
        );
    }
    // Kind 4 (INB-1): the port is always 0 — the guest kernel picks it.
    let mut bytes = accept();
    bytes[12..14].copy_from_slice(&50_000u16.to_be_bytes());
    assert_eq!(
        FlowRequest::decode(&bytes),
        Err(FlowWireError::InvalidField { field: WireField::Port })
    );

    // Kind 4 carries a non-zero declared listen port; kinds 1 and 2 carry 0.
    let mut bytes = accept();
    bytes[14..16].fill(0);
    assert_eq!(
        FlowRequest::decode(&bytes),
        Err(FlowWireError::InvalidField { field: WireField::ListenPort })
    );
    let mut bytes = connect();
    bytes[14..16].copy_from_slice(&8080u16.to_be_bytes());
    assert_eq!(
        FlowRequest::decode(&bytes),
        Err(FlowWireError::InvalidField { field: WireField::ListenPort })
    );
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn malformed_control_opens_are_refused_with_their_cause() {
    let base = ControlOpen { launch: token() }.encode();

    let mut bytes = base;
    bytes[0] = b'Z';
    assert_eq!(ControlOpen::decode(&bytes), Err(FlowWireError::BadMagic { observed: *b"ZC" }));

    let mut bytes = base;
    bytes[2] = 2;
    assert_eq!(ControlOpen::decode(&bytes), Err(FlowWireError::UnsupportedVersion { version: 2 }));

    for reserved in 3..8 {
        let mut bytes = base;
        bytes[reserved] = 1;
        assert_eq!(
            ControlOpen::decode(&bytes),
            Err(FlowWireError::InvalidField { field: WireField::ControlOpenReserved }),
            "reserved byte {reserved}",
        );
    }
}

// ===========================================================================
// P2-S1 / DG-7 — READY carries the launch token
// ===========================================================================

/// The golden READY line carries the launch token's 32-character lowercase
/// hex form as its third field and round-trips.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn golden_ready_line_carries_the_launch_token() {
    let ready = BeaconMessage::Ready { pid: 1, port: VsockPort::new(1234), launch: token() };
    let golden = format!("READY pid=1 port=1234 launch={TOKEN_HEX}");
    assert_eq!(ready.to_string(), golden);
    assert_eq!(golden.parse::<BeaconMessage>(), Ok(ready));
}

/// DG-7: a READY line with two or four fields is refused
/// `FieldCount { expected: 3 }`.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn ready_without_or_with_an_extra_field_is_refused_field_count() {
    let without = "READY pid=1 port=1234";
    match without.parse::<BeaconMessage>() {
        Err(BeaconParseError::FieldCount { kind: "READY", expected: 3, actual: 2, raw }) => {
            assert_eq!(raw, without);
        }
        other => panic!("expected FieldCount {{ expected: 3, actual: 2 }}, got {other:?}"),
    }
    let extra = format!("READY pid=1 port=1234 launch={TOKEN_HEX} extra=1");
    match extra.parse::<BeaconMessage>() {
        Err(BeaconParseError::FieldCount { kind: "READY", expected: 3, actual: 4, raw }) => {
            assert_eq!(raw, extra);
        }
        other => panic!("expected FieldCount {{ expected: 3, actual: 4 }}, got {other:?}"),
    }
}

/// DG-7: a third field not shaped `launch=<v>` is `MissingKey { field:
/// "launch" }`, naming the token found.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn ready_whose_third_field_is_not_launch_is_refused_missing_key() {
    for third in [format!("token={TOKEN_HEX}"), TOKEN_HEX.to_owned(), format!("launch:{TOKEN_HEX}")]
    {
        let line = format!("READY pid=1 port=1234 {third}");
        match line.parse::<BeaconMessage>() {
            Err(BeaconParseError::MissingKey { kind: "READY", field: "launch", found, .. }) => {
                assert_eq!(found, third);
            }
            other => panic!("expected MissingKey {{ field: launch }} for {third:?}, got {other:?}"),
        }
    }
}

/// DG-7: `launch=<v>` with `v` failing `LaunchToken::from_str` is
/// `InvalidLaunchToken`, carrying the token's own refusal — upper case, wrong
/// length and non-hex are `Unparseable`; all zero is `Zero`.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn ready_with_an_invalid_launch_token_is_refused_invalid_launch_token() {
    let cases = [
        (TOKEN_HEX.to_uppercase(), LaunchTokenError::Unparseable),
        ("a".repeat(31), LaunchTokenError::Unparseable),
        ("a".repeat(33), LaunchTokenError::Unparseable),
        ("g".repeat(32), LaunchTokenError::Unparseable),
        (String::new(), LaunchTokenError::Unparseable),
        ("0".repeat(32), LaunchTokenError::Zero),
    ];
    for (value, expected) in cases {
        let line = format!("READY pid=1 port=1234 launch={value}");
        match line.parse::<BeaconMessage>() {
            Err(BeaconParseError::InvalidLaunchToken { kind: "READY", raw, source }) => {
                assert_eq!(source, expected, "{value:?}");
                assert_eq!(raw, line);
            }
            other => panic!("expected InvalidLaunchToken for {value:?}, got {other:?}"),
        }
    }
}

// ===========================================================================
// P2-S2 — control messages: Paired, Refused { errno }, Abort, TimedOut
// ===========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Every control message a valid sender writes round-trips.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_control_message_round_trips(msg in a_control_message()) {
        let bytes = msg.encode();
        prop_assert_eq!(bytes.len(), CONTROL_MESSAGE_LEN);
        prop_assert_eq!(ControlMessage::decode(&bytes), Ok(msg));
    }

    /// Any 8 bytes `decode` accepts re-encode to themselves, and decode only
    /// to a pinned message type: no end-of-direction control message exists
    /// (R2-END).
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_accepted_control_message_is_canonical(bytes in any::<[u8; CONTROL_MESSAGE_LEN]>()) {
        if let Ok(msg) = ControlMessage::decode(&bytes) {
            prop_assert_eq!(msg.encode(), bytes);
            prop_assert!(
                matches!(
                    msg,
                    ControlMessage::Paired { .. }
                        | ControlMessage::Refused { .. }
                        | ControlMessage::Abort { .. }
                        | ControlMessage::TimedOut { .. }
                ),
                "decoded {msg:?}, which is not a pinned control message type"
            );
        }
    }
}

/// Golden bytes of each message type and each refusal code; code 11 and
/// `TimedOut` carry the guest's errno in bytes 2..4 (INB-1).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn golden_control_messages_pin_every_type_and_refusal_code() {
    let flow = guest_flow(3);
    let accept = host_flow(3);
    let cases = [
        (ControlMessage::Paired { flow }, control_bytes(1, 0, 0, 3)),
        (ControlMessage::Abort { flow }, control_bytes(3, 0, 0, 3)),
        (ControlMessage::Abort { flow: accept }, control_bytes(3, 0, 0, 0x8000_0003)),
        (
            ControlMessage::TimedOut { flow: accept, errno: KernelErrno::ETIMEDOUT },
            control_bytes(4, 0, 110, 0x8000_0003),
        ),
    ];
    for (msg, golden) in cases {
        assert_eq!(msg.encode(), golden, "{msg:?}");
        assert_eq!(ControlMessage::decode(&golden), Ok(msg));
    }
    for (refusal, code) in refusal_table() {
        assert_eq!(refusal.as_u8(), code, "{refusal:?}");
        let (target, flow_bytes, errno_field) = match refusal {
            FlowRefusal::ApplicationRefused { errno } => (accept, 0x8000_0003, errno.get()),
            _ => (flow, 3, 0),
        };
        let msg = ControlMessage::Refused { flow: target, refusal };
        let golden = control_bytes(2, code, errno_field, flow_bytes);
        assert_eq!(msg.encode(), golden, "Refused({refusal:?})");
        assert_eq!(ControlMessage::decode(&golden), Ok(msg));
    }
    assert_eq!(
        control_bytes(2, 11, 111, 0x8000_0003),
        ControlMessage::Refused {
            flow: accept,
            refusal: FlowRefusal::ApplicationRefused { errno: KernelErrno::ECONNREFUSED },
        }
        .encode(),
        "nothing listening: ECONNREFUSED (111)"
    );
}

/// Every malformed control message is refused with its typed cause (C6b).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn malformed_control_messages_are_refused_with_their_cause() {
    // Types 5 and 6 (the removed end-of-direction messages) are undefined.
    for ty in [0u8, 5, 6, 7, 255] {
        assert_eq!(
            ControlMessage::decode(&control_bytes(ty, 0, 0, 1)),
            Err(FlowWireError::UnknownControlType { ty })
        );
    }
    for code in [0u8, 13, 255] {
        assert_eq!(
            ControlMessage::decode(&control_bytes(2, code, 0, 1)),
            Err(FlowWireError::UnknownRefusal { code })
        );
    }
    // Code 11 and TimedOut carry a non-zero errno.
    assert_eq!(
        ControlMessage::decode(&control_bytes(2, 11, 0, 0x8000_0001)),
        Err(FlowWireError::InvalidField { field: WireField::ControlErrno })
    );
    assert_eq!(
        ControlMessage::decode(&control_bytes(4, 0, 0, 0x8000_0001)),
        Err(FlowWireError::InvalidField { field: WireField::ControlErrno })
    );
    // Every other message carries errno 0.
    for (ty, detail) in [(1u8, 0u8), (3, 0), (2, 1), (2, 10), (2, 12)] {
        assert_eq!(
            ControlMessage::decode(&control_bytes(ty, detail, 111, 1)),
            Err(FlowWireError::InvalidField { field: WireField::ControlErrno }),
            "type {ty} code {detail}"
        );
    }
    // Only Refused carries a detail byte.
    for (ty, errno) in [(1u8, 0u16), (3, 0), (4, 110)] {
        assert_eq!(
            ControlMessage::decode(&control_bytes(ty, 1, errno, 0x8000_0001)),
            Err(FlowWireError::InvalidField { field: WireField::ControlDetail }),
            "type {ty}"
        );
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn kinds_and_refusals_have_distinct_labels() {
    let kinds: BTreeSet<&str> =
        [FlowKind::TcpConnect, FlowKind::DatagramStream, FlowKind::TcpAccept]
            .map(FlowKind::as_str)
            .into_iter()
            .collect();
    assert_eq!(kinds.len(), 3);
    assert!(kinds.iter().all(|label| !label.is_empty()));

    let refusals: BTreeSet<&str> =
        refusal_table().map(|(refusal, _)| refusal.as_str()).into_iter().collect();
    assert_eq!(refusals.len(), 12);
    assert!(refusals.iter().all(|label| !label.is_empty()));
    // The label names the refusal class, not its errno.
    assert_eq!(
        FlowRefusal::ApplicationRefused { errno: KernelErrno::ECONNREFUSED }.as_str(),
        FlowRefusal::ApplicationRefused { errno: KernelErrno::ETIMEDOUT }.as_str(),
    );
}

// ===========================================================================
// KernelErrno — the errno newtype (never 0)
// ===========================================================================

proptest! {
    /// Every non-zero errno is accepted; `get` returns it, `Display` is
    /// decimal, `FromStr` re-parses it, and serde round-trips through the
    /// same form (the mandatory newtype roundtrip).
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_non_zero_errno_round_trips(raw in 1u16..=u16::MAX) {
        let errno = KernelErrno::new(raw).expect("a non-zero errno is accepted");
        prop_assert_eq!(errno.get(), raw);
        prop_assert_eq!(errno.to_string(), raw.to_string());
        prop_assert_eq!(raw.to_string().parse::<KernelErrno>(), Ok(errno));
        let json = serde_json::to_string(&errno).expect("serialize");
        prop_assert_eq!(serde_json::from_str::<KernelErrno>(&json).expect("deserialize"), errno);
    }
}

/// Errno 0 is refused; the named constants carry their Linux values.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn errno_zero_is_refused_and_the_named_errnos_carry_their_values() {
    assert_eq!(
        KernelErrno::new(0),
        Err(FlowWireError::InvalidField { field: WireField::ControlErrno })
    );
    assert_eq!(KernelErrno::ENODEV.get(), 19);
    assert_eq!(KernelErrno::ECONNRESET.get(), 104);
    assert_eq!(KernelErrno::ETIMEDOUT.get(), 110);
    assert_eq!(KernelErrno::ECONNREFUSED.get(), 111);
    assert_eq!(errno(111), KernelErrno::ECONNREFUSED);
}

/// D-UP-13: `FromStr` refuses 0 as the encoding rule does, and every other
/// non-decimal or out-of-width text as `NotDecimal`, naming the errno field.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn errno_text_that_is_zero_or_not_decimal_is_refused() {
    assert_eq!(
        "0".parse::<KernelErrno>(),
        Err(FlowWireError::InvalidField { field: WireField::ControlErrno })
    );
    for text in ["", "-1", "65536", "ECONNRESET", "0x68", " 104", "104 "] {
        assert_eq!(
            text.parse::<KernelErrno>(),
            Err(FlowWireError::NotDecimal { field: WireField::ControlErrno }),
            "{text:?}"
        );
    }
    assert!(serde_json::from_str::<KernelErrno>("\"0\"").is_err(), "serde refuses errno 0");
}

/// The wire error texts an operator reads in owner audits (C6b).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn wire_errors_render_their_pinned_messages() {
    assert_eq!(
        FlowWireError::NotDecimal { field: WireField::ControlErrno }.to_string(),
        "field ControlErrno is not a decimal integer"
    );
    assert_eq!(
        FlowWireError::UnknownKind { kind: 3 }.to_string(),
        "flow request kind 3 is not defined"
    );
    assert_eq!(
        FlowWireError::InvalidField { field: WireField::LaunchToken }.to_string(),
        "field LaunchToken has an invalid value"
    );
    assert_eq!(
        FlowWireError::OriginMismatch { kind: FlowKind::TcpAccept }.to_string(),
        "flow id origin does not match kind TcpAccept"
    );
}

/// The node pair capacity (D3, ADR-0208): four flows per attachment at the
/// target density.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_node_pair_capacity_carries_its_pinned_value() {
    assert_eq!(GUEST_FLOW_MAX_PAIRS, 65_536);
}

// ===========================================================================
// Deadlines (one SSOT) — INB-1 ordering
// ===========================================================================

/// The pinned deadlines, and INB-1's ordering: the host dial waits longer
/// than the guest's own connect bound, so a guest that answers within its
/// bound is always heard (`DialDeadlineAfterGuestReply`; also a compile-time
/// assertion in the codec module).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_deadlines_carry_their_pinned_values_and_the_dial_outwaits_the_guest() {
    assert_eq!(FLOW_REQUEST_DEADLINE, Duration::from_secs(2));
    assert_eq!(FLOW_PAIRING_DEADLINE, Duration::from_secs(5));
    assert_eq!(GUEST_INBOUND_CONNECT_TIMEOUT, Duration::from_millis(1_500));
    assert_eq!(GUEST_DIAL_PAIRING_DEADLINE, Duration::from_secs(3));
    assert_eq!(CONTROL_RECONNECT_BACKOFF, Duration::from_millis(250));
    assert!(GUEST_DIAL_PAIRING_DEADLINE > GUEST_INBOUND_CONNECT_TIMEOUT);
}

/// The declared-port strategy above excludes exactly the D25 guest
/// reservation.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_declared_port_strategy_excludes_exactly_the_guest_reservation() {
    assert_eq!(guest_reserved_ports(), 61_000..=62_024);
}
