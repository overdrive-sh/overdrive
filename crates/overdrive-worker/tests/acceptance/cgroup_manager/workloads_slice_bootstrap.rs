//! `CgroupManager::create_workloads_slice_with_controllers` performs
//! the two load-bearing writes on the configured cgroup root:
//! `mkdir -p overdrive.slice/workloads.slice` and write
//! `+cpu +memory +io +pids\n` to
//! `workloads.slice/cgroup.subtree_control`.
//!
//! E1 KEEP-AND-MOVE rows 15 + 16 — SimCgroupFs-backed analogue of the
//! pre-refactor inline tempfile tests
//! (`create_workloads_slice_with_controllers_creates_dir_and_writes_subtree_control`
//! + `create_workloads_slice_with_controllers_is_idempotent`).
//!
//! Moved from `src/cgroup_manager.rs` `#[cfg(test)] mod tests` at step
//! 01-07 alongside the async conversion of the bootstrap surface from
//! a sync `std::fs::*` free fn to an async method on `CgroupManager`
//! that routes through `self.fs.*`.
//!
//! PORT-TO-PORT: enters via `CgroupManager::create_workloads_slice_with_controllers`
//! (driving port), asserts at the `SimCgroupFs::snapshot()` byte-store
//! boundary (the port's `CgroupFs::write` driven-port surface). No
//! direct `std::fs::*` or `tokio::fs::*` reads.

use std::path::PathBuf;
use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_sim::{SimCgroupFs, SimEntry};
use overdrive_worker::cgroup_manager::CgroupManager;

/// E1 row 15 — `create_workloads_slice_with_controllers` creates the
/// workloads slice directory AND writes the canonical four-controller
/// payload to `cgroup.subtree_control`. Pins both side effects on a
/// `SimCgroupFs` snapshot — kills the body→`Ok(())` mutation (skips both
/// writes; neither entry appears).
#[tokio::test]
async fn create_workloads_slice_with_controllers_creates_dir_and_writes_subtree_control() {
    let sim = Arc::new(SimCgroupFs::new());
    let fs: Arc<dyn CgroupFs> = sim.clone();
    let cgroup_root = PathBuf::from("/sys/fs/cgroup");
    let manager = CgroupManager::new(cgroup_root.clone(), fs);

    manager
        .create_workloads_slice_with_controllers()
        .await
        .expect("bootstrap must succeed against SimCgroupFs");

    let snap = sim.snapshot();

    let workloads_slice = cgroup_root.join("overdrive.slice/workloads.slice");
    let (slice_entry, _) =
        snap.get(&workloads_slice).expect("workloads.slice dir must exist on snapshot");
    assert_eq!(
        *slice_entry,
        SimEntry::Dir,
        "workloads.slice path must be a directory entry; got {slice_entry:?}",
    );

    let subtree_control = workloads_slice.join("cgroup.subtree_control");
    let (file_entry, bytes) = snap
        .get(&subtree_control)
        .expect("workloads.slice/cgroup.subtree_control must exist on snapshot");
    assert_eq!(
        *file_entry,
        SimEntry::File,
        "subtree_control path must be a regular file entry; got {file_entry:?}",
    );
    assert_eq!(
        bytes.as_slice(),
        b"+cpu +memory +io +pids\n",
        "subtree_control body must match the canonical four-controller delegation",
    );
}

/// E1 row 16 — bootstrap is idempotent on repeated invocation against
/// the same `CgroupManager`. Asserts (a) both invocations return Ok,
/// and (b) the snapshot is byte-identical after invocations 1 and 2
/// (no spurious mutations between calls).
#[tokio::test]
async fn create_workloads_slice_with_controllers_is_idempotent() {
    let sim = Arc::new(SimCgroupFs::new());
    let fs: Arc<dyn CgroupFs> = sim.clone();
    let manager = CgroupManager::new(PathBuf::from("/sys/fs/cgroup"), fs);

    manager.create_workloads_slice_with_controllers().await.expect("first call must succeed");
    let snap_after_first = sim.snapshot();

    manager
        .create_workloads_slice_with_controllers()
        .await
        .expect("second call must be idempotent");
    let snap_after_second = sim.snapshot();

    assert_eq!(
        snap_after_first, snap_after_second,
        "snapshot must be byte-identical across repeated bootstrap calls",
    );
}
