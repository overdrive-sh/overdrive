//! `GET /v1/allocs?job=<id>` on a node whose guest-network attachment pool is
//! full (node-local persistent volume, GH #348; ADR-0336, feature delta
//! § "Operator surface: a workload waiting for capacity"): the handler-layer
//! half of the operator's `Waiting` line that needs a full node.
//!
//! Scenario. Other targets' admissions fill the pool to its cap of 16,384
//! through the pool's own admission (one `StartAllocation` per attachment
//! through the production action shim), then the real `alloc_status` handler
//! reads the response. It carries `capacity_wait { resource, held, cap }`
//! exactly while the workload's start is blocked only by capacity; `held` and
//! `cap` are the pool's. On the SAME full node the workloads that are not
//! withheld (a Running service, a finished Job, an operator-stopped service)
//! carry no wait.
//!
//! Cost. One fill of the real pool is about 56 s on a debug build, so this file
//! holds one test with one fill (the two pool-cap fills the acceptance lane used
//! to carry for the first placement and the full-node guards are merged here;
//! the restart-at-cap case is covered by `capacity_wait_composition`). It lives
//! under `integration-tests` for that reason. The cheap guards that need no fill
//! stay in `tests/acceptance/alloc_status_capacity_wait.rs`.
//!
//! Contract shape: bounded-change (the response over the reconciler's rows,
//! View, the stop sentinel, the generation and the pool occupancy). Owner path:
//! `handlers::{submit_workload, stop_workload, alloc_status}` over `AppState`,
//! the production reconciler evaluation and action shim, the real
//! `GuestAddressPool`; the rig is `tests/acceptance/capacity_wait_composition.rs`
//! (sim driven ports only; no row, lease or View entry is seeded). The response
//! is read through its wire form so the oracle names only the pinned JSON field.
//! Evidence tier: Tier 1, in-process, deterministic. The guard assertions run
//! and pass today; the test is PENDING and fails today at the headline wait
//! assertion, which comes after them.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown, clippy::too_many_lines)]

use overdrive_control_plane::api::AllocStatusResponse;
use overdrive_core::guest_network::MAX_GUEST_NETWORK_ATTACHMENTS;
use overdrive_core::traits::driver::ExitKind;
use overdrive_core::traits::observation_store::AllocState;
use overdrive_testing::placement;
use serde_json::{Value, json};
use std::time::Duration;

use super::capacity_wait_rig::{CAP, Node, full_node_wait};

placement!(host_pure);

fn wire(response: &AllocStatusResponse) -> Value {
    serde_json::to_value(response).expect("the response serialises")
}

/// The response carries no capacity wait, in the typed field and on the wire.
fn assert_not_waiting(response: &AllocStatusResponse, what: &str) {
    assert_eq!(response.capacity_wait, None, "{what} is not waiting on a full node");
    assert!(
        wire(response).get("capacity_wait").is_none(),
        "{what}: the field is absent from the wire: {}",
        wire(response)
    );
}

/// PENDING. A first placement is blocked only by capacity exactly when the pool
/// is at its cap: one attachment below the cap the response omits the wait, at
/// the cap it carries `held` and `cap` from the pool, and the wire shape is the
/// pinned one (boundary: cap - 1 versus cap). On the same full node a Running
/// workload (it holds one of the attachments), a finished Job (a restart never
/// gives it a new allocation) and an operator-stopped workload (stopped, not
/// withheld) are not waiting. CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[should_panic(
    expected = "a withheld first placement on a full node: the response must carry the pool's wait at the cap"
)]
async fn a_full_node_reports_the_wait_for_a_withheld_first_placement_and_for_no_other_workload() {
    let node = Node::compose().await;

    // The workloads that must never report a wait, brought to their states on
    // an empty node: `steady` Running (it keeps one attachment), `batch` a
    // finished Job, `parked` an operator-stopped service; the last two have
    // released their attachments.
    node.deploy_service("steady").await;
    assert!(
        node.evaluate_until("steady", |rows| rows.iter().any(|r| r.state == AllocState::Running))
            .await,
        "harness precondition: the service reaches Running"
    );
    node.deploy_job("batch").await;
    assert!(
        node.evaluate_until("batch", |rows| rows.iter().any(|r| r.state == AllocState::Running))
            .await,
        "harness precondition: the Job reaches Running"
    );
    let batch_alloc = node.rows_of("batch").await.remove(0).alloc_id;
    node.driver.inject_exit_after(&batch_alloc, Duration::ZERO, ExitKind::CleanExit);
    assert!(
        node.evaluate_until("batch", |rows| rows.iter().any(|r| r.state.is_terminal())).await,
        "harness precondition: the Job finishes"
    );
    assert!(
        node.evaluate_until("batch", |_| !node.ledger.holds(&batch_alloc)).await,
        "harness precondition: the finished Job's attachment is released"
    );
    node.deploy_service("parked").await;
    assert!(
        node.evaluate_until("parked", |rows| rows.iter().any(|r| r.state == AllocState::Running))
            .await,
        "harness precondition: the service to stop reaches Running"
    );
    let parked_alloc = node.rows_of("parked").await.remove(0).alloc_id;
    node.stop("parked").await;
    assert!(
        node.evaluate_until("parked", |rows| {
            !rows.is_empty() && rows.iter().all(|r| r.state == AllocState::Terminated)
        })
        .await,
        "harness precondition: the stop ends the allocation"
    );
    assert!(
        node.evaluate_until("parked", |_| !node.ledger.holds(&parked_alloc)).await,
        "harness precondition: the stopped allocation's attachment is released"
    );
    assert_eq!(node.ledger.held_count(), 1, "harness precondition: only `steady` holds one");

    // One attachment below the cap the workload would be placed: no wait.
    node.deploy_service("web").await;
    node.fill(CAP - 2).await;
    assert_eq!(node.ledger.held_count(), CAP - 1, "harness precondition: one slot is free");
    let below = node.describe("web").await;
    assert_eq!(below.capacity_wait, None, "one slot below the cap the workload is not waiting");

    // Another target takes the last slot; the workload's evaluation withholds.
    node.fill(1).await;
    assert_eq!(node.ledger.held_count(), CAP, "harness precondition: the node is full");
    node.evaluate("web").await.expect("the evaluation dispatches");
    assert!(node.rows_of("web").await.is_empty(), "harness precondition: nothing was placed");

    // GUARDS on the full node: nobody else is waiting.
    node.evaluate("batch").await.expect("the evaluation dispatches");
    node.evaluate("parked").await.expect("the evaluation dispatches");
    assert_not_waiting(&node.describe("steady").await, "a Running workload");
    assert_not_waiting(&node.describe("batch").await, "a finished Job");
    assert_not_waiting(&node.describe("parked").await, "an operator-stopped workload");

    // The withheld first placement.
    let at_cap = node.describe("web").await;
    assert_eq!(
        at_cap.capacity_wait,
        Some(full_node_wait()),
        "a withheld first placement on a full node: the response must carry the pool's wait at the cap"
    );
    assert_eq!(
        wire(&at_cap)["capacity_wait"],
        json!({"resource": "guest-network-attachments", "held": node.ledger.held_count(),
               "cap": MAX_GUEST_NETWORK_ATTACHMENTS}),
        "the wire shape is {{resource, held, cap}} with the pool's counts"
    );
}
