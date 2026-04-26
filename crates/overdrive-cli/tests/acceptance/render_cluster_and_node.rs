//! Acceptance tests for `overdrive_cli::render::{cluster_status, node_list}`
//! — step 05-03, amended at 01-03 of redesign-drop-commit-index.
//!
//! Rendering functions are pure string-builders — no I/O, no server
//! dependency — so they belong in the default acceptance lane rather
//! than the `integration-tests`-gated slow lane. Separating rendering
//! from handler correctness prevents rendering drift from bleeding into
//! handler tests.
//!
//! Acceptance coverage:
//!   (d) `render::cluster_status` emits a multi-line string with
//!       `Mode:`, `Region:`, `Reconcilers:`, `Broker counters:` labels
//!       and the corresponding values — four fields, NOT five. The
//!       `Commit index:` line was dropped per ADR-0020 §Decision §4.
//!   (e) `render::node_list` on an empty result emits a string
//!       containing a zero-node marker AND the
//!       `phase-1-first-workload` reference from the empty-state
//!       message.
//!   (f) `render::node_list` with rows emits one line per node.
//!
//! The walking-skeleton scenario WS-2 (`distill/test-scenarios.md`
//! §1.2 Amendment 2026-04-26 ADR-0020) names this output shape: four
//! fields, broker.dispatched proves the reconciler ran.

use overdrive_cli::commands::cluster::ClusterStatusOutput;
use overdrive_cli::commands::node::NodeListOutput;
use overdrive_control_plane::api::{BrokerCountersBody, NodeRowBody};

fn fixture_cluster_status_output() -> ClusterStatusOutput {
    ClusterStatusOutput {
        mode: "single".to_string(),
        region: "local".to_string(),
        reconcilers: vec!["noop-heartbeat".to_string()],
        broker: BrokerCountersBody { queued: 7, cancelled: 2, dispatched: 5 },
    }
}

// -------------------------------------------------------------------
// (d) render::cluster_status — four-field shape, no Commit-index line
// -------------------------------------------------------------------

#[test]
fn cluster_status_renders_four_fields_no_commit_index_line() {
    let out = fixture_cluster_status_output();
    let rendered = overdrive_cli::render::cluster_status(&out);

    // Four expected labels, in order — pinned keys the operator scans.
    for label in ["Mode:", "Region:", "Reconcilers:", "Broker counters:"] {
        assert!(
            rendered.contains(label),
            "rendered cluster-status must contain label `{label}`; got:\n{rendered}",
        );
    }

    // The `Commit index:` line was dropped per ADR-0020 §Decision §4 —
    // `ClusterStatus` is now `{mode, region, reconcilers, broker}`. A
    // re-introduction of the line is a wire-shape regression.
    assert!(
        !rendered.contains("Commit index"),
        "rendered cluster-status MUST NOT contain a `Commit index` line per ADR-0020; got:\n{rendered}",
    );

    // Values — prove values are rendered, not just labels.
    assert!(
        rendered.contains("single"),
        "rendered cluster-status must contain mode value; got:\n{rendered}",
    );
    assert!(
        rendered.contains("local"),
        "rendered cluster-status must contain region value; got:\n{rendered}",
    );
    assert!(
        rendered.contains("noop-heartbeat"),
        "rendered cluster-status must contain reconciler name; got:\n{rendered}",
    );
    // Broker counter values.
    assert!(
        rendered.contains('7') && rendered.contains('2') && rendered.contains('5'),
        "rendered cluster-status must contain broker counter values; got:\n{rendered}",
    );
}

// -------------------------------------------------------------------
// (e) render::node_list empty-state uses explicit message
// -------------------------------------------------------------------

#[test]
fn render_node_list_empty_state_uses_explicit_message() {
    let out = NodeListOutput {
        rows: vec![],
        empty_state_message: "no nodes yet — run `phase-1-first-workload` to register one"
            .to_string(),
    };
    let rendered = overdrive_cli::render::node_list(&out);

    assert!(
        rendered.contains("0 nodes") || rendered.contains("no nodes"),
        "rendered empty node-list must carry a zero-node marker; got:\n{rendered}",
    );
    assert!(
        rendered.contains("phase-1-first-workload"),
        "rendered empty node-list must carry the phase-1-first-workload reference; got:\n{rendered}",
    );
}

// -------------------------------------------------------------------
// (f) render::node_list with rows emits one line per node
// -------------------------------------------------------------------

#[test]
fn render_node_list_with_rows_emits_one_line_per_node() {
    let out = NodeListOutput {
        rows: vec![
            NodeRowBody { node_id: "node-a".to_string(), region: "local".to_string() },
            NodeRowBody { node_id: "node-b".to_string(), region: "local".to_string() },
        ],
        empty_state_message: "irrelevant when rows present".to_string(),
    };
    let rendered = overdrive_cli::render::node_list(&out);

    assert!(
        rendered.contains("node-a"),
        "rendered node-list must carry `node-a`; got:\n{rendered}",
    );
    assert!(
        rendered.contains("node-b"),
        "rendered node-list must carry `node-b`; got:\n{rendered}",
    );

    // One line per node — count lines containing either node id.
    let lines_with_nodes: usize =
        rendered.lines().filter(|line| line.contains("node-a") || line.contains("node-b")).count();
    assert_eq!(
        lines_with_nodes, 2,
        "rendered node-list must have one line per node; got {lines_with_nodes} lines with node ids in:\n{rendered}",
    );
}
