//! GH #295 D-295-R8 — placement refuses on the node's **held** guest
//! attachments, read through the occupancy read-port, never on the placed
//! workload's own Running rows (FD 2757-2771).
//!
//! `schedule` returns `NoCapacity` exactly when
//! `guest_attachments.held >= MAX_GUEST_NETWORK_ATTACHMENTS`, for every count of
//! the workload's own Running rows; below the cap its CPU/memory decision is
//! the unchanged Phase-1 first-fit computation (R9).

#![allow(clippy::doc_markdown, reason = "the exact CONTRACT_SHAPE marker is repository-mandated")]

use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{Node, WorkloadKind};
use overdrive_core::guest_network::{GuestAttachmentOccupancy, MAX_GUEST_NETWORK_ATTACHMENTS};
use overdrive_core::id::{AllocationId, NodeId, Region, WorkloadId};
use overdrive_core::scheduler::{PlacementError, schedule};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use proptest::prelude::*;

const CAP: u32 = MAX_GUEST_NETWORK_ATTACHMENTS;
/// One past the cap: the largest own-row count the property draws.
const MAX_OWN_ROWS: usize = CAP as usize + 1;

fn node_id() -> NodeId {
    NodeId::new("nd295-node").expect("node id")
}

fn running_row(index: usize, node: &NodeId) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::new(&format!("nd295-cap-{index:05}"))
            .expect("generated allocation id"),
        workload_id: WorkloadId::new("nd295-placed-workload").expect("workload id"),
        node_id: node.clone(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 1, writer: node.clone() },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Job,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

/// The placed workload's own Running rows, built once; each case borrows a
/// prefix of the requested length.
fn own_rows() -> &'static [AllocStatusRow] {
    static ROWS: OnceLock<Vec<AllocStatusRow>> = OnceLock::new();
    ROWS.get_or_init(|| {
        let node = node_id();
        (0..MAX_OWN_ROWS).map(|index| running_row(index, &node)).collect()
    })
}

/// Held counts clustered on the cap boundary (`CAP - 1`, `CAP`, `CAP + 1`) and
/// spread over the rest of the `u32` domain on either side.
fn held_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![0..CAP - 1, (CAP - 1)..=(CAP + 1), (CAP + 2)..=u32::MAX]
}

/// Own Running-row counts: a few, or on the old row-count boundary
/// (`CAP - 1`, `CAP`, `CAP + 1`), which must no longer decide anything.
fn own_row_count_strategy() -> impl Strategy<Value = usize> {
    prop_oneof![0..=8_usize, (CAP as usize - 1)..=MAX_OWN_ROWS]
}

/// Node capacity and per-workload demand, including the zero envelope that
/// isolates the attachment boundary from CPU/memory.
fn resources_strategy() -> impl Strategy<Value = Resources> {
    prop_oneof![
        Just(Resources { cpu_milli: 0, memory_bytes: 0 }),
        (0_u32..=8_000, 0_u64..=(16 << 30))
            .prop_map(|(cpu_milli, memory_bytes)| Resources { cpu_milli, memory_bytes }),
    ]
}

/// The unchanged CPU/memory first-fit decision for a single node (R9), stated
/// from the Phase-1 capacity model rather than read from the scheduler: each
/// of the workload's own Running rows reserves the placed envelope, the
/// reservation saturates, and the node is chosen iff what remains covers the
/// demand; otherwise `NoCapacity` reports what remains.
fn cpu_memory_decision(
    node: &Node,
    own_running_rows: usize,
    needed: &Resources,
) -> Result<NodeId, PlacementError> {
    let rows = u64::try_from(own_running_rows).expect("row count fits u64");
    let cpu_left = u64::from(node.capacity.cpu_milli)
        .saturating_sub(u64::from(needed.cpu_milli).saturating_mul(rows));
    let free = Resources {
        cpu_milli: u32::try_from(cpu_left).expect("remaining CPU never exceeds the node's u32"),
        memory_bytes: node
            .capacity
            .memory_bytes
            .saturating_sub(needed.memory_bytes.saturating_mul(rows)),
    };
    if free.cpu_milli >= needed.cpu_milli && free.memory_bytes >= needed.memory_bytes {
        Ok(node.id.clone())
    } else {
        Err(PlacementError::NoCapacity { needed: *needed, max_free: free })
    }
}

proptest! {
    /// Outcome anchor: OUT-ND295-SHARED-SWITCH.
    /// S-ND295-05B — Placement reads the node's held attachments, not the
    /// workload's rows.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER step 07-03 (S-ND295-05B)"]
    fn placement_refuses_exactly_when_held_attachments_reach_the_cap(
        (held, retiring) in held_strategy().prop_flat_map(|held| (Just(held), 0..=held)),
        own_row_count in own_row_count_strategy(),
        capacity in resources_strategy(),
        needed in resources_strategy(),
    ) {
        // `retiring` is a subset of `held`; it is reported, never compared.
        let occupancy = GuestAttachmentOccupancy { held, retiring };
        let id = node_id();
        let node = Node {
            id: id.clone(),
            region: Region::new("local").expect("region"),
            capacity,
        };
        let nodes = BTreeMap::from([(id, node.clone())]);
        let rows = &own_rows()[..own_row_count];

        let placed = schedule(&nodes, &needed, rows, occupancy);

        if held >= CAP {
            prop_assert!(
                matches!(&placed, Err(PlacementError::NoCapacity { needed: refused, .. }) if *refused == needed),
                "held {held} (retiring {}) at or above the cap {CAP} must refuse with NoCapacity \
                 whatever the {own_row_count} own Running rows and resources; got {placed:?}",
                occupancy.retiring,
            );
        } else {
            prop_assert_eq!(
                &placed,
                &cpu_memory_decision(&node, own_row_count, &needed),
                "held {} below the cap {} leaves the unchanged CPU/memory decision with {} own \
                 Running rows",
                held,
                CAP,
                own_row_count,
            );
        }
    }
}
