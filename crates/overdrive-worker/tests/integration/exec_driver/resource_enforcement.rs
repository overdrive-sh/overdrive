//! US-02 Scenario 2.4 — `cpu.weight` + `memory.max` written from spec.
//!
//! @real-io — Linux + cgroup v2 required. Asserts the cgroup limit
//! files exist and carry the values derived from `Resources`:
//! `cpu.weight = clamp(cpu_milli/10, 1, 10000)`,
//! `memory.max = memory_bytes`.
//!
//! Phase 02 migration: real `/sys/fs/cgroup`. Per the bugfix RCA § D,
//! the previous TempDir-backed shape masked the `subtree_control` bug
//! because tmpfs honoured `O_CREAT` for the synthetic `cpu.weight` /
//! `memory.max` files; under real cgroupfs the kernel itself
//! synthesises those files when the controllers are delegated, and
//! the test now exercises the real delegation path.

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
async fn cpu_weight_and_memory_max_are_written_from_spec() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    let driver: Arc<dyn Driver> =
        Arc::new(ExecDriver::new(cgroup_root.to_path_buf(), Arc::new(SimClock::new()), fs));

    // cpu_milli=2000 -> cpu.weight=200; memory_bytes=128MiB.
    let alloc = AllocationId::new("alloc-resource-enforcement").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/x/alloc/re")
            .expect("valid spiffe id"),
        command: "/bin/sleep".to_owned(),
        args: vec!["60".to_owned()],
        resources: Resources { cpu_milli: 2_000, memory_bytes: 128 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        // transparent-mtls-enrollment step 04-01 (JOIN-4/JOIN-6): off the
        // mTLS-composed boot gate — no provisioned netns/veth.
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
    };

    let handle = driver.start(&spec).await.expect("start succeeds");

    let scope_dir = cgroup_root.join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));

    let cpu_weight = std::fs::read_to_string(scope_dir.join("cpu.weight"))
        .expect("cpu.weight readable")
        .trim()
        .to_owned();
    assert_eq!(cpu_weight, "200", "cpu_milli=2000 -> cpu.weight=200");

    let memory_max = std::fs::read_to_string(scope_dir.join("memory.max"))
        .expect("memory.max readable")
        .trim()
        .to_owned();
    assert_eq!(memory_max, format!("{}", 128 * 1024 * 1024));

    driver.stop(&handle).await.expect("stop succeeds");
}
