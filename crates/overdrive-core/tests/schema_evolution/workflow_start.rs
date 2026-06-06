//! Schema-evolution golden-bytes test — `WorkflowStartEnvelope`
//! (DISTILL DELIVER-ready scaffold, `workflow-result-error-model` /
//! ADR-0065 § 5 / D5).
//!
//! Mandatory per ADR-0048 § 1 + `.claude/rules/testing.md` § "Archive
//! schema-evolution roundtrip": `WorkflowStart` grows from identity-only
//! (`{ name }`) to an input-bearing durable INTENT aggregate
//! (`{ name: WorkflowName, input: Vec<u8> }`) read back on every restart,
//! so it crosses the rkyv versioned-envelope + co-located-typed-codec
//! boundary (the `Job` aggregate precedent, ADR-0048 § 4b). This fixture
//! pins the V1 archived layout so any future commit that appends a field
//! to the V1 payload — rather than minting a `V2` — breaks this test.
//!
//! The V1 payload (`WorkflowStartV1`) persists the start INPUTS, never
//! derived state (`development.md` § "Persist inputs, not derived state"):
//! the `name` (kind identity) and the opaque CBOR `input` bytes (the erased
//! `W::Input`). The rkyv envelope wraps the OUTER `WorkflowStart` only — the
//! inner `input: Vec<u8>` stays opaque CBOR (the "aggregate envelopes wrap
//! the outer type only" rule; the two codecs — rkyv outer, CBOR inner —
//! stay separate per `development.md` § "rkyv schema evolution").
//!
//! **`FIXTURE_V1` is never touched** once minted. Bumping to `V2` adds a
//! new `FIXTURE_V2` constant + a new assertion in the same commit; existing
//! constants stay verbatim (`development.md` § "Version-bump procedure").
//!
//! Default lane (no `integration-tests` feature) — pure in-memory rkyv, no
//! I/O.
//!
//! # DELIVER ACTIVATION (Slice 01)
//!
//! This file references `WorkflowStartEnvelope` / `WorkflowStartV1` /
//! `WorkflowStartLatest`, which DO NOT EXIST until DELIVER Slice 01
//! creates them in `overdrive-core::workflow`. It is therefore NOT a
//! green-at-the-bar `#[should_panic]` scaffold — it cannot compile
//! standalone — and is deliberately LEFT UN-WIRED from
//! `tests/schema_evolution.rs` (its `mod workflow_start;` line is commented
//! there with this marker). Wiring it in now would break the WHOLE
//! `overdrive-core` schema-evolution test binary (every other fixture's
//! ability to run) against a type that does not exist yet — the Rust
//! analogue of a fixture that cannot be skipped.
//!
//! Slice 01 DELIVER steps (step 01-02), all in one commit:
//!
//!   1. Create `WorkflowStartV1` + `WorkflowStartEnvelope` (V1) + the
//!      `VersionedEnvelope` impl + the co-located typed codec
//!      (`WorkflowStart::archive_for_store` / `from_store_bytes`).
//!   2. Run `print_fixture_v1_bytes` (below) and paste the hex into
//!      `FIXTURE_V1`; pin `GOLDEN_DISCRIMINANT_OFFSET_V1` from the same run.
//!   3. UNCOMMENT `mod workflow_start;` in `tests/schema_evolution.rs`.
//!
//! All three steps landed in step 01-02; the three `#[test]`s below now run
//! on every `overdrive-core` schema-evolution build.

use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::workflow::{
    WorkflowName, WorkflowStartEnvelope, WorkflowStartLatest, WorkflowStartV1,
};

use super::harness::{
    assert_discriminant_offset_triangulation, assert_envelope_v_roundtrip,
    assert_unknown_version_probe_surfaces,
};

/// Independent pin of the V1 discriminant offset for triangulation against
/// `WorkflowStartEnvelope::discriminant_offset_from_end()` (two-source guard
/// against unilateral drift of either pin, per ADR-0048). On a `V<N+1>`
/// bump BOTH this constant and the trait method update in the same commit.
///
/// Pinned in DELIVER Slice 01 (step 01-02) by regenerating `FIXTURE_V1` and
/// flipping each byte to locate the trailing-root discriminant byte: rkyv
/// rejects a flip at `from_end == 20` with `invalid discriminant for enum
/// 'ArchivedWorkflowStartEnvelope'` (mirror
/// `alloc_status_row.rs::GOLDEN_DISCRIMINANT_OFFSET_V1`). On a `V<N+1>` bump
/// re-pin BOTH this constant and
/// `WorkflowStartEnvelope::discriminant_offset_from_end()` in the same commit.
const GOLDEN_DISCRIMINANT_OFFSET_V1: usize = 20;

/// Canonical V1 payload pinned by `FIXTURE_V1` below. The expected
/// projection is built from these values verbatim — change any one of them
/// and the test fails until `FIXTURE_V1` is regenerated.
///
/// The `input` bytes are a fixed, arbitrary-but-stable CBOR-shaped byte
/// vector standing in for an erased `W::Input` — the fixture pins the
/// rkyv layout of `WorkflowStart` carrying a NON-EMPTY input (the #217
/// shape: identity-only had no envelope; input-bearing does).
fn canonical_v1_payload() -> WorkflowStartLatest {
    WorkflowStartV1 {
        name: WorkflowName::new("provision-record").expect("valid kebab workflow name"),
        // A stable, non-empty opaque CBOR input (the erased W::Input). The
        // value is arbitrary; the test asserts byte-stable round-trip of the
        // OUTER WorkflowStart rkyv layout, not the inner CBOR semantics.
        input: vec![0xa1, 0x63, 0x66, 0x6f, 0x6f, 0x18, 0x2a],
    }
}

/// Hex-encoded rkyv-archived bytes of
/// `WorkflowStartEnvelope::V1(canonical_v1_payload())`.
///
/// Minted by `print_fixture_v1_bytes` (below) in DELIVER Slice 01 (step
/// 01-02) and pasted verbatim. Decodes as
/// `WorkflowStartEnvelope::V1(WorkflowStartV1 { name: "provision-record",
/// input: [0xa1, 0x63, 0x66, 0x6f, 0x6f, 0x18, 0x2a] })`.
///
/// Pre-shipment regeneration is allowed under
/// `feedback_single_cut_greenfield_migrations.md`; once V1 ships to a
/// deployed consumer this constant becomes immutable per
/// `.claude/rules/development.md` § "rkyv schema evolution" — future
/// variants need a `V2` envelope.
const FIXTURE_V1: &str =
    "70726f766973696f6e2d7265636f7264a163666f6f182a000000000090000000e4ffffffecffffff07000000";

/// `@property` `@D5` `@issue-217` (NEW-3) — golden-bytes roundtrip:
/// hex-decode `FIXTURE_V1`, rkyv-deserialise into `WorkflowStartEnvelope`,
/// `into_latest()`, assert equality against the canonical `Latest`
/// projection; the archived layout is byte-stable. A future additive field
/// on `WorkflowStartV1` (instead of a `V2`) shifts the layout and fails this
/// test — the structural defense ADR-0048 mandates.
#[test]
fn workflow_start_envelope_v1_golden_bytes_roundtrip() {
    let expected = canonical_v1_payload();
    assert_envelope_v_roundtrip::<WorkflowStartEnvelope>(FIXTURE_V1, &expected);
}

/// `@property` `@D5` (NEW-3b) — discriminant-offset triangulation: the
/// independent pin (`GOLDEN_DISCRIMINANT_OFFSET_V1`) must agree with
/// `WorkflowStartEnvelope::discriminant_offset_from_end()` (two-source guard
/// against unilateral drift, per ADR-0048).
#[test]
fn workflow_start_envelope_discriminant_offset_triangulates() {
    assert_discriminant_offset_triangulation::<WorkflowStartEnvelope>(
        canonical_v1_payload(),
        GOLDEN_DISCRIMINANT_OFFSET_V1,
        0,
    );
}

/// `@property` `@D5` `@error` (NEW-3c) — an unknown/forward envelope version
/// surfaces `EnvelopeError` via the probe rather than decoding into garbage.
/// This is the intent fail-fast precursor: per ADR-0065 § 5 the `IntentStore`
/// decode path for the persisted `WorkflowStart` intent refuses with a
/// `health.startup.refused`-class surface on this error (intent is SSOT —
/// the ADR-0048 intent asymmetry).
///
/// `supported_max == 0` because today's envelope is V1-only; bumping to V2
/// means re-pinning this assertion in the same commit per
/// `development.md` § "Version-bump procedure".
#[test]
fn workflow_start_envelope_unknown_version_probe_surfaces_error() {
    assert_unknown_version_probe_surfaces::<WorkflowStartEnvelope>(
        canonical_v1_payload(),
        "WorkflowStartEnvelope",
        0,
    );
}

// ---------------------------------------------------------------------
// Bootstrap helper — generates the canonical V1 bytes on demand for the
// crafter to paste into `FIXTURE_V1` above. Run via:
//
//   cargo nextest run -p overdrive-core --test schema_evolution \
//       -E 'test(/workflow_start.*print_fixture_v1_bytes/)' --no-capture
//
// Marked `#[ignore]` so it never runs in normal test execution; the pinned
// `FIXTURE_V1` constant is the load-bearing artifact, this is a one-shot
// regeneration aid (mirror alloc_status_row.rs / root_ca_key.rs).
// ---------------------------------------------------------------------

#[test]
#[ignore = "fixture regeneration tool — run on demand in DELIVER Slice 01 to mint FIXTURE_V1; the pinned FIXTURE_V<N> constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_V1"
)]
fn print_fixture_v1_bytes() {
    let envelope = WorkflowStartEnvelope::latest(canonical_v1_payload());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("FIXTURE_V1 = \"{}\"", hex::encode(bytes.as_ref()));
}
