//! US-02 Scenario 2.6 — `Driver::stop` drives to `Terminated` and
//! removes the workload scope directory after reap.
//!
//! @real-io — Linux. SIGTERM-respecting `/bin/sleep` exits cleanly
//! within the grace window; afterward the scope dir must be gone
//! and `Driver::status` returns `NotFound`.
//!
//! Phase 02 migration: real `/sys/fs/cgroup` per the bugfix RCA § D.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::CgroupFs;
use overdrive_core::traits::driver::{AllocationSpec, Driver, DriverError, Resources};
use overdrive_host::RealCgroupFs;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_worker::ExecDriver;
use overdrive_worker::cgroup_manager::CgroupManager;
use serial_test::serial;
use tokio::time::Instant;

use super::cleanup::AllocCleanup;

#[tokio::test]
#[serial(cgroup)]
async fn stop_with_grace_drives_to_terminated_and_removes_scope() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    // Use an explicit, generous grace window. With SIGTERM working,
    // `/bin/sleep` exits within milliseconds. With SIGTERM swallowed
    // (the `send_sigterm -> ()` mutation), `stop` blocks the full
    // grace window before falling back to `Child::start_kill`. The
    // elapsed-time assertion below is what kills that mutant —
    // without the time bound, the test passes either way because
    // the SIGKILL fallback eventually reaps the workload.
    let stop_grace = Duration::from_secs(5);
    let driver: Arc<dyn Driver> = Arc::new(
        ExecDriver::new(cgroup_root.to_path_buf(), Arc::new(SimClock::new()), fs)
            .with_stop_grace(stop_grace),
    );

    let alloc = AllocationId::new("alloc-stop-grace").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/x/alloc/sg")
            .expect("valid spiffe id"),
        command: "/bin/sleep".to_owned(),
        args: vec!["60".to_owned()],
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        // transparent-mtls-enrollment step 04-01 (JOIN-4/JOIN-6): off the
        // mTLS-composed boot gate — no provisioned netns/veth.
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
    };

    let handle = driver.start(&spec).await.expect("start succeeds");

    let started = Instant::now();
    driver.stop(&handle).await.expect("stop succeeds");
    let elapsed = started.elapsed();

    // SIGTERM-responsive workload — stop must return long before the
    // grace window elapses. A no-op `send_sigterm` would defer
    // termination to the `Child::start_kill` fallback at the grace
    // deadline; capping at half the grace catches that mutation
    // without flaking under load. Empirical happy-path: <100 ms.
    let max_responsive = stop_grace / 2;
    assert!(
        elapsed < max_responsive,
        "stop must return within {max_responsive:?} when the workload \
         responds to SIGTERM (elapsed: {elapsed:?}); a longer wait \
         indicates SIGTERM was not delivered and the grace timeout \
         fell through to SIGKILL escalation",
    );

    // Per `fix-terminated-slot-accumulation` Step 01-02: the driver
    // does not retain a terminal-state slot after stop. Durable
    // terminal-state truth lives in `ObservationStore::AllocStatusRow`;
    // `Driver::status` returns `Err(NotFound)` post-stop. See the
    // `Driver::status` rustdoc in `overdrive-core`.
    let err = driver.status(&handle).await.expect_err("status returns NotFound after stop");
    assert!(
        matches!(err, DriverError::NotFound { ref alloc } if *alloc == handle.alloc),
        "status after stop must be Err(NotFound {{ alloc }}); got {err:?}",
    );

    let scope_dir = cgroup_root.join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));
    assert!(
        !scope_dir.exists(),
        "scope directory must be removed after stop, still present at {}",
        scope_dir.display()
    );
}
