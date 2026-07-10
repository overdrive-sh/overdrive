//! US-01 Scenario 1.6 — Empty node set returns `NoHealthyNode`.
//!
//! Phase-1 single-node guarantees the `node_health` table always has one
//! row, so this branch is operationally unreachable at runtime — but
//! the scheduler is a pure function over its inputs, and an empty
//! `BTreeMap<NodeId, Node>` must produce a structured error rather
//! than panicking.
//!
//! Migrated from the deleted `overdrive-scheduler` crate per ADR-0074;
//! adapted to the kind-agnostic `&Resources` signature.

use std::collections::BTreeMap;

use overdrive_core::id::NodeId;
use overdrive_core::scheduler::{PlacementError, schedule};

use super::scheduler_common::{make_job, res};

#[test]
fn scheduler_returns_no_healthy_node_for_empty_input() {
    // Given an empty BTreeMap of nodes
    let nodes: BTreeMap<NodeId, _> = BTreeMap::new();

    // And a job with any resources
    let job = make_job("anything", res(100, 1024));

    // When schedule is called
    let result = schedule(&nodes, &job.resources, &[]);

    // Then the result is Err(NoHealthyNode)
    assert_eq!(result, Err(PlacementError::NoHealthyNode));
}
