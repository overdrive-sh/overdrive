//! Schema-evolution test entrypoint — per ADR-0048 § 6 and
//! `.claude/rules/testing.md` § "Archive schema-evolution roundtrip".
//!
//! Every rkyv-versioned envelope ships a per-version golden-bytes
//! fixture in `tests/schema_evolution/<envelope_snake>.rs`. Each test
//! constructs the canonical `V<N>` payload, decodes it through the
//! current envelope shape, calls `into_latest()`, and asserts equality
//! against a canonical `Latest` projection. Pre-existing fixtures are
//! NEVER touched — adding a new variant adds a new fixture and a new
//! assertion in the same commit.
//!
//! This file is the Cargo test-binary entrypoint. Per submodule rules,
//! `mod schema_evolution { ... }` shifts the lookup base from
//! `tests/<file>.rs` to `tests/schema_evolution/<file>.rs`.

// `expect` is the standard idiom in test code — a panic with a message
// is exactly what you want when a fixture precondition fails. Matches
// the convention used by `tests/acceptance.rs`.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

mod schema_evolution {
    mod alloc_status_row;
    mod harness;
    mod node_health_row;
    mod probe_result_row;
    mod reconcile_conflict_row;
    mod service_backend_row;
    mod service_hydration_result_row;
    mod service_spec;
    mod workload_intent;

    // built-in-ca (GH #28) — DISTILL RED scaffolds for the two new rkyv
    // envelopes (ADR-0063 D2/D6, ADR-0048 golden-bytes obligation).
    mod issued_certificate_row;
    mod root_ca_key;

    // workflow-result-error-model DISTILL (ADR-0065 § 5 / D5; resolves
    // #217) — the mandatory golden-bytes fixture for the NEW
    // `WorkflowSpecEnvelope` (V1). The fixture FILE
    // (`tests/schema_evolution/workflow_spec.rs`) ships now, DELIVER-ready,
    // but its `mod` line is DELIBERATELY COMMENTED OUT: the fixture
    // references `WorkflowSpecEnvelope` / `WorkflowSpecV1`, which do NOT
    // exist until DELIVER Slice 01 creates them in
    // `overdrive-core::workflow`. Unlike a self-contained
    // `#[should_panic]` acceptance scaffold, a schema-evolution fixture
    // CANNOT compile standalone (the harness needs the real envelope type),
    // so wiring it in now would break the WHOLE schema-evolution test
    // binary against a not-yet-existing type. DELIVER Slice 01, in the same
    // commit that creates `WorkflowSpecEnvelope`: (1) run the file's
    // `print_fixture_v1_bytes` to mint `FIXTURE_V1`, (2) pin
    // `GOLDEN_DISCRIMINANT_OFFSET_V1`, (3) UNCOMMENT the line below.
    // mod workflow_spec; // NEW-3 / D5 / #217 — UNCOMMENT in DELIVER Slice 01
}
