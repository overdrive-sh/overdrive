//! Acceptance — `wire-exec-spec-end-to-end` `OpenAPI` propagation.
//!
//! Per ADR-0031 §8 / DWD-8 in
//! `docs/feature/wire-exec-spec-end-to-end/distill/wave-decisions.md`:
//! the live `OverdriveApi::openapi()` rendering must include
//! `JobSpecInput` with the nested `resources` object and the
//! tagged-driver `oneOf` carrying an `exec` variant. This is Layer 1
//! of the two-layer `OpenAPI` defence — Layer 2 is the existing
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

/// Helper — render the live `OpenAPI` document to YAML.
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
    // The original count-based proxy ("appears exactly once") held when
    // `cpu_milli` could only legitimately surface on `ResourcesInput`.
    // Step 01-01 of the cause-class refactor added
    // `TransitionReason::NoCapacity { requested: ResourceEnvelope, free:
    // ResourceEnvelope }` — `ResourceEnvelope` carries `cpu_milli`. Step
    // 01-02 added `ResourcesBody` — also carries `cpu_milli`. Both are
    // legitimate post-refactor surfaces, so the count of `cpu_milli:`
    // tokens is no longer a stable proxy for the invariant.
    //
    // The honest assertion: extract the `JobSpecInput` schema block
    // from the YAML and assert `cpu_milli` does not appear inside
    // *that* block's properties. Other schemas may legitimately carry
    // it.
    let yaml = render_yaml();

    let jobspec_block = extract_schema_block(&yaml, "JobSpecInput")
        .expect("OpenAPI YAML must contain a `JobSpecInput:` schema block");
    assert!(
        !jobspec_block.contains("cpu_milli:"),
        "JobSpecInput must not carry `cpu_milli:` at top level after the ADR-0031 reshape; \
         the field belongs inside the nested `ResourcesInput` only. JobSpecInput block was:\n\
         {jobspec_block}",
    );
    assert!(
        !jobspec_block.contains("memory_bytes:"),
        "JobSpecInput must not carry `memory_bytes:` at top level after the ADR-0031 reshape; \
         the field belongs inside the nested `ResourcesInput` only. JobSpecInput block was:\n\
         {jobspec_block}",
    );

    // The fields still must surface inside `ResourcesInput` exactly —
    // the reshape moved them, it didn't drop them.
    let resources_input_block = extract_schema_block(&yaml, "ResourcesInput")
        .expect("OpenAPI YAML must contain a `ResourcesInput:` schema block");
    assert!(
        resources_input_block.contains("cpu_milli:"),
        "ResourcesInput must carry `cpu_milli:` (the field moved here in the reshape); \
         block was:\n{resources_input_block}",
    );
    assert!(
        resources_input_block.contains("memory_bytes:"),
        "ResourcesInput must carry `memory_bytes:` (the field moved here in the reshape); \
         block was:\n{resources_input_block}",
    );
}

/// Extract the YAML block for a single `components.schemas.<name>:`
/// entry from the rendered `OpenAPI` document. Returns the substring
/// from the schema header line through (but not including) the next
/// sibling schema header at the same indent.
///
/// utoipa 5.x renders component schemas under `components: schemas:`
/// at four-space indent; each schema header is `    <Name>:`. This
/// helper finds the named header and slices to the next four-space
/// header that is not deeper.
fn extract_schema_block<'yaml>(yaml: &'yaml str, schema_name: &str) -> Option<&'yaml str> {
    // utoipa renders schema headers as `    <Name>:` (four spaces, then
    // name, then colon). Find the start of that line.
    let needle = format!("\n    {schema_name}:\n");
    let start = yaml.find(&needle)? + 1; // skip the leading newline
    let after_header = &yaml[start..];

    // The block ends at the next sibling schema header — a line that
    // begins with exactly four spaces, a non-space character, and ends
    // with a colon. Walk lines until we hit one that fits.
    let mut consumed = 0_usize;
    let mut iter = after_header.lines();
    // Always consume the header line itself.
    if let Some(header_line) = iter.next() {
        consumed += header_line.len() + 1; // +1 for the newline
    }
    for line in iter {
        let is_sibling_header = line.len() >= 5
            && line.starts_with("    ")
            && !line.starts_with("     ")
            && line.as_bytes().get(4).is_some_and(|b| *b != b' ')
            && line.trim_end().ends_with(':');
        if is_sibling_header {
            break;
        }
        consumed += line.len() + 1; // +1 for the newline
    }
    Some(&after_header[..consumed.min(after_header.len())])
}
