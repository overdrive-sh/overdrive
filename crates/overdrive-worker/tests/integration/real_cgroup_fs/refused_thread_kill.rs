//! E23 (user decision 2 of 2026-09-30): a `cgroup.kill` write whose write path
//! cannot get an OS thread returns the refusal as a typed `io::Error` and never
//! aborts the process.
//!
//! Tier 3, real-io, Lima root. The kill capability reaches
//! `RealCgroupFs::write`, which uses `tokio::fs` today
//! (`overdrive-host/src/cgroup_fs.rs`); under the release profile's
//! `panic = "abort"` a `tokio::fs` write that needs a new blocking-pool thread
//! the OS refuses would end the process (`tokio` panics when the blocking pool
//! is empty and the OS refuses a thread). DELIVER step 09-01 moves the write
//! off `tokio::fs` to a fallible thread primitive, so the refusal becomes the
//! write's `io::Error`.
//!
//! RED-against-the-no-panic-baseline: before 09-01 the kill write on the empty
//! blocking pool panics/aborts under the `pids.max` cap; the body never reaches
//! its assertion. After 09-01 the write either lands (needs no new thread) or
//! returns an `io::Error`, and the body reaches the assertion.

#![allow(
    clippy::doc_markdown,
    reason = "the Outcome/CONTRACT_SHAPE header lines use the exact mandated #295 tokens, \
              which carry no backticks by convention"
)]

use std::path::Path;
use std::sync::Arc;

use overdrive_core::cgroup::CgroupPath;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::CgroupFs;
use overdrive_host::RealCgroupFs;
use overdrive_worker::cgroup_manager::CgroupManager;
use serial_test::serial;

use super::super::cgroup_cleanup::AllocCleanup;

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// E23 — A refused OS thread is a typed failure, never a panic.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 09-01 (E23)"]
fn a_kill_write_under_thread_refusal_returns_an_io_error_and_never_aborts() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    let manager = CgroupManager::new(cgroup_root.to_path_buf(), fs.clone());
    let alloc = AllocationId::new("alloc-e23kill-0").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());
    let scope = CgroupPath::for_alloc(&alloc);
    let scope_dir = scope.resolve(cgroup_root);

    // Set the scope up on a throwaway runtime, before any thread cap. The kill
    // path writes `cgroup.kill`, a core cgroup-v2 interface file the kernel
    // synthesises in every cgroup directory without any controller delegation,
    // so creating the scope directory is the whole precondition. `create_dir`
    // is `mkdir -p`, so `overdrive.slice` and `workloads.slice` land in the one
    // call — no slice-controller enrolment (which the kill write never needs).
    let setup_rt =
        tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("setup runtime");
    setup_rt.block_on(async {
        fs.create_dir(&scope_dir).await.expect("create the alloc scope");
    });
    drop(setup_rt);

    // A fresh current-thread runtime has an empty blocking pool, so the kill
    // write's `tokio::fs` `spawn_blocking` needs a NEW OS thread — which the
    // `pids.max` cap refuses.
    let kill_rt =
        tokio::runtime::Builder::new_current_thread().enable_all().build().expect("kill runtime");
    let _refused = overdrive_testing::pids_max::refuse_thread_creation()
        .expect("the Lima substrate delegates the pids controller to the cgroup root");

    let result = kill_rt.block_on(async { manager.cgroup_kill(&scope).await });

    // Reaching this assertion proves the kill write did not abort or panic the
    // process (user decision 2 of 2026-09-30): it either landed through a
    // realization that needed no new thread, or returned a typed `io::Error`
    // for the refused thread.
    match result {
        Ok(()) => {}
        Err(error) => {
            assert!(
                error.raw_os_error().is_some() || error.kind() != std::io::ErrorKind::Other,
                "a refused kill write is a typed io::Error, got {error:?}",
            );
        }
    }
}
