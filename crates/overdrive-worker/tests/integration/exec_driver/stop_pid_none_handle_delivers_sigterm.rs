//! Regression test for the PID-discard bug:
//!
//! `StartAllocation` in the action shim discarded the `AllocationHandle`
//! (including `pid: Some(pid)`) returned by `driver.start()`. When
//! `StopAllocation` constructed a synthetic `AllocationHandle { pid: None }`,
//! `ExecDriver::stop` skipped the `send_sigterm` call because it read the
//! pid exclusively from `handle.pid`, wasting the 5-second grace window
//! before falling through to `cgroup.kill`.
//!
//! This test exercises the pre-fix shim behaviour by calling `driver.stop`
//! with a hand-constructed `pid: None` handle after a real `driver.start`.
//! The driver must use its internally-stored PID (from `LiveAllocation`)
//! rather than `handle.pid`; SIGTERM must reach the workload so it exits
//! within the grace window.
//!
//! Phase 02 migration: real `/sys/fs/cgroup` per the bugfix RCA § D.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::CgroupFs;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, Driver, DriverError, Resources,
};
use overdrive_host::RealCgroupFs;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_worker::ExecDriver;
use overdrive_worker::cgroup_manager::CgroupManager;
use serial_test::serial;
use tokio::time::Instant;

use super::cleanup::AllocCleanup;

#[tokio::test]
#[serial(cgroup)]
async fn stop_with_pid_none_handle_still_delivers_sigterm() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    let stop_grace = Duration::from_secs(5);
    let driver: Arc<dyn Driver> = Arc::new(
        ExecDriver::new(cgroup_root.to_path_buf(), Arc::new(SimClock::new()), fs)
            .with_stop_grace(stop_grace),
    );

    let alloc = AllocationId::new("alloc-pid-none").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/x/alloc/pn")
            .expect("valid spiffe id"),
        command: "/bin/sleep".to_owned(),
        args: vec!["60".to_owned()],
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
    };

    // Start the allocation but intentionally discard the returned handle,
    // simulating the pre-fix action shim's `Ok(_handle) => ...` arm.
    let _discarded = driver.start(&spec).await.expect("start succeeds");

    // Construct a pid: None handle — exactly what the action shim passed
    // to driver.stop() in the StopAllocation path before the fix.
    let pid_none_handle = AllocationHandle { alloc: alloc.clone(), pid: None };

    let started = Instant::now();
    driver.stop(&pid_none_handle).await.expect("stop succeeds");
    let elapsed = started.elapsed();

    // The driver must use its internally-stored PID (from `LiveAllocation`)
    // to deliver SIGTERM, not `handle.pid`. A SIGTERM-responsive workload
    // (`/bin/sleep 60`) exits in milliseconds. Without the fix, `stop`
    // skips `send_sigterm` entirely and blocks the full 5-second grace
    // window before `cgroup.kill` fires — this assertion catches that.
    let max_responsive = stop_grace / 2;
    assert!(
        elapsed < max_responsive,
        "stop with pid: None handle must still deliver SIGTERM via the \
         driver's internal PID tracking (elapsed: {elapsed:?}, limit: {max_responsive:?}); \
         a longer elapsed time indicates the driver relied on handle.pid \
         instead of its own LiveAllocation state",
    );

    // Post-stop invariants: live slot removed, NotFound returned.
    let err =
        driver.status(&pid_none_handle).await.expect_err("status returns NotFound after stop");
    assert!(
        matches!(err, DriverError::NotFound { ref alloc } if *alloc == pid_none_handle.alloc),
        "status after stop must be Err(NotFound {{ alloc }}); got {err:?}",
    );
}
