//! Step 03-01 / Slice 4 scenario 4.3 —
//! `slice_creation_is_idempotent_across_boots`.
//!
//! Per ADR-0029: a second boot reuses the existing slice rather
//! than failing. `create_and_enrol_control_plane_slice_at` uses
//! `mkdir -p` semantics on the directory and a fresh write to
//! `cgroup.procs` — neither operation must fail when the slice
//! already exists.

#![cfg(target_os = "linux")]

use overdrive_control_plane::cgroup_manager::{
    CONTROL_PLANE_SLICE, create_and_enrol_control_plane_slice_at,
};
use tempfile::TempDir;

#[test]
fn slice_creation_is_idempotent_across_boots() {
    let tmp = TempDir::new().expect("tempdir");
    let cgroup_root = tmp.path();
    let pid_first = 11_111;
    let pid_second = 22_222;

    // First boot — creates the slice and enrols pid_first.
    create_and_enrol_control_plane_slice_at(cgroup_root, pid_first).expect("first boot must enrol");
    let slice_dir = cgroup_root.join(CONTROL_PLANE_SLICE);
    assert!(slice_dir.is_dir(), "slice dir must exist after first boot");

    // Second boot — slice dir already exists; the call must NOT fail
    // and the most recent PID must be observable in cgroup.procs.
    create_and_enrol_control_plane_slice_at(cgroup_root, pid_second)
        .expect("second boot must NOT fail (idempotent)");

    let procs = std::fs::read_to_string(slice_dir.join("cgroup.procs")).expect("read procs");
    assert!(
        procs.split_ascii_whitespace().any(|tok| tok == pid_second.to_string()),
        "cgroup.procs after second boot ({procs:?}) must carry pid_second={pid_second}"
    );
}
