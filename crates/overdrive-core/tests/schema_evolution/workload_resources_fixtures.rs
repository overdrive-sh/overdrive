//! GH #372 (D-5, DD-5, AC-1.6 / AC-2.6) — the existing golden fixtures of the persisted workload
//! intent and the parser-side service spec decode unchanged through the new `ByteSize` /
//! `CpuQuantity` types, and read back as the canonical strings of the sizes they were archived with.
//!
//! The hex literals are NOT copied or edited: each test reads the literal out of the fixture file that
//! owns it (`include_str!` of `workload_intent.rs` / `service_spec.rs`), so a changed literal in either
//! file changes what this test decodes. A fixture that fails to decode means the layout claim of
//! DD-5 is false for that envelope: stop and surface it (feature delta § Envelope plan).
//!
//! The projection is read through serde (the resources' canonical string form), so the test is
//! independent of the field rename. Evidence tier: unit (layer 1), default lane.

#![allow(clippy::doc_markdown)]

use overdrive_core::aggregate::{
    ServiceSpecEnvelope, WorkloadIntentEnvelope, WorkloadIntentLatest,
};
use serde_json::{Value, json};

use super::harness::decode_envelope_v_latest;

const WORKLOAD_INTENT_FIXTURES: &str = include_str!("workload_intent.rs");
const SERVICE_SPEC_FIXTURES: &str = include_str!("service_spec.rs");

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
#[should_panic(expected = "the archived resources must read back as canonical strings")]
fn the_workload_intent_fixture_decodes_unchanged_and_reads_back_canonically() {
    // Only the fixture its owning file still asserts byte-for-byte against the current layout is
    // decoded; the older `FIXTURE_V1*` consts there are `#[allow(dead_code)]` history.
    let name = "CURRENT_LAYOUT_V1_SERVICE_GUEST_LISTENERS";
    let hex = literal(WORKLOAD_INTENT_FIXTURES, name);
    let latest = decode_envelope_v_latest::<WorkloadIntentEnvelope>(&hex);
    assert_eq!(
        intent_resources(&latest),
        json!({ "cpu": "500m", "memory": "128MiB" }),
        "{name}: the archived resources must read back as canonical strings"
    );
}

/// AC-1.6 / AC-2.6: the parser-side service spec fixture decodes through the current envelope and its
/// `[resources]` reads back canonically (`100` millicores, 128 MiB). CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the archived resources must read back as canonical strings")]
fn the_service_spec_fixture_decodes_unchanged_and_reads_back_canonically() {
    let hex = literal(SERVICE_SPEC_FIXTURES, "CURRENT_LAYOUT_V1_GUEST_LISTENERS");
    let latest = decode_envelope_v_latest::<ServiceSpecEnvelope>(&hex);
    assert_eq!(
        serde_json::to_value(&latest.resources).expect("the decoded resources serialise"),
        json!({ "cpu": "100m", "memory": "128MiB" }),
        "the archived resources must read back as canonical strings"
    );
}
