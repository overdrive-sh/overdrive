//! Regression test for `fix-terminated-slot-accumulation` Step 01-01.
//!
//! Asserts `ExecDriver::live` does not accumulate per-allocation
//! entries across the start/stop lifecycle. After N start+stop cycles
//! against distinct `AllocationId`s, the live-map cardinality must be
//! zero — the workload is gone, the slot must be evicted.
//!
//! Phase 02 of `fix-cgroup-subtree-control-delegation` migrated this
//! test off `tempfile::TempDir` onto real `/sys/fs/cgroup`. Each
//! cycle's distinct alloc id gets its own `AllocCleanup` guard so a
//! mid-loop panic / SIGKILL does not leave behind 8 stale scopes.

use std::path::Path;
use std::sync::Arc;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::driver::{AllocationSpec, Driver, Resources};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_worker::ExecDriver;
use overdrive_worker::cgroup_manager::create_workloads_slice_with_controllers;
use serial_test::serial;

use super::cleanup::AllocCleanup;

const CYCLES: usize = 8;

#[tokio::test]
#[serial(cgroup)]
async fn live_map_returns_to_zero_after_eight_start_stop_cycles() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    create_workloads_slice_with_controllers(cgroup_root)
        .expect("workloads.slice bootstrap succeeds");
    let driver = ExecDriver::new(cgroup_root.to_path_buf(), Arc::new(SimClock::new()));

    // Pre-condition: the live map starts empty.
    assert_eq!(
        driver.live_count(),
        0,
        "live map must be empty before any start; got {}",
        driver.live_count(),
    );

    // Hold every cycle's cleanup guard until the end of the test so
    // that a panic mid-loop does not strand any partially-created
    // scope. Production `Driver::stop` removes the scope on the happy
    // path — the guards' `rmdir` will be a benign no-op there.
    let mut cleanups: Vec<AllocCleanup> = Vec::with_capacity(CYCLES);

    // CYCLES start+stop cycles against distinct allocation IDs.
    // Sequential — each stop completes before the next start so the
    // map never holds more than one entry at a time on the GREEN path.
    for cycle in 0..CYCLES {
        let alloc = AllocationId::new(&format!("alloc-live-map-{cycle}")).expect("valid alloc id");
        cleanups.push(AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone()));
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
    // evicted on stop. Pre-`fix-terminated-slot-accumulation` Step
    // 01-02: `Terminated` slot retained, count == CYCLES; post-fix:
    // slot evicted, count == 0.
    assert_eq!(
        driver.live_count(),
        0,
        "live map must be empty after {CYCLES} start+stop cycles; \
         got {} — `LiveAllocation::Terminated` slots are accumulating.",
        driver.live_count(),
    );

    drop(cleanups);
}
