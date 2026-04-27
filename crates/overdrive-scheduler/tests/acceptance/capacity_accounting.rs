//! US-01 Scenarios 1.4, 1.5, 1.7 — capacity accounting + zero-capacity edge.
//!
//! # Phase-1 capacity model
//!
//! `AllocStatusRow` carries no per-alloc `Resources` field today
//! (REUSE AS-IS per design wave-decisions). The Phase-1 first-fit
//! scheduler therefore treats each `Running` allocation targeting a
//! node as reserving the resource envelope of the *new job being
//! placed*. The mathematical content of these scenarios is preserved
//! — running allocations subtract from a node's free capacity, and
//! `NoCapacity` reports both `needed` (the requested envelope) and
//! `max_free` (the largest envelope free across the input nodes).
//!
//! Scenario 1.4 in `distill/test-scenarios.md` was originally written
//! with a heterogeneous-resources framing (running alloc consuming
//! 3000 mCPU vs new job needing 2000 mCPU); the Rust translation here
//! preserves the BUSINESS intent (running allocs reduce free capacity
//! and can produce `NoCapacity`) using the homogeneous-job-resources
//! Phase-1 model. Phase 2+ adds per-alloc `Resources` to
//! `AllocStatusRow` and switches to the heterogeneous shape.

use std::collections::BTreeMap;

use overdrive_core::traits::driver::Resources;
use overdrive_scheduler::{PlacementError, schedule};

use super::common::{make_alloc_running, make_job, make_node, res};

#[test]
fn scheduler_subtracts_running_allocs_from_capacity() {
    // Given a node "local" with capacity 6000 mCPU / 6 GiB
    let local = make_node("local", res(6000, 6 * 1024 * 1024 * 1024));
    let mut nodes = BTreeMap::new();
    nodes.insert(local.id.clone(), local);

    // And one Running allocation for "payments" targeting "local"
    // (each running alloc reserves `job.resources` per the Phase-1 model)
    let allocs = vec![make_alloc_running("a-1", "payments", "local")];

    // And a new replica request for "payments" needing 4000 mCPU / 4 GiB
    let job = make_job("payments", res(4000, 4 * 1024 * 1024 * 1024));

    // When schedule is called
    let result = schedule(&nodes, &job, &allocs);

    // Then the result is Err(NoCapacity) because the running alloc
    // consumed 4000 mCPU; only 2000 mCPU remain, but 4000 are needed.
    let Err(PlacementError::NoCapacity { needed, max_free }) = result else {
        panic!("expected NoCapacity, got {result:?}");
    };
    assert_eq!(
        needed,
        Resources { cpu_milli: 4000, memory_bytes: 4 * 1024 * 1024 * 1024 },
        "needed must echo the new job's resources"
    );
    assert_eq!(
        max_free,
        Resources { cpu_milli: 2000, memory_bytes: 2 * 1024 * 1024 * 1024 },
        "max_free must reflect post-running-allocs free capacity"
    );
}

#[test]
fn scheduler_reports_needed_and_max_free_on_memory_exhaustion() {
    // Given a node "local" with 4 GiB free memory
    let local = make_node("local", res(4000, 4 * 1024 * 1024 * 1024));
    let mut nodes = BTreeMap::new();
    nodes.insert(local.id.clone(), local);

    // And a job requesting 8 GiB memory
    let job = make_job("memhog", res(1000, 8 * 1024 * 1024 * 1024));

    // When schedule is called
    let result = schedule(&nodes, &job, &[]);

    // Then the result is Err(NoCapacity) and both fields are populated
    let Err(PlacementError::NoCapacity { needed, max_free }) = result else {
        panic!("expected NoCapacity, got {result:?}");
    };
    assert_eq!(needed.memory_bytes, 8 * 1024 * 1024 * 1024, "needed.memory_bytes");
    assert_eq!(max_free.memory_bytes, 4 * 1024 * 1024 * 1024, "max_free.memory_bytes");
}

#[test]
fn scheduler_handles_zero_capacity_without_underflow() {
    // Given a node "local" with capacity 0 mCPU / 1 byte (1 byte
    // because Node::new rejects zero-memory at the aggregate
    // constructor; the spirit of the scenario is "minimum capacity
    // representable, not enough for the job, must not underflow")
    let local = make_node("local", res(0, 1));
    let mut nodes = BTreeMap::new();
    nodes.insert(local.id.clone(), local);

    // And a job requesting 1000 mCPU / 1 GiB
    let job = make_job("normal", res(1000, 1024 * 1024 * 1024));

    // When schedule is called — must NOT panic from arithmetic underflow
    let result = schedule(&nodes, &job, &[]);

    // Then the result is Err(NoCapacity { ... })
    assert!(
        matches!(result, Err(PlacementError::NoCapacity { .. })),
        "expected NoCapacity for zero-capacity node, got {result:?}"
    );
}
