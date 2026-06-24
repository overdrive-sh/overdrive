//! US-02 Scenario 2.3 — child PID lands in `cgroup.procs`.
//!
//! @real-io — Linux + cgroup v2 required. Asserts that after
//! `Driver::start`, the workload scope's `cgroup.procs` file
//! contains the child's PID.
//!
//! Phase 02 of `fix-cgroup-subtree-control-delegation` migrated this
//! test off `tempfile::TempDir` onto real `/sys/fs/cgroup` — see
//! `docs/feature/fix-cgroup-subtree-control-delegation/bugfix-rca.md`
//! § D for the masking failure mode the migration closes. The
//! `create_workloads_slice_with_controllers` call mirrors the
//! production bootstrap from step 01-02.

use std::path::Path;
use std::sync::Arc;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::CgroupFs;
use overdrive_core::traits::driver::{AllocationSpec, Driver, Resources};
use overdrive_host::RealCgroupFs;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_worker::ExecDriver;
use overdrive_worker::cgroup_manager::CgroupManager;
use serial_test::serial;

use super::cleanup::AllocCleanup;

#[tokio::test]
#[serial(cgroup)]
async fn child_pid_appears_in_cgroup_procs() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    let driver: Arc<dyn Driver> =
        Arc::new(ExecDriver::new(cgroup_root.to_path_buf(), Arc::new(SimClock::new()), fs));

    let alloc = AllocationId::new("alloc-cgroup-procs-test").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/x/alloc/cp")
            .expect("valid spiffe id"),
        command: "/bin/sleep".to_owned(),
        args: vec!["60".to_owned()],
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        // transparent-mtls-enrollment step 04-01 (JOIN-4/JOIN-6): off the
        // mTLS-composed boot gate — no provisioned netns/veth.
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
    };

    let handle = driver.start(&spec).await.expect("start succeeds");
    let pid = handle.pid.expect("pid populated");

    let procs_path =
        cgroup_root.join(format!("overdrive.slice/workloads.slice/{alloc}.scope/cgroup.procs"));
    let contents = std::fs::read_to_string(&procs_path).expect("cgroup.procs readable");

    let pids: Vec<u32> = contents.lines().filter_map(|l| l.trim().parse().ok()).collect();
    assert!(
        pids.contains(&pid),
        "expected pid {pid} in cgroup.procs ({}), got {pids:?}",
        procs_path.display()
    );

    driver.stop(&handle).await.expect("stop succeeds");
}
