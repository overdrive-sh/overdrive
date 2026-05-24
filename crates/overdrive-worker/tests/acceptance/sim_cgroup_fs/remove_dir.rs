//! Class B `remove_dir` scenarios for `SimCgroupFs` per ADR-0054
//! § Sim adapter (step 01-03).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_sim::{SimCgroupFs, SimOp};

fn fresh() -> (Arc<SimCgroupFs>, Arc<dyn CgroupFs>) {
    let sim = Arc::new(SimCgroupFs::new());
    let fs: Arc<dyn CgroupFs> = sim.clone();
    (sim, fs)
}

#[tokio::test]
async fn b_remove_dir_happy() {
    let (sim, fs) = fresh();
    let path = Path::new("/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-x.scope");

    fs.create_dir(path).await.expect("create_dir Ok");
    fs.remove_dir(path).await.expect("remove_dir Ok");

    let snap = sim.snapshot();
    assert!(!snap.contains_key(path), "remove_dir on leaf must drop entry");
    // mkdir-p ancestors REMAIN — only the leaf was removed (matches
    // `rmdir` shape, not `rm -r`).
    assert!(
        snap.contains_key(Path::new("/sys/fs/cgroup/overdrive.slice")),
        "ancestors not affected by leaf rmdir"
    );
}

#[tokio::test]
async fn b_remove_dir_notfound() {
    let (_sim, fs) = fresh();
    let absent = Path::new("/never-created");

    let err = fs.remove_dir(absent).await.expect_err("absent path must surface NotFound");
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
}

#[tokio::test]
async fn b_remove_dir_directory_not_empty() {
    let (_sim, fs) = fresh();
    let parent = Path::new("/sys/fs/cgroup/overdrive.slice");
    let child = parent.join("workloads.slice");

    fs.create_dir(&child).await.expect("create child");
    // child exists -> parent is non-empty (has a transitive child).
    let err =
        fs.remove_dir(parent).await.expect_err("non-empty parent must surface DirectoryNotEmpty");
    assert_eq!(err.kind(), io::ErrorKind::DirectoryNotEmpty);
}

#[tokio::test]
async fn b_remove_dir_injected_permission_denied() {
    let (sim, fs) = fresh();
    let path = Path::new("/sys/fs/cgroup/overdrive.slice");

    fs.create_dir(path).await.expect("create_dir Ok");
    sim.inject_error(
        SimOp::RemoveDir,
        PathBuf::from("/sys/fs/cgroup/overdrive.slice"),
        io::ErrorKind::PermissionDenied,
    );

    let err = fs.remove_dir(path).await.expect_err("injected PermissionDenied fires");
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);

    let snap = sim.snapshot();
    assert!(snap.contains_key(path), "injected error leaves entry intact");
}
