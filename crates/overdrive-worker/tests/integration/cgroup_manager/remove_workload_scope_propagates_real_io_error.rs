//! E1 row 10 — `CgroupManager::remove_workload_scope` propagates a
//! REAL `io::Error` from the underlying `tokio::fs::*` syscall when
//! the path under removal is a regular file, not a directory
//! (`ENOTDIR` / `NotADirectory`).
//!
//! KEEP-TEMPFILE per the E1 triage matrix at
//! `docs/feature/cgroup-fs-port/distill/test-scenarios.md`. The
//! regular-file-where-dir-is-expected mechanism is a contrivance to
//! *trigger* the error; the test *boundary* (real-substrate
//! `io::Error` → propagation through `CgroupManager` → caller) is
//! real and load-bearing. Distinct from the SimCgroupFs-backed
//! `remove_workload_scope_is_idempotent_on_missing_scope` acceptance
//! test which covers the LOGIC of NotFound-swallow; this one covers
//! the REAL-SUBSTRATE propagation chain.
//!
//! Candidate for retirement once Class C scenario
//! `write_to_readonly_cgroup_file` lands and proves equivalent
//! substrate-boundary coverage in production-realistic shape.
//!
//! Gated behind `--features integration-tests`; the
//! `tests/integration.rs` entrypoint carries the feature cfg.

use std::str::FromStr;
use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_host::RealCgroupFs;
use overdrive_worker::cgroup_manager::{CgroupManager, CgroupPath};

#[tokio::test]
async fn remove_workload_scope_propagates_real_io_error_when_path_is_a_regular_file() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let cgroup_root = tmp.path().to_path_buf();

    // Place a regular file where the workload scope directory should
    // be. `rmdir`-equivalent on a regular file fails with ENOTDIR
    // (or, on some kernels, with a different errno reflected in
    // `ErrorKind::NotADirectory`) — the test pins that the wrapper
    // does NOT swallow this as NotFound.
    let scope_rel = "overdrive.slice/workloads.slice/alloc-rmdir-notadir-0.scope";
    let scope_dir_parent = cgroup_root.join("overdrive.slice/workloads.slice");
    std::fs::create_dir_all(&scope_dir_parent).expect("mkdir -p parent");
    let scope_as_file = cgroup_root.join(scope_rel);
    std::fs::write(&scope_as_file, b"").expect("write empty regular file at scope slot");
    assert!(scope_as_file.is_file(), "test setup must place a regular file at scope path");

    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    let manager = CgroupManager::new(cgroup_root, fs);
    let scope = CgroupPath::from_str(scope_rel).expect("valid CgroupPath");

    let err = manager
        .remove_workload_scope(&scope)
        .await
        .expect_err("remove_workload_scope must propagate the real ENOTDIR io::Error");

    let kind = err.kind();
    let raw = err.raw_os_error();
    assert!(
        kind == std::io::ErrorKind::NotADirectory || raw == Some(libc::ENOTDIR),
        "expected ENOTDIR / NotADirectory; got kind={kind:?} raw={raw:?}",
    );
    assert_ne!(
        kind,
        std::io::ErrorKind::NotFound,
        "ENOTDIR must NOT be silently swallowed as NotFound; the wrapper would mask real I/O failures",
    );
}
