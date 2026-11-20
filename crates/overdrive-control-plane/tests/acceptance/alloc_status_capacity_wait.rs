//! `GET /v1/allocs?job=<id>` reports a workload that is waiting for
//! guest-network capacity (node-local persistent volume, GH #348; ADR-0327,
//! feature delta § "Operator surface: a workload waiting for capacity"): the
//! handler-layer half of the operator's `Waiting` line, the cheap half.
//!
//! The two GUARDS here run on a node with room, so they fill nothing and stay
//! in the default lane. The tests that fill the real attachment pool to its cap
//! of 16,384 (about 56 s per fill on a debug build) live under the
//! `integration-tests` entrypoint, in
//! `tests/integration/alloc_status_capacity_wait.rs`, over the same rig
//! (`capacity_wait_composition`).
//!
//! Contract shape: bounded-change (the response over the reconciler's rows,
//! View, the stop sentinel, the generation and the pool occupancy). Owner path:
//! `handlers::{submit_workload, alloc_status}` over `AppState`, the production
//! reconciler evaluation and action shim, the real `GuestAddressPool`; sim
//! driven ports only; no row, lease or View entry is seeded. The response is
//! read through its wire form so the oracle names only the pinned JSON field.
//! Evidence tier: Tier 1, in-process, deterministic. A GUARD passes today (the
//! handler writes `None`) and must keep passing.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown, clippy::too_many_lines)]

use overdrive_control_plane::api::AllocStatusResponse;
use overdrive_core::traits::observation_store::AllocState;
use overdrive_testing::placement;
use serde_json::Value;

use super::capacity_wait_composition::Node;

placement!(host_pure);

fn wire(response: &AllocStatusResponse) -> Value {
    serde_json::to_value(response).expect("the response serialises")
}

/// GUARD. An ordinary Running workload on a node with room carries no
/// `capacity_wait` key at all (`skip_serializing_if`), so the pre-feature
/// response is unchanged. CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn guard_a_running_workload_with_room_on_the_node_carries_no_capacity_wait() {
    let node = Node::compose().await;
    node.deploy_service("web").await;
    assert!(
        node.evaluate_until("web", |rows| rows.iter().any(|r| r.state == AllocState::Running))
            .await,
        "harness precondition: the workload reaches Running"
    );

    let response = node.describe("web").await;

    assert_eq!(response.capacity_wait, None, "a Running workload is not waiting");
    assert!(
        wire(&response).get("capacity_wait").is_none(),
        "the field is absent from the wire when the workload is not waiting: {}",
        wire(&response)
    );
}

/// GUARD, boundary. A workload deployed but not yet evaluated, on a node with
/// room, has no rows and is not waiting: the empty-allocation state alone does
/// not make a wait. CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn guard_a_deployed_workload_on_a_node_with_room_is_not_waiting() {
    let node = Node::compose().await;
    node.deploy_service("web").await;

    let response = node.describe("web").await;

    assert!(response.rows.is_empty(), "no evaluation has run, so no row exists");
    assert_eq!(response.capacity_wait, None, "no allocation yet is not a capacity wait");
}
