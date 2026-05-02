//! Step 03-01 / Slice 4 scenario 4.5 —
//! `preflight_refuses_on_cgroup_v1_host`.
//!
//! Per ADR-0028 §4 step 1: a kernel without cgroup v2 in
//! `/proc/filesystems` (cgroup v1-only host, or a stripped kernel)
//! must be rejected with `NoCgroupV2`. The rendered message must
//! mention the canonical `cargo xtask lima run --` Lima dev path
//! (per ADR-0034) and the docs URL — Phase 1 of Overdrive does not
//! support cgroup v1.

#![cfg(target_os = "linux")]

use overdrive_control_plane::cgroup_preflight::{CgroupPreflightError, run_preflight_at};
use tempfile::TempDir;

#[test]
fn preflight_refuses_on_cgroup_v1_host() {
    let tmp = TempDir::new().expect("tempdir");
    // /proc/filesystems analogue WITHOUT cgroup2 — simulate a host
    // running cgroup v1 only (the line names `cgroup`, not `cgroup2`).
    let proc_fs = tmp.path().join("filesystems");
    std::fs::write(&proc_fs, "nodev\tcgroup\nnodev\ttmpfs\n").expect("write proc/filesystems");

    // Step 1 fails before step 4 ever runs, so the contents and
    // existence of `proc_self_cgroup` are irrelevant here — pass an
    // unused tempdir-relative path purely to satisfy the signature.
    let proc_self_cgroup = tmp.path().join("proc-self-cgroup");
    let err = run_preflight_at(tmp.path(), /* uid = */ 1000, &proc_fs, &proc_self_cgroup)
        .expect_err("cgroup-v1-only host must fail step 1");

    match &err {
        CgroupPreflightError::NoCgroupV2 { kernel } => {
            // Kernel field is populated; in a real run on this host
            // it reads /proc/sys/kernel/osrelease (populated by the
            // Linux kernel) and falls back to "unknown" only when
            // that file is also absent. We assert it is non-empty
            // AND has a kernel-version shape (contains a `.`) so a
            // body→`String::new()` or body→`"xyzzy".into()` mutation
            // on `uname_release` would fail one of these checks.
            assert!(!kernel.is_empty(), "kernel field must be non-empty");
            assert!(
                kernel == "unknown" || kernel.contains('.'),
                "kernel field must be either the fallback `unknown` or a real kernel version (`x.y.z`); \
                 got {kernel:?}",
            );
            // Belt-and-braces: reject the explicit mutant marker.
            assert_ne!(kernel, "xyzzy", "kernel must not be the mutant marker `xyzzy`");
        }
        other => panic!("expected NoCgroupV2, got {other:?}"),
    }

    let msg = err.to_string();
    assert!(msg.contains("cgroup v2"), "must mention cgroup v2: {msg}");
    assert!(
        msg.contains("cargo xtask lima run"),
        "must mention canonical Lima dev path (ADR-0034): {msg}"
    );
    assert!(msg.contains("docs.overdrive.sh"), "must mention docs URL: {msg}");
}

/// Belt-and-braces — `NotFound` on `/proc/filesystems` is the v1-host
/// signal on a stripped kernel without procfs entries, and must
/// continue to flow through `NoCgroupV2` (NOT the new
/// `ProcFilesystemsUnreadable` variant added by the
/// `fix-cgroup-preflight-procfs-unreadable` bugfix).
///
/// This test locks in the `NotFound → NoCgroupV2` mapping so a future
/// refactor cannot silently route `NotFound` through the new variant.
/// Without this test, the existing `preflight_refuses_on_cgroup_v1_host`
/// fixture above (which writes a real file with cgroup v1 content)
/// would not exercise the `Err(io::Error)` branch — its `read_to_string`
/// returns `Ok(non_empty_string)`. Pointing `proc_filesystems` at a
/// path that does not exist forces `read_to_string` to return
/// `Err(NotFound)` and proves the match arm `Err(err) if err.kind() ==
/// ErrorKind::NotFound => String::new()` is wired correctly.
#[test]
fn preflight_treats_missing_proc_filesystems_as_v1_host() {
    let tmp = TempDir::new().expect("tempdir");
    // Path inside tmpdir that we deliberately do NOT create. Reading
    // it returns Err(io::Error { kind: NotFound, .. }), which the
    // forthcoming match arm maps to String::new() so step 1 falls
    // through to NoCgroupV2 — same behaviour the prior
    // `unwrap_or_default()` had for this specific kind, but now via
    // an explicit, named branch.
    let proc_fs_missing = tmp.path().join("does-not-exist");
    let proc_self_cgroup = tmp.path().join("proc-self-cgroup");

    let err =
        run_preflight_at(tmp.path(), /* uid = */ 1000, &proc_fs_missing, &proc_self_cgroup)
            .expect_err("missing /proc/filesystems must still fail step 1");

    match &err {
        CgroupPreflightError::NoCgroupV2 { .. } => {
            // Correct: NotFound is the v1-host signal.
        }
        CgroupPreflightError::ProcFilesystemsUnreadable { source } => panic!(
            "NotFound must NOT route through ProcFilesystemsUnreadable — \
             that variant is reserved for I/O errors with kinds OTHER \
             than NotFound (PermissionDenied, EIO, IsADirectory, broken \
             procfs). NotFound is the v1-host signal and must flow to \
             NoCgroupV2 with the kernel-upgrade remediation. Got \
             source = {source:?}",
        ),
        other => panic!("expected NoCgroupV2, got {other:?}"),
    }
}
