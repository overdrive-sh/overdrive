//! P2 — vsock flow/control/token wire codec (DISTILL, netns-density-295).
//!
//! Translates `docs/feature/netns-density-295/distill/test-scenarios-p2-owner-flows.md`
//! P2-S1 (codec roundtrip + reject-all-zero) and P2-S2 (control-message
//! half-close codec) into Rust `#[test]` / `proptest!` bodies. No `.feature`
//! files; port-to-port against the public API of the design-pinned wire module
//! (`overdrive_core::guest_transport::wire`), `overdrive_core::guest_transport::
//! LaunchToken` and `overdrive_core::net::ListenPort`
//! (feature-delta §"[REF] Wire contracts — flow request, control message,
//! datagram frame (D5, D18, D23)" and §"[REF] Core vocabulary").
//!
//! Layer 1 (pure function): `proptest` full per Mandate 9. The codec mutates no
//! observable state, so the state-delta/Universe guard does not apply
//! (pure-function exemption). `@example`-pinned golden bytes pin the exact wire
//! layout across the Rust codec (the host programs and guest programs pin the
//! same bytes in their own Tier-2/3 tests — this is the Rust half of that one
//! golden set).
//!
//! RED scaffold: this body is authored to the design-pinned interface contract
//! and does not compile until the DELIVER roadmap-reconciliation lands the
//! `overdrive_core::guest_transport` wire codec + `LaunchToken` + `GuestCid` +
//! `overdrive_core::net::ListenPort`. CLAUDE.md §"Implement to the design —
//! never invent API surface": this file references only surface the design
//! pins. It is registered in `tests/acceptance.rs` by the author when that
//! scaffold co-arrives (shared doc §6); until then it is intentionally
//! unregistered (dangling) so it never breaks the workspace build.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::str::FromStr as _;

use overdrive_core::guest_transport::wire::{
    CONTROL_MESSAGE_LEN, CONTROL_OPEN_LEN, ControlMessage, ControlOpen, FLOW_REQUEST_LEN, FlowId,
    FlowKind, FlowOrigin, FlowRefusal, FlowRequest, FlowWireError, WireField,
};
use overdrive_core::guest_transport::{LaunchToken, LaunchTokenError};
use overdrive_core::net::ListenPort;
use proptest::prelude::*;

const IGNORE: &str = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort \
     (roadmap-reconciliation core-vocabulary step)";

// -----------------------------------------------------------------------------
// Strategies
// -----------------------------------------------------------------------------

/// A non-zero 16-byte launch token (all-zero is the one invalid encoding).
fn a_token() -> impl Strategy<Value = LaunchToken> {
    any::<[u8; 16]>().prop_map(|mut bytes| {
        if bytes.iter().all(|&b| b == 0) {
            bytes[0] = 1;
        }
        LaunchToken::from_bytes(bytes).expect("non-zero bytes are a valid launch token")
    })
}

/// A guest-origin flow id (`seq < 2^31`); kinds 1–3 require origin Guest.
fn a_guest_flow() -> impl Strategy<Value = FlowId> {
    (0u32..(1 << 31))
        .prop_map(|seq| FlowId::new(FlowOrigin::Guest, seq).expect("seq < 2^31 is valid"))
}

/// A host-origin flow id; kind 4 (`TcpAccept`) requires origin Host.
fn a_host_flow() -> impl Strategy<Value = FlowId> {
    (0u32..(1 << 31))
        .prop_map(|seq| FlowId::new(FlowOrigin::Host, seq).expect("seq < 2^31 is valid"))
}

/// A unicast-shaped `SocketAddrV4`; the port field is never 0 (wire contract).
fn a_sockaddr() -> impl Strategy<Value = SocketAddrV4> {
    (any::<[u8; 4]>(), 1u16..=u16::MAX)
        .prop_map(|(octets, port)| SocketAddrV4::new(Ipv4Addr::from(octets), port))
}

/// A non-zero guest listen port (`TcpAccept.listen_port`).
fn a_listen_port() -> impl Strategy<Value = ListenPort> {
    (1u16..=u16::MAX).prop_map(|p| ListenPort::new(p).expect("non-zero listen port is valid"))
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
    prop_oneof![
        Just(FlowRefusal::NotActive),
        Just(FlowRefusal::Policy),
        Just(FlowRefusal::IntakeUnavailable),
        Just(FlowRefusal::Capacity),
        Just(FlowRefusal::Malformed),
        Just(FlowRefusal::Unsupported),
        Just(FlowRefusal::Quiesced),
        Just(FlowRefusal::MeshUnreachable),
        Just(FlowRefusal::ResolveFailed),
        Just(FlowRefusal::DestinationUnreachable),
        Just(FlowRefusal::ApplicationNotListening),
        Just(FlowRefusal::HostInternal),
    ]
}

fn a_control_message() -> impl Strategy<Value = ControlMessage> {
    prop_oneof![
        a_guest_flow().prop_map(|flow| ControlMessage::Paired { flow }),
        a_host_flow().prop_map(|flow| ControlMessage::Paired { flow }),
        (a_guest_flow(), a_refusal())
            .prop_map(|(flow, refusal)| ControlMessage::Refused { flow, refusal }),
        a_guest_flow().prop_map(|flow| ControlMessage::Abort { flow }),
        (a_listen_port(), any::<bool>())
            .prop_map(|(port, listening)| ControlMessage::ListenState { port, listening }),
        a_guest_flow().prop_map(|flow| ControlMessage::HalfClose { flow }),
        a_host_flow().prop_map(|flow| ControlMessage::HalfCloseAck { flow }),
    ]
}

// -----------------------------------------------------------------------------
// P2-S1 — flow request / control open / launch token roundtrip + reject-all-zero
// -----------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Every flow-request kind round-trips carrying its launch token unchanged.
    #[test]
    #[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
    fn flow_request_round_trips(req in a_flow_request()) {
        let bytes = req.encode();
        prop_assert_eq!(bytes.len(), FLOW_REQUEST_LEN);
        let decoded = FlowRequest::decode(&bytes).expect("canonical encoding decodes");
        prop_assert_eq!(decoded, req);
        // The launch token survives the round trip bit-exact (ATTR-TOKEN).
        prop_assert_eq!(decoded.launch(), req.launch());
    }

    /// Launch token round-trips through `from_bytes` / `as_bytes` and through
    /// `Display` / `FromStr` (32 lowercase hex); never all zero.
    #[test]
    #[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
    fn launch_token_round_trips(token in a_token()) {
        let via_bytes = LaunchToken::from_bytes(*token.as_bytes()).expect("non-zero");
        prop_assert_eq!(via_bytes, token);

        let rendered = token.to_string();
        prop_assert_eq!(rendered.len(), 32, "32 hex characters");
        prop_assert!(
            rendered.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "lowercase hex only, got {rendered:?}"
        );
        let parsed = LaunchToken::from_str(&rendered).expect("display output re-parses");
        prop_assert_eq!(parsed, token);
    }

    /// `ControlOpen` round-trips carrying the guest's own launch token.
    #[test]
    #[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
    fn control_open_round_trips(launch in a_token()) {
        let open = ControlOpen { launch };
        let bytes = open.encode();
        prop_assert_eq!(bytes.len(), CONTROL_OPEN_LEN);
        prop_assert_eq!(ControlOpen::decode(&bytes).expect("decodes"), open);
    }
}

#[test]
#[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
fn all_zero_launch_token_is_rejected_everywhere() {
    // from_bytes rejects all zero as the invalid encoding (ATTR-TOKEN).
    assert_eq!(LaunchToken::from_bytes([0u8; 16]), Err(LaunchTokenError::Zero));

    // 32 zero hex characters decode to all-zero bytes → Zero.
    assert_eq!(LaunchToken::from_str(&"0".repeat(32)), Err(LaunchTokenError::Zero));

    // A flow request whose token field is all zero is InvalidField { LaunchToken }.
    let mut bytes = canonical_tcp_connect();
    for b in &mut bytes[16..32] {
        *b = 0;
    }
    assert_eq!(
        FlowRequest::decode(&bytes),
        Err(FlowWireError::InvalidField { field: WireField::LaunchToken })
    );
}

#[test]
#[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
fn launch_token_from_str_rejects_non_canonical() {
    // Uppercase hex is rejected (machine-generated, never typed; case-sensitive).
    assert_eq!(
        LaunchToken::from_str("0102030405060708090A0B0C0D0E0F10"),
        Err(LaunchTokenError::Unparseable)
    );
    // Wrong length (31 and 33 characters).
    assert_eq!(LaunchToken::from_str(&"a".repeat(31)), Err(LaunchTokenError::Unparseable));
    assert_eq!(LaunchToken::from_str(&"a".repeat(33)), Err(LaunchTokenError::Unparseable));
    // Non-hex character.
    assert_eq!(LaunchToken::from_str(&"g".repeat(32)), Err(LaunchTokenError::Unparseable));
}

// -----------------------------------------------------------------------------
// P2-S1 — flow id origin / seq boundaries and decode rejections
// -----------------------------------------------------------------------------

#[test]
#[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
fn flow_id_origin_bit_and_seq_boundary() {
    // seq boundary: 2^31 - 1 is valid, 2^31 is rejected.
    let max = FlowId::new(FlowOrigin::Guest, (1 << 31) - 1).expect("2^31 - 1 is valid");
    assert_eq!(max.origin(), FlowOrigin::Guest);
    assert_eq!(
        FlowId::new(FlowOrigin::Guest, 1 << 31),
        Err(FlowWireError::InvalidField { field: WireField::FlowSeq })
    );
    assert_eq!(
        FlowId::new(FlowOrigin::Host, u32::MAX),
        Err(FlowWireError::InvalidField { field: WireField::FlowSeq })
    );

    // The top bit carries the origin: a guest-origin and a host-origin id for
    // the same seq differ exactly in the top bit.
    let g = FlowId::new(FlowOrigin::Guest, 1).expect("valid");
    let h = FlowId::new(FlowOrigin::Host, 1).expect("valid");
    assert_eq!(g.origin(), FlowOrigin::Guest);
    assert_eq!(h.origin(), FlowOrigin::Host);
    assert_eq!(h.get(), g.get() | (1 << 31));
}

#[test]
#[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
fn flow_request_decode_rejects_malformed() {
    // Bad magic.
    let mut bytes = canonical_tcp_connect();
    bytes[0] = b'X';
    assert_eq!(
        FlowRequest::decode(&bytes),
        Err(FlowWireError::BadMagic { observed: [b'X', b'F'] })
    );

    // Unsupported version.
    let mut bytes = canonical_tcp_connect();
    bytes[2] = 3;
    assert_eq!(FlowRequest::decode(&bytes), Err(FlowWireError::UnsupportedVersion { version: 3 }));

    // Unknown kind (0 and 5).
    let mut bytes = canonical_tcp_connect();
    bytes[3] = 0;
    assert_eq!(FlowRequest::decode(&bytes), Err(FlowWireError::UnknownKind { kind: 0 }));
    bytes[3] = 5;
    assert_eq!(FlowRequest::decode(&bytes), Err(FlowWireError::UnknownKind { kind: 5 }));

    // Origin mismatch: a TcpConnect (kind 1, must be Guest-origin) whose flow id
    // has the host-origin top bit set.
    let mut bytes = canonical_tcp_connect();
    bytes[4] |= 0x80; // set the origin top bit of the BE flow id
    assert_eq!(
        FlowRequest::decode(&bytes),
        Err(FlowWireError::OriginMismatch { kind: FlowKind::TcpConnect })
    );

    // Port 0 is never valid (kinds 1–3 destination port, kind 4 peer port).
    let mut bytes = canonical_tcp_connect();
    bytes[12] = 0;
    bytes[13] = 0;
    assert_eq!(
        FlowRequest::decode(&bytes),
        Err(FlowWireError::InvalidField { field: WireField::Port })
    );
}

// -----------------------------------------------------------------------------
// P2-S1 — golden bytes (pin the exact wire layout; the one golden set the host
//          and guest programs mirror)
// -----------------------------------------------------------------------------

/// Canonical `TcpConnect` bytes: flow Guest(1), dst 192.0.2.7:443, token 01..10.
fn canonical_tcp_connect() -> [u8; FLOW_REQUEST_LEN] {
    [
        b'O', b'F', // magic
        2,    // version
        1,    // kind = TcpConnect
        0x00, 0x00, 0x00, 0x01, // flow id (Guest, seq 1)
        192, 0, 2, 7, // address 192.0.2.7
        0x01, 0xBB, // port 443
        0x00, 0x00, // listen port (0 for kinds 1–3)
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, // launch token
        0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
    ]
}

#[test]
#[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
fn golden_tcp_connect_request() {
    let token = LaunchToken::from_bytes([
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10,
    ])
    .expect("non-zero");
    let req = FlowRequest::TcpConnect {
        flow: FlowId::new(FlowOrigin::Guest, 1).expect("valid"),
        destination: SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 7), 443),
        launch: token,
    };
    assert_eq!(req.encode(), canonical_tcp_connect());
    assert_eq!(FlowRequest::decode(&canonical_tcp_connect()).expect("decodes"), req);
    // The Display form of the token is the kernel command-line value
    // (`overdrive.transport=vsock2,...,launch=<hex>`).
    assert_eq!(token.to_string(), "0102030405060708090a0b0c0d0e0f10");
}

#[test]
#[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
fn golden_tcp_accept_request_carries_listen_port() {
    let token = LaunchToken::from_bytes([0xAA; 16]).expect("non-zero");
    let req = FlowRequest::TcpAccept {
        flow: FlowId::new(FlowOrigin::Host, 2).expect("valid"),
        client: SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 9), 50000),
        listen_port: ListenPort::new(8080).expect("non-zero"),
        launch: token,
    };
    let bytes = req.encode();
    // kind byte = 4; flow id top bit set (Host origin); listen port field 8080.
    assert_eq!(bytes[3], 4);
    assert_eq!(bytes[4] & 0x80, 0x80);
    assert_eq!(u16::from_be_bytes([bytes[14], bytes[15]]), 8080);
    assert_eq!(FlowRequest::decode(&bytes).expect("decodes"), req);
}

#[test]
#[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
fn golden_control_open() {
    let open = ControlOpen { launch: LaunchToken::from_bytes([0x11; 16]).expect("non-zero") };
    let bytes = open.encode();
    assert_eq!(&bytes[0..2], b"OC");
    assert_eq!(bytes[2], 1, "version");
    assert_eq!(&bytes[3..8], &[0u8; 5], "reserved all zero");
    assert_eq!(&bytes[8..24], &[0x11u8; 16]);
    assert_eq!(ControlOpen::decode(&bytes).expect("decodes"), open);
}

#[test]
#[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
fn control_open_decode_rejects_malformed() {
    let base =
        ControlOpen { launch: LaunchToken::from_bytes([0x22; 16]).expect("non-zero") }.encode();

    // Bad magic.
    let mut bytes = base;
    bytes[0] = b'Z';
    assert_eq!(
        ControlOpen::decode(&bytes),
        Err(FlowWireError::BadMagic { observed: [b'Z', b'C'] })
    );

    // Unsupported version.
    let mut bytes = base;
    bytes[2] = 2;
    assert_eq!(ControlOpen::decode(&bytes), Err(FlowWireError::UnsupportedVersion { version: 2 }));

    // A non-zero reserved byte is InvalidField { ControlOpenReserved }.
    let mut bytes = base;
    bytes[5] = 1;
    assert_eq!(
        ControlOpen::decode(&bytes),
        Err(FlowWireError::InvalidField { field: WireField::ControlOpenReserved })
    );

    // All-zero launch token is still rejected.
    let mut bytes = base;
    for b in &mut bytes[8..24] {
        *b = 0;
    }
    assert_eq!(
        ControlOpen::decode(&bytes),
        Err(FlowWireError::InvalidField { field: WireField::LaunchToken })
    );
}

// -----------------------------------------------------------------------------
// P2-S2 — control message (incl. HalfClose / HalfCloseAck) codec
// -----------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Every control-message type round-trips, including the half-close signals.
    #[test]
    #[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
    fn control_message_round_trips(msg in a_control_message()) {
        let bytes = msg.encode();
        prop_assert_eq!(bytes.len(), CONTROL_MESSAGE_LEN);
        prop_assert_eq!(ControlMessage::decode(&bytes).expect("decodes"), msg);
    }

    /// The refusal code is a stable bijection with `FlowRefusal` over 1..=12.
    #[test]
    #[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
    fn refusal_code_is_bijective(refusal in a_refusal()) {
        let code = refusal.as_u8();
        prop_assert!((1..=12).contains(&code), "refusal codes are 1..=12, got {code}");
        // The code embedded in a Refused control message round-trips.
        let msg = ControlMessage::Refused {
            flow: FlowId::new(FlowOrigin::Guest, 7).expect("valid"),
            refusal,
        };
        prop_assert_eq!(ControlMessage::decode(&msg.encode()).expect("decodes"), msg);
    }
}

#[test]
#[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
fn control_message_decode_rejects_unknown_type_and_refusal() {
    // Control message type 0 and 7 (out of the defined 1..=6 range) are rejected.
    let mut bytes = [0u8; CONTROL_MESSAGE_LEN];
    bytes[0] = 0;
    assert_eq!(ControlMessage::decode(&bytes), Err(FlowWireError::UnknownControlType { ty: 0 }));
    bytes[0] = 7;
    assert_eq!(ControlMessage::decode(&bytes), Err(FlowWireError::UnknownControlType { ty: 7 }));

    // A Refused (type 2) carrying an undefined refusal code is UnknownRefusal.
    let mut bytes = [0u8; CONTROL_MESSAGE_LEN];
    bytes[0] = 2; // Refused
    bytes[1] = 0; // refusal code 0 (undefined)
    bytes[4..8].copy_from_slice(&1u32.to_be_bytes());
    assert_eq!(ControlMessage::decode(&bytes), Err(FlowWireError::UnknownRefusal { code: 0 }));
    bytes[1] = 13; // refusal code 13 (undefined)
    assert_eq!(ControlMessage::decode(&bytes), Err(FlowWireError::UnknownRefusal { code: 13 }));
}

#[test]
#[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
fn golden_half_close_messages() {
    let flow = FlowId::new(FlowOrigin::Guest, 3).expect("valid");
    // HalfClose = type 5; HalfCloseAck = type 6; flow id in the last 4 bytes.
    let half = ControlMessage::HalfClose { flow };
    let half_bytes = half.encode();
    assert_eq!(half_bytes[0], 5);
    assert_eq!(
        u32::from_be_bytes([half_bytes[4], half_bytes[5], half_bytes[6], half_bytes[7]]),
        flow.get()
    );
    assert_eq!(ControlMessage::decode(&half_bytes).expect("decodes"), half);

    let ack = ControlMessage::HalfCloseAck { flow };
    let ack_bytes = ack.encode();
    assert_eq!(ack_bytes[0], 6);
    assert_eq!(ControlMessage::decode(&ack_bytes).expect("decodes"), ack);
}

#[test]
#[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
fn golden_listen_state_message() {
    // ListenState = type 4; detail byte 0/1 listening; port in bytes 2..4; flow 0.
    let msg = ControlMessage::ListenState {
        port: ListenPort::new(8443).expect("non-zero"),
        listening: true,
    };
    let bytes = msg.encode();
    assert_eq!(bytes[0], 4);
    assert_eq!(bytes[1], 1, "listening = true");
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 8443);
    assert_eq!(&bytes[4..8], &[0u8; 4], "flow id field is 0 for ListenState");
    assert_eq!(ControlMessage::decode(&bytes).expect("decodes"), msg);
}

/// Keep `IGNORE` referenced so the constant documents the single activation
/// reason even though each test spells its own `#[ignore]` literal (proptest
/// attribute macros require a string literal, not a `const`).
#[test]
#[ignore = "pending DELIVER: overdrive-core::guest_transport wire codec + LaunchToken + ListenPort (roadmap-reconciliation core-vocabulary step)"]
fn ignore_reason_documented() {
    assert!(IGNORE.contains("roadmap-reconciliation"));
}
