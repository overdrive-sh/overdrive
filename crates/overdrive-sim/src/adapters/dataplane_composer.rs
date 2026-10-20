//! `SimDataplaneComposer` — the simulation
//! `overdrive_control_plane::DataplaneComposer` (GH #295, NO-OVERRIDE-SEAMS,
//! ADR-0219; the feature-delta's replacement table, first row).
//!
//! `compose` returns the [`SimDataplane`] it was built over and the node
//! address it was built with (`Ipv4Addr::LOCALHOST` for every test that
//! injected `SimDataplane` before), or — when scripted — the `DataplaneBoot`
//! refusal it was given, constructing nothing. It carries no pin: a scripted
//! refusal is the simulated-boot counterpart of the production composer's
//! `Err` path, not its evidence.
//!
//! Like [`SimGuestDnsFactory`](super::SimGuestDnsFactory), it implements a
//! port `overdrive-control-plane` declares, so it serves that crate's
//! `tests/` suites and other crates; the control-plane's source-local tests
//! use their crate-private `TestDataplaneComposer`.

use std::net::Ipv4Addr;
use std::sync::Arc;

use async_trait::async_trait;
use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::{ComposedDataplane, DataplaneComposer, DataplanePlan};
use parking_lot::Mutex;

use super::dataplane::SimDataplane;

/// In-memory [`DataplaneComposer`] double.
pub struct SimDataplaneComposer {
    dataplane: Arc<SimDataplane>,
    host_ipv4: Ipv4Addr,
    refusal: Mutex<Option<ControlPlaneError>>,
}

impl SimDataplaneComposer {
    /// Every `compose` succeeds over `dataplane`, reporting `host_ipv4` as
    /// the node's address.
    #[must_use]
    pub const fn new(dataplane: Arc<SimDataplane>, host_ipv4: Ipv4Addr) -> Self {
        Self { dataplane, host_ipv4, refusal: Mutex::new(None) }
    }

    /// The next `compose` returns `refusal` (one of the boot's
    /// `DataplaneBoot` refusals) and constructs nothing.
    pub fn script_refusal(&self, refusal: ControlPlaneError) {
        *self.refusal.lock() = Some(refusal);
    }
}

#[async_trait]
impl DataplaneComposer for SimDataplaneComposer {
    async fn compose(
        &self,
        _plan: DataplanePlan<'_>,
    ) -> Result<ComposedDataplane, ControlPlaneError> {
        let scripted = self.refusal.lock().take();
        if let Some(refusal) = scripted {
            return Err(refusal);
        }
        Ok(ComposedDataplane {
            dataplane: Arc::clone(&self.dataplane) as _,
            host_ipv4: self.host_ipv4,
        })
    }
}
