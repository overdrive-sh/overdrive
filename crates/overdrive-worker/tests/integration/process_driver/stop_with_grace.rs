//! US-02 Scenario 2.6 — `Driver::stop` drives to `Terminated` and
//! removes the workload scope directory after reap.
//!
//! @real-io — Linux. SIGTERM-respecting `/bin/sleep` exits cleanly
//! within the grace window; afterward the scope dir must be gone
//! and `Driver::status` returns `Terminated`.

use std::sync::Arc;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::driver::{AllocationSpec, AllocationState, Driver, Resources};
use overdrive_worker::ProcessDriver;
use tempfile::TempDir;

#[tokio::test]
async fn stop_with_grace_drives_to_terminated_and_removes_scope() {
    let cgroup_root = TempDir::new().expect("tempdir created");
    std::fs::create_dir_all(cgroup_root.path().join("overdrive.slice/workloads.slice"))
        .expect("workloads.slice created");

    let driver: Arc<dyn Driver> = Arc::new(ProcessDriver::new(cgroup_root.path().to_path_buf()));

    let alloc = AllocationId::new("alloc-stop-grace").expect("valid alloc id");
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/x/alloc/sg")
            .expect("valid spiffe id"),
        image: "/bin/sleep".to_owned(),
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
    };

    let handle = driver.start(&spec).await.expect("start succeeds");
    driver.stop(&handle).await.expect("stop succeeds");

    let state = driver.status(&handle).await.expect("status succeeds after stop");
    assert_eq!(state, AllocationState::Terminated);

    let scope_dir =
        cgroup_root.path().join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));
    assert!(
        !scope_dir.exists(),
        "scope directory must be removed after stop, still present at {}",
        scope_dir.display()
    );
}
