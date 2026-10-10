//! P1-S12 — Tier-3: the real host-kernel CID claim and its claim-level
//! handoff (GH #295, vsock attachment replacement).
//!
//! Spec: `docs/feature/netns-density-295/distill/test-scenarios-p1-admission-cid-lease.md`
//! (P1-S12). Design: D16-CLAIM / CLAIM-PINS (A-29), DG-6
//! (`VhostVsockCidClaim::new`, `VHOST_VSOCK_DEVICE`), boundary row R5-19, the
//! QPROJ-7 pairing of `SimGuestCidClaim::held_cids` (R5-19 (a), (b), (e)),
//! validation item V-25 (done, P-39, `spike/findings-v25.md`).
//!
//! Driving port: `overdrive_core::guest_transport::GuestCidClaim` on the
//! production adapter `overdrive_host::VhostVsockCidClaim`, over the real
//! `/dev/vhost-vsock` (no path override exists, DG-6). The universe is the
//! kernel's CID holder table, observed through a second claim of the same CID
//! (`InUse` ⇔ held) — the fact `SimGuestCidClaim::held_cids` reports for the
//! sim.
//!
//! LANE: example-based Tier-3 only (no PBT at Tier 3). Runs on the native
//! `x86_64` metal host (`cargo xtask metal run -- cargo nextest run -p
//! overdrive-host --features integration-tests,kvm-tests`), as root; no cell
//! establishes the `serve` identity. These cells boot no guest; the cells that hand the claim
//! to a VMM through `Vmm::create(&cfg, claim)` (DG-2 row 6) are authored at the
//! attachment cut, and the fork-boot cells live in the P6 walking skeleton.
//!
//! NODE-GLOBAL KERNEL STATE: the vhost-vsock CID holder table is node-global.
//! The CIDs below are offsets 100..=105 of the leased span, which no other
//! test claims; the whole `overdrive-host` integration binary is serialized in
//! the `host-kernel-shared` nextest group (`.config/nextest.toml`).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    reason = "test fixtures assert their preconditions; the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::path::Path;

use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GuestCid, GuestCidClaim, GuestCidClaimError,
};
use overdrive_host::{VHOST_VSOCK_DEVICE, VhostVsockCidClaim};

fn cid(offset: u32) -> GuestCid {
    GuestCid::new(GUEST_CID_BASE + offset).expect("an offset within the leased span")
}

/// `true` when a fresh claim of `c` is refused `InUse` (the kernel holds it
/// for another instance); `false` when the fresh claim succeeds (it is
/// released again at once). Any other outcome fails the test.
fn held_elsewhere(claimer: &VhostVsockCidClaim, c: GuestCid) -> bool {
    match claimer.claim(c) {
        Err(GuestCidClaimError::InUse { cid }) => {
            assert_eq!(cid, c, "InUse names the refused CID");
            true
        }
        Ok(fresh) => {
            assert_eq!(fresh.cid(), c);
            drop(fresh);
            false
        }
        Err(other) => panic!("a claim of {c} failed for a cause other than InUse: {other}"),
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_claim_adapter_names_the_real_device() {
    assert_eq!(VHOST_VSOCK_DEVICE, "/dev/vhost-vsock");
}

// ---------------------------------------------------------------------------
// P1-S12a / R5-19 (a), (b) — an unowned instance holds the CID exclusively
// while its reference is open.
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 15, VhostVsockCidClaim, D16-CLAIM)"]
fn claim_on_unowned_instance_holds_cid_exclusively() {
    assert!(Path::new(VHOST_VSOCK_DEVICE).exists(), "the metal host provides /dev/vhost-vsock");
    let claimer = VhostVsockCidClaim::new();
    let c = cid(100);
    assert!(!held_elsewhere(&claimer, c), "precondition: nothing holds {c}");

    let held = claimer.claim(c).expect("SET_GUEST_CID succeeds on an unowned instance");
    assert_eq!(held.cid(), c);
    assert!(
        held.into_device().vhost_fd().is_some(),
        "a host claim carries its /dev/vhost-vsock open file description",
    );
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 15, VhostVsockCidClaim, D16-CLAIM)"]
fn a_second_claim_while_the_first_is_open_is_in_use_and_holds_nothing() {
    let claimer = VhostVsockCidClaim::new();
    let c = cid(101);
    let first = claimer.claim(c).expect("first claim");

    // The kernel's EADDRINUSE surfaces as InUse, never a different variant,
    // and the refused claim holds nothing: the first still owns the CID.
    assert!(matches!(claimer.claim(c), Err(GuestCidClaimError::InUse { cid }) if cid == c));
    assert!(held_elsewhere(&claimer, c), "the first claim still holds {c}");
    assert_eq!(first.cid(), c);
}

// ---------------------------------------------------------------------------
// P1-S12 / A-29 / R5-19 (e) — released at the last reference close.
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 15, VhostVsockCidClaim, D16-CLAIM)"]
fn claim_released_at_last_reference_close() {
    let claimer = VhostVsockCidClaim::new();
    let c = cid(102);
    let held = claimer.claim(c).expect("first claim");
    assert!(held_elsewhere(&claimer, c));

    drop(held); // the last (only) reference closes

    // Re-claimable after the last close (P-39: ≤ 0.06 ms after reap; the
    // latency is reported by V-25, not hard-asserted here).
    let reclaimed = claimer.claim(c).expect("claimable after the last close");
    assert_eq!(reclaimed.cid(), c);
}

// ---------------------------------------------------------------------------
// P1-S12 — the claim probe on the real device. The test runs as root and does
// not establish the `serve` identity: the design pins no way to assume it, so
// this cell proves the probe's contract on the real `/dev/vhost-vsock`, not
// the device's permissions for `serve`.
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 15, VhostVsockCidClaim, D16-CLAIM)"]
fn claim_probe_holds_on_the_real_device() {
    // Boot Earned Trust: claim GUEST_CID_PROBE, a second claim must fail,
    // release, a third must succeed, release. On a real kernel this holds; a
    // deviation is a typed ProbeMismatch, never a silent pass.
    let claimer = VhostVsockCidClaim::new();
    claimer.probe().expect("the kernel honours the claim contract");
    // It leaves nothing held: a second probe passes as well.
    claimer.probe().expect("the probe leaves GUEST_CID_PROBE unheld");
}

// ---------------------------------------------------------------------------
// P1-S12 / R5-19 (d), claim level — the device taken for handoff holds the CID
// exactly once and refuses a fresh claim until it is dropped (as the VMM's
// exit would drop it). The handoff to a VMM through `Vmm::create(&cfg, claim)`
// is DG-2 row 6 and is authored at the attachment cut.
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 15, VhostVsockCidClaim, D16-CLAIM)"]
fn claimed_device_held_refuses_reclaim_until_released() {
    let claimer = VhostVsockCidClaim::new();
    let c = cid(103);
    let device = claimer.claim(c).expect("claim").into_device();
    assert!(device.vhost_fd().is_some(), "the descriptor the fd= handoff passes");

    assert!(held_elsewhere(&claimer, c), "the held device refuses a fresh claim of {c}");
    assert!(held_elsewhere(&claimer, c), "a refused claim changed nothing");

    drop(device); // the holder exits
    assert!(!held_elsewhere(&claimer, c), "{c} is claimable once the device is dropped");
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 15, VhostVsockCidClaim, D16-CLAIM)"]
fn claims_of_distinct_cids_are_independent() {
    let claimer = VhostVsockCidClaim::new();
    let (a, b) = (cid(104), cid(105));
    let held_a = claimer.claim(a).expect("claim a");
    let held_b = claimer.claim(b).expect("claim b: another CID is unaffected by a's holder");

    drop(held_a);
    assert!(!held_elsewhere(&claimer, a), "a is free after its holder drops");
    assert!(held_elsewhere(&claimer, b), "b is still held by its own holder");
    drop(held_b);
}
