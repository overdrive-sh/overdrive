//! US-02 Scenario 2.5 — missing binary returns `DriverError` AND
//! does not leave an orphaned cgroup scope behind.
//!
//! @real-io — Linux. Asserts that when the spec's `image` does not
//! resolve, `Driver::start` returns `Err(_)` and the workload scope
//! directory was either never created or was cleaned up.

use std::sync::Arc;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::driver::{
    AllocationSpec, Driver, Resources,
};
use overdrive_worker::ProcessDriver;
use tempfile::TempDir;

#[tokio::test]
async fn missing_binary_does_not_create_cgroup_scope() {
    let cgroup_root = TempDir::new().expect("tempdir created");
    std::fs::create_dir_all(cgroup_root.path().join("overdrive.slice/workloads.slice"))
        .expect("workloads.slice created");

    let driver: Arc<dyn Driver> =
        Arc::new(ProcessDriver::new(cgroup_root.path().to_path_buf()));

    let alloc = AllocationId::new("alloc-missing-binary").expect("valid alloc id");
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/x/alloc/mb")
            .expect("valid spiffe id"),
        image: "/this/binary/does/not/exist/anywhere".to_owned(),
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
    };

    let result = driver.start(&spec).await;
    assert!(
        result.is_err(),
        "expected start to fail for missing binary, got {result:?}"
    );

    let scope_dir = cgroup_root
        .path()
        .join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));
    assert!(
        !scope_dir.exists(),
        "missing-binary path must not leave an orphaned scope at {}",
        scope_dir.display()
    );
}
