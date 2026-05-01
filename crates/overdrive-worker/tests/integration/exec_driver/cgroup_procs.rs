//! US-02 Scenario 2.3 — child PID lands in `cgroup.procs`.
//!
//! @real-io — Linux + cgroup v2 required. Asserts that after
//! `Driver::start`, the workload scope's `cgroup.procs` file
//! contains the child's PID.

use std::sync::Arc;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::driver::{AllocationSpec, Driver, Resources};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_worker::ExecDriver;
use tempfile::TempDir;

#[tokio::test]
async fn child_pid_appears_in_cgroup_procs() {
    let cgroup_root = TempDir::new().expect("tempdir created");
    std::fs::create_dir_all(cgroup_root.path().join("overdrive.slice/workloads.slice"))
        .expect("workloads.slice created");

    let driver: Arc<dyn Driver> =
        Arc::new(ExecDriver::new(cgroup_root.path().to_path_buf(), Arc::new(SimClock::new())));

    let alloc = AllocationId::new("alloc-cgroup-procs-test").expect("valid alloc id");
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/x/alloc/cp")
            .expect("valid spiffe id"),
        command: "/bin/sleep".to_owned(),
        args: vec!["60".to_owned()],
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
    };

    let handle = driver.start(&spec).await.expect("start succeeds");
    let pid = handle.pid.expect("pid populated");

    let procs_path = cgroup_root
        .path()
        .join(format!("overdrive.slice/workloads.slice/{alloc}.scope/cgroup.procs"));
    let contents = std::fs::read_to_string(&procs_path).expect("cgroup.procs readable");

    let pids: Vec<u32> = contents.lines().filter_map(|l| l.trim().parse().ok()).collect();
    assert!(
        pids.contains(&pid),
        "expected pid {pid} in cgroup.procs ({}), got {pids:?}",
        procs_path.display()
    );

    driver.stop(&handle).await.expect("stop succeeds");
}
