//! Step 01-01 (bugfix `fix-cgroup-preflight-procfs-unreadable`) —
//! coverage for the new `ProcFilesystemsUnreadable` error variant.
//!
//! Per the RCA at
//! `docs/feature/fix-cgroup-preflight-procfs-unreadable/bugfix-rca.md`,
//! the previous step-1 read of `/proc/filesystems`
//! (`cgroup_preflight.rs:189`) used `unwrap_or_default()`, which
//! collapsed every `io::Error` (PermissionDenied, EIO, broken procfs,
//! /proc not mounted, …) into the empty string. The empty string then
//! tripped `cgroup_v2_available = false` and the function returned
//! `NoCgroupV2` — naming the wrong cause and prescribing "boot a
//! newer kernel" as the remediation.
//!
//! This test fabricates an unreadable `/proc/filesystems` analogue by
//! pointing the `proc_filesystems` parameter at a directory inside a
//! `tempfile::TempDir`. `read_to_string` on a directory returns
//! `Err(io::Error)` with a kind other than `NotFound` (Linux 6.0+:
//! `IsADirectory`; older kernels: `Other`). The directory-path fixture
//! is portable across the test-process UID surface — root and non-root
//! both see the same error — so it works under nextest's Lima root,
//! GitHub Actions non-root, and the macOS `--no-run` compile gate
//! identically.
//!
//! The assertion shape:
//!   1. The error is `ProcFilesystemsUnreadable { source }` (NOT
//!      `NoCgroupV2`).
//!   2. The wrapped `source.kind()` is NOT `NotFound` — `NotFound` is
//!      reserved for the legitimate v1-host fallthrough; every other
//!      kind exits early via the new variant.
//!   3. The rendered `Display` message names the I/O cause, surfaces
//!      `--allow-no-cgroups`, and explicitly does NOT contain "boot a
//!      newer kernel" or "cgroup v2 not available on this kernel" —
//!      those phrases are reserved for the `NoCgroupV2` variant and
//!      are precisely the misdiagnosis this fix corrects.
//!
//! On the buggy code (line 189 still `unwrap_or_default()`) this test
//! panics with "expected ProcFilesystemsUnreadable, got NoCgroupV2".
//! That is the correct shape for a RED scaffold commit per
//! `.claude/rules/testing.md` § "RED scaffolds and intentionally-
//! failing commits".

#![cfg(target_os = "linux")]

use overdrive_control_plane::cgroup_preflight::{CgroupPreflightError, run_preflight_at};
use tempfile::TempDir;

#[test]
fn preflight_surfaces_procfs_io_error_not_no_cgroup_v2() {
    let tmp = TempDir::new().expect("tempdir");
    let cgroup_root = tmp.path();

    // Step 2 must NOT short-circuit before step 1's I/O error —
    // step 1 runs first regardless. Seed cgroup.controllers anyway so
    // that, if step 1 ever stops failing, we don't accidentally trip
    // NotMounted instead and obscure the regression signal.
    std::fs::write(cgroup_root.join("cgroup.controllers"), "cpu memory io\n")
        .expect("write cgroup.controllers");

    // /proc/filesystems analogue points at a DIRECTORY, not a file.
    // `read_to_string` on a directory returns Err(io::Error) with
    // kind() == IsADirectory (Linux 6.0+) or Other (older kernels) —
    // critically NOT NotFound. NotFound is the v1-host signal and
    // must continue to flow through NoCgroupV2; every other kind
    // surfaces as ProcFilesystemsUnreadable.
    let proc_fs_dir = tmp.path().join("filesystems-as-dir");
    std::fs::create_dir(&proc_fs_dir).expect("create directory at proc_filesystems path");

    // Step 4 won't be reached, but the parameter is required by the
    // signature.
    let proc_self_cgroup = tmp.path().join("proc-self-cgroup");

    let err = run_preflight_at(cgroup_root, /* uid = */ 1000, &proc_fs_dir, &proc_self_cgroup)
        .expect_err(
            "preflight must refuse: /proc/filesystems is unreadable (it is a \
         directory, not a regular file). The I/O error must surface as \
         ProcFilesystemsUnreadable, NOT be silently absorbed into NoCgroupV2.",
        );

    let msg = err.to_string();
    match &err {
        CgroupPreflightError::ProcFilesystemsUnreadable { source } => {
            // The wrapped I/O error must NOT be NotFound — NotFound is
            // the v1-host signal and is mapped to NoCgroupV2 by the
            // (forthcoming) line-189 match. Every other ErrorKind
            // (IsADirectory on Linux 6.0+, Other on older kernels,
            // PermissionDenied on `chmod 0o000`, EIO on a broken
            // procfs, …) reaches this branch.
            assert_ne!(
                source.kind(),
                std::io::ErrorKind::NotFound,
                "NotFound must flow to NoCgroupV2 (the v1-host signal), \
                 not ProcFilesystemsUnreadable; got source = {source:?}",
            );
        }
        other => panic!(
            "expected ProcFilesystemsUnreadable, got {other:?}.\n\
             \n\
             This is the regression: an I/O error on /proc/filesystems \
             (here: it is a directory, not a regular file) was silently \
             absorbed into NoCgroupV2 with a `boot a newer kernel` \
             remediation. The fix routes every io::ErrorKind other than \
             NotFound through ProcFilesystemsUnreadable so the operator \
             sees the actual cause and the actual fix.",
        ),
    }

    // The rendered message must surface the dev escape hatch and the
    // docs URL, matching every other variant per nw-ux-tui-patterns.
    assert!(msg.contains("--allow-no-cgroups"), "must mention --allow-no-cgroups: {msg}",);
    assert!(msg.contains("docs.overdrive.sh"), "must mention docs URL: {msg}",);

    // Critically, the message must NOT prescribe "boot a newer kernel"
    // — that is the specific misdiagnosis the fix corrects. Those
    // phrases are reserved for `NoCgroupV2`, the variant that fires
    // when /proc/filesystems is genuinely readable but does not list
    // `cgroup2`.
    assert!(
        !msg.contains("Boot a kernel") && !msg.contains("boot a newer kernel"),
        "ProcFilesystemsUnreadable must NOT prescribe `boot a newer kernel` \
         — that is the misdiagnosis this fix corrects. Got: {msg}",
    );
    assert!(
        !msg.contains("cgroup v2 not available on this kernel"),
        "ProcFilesystemsUnreadable must NOT claim `cgroup v2 not available \
         on this kernel` — that phrase is reserved for NoCgroupV2 and is \
         precisely the misdiagnosis this fix corrects. Got: {msg}",
    );
}
