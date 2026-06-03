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
}
