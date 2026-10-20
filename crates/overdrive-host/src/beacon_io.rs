//! `VsockBeaconIo` — the production adapter of the node beacon listener port
//! (`overdrive_core::vm::beacon::BeaconIo`, D17 / DG-4): real `AF_VSOCK`,
//! async over `AsyncFd`.
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! DG-4. Sim counterpart: `overdrive-sim::SimBeaconIo`.
//!
//! RED scaffold (DISTILL, Mandate 7): `bind` opens nothing and returns the
//! fixed port it was given (`BEACON_VSOCK_PORT` for a kernel-assigned one),
//! and every other port method panics, until DELIVER (the attachment-cut
//! step, DG-2 row 15) lands them. A node that composes the VM driver boots;
//! nothing accepts on the listener until that step. The constructor is the
//! one DG-4-CTOR pins: infallible, opens nothing.

// SCAFFOLD: true — netns-density-295 DISTILL (DELIVER attachment-cut step (6)).

use std::io;

use overdrive_core::guest_transport::VsockListenPort;
use overdrive_core::vm::beacon::{BEACON_VSOCK_PORT, BeaconConnKey, BeaconIo, BeaconSocket};
use overdrive_core::vm::config::VsockPort;

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

/// The node beacon listener on the host kernel: binds `VMADDR_CID_ANY` /
/// `BEACON_VSOCK_PORT`, accepts guest connections and reports each one's
/// kernel peer (`BeaconConnKey`).
pub struct VsockBeaconIo {
    _scaffold: (),
}

impl VsockBeaconIo {
    /// Infallible; opens no socket and touches no kernel state. The listener
    /// is created by `BeaconIo::bind`, whose failure `BeaconIntake::bind`
    /// reports as `BeaconIntakeError::Bind`.
    #[must_use]
    #[expect(
        clippy::new_without_default,
        reason = "DG-4-CTOR pins no `Default`: the beacon adapter is constructed explicitly at the composition root"
    )]
    pub const fn new() -> Self {
        Self { _scaffold: () }
    }
}

#[async_trait::async_trait]
impl BeaconIo for VsockBeaconIo {
    async fn bind(&self, port: VsockListenPort) -> Result<VsockPort, io::Error> {
        match port {
            VsockListenPort::Fixed(port) => Ok(port),
            // SCAFFOLD: nothing is bound until the attachment-cut step; a
            // kernel-assigned request reports the default beacon port so a
            // composed `serve` boots (the kernel-assigned cells are pending
            // that step).
            VsockListenPort::KernelAssigned => Ok(BEACON_VSOCK_PORT),
        }
    }

    async fn accept(&self) -> Result<(BeaconSocket, BeaconConnKey), io::Error> {
        panic!("{SCAFFOLD}")
    }

    async fn read_line(&self, _socket: &BeaconSocket) -> Result<Option<String>, io::Error> {
        panic!("{SCAFFOLD}")
    }

    async fn write_line(&self, _socket: &BeaconSocket, _line: &str) -> Result<(), io::Error> {
        panic!("{SCAFFOLD}")
    }

    async fn close(&self, _socket: BeaconSocket) {
        panic!("{SCAFFOLD}")
    }
}
