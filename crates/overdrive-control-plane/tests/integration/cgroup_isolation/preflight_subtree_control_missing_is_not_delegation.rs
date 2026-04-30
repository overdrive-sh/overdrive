//! Step 01-01 (bugfix `fix-cgroup-preflight-subtree-unreadable`) —
//! the test that pins **Option B** of the RCA at
//! `docs/feature/fix-cgroup-preflight-subtree-unreadable/bugfix-rca.md`.
//!
//! When `cgroup.subtree_control` does NOT exist under the enclosing
//! slice directory (the `read_to_string` returns
//! `Err(io::ErrorKind::NotFound)`), the error must surface as
//! `SubtreeControlUnreadable`, **not** as `DelegationMissing`.
//!
//! Why this matters — the asymmetry with the step-1 fix
//! (`fix-cgroup-preflight-procfs-unreadable`) is load-bearing:
//!
//!   - `/proc/filesystems` CAN legitimately be absent on a stripped
//!     kernel without procfs entries for cgroup support; that case
//!     IS the same application-semantic state as "cgroup2 not
//!     listed", so step-1 maps `NotFound → NoCgroupV2` via empty-
//!     string fallthrough. The new `development.md` rule explicitly
//!     authorises that absorption.
//!
//!   - `cgroup.subtree_control` CANNOT legitimately be absent under a
//!     real cgroup-v2 directory — the kernel creates it for every
//!     cgroup directory per
//!     `Documentation/admin-guide/cgroup-v2.rst`. Its absence
//!     therefore indicates the enclosing-slice path is not a cgroup
//!     directory at all (race against unmount, misconfigured
//!     `cgroup_root`, parsed `/proc/self/cgroup` line points at a
//!     non-cgroup path, …) — none of which are "delegation missing"
//!     and none of which are fixed by `Delegate=yes`. The new
//!     `development.md` rule's escape clause does NOT authorise
//!     absorbing `NotFound` here.
//!
//! The fixture creates the enclosing-slice DIRECTORY but does NOT
//! create `cgroup.subtree_control` inside it. The buggy step-4
//! body's `unwrap_or_default()` absorbs `NotFound` into the empty
//! string; the token-scan finds neither `cpu` nor `memory`; and the
//! function returns `DelegationMissing` — the precise misdiagnosis
//! this regression test exists to prevent.
//!
//! On the buggy code (line 273 still `unwrap_or_default()`) this test
//! panics with "expected `SubtreeControlUnreadable`, got
//! `DelegationMissing`". That is the correct shape for a RED scaffold
//! commit per `.claude/rules/testing.md` § "RED scaffolds and
//! intentionally-failing commits".

#![cfg(target_os = "linux")]

use overdrive_control_plane::cgroup_preflight::{CgroupPreflightError, run_preflight_at};
use tempfile::TempDir;

#[test]
fn preflight_treats_missing_subtree_control_as_io_error() {
    let tmp = TempDir::new().expect("tempdir");
    let cgroup_root = tmp.path();

    // Step 1 must pass: write a /proc/filesystems analogue that
    // includes cgroup2.
    let proc_fs = tmp.path().join("filesystems");
    std::fs::write(&proc_fs, "nodev\tcgroup2\nnodev\ttmpfs\n").expect("write proc/filesystems");

    // Step 2 must pass: cgroup.controllers exists at the cgroupfs
    // mount root.
    std::fs::write(cgroup_root.join("cgroup.controllers"), "cpu memory io\n")
        .expect("write cgroup.controllers");

    // Step 4 fixture — create the enclosing-slice DIRECTORY, but do
    // NOT create cgroup.subtree_control inside it. `read_to_string`
    // on a non-existent file returns Err(io::Error) with
    // kind() == NotFound — the precise shape this test pins.
    //
    // Per Option B of the RCA: NotFound on cgroup.subtree_control is
    // structurally distinct from "no controllers delegated"; the
    // kernel guarantees the file exists under every cgroup directory.
    // The fix surfaces NotFound as SubtreeControlUnreadable, NOT as
    // DelegationMissing fallthrough.
    let user_slice_dir = cgroup_root.join("user.slice").join("user-1000.slice");
    std::fs::create_dir_all(&user_slice_dir).expect("create user-1000.slice dir");
    // Deliberately NOT creating user_slice_dir.join("cgroup.subtree_control").

    // /proc/self/cgroup analogue — the unified-hierarchy line points
    // the discovery logic at user.slice/user-1000.slice.
    let proc_self_cgroup = tmp.path().join("proc-self-cgroup");
    std::fs::write(&proc_self_cgroup, "0::/user.slice/user-1000.slice\n")
        .expect("write proc/self/cgroup");

    let err = run_preflight_at(cgroup_root, /* uid = */ 1000, &proc_fs, &proc_self_cgroup)
        .expect_err(
            "preflight must refuse: cgroup.subtree_control does not exist \
             under the enclosing slice. NotFound on this file indicates the \
             slice path is not a cgroup directory — structurally distinct \
             from `no controllers delegated`. The error must surface as \
             SubtreeControlUnreadable, NOT as DelegationMissing.",
        );

    let msg = err.to_string();
    match &err {
        CgroupPreflightError::SubtreeControlUnreadable { slice, source } => {
            assert_eq!(
                slice, &user_slice_dir,
                "slice path must be the enclosing slice discovered from \
                 /proc/self/cgroup (operator triage); got slice = {slice:?}",
            );
            // This test specifically pins NotFound — that is the whole
            // point of Option B. The buggy code absorbs NotFound into
            // the empty string and returns DelegationMissing; the fix
            // surfaces NotFound as the new variant.
            assert_eq!(
                source.kind(),
                std::io::ErrorKind::NotFound,
                "this test exercises the missing-file fixture (kind = \
                 NotFound) — Option B of the RCA. Got source = {source:?}",
            );
        }
        other => panic!(
            "expected SubtreeControlUnreadable, got {other:?}.\n\
             \n\
             This is the regression that pins Option B: a NotFound on \
             <enclosing_slice>/cgroup.subtree_control was silently \
             absorbed via `unwrap_or_default()` into DelegationMissing \
             with a `Delegate=yes` remediation — but the kernel \
             guarantees cgroup.subtree_control exists under every \
             cgroup-v2 directory, so its absence is structurally \
             anomalous, not `no controllers delegated`. The fix routes \
             NotFound through SubtreeControlUnreadable so the operator \
             sees the actual cause (the slice path is not a cgroup \
             directory) and the actual fix (verify cgroupfs \
             configuration), not a misdiagnosis.",
        ),
    }

    // The rendered message must surface the dev escape hatch and the
    // docs URL, matching every other variant per nw-ux-tui-patterns.
    assert!(msg.contains("--allow-no-cgroups"), "must mention --allow-no-cgroups: {msg}");
    assert!(msg.contains("docs.overdrive.sh"), "must mention docs URL: {msg}");

    // Critically, the message must NOT prescribe `Delegate=yes` or
    // "delegation required" — those phrases are reserved for
    // `DelegationMissing` and are precisely the misdiagnosis this fix
    // corrects.
    assert!(
        !msg.contains("Delegate=yes"),
        "SubtreeControlUnreadable must NOT prescribe `Delegate=yes` — that \
         phrase is reserved for DelegationMissing and is the misdiagnosis \
         this fix corrects. Got: {msg}",
    );
    assert!(
        !msg.contains("delegation required"),
        "SubtreeControlUnreadable must NOT claim `delegation required` — \
         that phrase is reserved for DelegationMissing and is the \
         misdiagnosis this fix corrects. Got: {msg}",
    );
}
