//! Step 03-01 / Slice 4 scenario 4.6 —
//! `preflight_names_missing_cpu_controller`.
//!
//! Per ADR-0028 §4 step 4 grammar branch: when ONLY `cpu` is missing
//! from `subtree_control` (the user slice has memory delegated but
//! not cpu), the rendered error must specifically name `cpu` — not
//! "cpu and memory" — so the operator goes fix the right delegation
//! gap. Companion test to `preflight_no_delegation.rs`, which covers
//! the both-missing case.

#![cfg(target_os = "linux")]

use overdrive_control_plane::cgroup_preflight::{CgroupPreflightError, run_preflight_at};
use tempfile::TempDir;

#[test]
fn preflight_names_missing_cpu_controller_specifically() {
    let tmp = TempDir::new().expect("tempdir");
    let cgroup_root = tmp.path();
    let proc_fs = tmp.path().join("filesystems");
    std::fs::write(&proc_fs, "nodev\tcgroup2\n").expect("write proc/filesystems");
    std::fs::write(cgroup_root.join("cgroup.controllers"), "cpu memory io\n")
        .expect("write cgroup.controllers");
    // Only `memory` and `io` are delegated; `cpu` is missing.
    std::fs::write(cgroup_root.join("cgroup.subtree_control"), "memory io\n")
        .expect("write subtree_control");

    let err = run_preflight_at(cgroup_root, /* uid = */ 1000, &proc_fs)
        .expect_err("missing-cpu must fail");

    match &err {
        CgroupPreflightError::DelegationMissing { missing, .. } => {
            assert_eq!(missing, &vec!["cpu".to_owned()], "missing set must be exactly [cpu]");
        }
        other => panic!("expected DelegationMissing, got {other:?}"),
    }

    let msg = err.to_string();
    // Singular grammar — the operator-facing message says "cpu
    // controller", not "cpu and memory controllers".
    assert!(msg.contains("cpu controller"), "must say `cpu controller`: {msg}");
    assert!(
        !msg.contains("cpu and memory"),
        "must NOT claim memory missing when only cpu is missing: {msg}"
    );
}

#[test]
fn preflight_names_missing_memory_controller_specifically() {
    let tmp = TempDir::new().expect("tempdir");
    let cgroup_root = tmp.path();
    let proc_fs = tmp.path().join("filesystems");
    std::fs::write(&proc_fs, "nodev\tcgroup2\n").expect("write proc/filesystems");
    std::fs::write(cgroup_root.join("cgroup.controllers"), "cpu memory io\n")
        .expect("write cgroup.controllers");
    // Only `cpu` and `io` are delegated; `memory` is missing.
    std::fs::write(cgroup_root.join("cgroup.subtree_control"), "cpu io\n")
        .expect("write subtree_control");

    let err = run_preflight_at(cgroup_root, /* uid = */ 1000, &proc_fs)
        .expect_err("missing-memory must fail");

    match &err {
        CgroupPreflightError::DelegationMissing { missing, .. } => {
            assert_eq!(missing, &vec!["memory".to_owned()], "missing set must be exactly [memory]");
        }
        other => panic!("expected DelegationMissing, got {other:?}"),
    }

    let msg = err.to_string();
    assert!(msg.contains("memory controller"), "must say `memory controller`: {msg}");
}
