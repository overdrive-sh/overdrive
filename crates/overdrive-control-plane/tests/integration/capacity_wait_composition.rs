//! A workload waiting for guest-network capacity, composed end to end
//! (node-local persistent volume, GH #348; ADR-0336 the view, ADR-0338 the
//! admission handling; feature delta § "Operator surface: a workload waiting
//! for capacity"; DISTILL ruling W-3 = A).
//!
//! Scenario, on one node and one fill of the pool. A service `db` runs on an
//! empty node; other targets then fill the guest-network attachment pool to its
//! cap of 16,384 through the pool's own admission. (a) An operator deploys a VM
//! service `web` onto the full node. While the node is full the alloc-status
//! response carries the capacity wait; `web` has no allocation row, no `Failed`
//! row exists, the reconciler's restart counts are unchanged. One freed slot and
//! the next evaluation place `web`, it reaches `Running` and the response
//! carries no wait. (b) The node is full again; the operator restarts `db`. Its
//! predecessor's lease is released and another target's admission takes the
//! freed slot before the successor is placed: `db` waits (the response carries
//! the wait, no successor row and no `Failed` row exist, the restart counts are
//! unchanged, the predecessor's lease stays released). One freed slot and the
//! next evaluation place the successor and the wait ends.
//!
//! The CLI render of the response (the `Waiting` line) is not driven here: this
//! crate does not depend on `overdrive-cli`. The line is pinned by the CLI's own
//! `workload_describe` tests over a response carrying the wait.
//!
//! Cost. The deploy and the restart share one fill (the two fills the
//! acceptance lane used to carry are merged); a full fill is about 56 s on a
//! debug build, which is why the test lives under `integration-tests`, in the
//! default 120 s slow-test budget.
//!
//! Contract shape: bounded-change over the composed production owners. Owner
//! path (all production) and substituted driven ports: see the rig,
//! `tests/acceptance/capacity_wait_composition.rs`. The test is PENDING: it
//! fails today at the first headline assertion, `describe` showing the wait
//! while the deploy is withheld; the restart half runs once that passes.
//! Evidence tier: Tier 1, in-process, deterministic.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown, clippy::too_many_lines)]

use overdrive_core::traits::observation_store::AllocState;
use overdrive_testing::placement;

use super::capacity_wait_rig::{CAP, MAX_EVALUATIONS, Node, assert_no_failed_row, full_node_wait};

placement!(host_pure);

/// Fault: the node is full (the only stimulus), then a competing admission wins
/// a freed slot; recovery: another workload's cleanup releases one attachment.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[should_panic(
    expected = "a deploy onto a full node: describe must show the capacity wait while a deploy is withheld"
)]
async fn a_deploy_onto_a_full_node_and_a_restart_that_loses_its_slot_wait_without_charge_and_resume_when_a_slot_frees()
 {
    let node = Node::compose().await;
    node.deploy_service("db").await;
    assert!(
        node.evaluate_until("db", |rows| rows.iter().any(|r| r.state == AllocState::Running)).await,
        "harness precondition: the workload reaches Running on an empty node"
    );
    let predecessor = node.rows_of("db").await.remove(0).alloc_id;
    node.fill(CAP - 1).await;
    assert_eq!(node.ledger.held_count(), CAP, "harness precondition: the node is full");

    // (a) A first deploy onto the full node.
    node.deploy_service("web").await;
    let web_counts_before = node.restart_counts("web");
    for _ in 0..3 {
        node.evaluate("web").await.expect("a withheld evaluation dispatches nothing");
        node.settle().await;
    }

    // The workload is withheld, not failed and not charged.
    assert!(node.rows_of("web").await.is_empty(), "a withheld deploy has no allocation row");
    assert_no_failed_row(&node, "web", "while the node is full").await;
    assert_eq!(
        node.restart_counts("web"),
        web_counts_before,
        "a withheld deploy charges no restart"
    );
    assert_eq!(node.ledger.held_count(), CAP, "a withheld deploy takes no attachment");

    let waiting = node.describe("web").await;
    assert_eq!(
        waiting.capacity_wait,
        Some(full_node_wait()),
        "a deploy onto a full node: describe must show the capacity wait while a deploy is withheld"
    );

    // One slot frees; the resync pass's evaluation places the workload.
    node.free_filler(0).await;
    node.resync("web").await;
    assert!(
        node.evaluate_until("web", |rows| rows.iter().any(|r| r.state == AllocState::Running))
            .await,
        "the workload must be placed and reach Running once a slot frees"
    );
    assert_eq!(node.running_count("web").await, 1, "exactly one allocation runs");
    assert_eq!(
        node.describe("web").await.capacity_wait,
        None,
        "describe must stop showing the wait once the workload is placed"
    );

    // (b) The node is full again (`db`, `web` and the remaining fillers hold
    // every attachment); the operator restarts `db`.
    assert_eq!(node.ledger.held_count(), CAP, "harness precondition: the node is full again");
    let db_counts_before = node.restart_counts("db");

    node.restart("db").await;
    let mut released = false;
    for _ in 0..MAX_EVALUATIONS {
        node.evaluate("db").await.expect("the evaluation dispatches");
        node.settle().await;
        if !node.ledger.holds(&predecessor) {
            released = true;
            break;
        }
    }
    assert!(released, "the restart must release the predecessor's lease");
    // Another target's admission takes the slot the predecessor released,
    // before this workload's next evaluation.
    node.fill(1).await;
    for _ in 0..3 {
        node.evaluate("db").await.expect("a withheld evaluation dispatches nothing");
        node.settle().await;
    }

    let rows = node.rows_of("db").await;
    assert!(
        rows.iter().all(|row| row.alloc_id == predecessor),
        "a withheld restart writes no successor row: {rows:?}"
    );
    assert_no_failed_row(&node, "db", "while the node is full").await;
    assert_eq!(
        node.restart_counts("db"),
        db_counts_before,
        "a withheld restart charges no restart"
    );
    assert!(!node.ledger.holds(&predecessor), "the predecessor's lease stays released");

    let waiting = node.describe("db").await;
    assert_eq!(
        waiting.capacity_wait,
        Some(full_node_wait()),
        "a restart that loses its slot: describe must show the capacity wait while a restart is withheld"
    );

    node.free_filler(1).await;
    node.resync("db").await;
    assert!(
        node.evaluate_until("db", |rows| rows
            .iter()
            .any(|r| r.alloc_id != predecessor && r.state == AllocState::Running))
            .await,
        "the successor must be placed and reach Running once a slot frees"
    );
    assert_eq!(
        node.describe("db").await.capacity_wait,
        None,
        "describe must stop showing the wait once the successor runs"
    );
}
