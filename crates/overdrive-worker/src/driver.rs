//! `ProcessDriver` — the Phase 1 production driver impl per ADR-0026
//! and ADR-0029.
//!
//! Linux-only by design. Spawns child processes via
//! `tokio::process::Command`, places them into a workload cgroup
//! scope, writes resource limits, and supervises lifecycle.
//!
//! # Status — RED scaffold
//!
//! Phase: phase-1-first-workload, slice 2 (US-02). Wave: DISTILL.

use async_trait::async_trait;

use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, Resources,
};

/// SCAFFOLD marker.
pub const SCAFFOLD: bool = true;

/// Production `Driver` impl for native processes under cgroup v2
/// supervision. Linux-only; macOS / Windows builds skip this module
/// (the worker crate is `adapter-host` class — host-OS adapters are
/// expected to be platform-specific).
///
/// Per ADR-0026 D6: direct cgroupfs writes; no `cgroups-rs` dep.
/// Per ADR-0026 D9: `cpu.weight` + `memory.max` derived from
/// `AllocationSpec::resources` at start time.
#[derive(Debug, Default)]
pub struct ProcessDriver {
    // Phase 1 RED scaffold — fields are placeholder. DELIVER will
    // wire the live-PID tracking map (most likely
    // `Arc<Mutex<BTreeMap<AllocationId, ChildHandle>>>` per the
    // ordered-collection rule).
    _placeholder: (),
}

impl ProcessDriver {
    /// Construct a fresh `ProcessDriver`.
    ///
    /// # Panics
    ///
    /// RED scaffold.
    #[must_use]
    pub fn new() -> Self {
        panic!("Not yet implemented -- RED scaffold")
    }
}

#[async_trait]
impl Driver for ProcessDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Process
    }

    async fn start(&self, _spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        panic!("Not yet implemented -- RED scaffold")
    }

    async fn stop(&self, _handle: &AllocationHandle) -> Result<(), DriverError> {
        panic!("Not yet implemented -- RED scaffold")
    }

    async fn status(&self, _handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        panic!("Not yet implemented -- RED scaffold")
    }

    async fn resize(
        &self,
        _handle: &AllocationHandle,
        _resources: Resources,
    ) -> Result<(), DriverError> {
        panic!("Not yet implemented -- RED scaffold")
    }
}
