//! `CgroupManager::cgroup_kill` is idempotent on a missing scope.
//!
//! E1 CONVERT row 6 — SimCgroupFs-backed analogue of the pre-refactor
//! `cgroup_kill_is_idempotent_on_missing_scope` inline tempfile test.
//! Asserts the `NotFound` arm of the match in
//! `CgroupManager::cgroup_kill` returns `Ok(())`.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_sim::SimCgroupFs;
use overdrive_worker::cgroup_manager::{CgroupManager, CgroupPath};

#[tokio::test]
async fn cgroup_kill_is_idempotent_on_missing_scope() {
    let sim = Arc::new(SimCgroupFs::new());
    let fs: Arc<dyn CgroupFs> = sim.clone();
    let manager = CgroupManager::new(PathBuf::from("/sys/fs/cgroup"), fs);
    let scope = CgroupPath::from_str("overdrive.slice/workloads.slice/alloc-missing-0.scope")
        .expect("valid CgroupPath");

    let result = manager.cgroup_kill(&scope).await;
    assert!(
        result.is_ok(),
        "cgroup_kill on a missing scope must be idempotent (Ok); got {result:?}",
    );

    // The cgroup.kill file was never created — the NotFound arm
    // returns Ok without writing.
    let snap = sim.snapshot();
    let kill_path = PathBuf::from(
        "/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-missing-0.scope/cgroup.kill",
    );
    assert!(!snap.contains_key(&kill_path), "idempotent path must not write cgroup.kill");
}
