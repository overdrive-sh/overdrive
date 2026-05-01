//! US-02 Scenario 2.6 — `Driver::stop` drives to `Terminated` and
//! removes the workload scope directory after reap.
//!
//! @real-io — Linux. SIGTERM-respecting `/bin/sleep` exits cleanly
//! within the grace window; afterward the scope dir must be gone
//! and `Driver::status` returns `Terminated`.

use std::sync::Arc;
use std::time::Duration;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::driver::{AllocationSpec, AllocationState, Driver, Resources};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_worker::ExecDriver;
use tempfile::TempDir;
use tokio::time::Instant;

#[tokio::test]
async fn stop_with_grace_drives_to_terminated_and_removes_scope() {
    let cgroup_root = TempDir::new().expect("tempdir created");
    std::fs::create_dir_all(cgroup_root.path().join("overdrive.slice/workloads.slice"))
        .expect("workloads.slice created");

    // Use an explicit, generous grace window. With SIGTERM working,
    // `/bin/sleep` exits within milliseconds. With SIGTERM swallowed
    // (the `send_sigterm -> ()` mutation), `stop` blocks the full
    // grace window before falling back to `Child::start_kill`. The
    // elapsed-time assertion below is what kills that mutant —
    // without the time bound, the test passes either way because
    // the SIGKILL fallback eventually reaps the workload.
    let stop_grace = Duration::from_secs(5);
    let driver: Arc<dyn Driver> = Arc::new(
        ExecDriver::new(cgroup_root.path().to_path_buf(), Arc::new(SimClock::new()))
            .with_stop_grace(stop_grace),
    );

    let alloc = AllocationId::new("alloc-stop-grace").expect("valid alloc id");
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/x/alloc/sg")
            .expect("valid spiffe id"),
        command: "/bin/sleep".to_owned(),
        args: vec!["60".to_owned()],
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
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

    let state = driver.status(&handle).await.expect("status succeeds after stop");
    assert_eq!(state, AllocationState::Terminated);

    let scope_dir =
        cgroup_root.path().join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));
    assert!(
        !scope_dir.exists(),
        "scope directory must be removed after stop, still present at {}",
        scope_dir.display()
    );
}
