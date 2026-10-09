//! P1 — vsock admission vocabulary & CID-claim port contract (overdrive-core).
//!
//! DISTILL acceptance scaffolds for feature #295 (netns-density, vsock
//! attachment replacement). Spec: `docs/feature/netns-density-295/distill/
//! test-scenarios-p1-admission-cid-lease.md`; design pins:
//! feature-delta.md §"[REF] Core vocabulary — overdrive-core" (L17449+),
//! decisions D16 / D16-CLAIM / D8a-CAP / INTAKE-PORTS, gate G-V0, R5-19 /
//! R5-37 / R5-44.
//!
//! SCOPE OF THIS FILE (overdrive-core only). It exercises the pieces of the
//! admission contract that live in `overdrive_core::guest_transport` + the
//! driven CID-claim port, reachable from this crate:
//!   * the `GuestCid` / `LaunchToken` newtypes (completeness proptest — the
//!     mandatory newtype call site, `.claude/rules/testing.md` §PBT);
//!   * the pure CID-derivation identity (D16);
//!   * the `GuestCidClaim` / `ClaimedGuestCid` / `ClaimedVsockDevice` trait
//!     contract and `GuestCidClaim::probe`, driven through the sim adapter
//!     `overdrive_sim::adapters::guest_cid_claim::SimGuestCidClaim`
//!     (overdrive-sim is a dev-dep of overdrive-core — dev-dep back-edge);
//!   * `GuestTransportAssignment::take_claim` take-once atomicity (R5-19d, the
//!     overdrive-core half);
//!   * the pure intake host-port / reserved-range / capacity functions
//!     (D8a-CAP, INTAKE-PORTS; S8/S10/S11 pure parts).
//!
//! NOT IN THIS FILE — surfaced as a testability-placement gap: the STATEFUL
//! `GuestAddressPool::assign` scenarios (P1-S2..S8 next-fit skip on `InUse`,
//! `GuestCidsHeldElsewhere` aggregation, reservation state, retire/release)
//! need the production pool, which lives in
//! `overdrive_control_plane::guest_network` (its `assign` is `pub(crate)`,
//! reached cross-crate via the `#[cfg(feature="integration-tests")]`
//! `e18_test_support` wrapper). overdrive-core does NOT depend on
//! overdrive-control-plane, so those bodies belong in
//! `crates/overdrive-control-plane/tests/acceptance/vsock_guest_address_pool.rs`
//! (NOT in P1's allowed file set) and in the Quint conformance driver
//! (`vsock_cid_lease_conformance.rs`, which CAN reach both). See the doc's
//! "Placement gaps" section.
//!
//! RED / gating: every production symbol named below is pinned by the design
//! but does NOT exist yet — `overdrive_core::guest_transport` is a new module
//! created by the DELIVER roadmap reconciliation + the admission step; the
//! `SimGuestCidClaim` adapter (P1, `overdrive-sim`) lands with it. This module
//! is intentionally NOT registered in `tests/acceptance.rs`; the crafter adds
//! its `mod vsock_guest_address_pool;` line when the body co-arrives with that
//! production surface (`.claude/rules/testing.md` §"RED acceptance scaffolds
//! and activation"). No production API is invented here to make it compile.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use overdrive_core::guest_transport::{
    ClaimedGuestCid, ClaimedVsockDevice, GUEST_CID_BASE, GUEST_CID_PROBE, GUEST_CID_SPAN,
    GUEST_INTAKE_HOST_PORT_BASE, GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX, GUEST_INTAKE_STEERING_MAX,
    GUEST_TCP_INTAKE_PORT, GUEST_UDP_SLOT_BASE, GUEST_UDP_SLOTS, GuestCid, GuestCidClaim,
    GuestCidClaimError, GuestCidClaimProbeStage, GuestCidError, GuestTransportAssignment,
    LaunchToken, LaunchTokenError, guest_intake_host_port, guest_intake_host_ports,
    guest_reserved_ports,
};
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_cid_claim::SimGuestCidClaim;
use proptest::prelude::*;

const DELIVER_STEP: &str = "pending DELIVER: vsock admission step \
    (overdrive_core::guest_transport vocabulary + GuestCidClaim port + SimGuestCidClaim)";

// A gateway and workload address only matter structurally to the assignment
// equality/take-claim scenarios below; concrete values are domain-arbitrary.
fn wl_addr() -> std::net::Ipv4Addr {
    std::net::Ipv4Addr::new(100, 95, 0, 7)
}
fn gw_addr() -> std::net::Ipv4Addr {
    std::net::Ipv4Addr::new(100, 95, 0, 1)
}

// =====================================================================
// GuestCid — newtype completeness + derivation (D16). Pure overdrive-core.
// =====================================================================

proptest! {
    // P1-S1 (derivation half) + newtype-roundtrip mandate.
    // Contract: GuestCid::new accepts exactly GUEST_CID_BASE ..
    // GUEST_CID_BASE+GUEST_CID_SPAN; Display(decimal)/FromStr/serde round-trip
    // bit-equivalent; CID derivation is `GUEST_CID_BASE + offset`.
    #[test]
    #[ignore = "pending DELIVER: vsock admission step (overdrive_core::guest_transport vocabulary + GuestCidClaim port + SimGuestCidClaim)"]
    fn gcid_roundtrips_and_derivation_is_base_plus_offset(offset in 0u32..GUEST_CID_SPAN) {
        let raw = GUEST_CID_BASE + offset;
        let cid = GuestCid::new(raw).expect("in-range CID accepted");
        prop_assert_eq!(cid.get(), raw, "derivation is base + offset, computed once");

        // Display is decimal; FromStr is its exact inverse.
        let rendered = cid.to_string();
        prop_assert_eq!(rendered.parse::<GuestCid>().expect("round-trips"), cid);

        // serde round-trips through the same decimal form.
        let json = serde_json::to_string(&cid).expect("serialize");
        let back: GuestCid = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(back, cid);
    }
}

#[test]
#[ignore = "pending DELIVER: vsock admission step (overdrive_core::guest_transport vocabulary + GuestCidClaim port + SimGuestCidClaim)"]
fn gcid_rejects_out_of_range_and_reserved() {
    // Edge cases the design enumerates (feature-delta "GuestCid::new" contract):
    // kernel-reserved 0/1/2, u32::MAX, and the probe CID (BASE-1) are refused.
    for raw in [0u32, 1, 2, u32::MAX, GUEST_CID_PROBE] {
        assert!(
            matches!(GuestCid::new(raw), Err(GuestCidError::OutOfRange { raw: r }) if r == raw),
            "CID {raw} must be OutOfRange",
        );
    }
    // One past the top of the span is also out of range.
    assert!(matches!(
        GuestCid::new(GUEST_CID_BASE + GUEST_CID_SPAN),
        Err(GuestCidError::OutOfRange { .. })
    ));
    // Non-decimal text is Unparseable.
    assert!(matches!("0x100".parse::<GuestCid>(), Err(GuestCidError::Unparseable)));
}

// =====================================================================
// LaunchToken — newtype completeness (ATTR-TOKEN). Pure overdrive-core.
// =====================================================================

proptest! {
    // ATTR-TOKEN newtype roundtrip: 32 lowercase hex, case-sensitive; serde
    // matches Display/FromStr; all-zero and malformed are rejected.
    #[test]
    #[ignore = "pending DELIVER: vsock admission step (overdrive_core::guest_transport vocabulary + GuestCidClaim port + SimGuestCidClaim)"]
    fn launch_token_roundtrips(bytes in proptest::array::uniform16(any::<u8>())) {
        prop_assume!(bytes != [0u8; 16]); // all-zero is the invalid encoding
        let token = LaunchToken::from_bytes(bytes).expect("non-zero accepted");
        let rendered = token.to_string();
        prop_assert_eq!(rendered.len(), 32);
        prop_assert!(rendered.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        prop_assert_eq!(rendered.parse::<LaunchToken>().expect("round-trips"), token);
        let json = serde_json::to_string(&token).expect("serialize");
        prop_assert_eq!(serde_json::from_str::<LaunchToken>(&json).expect("deserialize"), token);
    }
}

#[test]
#[ignore = "pending DELIVER: vsock admission step (overdrive_core::guest_transport vocabulary + GuestCidClaim port + SimGuestCidClaim)"]
fn launch_token_rejects_zero_upper_and_wrong_length() {
    assert!(matches!(LaunchToken::from_bytes([0u8; 16]), Err(LaunchTokenError::Zero)));
    // uppercase hex is rejected (machine-generated, never typed).
    assert!(matches!(
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".parse::<LaunchToken>(),
        Err(LaunchTokenError::Unparseable)
    ));
    // wrong length rejected.
    assert!(matches!("abcd".parse::<LaunchToken>(), Err(LaunchTokenError::Unparseable)));
    // 32 lowercase zero-hex decodes to the all-zero bytes, which are invalid.
    assert!(matches!(
        "00000000000000000000000000000000".parse::<LaunchToken>(),
        Err(LaunchTokenError::Zero)
    ));
}

#[test]
#[ignore = "pending DELIVER: vsock admission step (overdrive_core::guest_transport vocabulary + GuestCidClaim port + SimGuestCidClaim)"]
fn launch_token_generate_is_nonzero_from_injected_entropy() {
    // The pool draws the token from the injected Entropy port; a seeded sim
    // entropy yields a usable (non-zero) token deterministically.
    let entropy = SimEntropy::with_seed(0x295);
    let token = LaunchToken::generate(&entropy);
    assert_ne!(token.as_bytes(), &[0u8; 16], "generate redraws until non-zero");
}

// =====================================================================
// GuestCidClaim / ClaimedGuestCid / probe — the driven-port contract
// (D16-CLAIM, CLAIM-PINS), driven through SimGuestCidClaim.
// Universe (observable surface): the claim set's holder table + the returned
// ClaimedGuestCid.cid(); asserted explicitly (Rust-idiomatic state-delta —
// no Python state_delta port in this codebase, per .claude/rules/testing.md).
// =====================================================================

#[test]
#[ignore = "pending DELIVER: vsock admission step (overdrive_core::guest_transport vocabulary + GuestCidClaim port + SimGuestCidClaim)"]
fn claim_is_exclusive_and_drop_releases() {
    // P1-S3 / R5-19a (claim-port level). A claim holds the CID; a second claim
    // of the same CID while the first is open is InUse; dropping frees it.
    let claimset = Arc::new(SimGuestCidClaim::new());
    let cid = GuestCid::new(GUEST_CID_BASE + 5).unwrap();

    let held = claimset.claim(cid).expect("first claim succeeds");
    assert_eq!(held.cid(), cid);
    assert!(matches!(claimset.claim(cid), Err(GuestCidClaimError::InUse { cid: c }) if c == cid));

    drop(held); // last reference closes -> CID released
    let reclaimed = claimset.claim(cid).expect("claimable after release");
    assert_eq!(reclaimed.cid(), cid);
}

#[test]
#[ignore = "pending DELIVER: vsock admission step (overdrive_core::guest_transport vocabulary + GuestCidClaim port + SimGuestCidClaim)"]
fn non_inuse_claim_failure_is_kernel_not_inuse() {
    // P1-S5 / R5-19c (claim-port level). A scripted non-EADDRINUSE failure
    // surfaces as a `Kernel`/`DeviceUnavailable` error — never `InUse` — so the
    // pool does NOT skip the offset on it.
    let claimset = SimGuestCidClaim::new();
    let cid = GuestCid::new(GUEST_CID_BASE + 9).unwrap();
    claimset.script_claim_error(cid); // next claim of `cid` fails non-InUse
    match claimset.claim(cid) {
        Err(GuestCidClaimError::Kernel { cid: c, .. }) => assert_eq!(c, cid),
        other => panic!("expected a non-InUse Kernel error, got {other:?}"),
    }
}

#[test]
#[ignore = "pending DELIVER: vsock admission step (overdrive_core::guest_transport vocabulary + GuestCidClaim port + SimGuestCidClaim)"]
fn claim_probe_holds_under_the_contract() {
    // Earned-Trust probe: claim GUEST_CID_PROBE, a second claim must fail InUse,
    // release, a third must succeed, release; leaves nothing held.
    let claimset = SimGuestCidClaim::new();
    claimset.probe().expect("a faithful claim set passes the probe");
    // and a scripted deviation at a named stage is reported, not swallowed.
    let broken = SimGuestCidClaim::new();
    broken.script_probe_deviation(GuestCidClaimProbeStage::SecondClaimRefused);
    assert!(matches!(
        broken.probe(),
        Err(GuestCidClaimError::ProbeMismatch { stage })
            if stage == GuestCidClaimProbeStage::SecondClaimRefused
    ));
}

// =====================================================================
// GuestTransportAssignment::take_claim — take-once atomicity (R5-19d, core
// half). A test-local ClaimedVsockDevice builds a ClaimedGuestCid.
// =====================================================================

#[derive(Debug)]
struct TestVsockDevice;
impl ClaimedVsockDevice for TestVsockDevice {
    fn vhost_fd(&self) -> Option<std::os::fd::BorrowedFd<'_>> {
        None // a test-local claim has no real descriptor (same as a sim claim)
    }
}

#[test]
#[ignore = "pending DELIVER: vsock admission step (overdrive_core::guest_transport vocabulary + GuestCidClaim port + SimGuestCidClaim)"]
fn assignment_take_claim_yields_exactly_one_some_across_clones() {
    // R5-19d: the claim reaches the VMM exactly once. Across every clone of the
    // assignment, `take_claim` returns Some for exactly one caller; all others
    // return None. Equality ignores the claim slot.
    let cid = GuestCid::new(GUEST_CID_BASE + 11).unwrap();
    let claim = ClaimedGuestCid::new(cid, Box::new(TestVsockDevice));
    let token = LaunchToken::generate(&SimEntropy::with_seed(7));
    let assignment = GuestTransportAssignment::new(wl_addr(), gw_addr(), token, claim);

    let clones: Vec<_> = std::iter::repeat_with(|| assignment.clone()).take(8).collect();
    // Two assignments sharing the same slot are equal (claim slot excluded).
    assert_eq!(assignment, clones[0]);
    assert_eq!(assignment.cid(), cid);
    assert_eq!(assignment.launch(), token);

    let somes = clones.iter().filter(|c| c.take_claim().is_some()).count();
    // One of the clones won; the original and the rest now see None.
    assert_eq!(somes, 1, "exactly one clone takes the claim");
    assert!(assignment.take_claim().is_none(), "already taken elsewhere");
}

// =====================================================================
// Intake host-port / reserved-range / capacity — pure functions (D8a-CAP,
// INTAKE-PORTS; S8/S10/S11 pure parts).
// =====================================================================

proptest! {
    // R5-42 / R5-44 pure part: guest_intake_host_port is injective over
    // 0..PORTS_PER_ALLOCATION_MAX, lands in guest_intake_host_ports(), and is
    // None at/above the max.
    #[test]
    #[ignore = "pending DELIVER: vsock admission step (overdrive_core::guest_transport vocabulary + GuestCidClaim port + SimGuestCidClaim)"]
    fn intake_host_port_is_injective_in_block_none_beyond(
        i in 0u16..GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX,
        j in 0u16..GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX,
    ) {
        let pi = guest_intake_host_port(i).expect("index in range maps");
        prop_assert_eq!(pi, GUEST_INTAKE_HOST_PORT_BASE + i);
        prop_assert!(guest_intake_host_ports().contains(&pi));
        if i != j {
            prop_assert_ne!(pi, guest_intake_host_port(j).unwrap(), "injective");
        }
    }
}

#[test]
#[ignore = "pending DELIVER: vsock admission step (overdrive_core::guest_transport vocabulary + GuestCidClaim port + SimGuestCidClaim)"]
fn intake_host_port_none_at_and_above_max() {
    assert_eq!(guest_intake_host_port(GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX), None);
}

#[test]
#[ignore = "pending DELIVER: vsock admission step (overdrive_core::guest_transport vocabulary + GuestCidClaim port + SimGuestCidClaim)"]
fn reserved_range_is_the_intake_block_above_ephemeral() {
    // S10 pure part (D25 / D15-R2): reserved = 61_000..=62_024, above the guest
    // kernel's default ephemeral range (32,768-60,999).
    let range = guest_reserved_ports();
    assert_eq!(*range.start(), GUEST_TCP_INTAKE_PORT);
    assert_eq!(*range.end(), GUEST_UDP_SLOT_BASE + GUEST_UDP_SLOTS - 1);
    assert!(range.contains(&GUEST_TCP_INTAKE_PORT));
    assert!(*range.start() > 60_999, "reserved range sits above ephemeral");
}

proptest! {
    // S8 pure-arithmetic part (D8a-CAP): the node reservation predicate — a
    // request is refused iff it would push total reserved past the steering
    // capacity. (The STATEFUL pool version with IntakeCapacityReached lives in
    // the control-plane acceptance file + the conformance driver — see header.)
    #[test]
    #[ignore = "pending DELIVER: vsock admission step (overdrive_core::guest_transport vocabulary + GuestCidClaim port + SimGuestCidClaim)"]
    fn capacity_predicate_refuses_exactly_on_overflow(
        reserved in 0u32..=GUEST_INTAKE_STEERING_MAX,
        requested in 0u32..=GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX as u32,
    ) {
        let would_exceed = reserved + requested > GUEST_INTAKE_STEERING_MAX;
        let admits = reserved + requested <= GUEST_INTAKE_STEERING_MAX;
        prop_assert_ne!(would_exceed, admits, "admit XOR refuse at the boundary");
    }
}
