//! US-01 Scenario 1.1 — Scheduler picks the local node when capacity fits.
//!
//! The minimum demonstration of the first-fit pure function
//! (`overdrive_core::scheduler::schedule`): one node "local" with 4 GiB
//! free, one job requesting 1 GiB. Result: Ok(local).
//!
//! Migrated from the deleted `overdrive-scheduler` crate per ADR-0074;
//! adapted to the kind-agnostic `&Resources` signature — the direct-port
//! `schedule` call passes `&job.resources`.

use std::collections::BTreeMap;

use overdrive_core::scheduler::schedule;

use super::scheduler_common::{make_job, make_node, nid, res};
use overdrive_core::guest_network::GuestAttachmentOccupancy;

/// Zero held guest-attachment occupancy: these scenarios exercise CPU/memory
/// placement, not the attachment cap (D-295-R8).
const NO_GUEST_ATTACHMENTS: GuestAttachmentOccupancy =
    GuestAttachmentOccupancy { held: 0, retiring: 0 };

#[test]
fn scheduler_picks_local_node_when_capacity_fits() {
    // Given a node "local" with 4 GiB / 4000 mCPU capacity
    let local = make_node("local", res(4000, 4 * 1024 * 1024 * 1024));
    let mut nodes = BTreeMap::new();
    nodes.insert(local.id.clone(), local);

    // And a new job requesting 1 GiB / 500 mCPU
    let job = make_job("payments", res(500, 1024 * 1024 * 1024));

    // When schedule is called with no running allocations
    let result = schedule(&nodes, &job.resources, &[], NO_GUEST_ATTACHMENTS);

    // Then the result is Ok(local)
    assert_eq!(result, Ok(nid("local")));
}
