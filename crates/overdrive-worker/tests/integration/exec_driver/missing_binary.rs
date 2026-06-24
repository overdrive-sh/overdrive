//! US-02 Scenario 2.5 — missing binary returns `DriverError` AND
//! does not leave an orphaned cgroup scope behind.
//!
//! @real-io — Linux. Asserts that when the spec's `image` does not
//! resolve, `Driver::start` returns `Err(_)` and the workload scope
//! directory was either never created or was cleaned up.
//!
//! Phase 02 migration: real `/sys/fs/cgroup` per
//! `docs/feature/fix-cgroup-subtree-control-delegation/bugfix-rca.md`
//! § D.

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
async fn missing_binary_does_not_create_cgroup_scope() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    let driver: Arc<dyn Driver> =
        Arc::new(ExecDriver::new(cgroup_root.to_path_buf(), Arc::new(SimClock::new()), fs));

    let alloc = AllocationId::new("alloc-missing-binary").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/x/alloc/mb")
            .expect("valid spiffe id"),
        command: "/this/binary/does/not/exist/anywhere".to_owned(),
        args: vec![],
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        // transparent-mtls-enrollment step 04-01 (JOIN-4/JOIN-6): off the
        // mTLS-composed boot gate — no provisioned netns/veth.
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
    };

    let result = driver.start(&spec).await;
    assert!(result.is_err(), "expected start to fail for missing binary, got {result:?}");

    let scope_dir = cgroup_root.join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));
    assert!(
        !scope_dir.exists(),
        "missing-binary path must not leave an orphaned scope at {}",
        scope_dir.display()
    );
}
