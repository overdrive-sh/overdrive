//! Architecture.md §8 — `LifecycleEvent` MUST NOT carry an
//! `AllocStatusRow` directly. The broadcast-channel payload is a
//! wire-shape projection (typed `from`/`to` states, typed `reason`,
//! typed `source`); the raw observation row is a *different concept*
//! and the type system enforces the boundary.
//!
//! This fixture attempts to construct a `LifecycleEvent` whose `from`
//! field is replaced with an `AllocStatusRow`. The line must fail to
//! compile because `AllocStatusRow` does NOT implement
//! `From<AllocStatusRow>` for `AllocStateWire` — and even if it did,
//! the canonical `LifecycleEvent` declaration in `action_shim` declares
//! `from: AllocStateWire`, NOT `from: AllocStatusRow`.
//!
//! The diagnostic the compiler produces is the load-bearing assertion:
//! a future refactor that loosens the field to `AllocStatusRow` would
//! break this fixture, surfacing the architectural violation at PR
//! time.

use std::time::Duration;

use overdrive_control_plane::action_shim::LifecycleEvent;
use overdrive_core::TransitionReason;
use overdrive_core::UnixInstant;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};

fn main() {
    let alloc_id = AllocationId::new("alloc-x").unwrap();
    let workload_id = WorkloadId::new("payments").unwrap();
    let node_id = NodeId::new("local").unwrap();
    let row = AllocStatusRow {
        alloc_id: alloc_id.clone(),
        workload_id: workload_id.clone(),
        node_id: node_id.clone(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 1, writer: node_id.clone() },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        // AllocStatusRowV2 additive field (GH #241): present so this
        // compile-fail fixture fails for its SINGLE intended reason
        // (the `from:` type mismatch below), not an incidental
        // missing-field error.
        workload_addr: None,
    };

    // This line MUST fail to compile: `LifecycleEvent.from` is typed
    // `AllocStateWire`, NOT `AllocStatusRow`. The diagnostic names both
    // types so a reviewer can tell which side of the projection they
    // conflated.
    let _ = LifecycleEvent {
        alloc_id,
        workload_id,
        from: row, // <-- expected type `AllocStateWire`, found `AllocStatusRow`
        to: overdrive_control_plane::api::AllocStateWire::Running,
        reason: TransitionReason::Started,
        detail: None,
        source: overdrive_control_plane::api::TransitionSource::Reconciler,
        at: "1@local".to_owned(),
        terminal: None,
    };
}
