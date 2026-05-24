//! `CgroupManager::write_resource_limits` writes both `cpu.weight`
//! (derived via `cpu_weight_for`) and `memory.max`.
//!
//! E1 CONVERT row 13 — SimCgroupFs-backed analogue of the pre-refactor
//! `write_resource_limits_writes_cpu_weight_and_memory_max` inline
//! tempfile test. Pins both writes and the `cpu_weight_for` delegation:
//! 100 mCPU → weight 10; `memory_bytes` flows through verbatim.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_core::traits::driver::Resources;
use overdrive_sim::{SimCgroupFs, SimEntry};
use overdrive_worker::cgroup_manager::{CgroupManager, CgroupPath};

#[tokio::test]
async fn write_resource_limits_writes_cpu_weight_and_memory_max() {
    let sim = Arc::new(SimCgroupFs::new());
    let fs: Arc<dyn CgroupFs> = sim.clone();
    let manager = CgroupManager::new(PathBuf::from("/sys/fs/cgroup"), fs);
    let scope = CgroupPath::from_str("overdrive.slice/workloads.slice/alloc-limits-0.scope")
        .expect("valid CgroupPath");
    manager.create_workload_scope(&scope).await.expect("create scope");

    let resources = Resources { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 };
    let result = manager.write_resource_limits(&scope, &resources).await;
    assert!(result.is_ok(), "write_resource_limits must succeed; got {result:?}");

    let snap = sim.snapshot();
    let weight_path = PathBuf::from(
        "/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-limits-0.scope/cpu.weight",
    );
    let (weight_entry, weight_bytes) = snap.get(&weight_path).expect("cpu.weight written");
    assert_eq!(*weight_entry, SimEntry::File);
    assert_eq!(weight_bytes.as_slice(), b"10\n", "cpu.weight must be cpu_milli/10 = 10");

    let memmax_path = PathBuf::from(
        "/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-limits-0.scope/memory.max",
    );
    let (memmax_entry, memmax_bytes) = snap.get(&memmax_path).expect("memory.max written");
    assert_eq!(*memmax_entry, SimEntry::File);
    assert_eq!(
        memmax_bytes.as_slice(),
        format!("{}\n", 256 * 1024 * 1024).as_bytes(),
        "memory.max must equal memory_bytes",
    );
}
