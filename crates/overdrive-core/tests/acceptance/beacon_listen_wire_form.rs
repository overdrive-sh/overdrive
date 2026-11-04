//! The `LISTEN <tcp|udp> <port>` beacon line (D13 item 2; S-wire).
//!
//! Display / FromStr round-trip for every protocol and every non-zero port, and
//! a typed `BeaconParseError` (never a panic) for malformed lines.
//! CONTRACT_SHAPE: pure-function.

#![allow(clippy::expect_used, clippy::doc_markdown, reason = "test fixtures")]

use std::num::NonZeroU16;

use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::vm::beacon::{BeaconMessage, BeaconParseError};
use proptest::prelude::*;

fn arb_proto() -> impl Strategy<Value = Proto> {
    prop_oneof![Just(Proto::Tcp), Just(Proto::Udp)]
}

proptest! {
    /// Every protocol x every non-zero port round-trips through the text form.
    #[test]
    #[ignore = "pending DELIVER: D13 LISTEN beacon line wire form"]
    fn listen_round_trips_for_every_protocol_and_port(
        protocol in arb_proto(),
        port in 1_u16..=u16::MAX,
    ) {
        let message = BeaconMessage::Listen {
            protocol,
            port: NonZeroU16::new(port).expect("range excludes zero"),
        };
        let line = message.to_string();
        let proto = if protocol == Proto::Tcp { "tcp" } else { "udp" };
        prop_assert_eq!(&line, &format!("LISTEN {proto} {port}"));
        prop_assert_eq!(line.parse::<BeaconMessage>(), Ok(message));
    }

    /// A non-numeric or out-of-range port is a typed error, never a panic.
    #[test]
    #[ignore = "pending DELIVER: D13 LISTEN beacon line wire form"]
    fn listen_with_an_out_of_range_port_is_a_typed_error(
        protocol in arb_proto(),
        port in 65_536_u32..=u32::MAX,
    ) {
        let proto = if protocol == Proto::Tcp { "tcp" } else { "udp" };
        let parsed: Result<BeaconMessage, BeaconParseError> =
            format!("LISTEN {proto} {port}").parse();
        prop_assert!(parsed.is_err());
    }
}

/// Wrong field count, port 0, unknown protocol and a non-numeric port are each
/// a typed `BeaconParseError`.
#[test]
#[ignore = "pending DELIVER: D13 LISTEN beacon line wire form"]
fn malformed_listen_lines_are_rejected_with_a_typed_error() {
    for line in [
        "LISTEN",
        "LISTEN tcp",
        "LISTEN tcp 8080 extra",
        "LISTEN tcp 0",
        "LISTEN sctp 8080",
        "LISTEN tcp http",
        "LISTEN tcp -1",
        "LISTEN tcp 65536",
        "LISTEN tcp ",
    ] {
        let parsed: Result<BeaconMessage, BeaconParseError> = line.parse();
        assert!(parsed.is_err(), "{line:?} must be rejected, got {parsed:?}");
    }
}
