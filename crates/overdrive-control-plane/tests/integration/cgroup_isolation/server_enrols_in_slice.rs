//! Step 03-01 / Slice 4 scenario 4.1 —
//! `server_enrols_pid_in_control_plane_slice`.
//!
//! Per ADR-0029: the control-plane process owns
//! `overdrive.slice/control-plane.slice/`. After
//! `create_and_enrol_control_plane_slice_at` returns, the directory
//! exists under the cgroup root AND `cgroup.procs` carries the PID
//! that was passed in.
//!
//! Tested through the public test seam
//! (`create_and_enrol_control_plane_slice_at`) so the harness can
//! point at a `tempfile::TempDir`-backed cgroupfs analogue without
//! mutating the real `/sys/fs/cgroup` tree under the test runner's
//! UID. Production wires the same function with `/sys/fs/cgroup`
//! and `getpid()`.

#![cfg(target_os = "linux")]

use overdrive_control_plane::cgroup_manager::{
    CONTROL_PLANE_SLICE, create_and_enrol_control_plane_slice_at,
};
use tempfile::TempDir;

#[test]
fn server_enrols_pid_in_control_plane_slice() {
    let tmp = TempDir::new().expect("tempdir");
    let cgroup_root = tmp.path();
    let pid = 12_345;

    create_and_enrol_control_plane_slice_at(cgroup_root, pid).expect("enrol must succeed");

    let slice_dir = cgroup_root.join(CONTROL_PLANE_SLICE);
    assert!(slice_dir.is_dir(), "control-plane slice dir must exist at {}", slice_dir.display());

    let procs_path = slice_dir.join("cgroup.procs");
    assert!(procs_path.exists(), "cgroup.procs must exist at {}", procs_path.display());

    let procs = std::fs::read_to_string(&procs_path).expect("read cgroup.procs");
    assert!(
        procs.split_ascii_whitespace().any(|tok| tok == pid.to_string()),
        "cgroup.procs ({procs:?}) must carry pid={pid}"
    );
}
