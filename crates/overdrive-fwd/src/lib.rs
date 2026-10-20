//! `overdrive-fwd` — the kernel adapter of the core port
//! [`FlowForwarder`]: the control context of the in-kernel event-driven
//! flow forwarder `ovd_fwd` (EVMOD-PORT, EVMOD-CTL, EVMOD-REC;
//! ADR-0197, ADR-0199, ADR-0202).
//!
//! # Kernel requirement
//!
//! The adapter drives the module `ovd_fwd` (`kmod/ovd_fwd/`, its control
//! ABI SSOT `include/uapi/linux/ovd_fwd.h`) built for **Linux 7.0**, the
//! pinned appliance kernel (ADR-0068), or for one of the stock 7.0 kernels
//! of the Tier-3 hosts and guest fixture (EVMOD-BUILD, ADR-0203). Every
//! kernel fact this adapter rests on is cited against `vendors/linux` at
//! `v7.0` in `docs/feature/netns-density-295/feature-delta.md`, § *[REF]
//! vsock data path — the in-kernel event-driven forwarder (EVMOD)*. The
//! module unloads only when nothing references it (EVMOD-LIFE as amended by
//! MODULE-REFCOUNT, ADR-0198): every open context and every hooked socket
//! holds a module reference; the appliance kernel has no module unloading.
//!
//! The module is loaded by the host image and by `overdrive-init`, never by
//! `overdrive serve` (EVMOD-LOAD, ADR-0205); this crate never loads it.
//!
//! # Shape
//!
//! - [`KernelFlowForwarder::open`] opens one context on `/dev/ovd_fwd` with
//!   `O_RDWR | O_CLOEXEC | O_NONBLOCK`. The type is not `Clone`; dropping it
//!   closes the context, whose release aborts every pair and watch the
//!   context created and queues no record (EVMOD-CTL).
//! - It implements `FlowForwarder<dyn AsFd>` and lends the context's
//!   descriptor for the owners' event loops through
//!   [`KernelFlowForwarder::readiness_fd`] (`EPOLLIN` while an end record is
//!   queued, EVMOD-REC).
//! - The crate-private `uapi` module holds the `#[repr(C)]` mirrors of the
//!   control ABI with `const` layout assertions.
//!
//! The crate depends on `overdrive-core`, `nix` and `thiserror` only (no
//! async runtime), so both `overdrive-control-plane` and the static
//! `overdrive-init` link it.
//!
//! # Tests
//!
//! The Tier-3 suite (`tests/integration/`, `integration-tests` feature)
//! runs against the real module through this adapter on the host role in
//! Lima (`cargo xtask lima run --`), in the `host-kernel-shared` nextest
//! group.
//!
//! RED scaffold: every body panics until DELIVER step 4 ("Forwarder module
//! and adapter").

// SCAFFOLD: true — netns-density-295 DISTILL (DELIVER step 4).

mod uapi;

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use overdrive_core::guest_transport::forwarder::{
    AbortResult, FlowForwarder, ForwardEnd, ForwardId, ForwarderError, ForwarderInventory,
    PairMode, PairState,
};

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

/// One forwarder context: an open `/dev/ovd_fwd` (EVMOD-CTL, ADR-0199).
///
/// Not `Clone`, never duplicated, sent or inherited (`O_CLOEXEC`). Dropping
/// it closes the descriptor; the release aborts every live pair (TCP halves
/// reset, vsock halves `SHUT_RDWR`) and every live watch (its `V_h` shut
/// `SHUT_RDWR` — the only operation that shuts a watched socket,
/// WATCH-ABORT, ADR-0211) and queues no record.
#[derive(Debug)]
pub struct KernelFlowForwarder {
    context: OwnedFd,
}

impl KernelFlowForwarder {
    /// Opens a new context on `/dev/ovd_fwd` with
    /// `O_RDWR | O_CLOEXEC | O_NONBLOCK`.
    ///
    /// # Errors
    ///
    /// [`ForwarderError::DeviceAbsent`] when the device cannot be opened.
    /// The ABI version is checked by [`FlowForwarder::probe`] (`Version`
    /// stage), not here.
    pub fn open() -> Result<Self, ForwarderError> {
        panic!("{SCAFFOLD}")
    }

    /// The context's descriptor, for the owner's event loop: readable
    /// (`EPOLLIN`) while an end record is queued (EVMOD-REC).
    #[must_use]
    pub fn readiness_fd(&self) -> BorrowedFd<'_> {
        let _ = &self.context;
        panic!("{SCAFFOLD}")
    }
}

impl FlowForwarder<dyn AsFd> for KernelFlowForwarder {
    /// The forwarder probe (§ *Earned Trust — the forwarder probe*): the
    /// `ForwarderProbeStage` order `DeviceOpen`, `Version`,
    /// `StreamRoundTrip`, `HalfClose`, `PeerAbortClassified`,
    /// `Backpressure`, `DatagramRoundTrip`, `WatchClassified`,
    /// `ContextRelease`, `Cleanup`, over the vsock loopback transport
    /// (CID 1). A failure names its stage.
    fn probe(&self) -> Result<(), ForwarderError> {
        panic!("{SCAFFOLD}")
    }

    fn attach(
        &self,
        _a: &dyn AsFd,
        _b: &dyn AsFd,
        _mode: PairMode,
    ) -> Result<ForwardId, ForwarderError> {
        panic!("{SCAFFOLD}")
    }

    fn watch(&self, _vsock: &dyn AsFd) -> Result<ForwardId, ForwarderError> {
        panic!("{SCAFFOLD}")
    }

    fn abort(&self, _id: ForwardId) -> Result<AbortResult, ForwarderError> {
        panic!("{SCAFFOLD}")
    }

    fn abort_all(&self) -> Result<u32, ForwarderError> {
        panic!("{SCAFFOLD}")
    }

    fn take_end(&self) -> Result<Option<ForwardEnd>, ForwarderError> {
        panic!("{SCAFFOLD}")
    }

    fn pair_state(&self, _id: ForwardId) -> Result<Option<PairState>, ForwarderError> {
        panic!("{SCAFFOLD}")
    }

    fn inventory(&self) -> Result<ForwarderInventory, ForwarderError> {
        panic!("{SCAFFOLD}")
    }
}
