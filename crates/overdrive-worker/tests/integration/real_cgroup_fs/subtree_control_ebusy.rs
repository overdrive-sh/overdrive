//! C-subtree-control-ebusy — placing a PID directly in a parent
//! cgroup whose `cgroup.subtree_control` already enables controllers
//! returns `EBUSY` (cgroup v2 "no internal process" rule), and
//! `WorkloadsBootstrapError::from_subtree_control_io` dispatches to
//! the typed `SubtreeControlBusy` variant.
//!
//! Tier 3, real-io. Requires Lima sudo.
//!
//! Exercises ADR-0054 § D3 row 2 — the cgroup v2 kernel contract
//! forbids a non-leaf cgroup (one with controllers enabled in its
//! own `subtree_control`) from carrying processes directly in its
//! `cgroup.procs`. The typed EBUSY discrimination in
//! `WorkloadsBootstrapError::from_subtree_control_io` is the
//! structural defense against the "all I/O errors look the same to
//! the caller" failure mode that ADR-0054 § D5 calls out.
//!
//! # Choice of EBUSY trigger
//!
//! cgroup v2 admits several EBUSY shapes on `cgroup.subtree_control`
//! and adjacent surfaces:
//!   - Write to `cgroup.procs` of a parent that has `subtree_control`
//!     enabled (no-internal-process rule). DETERMINISTIC and the
//!     trigger this test uses.
//!   - Write `-<controller>` to a parent's `subtree_control` while a
//!     child has live PIDs. NON-deterministic across kernels — some
//!     versions accept the disable as a no-op when the controller has
//!     no remaining demand.
//!   - Write `+<new-controller>` to a parent's `subtree_control` when
//!     descendants violate the resulting hierarchy constraint. The
//!     bootstrap already delegates `+cpu +memory +io +pids` so a
//!     re-add is a kernel no-op.
//!
//! The chosen trigger (write PID directly into parent cgroup.procs)
//! fires the same `from_subtree_control_io` typed-dispatch path
//! because the underlying `io::Error` carries the same EBUSY
//! `ErrorKind` / errno. The constructor's discrimination is by
//! `ErrorKind` and `raw_os_error`, not by which write surface
//! produced the error — so the typed-dispatch assertion remains
//! load-bearing.
//!
//! `SimCgroupFs` CANNOT model this: writing a PID payload to an
//! in-memory `cgroup.procs` path is just a byte write; the
//! kernel-side no-internal-process validation does not run.
//!
//! Scenario reference: `docs/feature/cgroup-fs-port/distill/test-scenarios.md`
//! § C-subtree-control-ebusy.

use std::path::Path;
use std::sync::Arc;

use overdrive_core::id::AllocationId;
use overdrive_core::traits::CgroupFs;
use overdrive_host::RealCgroupFs;
use overdrive_worker::cgroup_manager::{CgroupManager, WorkloadsBootstrapError};
use serial_test::serial;

use super::super::exec_driver::cleanup::AllocCleanup;

/// RAII guard that SIGKILLs and reaps the spawned sleep PID on test
/// exit, regardless of outcome. `AllocCleanup` covers a workload-scope
/// directory under `workloads.slice`; this guard covers a PID that
/// stayed in the test process's own cgroup because the EBUSY write
/// never moved it.
struct KillOnDrop(libc::pid_t);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        // SAFETY: thin syscall wrapper; pid is a real pid_t we own.
        // SIGKILL is always deliverable.
        unsafe {
            libc::kill(self.0, libc::SIGKILL);
        }
        let mut status: libc::c_int = 0;
        // SAFETY: same as above; loop bails after one successful reap.
        for _ in 0..20 {
            let r = unsafe { libc::waitpid(self.0, &raw mut status, libc::WNOHANG) };
            if r == self.0 || r == -1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

#[tokio::test]
#[serial(cgroup)]
async fn no_internal_process_rule_returns_ebusy_dispatches_to_typed_variant() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    // The cleanup guard reaps any scope dir we create; even though
    // this scenario does NOT create a scope under workloads.slice,
    // the guard's writes/rmdirs are best-effort and no-op when the
    // scope path is absent.
    let alloc = AllocationId::new("alloc-EBUSY-0").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());

    let workloads_slice = cgroup_root.join("overdrive.slice/workloads.slice");

    // Spawn `/bin/sleep 3600` and attempt to enroll it directly in
    // `workloads.slice/cgroup.procs` (the parent). With
    // `subtree_control` already enabling `+cpu +memory +io +pids` on
    // workloads.slice, the kernel rejects the enroll with EBUSY per
    // the "no internal process" rule.
    let child =
        tokio::process::Command::new("/bin/sleep").arg("3600").spawn().expect("spawn /bin/sleep");
    // Linux PIDs are bounded by `kernel.pid_max` (≤ 2^22), well
    // within `i32::MAX`; cast is safe by domain.
    #[allow(clippy::cast_possible_wrap)]
    let pid = child.id().expect("child PID populated") as libc::pid_t;

    // RAII guard reaps the spawned PID on test exit (declared above).
    let _kill_guard = KillOnDrop(pid);

    let parent_procs = workloads_slice.join("cgroup.procs");
    let err = fs
        .write(&parent_procs, format!("{pid}\n").as_bytes())
        .await
        .expect_err("write PID to non-leaf cgroup.procs must fail with EBUSY");

    let kind = err.kind();
    let raw = err.raw_os_error();
    assert!(
        kind == std::io::ErrorKind::ResourceBusy || raw == Some(libc::EBUSY),
        "expected EBUSY / ResourceBusy from no-internal-process rule; \
         got kind={kind:?} raw={raw:?}",
    );

    // Typed-dispatch contract: from_subtree_control_io must produce
    // the discrete SubtreeControlBusy variant for an EBUSY io::Error.
    // The discrimination is by ErrorKind / errno, not by which write
    // surface emitted the error — so EBUSY from any source dispatches
    // to SubtreeControlBusy, defending the typed-classifier contract.
    let mapped = WorkloadsBootstrapError::from_subtree_control_io(err);
    assert!(
        matches!(mapped, WorkloadsBootstrapError::SubtreeControlBusy { .. }),
        "EBUSY io::Error must map to SubtreeControlBusy; got {mapped:?}",
    );
}
