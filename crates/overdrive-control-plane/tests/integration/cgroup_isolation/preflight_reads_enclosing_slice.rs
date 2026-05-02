//! Step 01-01 (bugfix `fix-cgroup-preflight-wrong-slice`) — oracle for
//! ADR-0028 §4 step 4.
//!
//! The pre-existing step-4 implementation reads
//! `<cgroup_root>/cgroup.subtree_control` directly. With the
//! production `cgroup_root = /sys/fs/cgroup` that resolves to the
//! kernel-root cgroup, whose `subtree_control` lists every controller
//! on every modern kernel — so the delegation check passes
//! unconditionally for every non-root user. ADR-0028 §4 step 4
//! requires reading `/proc/self/cgroup` to discover the *enclosing*
//! slice and inspecting THAT slice's `subtree_control`.
//!
//! This oracle distinguishes the two: it seeds the kernel-root
//! `subtree_control` with all controllers (so the buggy code,
//! reading the wrong file, would pass) and the enclosing slice's
//! `subtree_control` with neither `cpu` nor `memory` (so the fixed
//! code, reading the right file, must refuse). The buggy code reads
//! the kernel-root file and incorrectly returns `Ok(())`; the fixed
//! code reads the enclosing slice and returns `DelegationMissing`.
//!
//! This is the missing oracle from Root Cause B in
//! `docs/feature/fix-cgroup-preflight-wrong-slice/bugfix-rca.md`.

#![cfg(target_os = "linux")]

use overdrive_control_plane::cgroup_preflight::{CgroupPreflightError, run_preflight_at};
use tempfile::TempDir;

#[test]
fn preflight_reads_enclosing_slice_via_proc_self_cgroup() {
    let tmp = TempDir::new().expect("tempdir");
    let cgroup_root = tmp.path();

    // Step 1 must pass: /proc/filesystems lists cgroup2.
    let proc_fs = tmp.path().join("filesystems");
    std::fs::write(&proc_fs, "nodev\tcgroup2\n").expect("write proc/filesystems");

    // Step 2 must pass: cgroup.controllers exists at the cgroupfs
    // mount root (cgroup v2 mounted).
    std::fs::write(cgroup_root.join("cgroup.controllers"), "cpu memory io\n")
        .expect("write cgroup.controllers");

    // Kernel-root state — every controller is delegated to the root
    // cgroup. The buggy step 4 reads this file and incorrectly
    // concludes delegation is fine.
    std::fs::write(cgroup_root.join("cgroup.subtree_control"), "cpu memory io\n")
        .expect("write kernel-root subtree_control");

    // Enclosing slice — what ADR-0028 §4 step 4 actually requires
    // inspecting. This user slice has neither `cpu` nor `memory`
    // delegated (Delegate=yes was never granted on user-1000.slice),
    // so a correct preflight refuses.
    let user_slice_dir = cgroup_root.join("user.slice").join("user-1000.slice");
    std::fs::create_dir_all(&user_slice_dir).expect("create user-1000.slice dir");
    std::fs::write(user_slice_dir.join("cgroup.subtree_control"), "io pids\n")
        .expect("write user-slice subtree_control");

    // /proc/self/cgroup analogue — the unified-hierarchy line points
    // the discovery logic at user.slice/user-1000.slice.
    let proc_self_cgroup = tmp.path().join("proc-self-cgroup");
    std::fs::write(&proc_self_cgroup, "0::/user.slice/user-1000.slice\n")
        .expect("write proc/self/cgroup");

    let err = run_preflight_at(cgroup_root, /* uid = */ 1000, &proc_fs, &proc_self_cgroup)
        .expect_err(
            "preflight must refuse: enclosing slice lacks cpu and memory, \
             even though kernel-root subtree_control lists every controller",
        );

    let msg = err.to_string();
    match err {
        CgroupPreflightError::DelegationMissing { uid, missing, .. } => {
            assert_eq!(uid, 1000, "uid must be propagated to the rendered error");
            assert!(
                missing.iter().any(|m| m == "cpu"),
                "must name cpu as missing in enclosing slice: missing = {missing:?}",
            );
            assert!(
                missing.iter().any(|m| m == "memory"),
                "must name memory as missing in enclosing slice: missing = {missing:?}",
            );
        }
        other => panic!(
            "expected DelegationMissing (the enclosing slice has neither \
             cpu nor memory); got {other:?} — the buggy implementation \
             reads <cgroup_root>/cgroup.subtree_control (kernel-root, \
             which lists every controller) instead of the slice named \
             by /proc/self/cgroup",
        ),
    }

    assert!(
        msg.contains("cargo xtask lima run"),
        "must mention canonical Lima dev path (ADR-0034): {msg}"
    );
    assert!(msg.contains("docs.overdrive.sh"), "must mention docs URL: {msg}");
}
