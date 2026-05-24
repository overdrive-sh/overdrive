//! Class B `create_dir` scenarios for `SimCgroupFs` per ADR-0054
//! § Sim adapter (step 01-03).
//!
//! PORT-TO-PORT: enters via `Arc<dyn CgroupFs>::create_dir`; observes
//! the resulting state via the test-only `snapshot()` hook on the
//! retained concrete handle (`Arc::clone` of the same underlying
//! state).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_sim::{SimCgroupFs, SimEntry, SimOp};

fn fresh() -> (Arc<SimCgroupFs>, Arc<dyn CgroupFs>) {
    let sim = Arc::new(SimCgroupFs::new());
    let fs: Arc<dyn CgroupFs> = sim.clone();
    (sim, fs)
}

#[tokio::test]
async fn b_create_dir_happy() {
    let (sim, fs) = fresh();
    let path = Path::new("/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-x.scope");

    fs.create_dir(path).await.expect("create_dir Ok");

    let snap = sim.snapshot();
    // mkdir-p semantics: every ancestor synthesised as SimEntry::Dir.
    assert_eq!(
        snap.get(path).map(|(e, _)| *e),
        Some(SimEntry::Dir),
        "leaf directory must exist as Dir entry"
    );
    assert_eq!(
        snap.get(Path::new("/sys/fs/cgroup/overdrive.slice")).map(|(e, _)| *e),
        Some(SimEntry::Dir),
        "intermediate ancestor must exist as Dir entry"
    );
}

#[tokio::test]
async fn b_create_dir_injected_permission_denied() {
    let (sim, fs) = fresh();
    let path = Path::new("/sys/fs/cgroup/overdrive.slice");

    sim.inject_error(
        SimOp::CreateDir,
        PathBuf::from("/sys/fs/cgroup/overdrive.slice"),
        io::ErrorKind::PermissionDenied,
    );

    let err = fs.create_dir(path).await.expect_err("injected error fires");
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);

    let snap = sim.snapshot();
    assert!(!snap.contains_key(path), "injected error must leave state unchanged");
}
