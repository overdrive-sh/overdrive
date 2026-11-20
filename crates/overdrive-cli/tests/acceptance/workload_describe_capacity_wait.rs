//! `overdrive workload describe` shows a workload waiting for guest-network
//! capacity (node-local persistent volume, GH #348; ADR-0336, feature delta
//! § "Operator surface: a workload waiting for capacity").
//!
//! Scenario. The server's alloc-status response carries
//! `capacity_wait { resource, held, cap }` while the node's attachment pool is
//! full and the workload's start is blocked only by that. The render prints
//! `Waiting  guest-network attachment capacity is full on this node (16384 of
//! 16384 attached); placement resumes when one frees.` immediately after the
//! kind-aware body, for a Service and for a Job, and prints nothing new when
//! the field is absent. The wire field is additive and skipped when `None`.
//!
//! Contract shape: pure-function (`render::workload_describe`) plus the serde
//! wire shape of `AllocStatusResponse`. Owner path: the render function the
//! CLI calls over the JSON an operator's server returns. Oracle: the rendered
//! text and the serialised JSON. Evidence tier: unit (layer 1). A test marked
//! GUARD passes today (the wire type exists; the render ignores the field) and
//! must keep passing; a test marked PENDING fails today on its own assertion.
//!
//! Not covered here: the suppression of the generic "check that a node is
//! eligible" empty-state message. It is computed inside
//! `commands::workload::describe`, which reads the response from an HTTPS
//! control plane; the pure render only prints the message it is handed. The
//! suppression needs a `describe` driver over a served control plane (an
//! integration-lane fixture), which this acceptance lane may not host.

#![allow(clippy::expect_used, clippy::doc_markdown)]

use overdrive_cli::commands::workload::WorkloadDescribeOutput;
use overdrive_cli::render::workload_describe;
use overdrive_control_plane::api::{AllocStatusResponse, CapacityResource, CapacityWait};
use overdrive_core::aggregate::{Listener, WorkloadKind};
use overdrive_core::dataplane::Proto;
use overdrive_testing::placement;
use serde_json::json;
use std::num::NonZeroU16;

placement!(host_pure);

const WAITING_LINE: &str = "Waiting  guest-network attachment capacity is full on this node (16384 of 16384 attached); placement resumes when one frees.";

const fn full_node() -> CapacityWait {
    CapacityWait { resource: CapacityResource::GuestNetworkAttachments, held: 16_384, cap: 16_384 }
}

/// The response for a workload with no allocation row yet, which is what a
/// waiting workload's response looks like (the wait is shown exactly when no
/// allocation is running or launching).
fn response(kind: WorkloadKind, wait: Option<CapacityWait>) -> AllocStatusResponse {
    let mut response = AllocStatusResponse {
        workload_id: Some("web".to_owned()),
        spec_digest: Some("a".repeat(64)),
        kind: Some(kind),
        replicas_desired: 1,
        replicas_running: 0,
        capacity_wait: wait,
        ..Default::default()
    };
    if kind == WorkloadKind::Service {
        response.listeners =
            vec![Listener { port: NonZeroU16::new(8080).expect("port"), protocol: Proto::Tcp }];
        response.vip = Some("10.96.0.7".to_owned());
    }
    response
}

/// What `commands::workload::describe` hands the render for a response with no
/// rows and no wait suppression involved (empty message).
fn render(response: AllocStatusResponse) -> String {
    workload_describe(&WorkloadDescribeOutput {
        workload_id: "web".to_owned(),
        spec_digest: "a".repeat(64),
        allocations_total: response.rows.len(),
        empty_state_message: String::new(),
        snapshot: response,
    })
}

fn has_line(text: &str, line: &str) -> bool {
    text.lines().any(|l| l == line)
}

/// PENDING. A waiting workload shows the `Waiting` line verbatim, for a Service and for a Job.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the Waiting line must be printed for a workload of kind Service")]
fn describe_shows_the_waiting_line_for_a_waiting_service_and_job() {
    for kind in [WorkloadKind::Service, WorkloadKind::Job] {
        let text = render(response(kind, Some(full_node())));
        assert!(
            has_line(&text, WAITING_LINE),
            "the Waiting line must be printed for a workload of kind {kind:?}, got:\n{text}"
        );
    }
}

/// PENDING. The section adds exactly its own line: removing it leaves the
/// output of the same response without the wait, byte for byte, so nothing
/// else in the describe output moves. CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the Waiting section must add exactly its own line")]
fn the_waiting_section_adds_exactly_one_line_and_changes_nothing_else() {
    let without = render(response(WorkloadKind::Service, None));
    let with = render(response(WorkloadKind::Service, Some(full_node())));

    let stripped: String =
        with.split_inclusive('\n').filter(|line| line.trim_end() != WAITING_LINE).collect();
    assert_eq!(
        stripped, without,
        "the Waiting section must add exactly its own line; with the wait:\n{with}"
    );
    assert!(with.len() > without.len(), "the Waiting section must add exactly its own line");
}

/// PENDING. The line comes immediately after the kind-aware body: after the
/// Service header and replicas line, before the VIP and Listeners sections
/// that follow the body. CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the Waiting line must come right after the kind-aware body")]
fn the_waiting_line_follows_the_kind_aware_body_and_precedes_the_shared_sections() {
    let text = render(response(WorkloadKind::Service, Some(full_node())));

    let waiting = text.find(WAITING_LINE);
    let replicas = text.find("Replicas (desired/running)");
    let listeners = text.find("8080/tcp");
    let vip = text.find("10.96.0.7");
    assert!(
        matches!((waiting, replicas, listeners, vip),
            (Some(w), Some(r), Some(l), Some(v)) if r < w && w < l && w < v),
        "the Waiting line must come right after the kind-aware body (replicas {replicas:?}, \
         waiting {waiting:?}, listeners {listeners:?}, vip {vip:?}), got:\n{text}"
    );
}

/// PENDING, edge. The held and cap counts are the response's, not a constant:
/// a response carrying other counts prints them. CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the Waiting line must print the response's held and cap")]
fn the_waiting_line_prints_the_counts_the_response_carries() {
    let wait = CapacityWait { held: 100, cap: 200, ..full_node() };
    let text = render(response(WorkloadKind::Job, Some(wait)));
    assert!(
        text.contains("(100 of 200 attached)"),
        "the Waiting line must print the response's held and cap, got:\n{text}"
    );
}

/// GUARD (feature disabled). With no wait the output has no `Waiting` text, and a response whose
/// JSON lacks the field renders identically to one that carries `None` (the pre-feature output is
/// unchanged, for both kinds); the wire field is skipped when `None` and, when present, is
/// `{resource, held, cap}`. CONTRACT_SHAPE: pure-function.
#[test]
fn guard_a_workload_that_is_not_waiting_renders_without_the_waiting_line() {
    for kind in [WorkloadKind::Service, WorkloadKind::Job] {
        let explicit_none = render(response(kind, None));
        let mut wire = serde_json::to_value(response(kind, None)).expect("serialises");
        assert!(
            wire.get("capacity_wait").is_none(),
            "{kind:?}: an absent wait is skipped, not serialised as null: {wire}"
        );
        wire.as_object_mut().expect("an object").remove("capacity_wait");
        let from_wire: AllocStatusResponse =
            serde_json::from_value(wire).expect("a response without the field deserialises");
        assert_eq!(
            render(from_wire),
            explicit_none,
            "{kind:?}: absent and None render identically"
        );
        assert!(
            !explicit_none.contains("Waiting"),
            "{kind:?}: no wait, no Waiting line:\n{explicit_none}"
        );
    }
    let waiting = serde_json::to_value(response(WorkloadKind::Service, Some(full_node())))
        .expect("serialises");
    assert_eq!(
        waiting["capacity_wait"],
        json!({"resource": "guest-network-attachments", "held": 16384, "cap": 16384})
    );
}
