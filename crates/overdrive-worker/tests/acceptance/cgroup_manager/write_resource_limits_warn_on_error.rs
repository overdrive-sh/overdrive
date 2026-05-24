//! `CgroupManager::write_resource_limits_warn_on_error` writes the
//! limit files on the happy path AND swallows write errors per
//! ADR-0026 D9 warn-and-continue disposition.
//!
//! E1 CONVERT row 14 — SimCgroupFs-backed analogue of the pre-refactor
//! `write_resource_limits_warn_on_error_writes_files_on_success`
//! inline tempfile test. The happy-path assertion catches the body→()
//! mutation (mutant skips both writes; cpu.weight does NOT appear).
//! Bonus: a `SimCgroupFs` error injection on `cpu.weight` exercises the
//! warn-and-continue arm and asserts no panic + control returns.

use std::io;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_core::traits::driver::Resources;
use overdrive_sim::{SimCgroupFs, SimEntry, SimOp};
use overdrive_worker::cgroup_manager::{CgroupManager, CgroupPath};

#[tokio::test]
async fn write_resource_limits_warn_on_error_writes_files_on_success() {
    let sim = Arc::new(SimCgroupFs::new());
    let fs: Arc<dyn CgroupFs> = sim.clone();
    let manager = CgroupManager::new(PathBuf::from("/sys/fs/cgroup"), fs);
    let scope = CgroupPath::from_str("overdrive.slice/workloads.slice/alloc-warn-ok-0.scope")
        .expect("valid CgroupPath");
    manager.create_workload_scope(&scope).await.expect("create scope");

    let resources = Resources { cpu_milli: 200, memory_bytes: 1024 * 1024 };
    manager.write_resource_limits_warn_on_error(&scope, &resources).await;

    let snap = sim.snapshot();
    let weight_path = PathBuf::from(
        "/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-warn-ok-0.scope/cpu.weight",
    );
    let (weight_entry, _) = snap.get(&weight_path).expect("cpu.weight must appear on success");
    assert_eq!(*weight_entry, SimEntry::File);

    let memmax_path = PathBuf::from(
        "/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-warn-ok-0.scope/memory.max",
    );
    let (memmax_entry, _) = snap.get(&memmax_path).expect("memory.max must appear on success");
    assert_eq!(*memmax_entry, SimEntry::File);
}

#[tokio::test]
async fn write_resource_limits_warn_on_error_swallows_injected_write_failure() {
    let sim = Arc::new(SimCgroupFs::new());
    let fs: Arc<dyn CgroupFs> = sim.clone();
    let manager = CgroupManager::new(PathBuf::from("/sys/fs/cgroup"), fs);
    let scope = CgroupPath::from_str("overdrive.slice/workloads.slice/alloc-warn-fail-0.scope")
        .expect("valid CgroupPath");
    manager.create_workload_scope(&scope).await.expect("create scope");

    let weight_path = PathBuf::from(
        "/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-warn-fail-0.scope/cpu.weight",
    );
    sim.inject_error(SimOp::Write, weight_path.clone(), io::ErrorKind::PermissionDenied);

    let resources = Resources { cpu_milli: 200, memory_bytes: 1024 * 1024 };
    // Warn-and-continue: returns unit, no panic, no propagation.
    manager.write_resource_limits_warn_on_error(&scope, &resources).await;

    // Injected error on cpu.weight short-circuits the second write
    // — neither file is on the snapshot beyond the directory entry.
    let snap = sim.snapshot();
    assert!(!snap.contains_key(&weight_path), "injected error must not persist cpu.weight");
}
