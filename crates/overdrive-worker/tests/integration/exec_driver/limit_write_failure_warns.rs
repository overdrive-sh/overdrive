//! US-02 Scenario 2.8 — limit-write failure warns and continues.
//!
//! @real-io — Linux. Per ADR-0026 D9, when `cpu.weight` or
//! `memory.max` write fails, the driver emits a `tracing::warn!` log
//! and proceeds to PID enrolment — `Driver::start` succeeds, the
//! alloc reaches `Running`. We force the failure with a test-injected
//! toggle on `ExecDriver` that makes the limit-write helper return
//! error synthetically.
//!
//! The `force_limit_write_failure` injection seam is filesystem-
//! agnostic — it short-circuits the limit-write call with a synthetic
//! EACCES regardless of whether the underlying path is on tmpfs or
//! cgroupfs. This test continues to assert the warn-and-continue path
//! after the Phase 02 migration onto real `/sys/fs/cgroup`.

use std::path::Path;
use std::sync::Arc;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::CgroupFs;
use overdrive_core::traits::driver::{AllocationSpec, AllocationState, Driver, Resources};
use overdrive_host::RealCgroupFs;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_worker::ExecDriver;
use overdrive_worker::cgroup_manager::CgroupManager;
use serial_test::serial;

use super::cleanup::AllocCleanup;

#[tokio::test]
#[serial(cgroup)]
async fn limit_write_failure_warns_and_continues() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    let driver: Arc<dyn Driver> = Arc::new(
        ExecDriver::new(cgroup_root.to_path_buf(), Arc::new(SimClock::new()), fs)
            .with_force_limit_write_failure(true),
    );

    let alloc = AllocationId::new("alloc-limit-write-fail").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/x/alloc/lw")
            .expect("valid spiffe id"),
        command: "/bin/sleep".to_owned(),
        args: vec!["60".to_owned()],
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
