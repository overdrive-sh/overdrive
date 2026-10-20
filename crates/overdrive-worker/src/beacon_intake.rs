//! `BeaconIntake` — the node's bound beacon listener and its accept task
//! (D17, DG-4).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! DG-4 (`BeaconIntake::bind`, `BeaconIntakeError`) over the core
//! `overdrive_core::vm::beacon::BeaconIo` port (production `VsockBeaconIo`,
//! simulation `SimBeaconIo`).
//!
//! RED scaffold (DISTILL, Mandate 7): `bind` binds through `io` and starts no
//! accept task until the DELIVER attachment-cut step (beacon relocation, D17)
//! lands it; `BeaconIntakeError` is real. `VmDriver::new(.., beacon)` takes
//! the bound intake (DG-2 row 15). A node that composes the VM driver boots;
//! a VM launch stays RED in the driver's READY accept, which panics until the
//! same step.

// SCAFFOLD: true — netns-density-295 DISTILL (attachment-cut step 6).

use std::sync::Arc;

use overdrive_core::guest_transport::VsockListenPort;
use overdrive_core::vm::beacon::BeaconIo;
use overdrive_core::vm::config::VsockPort;

/// D17: the node's bound beacon listener and its accept task. Exists only
/// bound; dropping it stops the accept task and closes the listener.
pub struct BeaconIntake {
    #[allow(
        dead_code,
        reason = "RED scaffold — the accept task the attachment cut lands reads it"
    )]
    io: Arc<dyn BeaconIo>,
    bound_port: VsockPort,
}

impl BeaconIntake {
    /// Binds through `io` on `port` and starts the accept task. Production
    /// passes `VsockBeaconIo` and `config.vsock_ports.beacon()`; tests and
    /// `run_server` boots with a simulated VMM pass `SimBeaconIo`. Called
    /// once by `compose_vm_driver`, after `Vmm::probe`. Err →
    /// `health.startup.refused`, reason `vm_driver.beacon_bind`; nothing
    /// bound.
    pub async fn bind(
        io: Arc<dyn BeaconIo>,
        port: VsockListenPort,
    ) -> Result<Self, BeaconIntakeError> {
        let bound_port =
            io.bind(port).await.map_err(|source| BeaconIntakeError::Bind { port, source })?;
        Ok(Self { io, bound_port })
    }

    /// PORTS-CONFIG: the port `bind` took; constant for the intake's life.
    /// The VM driver writes it on each guest's kernel command line
    /// (`beacon=`, ADR-0217).
    #[must_use]
    pub const fn bound_port(&self) -> VsockPort {
        self.bound_port
    }
}

/// Why the node beacon intake could not be bound.
#[derive(Debug, thiserror::Error)]
pub enum BeaconIntakeError {
    /// Binding the beacon vsock listener failed; nothing is bound. `port` is
    /// the configured `VsockListenPort`.
    #[error("binding the beacon vsock listener on port {port:?} failed: {source}")]
    Bind {
        /// The configured port.
        port: VsockListenPort,
        /// The bind error.
        #[source]
        source: std::io::Error,
    },
}
