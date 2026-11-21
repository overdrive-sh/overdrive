//! GH #372 (DD-4) — the `NoCapacity` cause prose an operator reads renders both resource
//! envelopes through the canonical `Display` of the two quantity types.
//!
//! Why this is a test: the prose line is pinned by DESIGN as operator-facing text of a persisted
//! `AllocStatusRow` cause (`TransitionReason::NoCapacity`), and the rename retypes its envelope. No
//! production site emits `NoCapacity` today (GH #261), so this is the only guard of the pinned text;
//! the envelope's wire shape and bare-number refusal are the same serde the spec tests cover.
//!
//! Written against JSON text (the serde shape of `TransitionReason`), so it compiles before the
//! field rename and fails on its own assertion. Evidence tier: unit (layer 1).
//!
//! Pending bodies are complete and marked `#[should_panic(expected = ...)]`; DELIVER removes the marker.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use overdrive_core::TransitionReason;
use serde_json::json;

fn no_capacity(requested: (&str, &str), free: (&str, &str)) -> TransitionReason {
    let wire = json!({
        "kind": "no_capacity",
        "data": {
            "requested": { "cpu": requested.0, "memory": requested.1 },
            "free": { "cpu": free.0, "memory": free.1 }
        }
    });
    serde_json::from_value(wire)
        .unwrap_or_else(|e| panic!("string cpu/memory resources are not accepted yet: {e}"))
}

/// DD-4 / DESIGN § Wire and spec shapes: the `NoCapacity` prose renders both envelopes through the
/// same `Display` as everywhere else: `no capacity (requested cpu=500m mem=128MiB / free cpu=0 mem=0B)`.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn no_capacity_prose_renders_the_canonical_quantities() {
    let reason = no_capacity(("500m", "128MiB"), ("0", "0B"));
    assert_eq!(
        reason.human_readable(),
        "no capacity (requested cpu=500m mem=128MiB / free cpu=0 mem=0B)"
    );
    let reason = no_capacity(("2000m", "1024MiB"), ("1.5", "1536MiB"));
    assert_eq!(
        reason.human_readable(),
        "no capacity (requested cpu=2 mem=1GiB / free cpu=1500m mem=1536MiB)",
        "any spelling renders canonically"
    );
}
