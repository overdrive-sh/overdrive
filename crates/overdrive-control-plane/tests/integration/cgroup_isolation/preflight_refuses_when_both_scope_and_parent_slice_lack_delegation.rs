//! Step 01-02 (bugfix `fix-cgroup-preflight-scope-vs-slice`) — locking
//! test that the parent-slice fallback (ADR-0028 §4 step 4) reports
//! the slice it actually inspected when delegation is missing.
//!
//! Sibling oracle to
//! `preflight_falls_back_to_parent_slice_on_empty_scope.rs`. That
//! oracle pins the happy path: scope `subtree_control` is empty, parent
//! slice carries `cpu memory` — preflight returns `Ok(())`. This test
//! pins the unhappy path: scope `subtree_control` is empty AND the
//! parent slice's `subtree_control` lacks `cpu` and `memory` — preflight
//! must return `DelegationMissing` against the PARENT slice path (the
//! one the operator must `Delegate=yes` on), not against the scope
//! path.
//!
//! Why the slice-path assertion matters: `DelegationMissing.slice` is
//! the operator-facing path that drives the rendered remediation
//! ("`sudo systemctl set-property <slice> Delegate=yes`"). If the fix
//! reported the scope path here, the operator would set delegation on
//! a leaf scope — which has no effect, since cgroup v2's
//! no-internal-processes rule forces controllers to be enabled in the
//! parent's `subtree_control`. The test pins that the inspected path
//! flows through to the error variant.
//!
//! Fixture shape:
//!   <tmp>/cgroup.controllers                                   = "cpu memory io"
//!   <tmp>/user.slice/user-1000.slice/cgroup.subtree_control    = "io pids"  ← no cpu, no memory
//!   <tmp>/user.slice/.../session-3.scope/cgroup.subtree_control = ""        (leaf, falls back)
//!   <tmp>/proc-self-cgroup                                     = "0::/user.slice/user-1000.slice/session-3.scope\n"
//!
//! Under the buggy code (no fallback) this test panics in two ways:
//!   - `slice` field would be the SCOPE path, not the parent slice
//!     path — the buggy implementation reports the discovered path.
//!   - The bug also makes the happy-path sibling test fail; this test
//!     additionally locks in that the failure path uses the parent
//!     slice's path so the operator sees actionable remediation.
//!
//! Under the GREEN fix (parent-slice fallback when the scope file is
//! empty, threaded through to error variants), the assertion holds:
//! `slice == <tmp>/user.slice/user-1000.slice` and `missing` contains
//! both `cpu` and `memory`.

#![cfg(target_os = "linux")]

use overdrive_control_plane::cgroup_preflight::{CgroupPreflightError, run_preflight_at};
use tempfile::TempDir;

#[test]
fn preflight_refuses_when_both_scope_and_parent_slice_lack_delegation() {
    let tmp = TempDir::new().expect("tempdir");
    let cgroup_root = tmp.path();

    // Step 1 must pass: /proc/filesystems lists cgroup2.
    let proc_fs = tmp.path().join("filesystems");
    std::fs::write(&proc_fs, "nodev\tcgroup2\n").expect("write proc/filesystems");

    // Step 2 must pass: cgroup.controllers exists at the cgroupfs
    // mount root (cgroup v2 mounted).
    std::fs::write(cgroup_root.join("cgroup.controllers"), "cpu memory io\n")
        .expect("write cgroup.controllers");

    // Parent slice — user-1000.slice. Its subtree_control lacks BOTH
    // `cpu` and `memory` (the operator never received `Delegate=yes`
    // for those controllers). This is the path the operator must act
    // on; the rendered error message must name THIS path.
    let user_slice_dir = cgroup_root.join("user.slice").join("user-1000.slice");
    std::fs::create_dir_all(&user_slice_dir).expect("create user-1000.slice dir");
    std::fs::write(user_slice_dir.join("cgroup.subtree_control"), "io pids\n")
        .expect("write user-slice subtree_control");

    // Leaf scope — session-3.scope. Empty subtree_control (the
    // cgroup-v2 "no internal processes" rule forces leaves to be
    // controller-empty). The fix's empty-detection branch fires here
    // and falls back to the parent slice — but the parent ALSO lacks
    // delegation, so DelegationMissing fires against the parent path.
    let session_scope_dir = user_slice_dir.join("session-3.scope");
    std::fs::create_dir_all(&session_scope_dir).expect("create session-3.scope dir");
    std::fs::write(session_scope_dir.join("cgroup.subtree_control"), "")
        .expect("write empty session-scope subtree_control");

    // /proc/self/cgroup analogue — discovery points at the leaf scope
    // (matching what `overdrive serve` invoked from an interactive TTY
    // actually sees).
    let proc_self_cgroup = tmp.path().join("proc-self-cgroup");
    std::fs::write(&proc_self_cgroup, "0::/user.slice/user-1000.slice/session-3.scope\n")
        .expect("write proc/self/cgroup");

    let err = run_preflight_at(cgroup_root, /* uid = */ 1000, &proc_fs, &proc_self_cgroup)
        .expect_err(
            "preflight must refuse: parent slice lacks cpu and memory, \
             even though the empty scope subtree_control falls back to \
             inspecting the parent",
        );

    match &err {
        CgroupPreflightError::DelegationMissing { uid, slice, missing, .. } => {
            assert_eq!(*uid, 1000, "uid must be propagated to the rendered error");
            assert_eq!(
                slice, &user_slice_dir,
                "slice must be the PARENT slice path (the one whose \
                 subtree_control was actually inspected after the empty- \
                 scope fallback fired) — the operator must `Delegate=yes` \
                 on the parent slice, not the leaf scope. Got slice = {slice:?}",
            );
            assert!(
                missing.iter().any(|m| m == "cpu"),
                "must name cpu as missing in parent slice: missing = {missing:?}",
            );
            assert!(
                missing.iter().any(|m| m == "memory"),
                "must name memory as missing in parent slice: missing = {missing:?}",
            );
        }
        other => panic!(
            "expected DelegationMissing against parent slice (the \
             empty-scope fallback fired and the parent also lacks \
             delegation); got {other:?}",
        ),
    }
}
