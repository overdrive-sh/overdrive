//! Step 01-02 — idempotency regression for the cgroup `subtree_control`
//! delegation production fix.
//!
//! Per AC4 of step 01-02 (and `docs/feature/fix-cgroup-subtree-control-
//! delegation/bugfix-rca.md` § "Production fix #1 — Idempotent"): both
//! `create_and_enrol_control_plane_slice_at` and the new
//! `create_workloads_slice_with_controllers` MUST be idempotent across
//! repeated boots. The kernel accepts a re-enable write
//! (`+cpu +memory +io +pids` to `cgroup.subtree_control` whose
//! controllers are already enabled) as a no-op; the second call MUST
//! NOT fail.
//!
//! The test calls each init function twice in succession on the same
//! `cgroup_root` (`/sys/fs/cgroup` under Lima) and asserts both calls
//! return Ok. This exercises the production fix's ordering invariant
//! AND the idempotent-re-enable kernel contract together — a regression
//! that introduced a "create-only-on-fresh" branch (e.g. by reading
//! `cgroup.subtree_control` first and skipping the write when
//! non-empty) would still pass on a fresh boot but fail on a second
//! boot of the same process supervisor.
//!
//! Runs against real `/sys/fs/cgroup`, so it requires root + cgroup
//! delegation; gated `integration-tests` and invoked through
//! `cargo xtask lima run --` per
//! `.claude/rules/testing.md` § "Cgroup writes need root or delegation".

#![cfg(target_os = "linux")]

use std::path::Path;
use std::sync::Arc;

use overdrive_control_plane::cgroup_manager::create_and_enrol_control_plane_slice_at;
use overdrive_core::traits::CgroupFs;
use overdrive_host::RealCgroupFs;
use overdrive_worker::cgroup_manager::CgroupManager;
use serial_test::serial;

/// AC4 — both inits are idempotent under repeated boot. A second call
/// to either function on the same `cgroup_root` MUST return Ok with no
/// observable side effects beyond the first call's. The kernel-level
/// contract: re-writing `+cpu +memory +io +pids` to an already-enabled
/// `cgroup.subtree_control` is a no-op.
#[tokio::test]
#[serial(cgroup)]
async fn subtree_control_delegation_is_idempotent_across_boots() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let pid = std::process::id();
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    let manager = CgroupManager::new(cgroup_root.to_path_buf(), fs);

    // First call — establishes the slice + delegates controllers.
    create_and_enrol_control_plane_slice_at(cgroup_root, pid)
        .expect("first control-plane init must succeed");
    manager
        .create_workloads_slice_with_controllers()
        .await
        .expect("first workloads-slice init must succeed");

    // Second call — MUST be a no-op, not a regression. Validates
    // both that `mkdir -p` semantics survive and that the
    // `+cpu +memory +io +pids` re-write is accepted by the kernel.
    create_and_enrol_control_plane_slice_at(cgroup_root, pid)
        .expect("second control-plane init must be idempotent");
    manager
        .create_workloads_slice_with_controllers()
        .await
        .expect("second workloads-slice init must be idempotent");

    // Verify the post-state matches the production-fix contract: the
    // overdrive.slice/cgroup.subtree_control file lists at minimum
    // `cpu` and `memory` (plus `io` and `pids` per AC2's symmetry
    // choice) — without these, the workloads.slice child would have
    // no `cpu.weight` / `memory.max` interface files, which is the
    // exact bug the fix addresses.
    let overdrive_subtree_control =
        std::fs::read_to_string(cgroup_root.join("overdrive.slice/cgroup.subtree_control"))
            .expect("overdrive.slice/cgroup.subtree_control readable");
    let controllers: std::collections::BTreeSet<&str> =
        overdrive_subtree_control.split_ascii_whitespace().collect();
    assert!(
        controllers.contains("cpu") && controllers.contains("memory"),
        "overdrive.slice/cgroup.subtree_control must list cpu+memory after \
         init (got {overdrive_subtree_control:?})",
    );

    let workloads_subtree_control = std::fs::read_to_string(
        cgroup_root.join("overdrive.slice/workloads.slice/cgroup.subtree_control"),
    )
    .expect("workloads.slice/cgroup.subtree_control readable");
    let controllers: std::collections::BTreeSet<&str> =
        workloads_subtree_control.split_ascii_whitespace().collect();
    assert!(
        controllers.contains("cpu") && controllers.contains("memory"),
        "workloads.slice/cgroup.subtree_control must list cpu+memory after \
         init (got {workloads_subtree_control:?})",
    );
}
