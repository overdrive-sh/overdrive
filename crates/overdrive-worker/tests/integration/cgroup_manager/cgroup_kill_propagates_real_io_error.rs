//! E1 row 8 — `CgroupManager::cgroup_kill` propagates a REAL
//! `io::Error` from the underlying `tokio::fs::*` syscall when the
//! write target is NOT a directory (ENOTDIR).
//!
//! KEEP-TEMPFILE per the E1 triage matrix at
//! `docs/feature/cgroup-fs-port/distill/test-scenarios.md`. The
//! ENOTDIR-via-regular-file-in-dir-slot mechanism is a contrivance
//! to *trigger* the error; the test *boundary* (real-substrate
//! `io::Error` → propagation through `CgroupManager` → caller) is
//! real and load-bearing. Distinct from the SimCgroupFs-backed
//! `cgroup_kill_is_idempotent_on_missing_scope` acceptance test which
//! covers the LOGIC of NotFound-swallow; this one covers the
//! REAL-SUBSTRATE propagation chain.
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
async fn cgroup_kill_propagates_real_io_error_when_scope_is_a_regular_file() {
    // Real tempdir as the cgroup-root substrate; the test never
    // touches `/sys/fs/cgroup`. RealCgroupFs operates on whatever
    // path it is asked to write — the boundary under test is the
    // `tokio::fs::*` → kernel VFS → `io::Error` propagation chain.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let cgroup_root = tmp.path().to_path_buf();

    // Place a regular file where the workload SCOPE directory should
    // be. CgroupManager::cgroup_kill will then attempt to write
    // `<scope>/cgroup.kill`, which the kernel rejects with ENOTDIR
    // (a regular-file path cannot have child entries).
    let scope_rel = "overdrive.slice/workloads.slice/alloc-enotdir-0.scope";
    let scope_dir_parent = cgroup_root.join("overdrive.slice/workloads.slice");
    std::fs::create_dir_all(&scope_dir_parent).expect("mkdir -p parent");
    let scope_as_file = cgroup_root.join(scope_rel);
    std::fs::write(&scope_as_file, b"").expect("write empty regular file at scope slot");
    assert!(scope_as_file.is_file(), "test setup must place a regular file at scope path");

    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    let manager = CgroupManager::new(cgroup_root, fs);
    let scope = CgroupPath::from_str(scope_rel).expect("valid CgroupPath");

    let err = manager
        .cgroup_kill(&scope)
        .await
        .expect_err("cgroup_kill must propagate the real ENOTDIR io::Error");

    // The real kernel returns ENOTDIR (or NotADirectory) when a
    // path traversal hits a regular file where a directory is
    // expected. Both shapes are accepted: the discrete
    // `ErrorKind::NotADirectory` variant on newer rustc, and the
    // raw `libc::ENOTDIR` errno on older toolchains.
    let kind = err.kind();
    let raw = err.raw_os_error();
    assert!(
        kind == std::io::ErrorKind::NotADirectory || raw == Some(libc::ENOTDIR),
        "expected ENOTDIR / NotADirectory; got kind={kind:?} raw={raw:?}",
    );
    // The NotFound arm in CgroupManager::cgroup_kill MUST NOT have
    // intercepted this — that's the regression this test defends.
    assert_ne!(
        kind,
        std::io::ErrorKind::NotFound,
        "ENOTDIR must NOT be silently swallowed as NotFound; the wrapper would mask real I/O failures",
    );
}
