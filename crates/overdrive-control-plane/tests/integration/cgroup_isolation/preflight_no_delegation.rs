//! Step 03-01 / Slice 4 scenario 4.4 —
//! `preflight_refuses_without_delegation`.
//!
//! Per ADR-0028 §4 step 4: when running as a non-root UID and the
//! parent slice's `subtree_control` lacks one or both of `cpu` /
//! `memory`, the pre-flight refuses with `DelegationMissing`. The
//! rendered message must name `Delegate=yes`, the `--allow-no-cgroups`
//! escape hatch, and the docs URL — operators without cgroup
//! delegation see actionable next steps, not a silent panic.

#![cfg(target_os = "linux")]

use overdrive_control_plane::cgroup_preflight::{CgroupPreflightError, run_preflight_at};
use tempfile::TempDir;

#[test]
fn preflight_refuses_without_delegation() {
    let tmp = TempDir::new().expect("tempdir");
    let cgroup_root = tmp.path();
    // Step 1 must pass: write a /proc/filesystems analogue that
    // includes cgroup2.
    let proc_fs = tmp.path().join("filesystems");
    std::fs::write(&proc_fs, "nodev\tcgroup2\nnodev\ttmpfs\n").expect("write proc/filesystems");

    // Step 2 must pass: cgroup.controllers exists (cgroup v2 mounted).
    std::fs::write(cgroup_root.join("cgroup.controllers"), "cpu memory io\n")
        .expect("write cgroup.controllers");

    // Step 4 must FAIL: subtree_control is missing both controllers
    // (operator has not been delegated cpu/memory).
    std::fs::write(cgroup_root.join("cgroup.subtree_control"), "io pids\n")
        .expect("write subtree_control");

    let err = run_preflight_at(cgroup_root, /* uid = */ 1000, &proc_fs)
        .expect_err("delegation missing must fail");

    let msg = err.to_string();
    match err {
        CgroupPreflightError::DelegationMissing { uid, missing, .. } => {
            assert_eq!(uid, 1000, "uid must be propagated to the rendered error");
            assert!(missing.iter().any(|m| m == "cpu"), "must name cpu as missing");
            assert!(missing.iter().any(|m| m == "memory"), "must name memory as missing");
        }
        other => panic!("expected DelegationMissing, got {other:?}"),
    }

    assert!(msg.contains("Delegate=yes"), "must mention `Delegate=yes`: {msg}");
    assert!(msg.contains("--allow-no-cgroups"), "must mention --allow-no-cgroups: {msg}");
    assert!(msg.contains("docs.overdrive.sh"), "must mention docs URL: {msg}");
}
