//! Acceptance — `wire-exec-spec-end-to-end` OpenAPI propagation.
//!
//! Per ADR-0031 §8 / DWD-8 in
//! `docs/feature/wire-exec-spec-end-to-end/distill/wave-decisions.md`:
//! the live `OverdriveApi::openapi()` rendering must include
//! `JobSpecInput` with the nested `resources` object and the
//! tagged-driver `oneOf` carrying an `exec` variant. This is Layer 1
//! of the two-layer OpenAPI defence — Layer 2 is the existing
//! `cargo xtask openapi-check` CI gate that diffs the live render
//! against the checked-in `api/openapi.yaml`.
//!
//! This test calls `OverdriveApi::openapi()` directly, renders to YAML,
//! and asserts the schema shape — no subprocess, no file I/O against
//! `api/openapi.yaml` (Layer 2's job).
//!
//! At RED time the new schema types (`ResourcesInput`, `ExecInput`,
//! `DriverInput`) are NOT registered in
//! `OverdriveApi`'s `components(schemas(...))` macro arm yet. The
//! DELIVER crafter adds them in the same step that lands the
//! `JobSpecInput` reshape. Until then this test panics on the missing
//! schema — the panic IS the specification of work not yet done per
//! `.claude/rules/testing.md` § "RED scaffolds".

#![allow(clippy::expect_used)]

use overdrive_control_plane::api::OverdriveApi;
use utoipa::OpenApi as _;

/// Helper — render the live OpenAPI document to YAML.
fn render_yaml() -> String {
    OverdriveApi::openapi().to_yaml().expect("to_yaml must succeed for the live OpenAPI document")
}

#[test]
fn openapi_schema_carries_jobspec_input_with_nested_resources_and_tagged_driver_exec_variant() {
    let yaml = render_yaml();

    // 1. JobSpecInput schema exists.
    assert!(
        yaml.contains("JobSpecInput"),
        "OpenAPI schema must register `JobSpecInput`; got:\n{yaml}",
    );

    // 2. ResourcesInput is registered as a top-level schema (proves
    //    the DELIVER crafter added it to OverdriveApi's
    //    components(schemas(...))) — without this entry, utoipa would
    //    inline `ResourcesInput` rather than reference it via $ref,
    //    and downstream client codegen would not expose the type.
    assert!(
        yaml.contains("ResourcesInput"),
        "OpenAPI schema must register `ResourcesInput` as a top-level component schema \
         (per ADR-0031 §8 / DWD-8); got:\n{yaml}",
    );

    // 3. ExecInput is registered as a top-level schema.
    assert!(
        yaml.contains("ExecInput"),
        "OpenAPI schema must register `ExecInput` as a top-level component schema; got:\n{yaml}",
    );

    // 4. DriverInput is registered (the tagged-enum dispatch type).
    //    Until DELIVER adds the schemas() entry this assertion fails.
    assert!(
        yaml.contains("DriverInput"),
        "OpenAPI schema must register `DriverInput` as the tagged-enum driver dispatch \
         type; got:\n{yaml}",
    );

    // 5. The `exec` variant name is present (rendered as `exec` per
    //    `#[serde(rename_all = "kebab-case")]` on `DriverInput`).
    assert!(
        yaml.contains("exec"),
        "OpenAPI schema must surface the `exec` driver variant; got:\n{yaml}",
    );

    // 6. The exec block's required fields (command, args) appear.
    //    These are pinned in the ExecInput schema as `required:
    //    [command, args]` per ADR-0031 §8.
    assert!(
        yaml.contains("command"),
        "OpenAPI schema must surface ExecInput.command field; got:\n{yaml}",
    );
    assert!(
        yaml.contains("args"),
        "OpenAPI schema must surface ExecInput.args field; got:\n{yaml}",
    );
}

#[test]
fn openapi_schema_drops_flat_cpu_milli_field_from_jobspec_input_top_level() {
    // The pre-ADR-0031 wire shape carried `cpu_milli` and `memory_bytes`
    // as direct top-level fields on `JobSpecInput`. After the reshape
    // they live inside the nested `ResourcesInput` only — the
    // `JobSpecInput` schema MUST NOT expose them at top level any more.
    //
    // This test asserts the negative: the YAML contains `cpu_milli`
    // exactly inside the ResourcesInput section, NOT as a top-level
    // JobSpecInput field. We approximate by counting occurrences and
    // asserting the count matches "one schema only" — utoipa renders
    // each field once per schema in which it appears.
    let yaml = render_yaml();

    // Count occurrences of `cpu_milli:` (the YAML key form). Pre-reshape
    // it would appear twice: once on `JobSpecInput` and once on
    // `ResourcesInput` (if such a type existed). Post-reshape it
    // appears exactly once — on `ResourcesInput`.
    let count = yaml.matches("cpu_milli:").count();
    assert_eq!(
        count, 1,
        "cpu_milli must appear exactly once in the OpenAPI schema (inside ResourcesInput); \
         got {count} occurrences in:\n{yaml}",
    );

    let count_mem = yaml.matches("memory_bytes:").count();
    assert_eq!(
        count_mem, 1,
        "memory_bytes must appear exactly once in the OpenAPI schema (inside ResourcesInput); \
         got {count_mem} occurrences in:\n{yaml}",
    );
}
