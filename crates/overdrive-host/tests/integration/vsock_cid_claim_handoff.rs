//! P1 — Tier-3: the real host-kernel CID claim and its descriptor handoff.
//!
//! Feature #295, vsock attachment replacement. Spec:
//! `docs/feature/netns-density-295/distill/test-scenarios-p1-admission-cid-lease.md`
//! (P1-S12); design: D16-CLAIM / CLAIM-PINS, gate G-V0 + G-V4, boundary row
//! R5-19d, validation item V-25 (done, P-39, `spike/findings-v25.md`).
//!
//! Production surface under test (all pinned by the design, none exists yet):
//!   * `overdrive_host::VhostVsockCidClaim` — the host adapter implementing
//!     `overdrive_core::guest_transport::GuestCidClaim` over a new
//!     `/dev/vhost-vsock` instance per claim (`VHOST_VSOCK_SET_GUEST_CID`);
//!   * `ClaimedGuestCid::into_device()` → the CH fork's `--vsock
//!     cid=<cid>,backend=vhost-kernel,fd=<K>` launch (fork `fd=` mode = DELIVER
//!     step 1), with `serve` closing its copy once the VMM holds it.
//!
//! LANE (Mandate 9 / 11): example-based ONLY — NO PBT at Tier-3. Each failure
//! mode is one named scenario. Real kernel + real `/dev/vhost-vsock` + (for the
//! handoff) a real CH-fork microVM boot on `/dev/kvm`: runs on the native
//! x86_64 KVM metal host via `cargo xtask metal run -- cargo nextest run -p
//! overdrive-host --features integration-tests,kvm-tests`
//! (`.claude/rules/testing.md` §"native x86_64 bare-metal KVM host"), never
//! Lima for the boot cells.
//!
//! NODE-GLOBAL KERNEL STATE: the `/dev/vhost-vsock` CID holder table is shared
//! by every vsock Tier-3 test. This module MUST join the `host-kernel-shared`
//! nextest group (`.config/nextest.toml`, `max-threads = 1`, matched by MODULE)
//! when DELIVER registers it — shared-infra §5. The DISTILL planner does not
//! edit the nextest config (a shared file); the crafter adds the group entry
//! and the `mod vsock_cid_claim_handoff;` line in `tests/integration.rs` when
//! this body co-arrives with `VhostVsockCidClaim` (shared-infra §6).
//!
//! RED: `#[ignore]` naming the activating DELIVER step; the whole module is
//! `kvm-tests`-gated so it never compiles into a non-metal run.

#![cfg(feature = "kvm-tests")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GuestCid, GuestCidClaim, GuestCidClaimError,
};
use overdrive_host::VhostVsockCidClaim;

fn cid(offset: u32) -> GuestCid {
    GuestCid::new(GUEST_CID_BASE + offset).expect("offset within the leased span")
}

// ---------------------------------------------------------------------------
// P1-S12a / R5-19d / V-25 — exclusive claim on an unowned instance.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "pending DELIVER step 01 — VhostVsockCidClaim + CH fork fd= mode (metal, kvm-tests)"]
fn claim_on_unowned_instance_holds_cid_exclusively() {
    // `VhostVsockCidClaim` runs under the `serve` identity (it is the only
    // opener of `/dev/vhost-vsock`; the VMM never opens it).
    let claimer = VhostVsockCidClaim::open().expect("/dev/vhost-vsock opens under serve identity");
    let c = cid(100);

    let held = claimer.claim(c).expect("SET_GUEST_CID succeeds on an unowned instance");
    assert_eq!(held.cid(), c);

    // A second claim of the same CID while the first reference is open is the
    // kernel's EADDRINUSE, surfaced as InUse — never a different errno.
    assert!(matches!(
        claimer.claim(c),
        Err(GuestCidClaimError::InUse { cid }) if cid == c
    ));
}

// ---------------------------------------------------------------------------
// P1-S12 / V-25 (A-29) — released at the last reference close.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "pending DELIVER step 01 — VhostVsockCidClaim + CH fork fd= mode (metal, kvm-tests)"]
fn claim_released_at_last_reference_close() {
    let claimer = VhostVsockCidClaim::open().expect("open");
    let c = cid(101);

    let held = claimer.claim(c).expect("first claim");
    drop(held); // the last (only) reference closes

    // The CID is claimable again after the last close (P-39: ≤ 0.06 ms after
    // reap). We assert re-claimability (the observable contract); the latency
    // is reported, not hard-asserted, per the spike's percentile discipline.
    let reclaimed = claimer.claim(c).expect("claimable after last close");
    assert_eq!(reclaimed.cid(), c);
}

// ---------------------------------------------------------------------------
// P1-S13-probe — GuestCidClaim::probe under the serve identity.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "pending DELIVER step 01 — VhostVsockCidClaim + CH fork fd= mode (metal, kvm-tests)"]
fn claim_probe_holds_under_serve_identity() {
    // The boot Earned-Trust probe: claim GUEST_CID_PROBE, a second claim must
    // fail, release, a third must succeed, release. On a real kernel this must
    // hold; a deviation is a typed `ProbeMismatch`, never a silent pass.
    let claimer = VhostVsockCidClaim::open().expect("open");
    claimer.probe().expect("the kernel honours the claim contract");
}

// ---------------------------------------------------------------------------
// P1-S12 / R5-19d — the claimed device is held exactly once and refuses a
// fresh claim until released. This is the CLAIM-LEVEL half of the handoff,
// authorable with no fork: `into_device()` consumes the claim once; while the
// device is held (by the test here, as the VMM would hold it), a fresh claim of
// the same CID is `InUse`; dropping it (as the VMM's exit would) frees it.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "pending DELIVER step 01 — VhostVsockCidClaim (metal, kvm-tests)"]
fn claimed_device_held_refuses_reclaim_until_released() {
    let claimer = VhostVsockCidClaim::open().expect("open");
    let c = cid(102);
    let held = claimer.claim(c).expect("claim");

    // `into_device` consumes the claim for handoff; the device holds the CID
    // (the real handoff passes this open file description itself — no reopen of
    // /proc/self/fd/<n>, which P-39 showed yields a fresh unclaimed instance).
    let device = held.into_device();
    assert!(
        device.vhost_fd().is_some(),
        "the host claim carries a real /dev/vhost-vsock descriptor for fd= handoff",
    );

    // While the device is held, the CID is refused to a fresh claim (no second
    // holder) — the kernel exclusivity the whole design rests on.
    assert!(matches!(
        claimer.claim(c),
        Err(GuestCidClaimError::InUse { cid }) if cid == c
    ));

    // Dropping the device (as the VMM's exit would) releases the CID.
    drop(device);
    assert_eq!(claimer.claim(c).expect("claimable after the holder exits").cid(), c);
}

// NOTE — the FULL fork-boot cells of P1-S12 / R5-19d / R5-19f live in P6's
// walking skeleton (`tests/conformance/tests/vsock_walking_skeleton.rs`), not
// here, because they need a real CH-fork `--vsock
// cid=...,backend=vhost-kernel,fd=K` boot (the fork's `fd=` mode is DELIVER
// step 1) driven through `overdrive serve` + `overdrive deploy`:
//   * the guest reports peer CID == the claimed CID (vsock stamps it),
//     `serve` closes its copy after spawn, a byte round-trips, and the CID is
//     `InUse` while the VMM runs and claimable after it exits (poweroff AND
//     SIGKILL); and
//   * a bad/consumed descriptor makes CH exit before READY — projected through
//     the EXISTING pre-READY VMM-exit start failure, never a CID-clash cause
//     (D16-CLAIM) — with the CID still claimable afterward.
// Authoring a fork-boot body here would require inventing the fork launch
// harness (production, DELIVER step 1), which this file must not do. The
// boundary is recorded in the P1 scenario doc's "Placement gaps".
