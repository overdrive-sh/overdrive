//! `BeaconIntake` — the node's bound beacon listener and its accept task
//! (D17, DG-4).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! DG-4 (`BeaconIntake::bind`, `BeaconIntakeError`) over the core
//! `overdrive_core::vm::beacon::BeaconIo` port (production `VsockBeaconIo`,
//! simulation `SimBeaconIo`).
//!
//! RED scaffold (DISTILL, Mandate 7): `bind` panics until DELIVER vsock
//! new-step 6 (beacon relocation, D17) lands it; `BeaconIntakeError` is real.
//! `VmDriver::new` does not take the intake yet — that parameter is DG-2
//! row 15 and lands with the attachment cut.

// SCAFFOLD: true — netns-density-295 DISTILL (vsock new-step 6).
#![expect(
    clippy::unused_async,
    reason = "RED scaffold body panics before awaiting; the lint self-trips when DELIVER lands the body"
)]

use std::sync::Arc;

use overdrive_core::vm::beacon::BeaconIo;
use overdrive_core::vm::config::VsockPort;

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

/// D17: the node's bound beacon listener and its accept task. Exists only
/// bound; dropping it stops the accept task and closes the listener.
pub struct BeaconIntake {
    _scaffold: (),
}

impl BeaconIntake {
    /// Binds through `io` and starts the accept task. Production passes
    /// `VsockBeaconIo`; tests and `run_server` boots with a simulated VMM
    /// pass `SimBeaconIo`. Called once by `compose_vm_driver`, after
    /// `Vmm::probe`. Err → `health.startup.refused`, reason
    /// `vm_driver.beacon_bind`; nothing bound.
    pub async fn bind(_io: Arc<dyn BeaconIo>) -> Result<Self, BeaconIntakeError> {
        panic!("{SCAFFOLD}")
    }
}

/// Why the node beacon intake could not be bound.
#[derive(Debug, thiserror::Error)]
pub enum BeaconIntakeError {
    /// Binding the beacon vsock listener failed; nothing is bound.
    #[error("binding the beacon vsock listener on port {port} failed: {source}")]
    Bind {
        /// The beacon port (`BEACON_VSOCK_PORT`).
        port: VsockPort,
        /// The bind error.
        #[source]
        source: std::io::Error,
    },
}
