//! P2 — guest-transport wire codec: flow request, control open, control
//! message (GH #295, vsock attachment replacement).
//!
//! Spec: `docs/feature/netns-density-295/distill/test-scenarios-p2-owner-flows.md`
//! P2-S1 (request / control-open / launch-token codec, all-zero token refused)
//! and P2-S2 (control messages incl. `HalfClose` / `HalfCloseAck`). Contract:
//! `docs/feature/netns-density-295/feature-delta.md` § "[REF] Wire contracts —
//! flow request, control message, datagram frame (D5, D18, D23)".
//!
//! Driving port: `overdrive_core::guest_transport::wire` (layer 1, pure:
//! `proptest` full, Mandate 9). `decode` accepts only canonical encodings, so
//! two properties pin the codec from both sides: every valid value
//! round-trips, and every byte string `decode` accepts re-encodes to itself.
//! The golden bytes are the Rust half of the one golden set the host and
//! guest programs mirror in their own tiers.
//!
//! Not here: READY's `launch=` field (`BeaconMessage::Ready { launch }` and its
//! `InvalidLaunchToken` refusal, DG-7, DG-2 row 13; `vm_beacon_roundtrip.rs`)
//! and the datagram-frame codec (DG-8; `vsock_datagram_frame.rs`).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    reason = "test bodies unwrap fixtures they construct; the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::str::FromStr as _;

use overdrive_core::guest_transport::wire::{
    CONTROL_MESSAGE_LEN, CONTROL_OPEN_LEN, ControlMessage, ControlOpen, FLOW_REQUEST_LEN, FlowId,
    FlowKind, FlowOrigin, FlowRefusal, FlowRequest, FlowWireError, WireField,
};
use overdrive_core::guest_transport::{LaunchToken, LaunchTokenError, guest_reserved_ports};
use overdrive_core::net::ListenPort;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Vocabulary
// ---------------------------------------------------------------------------

/// The pinned refusal-code table (feature-delta § Wire contracts, control
/// message refusal codes 1..=12).
const REFUSAL_TABLE: [(FlowRefusal, u8); 12] = [
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
    (FlowRefusal::ApplicationNotListening, 11),
    (FlowRefusal::HostInternal, 12),
];

const TOKEN_BYTES: [u8; 16] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
];

fn token() -> LaunchToken {
    LaunchToken::from_bytes(TOKEN_BYTES).expect("non-zero token")
}

fn listen_port(port: u16) -> ListenPort {
    port.to_string().parse().expect("a non-zero listen port")
}

fn guest_flow(seq: u32) -> FlowId {
    FlowId::new(FlowOrigin::Guest, seq).expect("seq < 2^31")
}

fn host_flow(seq: u32) -> FlowId {
    FlowId::new(FlowOrigin::Host, seq).expect("seq < 2^31")
}

/// The flow request header the golden cases share: magic `OF`, version 2.
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

fn control_bytes(ty: u8, detail: u8, port: u16, flow: u32) -> [u8; CONTROL_MESSAGE_LEN] {
    let mut bytes = [0u8; CONTROL_MESSAGE_LEN];
    bytes[0] = ty;
    bytes[1] = detail;
    bytes[2..4].copy_from_slice(&port.to_be_bytes());
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

fn a_sockaddr() -> impl Strategy<Value = SocketAddrV4> {
    (any::<[u8; 4]>(), 1u16..=u16::MAX)
        .prop_map(|(octets, port)| SocketAddrV4::new(Ipv4Addr::from(octets), port))
}

/// A declared or reported listen port. The guest never reports a port of its
/// own reservation (`guest_reserved_ports()`, 61,000..=62,024), so no valid
/// sender encodes one (DG-12): the round-trip property ranges over ports
/// outside it.
fn a_listen_port() -> impl Strategy<Value = ListenPort> {
    prop_oneof![1u16..61_000, 62_025u16..=u16::MAX].prop_map(listen_port)
}

fn a_flow_request() -> impl Strategy<Value = FlowRequest> {
    prop_oneof![
        (a_guest_flow(), a_sockaddr(), a_token()).prop_map(|(flow, destination, launch)| {
            FlowRequest::TcpConnect { flow, destination, launch }
        }),
        (a_guest_flow(), a_sockaddr(), a_token()).prop_map(|(flow, destination, launch)| {
            FlowRequest::DatagramStream { flow, destination, launch }
        }),
        (a_guest_flow(), a_sockaddr(), a_token()).prop_map(|(flow, destination, launch)| {
            FlowRequest::DatagramSeqpacket { flow, destination, launch }
        }),
        (a_host_flow(), a_sockaddr(), a_listen_port(), a_token()).prop_map(
            |(flow, client, listen_port, launch)| FlowRequest::TcpAccept {
                flow,
                client,
                listen_port,
                launch
            }
        ),
    ]
}

fn a_refusal() -> impl Strategy<Value = FlowRefusal> {
    proptest::sample::select(REFUSAL_TABLE.map(|(refusal, _)| refusal).to_vec())
}

fn a_control_message() -> impl Strategy<Value = ControlMessage> {
    prop_oneof![
        any_flow().prop_map(|flow| ControlMessage::Paired { flow }),
        (any_flow(), a_refusal())
            .prop_map(|(flow, refusal)| ControlMessage::Refused { flow, refusal }),
        any_flow().prop_map(|flow| ControlMessage::Abort { flow }),
        (a_listen_port(), any::<bool>())
            .prop_map(|(port, listening)| ControlMessage::ListenState { port, listening }),
        any_flow().prop_map(|flow| ControlMessage::HalfClose { flow }),
        any_flow().prop_map(|flow| ControlMessage::HalfCloseAck { flow }),
    ]
}

// ===========================================================================
// P2-S1 — flow request and control open
// ===========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Every request kind round-trips; its kind, flow and launch token
    /// survive bit-exact (ATTR-TOKEN).
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    fn every_flow_request_round_trips(req in a_flow_request()) {
        let bytes = req.encode();
        prop_assert_eq!(bytes.len(), FLOW_REQUEST_LEN);
        let decoded = FlowRequest::decode(&bytes).expect("a canonical encoding decodes");
        prop_assert_eq!(decoded, req);
        prop_assert_eq!(decoded.kind(), req.kind());
        prop_assert_eq!(decoded.flow(), req.flow());
        prop_assert_eq!(decoded.launch(), req.launch());
        // Kinds 1–3 are guest-opened, kind 4 host-opened.
        let host_opened = matches!(req.kind(), FlowKind::TcpAccept);
        prop_assert_eq!(req.flow().origin() == FlowOrigin::Host, host_opened);
    }

    /// `decode` accepts only canonical encodings: any 32 bytes it accepts
    /// re-encode to exactly those bytes.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    fn every_accepted_request_is_canonical(bytes in any::<[u8; FLOW_REQUEST_LEN]>()) {
        if let Ok(req) = FlowRequest::decode(&bytes) {
            prop_assert_eq!(req.encode(), bytes);
        }
    }

    /// A request near the canonical form (valid header, one field mutated)
    /// either decodes to a value that re-encodes to the same bytes or is
    /// refused — the arbitrary-bytes property rarely reaches past the magic.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
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
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
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

    /// Launch tokens round-trip through their bytes and their 32-character
    /// lowercase hex form (the kernel command-line value).
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    fn launch_token_round_trips(launch in a_token()) {
        prop_assert_eq!(LaunchToken::from_bytes(*launch.as_bytes()), Ok(launch));
        let rendered = launch.to_string();
        prop_assert_eq!(rendered.len(), 32);
        prop_assert_eq!(LaunchToken::from_str(&rendered), Ok(launch));
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn golden_flow_requests_pin_every_kind() {
    let destination = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 7), 443);
    let guest_one = [0x00, 0x00, 0x00, 0x01];
    let cases = [
        (
            FlowRequest::TcpConnect { flow: guest_flow(1), destination, launch: token() },
            request_bytes(1, guest_one, [192, 0, 2, 7], 443, 0),
        ),
        (
            FlowRequest::DatagramStream { flow: guest_flow(1), destination, launch: token() },
            request_bytes(2, guest_one, [192, 0, 2, 7], 443, 0),
        ),
        (
            FlowRequest::DatagramSeqpacket { flow: guest_flow(1), destination, launch: token() },
            request_bytes(3, guest_one, [192, 0, 2, 7], 443, 0),
        ),
        (
            FlowRequest::TcpAccept {
                flow: host_flow(2),
                client: SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 9), 50_000),
                listen_port: listen_port(8080),
                launch: token(),
            },
            request_bytes(4, [0x80, 0x00, 0x00, 0x02], [10, 0, 0, 9], 50_000, 8080),
        ),
    ];
    let kinds = [
        FlowKind::TcpConnect,
        FlowKind::DatagramStream,
        FlowKind::DatagramSeqpacket,
        FlowKind::TcpAccept,
    ];
    for ((req, golden), kind) in cases.into_iter().zip(kinds) {
        assert_eq!(req.encode(), golden, "{kind:?} encodes to its golden bytes");
        assert_eq!(FlowRequest::decode(&golden), Ok(req), "{kind:?} golden bytes decode");
        assert_eq!(req.kind(), kind);
    }
    assert_eq!(token().to_string(), "0102030405060708090a0b0c0d0e0f10");
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn golden_control_open() {
    let mut golden = [0u8; CONTROL_OPEN_LEN];
    golden[0..2].copy_from_slice(b"OC");
    golden[2] = 1;
    golden[8..24].copy_from_slice(&TOKEN_BYTES);
    let open = ControlOpen { launch: token() };
    assert_eq!(open.encode(), golden);
    assert_eq!(ControlOpen::decode(&golden), Ok(open));
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn an_all_zero_launch_token_is_refused_everywhere() {
    assert_eq!(LaunchToken::from_bytes([0u8; 16]), Err(LaunchTokenError::Zero));
    assert_eq!(LaunchToken::from_str(&"0".repeat(32)), Err(LaunchTokenError::Zero));

    let mut request = request_bytes(1, [0, 0, 0, 1], [192, 0, 2, 7], 443, 0);
    request[16..32].fill(0);
    assert_eq!(
        FlowRequest::decode(&request),
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
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
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

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn malformed_flow_requests_are_refused_with_their_cause() {
    let connect = || request_bytes(1, [0, 0, 0, 1], [192, 0, 2, 7], 443, 0);
    let accept = || request_bytes(4, [0x80, 0, 0, 2], [10, 0, 0, 9], 50_000, 8080);

    let mut bytes = connect();
    bytes[0] = b'X';
    assert_eq!(FlowRequest::decode(&bytes), Err(FlowWireError::BadMagic { observed: *b"XF" }));

    let mut bytes = connect();
    bytes[2] = 3;
    assert_eq!(FlowRequest::decode(&bytes), Err(FlowWireError::UnsupportedVersion { version: 3 }));

    for kind in [0u8, 5, 255] {
        let mut bytes = connect();
        bytes[3] = kind;
        assert_eq!(FlowRequest::decode(&bytes), Err(FlowWireError::UnknownKind { kind }));
    }

    // Kinds 1–3 must be guest-opened; kind 4 host-opened.
    let mut bytes = connect();
    bytes[4] |= 0x80;
    assert_eq!(
        FlowRequest::decode(&bytes),
        Err(FlowWireError::OriginMismatch { kind: FlowKind::TcpConnect })
    );
    let mut bytes = accept();
    bytes[4] &= 0x7F;
    assert_eq!(
        FlowRequest::decode(&bytes),
        Err(FlowWireError::OriginMismatch { kind: FlowKind::TcpAccept })
    );

    // The port is never 0, for any kind.
    for mut bytes in [connect(), accept()] {
        bytes[12..14].fill(0);
        assert_eq!(
            FlowRequest::decode(&bytes),
            Err(FlowWireError::InvalidField { field: WireField::Port })
        );
    }

    // Kind 4 carries a non-zero declared listen port; kinds 1–3 carry 0.
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
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
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

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn launch_token_text_is_exactly_32_lowercase_hex_characters() {
    for text in [
        "0102030405060708090A0B0C0D0E0F10".to_owned(),
        "a".repeat(31),
        "a".repeat(33),
        "g".repeat(32),
    ] {
        assert_eq!(LaunchToken::from_str(&text), Err(LaunchTokenError::Unparseable), "{text:?}");
    }
}

// ===========================================================================
// P2-S2 — control messages, half-close signals included
// ===========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Every control message type round-trips.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    fn every_control_message_round_trips(msg in a_control_message()) {
        let bytes = msg.encode();
        prop_assert_eq!(bytes.len(), CONTROL_MESSAGE_LEN);
        prop_assert_eq!(ControlMessage::decode(&bytes), Ok(msg));
    }

    /// Any 8 bytes `decode` accepts re-encode to themselves.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    fn every_accepted_control_message_is_canonical(bytes in any::<[u8; CONTROL_MESSAGE_LEN]>()) {
        if let Ok(msg) = ControlMessage::decode(&bytes) {
            prop_assert_eq!(msg.encode(), bytes);
        }
    }

    /// DG-12: a `ListenState` whose port lies in `guest_reserved_ports()` is
    /// refused `InvalidField { field: ListenPort }`, listening or not — no
    /// valid sender produces it, so a canonical decoder does not accept it.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    fn every_reserved_port_in_a_listen_state_is_refused(
        port in guest_reserved_ports(),
        listening in any::<bool>(),
    ) {
        prop_assert_eq!(
            ControlMessage::decode(&control_bytes(4, u8::from(listening), port, 0)),
            Err(FlowWireError::InvalidField { field: WireField::ListenPort })
        );
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn golden_control_messages_pin_every_type_and_refusal_code() {
    let flow = guest_flow(3);
    let cases = [
        (ControlMessage::Paired { flow }, control_bytes(1, 0, 0, 3)),
        (ControlMessage::Abort { flow }, control_bytes(3, 0, 0, 3)),
        (
            ControlMessage::ListenState { port: listen_port(8443), listening: true },
            control_bytes(4, 1, 8443, 0),
        ),
        (
            ControlMessage::ListenState { port: listen_port(8443), listening: false },
            control_bytes(4, 0, 8443, 0),
        ),
        (ControlMessage::HalfClose { flow }, control_bytes(5, 0, 0, 3)),
        (ControlMessage::HalfCloseAck { flow: host_flow(3) }, control_bytes(6, 0, 0, 0x8000_0003)),
    ];
    for (msg, golden) in cases {
        assert_eq!(msg.encode(), golden, "{msg:?}");
        assert_eq!(ControlMessage::decode(&golden), Ok(msg));
    }
    for (refusal, code) in REFUSAL_TABLE {
        assert_eq!(refusal.as_u8(), code, "{refusal:?}");
        let msg = ControlMessage::Refused { flow, refusal };
        let golden = control_bytes(2, code, 0, 3);
        assert_eq!(msg.encode(), golden, "Refused({refusal:?})");
        assert_eq!(ControlMessage::decode(&golden), Ok(msg));
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn malformed_control_messages_are_refused_with_their_cause() {
    for ty in [0u8, 7, 255] {
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
    // ListenState: the detail byte is 0 or 1, the port is non-zero, the flow
    // field is 0.
    assert_eq!(
        ControlMessage::decode(&control_bytes(4, 2, 8443, 0)),
        Err(FlowWireError::InvalidField { field: WireField::ControlDetail })
    );
    // Port 0 is no listen port (DG-12: as for a reserved port).
    assert_eq!(
        ControlMessage::decode(&control_bytes(4, 1, 0, 0)),
        Err(FlowWireError::InvalidField { field: WireField::ListenPort })
    );
    assert_eq!(
        ControlMessage::decode(&control_bytes(4, 1, 8443, 9)),
        Err(FlowWireError::InvalidField { field: WireField::ControlFlow })
    );
    // Flow-scoped types carry no detail and no port.
    for ty in [1u8, 3, 5, 6] {
        assert_eq!(
            ControlMessage::decode(&control_bytes(ty, 1, 0, 1)),
            Err(FlowWireError::InvalidField { field: WireField::ControlDetail }),
            "type {ty} detail",
        );
        assert_eq!(
            ControlMessage::decode(&control_bytes(ty, 0, 80, 1)),
            Err(FlowWireError::InvalidField { field: WireField::ControlPort }),
            "type {ty} port",
        );
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn kinds_and_refusals_have_distinct_labels() {
    let kinds: BTreeSet<&str> = [
        FlowKind::TcpConnect,
        FlowKind::DatagramStream,
        FlowKind::DatagramSeqpacket,
        FlowKind::TcpAccept,
    ]
    .map(FlowKind::as_str)
    .into_iter()
    .collect();
    assert_eq!(kinds.len(), 4);
    assert!(kinds.iter().all(|label| !label.is_empty()));

    let refusals: BTreeSet<&str> =
        REFUSAL_TABLE.map(|(refusal, _)| refusal.as_str()).into_iter().collect();
    assert_eq!(refusals.len(), REFUSAL_TABLE.len());
    assert!(refusals.iter().all(|label| !label.is_empty()));
}
