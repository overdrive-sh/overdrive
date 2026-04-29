//! US-02 Scenario 2.8 — limit-write failure warns and continues.
//!
//! @real-io — Linux. Per ADR-0026 D9, when `cpu.weight` or
//! `memory.max` write fails (e.g. tmpfs without cgroup-controller
//! semantics), the driver emits a `tracing::warn!` log and proceeds
//! to PID enrolment — `Driver::start` succeeds, the alloc reaches
//! `Running`. We force the failure with a test-injected toggle on
//! `ExecDriver` that makes the limit-write helper return error
//! synthetically.

use std::sync::Arc;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::driver::{AllocationSpec, AllocationState, Driver, Resources};
use overdrive_worker::ExecDriver;
use tempfile::TempDir;

#[tokio::test]
async fn limit_write_failure_warns_and_continues() {
    let cgroup_root = TempDir::new().expect("tempdir created");
    std::fs::create_dir_all(cgroup_root.path().join("overdrive.slice/workloads.slice"))
        .expect("workloads.slice created");

    let driver: Arc<dyn Driver> = Arc::new(
        ExecDriver::new(cgroup_root.path().to_path_buf()).with_force_limit_write_failure(true),
    );

    let alloc = AllocationId::new("alloc-limit-write-fail").expect("valid alloc id");
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/x/alloc/lw")
            .expect("valid spiffe id"),
        image: "/bin/sleep".to_owned(),
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
    };

    // With force-fail injection, the limit-write helper returns
    // an error; the driver warn-and-continues, so start MUST still
    // succeed and the alloc MUST reach Running.
    let handle = driver.start(&spec).await.expect("start succeeds even when limit writes fail");
    let state = driver.status(&handle).await.expect("status succeeds");
    assert_eq!(state, AllocationState::Running);

    driver.stop(&handle).await.expect("stop succeeds");
}
