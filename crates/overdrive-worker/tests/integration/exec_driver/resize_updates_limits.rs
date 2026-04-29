//! Linux-only integration test for `Driver::resize`.
//!
//! Pins the resize → cgroup write delegation. Kills the mutation
//! `<impl Driver for ExecDriver>::resize -> Result<(),
//! DriverError> with Ok(())` — under the mutation, `resize` skips
//! the `write_resource_limits_warn_on_error` call entirely, so
//! `cpu.weight` and `memory.max` retain their original values.
//! Asserting on the post-resize file contents catches the missing
//! write.

use std::sync::Arc;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::driver::{AllocationSpec, Driver, Resources};
use overdrive_worker::ExecDriver;
use tempfile::TempDir;

#[tokio::test]
async fn resize_updates_cpu_weight_and_memory_max_in_cgroup() {
    let cgroup_root = TempDir::new().expect("tempdir created");
    std::fs::create_dir_all(cgroup_root.path().join("overdrive.slice/workloads.slice"))
        .expect("workloads.slice created");

    let driver: Arc<dyn Driver> = Arc::new(ExecDriver::new(cgroup_root.path().to_path_buf()));

    let alloc = AllocationId::new("alloc-resize-test").expect("valid alloc id");
    let initial_spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/x/alloc/rz")
            .expect("valid spiffe id"),
        command: "/bin/sleep".to_owned(),
        args: vec!["60".to_owned()],
        resources: Resources { cpu_milli: 1_000, memory_bytes: 64 * 1024 * 1024 },
    };

    let handle = driver.start(&initial_spec).await.expect("start succeeds");

    let scope_dir =
        cgroup_root.path().join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));

    // Pre-resize values.
    let pre_weight = std::fs::read_to_string(scope_dir.join("cpu.weight"))
        .expect("pre-resize cpu.weight readable")
        .trim()
        .to_owned();
    assert_eq!(pre_weight, "100", "initial cpu_milli=1000 -> cpu.weight=100");

    // Resize to a new envelope. Production: write_resource_limits
    // overwrites cpu.weight to 400 and memory.max to 256 MiB.
    // Mutant `body → Ok(())`: skips the write; files retain "100"
    // and 64 MiB respectively.
    let new_resources = Resources { cpu_milli: 4_000, memory_bytes: 256 * 1024 * 1024 };
    driver.resize(&handle, new_resources).await.expect("resize succeeds");

    let post_weight = std::fs::read_to_string(scope_dir.join("cpu.weight"))
        .expect("post-resize cpu.weight readable")
        .trim()
        .to_owned();
    assert_eq!(
        post_weight, "400",
        "after resize cpu_milli=4000 -> cpu.weight=400; mutant `Ok(())` would leave 100",
    );

    let post_memmax = std::fs::read_to_string(scope_dir.join("memory.max"))
        .expect("post-resize memory.max readable")
        .trim()
        .to_owned();
    assert_eq!(
        post_memmax,
        format!("{}", 256 * 1024 * 1024),
        "after resize memory_bytes=256 MiB; mutant `Ok(())` would leave 64 MiB",
    );

    driver.stop(&handle).await.expect("stop succeeds");
}
