//! C-controller-validation — writing an out-of-range value to
//! `cpu.weight` returns `EINVAL` (or `InvalidInput`) from the kernel.
//!
//! Tier 3, real-io. Requires Lima sudo.
//!
//! Exercises ADR-0054 § D3 row 3 — the kernel parses and validates
//! controller values; `SimCgroupFs` accepts arbitrary bytes. The
//! `cpu_weight_for` clamp in production means the worker never writes
//! out-of-range values; this scenario is the structural defense
//! against a future refactor that removes the clamp.
//!
//! cgroup v2 `cpu.weight` accepts integers in `1..=10000`. Writing
//! `99999999\n` triggers the kernel's range-check.
//!
//! Scenario reference: `docs/feature/cgroup-fs-port/distill/test-scenarios.md`
//! § C-controller-validation.

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
async fn cpu_weight_out_of_range_returns_einval() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    let alloc = AllocationId::new("alloc-EINVAL-0").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());

    let scope_dir = cgroup_root.join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));
    fs.create_dir(&scope_dir).await.expect("create alloc scope");

    // The +cpu controller is already delegated by
    // create_workloads_slice_with_controllers, so cpu.weight is
    // writable on the child scope.
    let err = fs
        .write(&scope_dir.join("cpu.weight"), b"99999999\n")
        .await
        .expect_err("out-of-range cpu.weight write must fail");

    // The cgroup v2 cpu controller validates the range and returns
    // one of:
    //   - EINVAL — parse-stage rejection (e.g. non-numeric).
    //   - ERANGE — value parsed but outside [1..=10000].
    // Older / newer kernels may surface either; both flow through
    // the same trait method and prove the substrate-boundary
    // validation runs. The ERANGE shape lands as
    // `ErrorKind::Uncategorized` on stable rustc because
    // `io::ErrorKind` has no `OutOfRange` variant — so we discriminate
    // on raw errno for ERANGE and on either ErrorKind or errno for
    // EINVAL.
    let kind = err.kind();
    let raw = err.raw_os_error();
    let accepted = kind == std::io::ErrorKind::InvalidInput
        || raw == Some(libc::EINVAL)
        || raw == Some(libc::ERANGE);
    assert!(
        accepted,
        "expected kernel range-check rejection (EINVAL or ERANGE) of \
         out-of-range cpu.weight; got kind={kind:?} raw={raw:?}",
    );
    // The error MUST carry a real kernel errno — `raw_os_error().is_some()`
    // proves the rejection came from a real syscall, NOT from a
    // shim that silently accepted the write.
    assert!(raw.is_some(), "expected real kernel errno on range-violation; got {err:?}");
}
