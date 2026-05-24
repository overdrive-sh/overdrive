//! Class B `B-error-schedule-determinism` scenario for `SimCgroupFs`
//! per ADR-0054 § Sim adapter (step 01-03).
//!
//! Proves the injectable error schedule is queue-FIFO per-(`SimOp`,
//! `PathBuf`) and the queue iteration order is deterministic across
//! runs (`BTreeMap`-keyed schedule => `Ord`-deterministic iteration).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_sim::{SimCgroupFs, SimOp};

#[tokio::test]
async fn b_error_schedule_determinism() {
    let sim = Arc::new(SimCgroupFs::new());
    let fs: Arc<dyn CgroupFs> = sim.clone();

    let parent = Path::new("/sys/fs/cgroup/overdrive.slice");
    let file = parent.join("cgroup.subtree_control");
    fs.create_dir(parent).await.expect("create parent");

    // Queue three errors for the same (Write, file) key in a known
    // order; pop them via three sequential `write` calls and verify
    // the surfaced kinds match the injection order.
    sim.inject_error(SimOp::Write, file.clone(), io::ErrorKind::PermissionDenied);
    sim.inject_error(SimOp::Write, file.clone(), io::ErrorKind::Other);
    sim.inject_error(SimOp::Write, file.clone(), io::ErrorKind::InvalidInput);

    let kinds: Vec<io::ErrorKind> = vec![
        fs.write(&file, b"a").await.unwrap_err().kind(),
        fs.write(&file, b"b").await.unwrap_err().kind(),
        fs.write(&file, b"c").await.unwrap_err().kind(),
    ];
    assert_eq!(
        kinds,
        vec![io::ErrorKind::PermissionDenied, io::ErrorKind::Other, io::ErrorKind::InvalidInput,],
        "per-(op,path) schedule is FIFO"
    );

    // Queue drained; the next call proceeds normally.
    fs.write(&file, b"d").await.expect("queue drained -> Ok");

    // Distinct-path injections do not bleed across keys.
    let other_file = parent.join("cgroup.kill");
    sim.inject_error(SimOp::Write, other_file.clone(), io::ErrorKind::PermissionDenied);
    sim.inject_error(SimOp::Write, file.clone(), io::ErrorKind::Other);

    // Calling against `file` pops `file`'s error, not `other_file`'s.
    let err = fs.write(&file, b"e").await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
    // `other_file`'s injection is still pending.
    let err = fs.write(&other_file, b"f").await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);

    // Schedule replay: a fresh sim handed the same injection sequence
    // and the same call sequence produces the same kinds in the same
    // order. BTreeMap-deterministic iteration is what guarantees this.
    let sim_b = Arc::new(SimCgroupFs::new());
    let fs_b: Arc<dyn CgroupFs> = sim_b.clone();
    fs_b.create_dir(parent).await.expect("create parent b");
    let file = PathBuf::from("/sys/fs/cgroup/overdrive.slice/cgroup.subtree_control");
    sim_b.inject_error(SimOp::Write, file.clone(), io::ErrorKind::PermissionDenied);
    sim_b.inject_error(SimOp::Write, file.clone(), io::ErrorKind::Other);
    let kinds_b: Vec<io::ErrorKind> = vec![
        fs_b.write(&file, b"a").await.unwrap_err().kind(),
        fs_b.write(&file, b"b").await.unwrap_err().kind(),
    ];
    assert_eq!(
        kinds_b,
        vec![io::ErrorKind::PermissionDenied, io::ErrorKind::Other],
        "replay yields identical kind sequence"
    );
}
