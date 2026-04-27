//! Step 02-02 / Slice 3A.2 scenario 3.13 — `alloc status` rendering
//! surfaces `PlacementError::NoCapacity` reason text actionably for
//! Pending allocations.
//!
//! When the `JobLifecycle` reconciler hits `NoCapacity` from the
//! scheduler, the obs row stays Pending (the reconciler emits no
//! `StartAllocation`). The CLI/HTTP rendering of that Pending row
//! must surface the reason field so an operator can see what's
//! wrong without spelunking through control-plane logs.
//!
//! Default-lane: pure rendering test — no real server, no real
//! reconciler tick. We exercise the rendering layer directly by
//! constructing a Pending row with a `reason` field and asserting
//! the API response body carries the actionable text.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use overdrive_control_plane::api::AllocStatusRowBody;
use overdrive_core::id::{AllocationId, JobId, NodeId};
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};

#[test]
fn pending_renders_no_capacity_reason_actionably() {
    // Construct a Pending alloc row carrying the NoCapacity reason —
    // the JobLifecycle reconciler writes this shape when scheduler
    // returns Err(NoCapacity { needed, max_free }).
    let row = AllocStatusRow {
        alloc_id: AllocationId::new("alloc-pending-no-cap").expect("valid alloc id"),
        job_id: JobId::new("payments").expect("valid job id"),
        node_id: NodeId::new("node-0").expect("valid node id"),
        state: AllocState::Pending,
        updated_at: LogicalTimestamp {
            counter: 1,
            writer: NodeId::new("node-0").expect("writer node id"),
        },
    };

    // Render with the NoCapacity reason — the rendering layer must
    // accept a reason override so the JobLifecycle reconciler's
    // placement decision can be surfaced to the operator.
    let needed = overdrive_core::traits::driver::Resources {
        cpu_milli: 2000,
        memory_bytes: 4 * 1024 * 1024 * 1024,
    };
    let max_free = overdrive_core::traits::driver::Resources {
        cpu_milli: 500,
        memory_bytes: 1024 * 1024 * 1024,
    };
    let reason = format!("no node has capacity: needed {needed:?}, max free {max_free:?}");

    let body = AllocStatusRowBody::pending_with_reason(&row, reason.clone());

    assert_eq!(body.alloc_id, "alloc-pending-no-cap");
    assert_eq!(body.job_id, "payments");
    assert_eq!(body.node_id, "node-0");
    assert_eq!(body.state, "pending");
    assert_eq!(
        body.reason.as_deref(),
        Some(reason.as_str()),
        "Pending row must surface the NoCapacity reason actionably"
    );

    // Reason must mention the requested envelope and the max free seen
    // so an operator can act on it (add capacity / shrink the job).
    let reason_text = body.reason.expect("reason set");
    assert!(
        reason_text.contains("needed"),
        "reason must mention 'needed' (the requested envelope), got: {reason_text}"
    );
    assert!(
        reason_text.contains("max free"),
        "reason must mention 'max free' (the largest free envelope), got: {reason_text}"
    );
}

#[test]
fn non_pending_row_renders_with_no_reason() {
    // Running rows have no NoCapacity context — the reason field stays None
    // when no reason override is applied (default From conversion).
    let row = AllocStatusRow {
        alloc_id: AllocationId::new("alloc-running").expect("valid alloc id"),
        job_id: JobId::new("payments").expect("valid job id"),
        node_id: NodeId::new("node-0").expect("valid node id"),
        state: AllocState::Running,
        updated_at: LogicalTimestamp {
            counter: 1,
            writer: NodeId::new("node-0").expect("writer node id"),
        },
    };

    let body = AllocStatusRowBody::from(row);

    assert_eq!(body.state, "running");
    assert!(body.reason.is_none(), "Running rows carry no reason text");
}
