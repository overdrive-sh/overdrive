//! US-02 Scenario 2.4 — `cpu.weight` + `memory.max` written from spec.
//!
//! @real-io — Linux + cgroup v2 required. Asserts the cgroup limit
//! files exist and carry the values derived from `Resources`:
//! `cpu.weight = clamp(cpu_milli/10, 1, 10000)`, `memory.max = memory_bytes`.

use std::sync::Arc;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::driver::{AllocationSpec, Driver, Resources};
use overdrive_worker::ExecDriver;
use tempfile::TempDir;

#[tokio::test]
async fn cpu_weight_and_memory_max_are_written_from_spec() {
    let cgroup_root = TempDir::new().expect("tempdir created");
    std::fs::create_dir_all(cgroup_root.path().join("overdrive.slice/workloads.slice"))
        .expect("workloads.slice created");

    let driver: Arc<dyn Driver> = Arc::new(ExecDriver::new(cgroup_root.path().to_path_buf()));

    // cpu_milli=2000 -> cpu.weight=200; memory_bytes=128MiB.
    let alloc = AllocationId::new("alloc-resource-enforcement").expect("valid alloc id");
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/x/alloc/re")
            .expect("valid spiffe id"),
        command: "/bin/sleep".to_owned(),
        args: vec!["60".to_owned()],
        resources: Resources { cpu_milli: 2_000, memory_bytes: 128 * 1024 * 1024 },
    };

    let handle = driver.start(&spec).await.expect("start succeeds");

    let scope_dir =
        cgroup_root.path().join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));

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
