//! Fixed attachment admission boundary for GH #295.

#![allow(clippy::doc_markdown, reason = "the exact CONTRACT_SHAPE marker is repository-mandated")]

use std::collections::BTreeMap;
use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::Node;
use overdrive_core::id::{AllocationId, NodeId, Region, WorkloadId};
use overdrive_core::scheduler::{PlacementError, schedule};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};

fn row(index: usize, node: &NodeId) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::new(&format!("nd295-cap-{index:05}"))
            .expect("generated allocation id"),
        workload_id: WorkloadId::new(&format!("nd295-workload-{index:05}"))
            .expect("generated workload id"),
        node_id: node.clone(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 1, writer: node.clone() },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Job,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn fixed_attachment_cap_returns_no_capacity_before_pool_assignment() {
    for (active, admitted) in [(16_383, true), (16_384, false), (16_385, false)] {
        let node_id = NodeId::new("nd295-node").expect("node id");
        let nodes = BTreeMap::from([(
            node_id.clone(),
            Node {
                id: node_id.clone(),
                region: Region::new("local").expect("region"),
                // Zero demand/capacity isolates the attachment-count boundary
                // from the existing CPU/memory resource calculation.
                capacity: Resources { cpu_milli: 0, memory_bytes: 0 },
            },
        )]);
        let allocations = (0..active).map(|index| row(index, &node_id)).collect::<Vec<_>>();
        let result = schedule(&nodes, &Resources { cpu_milli: 0, memory_bytes: 0 }, &allocations);

        if admitted {
            assert_eq!(result.expect("below cap continues"), node_id);
        } else {
            assert!(
                matches!(result, Err(PlacementError::NoCapacity { .. })),
                "{active} active attachments refuse before any control-plane pool can run"
            );
        }
    }
}
