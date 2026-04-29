//! Step 03-01 / Slice 4 scenario 4.5 —
//! `preflight_refuses_on_cgroup_v1_host`.
//!
//! Per ADR-0028 §4 step 1: a kernel without cgroup v2 in
//! `/proc/filesystems` (cgroup v1-only host, or a stripped kernel)
//! must be rejected with `NoCgroupV2`. The rendered message must
//! mention the `--allow-no-cgroups` escape hatch and the docs URL —
//! Phase 1 of Overdrive does not support cgroup v1.

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
    assert!(msg.contains("--allow-no-cgroups"), "must mention --allow-no-cgroups: {msg}");
    assert!(msg.contains("docs.overdrive.sh"), "must mention docs URL: {msg}");
}
