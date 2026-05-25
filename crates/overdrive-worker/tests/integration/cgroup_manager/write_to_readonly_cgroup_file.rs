//! C-write-to-readonly-cgroup-file — real `EACCES` from a
//! kernel-read-only pseudo-file propagates back through `CgroupManager`
//! to the caller unmodified-in-kind.
//!
//! Tier 3, real-io. Requires Lima sudo.
//!
//! Exercises ADR-0054 § D3 (read-only pseudo-file enforcement) and
//! validates AC5 (substrate-boundary propagation through
//! `cgroup_manager`). `cgroup.events` is kernel-read-only by design
//! per cgroup v2 documentation; the kernel rejects writes with
//! `EACCES`. A buggy reconciler writing the wrong field name would
//! trip this exact path — the error is real-substrate, not contrived
//! (unlike the ENOTDIR-via-regular-file-in-dir-slot mechanism of the
//! E1 KEEP-TEMPFILE rows alongside this file).
//!
//! # `CgroupManager` wrapping vs direct trait call
//!
//! `CgroupManager` does not expose a public method whose semantics
//! naturally target `cgroup.events` (it is never written by production
//! code). The DISTILL scenario explicitly allows either an exposed
//! test-only helper OR direct invocation of the same underlying
//! `fs.write` path the public surface uses — the load-bearing
//! assertion is "the REAL `io::Error` propagates back through
//! `cgroup_manager` to the caller unmodified-in-kind". We construct a
//! `CgroupManager` to materially exercise the wiring (Arc<dyn CgroupFs>
//! flowing through the manager's `fs` field) and then invoke the same
//! underlying `fs.write` the manager itself uses — proving the
//! substrate-boundary chain (real kernel VFS → `tokio::fs::*` → trait
//! method) is intact when wired through the manager.
//!
//! Scenario reference: `docs/feature/cgroup-fs-port/distill/test-scenarios.md`
//! § C-write-to-readonly-cgroup-file.

use std::path::Path;
use std::sync::Arc;

use overdrive_core::id::AllocationId;
use overdrive_core::traits::CgroupFs;
use overdrive_host::RealCgroupFs;
use overdrive_worker::cgroup_manager::CgroupManager;
use serial_test::serial;

use super::super::exec_driver::cleanup::AllocCleanup;

#[tokio::test]
#[serial(cgroup)]
async fn write_to_readonly_cgroup_events_propagates_real_eacces() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());

    // Construct a CgroupManager around the SAME fs — exercises the
    // wiring even though the test invokes the underlying fs.write
    // directly. The clone of `fs` materialises the Arc<dyn CgroupFs>
    // flowing through the manager's field.
    let manager = CgroupManager::new(cgroup_root.to_path_buf(), fs.clone());
    manager
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    let alloc = AllocationId::new("alloc-roC-0").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());

    let scope_dir =
        manager.cgroup_root().join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));
    fs.create_dir(&scope_dir).await.expect("create alloc scope");

    // cgroup.events is kernel-read-only by design (cgroup-v2.rst).
    // Writing to it must be rejected by the kernel. The error flows
    // from the real kernel VFS through `tokio::fs::write` (inside
    // RealCgroupFs::write) to the trait method's caller — the
    // substrate-boundary propagation chain that AC5 defends.
    //
    // The specific errno varies by kernel version:
    //   - EACCES (PermissionDenied) — kernels that route the write
    //     through the permission check.
    //   - EINVAL (InvalidInput) — kernels where cgroup.events has
    //     NO `write` handler registered; the VFS rejects with
    //     EINVAL because the kernfs operation is unsupported.
    //   - EROFS — older shapes on read-only filesystems.
    // We accept any of these — the load-bearing assertion is
    // "kernel rejects the write, error propagates through trait",
    // NOT which specific errno fires.
    let err = fs
        .write(&scope_dir.join("cgroup.events"), b"populated 1\n")
        .await
        .expect_err("write to read-only cgroup.events must fail");

    let kind = err.kind();
    let raw = err.raw_os_error();
    let accepted = kind == std::io::ErrorKind::PermissionDenied
        || kind == std::io::ErrorKind::InvalidInput
        || raw == Some(libc::EACCES)
        || raw == Some(libc::EINVAL)
        || raw == Some(libc::EROFS);
    assert!(
        accepted,
        "expected kernel rejection (EACCES / EINVAL / EROFS) of write to read-only \
         cgroup.events; got kind={kind:?} raw={raw:?} — the substrate-boundary \
         propagation chain is broken if NO error surfaces",
    );

    // The error MUST be a real `io::Error` from the kernel — NOT
    // `Ok(())` (which would mean the write silently succeeded against
    // a read-only file, indicating either a kernel bug OR a wiring
    // bug where the trait method bypassed the actual syscall).
    assert!(raw.is_some(), "expected real kernel errno on read-only write; got {err:?}");
}
