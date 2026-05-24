//! `CgroupManager::cgroup_kill` writes `"1\n"` to `<scope>/cgroup.kill`
//! on the happy path.
//!
//! E1 CONVERT row 7 — SimCgroupFs-backed analogue of the pre-refactor
//! `cgroup_kill_writes_one_to_cgroup_kill_file` inline tempfile test.
//! Pins the body of `CgroupManager::cgroup_kill`: kernel cgroup.kill
//! protocol mandates exactly `b"1\n"`.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_sim::{SimCgroupFs, SimEntry};
use overdrive_worker::cgroup_manager::{CgroupManager, CgroupPath};

#[tokio::test]
async fn cgroup_kill_writes_one_newline_to_cgroup_kill_file() {
    let sim = Arc::new(SimCgroupFs::new());
    let fs: Arc<dyn CgroupFs> = sim.clone();
    let manager = CgroupManager::new(PathBuf::from("/sys/fs/cgroup"), fs);
    let scope = CgroupPath::from_str("overdrive.slice/workloads.slice/alloc-kill-write-0.scope")
        .expect("valid CgroupPath");

    // Pre-create the scope directory so the write() succeeds — Sim
    // enforces parent-existence per ADR-0054 § Trait contract.
    manager.create_workload_scope(&scope).await.expect("create scope");

    manager.cgroup_kill(&scope).await.expect("cgroup_kill on existing scope must Ok");

    let snap = sim.snapshot();
    let kill_path = PathBuf::from(
        "/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-kill-write-0.scope/cgroup.kill",
    );
    let (entry, bytes) = snap.get(&kill_path).expect("cgroup.kill must be written");
    assert_eq!(*entry, SimEntry::File);
    assert_eq!(
        bytes.as_slice(),
        b"1\n",
        "cgroup_kill must write `1\\n` per kernel cgroup.kill protocol",
    );
}
