//! `SimGuestCidClaim` — the in-memory double of the D16-CLAIM host-kernel
//! CID-claim port (`overdrive_core::guest_transport::GuestCidClaim`;
//! production adapter `overdrive-host::VhostVsockCidClaim`).
//!
//! Feature #295, vsock attachment replacement. Design:
//! `docs/feature/netns-density-295/feature-delta.md` §"[REF] Core vocabulary"
//! (the `GuestCidClaim` / `ClaimedGuestCid` / `ClaimedVsockDevice` contract)
//! and §"Sim counterparts" (L20022): "an atomic claim set over CIDs (the
//! `ClaimSet` discipline) with scripted holders outside the pool's leases —
//! VMMs that survived a `serve` crash — that hold CIDs at boot and release
//! them at any time, a scripted non-`InUse` failure, and a scripted probe
//! deviation per `GuestCidClaimProbeStage`; dropping a sim claim releases it;
//! its devices return `vhost_fd() = None`."
//!
//! It models the one real arbiter the kernel provides — the per-CID holder
//! table — with `overdrive_core::claim_set::ClaimSet`, whose `try_claim` is the
//! atomic check-and-act the design relies on (`.claude/rules/rust.md`
//! §"Check-and-act must be atomic"). A test double that is MORE permissive than
//! the real adapter would hide wiring bugs (`.claude/rules/tdd-methodology`,
//! "test doubles must validate inputs"); this one refuses an already-held CID
//! exactly as `VHOST_VSOCK_SET_GUEST_CID` does with `EADDRINUSE`.
//!
//! Test support (DISTILL): the scripting methods (`survivor_hold`,
//! `script_claim_error`, `script_probe_deviation`) are this sim's own surface;
//! the arbiter read accessor `held_cids` is pinned by QPROJ-7 (feature-delta
//! § "Conformance observation surface and action map"). Its Tier-3 pairing
//! (R5-19 (a), (b), (e) on the real `VhostVsockCidClaim`) is not authored yet:
//! the host adapter's constructor is not pinned
//! (`docs/feature/netns-density-295/distill/upstream-issues.md` UI-HOST-1), so
//! no conformance projection may read `held_cids` until that pairing exists.

use overdrive_core::claim_set::ClaimSet;
use overdrive_core::guest_transport::{
    ClaimedGuestCid, ClaimedVsockDevice, GUEST_CID_PROBE, GuestCid, GuestCidClaim,
    GuestCidClaimError, GuestCidClaimProbeStage,
};
use parking_lot::Mutex;
use std::collections::BTreeSet;

/// In-memory `GuestCidClaim`. Cheap to clone-share via the interior `ClaimSet`.
#[derive(Default)]
pub struct SimGuestCidClaim {
    /// The host-kernel CID holder table the design models (K-C2): a CID held
    /// by ANY instance — a pool lease's claim, a handed-off VMM device, or a
    /// scripted surviving-VMM holder — is refused to every other claim.
    kern: ClaimSet<GuestCid>,
    /// CIDs whose next `claim` is scripted to fail for a cause OTHER than
    /// `InUse` (a kernel/device error); consumed on use. Drives R5-19c.
    scripted_claim_errors: Mutex<BTreeSet<GuestCid>>,
    /// If set, the next `probe` reports a deviation at this stage instead of
    /// succeeding; consumed on use.
    scripted_probe_deviation: Mutex<Option<GuestCidClaimProbeStage>>,
    /// A dedicated claim set for the probe's reserved CID, so the probe can
    /// exercise the claim/second-claim-refused/reclaim discipline without a
    /// `GuestCid` (which cannot represent the reserved `GUEST_CID_PROBE`).
    probe_slot: ClaimSet<u32>,
}

impl SimGuestCidClaim {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Script the next `claim(cid)` to fail with a non-`InUse` kernel error
    /// (R5-19c): the pool must NOT skip the offset on it. One-shot.
    pub fn script_claim_error(&self, cid: GuestCid) {
        self.scripted_claim_errors.lock().insert(cid);
    }

    /// Script the next `probe()` to deviate at `stage` (an unfaithful kernel).
    /// One-shot.
    pub fn script_probe_deviation(&self, stage: GuestCidClaimProbeStage) {
        *self.scripted_probe_deviation.lock() = Some(stage);
    }

    /// A VMM that survived a `serve` crash holds `cid` outside any pool lease
    /// (K-C2). The returned device releases the CID when dropped — the test
    /// drops it to model that VMM exiting "at any time". While held, the pool's
    /// `assign` must skip `cid`'s offset (R5-19a/e). `Err(InUse)` if `cid` is
    /// already held: a survivor exists only for a CID it could claim.
    pub fn survivor_hold(&self, cid: GuestCid) -> Result<ClaimedGuestCid, GuestCidClaimError> {
        let guard = self.kern.try_claim(cid).ok_or(GuestCidClaimError::InUse { cid })?;
        Ok(ClaimedGuestCid::new(cid, Box::new(SimClaimedVsockDevice { _guard: guard })))
    }

    /// Observable surface for the conformance driver's `State::from_driver`:
    /// the set of CIDs the kernel currently holds (the `kern` table).
    #[must_use]
    pub fn held_cids(&self) -> BTreeSet<GuestCid> {
        self.kern.snapshot()
    }
}

impl GuestCidClaim for SimGuestCidClaim {
    fn claim(&self, cid: GuestCid) -> Result<ClaimedGuestCid, GuestCidClaimError> {
        // Atomic check-and-act: `try_claim` returns None iff already held. A
        // held CID answers `InUse` whatever is scripted, as the kernel's
        // `EADDRINUSE` does, so `held_cids` is exactly the set a claim would
        // refuse `InUse` (QPROJ-7).
        let Some(guard) = self.kern.try_claim(cid) else {
            return Err(GuestCidClaimError::InUse { cid });
        };
        // A scripted non-`InUse` failure of an unheld CID is consumed; the
        // guard drops, so nothing is held.
        if self.scripted_claim_errors.lock().remove(&cid) {
            drop(guard);
            return Err(GuestCidClaimError::Kernel {
                cid,
                source: std::io::Error::other("sim-scripted non-InUse claim failure"),
            });
        }
        Ok(ClaimedGuestCid::new(cid, Box::new(SimClaimedVsockDevice { _guard: guard })))
    }

    fn probe(&self) -> Result<(), GuestCidClaimError> {
        let scripted = self.scripted_probe_deviation.lock().take();
        if let Some(stage) = scripted {
            return Err(GuestCidClaimError::ProbeMismatch { stage });
        }
        // Faithfully exercise the kernel contract on the reserved probe CID:
        // claim, a second claim must be refused, release, a third must succeed.
        let first = self.probe_slot.try_claim(GUEST_CID_PROBE).ok_or(
            GuestCidClaimError::ProbeMismatch { stage: GuestCidClaimProbeStage::FirstClaim },
        )?;
        if self.probe_slot.try_claim(GUEST_CID_PROBE).is_some() {
            return Err(GuestCidClaimError::ProbeMismatch {
                stage: GuestCidClaimProbeStage::SecondClaimRefused,
            });
        }
        drop(first); // release
        let third = self.probe_slot.try_claim(GUEST_CID_PROBE).ok_or(
            GuestCidClaimError::ProbeMismatch {
                stage: GuestCidClaimProbeStage::ReclaimAfterRelease,
            },
        )?;
        drop(third); // leaves nothing held
        Ok(())
    }
}

/// The device instance holding a simulated claim. Dropping it (either directly,
/// or after `ClaimedGuestCid::into_device` hands it to the "VMM" and that is
/// later dropped at the VMM's exit) releases the `ClaimSet` guard, freeing the
/// CID — the sim analogue of the real last-close release (A-29).
struct SimClaimedVsockDevice {
    /// Held for its `Drop`: releasing the guard frees the CID.
    _guard: overdrive_core::claim_set::ClaimGuard<GuestCid>,
}

impl std::fmt::Debug for SimClaimedVsockDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `ClaimGuard` has no `Debug`; the device is identified by nothing more
        // than being a simulated claim.
        f.debug_struct("SimClaimedVsockDevice").finish_non_exhaustive()
    }
}

impl ClaimedVsockDevice for SimClaimedVsockDevice {
    fn vhost_fd(&self) -> Option<std::os::fd::BorrowedFd<'_>> {
        None // a simulated claim has no real `/dev/vhost-vsock` descriptor
    }
}
