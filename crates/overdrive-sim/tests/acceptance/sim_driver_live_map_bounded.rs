//! Regression test for `fix-terminated-slot-accumulation` Step 01-01
//! — sim-side counterpart of
//! `crates/overdrive-worker/tests/integration/exec_driver/live_map_bounded.rs`.
//!
//! Asserts `SimDriver::allocations` does not accumulate per-allocation
//! entries across the start/stop lifecycle. After N start+stop cycles
//! against distinct `AllocationId`s, the map cardinality must be
//! zero. The host (`ExecDriver`) and sim (`SimDriver`) bindings must
//! agree on the post-stop cardinality contract, otherwise DST diverges
//! from production behaviour for the same `Driver` trait surface.
//!
//! RED scaffold: against current code, `SimDriver::stop` overwrites
//! the slot to `AllocationState::Terminated` (see `driver.rs:205`)
//! rather than removing it, so after 8 stop()s the map contains 8
//! entries → `live_count() == 8`. The test expects 0 → fails. The
//! GREEN fix (Step 01-02) removes the entry in `stop()`.
//!
//! Per `.claude/rules/testing.md` § "RED scaffolds and
//! intentionally-failing commits": this test is committed RED via
//! `git commit --no-verify` so the GREEN-next-commit loop in Step
//! 01-02 has a target to flip.

use std::str::FromStr;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::driver::{AllocationSpec, Driver, DriverType, Resources};
use overdrive_sim::adapters::driver::SimDriver;

const CYCLES: usize = 8;

#[tokio::test]
async fn sim_driver_live_map_returns_to_zero_after_eight_start_stop_cycles() {
    let driver = SimDriver::new(DriverType::Exec);

    // Pre-condition: the allocations map starts empty.
    assert_eq!(
        driver.live_count(),
        0,
        "sim driver allocations map must be empty before any start; got {}",
        driver.live_count(),
    );

    // 8 start+stop cycles against distinct allocation IDs. Sequential
    // — each stop completes before the next start so the map never
    // holds more than one entry at a time on the GREEN path.
    for cycle in 0..CYCLES {
        let alloc =
            AllocationId::from_str(&format!("alloc-sim-live-map-{cycle}")).expect("valid alloc id");
        let identity =
            SpiffeId::new(&format!("spiffe://overdrive.local/job/livemap/alloc/sim{cycle}"))
                .expect("valid spiffe id");
        let spec = AllocationSpec {
            alloc: alloc.clone(),
            identity,
            command: "registry/livemap:1.0".to_owned(),
            args: vec![],
            resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
        };

        let handle = driver.start(&spec).await.expect("start succeeds");
        driver.stop(&handle).await.expect("stop succeeds");
    }

    // The defended invariant: every started alloc must have its slot
    // evicted on stop. RED today (slot overwritten with Terminated,
    // count == CYCLES); GREEN after Step 01-02 (slot removed, count
    // == 0). The contract is shared with `ExecDriver` — see
    // `crates/overdrive-worker/tests/integration/exec_driver/live_map_bounded.rs`.
    assert_eq!(
        driver.live_count(),
        0,
        "RED scaffold (fix-terminated-slot-accumulation Step 01-01): \
         sim driver allocations map must be empty after {CYCLES} \
         start+stop cycles; got {} — Terminated slots are accumulating. \
         GREEN fix lands in Step 01-02.",
        driver.live_count(),
    );
}
