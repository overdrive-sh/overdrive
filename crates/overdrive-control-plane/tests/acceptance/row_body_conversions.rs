//! Acceptance tests for the `From<AllocStatusRow>` /
//! `From<NodeHealthRow>` projections from the observation-store row
//! types onto their wire-facing `AllocStatusRowBody` / `NodeRowBody`
//! counterparts.
//!
//! The impls live in `src/handlers.rs` (call site is
//! `handlers::{alloc_status, node_list}`), so a mutation that replaces
//! the conversion body with `Default::default()` produces a wire body
//! full of empty strings — the rows leak onto `/v1/allocs` and
//! `/v1/nodes` with lost identities, lost state, and lost region
//! attribution. The handler tests pass because they only check list
//! length, not field preservation.
//!
//! These tests pin the field-by-field projection: every non-Default
//! component of the source row must appear verbatim in the wire body.

use std::str::FromStr;

use overdrive_control_plane::api::{AllocStateWire, AllocStatusRowBody, NodeRowBody};
use overdrive_core::id::{AllocationId, JobId, NodeId, Region};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, NodeHealthRow,
};

fn sample_alloc_status_row() -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::from_str("alloc-a1b2c3").expect("valid alloc id"),
        job_id: JobId::from_str("payments").expect("valid job id"),
        node_id: NodeId::from_str("node-a").expect("valid node id"),
        state: AllocState::Running,
        updated_at: LogicalTimestamp {
            counter: 1,
            writer: NodeId::from_str("node-a").expect("valid node id"),
        },
        reason: None,
        detail: None,
    }
}

fn sample_node_health_row() -> NodeHealthRow {
    NodeHealthRow {
        node_id: NodeId::from_str("node-a").expect("valid node id"),
        region: Region::from_str("eu-west-1").expect("valid region"),
        last_heartbeat: LogicalTimestamp {
            counter: 1,
            writer: NodeId::from_str("node-a").expect("valid node id"),
        },
    }
}

// ---------------------------------------------------------------------------
// From<AllocStatusRow> for AllocStatusRowBody
// ---------------------------------------------------------------------------

#[test]
fn alloc_status_row_body_carries_all_four_fields_verbatim() {
    let row = sample_alloc_status_row();
    let body: AllocStatusRowBody = row.clone().into();

    // Every wire-body field must carry the source row's canonical
    // string rendering. `Default::default()` would produce four empty
    // strings — this test catches that in one assertion per field.
    assert_eq!(
        body.alloc_id,
        row.alloc_id.to_string(),
        "alloc_id must carry the source AllocationId's canonical rendering",
    );
    assert_eq!(
        body.job_id,
        row.job_id.to_string(),
        "job_id must carry the source JobId's canonical rendering",
    );
    assert_eq!(
        body.node_id,
        row.node_id.to_string(),
        "node_id must carry the source NodeId's canonical rendering",
    );
    assert_eq!(
        body.state,
        AllocStateWire::from(row.state),
        "state must carry the typed AllocStateWire projection of the source AllocState",
    );

    // Belt-and-braces — a mutation that substitutes
    // `Default::default()` (empty strings) would fail this too.
    assert!(!body.alloc_id.is_empty(), "alloc_id must not be empty");
    assert!(!body.job_id.is_empty(), "job_id must not be empty");
    assert!(!body.node_id.is_empty(), "node_id must not be empty");
    // body.state is now typed (AllocStateWire), not String — variant
    // distinctness is asserted in the next test
    // (`alloc_status_row_body_distinguishes_state_variants`).
}

#[test]
fn alloc_status_row_body_distinguishes_state_variants() {
    // Prove the `state` projection is not a constant. Pair with
    // `observation_row_display.rs` — a mutation in `AllocState::fmt`
    // that emits empty string would make every row's `state` empty
    // regardless of source variant. This test pairs the projection
    // with state variance.
    let states = [
        AllocState::Pending,
        AllocState::Running,
        AllocState::Draining,
        AllocState::Suspended,
        AllocState::Terminated,
        AllocState::Failed,
    ];
    let mut rendered: Vec<AllocStateWire> = Vec::new();
    for s in states {
        let mut row = sample_alloc_status_row();
        row.state = s;
        let body: AllocStatusRowBody = row.into();
        rendered.push(body.state);
    }
    // AllocStateWire is Copy + PartialEq + Eq; collect the discriminants
    // by projecting through serde_json so the comparison key is the
    // wire-level lowercase string. Two distinct variants must produce
    // distinct wire keys.
    let keys: Vec<String> = rendered
        .iter()
        .map(|s| serde_json::to_string(s).expect("serialise AllocStateWire"))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        keys.len(),
        sorted.len(),
        "every AllocState variant must produce a distinct AllocStateWire projection; \
         got {keys:?} — either the projection collapsed or the From impl is broken",
    );
}

// ---------------------------------------------------------------------------
// From<NodeHealthRow> for NodeRowBody
// ---------------------------------------------------------------------------

#[test]
fn node_row_body_carries_node_id_and_region_verbatim() {
    let row = sample_node_health_row();
    let body: NodeRowBody = row.clone().into();

    assert_eq!(
        body.node_id,
        row.node_id.to_string(),
        "node_id must carry the source NodeId's canonical rendering",
    );
    assert_eq!(
        body.region,
        row.region.to_string(),
        "region must carry the source Region's canonical rendering",
    );

    // Default::default() would leave both empty.
    assert!(!body.node_id.is_empty(), "node_id must not be empty");
    assert!(!body.region.is_empty(), "region must not be empty");
}

#[test]
fn node_row_body_projection_is_not_constant() {
    // A mutation that swaps the body for a constant Default value
    // would make every row's body identical. Submit two rows with
    // distinct node_ids and regions — their wire bodies must differ.
    let row_a = NodeHealthRow {
        node_id: NodeId::from_str("node-a").expect("valid"),
        region: Region::from_str("eu-west-1").expect("valid"),
        last_heartbeat: LogicalTimestamp {
            counter: 1,
            writer: NodeId::from_str("node-a").expect("valid"),
        },
    };
    let row_b = NodeHealthRow {
        node_id: NodeId::from_str("node-b").expect("valid"),
        region: Region::from_str("us-east-1").expect("valid"),
        last_heartbeat: LogicalTimestamp {
            counter: 1,
            writer: NodeId::from_str("node-b").expect("valid"),
        },
    };
    let body_a: NodeRowBody = row_a.into();
    let body_b: NodeRowBody = row_b.into();

    assert_ne!(
        body_a, body_b,
        "distinct NodeHealthRows must produce distinct NodeRowBody projections; \
         identical output would indicate a constant-body mutation",
    );
    assert_ne!(body_a.node_id, body_b.node_id, "node_id must be per-source");
    assert_ne!(body_a.region, body_b.region, "region must be per-source");
}
