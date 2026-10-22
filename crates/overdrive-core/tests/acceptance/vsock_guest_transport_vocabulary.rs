//! P1 — guest-transport vocabulary in `overdrive-core` (GH #295, vsock
//! attachment replacement).
//!
//! Scenario P1-S1 (CID derivation and the transport newtypes), re-derived for
//! CID-NOCLAIM: a leased CID is `GUEST_CID_BASE + lease offset`, the lease
//! alone gives its uniqueness, and the VMM opens `/dev/vhost-vsock` itself —
//! no CID claim exists in the handoff. Contract:
//! `docs/feature/netns-density-295/feature-delta.md` § "[REF] Core vocabulary
//! — `overdrive-core`" (`GuestCid`, `LaunchToken`, `GuestTransportAssignment`,
//! the guest intake constants of D25, the per-allocation quota and probe cap
//! of D-UP-12 / PD-4, the guest kit's module path of EVMOD-LOAD) and DG-11
//! (`ListenPort` / `InterfaceIndex` in `overdrive_core::net`). Under EVMOD-AB
//! the guest holds no pool of per-flow slots: no slot or cell constant
//! exists, and `GUEST_PROBE_SLOTS` is the most quota-exempt probe dials one
//! allocation holds.
//!
//! Driving ports: the public constructors, codecs and functions of
//! `overdrive_core::guest_transport` / `overdrive_core::net` (layer 1, pure:
//! `proptest` full, Mandate 9).
//!
//! Not here: the stateful `GuestAddressPool::assign` scenarios (P1-S2, P1-S9,
//! P1-S13) — the pool lives in `overdrive-control-plane`.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    reason = "test bodies unwrap fixtures they construct; the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::time::Duration;

use overdrive_core::guest_transport::wire::GUEST_FLOW_MAX_PAIRS;
use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GUEST_CID_SPAN, GUEST_DATAGRAM_SLOT_IDLE, GUEST_FLOW_CONTROL_PORT,
    GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION, GUEST_FLOW_STREAM_PORT, GUEST_FORWARDER_MODULE_PATH,
    GUEST_INBOUND_PORT, GUEST_NODE_GATEWAY, GUEST_NODE_PREFIX, GUEST_OWNER_AUDIT_PERIOD,
    GUEST_PROBE_SLOTS, GUEST_TCP_INTAKE_PORT, GUEST_TCP_PENDING_MAX, GUEST_UDP_SLOT_BASE,
    GUEST_UDP_SLOTS, GuestCid, GuestCidError, GuestFlowPorts, GuestTransportAssignment,
    LaunchToken, LaunchTokenError, guest_reserved_ports,
};
use overdrive_core::net::{InterfaceIndex, InterfaceIndexError, ListenPort, ListenPortError};
use overdrive_core::vm::beacon::BEACON_VSOCK_PORT;
use overdrive_core::vm::config::VsockPort;
use overdrive_sim::adapters::entropy::SimEntropy;
use proptest::prelude::*;

const fn workload_addr() -> Ipv4Addr {
    Ipv4Addr::new(100, 95, 0, 7)
}

/// The leased CID of lease offset `offset` (CID-NOCLAIM).
fn cid_at(offset: u32) -> GuestCid {
    GuestCid::new(GUEST_CID_BASE + offset).expect("an in-range CID")
}

/// Two bound guest-flow listener ports (PORTS-CONFIG); the pool copies the
/// owner's bound ports into every handoff (GUEST-PORTS).
fn bound_flow_ports() -> GuestFlowPorts {
    GuestFlowPorts::new(VsockPort::new(50_240), VsockPort::new(50_243))
        .expect("two distinct ports above the reserved range")
}

fn token(seed: u64) -> LaunchToken {
    LaunchToken::generate(&SimEntropy::new(seed))
}

// ===========================================================================
// P1-S1 — GuestCid: the leased range, its derivation and newtype completeness
// ===========================================================================

proptest! {
    /// Every lease offset of the span derives the CID `GUEST_CID_BASE +
    /// offset`; its decimal `Display` re-parses through `FromStr`, and serde
    /// round-trips through the same form.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_lease_offset_derives_a_cid_that_round_trips(offset in 0u32..GUEST_CID_SPAN) {
        let raw = GUEST_CID_BASE + offset;
        let cid = GuestCid::new(raw).expect("in-range CID accepted");
        prop_assert_eq!(cid.get(), raw);

        let rendered = cid.to_string();
        prop_assert_eq!(&rendered, &raw.to_string(), "Display is decimal");
        prop_assert_eq!(rendered.parse::<GuestCid>().expect("re-parses"), cid);

        let json = serde_json::to_string(&cid).expect("serialize");
        prop_assert_eq!(serde_json::from_str::<GuestCid>(&json).expect("deserialize"), cid);
    }

    /// CID-NOCLAIM: the derivation is injective — distinct lease offsets
    /// never share a CID, so lease uniqueness is CID uniqueness, and the
    /// derived CIDs order as their offsets.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn distinct_lease_offsets_never_share_a_cid(
        a in 0u32..GUEST_CID_SPAN,
        b in 0u32..GUEST_CID_SPAN,
    ) {
        prop_assert_eq!(cid_at(a) == cid_at(b), a == b);
        prop_assert_eq!(cid_at(a).cmp(&cid_at(b)), a.cmp(&b));
    }

    /// Every value outside the leased span is refused `OutOfRange`, by the
    /// constructor and by `FromStr`.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_value_outside_the_span_is_out_of_range(
        raw in prop_oneof![0u32..GUEST_CID_BASE, (GUEST_CID_BASE + GUEST_CID_SPAN)..=u32::MAX],
    ) {
        prop_assert_eq!(GuestCid::new(raw), Err(GuestCidError::OutOfRange { raw }));
        prop_assert_eq!(raw.to_string().parse::<GuestCid>(), Err(GuestCidError::OutOfRange { raw }));
    }
}

/// The kernel-reserved CIDs (0 hypervisor, 1 local, 2 host) and `u32::MAX`
/// are never leasable; the span's last CID is, the next one is not.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_kernel_reserved_cids_are_never_leasable() {
    for raw in [0u32, 1, 2, u32::MAX, GUEST_CID_BASE - 1, GUEST_CID_BASE + GUEST_CID_SPAN] {
        assert_eq!(GuestCid::new(raw), Err(GuestCidError::OutOfRange { raw }), "CID {raw}");
    }
    let last = GUEST_CID_BASE + GUEST_CID_SPAN - 1;
    assert_eq!(GuestCid::new(last).expect("last leased CID").get(), last);
    for text in ["0x1000000", "", "-1", "16777216 ", "1e7", "sixteen"] {
        assert_eq!(text.parse::<GuestCid>(), Err(GuestCidError::Unparseable), "{text:?}");
    }
}

/// The span holds one CID per offset of the node's /16 guest prefix.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_cid_span_holds_one_cid_per_guest_prefix_address() {
    assert_eq!(GUEST_CID_BASE, 0x0100_0000);
    assert_eq!(GUEST_CID_SPAN, 65_536);
    assert_eq!(u64::from(GUEST_CID_SPAN), 1u64 << (32 - GUEST_NODE_PREFIX.prefix_len()));
}

// ===========================================================================
// ATTR-TOKEN — LaunchToken
// ===========================================================================

proptest! {
    /// A non-zero token round-trips through its bytes, its 32-character
    /// lowercase hex form and serde.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn launch_token_round_trips(bytes in proptest::array::uniform16(any::<u8>())) {
        prop_assume!(bytes != [0u8; 16]);
        let token = LaunchToken::from_bytes(bytes).expect("non-zero bytes accepted");
        prop_assert_eq!(token.as_bytes(), &bytes);

        let rendered = token.to_string();
        prop_assert_eq!(rendered.len(), 32);
        prop_assert!(rendered.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
        prop_assert_eq!(rendered.parse::<LaunchToken>().expect("re-parses"), token);

        let json = serde_json::to_string(&token).expect("serialize");
        prop_assert_eq!(serde_json::from_str::<LaunchToken>(&json).expect("deserialize"), token);
    }

    /// The pool draws tokens only from the injected entropy port: the same
    /// seed yields the same token, and a drawn token is never all zero.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn a_launch_token_is_drawn_from_the_injected_entropy(seed in any::<u64>()) {
        let first = token(seed);
        let again = token(seed);
        prop_assert_eq!(first, again, "drawn only from the injected entropy");
        prop_assert_ne!(first.as_bytes(), &[0u8; 16]);
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn launch_token_rejects_all_zero_upper_case_and_wrong_length() {
    assert_eq!(LaunchToken::from_bytes([0u8; 16]), Err(LaunchTokenError::Zero));
    assert_eq!("0".repeat(32).parse::<LaunchToken>(), Err(LaunchTokenError::Zero));
    for text in [
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned(),
        "0102030405060708090A0b0c0d0e0f10".to_owned(),
        "a".repeat(31),
        "a".repeat(33),
        "g".repeat(32),
        String::new(),
    ] {
        assert_eq!(text.parse::<LaunchToken>(), Err(LaunchTokenError::Unparseable), "{text:?}");
    }
    assert!(serde_json::from_str::<LaunchToken>(&format!("\"{}\"", "0".repeat(32))).is_err());
}

// ===========================================================================
// CID-NOCLAIM / ATTR-TOKEN — the allocation → driver handoff
// ===========================================================================

/// The handoff reports exactly what the pool assigned: the workload address,
/// the node gateway (the guest's resolver and the peer of every host-opened
/// flow, ADR-0174), the leased CID and the launch token. No CID claim travels
/// with it (CID-NOCLAIM).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_handoff_reports_the_lease_the_pool_assigned() {
    let launch = token(0x295);
    let assignment = GuestTransportAssignment::new(
        workload_addr(),
        GUEST_NODE_GATEWAY,
        cid_at(7),
        launch,
        bound_flow_ports(),
    );

    assert_eq!(assignment.workload_addr(), workload_addr());
    assert_eq!(assignment.gateway(), GUEST_NODE_GATEWAY);
    assert_eq!(assignment.cid(), cid_at(7));
    assert_eq!(assignment.cid().get(), GUEST_CID_BASE + 7);
    assert_eq!(assignment.launch(), launch);
    assert_eq!(assignment.flow_ports(), bound_flow_ports(), "the owner's bound flow ports");
    assert_eq!(assignment.clone(), assignment, "a clone is the same handoff");
}

/// ATTR-TOKEN: a new launch of the same CID (the next lease on the offset)
/// draws a new token, so it is a different handoff; the same launch is the
/// same handoff.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn a_new_launch_of_the_same_cid_is_a_different_handoff() {
    let first = GuestTransportAssignment::new(
        workload_addr(),
        GUEST_NODE_GATEWAY,
        cid_at(3),
        token(1),
        bound_flow_ports(),
    );
    let same = GuestTransportAssignment::new(
        workload_addr(),
        GUEST_NODE_GATEWAY,
        cid_at(3),
        token(1),
        bound_flow_ports(),
    );
    let next = GuestTransportAssignment::new(
        workload_addr(),
        GUEST_NODE_GATEWAY,
        cid_at(3),
        token(2),
        bound_flow_ports(),
    );
    let other = GuestTransportAssignment::new(
        workload_addr(),
        GUEST_NODE_GATEWAY,
        cid_at(4),
        token(1),
        bound_flow_ports(),
    );

    assert_ne!(token(1), token(2), "two draws of distinct seeds differ");
    assert_eq!(first, same);
    assert_ne!(first, next, "same CID, next launch");
    assert_ne!(first, other, "same token, another CID");
}

// ===========================================================================
// M-5 / DG-11 — ListenPort and InterfaceIndex
// ===========================================================================

proptest! {
    /// Every non-zero port is accepted; `get` returns it; `Display` / `FromStr`
    /// / serde round-trip through the decimal form.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn listen_port_round_trips(raw in 1u16..=u16::MAX) {
        let port = ListenPort::new(raw).expect("a non-zero port is accepted");
        prop_assert_eq!(port.get(), raw);
        prop_assert_eq!(port.to_string(), raw.to_string(), "Display is decimal");
        prop_assert_eq!(raw.to_string().parse::<ListenPort>(), Ok(port));
        let json = serde_json::to_string(&port).expect("serialize");
        prop_assert_eq!(serde_json::from_str::<ListenPort>(&json).expect("deserialize"), port);
    }

    /// Every non-zero index is accepted; `get` returns it; `Display` /
    /// `FromStr` / serde round-trip.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn interface_index_round_trips(raw in 1u32..=u32::MAX) {
        let index = InterfaceIndex::new(raw).expect("a non-zero index is accepted");
        prop_assert_eq!(index.get(), raw);
        prop_assert_eq!(index.to_string(), raw.to_string(), "Display is decimal");
        prop_assert_eq!(raw.to_string().parse::<InterfaceIndex>(), Ok(index));
        let json = serde_json::to_string(&index).expect("serialize");
        prop_assert_eq!(serde_json::from_str::<InterfaceIndex>(&json).expect("deserialize"), index);
    }

    /// Ordering follows the numeric value (the dial's admitted port set is
    /// the plan's ascending `listen_ports()`).
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn listen_ports_order_by_value(a in 1u16..=u16::MAX, b in 1u16..=u16::MAX) {
        let pa = ListenPort::new(a).expect("non-zero");
        let pb = ListenPort::new(b).expect("non-zero");
        prop_assert_eq!(pa.cmp(&pb), a.cmp(&b));
    }

    /// Text that is not a decimal integer in range is `Unparseable`, never
    /// `Zero` and never accepted.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn non_decimal_text_is_unparseable(text in "[0-9]{0,3}[^0-9+][^0-9]{0,4}[0-9]{0,3}") {
        prop_assert_eq!(text.parse::<ListenPort>(), Err(ListenPortError::Unparseable));
        prop_assert_eq!(text.parse::<InterfaceIndex>(), Err(InterfaceIndexError::Unparseable));
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn zero_is_refused_zero_by_the_constructors_from_str_and_serde() {
    assert_eq!(ListenPort::new(0), Err(ListenPortError::Zero));
    assert_eq!(InterfaceIndex::new(0), Err(InterfaceIndexError::Zero));
    assert_eq!("0".parse::<ListenPort>(), Err(ListenPortError::Zero));
    assert_eq!("0".parse::<InterfaceIndex>(), Err(InterfaceIndexError::Zero));
    assert!(serde_json::from_str::<ListenPort>("\"0\"").is_err(), "serde refuses port 0");
    assert!(serde_json::from_str::<InterfaceIndex>("\"0\"").is_err(), "serde refuses index 0");
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn out_of_range_and_malformed_text_is_unparseable() {
    for text in ["", "65536", "-1", "eighty", "0x50", " 80", "80 "] {
        assert_eq!(text.parse::<ListenPort>(), Err(ListenPortError::Unparseable), "{text:?}");
    }
    for text in ["", "4294967296", "-1", "lo", "0x1", " 1", "1 "] {
        assert_eq!(
            text.parse::<InterfaceIndex>(),
            Err(InterfaceIndexError::Unparseable),
            "{text:?}"
        );
    }
    assert!(serde_json::from_str::<ListenPort>("\"65536\"").is_err(), "serde refuses 65536");
}

/// The refusal texts are the pinned DG-11 messages.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn listen_port_and_interface_index_errors_render_their_pinned_messages() {
    assert_eq!(ListenPortError::Zero.to_string(), "listen port 0 is not a port");
    assert_eq!(
        ListenPortError::Unparseable.to_string(),
        "listen port is not a decimal integer in 1..=65535"
    );
    assert_eq!(InterfaceIndexError::Zero.to_string(), "interface index 0 names no interface");
    assert_eq!(
        InterfaceIndexError::Unparseable.to_string(),
        "interface index is not a decimal integer in 1..=4294967295"
    );
    assert_eq!(
        GuestCidError::OutOfRange { raw: 3 }.to_string(),
        "guest CID 3 is outside the leased range"
    );
    assert_eq!(GuestCidError::Unparseable.to_string(), "guest CID is not a decimal integer");
    assert_eq!(LaunchTokenError::Zero.to_string(), "launch token is all zero");
    assert_eq!(
        LaunchTokenError::Unparseable.to_string(),
        "launch token is not 32 lowercase hex characters"
    );
}

// ===========================================================================
// The vsock ports, the guest intake constants (D25) and the owner rules
// ===========================================================================

/// The pinned vsock ports: three distinct transport ports beside the
/// unchanged beacon port. A datagram association is one STREAM on 1240
/// (EVMOD-DGRAM); no SEQPACKET port is pinned.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_transport_vsock_ports_carry_their_pinned_values() {
    assert_eq!(GUEST_FLOW_STREAM_PORT, VsockPort::new(1240));
    assert_eq!(GUEST_INBOUND_PORT, VsockPort::new(1242));
    assert_eq!(GUEST_FLOW_CONTROL_PORT, VsockPort::new(1243));
    assert_eq!(BEACON_VSOCK_PORT, VsockPort::new(1234));
    let ports: BTreeSet<u32> =
        [GUEST_FLOW_STREAM_PORT, GUEST_INBOUND_PORT, GUEST_FLOW_CONTROL_PORT, BEACON_VSOCK_PORT]
            .map(VsockPort::as_u32)
            .into_iter()
            .collect();
    assert_eq!(ports.len(), 4, "no two listeners share a port");
}

/// The intake constants of D25, the guest's pending-TcpConnect bound
/// (EVMOD-PAIR) and the guest owner's audit tick (DG-5-A).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_guest_intake_constants_carry_their_pinned_values() {
    assert_eq!(GUEST_TCP_INTAKE_PORT, 61_000);
    assert_eq!(GUEST_UDP_SLOT_BASE, 61_001);
    assert_eq!(GUEST_UDP_SLOTS, 1_024);
    assert_eq!(GUEST_TCP_PENDING_MAX, 4_096);
    assert_eq!(GUEST_DATAGRAM_SLOT_IDLE, Duration::from_secs(120));
    assert_eq!(GUEST_OWNER_AUDIT_PERIOD, Duration::from_secs(1));
}

/// D-UP-12 / PD-4 / EVMOD-AB: the per-allocation flow quota and the probe
/// cap are one SSOT in core; the quota is a bulkhead inside the node pair
/// capacity, and the probe cap — the most quota-exempt probe dials of one
/// allocation — is far below both.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_quota_and_the_probe_cap_carry_their_pinned_values() {
    assert_eq!(GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION, 4_096);
    assert_eq!(GUEST_PROBE_SLOTS, 8);
    const { assert!(GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION < GUEST_FLOW_MAX_PAIRS) };
    const { assert!(GUEST_PROBE_SLOTS < GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION) };
}

/// EVMOD-LOAD: `overdrive-init` loads its guest kit's forwarder module from
/// one fixed path in the guest root filesystem.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_guest_kit_module_path_is_the_pinned_absolute_path() {
    assert_eq!(GUEST_FORWARDER_MODULE_PATH, "/lib/overdrive/ovd_fwd.ko");
    assert!(std::path::Path::new(GUEST_FORWARDER_MODULE_PATH).is_absolute());
}

/// D25: the reserved range holds the TCP intake and every UDP slot, above the
/// guest kernel's default ephemeral range (32,768–60,999).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_reserved_range_holds_the_tcp_intake_and_every_udp_slot_above_the_ephemeral_range() {
    let range = guest_reserved_ports();
    assert_eq!(*range.start(), GUEST_TCP_INTAKE_PORT);
    assert_eq!(*range.end(), GUEST_UDP_SLOT_BASE + GUEST_UDP_SLOTS - 1);
    assert_eq!((*range.start(), *range.end()), (61_000, 62_024));
    for n in 0..GUEST_UDP_SLOTS {
        assert!(range.contains(&(GUEST_UDP_SLOT_BASE + n)), "slot {n}");
    }
    assert!(!range.contains(&60_999), "the guest kernel's ephemeral range ends at 60,999");
    assert!(!range.contains(&62_025));
}
