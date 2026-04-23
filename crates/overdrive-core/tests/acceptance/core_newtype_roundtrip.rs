//! Acceptance scenarios for US-01 §2.1 — core newtype round-trip.
//!
//! Translates `docs/feature/phase-1-foundation/distill/test-scenarios.md`
//! §2.1 (four round-trip scenarios + the config-file realistic-value
//! scenario) directly into Rust `#[test]` bodies. No `.feature` files,
//! no BDD runner — plain port-to-port tests against the public API of
//! `overdrive_core::id`.
//!
//! The property-shaped scenarios (one per newtype) are exercised with a
//! small manual table here for the acceptance path; the full proptest
//! coverage required by `.claude/rules/testing.md` lives in
//! `crates/overdrive-core/tests/newtype_proptest.rs`.

use std::str::FromStr;

use overdrive_core::id::{AllocationId, JobId, NodeId};

// -----------------------------------------------------------------------------
// §2.1 — scenario 1: Newtype round-trip through Display and FromStr is
// lossless for JobId
// -----------------------------------------------------------------------------

#[test]
fn job_id_round_trips_through_display_and_from_str() {
    // Given any valid JobId value produced by the newtype generator.
    // For the acceptance path we use a small representative table of
    // values that exercise the DNS-1123-like character class (lowercase
    // ascii, digits, `-`, `_`, `.`).
    let inputs =
        ["payments", "payments-api-v2", "node_01", "region.eu-west-1", "a", "a9", "svc-1.internal"];

    for raw in inputs {
        let original = JobId::new(raw).expect("valid input");

        // When Ana formats it via Display and parses the output via FromStr.
        let rendered = original.to_string();
        let parsed = JobId::from_str(&rendered).expect("display output re-parses");

        // Then the parsed value equals the original.
        assert_eq!(parsed, original, "round-trip must be lossless for {raw:?}");
    }
}

// -----------------------------------------------------------------------------
// §2.1 — scenario 2: Newtype round-trip for NodeId
// -----------------------------------------------------------------------------

#[test]
fn node_id_round_trips_through_display_and_from_str() {
    let inputs = ["worker-01", "control-plane-a", "node.example.internal", "n1"];

    for raw in inputs {
        let original = NodeId::new(raw).expect("valid input");
        let rendered = original.to_string();
        let parsed = NodeId::from_str(&rendered).expect("display output re-parses");
        assert_eq!(parsed, original, "round-trip must be lossless for {raw:?}");
    }
}

// -----------------------------------------------------------------------------
// §2.1 — scenario 3: Newtype round-trip for AllocationId
// -----------------------------------------------------------------------------

#[test]
fn allocation_id_round_trips_through_display_and_from_str() {
    let inputs = ["a1b2c3", "alloc-00000001", "payments-alloc.001", "x"];

    for raw in inputs {
        let original = AllocationId::new(raw).expect("valid input");
        let rendered = original.to_string();
        let parsed = AllocationId::from_str(&rendered).expect("display output re-parses");
        assert_eq!(parsed, original, "round-trip must be lossless for {raw:?}");
    }
}

// -----------------------------------------------------------------------------
// §2.1 — scenario 4: serde JSON output matches Display byte-for-byte
// -----------------------------------------------------------------------------

#[test]
fn serde_json_output_equals_display_quoted_for_job_id() {
    let id = JobId::new("payments-api-v2").expect("valid input");

    // When Ana serialises it via serde_json.
    let json = serde_json::to_string(&id).expect("serialises");

    // Then the output equals the Display form surrounded by quotes.
    let expected = format!("\"{id}\"");
    assert_eq!(json, expected);
    assert_eq!(json, "\"payments-api-v2\"");

    // And deserialising the output produces the original value.
    let back: JobId = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(back, id);
}

#[test]
fn serde_json_output_equals_display_quoted_for_node_id() {
    let id = NodeId::new("worker-01").expect("valid input");
    let json = serde_json::to_string(&id).expect("serialises");
    assert_eq!(json, format!("\"{id}\""));
    assert_eq!(json, "\"worker-01\"");
    let back: NodeId = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(back, id);
}

#[test]
fn serde_json_output_equals_display_quoted_for_allocation_id() {
    let id = AllocationId::new("alloc-00000001").expect("valid input");
    let json = serde_json::to_string(&id).expect("serialises");
    assert_eq!(json, format!("\"{id}\""));
    assert_eq!(json, "\"alloc-00000001\"");
    let back: AllocationId = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(back, id);
}

// -----------------------------------------------------------------------------
// §2.1 — scenario 5 (realistic-value): a JobId parses from a realistic
// config-file value
// -----------------------------------------------------------------------------

#[test]
fn job_id_parses_from_realistic_config_value() {
    // Given the input "payments-api-v2" read from a TOML configuration file.
    let input = "payments-api-v2";

    // When Ana constructs a JobId from that input.
    let id = JobId::from_str(input).expect("realistic config value parses");

    // Then Ana receives a valid JobId whose Display output equals
    // "payments-api-v2".
    assert_eq!(id.to_string(), "payments-api-v2");

    // And serialising the JobId to JSON produces the string
    // "\"payments-api-v2\"".
    let json = serde_json::to_string(&id).expect("serialises");
    assert_eq!(json, "\"payments-api-v2\"");
}
