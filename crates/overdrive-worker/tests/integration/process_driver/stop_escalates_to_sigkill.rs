//! US-02 Scenario 2.7 — `Driver::stop` escalates to SIGKILL when the
//! workload ignores SIGTERM beyond the grace window.
//!
//! @real-io — Linux. The workload is a `/bin/sh -c 'trap "" TERM; ...'`
//! that ignores SIGTERM. After the grace window elapses, the driver
//! sends SIGKILL; the test asserts the process is reaped and the
//! state advances to `Terminated`.

use std::sync::Arc;
use std::time::Duration;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::driver::{AllocationSpec, AllocationState, Driver, Resources};
use overdrive_worker::ProcessDriver;
use tempfile::TempDir;
use tokio::time::Instant;

#[tokio::test]
async fn stop_escalates_to_sigkill_when_sigterm_ignored() {
    let cgroup_root = TempDir::new().expect("tempdir created");
    std::fs::create_dir_all(cgroup_root.path().join("overdrive.slice/workloads.slice"))
        .expect("workloads.slice created");

    // Custom stop-grace duration to keep the test fast — 250ms.
    let driver: Arc<dyn Driver> = Arc::new(
        ProcessDriver::new(cgroup_root.path().to_path_buf())
            .with_stop_grace(Duration::from_millis(250)),
    );

    let alloc = AllocationId::new("alloc-stop-sigkill").expect("valid alloc id");
    // /bin/sh that traps and ignores SIGTERM; sleeps 60s.
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/x/alloc/sk")
            .expect("valid spiffe id"),
        image: "/bin/sh".to_owned(),
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
    };

    let handle = driver.start(&spec).await.expect("start succeeds");

    let started = Instant::now();
    driver.stop(&handle).await.expect("stop eventually succeeds via SIGKILL");
    let elapsed = started.elapsed();

    // The stop must have waited at least the grace window before
    // escalating, but not by orders of magnitude.
    assert!(
        elapsed >= Duration::from_millis(250),
        "stop returned faster than the configured grace ({elapsed:?})"
    );
    assert!(elapsed < Duration::from_secs(10), "stop did not escalate within budget ({elapsed:?})");

    let state = driver.status(&handle).await.expect("status succeeds");
    assert_eq!(state, AllocationState::Terminated);
}
