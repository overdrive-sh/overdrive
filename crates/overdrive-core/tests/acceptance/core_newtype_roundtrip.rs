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

use overdrive_core::id::{AllocationId, MeshServiceName, NodeId, WorkloadId};
use proptest::prelude::*;

// -----------------------------------------------------------------------------
// §2.1 — scenario 1: Newtype round-trip through Display and FromStr is
// lossless for WorkloadId
// -----------------------------------------------------------------------------

#[test]
fn job_id_round_trips_through_display_and_from_str() {
    // Given any valid WorkloadId value produced by the newtype generator.
    // For the acceptance path we use a small representative table of
    // values that exercise the DNS-1123-like character class (lowercase
    // ascii, digits, `-`, `_`, `.`).
    let inputs =
        ["payments", "payments-api-v2", "node_01", "region.eu-west-1", "a", "a9", "svc-1.internal"];

    for raw in inputs {
        let original = WorkloadId::new(raw).expect("valid input");

        // When Ana formats it via Display and parses the output via FromStr.
        let rendered = original.to_string();
        let parsed = WorkloadId::from_str(&rendered).expect("display output re-parses");

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
    let id = WorkloadId::new("payments-api-v2").expect("valid input");

    // When Ana serialises it via serde_json.
    let json = serde_json::to_string(&id).expect("serialises");

    // Then the output equals the Display form surrounded by quotes.
    let expected = format!("\"{id}\"");
    assert_eq!(json, expected);
    assert_eq!(json, "\"payments-api-v2\"");

    // And deserialising the output produces the original value.
    let back: WorkloadId = serde_json::from_str(&json).expect("deserialises");
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
// §2.1 — scenario 5 (realistic-value): a WorkloadId parses from a realistic
// config-file value
// -----------------------------------------------------------------------------

#[test]
fn job_id_parses_from_realistic_config_value() {
    // Given the input "payments-api-v2" read from a TOML configuration file.
    let input = "payments-api-v2";

    // When Ana constructs a WorkloadId from that input.
    let id = WorkloadId::from_str(input).expect("realistic config value parses");

    // Then Ana receives a valid WorkloadId whose Display output equals
    // "payments-api-v2".
    assert_eq!(id.to_string(), "payments-api-v2");

    // And serialising the WorkloadId to JSON produces the string
    // "\"payments-api-v2\"".
    let json = serde_json::to_string(&id).expect("serialises");
    assert_eq!(json, "\"payments-api-v2\"");
}

// -----------------------------------------------------------------------------
// S-DBN-NAME-01 — Mesh service name round-trips through Display / FromStr / serde
//
// PROPERTY: for every valid <job> label L, a MeshServiceName built from
// "<L>.svc.overdrive.local" survives Display -> FromStr (re-parse equals
// original), as_str() yields the canonical lowercase <job> label, and serde
// round-trips to the quoted Display form (serde matches Display/FromStr
// exactly — the mandatory newtype rule). Mesh-DNS grammar for
// dial-by-name-responder (ADR-0072 / US-DBN-2). The newtype's own public
// surface IS the driving port (pure-function port-to-port).
//
// Pinned domain-readable canonical case: "server".
// -----------------------------------------------------------------------------

/// A valid v1 `<job>` label: lowercase ascii alphanumerics with `-`/`_` in the
/// interior (NOT `.` — the v1 contract is a SINGLE label, NO namespace
/// segment, ADR-0072:279; `validate_label` permits `.` for OTHER newtypes, but
/// `MeshServiceName::new` adds a single-label guard on top). Must start AND end
/// with an alphanumeric. Sized to reach the `<job>` ≤ `LABEL_MAX` (253) ceiling
/// — interior `{0,251}` so the longest generated single label is
/// `1 + 251 + 1 = 253` chars, pinning the positive length boundary for
/// `MeshServiceName` (the "one shared length ceiling" rule: size off
/// `LABEL_MAX`, never a bespoke smaller number).
fn valid_job_label() -> impl Strategy<Value = String> {
    prop_oneof![
        // Single-char labels (the boundary: start == end == alphanumeric).
        "[a-z0-9]",
        // Multi-char single labels: alnum boundary + interior class (no `.`) +
        // alnum boundary, up to the LABEL_MAX (253) ceiling. Includes the
        // pinned domain-readable "server" / "payments-api" shapes.
        "[a-z0-9][a-z0-9_-]{0,251}[a-z0-9]",
    ]
}

proptest! {
    /// S-DBN-NAME-01: Display -> FromStr -> serde round-trip, and as_str()
    /// yields the canonical lowercase <job> label.
    #[test]
    fn mesh_service_name_round_trips_through_display_from_str_and_serde(
        label in valid_job_label(),
    ) {
        let full = format!("{label}.{}", MeshServiceName::SUFFIX);
        let name = MeshServiceName::new(&full).expect("valid mesh service name");

        // as_str() is the canonical lowercase <job> label (the generator
        // already produces lowercase, so this is identity here; the case-fold
        // invariant is exercised by S-DBN-NAME-02).
        prop_assert_eq!(name.as_str(), label.as_str());

        // Display -> FromStr round-trip is lossless.
        let rendered = name.to_string();
        prop_assert_eq!(&rendered, &full);
        let reparsed = MeshServiceName::from_str(&rendered).expect("display output re-parses");
        prop_assert_eq!(&reparsed, &name);

        // serde matches Display/FromStr exactly: JSON is the quoted Display form.
        let json = serde_json::to_string(&name).expect("serialises");
        prop_assert_eq!(&json, &format!("\"{full}\""));
        let back: MeshServiceName = serde_json::from_str(&json).expect("deserialises");
        prop_assert_eq!(&back, &name);
    }
}

#[test]
fn mesh_service_name_canonical_example_round_trips() {
    // Pinned domain-readable canonical case (S-DBN-NAME-01 @example("server")).
    let name = MeshServiceName::new("server.svc.overdrive.local").expect("valid");
    assert_eq!(name.as_str(), "server");
    assert_eq!(name.to_string(), "server.svc.overdrive.local");
    let json = serde_json::to_string(&name).expect("serialises");
    assert_eq!(json, "\"server.svc.overdrive.local\"");
    let back: MeshServiceName = serde_json::from_str(&json).expect("deserialises");
    assert_eq!(back, name);
}

// -----------------------------------------------------------------------------
// S-DBN-NAME-02 — Mesh service name parse is case-insensitive, canonical form
// is lowercase
//
// PROPERTY: for every valid <job> label L and every case-permutation P of
// "<L>.svc.overdrive.local" (mixed case in BOTH the label AND the suffix),
// MeshServiceName::new(P) succeeds and equals new() of the all-lowercase form,
// and Display emits the lowercase canonical. Workloads type the name as it
// appears in their config; the suffix grammar and the <job> label both fold
// case (the validate_label precedent, id.rs:99).
// -----------------------------------------------------------------------------

/// Randomly upper/lower-case each ASCII alphabetic char of `s`, driven by a
/// proptest-generated bit per character (so shrinking is deterministic).
fn permute_case(s: &str, flips: &[bool]) -> String {
    s.chars()
        .enumerate()
        .map(|(i, c)| {
            if flips.get(i).copied().unwrap_or(false) {
                c.to_ascii_uppercase()
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect()
}

proptest! {
    /// S-DBN-NAME-02: case-insensitive parse, lowercase canonical.
    #[test]
    fn mesh_service_name_parse_is_case_insensitive_with_lowercase_canonical(
        label in valid_job_label(),
        // One case-flip bit per character of the full name (label + "." +
        // suffix). The label can reach LABEL_MAX (253) and the suffix is 19
        // chars, so the full name is at most 253 + 1 + 19 = 273 chars; size the
        // flips to 0..=273 so the case-fold property is exercised across the
        // ENTIRE name (a shorter bound would leave a long name's tail
        // unflipped, never testing case-folding there).
        flips in proptest::collection::vec(any::<bool>(), 0..=273),
    ) {
        let full_lower = format!("{label}.{}", MeshServiceName::SUFFIX);
        let permuted = permute_case(&full_lower, &flips);

        let from_permuted = MeshServiceName::new(&permuted).expect("case-permuted name parses");
        let from_lower = MeshServiceName::new(&full_lower).expect("lowercase name parses");

        // Case-insensitive: any case permutation equals the all-lowercase form.
        prop_assert_eq!(&from_permuted, &from_lower);

        // Display emits the lowercase canonical regardless of input case.
        prop_assert_eq!(from_permuted.as_str(), label.to_ascii_lowercase());
        prop_assert_eq!(from_permuted.to_string(), full_lower);
    }
}
