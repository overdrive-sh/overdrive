//! US-02 Scenario 2.2 — WALKING SKELETON @real-io @adapter-integration.
//!
//! `ExecDriver` starts a real `/bin/sleep` and places it under a
//! workload-scope directory. PORT-TO-PORT: enters via `Driver::start`,
//! asserts on the returned PID and on the scope directory's existence
//! under the test cgroup-root.
//!
//! Linux-only: requires cgroup v2 mounted at `/sys/fs/cgroup`.
//!
//! Phase 02 of `fix-cgroup-subtree-control-delegation` migrated this
//! test off `tempfile::TempDir` onto real `/sys/fs/cgroup` — see
//! `docs/feature/fix-cgroup-subtree-control-delegation/bugfix-rca.md`
//! § D. The bootstrap call mirrors production wiring from step 01-02.

use std::path::Path;
use std::sync::Arc;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::driver::{AllocationSpec, AllocationState, Driver, Resources};
use overdrive_host::RealCgroupFs;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_worker::ExecDriver;
use overdrive_worker::cgroup_manager::create_workloads_slice_with_controllers;
use serial_test::serial;

use super::cleanup::AllocCleanup;

#[tokio::test]
#[serial(cgroup)]
async fn exec_driver_starts_real_sleep_in_cgroup_scope() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    create_workloads_slice_with_controllers(cgroup_root)
        .expect("workloads.slice bootstrap succeeds");

    let driver: Arc<dyn Driver> = Arc::new(ExecDriver::new(
        cgroup_root.to_path_buf(),
        Arc::new(SimClock::new()),
        Arc::new(RealCgroupFs::new()),
    ));

    let alloc = AllocationId::new("alloc-walking-skeleton-2-2").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/sleep/alloc/ws22")
            .expect("valid spiffe id"),
        command: "/bin/sleep".to_owned(),
        args: vec!["60".to_owned()],
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
    };

    // Action — through driving port.
    let handle = driver.start(&spec).await.expect("ExecDriver::start succeeds for /bin/sleep");

    // Observable outcome 1 — handle carries the live PID.
    let pid = handle.pid.expect("ExecDriver populates pid on start");
    assert!(pid > 0, "pid must be positive, got {pid}");

    // Observable outcome 2 — driver reports `Running`.
    let state = driver.status(&handle).await.expect("status query succeeds");
    assert_eq!(state, AllocationState::Running, "freshly started process is Running");

    // Observable outcome 3 — the workload scope dir exists under
    // /sys/fs/cgroup.
    let scope_dir = cgroup_root.join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));
    assert!(scope_dir.exists(), "workload scope directory must exist at {}", scope_dir.display());

    driver.stop(&handle).await.expect("stop succeeds");
}
