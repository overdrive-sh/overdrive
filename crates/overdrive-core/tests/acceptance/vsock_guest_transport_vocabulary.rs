//! P1 — guest-transport vocabulary in `overdrive-core` (GH #295, vsock
//! attachment replacement).
//!
//! Spec: `docs/feature/netns-density-295/distill/test-scenarios-p1-admission-cid-lease.md`
//! (P1-S1, P1-S6 core half, P1-S8/S10/S11 pure parts). Contract:
//! `docs/feature/netns-density-295/feature-delta.md` § "[REF] Core vocabulary —
//! `overdrive-core`" — `GuestCid`, `LaunchToken`, `GuestTransportAssignment`,
//! `ClaimedGuestCid`, the intake constants (D25) and the intake host-port rule
//! (INTAKE-BIND, INTAKE-PORTS), and the M-5 newtypes `ListenPort` /
//! `InterfaceIndex` in `overdrive_core::net`.
//!
//! Driving ports: the public constructors, codecs and functions of
//! `overdrive_core::guest_transport` / `overdrive_core::net` (layer 1, pure:
//! `proptest` full, Mandate 9). The CID-claim port is a driven port here:
//! `SimGuestCidClaim` supplies real claims and its pinned arbiter accessor
//! `held_cids()` (QPROJ-7) is the observation of "the kernel holds this CID",
//! so the take-once / release tests observe the production
//! `GuestTransportAssignment` / `ClaimedGuestCid` through a port, never the sim
//! as the system under test.
//!
//! Not here: the stateful `GuestAddressPool::assign` scenarios (P1-S2..S9,
//! P1-S13). The pool lives in `overdrive-control-plane` and its pinned
//! signatures replace live ones (`docs/feature/netns-density-295/distill/upstream-issues.md`
//! UI-C1).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    reason = "test bodies unwrap fixtures they construct; the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

use overdrive_core::guest_transport::{
    ClaimedGuestCid, GUEST_CID_BASE, GUEST_CID_PROBE, GUEST_CID_SPAN, GUEST_INTAKE_HOST_PORT_BASE,
    GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX, GUEST_TCP_INTAKE_PORT, GUEST_UDP_SLOT_BASE,
    GUEST_UDP_SLOTS, GuestCid, GuestCidClaim, GuestCidError, GuestTransportAssignment, LaunchToken,
    LaunchTokenError, guest_intake_host_port, guest_intake_host_ports, guest_reserved_ports,
};
use overdrive_core::net::{InterfaceIndex, InterfaceIndexError, ListenPort, ListenPortError};
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_cid_claim::SimGuestCidClaim;
use proptest::prelude::*;

const fn workload_addr() -> Ipv4Addr {
    Ipv4Addr::new(100, 95, 0, 7)
}

const fn gateway() -> Ipv4Addr {
    Ipv4Addr::new(100, 95, 0, 1)
}

fn cid_at(offset: u32) -> GuestCid {
    GuestCid::new(GUEST_CID_BASE + offset).expect("an in-range CID")
}

/// A claim held through the CID-claim port, and the port's view of it.
fn claimed(port: &SimGuestCidClaim, offset: u32) -> ClaimedGuestCid {
    port.claim(cid_at(offset)).expect("an unheld CID is claimable")
}

fn held(port: &SimGuestCidClaim) -> BTreeSet<GuestCid> {
    port.held_cids()
}

// ===========================================================================
// P1-S1 — GuestCid: the leased range, its derivation and newtype completeness
// ===========================================================================

proptest! {
    /// Every offset of the leased span is a CID equal to `GUEST_CID_BASE +
    /// offset`; its decimal `Display` re-parses through `FromStr`, and serde
    /// round-trips through the same form.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    fn every_leased_offset_is_a_cid_that_round_trips(offset in 0u32..GUEST_CID_SPAN) {
        let raw = GUEST_CID_BASE + offset;
        let cid = GuestCid::new(raw).expect("in-range CID accepted");
        prop_assert_eq!(cid.get(), raw);

        let rendered = cid.to_string();
        prop_assert_eq!(&rendered, &raw.to_string(), "Display is decimal");
        prop_assert_eq!(rendered.parse::<GuestCid>().expect("re-parses"), cid);

        let json = serde_json::to_string(&cid).expect("serialize");
        prop_assert_eq!(serde_json::from_str::<GuestCid>(&json).expect("deserialize"), cid);
    }

    /// Every value outside the leased span is refused `OutOfRange`, by the
    /// constructor and by `FromStr`.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    fn every_value_outside_the_span_is_out_of_range(
        raw in prop_oneof![0u32..GUEST_CID_BASE, (GUEST_CID_BASE + GUEST_CID_SPAN)..=u32::MAX],
    ) {
        prop_assert_eq!(GuestCid::new(raw), Err(GuestCidError::OutOfRange { raw }));
        prop_assert_eq!(raw.to_string().parse::<GuestCid>(), Err(GuestCidError::OutOfRange { raw }));
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
fn the_kernel_reserved_cids_and_the_probe_cid_are_never_leasable() {
    for raw in [0u32, 1, 2, u32::MAX, GUEST_CID_PROBE] {
        assert_eq!(GuestCid::new(raw), Err(GuestCidError::OutOfRange { raw }), "CID {raw}");
    }
    // The span's last CID is leasable; the next one is not.
    let last = GUEST_CID_BASE + GUEST_CID_SPAN - 1;
    assert_eq!(GuestCid::new(last).expect("last leased CID").get(), last);
    for text in ["0x100", "", "-1", "16777216 ", "1e7"] {
        assert_eq!(text.parse::<GuestCid>(), Err(GuestCidError::Unparseable), "{text:?}");
    }
}

// ===========================================================================
// ATTR-TOKEN — LaunchToken
// ===========================================================================

proptest! {
    /// A non-zero token round-trips through its bytes, its 32-character
    /// lowercase hex form and serde.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
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
    #[ignore = "pending DELIVER vsock attachment cut (new-step 15, launch token drawn with the claim)"]
    fn a_launch_token_is_drawn_from_the_injected_entropy(seed in any::<u64>()) {
        let first = LaunchToken::generate(&SimEntropy::new(seed));
        let again = LaunchToken::generate(&SimEntropy::new(seed));
        prop_assert_eq!(first, again, "drawn only from the injected entropy");
        prop_assert_ne!(first.as_bytes(), &[0u8; 16]);
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
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
}

// ===========================================================================
// P1-S6 (core half) / R5-19(d) — the claim reaches the VMM exactly once
// ===========================================================================

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 15, move-only claim handle in the transport handoff)"]
fn concurrent_clones_take_the_claim_exactly_once() {
    let port = SimGuestCidClaim::new();
    let token = LaunchToken::generate(&SimEntropy::new(0x295));
    let assignment =
        GuestTransportAssignment::new(workload_addr(), gateway(), token, claimed(&port, 11));
    let clones: Vec<GuestTransportAssignment> =
        std::iter::repeat_with(|| assignment.clone()).take(16).collect();

    let winners: Vec<ClaimedGuestCid> = std::thread::scope(|scope| {
        // Spawn every racer before joining any, so the takes overlap.
        #[allow(clippy::needless_collect, reason = "all racers must start before the first join")]
        let racers: Vec<_> =
            clones.iter().map(|clone| scope.spawn(|| clone.take_claim())).collect();
        racers.into_iter().filter_map(|racer| racer.join().expect("racer")).collect()
    });

    assert_eq!(winners.len(), 1, "exactly one clone receives the claim");
    assert!(assignment.take_claim().is_none(), "every later call returns None");
    assert_eq!(winners[0].cid(), cid_at(11));
    assert!(held(&port).contains(&cid_at(11)), "the taken claim still holds the CID");
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 15, move-only claim handle in the transport handoff)"]
fn the_assignment_reports_its_claim_and_equality_ignores_the_slot() {
    let port = SimGuestCidClaim::new();
    let token = LaunchToken::generate(&SimEntropy::new(7));
    let assignment =
        GuestTransportAssignment::new(workload_addr(), gateway(), token, claimed(&port, 3));
    let clone = assignment.clone();

    assert_eq!(assignment.workload_addr(), workload_addr());
    assert_eq!(assignment.gateway(), gateway());
    assert_eq!(assignment.launch(), token);
    assert_eq!(assignment.cid(), cid_at(3));

    let claim = clone.take_claim().expect("the claim is in the slot");
    // The CID stays readable after the claim left the slot, and a clone whose
    // slot is now empty still equals the original.
    assert_eq!(assignment.cid(), cid_at(3));
    assert_eq!(assignment, clone);
    drop(claim);

    let other_token = LaunchToken::generate(&SimEntropy::new(8));
    let other =
        GuestTransportAssignment::new(workload_addr(), gateway(), other_token, claimed(&port, 3));
    assert_ne!(assignment, other, "a different launch of the same CID is a different assignment");
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 15, move-only claim handle in the transport handoff)"]
#[allow(
    clippy::drop_non_drop,
    reason = "the RED scaffold has no Drop; the production assignment owns the take-once claim slot"
)]
fn an_untaken_claim_is_released_when_the_last_clone_drops() {
    let port = SimGuestCidClaim::new();
    let token = LaunchToken::generate(&SimEntropy::new(1));
    let assignment =
        GuestTransportAssignment::new(workload_addr(), gateway(), token, claimed(&port, 4));
    let clone = assignment.clone();
    assert_eq!(held(&port), BTreeSet::from([cid_at(4)]));

    drop(assignment);
    assert_eq!(held(&port), BTreeSet::from([cid_at(4)]), "a clone still owns the slot");
    drop(clone);
    assert!(held(&port).is_empty(), "the kernel frees the CID with the untaken claim");
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 15, move-only claim handle in the transport handoff)"]
#[allow(
    clippy::drop_non_drop,
    reason = "the RED scaffold has no Drop; the production assignment owns the take-once claim slot"
)]
fn a_claim_handed_to_the_vmm_holds_the_cid_until_the_device_drops() {
    let port = SimGuestCidClaim::new();
    let token = LaunchToken::generate(&SimEntropy::new(2));
    let assignment =
        GuestTransportAssignment::new(workload_addr(), gateway(), token, claimed(&port, 5));

    let device = assignment.take_claim().expect("first take").into_device();
    drop(assignment);
    assert_eq!(held(&port), BTreeSet::from([cid_at(5)]), "the VMM's device holds the CID");
    assert!(device.vhost_fd().is_none(), "a simulated claim carries no descriptor");

    drop(device); // the VMM exits
    assert!(held(&port).is_empty(), "the VMM's exit is the release");
}

// ===========================================================================
// M-5 / DG-11 — ListenPort and InterfaceIndex: validating constructors,
// accessors, and newtype completeness
// ===========================================================================

proptest! {
    /// Every non-zero port is accepted by the constructor and `get` returns it;
    /// the decimal `Display` re-parses through `FromStr` to the same value, and
    /// serde round-trips through the same form.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    fn listen_port_round_trips(raw in 1u16..=u16::MAX) {
        let port = ListenPort::new(raw).expect("a non-zero port is accepted");
        prop_assert_eq!(port.get(), raw);
        prop_assert_eq!(port.to_string(), raw.to_string(), "Display is decimal");
        prop_assert_eq!(raw.to_string().parse::<ListenPort>(), Ok(port));
        let json = serde_json::to_string(&port).expect("serialize");
        prop_assert_eq!(serde_json::from_str::<ListenPort>(&json).expect("deserialize"), port);
    }

    /// Every non-zero index is accepted by the constructor and `get` returns
    /// it; `Display` / `FromStr` / serde round-trip.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    fn interface_index_round_trips(raw in 1u32..=u32::MAX) {
        let index = InterfaceIndex::new(raw).expect("a non-zero index is accepted");
        prop_assert_eq!(index.get(), raw);
        prop_assert_eq!(index.to_string(), raw.to_string(), "Display is decimal");
        prop_assert_eq!(raw.to_string().parse::<InterfaceIndex>(), Ok(index));
        let json = serde_json::to_string(&index).expect("serialize");
        prop_assert_eq!(serde_json::from_str::<InterfaceIndex>(&json).expect("deserialize"), index);
    }

    /// Ordering follows the numeric value (the observation sets of QPROJ-3
    /// are `BTreeSet<ListenPort>`).
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    fn listen_ports_order_by_value(a in 1u16..=u16::MAX, b in 1u16..=u16::MAX) {
        let pa = ListenPort::new(a).expect("non-zero");
        let pb = ListenPort::new(b).expect("non-zero");
        prop_assert_eq!(pa.cmp(&pb), a.cmp(&b));
    }

    /// Text that is not a decimal integer in range is `Unparseable`, never
    /// `Zero` and never accepted.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
    fn non_decimal_text_is_unparseable(text in "[0-9]{0,3}[^0-9+][^0-9]{0,4}[0-9]{0,3}") {
        prop_assert_eq!(text.parse::<ListenPort>(), Err(ListenPortError::Unparseable));
        prop_assert_eq!(text.parse::<InterfaceIndex>(), Err(InterfaceIndexError::Unparseable));
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
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
#[ignore = "pending DELIVER vsock new-step 2 (core types and wire codec)"]
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
}

// ===========================================================================
// D25 — the reserved guest port range; INTAKE-BIND / INTAKE-PORTS — the
// intake host-port block (P1-S10/S11 pure parts, R5-42, R5-44)
// ===========================================================================

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest intakes, D25)"]
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

proptest! {
    /// The intake host port of index `i` is `GUEST_INTAKE_HOST_PORT_BASE + i`,
    /// lies in the block, and distinct indices never share a port.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 9 (intake host ports, INTAKE-BIND)"]
    fn intake_host_ports_are_injective_inside_the_block(
        i in 0u16..GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX,
        j in 0u16..GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX,
    ) {
        let pi = guest_intake_host_port(i).expect("an index inside the block maps");
        prop_assert_eq!(pi, GUEST_INTAKE_HOST_PORT_BASE + i);
        prop_assert!(guest_intake_host_ports().contains(&pi));
        let pj = guest_intake_host_port(j).expect("an index inside the block maps");
        prop_assert_eq!(pi == pj, i == j, "injective");
    }

    /// Indices at or beyond the block size map to nothing.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 9 (intake host ports, INTAKE-BIND)"]
    fn indices_beyond_the_block_map_to_nothing(i in GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX..=u16::MAX) {
        prop_assert_eq!(guest_intake_host_port(i), None);
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 9 (intake host ports, INTAKE-BIND)"]
fn the_intake_host_block_sits_below_the_host_ephemeral_range_and_apart_from_the_guest_reservation()
{
    let block = guest_intake_host_ports();
    assert_eq!((*block.start(), *block.end()), (20_480, 24_575));
    assert_eq!(
        u32::from(*block.end()) - u32::from(*block.start()) + 1,
        u32::from(GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX),
        "one host port per declarable TCP listener port",
    );
    assert!(*block.end() < 32_768, "below the kernel's default ephemeral range");
    let reserved = guest_reserved_ports();
    assert!(*block.end() < *reserved.start(), "disjoint from the guest reservation");
}
