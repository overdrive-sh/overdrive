//! C-rmdir-auto-reap — `rmdir(2)` on an empty workload scope succeeds
//! and the kernel reaps the synthesised pseudo-files automatically.
//!
//! Tier 3, real-io. Requires Lima sudo.
//!
//! Exercises ADR-0054 § D3 row 5 and pins the assumption baked into
//! `cgroup_manager::remove_workload_scope`'s rustdoc: "the
//! kernel-managed virtual files inside a workload scope cannot be
//! `unlink`ed individually and are reaped automatically by
//! `rmdir(2)`."
//!
//! `SimCgroupFs`'s `remove_dir` returns `DirectoryNotEmpty` if children
//! are present — the in-memory store cannot model the kernel's
//! auto-reap. `RealCgroupFs`'s `remove_dir` (which delegates to
//! `tokio::fs::remove_dir`) succeeds against an "empty" workload scope
//! (no live PIDs, no child cgroup directories) even though the scope
//! technically contains kernel-managed pseudo-files like
//! `cgroup.procs`, `cgroup.events`, `cpu.weight`, `memory.max`, etc.
//!
//! Scenario reference: `docs/feature/cgroup-fs-port/distill/test-scenarios.md`
//! § C-rmdir-auto-reap.

use std::path::Path;
use std::sync::Arc;

use overdrive_core::id::AllocationId;
use overdrive_core::traits::CgroupFs;
use overdrive_host::RealCgroupFs;
use overdrive_worker::cgroup_manager::CgroupManager;
use serial_test::serial;

use super::super::exec_driver::cleanup::AllocCleanup;

#[tokio::test]
#[serial(cgroup)]
async fn empty_scope_rmdir_succeeds_kernel_reaps_pseudo_files() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    let alloc = AllocationId::new("alloc-rmdirC-0").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());

    let scope_dir = cgroup_root.join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));
    fs.create_dir(&scope_dir).await.expect("create alloc scope");

    // Sanity: the kernel synthesised at least one pseudo-file inside
    // the scope (proves the test is meaningful — we are NOT
    // rmdir'ing a literally empty dir).
    let cgroup_procs = scope_dir.join("cgroup.procs");
    assert!(
        cgroup_procs.exists(),
        "expected kernel-synthesised cgroup.procs inside scope; \
         test would be vacuous against a literally empty dir"
    );

    // No live PIDs in cgroup.procs — the scope is "empty" in the
    // cgroup-v2 sense (no enrolled processes, no child cgroups),
    // even though pseudo-files exist.

    // rmdir succeeds — the kernel reaps the pseudo-files
    // automatically.
    fs.remove_dir(&scope_dir).await.expect("rmdir empty workload scope must succeed");

    // The directory no longer exists.
    let meta = tokio::fs::metadata(&scope_dir).await;
    match meta {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        other => panic!("expected scope to be NotFound post-rmdir; got {other:?}"),
    }

    // The pseudo-files we never unlinked are also gone — proves the
    // kernel reaped them as part of the rmdir, not as a side effect
    // of any application code.
    assert!(
        !cgroup_procs.exists(),
        "kernel-synthesised cgroup.procs survived rmdir — \
         auto-reap assumption broken"
    );
}
