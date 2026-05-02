//! Regression test for `fix-terminated-slot-accumulation` Step 01-01.
//!
//! Asserts `ExecDriver::live` does not accumulate per-allocation
//! entries across the start/stop lifecycle. After N start+stop cycles
//! against distinct `AllocationId`s, the live-map cardinality must be
//! zero — the workload is gone, the slot must be evicted.
//!
//! RED scaffold: against current code, `Driver::stop` re-inserts
//! `LiveAllocation::Terminated` (see `driver.rs:507`), so after 8
//! stop()s the map contains 8 entries → `live_count() == 8`. The test
//! expects 0 → fails. The GREEN fix (Step 01-02) drops
//! `LiveAllocation::Terminated`, evicts the slot in `stop()`, and
//! makes this test pass.
//!
//! Per `.claude/rules/testing.md` § "RED scaffolds and
//! intentionally-failing commits": this test is committed RED via
//! `git commit --no-verify` so the GREEN-next-commit loop in Step
//! 01-02 has a target to flip.
//!
//! Fixture: a `TempDir` cgroup-root. Same shape as the other
//! exec-driver integration tests (`start_and_running.rs`,
//! `cgroup_procs.rs`): the cgroup operations succeed against plain
//! file writes under `TempDir` without requiring delegated
//! `/sys/fs/cgroup` access.

use std::sync::Arc;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::driver::{AllocationSpec, Driver, Resources};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_worker::ExecDriver;
use tempfile::TempDir;

const CYCLES: usize = 8;

#[tokio::test]
async fn live_map_returns_to_zero_after_eight_start_stop_cycles() {
    // Tempdir cgroup-root — same pattern as the other exec_driver
    // integration tests. Pre-create the workloads slice so
    // `create_workload_scope` finds its parent directory when
    // `Driver::start` runs.
    let cgroup_root = TempDir::new().expect("tempdir created");
    std::fs::create_dir_all(cgroup_root.path().join("overdrive.slice/workloads.slice"))
        .expect("workloads.slice created");
    let driver = ExecDriver::new(cgroup_root.path().to_path_buf(), Arc::new(SimClock::new()));

    // Pre-condition: the live map starts empty.
    assert_eq!(
        driver.live_count(),
        0,
        "live map must be empty before any start; got {}",
        driver.live_count(),
    );

    // 8 start+stop cycles against distinct allocation IDs. Sequential
    // — each stop completes before the next start so the map never
    // holds more than one entry at a time on the GREEN path.
    for cycle in 0..CYCLES {
        let alloc = AllocationId::new(&format!("alloc-live-map-{cycle}")).expect("valid alloc id");
        let spec = AllocationSpec {
            alloc: alloc.clone(),
            identity: SpiffeId::new(&format!("spiffe://overdrive.local/job/livemap/alloc/{cycle}"))
                .expect("valid spiffe id"),
            command: "/bin/sleep".to_owned(),
            args: vec!["60".to_owned()],
            resources: Resources { cpu_milli: 50, memory_bytes: 16 * 1024 * 1024 },
        };

        let handle = driver.start(&spec).await.expect("start succeeds");
        driver.stop(&handle).await.expect("stop succeeds");
    }

    // The defended invariant: every started alloc must have its slot
    // evicted on stop. RED today (`Terminated` slot retained, count
    // == CYCLES); GREEN after Step 01-02 (slot evicted, count == 0).
    assert_eq!(
        driver.live_count(),
        0,
        "RED scaffold (fix-terminated-slot-accumulation Step 01-01): \
         live map must be empty after {CYCLES} start+stop cycles; \
         got {} — `LiveAllocation::Terminated` slots are accumulating. \
         GREEN fix lands in Step 01-02.",
        driver.live_count(),
    );
}
