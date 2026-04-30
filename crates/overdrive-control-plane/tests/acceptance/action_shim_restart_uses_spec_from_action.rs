//! Acceptance — `wire-exec-spec-end-to-end` action-shim Restart contract.
//!
//! Per ADR-0031 §6: the `Action::RestartAllocation` variant carries
//! a fully-populated `AllocationSpec`; the shim reads it straight off
//! the action and passes it to `Driver::start`. The Phase-1
//! placeholder helpers (`build_phase1_restart_spec`, `build_identity`,
//! `default_restart_resources`) are deleted in the same PR; this test
//! pins the new contract from the consumer side — what `Driver::start`
//! actually receives.
//!
//! Test shape: a recording fake `Driver` captures every spec passed to
//! `start()`. The shim is invoked with a `RestartAllocation` carrying
//! `command = "/opt/x/y"` + `args = ["--mode=fast"]`. The captured
//! spec must equal what the action carried — NOT the deleted
//! `/bin/sleep` + `["60"]` baseline.
//!
//! Covers `docs/feature/wire-exec-spec-end-to-end/distill/test-scenarios.md`
//! §6 *Action shim deletion*.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use overdrive_control_plane::action_shim::dispatch;
use overdrive_core::SpiffeId;
use overdrive_core::id::{AllocationId, JobId, NodeId};
use overdrive_core::reconciler::{Action, TickContext};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, Resources,
};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_sim::adapters::observation_store::SimObservationStore;

/// Recording fake driver. Captures every `AllocationSpec` passed to
/// `start()` so the test can assert on what the shim actually
/// dispatched.
struct RecordingDriver {
    spawned_specs: Arc<Mutex<Vec<AllocationSpec>>>,
}

impl RecordingDriver {
    fn new() -> Self {
        Self { spawned_specs: Arc::new(Mutex::new(Vec::new())) }
    }

    fn captured_specs(&self) -> Arc<Mutex<Vec<AllocationSpec>>> {
        Arc::clone(&self.spawned_specs)
    }
}

#[async_trait]
impl Driver for RecordingDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Exec
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        self.spawned_specs.lock().expect("mutex").push(spec.clone());
        Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: None })
    }

    async fn stop(&self, _handle: &AllocationHandle) -> Result<(), DriverError> {
        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        Err(DriverError::NotFound { alloc: handle.alloc.clone() })
    }

    async fn resize(
        &self,
        _handle: &AllocationHandle,
        _resources: Resources,
    ) -> Result<(), DriverError> {
        Ok(())
    }
}

#[tokio::test]
async fn action_shim_restart_passes_spec_from_action_to_driver_start_unchanged() {
    let driver = Arc::new(RecordingDriver::new());
    let captured = driver.captured_specs();
    let driver_dyn: Arc<dyn Driver> = driver.clone();

    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("NodeId"), 0));

    // Seed a prior alloc row — the shim's Restart arm needs this to
    // recover (job_id, node_id) for the Terminated AllocStatusRow it
    // writes after the start half completes (per ADR-0023).
    let alloc_id = AllocationId::new("alloc-payments-0").expect("alloc id");
    let job_id = JobId::new("payments").expect("job id");
    let node_id = NodeId::new("local").expect("node id");
    let prior_row = AllocStatusRow {
        alloc_id: alloc_id.clone(),
        job_id: job_id.clone(),
        node_id: node_id.clone(),
        state: AllocState::Terminated,
        updated_at: LogicalTimestamp { counter: 1, writer: node_id.clone() },
        reason: None,
        detail: None,
    };
    obs.write(ObservationRow::AllocStatus(prior_row)).await.expect("seed prior alloc row");

    // Construct the RestartAllocation action with a fully-populated
    // spec carrying operator-declared command + args.
    let identity = SpiffeId::new(&format!(
        "spiffe://overdrive.local/job/{}/alloc/{}",
        job_id.as_str(),
        alloc_id.as_str(),
    ))
    .expect("spiffe id");
    let restart_spec = AllocationSpec {
        alloc: alloc_id.clone(),
        identity,
        command: "/opt/x/y".to_string(),
        args: vec!["--mode=fast".to_string()],
        resources: Resources { cpu_milli: 200, memory_bytes: 128 * 1024 * 1024 },
    };
    let action = Action::RestartAllocation { alloc_id, spec: restart_spec.clone() };

    let now = Instant::now();
    let tick = TickContext { now, tick: 0, deadline: now + Duration::from_secs(1) };

    // Dispatch the action through the shim.
    dispatch(vec![action], driver_dyn.as_ref(), obs.as_ref(), &tick)
        .await
        .expect("dispatch must succeed");

    // The shim must have called `Driver::start` exactly once with the
    // spec carried on the action — NOT with the deleted /bin/sleep
    // fabrication. Clone-out + drop the guard to keep the lock window
    // tight (clippy::significant_drop_tightening).
    let captured_spec = {
        let specs = captured.lock().expect("mutex");
        assert_eq!(
            specs.len(),
            1,
            "Driver::start must be invoked exactly once for a Restart; got {} calls",
            specs.len(),
        );
        specs[0].clone()
    };
    assert_eq!(
        captured_spec.command, "/opt/x/y",
        "Driver::start must receive the action's command, NOT the deleted /bin/sleep literal",
    );
    assert_eq!(
        captured_spec.args,
        vec!["--mode=fast".to_string()],
        "Driver::start must receive the action's args, NOT the deleted [\"60\"] literal",
    );
    assert_eq!(
        captured_spec.resources, restart_spec.resources,
        "Driver::start must receive the action's resources, NOT the deleted \
         default_restart_resources fabrication (100mCPU + 256MiB)",
    );
}
