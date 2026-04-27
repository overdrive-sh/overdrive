//! US-01 Scenario 1.1 — Scheduler picks the local node when capacity fits.
//!
//! The minimum demonstration of the first-fit pure function: one node
//! "local" with 4 GiB free, one job requesting 1 GiB. Result: Ok(local).

use std::collections::BTreeMap;

use overdrive_scheduler::schedule;

use super::common::{make_job, make_node, nid, res};

#[test]
fn scheduler_picks_local_node_when_capacity_fits() {
    // Given a node "local" with 4 GiB / 4000 mCPU capacity
    let local = make_node("local", res(4000, 4 * 1024 * 1024 * 1024));
    let mut nodes = BTreeMap::new();
    nodes.insert(local.id.clone(), local);

    // And a new job requesting 1 GiB / 500 mCPU
    let job = make_job("payments", res(500, 1024 * 1024 * 1024));

    // When schedule is called with no running allocations
    let result = schedule(&nodes, &job, &[]);

    // Then the result is Ok(local)
    assert_eq!(result, Ok(nid("local")));
}
