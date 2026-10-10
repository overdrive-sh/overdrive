//! `VhostVsockCidClaim` — the host-kernel adapter of the D16-CLAIM CID-claim
//! port (`overdrive_core::guest_transport::GuestCidClaim`).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! DG-6 (`VhostVsockCidClaim::new`, `VHOST_VSOCK_DEVICE`) and § "[REF] Core
//! vocabulary" (the `GuestCidClaim` / `ClaimedGuestCid` / `ClaimedVsockDevice`
//! contract, CLAIM-PINS). Sim counterpart: `overdrive-sim::SimGuestCidClaim`.
//!
//! RED scaffold (DISTILL, Mandate 7): the constructor is real (it opens
//! nothing); `claim` and `probe` panic until DELIVER vsock new-step 15 (the
//! host-kernel CID claim) lands them.

// SCAFFOLD: true — netns-density-295 DISTILL (vsock new-step 15).

use overdrive_core::guest_transport::{
    ClaimedGuestCid, GuestCid, GuestCidClaim, GuestCidClaimError,
};

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

/// The host device every claim opens a new instance of.
pub const VHOST_VSOCK_DEVICE: &str = "/dev/vhost-vsock";

/// `GuestCidClaim` on the host kernel.
///
/// Each `claim` opens a new instance of `VHOST_VSOCK_DEVICE`
/// (`O_RDWR | O_CLOEXEC`) and issues `VHOST_VSOCK_SET_GUEST_CID` once; an open
/// failure is `DeviceUnavailable`.
///
/// No `Default` and no device-path override: tests on the real kernel use the
/// real device (P1-S12 cells, QPROJ-7 pairing R5-19).
pub struct VhostVsockCidClaim {
    _private: (),
}

impl VhostVsockCidClaim {
    /// Infallible; opens nothing (device access is proven by `probe`).
    #[must_use]
    #[expect(
        clippy::new_without_default,
        reason = "DG-6 pins no `Default`: the claim adapter is constructed explicitly at the composition root"
    )]
    pub const fn new() -> Self {
        Self { _private: () }
    }
}

impl GuestCidClaim for VhostVsockCidClaim {
    /// Opens a new `VHOST_VSOCK_DEVICE` instance and issues
    /// `VHOST_VSOCK_SET_GUEST_CID` on it exactly once. Only `EADDRINUSE` is
    /// `InUse`; every other errno is `Kernel`; an open failure is
    /// `DeviceUnavailable`. `Ok`: the returned claim's device carries the open
    /// file description (`vhost_fd() = Some`).
    fn claim(&self, _cid: GuestCid) -> Result<ClaimedGuestCid, GuestCidClaimError> {
        panic!("{SCAFFOLD}")
    }

    /// Earned Trust: claim `GUEST_CID_PROBE`; a second claim of it must fail
    /// `InUse`; release; a third claim must succeed; release. Any deviation is
    /// `Err`. Leaves nothing held.
    fn probe(&self) -> Result<(), GuestCidClaimError> {
        panic!("{SCAFFOLD}")
    }
}
