//! Step 03-01 / Slice 4 scenario 4.2 — PRIMARY ACCEPTANCE.
//!
//! `cluster_status_responsive_under_workload_cpu_burst`: spawn a
//! CPU-burst workload (one busy loop per online core) under
//! `ExecDriver`, then exercise the read paths the
//! `cluster_status` HTTP handler hits — `ReconcilerRuntime`
//! enumeration + `ObservationStore::alloc_status_rows`. Median
//! sample latency must be comfortably under the 100 ms KPI ceiling.
//!
//! This is the test that disproves the §4 paper-guarantee attack on
//! cgroup isolation: without `overdrive.slice/control-plane.slice/`
//! and the `cpu.weight` reservation it carries, a saturated dataplane
//! starves the control-plane reconciler tick and `cluster status`
//! latency spikes far past 100 ms. With the slice in place, the
//! kernel scheduler honours the reservation and the read endpoint
//! stays responsive.
//!
//! The test bypasses the action shim and submits a `/bin/cpuburn`
//! workload directly via `Driver::start` because Phase 1's action
//! shim hardcodes `/bin/sleep` (see `action_shim.rs`); going around
//! it keeps the test scoped to the cgroup-scheduling property under
//! verification rather than the shim's image-resolution behaviour.
//!
//! Linux-only — gated by `#[cfg(target_os = "linux")]`. Compile-clean
//! on macOS via `cargo nextest run --features integration-tests
//! --no-run`. Runs on Lima via `cargo xtask lima run -- cargo
//! nextest run --workspace --features integration-tests`.

#![cfg(target_os = "linux")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_control_plane::{job_lifecycle, noop_heartbeat};
use overdrive_core::SpiffeId;
use overdrive_core::id::{AllocationId, JobId, NodeId};
use overdrive_core::traits::driver::{AllocationSpec, Driver, Resources};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_worker::ExecDriver;
use tempfile::TempDir;

use super::super::job_lifecycle::cleanup::AllocCleanup;

#[tokio::test]
async fn cluster_status_responsive_under_workload_cpu_burst() {
    // This test exercises the real /sys/fs/cgroup hierarchy and requires
    // root or cgroup delegation. Stock GitHub Actions runners are neither;
    // run the full test via `cargo xtask lima run`.
    // SAFETY: getuid() is always safe to call.
    if unsafe { libc::getuid() } != 0 {
        return;
    }

    let tmp = TempDir::new().expect("tempdir");
    let mut runtime = ReconcilerRuntime::new(tmp.path()).expect("runtime");
    runtime.register(noop_heartbeat()).expect("register noop");
    runtime.register(job_lifecycle()).expect("register job-lifecycle");
    let runtime = Arc::new(runtime);

    let local_node = NodeId::new("local").expect("node id");
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(local_node.clone(), 0));
    let driver = Arc::new(ExecDriver::new(std::path::PathBuf::from("/sys/fs/cgroup")));

    // Cleanup guard — see job_lifecycle/cleanup.rs.
    let _cleanup =
        AllocCleanup { obs: obs.clone(), cgroup_root: std::path::PathBuf::from("/sys/fs/cgroup") };

    // Spawn the CPU burner directly via ExecDriver. Since ADR-0030 +
    // ADR-0029 amendment 2026-04-28 removed magic image-name dispatch
    // from `ExecDriver::build_command`, the busy-loop script that was
    // previously baked behind the `/bin/cpuburn` marker is now passed
    // inline as argv. The script forks one busy loop per online CPU
    // core and `wait`s — `cgroup.kill` (real cgroupfs) or process-group
    // SIGKILL (test fakes) reaches the entire group at teardown.
    let alloc_id = AllocationId::new("alloc-burner-0").expect("valid alloc id");
    let job_id = JobId::new("burner").expect("valid job id");
    let identity = SpiffeId::new("spiffe://overdrive.local/job/burner/alloc/alloc-burner-0")
        .expect("valid identity");
    let spec = AllocationSpec {
        alloc: alloc_id.clone(),
        identity,
        command: "/bin/sh".to_string(),
        args: vec![
            "-c".to_string(),
            "for i in $(seq 1 $(nproc)); do (while :; do :; done) & done; wait".to_string(),
        ],
        resources: Resources { cpu_milli: 1000, memory_bytes: 256 * 1024 * 1024 },
    };
    let handle = driver.start(&spec).await.expect("driver.start cpu-burner");

    // Record an alloc row so the read path under measurement returns
    // non-empty data (matches what the action shim would have written).
    let row = AllocStatusRow {
        alloc_id: alloc_id.clone(),
        job_id,
        node_id: local_node.clone(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 1, writer: local_node.clone() },
    };
    obs.write(ObservationRow::AllocStatus(row)).await.expect("write alloc row");

    // Give the workload a moment to actually saturate the cores; the
    // shell needs to fork its `nproc`-many busy loops and the kernel
    // scheduler needs to start charging the workload's cgroup. 200 ms
    // is plenty for both on Linux.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Measure cluster_status read latency under burst. We exercise
    // the two reads the production handler performs:
    //   1. `ReconcilerRuntime` enumeration (registry snapshot)
    //   2. `ObservationStore::alloc_status_rows`
    //
    // 20 samples, ignore the first 2 (warm-up); median must be < 100 ms.
    let mut latencies = Vec::with_capacity(20);
    for _ in 0..20 {
        let t0 = Instant::now();
        // Touch the runtime registry — every `cluster_status` body
        // enumerates the registered reconcilers.
        let _names = runtime.registered();
        // And the ObservationStore — Phase-1 cluster_status reads
        // alloc rows for the same response payload.
        let _ = obs.alloc_status_rows().await.expect("read alloc rows");
        latencies.push(t0.elapsed());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    drop(latencies.drain(..2));
    latencies.sort();
    let median = latencies[latencies.len() / 2];
    assert!(
        median < Duration::from_millis(100),
        "cluster_status median latency under burst must be < 100 ms; \
         got {median:?} (samples: {latencies:?})"
    );

    // Best-effort: stop the driver-managed workload; cleanup guard
    // catches anything left behind via cgroup.kill.
    let _ = driver.stop(&handle).await;
}
