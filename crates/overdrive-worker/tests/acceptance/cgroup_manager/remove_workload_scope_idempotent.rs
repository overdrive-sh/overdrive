//! `CgroupManager::remove_workload_scope` is idempotent on a missing
//! scope.
//!
//! E1 CONVERT row 9 — SimCgroupFs-backed analogue of the pre-refactor
//! `remove_workload_scope_is_idempotent_on_missing_scope` inline
//! tempfile test. Asserts the `NotFound` arm of the match in
//! `CgroupManager::remove_workload_scope` returns `Ok(())`.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_sim::SimCgroupFs;
use overdrive_worker::cgroup_manager::{CgroupManager, CgroupPath};

#[tokio::test]
async fn remove_workload_scope_is_idempotent_on_missing_scope() {
    let sim = Arc::new(SimCgroupFs::new());
    let fs: Arc<dyn CgroupFs> = sim.clone();
    let manager = CgroupManager::new(PathBuf::from("/sys/fs/cgroup"), fs);
    let scope = CgroupPath::from_str("overdrive.slice/workloads.slice/alloc-missing-1.scope")
        .expect("valid CgroupPath");

    let result = manager.remove_workload_scope(&scope).await;
    assert!(
        result.is_ok(),
        "remove_workload_scope on missing scope must be idempotent; got {result:?}",
    );

    // No directory was created — snapshot remains empty.
    let snap = sim.snapshot();
    let scope_path =
        PathBuf::from("/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-missing-1.scope");
    assert!(!snap.contains_key(&scope_path), "idempotent path must not create the scope dir");
}
