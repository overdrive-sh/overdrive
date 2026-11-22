//! GH #372 (D-5, DD-5, AC-1.6 / AC-2.6) — the existing golden fixture of the persisted workload
//! intent decodes unchanged through the new `ByteSize` / `CpuQuantity` types, and reads back as the
//! canonical strings of the sizes it was archived with.
//!
//! The hex literals are NOT copied or edited: each test reads the literal out of the fixture file that
//! owns it (`include_str!` of `workload_intent.rs`), so a changed literal in that
//! file changes what this test decodes. A fixture that fails to decode means the layout claim of
//! DD-5 is false for that envelope: stop and surface it (feature delta § Envelope plan).
//!
//! The projection is read through serde (the resources' canonical string form), so the test is
//! independent of the field rename. Evidence tier: unit (layer 1), default lane.

#![allow(clippy::doc_markdown)]

use overdrive_core::aggregate::{WorkloadIntentEnvelope, WorkloadIntentLatest};
use serde_json::{Value, json};

use super::harness::decode_envelope_v_latest;

const WORKLOAD_INTENT_FIXTURES: &str = include_str!("workload_intent.rs");

/// The hex literal assigned to `const <name>: &str = "<hex>";` in a fixture source.
fn literal(source: &str, name: &str) -> String {
    let marker = format!("const {name}: &str = \"");
    let start = source
        .find(&marker)
        .unwrap_or_else(|| panic!("the fixture const {name} must exist in its owning file"))
        + marker.len();
    let len = source[start..]
        .find('"')
        .unwrap_or_else(|| panic!("the fixture const {name} must be terminated"));
    source[start..start + len].to_owned()
}

fn intent_resources(latest: &WorkloadIntentLatest) -> Value {
    let resources = match latest {
        WorkloadIntentLatest::Job(j) => serde_json::to_value(j.resources),
        WorkloadIntentLatest::Service(s) => serde_json::to_value(s.resources),
        WorkloadIntentLatest::Schedule(s) => serde_json::to_value(s.job.resources),
    };
    resources.expect("the decoded resources serialise")
}

/// AC-1.6 / AC-2.6: the pinned workload-intent fixture decodes through the current envelope and its
/// `Resources` read back as the canonical strings of the size archived (`cpu_milli = 500`,
/// `memory_bytes = 128 MiB` is `500m` / `128MiB`).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_workload_intent_fixture_decodes_unchanged_and_reads_back_canonically() {
    // The frozen V1 fixture of a service with guest listeners is decoded through the current
    // envelope (V1 is up-converted to the latest payload), so old stored bytes are what is checked.
    let name = "FIXTURE_V1_SERVICE_GUEST_LISTENERS";
    let hex = literal(WORKLOAD_INTENT_FIXTURES, name);
    let latest = decode_envelope_v_latest::<WorkloadIntentEnvelope>(&hex);
    assert_eq!(
        intent_resources(&latest),
        json!({ "cpu": "500m", "memory": "128MiB" }),
        "{name}: the archived resources must read back as canonical strings"
    );
}
/// AC-1.6 / AC-2.6: the pinned frozen-V1 workload-intent fixture of a Service without listeners decodes
/// through the current envelope and its `[resources]` read back canonically (`500m` / `128MiB`).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_service_intent_fixture_without_listeners_decodes_unchanged_and_reads_back_canonically() {
    let name = "FIXTURE_V1_SERVICE";
    let hex = literal(WORKLOAD_INTENT_FIXTURES, name);
    let latest = decode_envelope_v_latest::<WorkloadIntentEnvelope>(&hex);
    assert!(
        matches!(latest, WorkloadIntentLatest::Service(_)),
        "{name}: the fixture holds a Service workload"
    );
    assert_eq!(
        intent_resources(&latest),
        json!({ "cpu": "500m", "memory": "128MiB" }),
        "{name}: the archived resources must read back as canonical strings"
    );
}
