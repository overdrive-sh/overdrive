//! C-procs-pid-movement — writing a PID to `<scope>/cgroup.procs`
//! actually moves the process from its current cgroup INTO the
//! target scope. `/proc/<pid>/cgroup` reflects the new path.
//!
//! Tier 3, real-io. Requires Lima sudo.
//!
//! Exercises ADR-0054 § D3 row 6. The
//! `cgroup_manager::place_pid_in_scope` method depends on this
//! kernel-side semantic. `SimCgroupFs` only stores the byte payload at
//! the in-memory `cgroup.procs` path; no PID movement occurs because
//! no real PIDs exist.
//!
//! Scenario reference: `docs/feature/cgroup-fs-port/distill/test-scenarios.md`
//! § C-procs-pid-movement.

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
async fn writing_pid_to_cgroup_procs_moves_the_process_into_scope() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    let alloc = AllocationId::new("alloc-procsC-0").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());

    let scope_dir = cgroup_root.join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));
    fs.create_dir(&scope_dir).await.expect("create alloc scope");

    // Spawn `/bin/sleep 3600` in the test process's cgroup (NOT yet
    // in `<scope>`). The cleanup guard reaps the PID via
    // `cgroup.kill` + `rmdir` after the test body finishes.
    let child =
        tokio::process::Command::new("/bin/sleep").arg("3600").spawn().expect("spawn /bin/sleep");
    let pid = child.id().expect("child PID populated");

    // Move the PID into the target scope.
    fs.write(&scope_dir.join("cgroup.procs"), format!("{pid}\n").as_bytes())
        .await
        .expect("cgroup.procs write must succeed");

    // Read `/proc/<pid>/cgroup` — cgroup v2 shows a single line
    // `0::<path>` where <path> is the absolute cgroup path (rooted
    // at the cgroup hierarchy, NOT including the `/sys/fs/cgroup`
    // prefix).
    let proc_path = format!("/proc/{pid}/cgroup");
    let body = tokio::fs::read_to_string(&proc_path).await.expect("/proc/<pid>/cgroup readable");

    // The cgroup v2 line has form `0::<path>`. Compute the expected
    // suffix without the /sys/fs/cgroup prefix.
    let expected_suffix = format!("overdrive.slice/workloads.slice/{alloc}.scope");
    let cgroup_v2_line = body
        .lines()
        .find(|l| l.starts_with("0::"))
        .expect("/proc/<pid>/cgroup must contain a cgroup v2 line");
    assert!(
        cgroup_v2_line.ends_with(&expected_suffix),
        "expected /proc/{pid}/cgroup v2 line to end with `{expected_suffix}`; \
         got line={cgroup_v2_line:?} (full body: {body:?})"
    );
}
