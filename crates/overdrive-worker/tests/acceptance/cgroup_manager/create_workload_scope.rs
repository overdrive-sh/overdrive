//! `CgroupManager::create_workload_scope` writes a directory via the
//! `CgroupFs::create_dir` port.
//!
//! E1 CONVERT row 11 — SimCgroupFs-backed analogue of the pre-refactor
//! `create_workload_scope_writes_a_real_directory` inline tempfile
//! test. Kills the body→Ok(()) mutation — the mutant skips the
//! `create_dir` call entirely; the directory does NOT appear on the
//! snapshot.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_sim::{SimCgroupFs, SimEntry};
use overdrive_worker::cgroup_manager::{CgroupManager, CgroupPath};

#[tokio::test]
async fn create_workload_scope_writes_a_real_directory() {
    let sim = Arc::new(SimCgroupFs::new());
    let fs: Arc<dyn CgroupFs> = sim.clone();
    let manager = CgroupManager::new(PathBuf::from("/sys/fs/cgroup"), fs);
    let scope = CgroupPath::from_str("overdrive.slice/workloads.slice/alloc-create-0.scope")
        .expect("valid CgroupPath");

    let result = manager.create_workload_scope(&scope).await;
    assert!(result.is_ok(), "create_workload_scope must succeed; got {result:?}");

    let snap = sim.snapshot();
    let scope_path =
        PathBuf::from("/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-create-0.scope");
    let (entry, _) = snap.get(&scope_path).expect("scope dir must exist on snapshot");
    assert_eq!(*entry, SimEntry::Dir, "scope path must be a directory entry");
}
