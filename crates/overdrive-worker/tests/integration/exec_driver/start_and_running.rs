//! US-02 Scenario 2.2 — WALKING SKELETON @real-io @adapter-integration.
//!
//! `ExecDriver` starts a real `/bin/sleep` and places it under a
//! workload-scope directory. PORT-TO-PORT: enters via `Driver::start`,
//! asserts on the returned PID and on the scope directory's existence
//! under the test cgroup-root.
//!
//! Linux-only: requires cgroup v2 mounted at `/sys/fs/cgroup`. The
//! test passes a tempdir as `cgroup_root` so the test does not
//! pollute the host's real cgroup tree.

use std::sync::Arc;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::driver::{AllocationSpec, AllocationState, Driver, Resources};
use overdrive_worker::ExecDriver;
use tempfile::TempDir;

#[tokio::test]
async fn exec_driver_starts_real_sleep_in_cgroup_scope() {
    // Test fixture — point cgroup_root at a tempdir so the test
    // does not write under the real `/sys/fs/cgroup`. The slices
    // below are pre-created in the tempdir to mimic the host's
    // `overdrive.slice/workloads.slice` skeleton.
    let cgroup_root = TempDir::new().expect("tempdir created");
    std::fs::create_dir_all(cgroup_root.path().join("overdrive.slice/workloads.slice"))
        .expect("workloads.slice created");

    let driver: Arc<dyn Driver> = Arc::new(ExecDriver::new(cgroup_root.path().to_path_buf()));

    let alloc = AllocationId::new("alloc-walking-skeleton-2-2").expect("valid alloc id");
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

    // Observable outcome 3 — the workload scope dir exists under the
    // tempdir cgroup root.
    let scope_dir =
        cgroup_root.path().join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));
    assert!(scope_dir.exists(), "workload scope directory must exist at {}", scope_dir.display());

    // Cleanup — stop the process, drop tempdir.
    driver.stop(&handle).await.expect("stop succeeds");
}
