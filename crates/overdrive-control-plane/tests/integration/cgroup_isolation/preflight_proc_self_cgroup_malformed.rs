//! Step 01-02 (bugfix `fix-cgroup-preflight-wrong-slice`) — coverage
//! for the new `CgroupPathDiscoveryFailed` error variant.
//!
//! Per ADR-0028 §4 step 4, the enclosing slice is discovered by
//! reading `/proc/self/cgroup` and parsing the cgroup-v2 line of the
//! shape `0::/path/to/slice`. Two failure shapes both surface as
//! `CgroupPathDiscoveryFailed`:
//!
//! 1. The file lists only cgroup v1 hierarchy lines (no `0::` line).
//!    A cgroup-v1-only host that somehow slipped past step 1 (e.g. a
//!    tampered `/proc/filesystems`) — extremely rare, but a real
//!    failure mode.
//! 2. The file is empty — likely a /proc mount issue or a
//!    namespaced-init shape with an unusual layout.
//!
//! Both cases must produce the actionable
//! `CgroupPathDiscoveryFailed` rendering: it names the failure, the
//! detected condition, the `--allow-no-cgroups` escape hatch, and the
//! docs URL.

#![cfg(target_os = "linux")]

use overdrive_control_plane::cgroup_preflight::{CgroupPreflightError, run_preflight_at};
use tempfile::TempDir;

#[test]
fn preflight_rejects_v1_only_proc_self_cgroup() {
    let tmp = TempDir::new().expect("tempdir");
    let cgroup_root = tmp.path();

    // Step 1 must pass: /proc/filesystems lists cgroup2.
    let proc_fs = tmp.path().join("filesystems");
    std::fs::write(&proc_fs, "nodev\tcgroup2\n").expect("write proc/filesystems");

    // Step 2 must pass: cgroup.controllers exists.
    std::fs::write(cgroup_root.join("cgroup.controllers"), "cpu memory io\n")
        .expect("write cgroup.controllers");

    // /proc/self/cgroup analogue with ONLY cgroup v1 hierarchy lines
    // (no `0::` line). Step 4 cannot determine the enclosing slice.
    let proc_self_cgroup = tmp.path().join("proc-self-cgroup");
    std::fs::write(&proc_self_cgroup, "12:cpu:/foo\n11:memory:/foo\n")
        .expect("write proc/self/cgroup");

    let err = run_preflight_at(cgroup_root, /* uid = */ 1000, &proc_fs, &proc_self_cgroup)
        .expect_err(
            "preflight must refuse: /proc/self/cgroup has no cgroup-v2 (`0::`) line, \
             so the enclosing slice cannot be discovered",
        );

    let msg = err.to_string();
    match err {
        CgroupPreflightError::CgroupPathDiscoveryFailed { .. } => {}
        other => {
            panic!("expected CgroupPathDiscoveryFailed (v1-only /proc/self/cgroup); got {other:?}")
        }
    }

    assert!(msg.contains("--allow-no-cgroups"), "must mention --allow-no-cgroups: {msg}");
    assert!(msg.contains("docs.overdrive.sh"), "must mention docs URL: {msg}");
}

#[test]
fn preflight_rejects_empty_proc_self_cgroup() {
    let tmp = TempDir::new().expect("tempdir");
    let cgroup_root = tmp.path();

    let proc_fs = tmp.path().join("filesystems");
    std::fs::write(&proc_fs, "nodev\tcgroup2\n").expect("write proc/filesystems");
    std::fs::write(cgroup_root.join("cgroup.controllers"), "cpu memory io\n")
        .expect("write cgroup.controllers");

    // /proc/self/cgroup analogue is empty — no lines at all.
    let proc_self_cgroup = tmp.path().join("proc-self-cgroup");
    std::fs::write(&proc_self_cgroup, "").expect("write empty proc/self/cgroup");

    let err = run_preflight_at(cgroup_root, /* uid = */ 1000, &proc_fs, &proc_self_cgroup)
        .expect_err(
            "preflight must refuse: empty /proc/self/cgroup cannot identify the \
             enclosing slice",
        );

    match err {
        CgroupPreflightError::CgroupPathDiscoveryFailed { .. } => {}
        other => {
            panic!("expected CgroupPathDiscoveryFailed (empty /proc/self/cgroup); got {other:?}")
        }
    }
}
