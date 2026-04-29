//! Step 01-01 (bugfix `fix-cgroup-preflight-subtree-unreadable`) —
//! coverage for the new `SubtreeControlUnreadable` error variant.
//!
//! Per the RCA at
//! `docs/feature/fix-cgroup-preflight-subtree-unreadable/bugfix-rca.md`,
//! the step-4 read of `<enclosing_slice>/cgroup.subtree_control`
//! (`cgroup_preflight.rs:273`) used `unwrap_or_default()`, which
//! collapsed every `io::Error` (PermissionDenied, EIO, IsADirectory,
//! Other, …) into the empty string. The empty string then tripped the
//! token-scan to push both `cpu` and `memory` onto the missing list,
//! and the function returned `DelegationMissing` with the
//! `Delegate=yes` / `systemctl set-property` remediation —
//! regardless of whether the actual failure was a permissions error,
//! a transient cgroupfs I/O error, or the slice path simply not
//! resolving to a cgroup directory at all. None of those are fixed
//! by `Delegate=yes`.
//!
//! This test fabricates an unreadable `cgroup.subtree_control`
//! analogue by creating a DIRECTORY at the path where step 4 expects
//! a regular file. `read_to_string` on a directory returns
//! `Err(io::Error)` with a kind other than `NotFound` (Linux 6.0+:
//! `IsADirectory`; older kernels: `Other`). The directory-path fixture
//! is portable across the test-process UID surface — root and non-root
//! both see the same error — so it works under nextest's Lima root,
//! GitHub Actions non-root, and the macOS `--no-run` compile gate
//! identically.
//!
//! The assertion shape:
//!   1. The error is `SubtreeControlUnreadable { slice, source }`
//!      (NOT `DelegationMissing`).
//!   2. The wrapped `source.kind()` is NOT `NotFound` — `NotFound` is
//!      its own regression (see
//!      `preflight_subtree_control_missing_is_not_delegation.rs`),
//!      and pinning Option B requires both shapes to surface as the
//!      new variant.
//!   3. `slice` matches the enclosing-slice path discovered from
//!      `/proc/self/cgroup` (operator triage: which slice was
//!      unreadable?).
//!   4. The rendered `Display` message names the I/O cause, surfaces
//!      `--allow-no-cgroups` and the docs URL, and explicitly does NOT
//!      contain "Delegate=yes" or "delegation required" — those
//!      phrases are reserved for `DelegationMissing` and are precisely
//!      the misdiagnosis this fix corrects.
//!
//! On the buggy code (line 273 still `unwrap_or_default()`) this test
//! panics with "expected SubtreeControlUnreadable, got
//! DelegationMissing". That is the correct shape for a RED scaffold
//! commit per `.claude/rules/testing.md` § "RED scaffolds and
//! intentionally-failing commits".

#![cfg(target_os = "linux")]

use overdrive_control_plane::cgroup_preflight::{CgroupPreflightError, run_preflight_at};
use tempfile::TempDir;

#[test]
fn preflight_surfaces_subtree_control_io_error_not_delegation_missing() {
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

    // Step 4 fixture — create the enclosing-slice DIRECTORY, then put
    // a DIRECTORY (not a file) at <slice>/cgroup.subtree_control.
    // `read_to_string` on a directory returns Err(io::Error) with
    // kind() == IsADirectory (Linux 6.0+) or Other (older kernels) —
    // critically NOT NotFound. NotFound is the structurally-anomalous
    // signal pinned by the sibling test
    // `preflight_subtree_control_missing_is_not_delegation.rs`; both
    // shapes must surface as `SubtreeControlUnreadable` per Option B
    // of the RCA.
    let user_slice_dir = cgroup_root.join("user.slice").join("user-1000.slice");
    std::fs::create_dir_all(&user_slice_dir).expect("create user-1000.slice dir");
    let subtree_control_path = user_slice_dir.join("cgroup.subtree_control");
    std::fs::create_dir(&subtree_control_path)
        .expect("create directory in place of cgroup.subtree_control file");

    // /proc/self/cgroup analogue — the unified-hierarchy line points
    // the discovery logic at user.slice/user-1000.slice.
    let proc_self_cgroup = tmp.path().join("proc-self-cgroup");
    std::fs::write(&proc_self_cgroup, "0::/user.slice/user-1000.slice\n")
        .expect("write proc/self/cgroup");

    let err = run_preflight_at(cgroup_root, /* uid = */ 1000, &proc_fs, &proc_self_cgroup)
        .expect_err(
            "preflight must refuse: cgroup.subtree_control is unreadable (it is \
             a directory, not a regular file). The I/O error must surface as \
             SubtreeControlUnreadable, NOT be silently absorbed into \
             DelegationMissing with a `Delegate=yes` remediation.",
        );

    let msg = err.to_string();
    match &err {
        CgroupPreflightError::SubtreeControlUnreadable { slice, source } => {
            assert_eq!(
                slice, &user_slice_dir,
                "slice path must be the enclosing slice discovered from \
                 /proc/self/cgroup (operator triage); got slice = {slice:?}",
            );
            // The wrapped I/O error must NOT be NotFound — NotFound is
            // pinned by the sibling regression test as its own
            // structurally-anomalous shape (the kernel guarantees
            // cgroup.subtree_control exists under every cgroup-v2
            // directory; absence indicates the slice is not a cgroup
            // directory). Every other ErrorKind (IsADirectory on Linux
            // 6.0+, Other on older kernels, PermissionDenied, EIO, …)
            // reaches this branch.
            assert_ne!(
                source.kind(),
                std::io::ErrorKind::NotFound,
                "this test exercises the directory-path fixture (kind = \
                 IsADirectory / Other); NotFound is pinned by the sibling \
                 regression test. Got source = {source:?}",
            );
        }
        other => panic!(
            "expected SubtreeControlUnreadable, got {other:?}.\n\
             \n\
             This is the regression: an I/O error on cgroup.subtree_control \
             (here: it is a directory, not a regular file) was silently \
             absorbed via `unwrap_or_default()` into DelegationMissing with \
             a `Delegate=yes` remediation. The fix routes every io::Error \
             through SubtreeControlUnreadable so the operator sees the \
             actual cause and the actual fix.",
        ),
    }

    // The rendered message must surface the dev escape hatch and the
    // docs URL, matching every other variant per nw-ux-tui-patterns.
    assert!(
        msg.contains("--allow-no-cgroups"),
        "must mention --allow-no-cgroups: {msg}",
    );
    assert!(
        msg.contains("docs.overdrive.sh"),
        "must mention docs URL: {msg}",
    );

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
