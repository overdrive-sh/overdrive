//! `SimDriver` — in-memory [`Driver`] implementation for DST.
//!
//! Each `SimDriver` is configured with a fixed [`DriverType`] and owns
//! an allocation table that tracks lifecycle state (`Running`,
//! `Terminated`, `Failed`). Failure modes are injected via builder
//! methods — `fail_on_start_with(reason)` — so scheduler and
//! reconciler tests can exercise "driver rejected start" behaviour
//! without spawning a real VMM.

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;

use overdrive_core::id::AllocationId;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, Resources,
};

/// In-memory driver. Construct via [`SimDriver::new`], optionally
/// chain `.fail_on_start_with(reason)` to reject every subsequent
/// `start` call with a [`DriverError::StartRejected`].
pub struct SimDriver {
    r#type: DriverType,
    allocations: Mutex<HashMap<AllocationId, AllocationState>>,
    failure_mode: Mutex<Option<FailureMode>>,
}

/// Configured failure mode for the driver. Stored behind a mutex so
/// tests can mutate it after construction (e.g. "fail the next start,
/// then succeed").
#[derive(Debug, Clone)]
enum FailureMode {
    StartRejected { reason: String },
}

impl SimDriver {
    /// Construct a `SimDriver` that reports `r#type` from
    /// [`Driver::type`] and holds no allocations.
    #[must_use]
    pub fn new(r#type: DriverType) -> Self {
        Self { r#type, allocations: Mutex::new(HashMap::new()), failure_mode: Mutex::new(None) }
    }

    /// Configure this driver to reject every subsequent `start` call
    /// with the given reason.
    #[must_use]
    pub fn fail_on_start_with(self, reason: String) -> Self {
        *self.failure_mode.lock() = Some(FailureMode::StartRejected { reason });
        self
    }
}

#[async_trait]
impl Driver for SimDriver {
    fn r#type(&self) -> DriverType {
        self.r#type
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        let failure = self.failure_mode.lock().clone();
        if let Some(FailureMode::StartRejected { reason }) = failure {
            return Err(DriverError::StartRejected { driver: self.r#type, reason });
        }

        self.allocations.lock().insert(spec.alloc.clone(), AllocationState::Running);
        Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: None })
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        {
            let mut allocations = self.allocations.lock();
            if !allocations.contains_key(&handle.alloc) {
                return Err(DriverError::NotFound { alloc: handle.alloc.clone() });
            }
            allocations.insert(handle.alloc.clone(), AllocationState::Terminated);
        }
        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        self.allocations
            .lock()
            .get(&handle.alloc)
            .cloned()
            .ok_or_else(|| DriverError::NotFound { alloc: handle.alloc.clone() })
    }

    async fn resize(
        &self,
        handle: &AllocationHandle,
        _resources: Resources,
    ) -> Result<(), DriverError> {
        if !self.allocations.lock().contains_key(&handle.alloc) {
            return Err(DriverError::NotFound { alloc: handle.alloc.clone() });
        }
        Ok(())
    }
}
