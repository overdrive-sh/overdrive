//! `CgroupManager::place_pid_in_scope` writes `"{pid}\n"` to
//! `<scope>/cgroup.procs`.
//!
//! E1 CONVERT row 12 — SimCgroupFs-backed analogue of the pre-refactor
//! `place_pid_in_scope_writes_pid_to_cgroup_procs` inline tempfile
//! test. Kills the body→Ok(()) mutation — the mutant skips the
//! `write` call; cgroup.procs does NOT appear on the snapshot.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_sim::{SimCgroupFs, SimEntry};
use overdrive_worker::cgroup_manager::{CgroupManager, CgroupPath};

#[tokio::test]
async fn place_pid_in_scope_writes_pid_to_cgroup_procs() {
    let sim = Arc::new(SimCgroupFs::new());
    let fs: Arc<dyn CgroupFs> = sim.clone();
    let manager = CgroupManager::new(PathBuf::from("/sys/fs/cgroup"), fs);
    let scope = CgroupPath::from_str("overdrive.slice/workloads.slice/alloc-place-0.scope")
        .expect("valid CgroupPath");
    // Parent must exist for write() to succeed.
    manager.create_workload_scope(&scope).await.expect("create scope");

    let result = manager.place_pid_in_scope(&scope, 1234).await;
    assert!(result.is_ok(), "place_pid_in_scope must succeed; got {result:?}");

    let snap = sim.snapshot();
    let procs_path = PathBuf::from(
        "/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-place-0.scope/cgroup.procs",
    );
    let (entry, bytes) = snap.get(&procs_path).expect("cgroup.procs must be written");
    assert_eq!(*entry, SimEntry::File);
    assert_eq!(bytes.as_slice(), b"1234\n", "cgroup.procs must contain the pid + newline");
}
