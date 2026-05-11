#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Acceptance scenarios for phase-1-control-plane-core step 02-04 —
//! `cargo openapi-gen` / `openapi-check` cargo aliases.
//!
//! Covers test-scenarios.md §3.3 — `OpenAPI` schema derivation is
//! byte-identical on repeat runs, and `openapi-check` surfaces drift
//! with an actionable message pointing at the first drifted schema.
//!
//! Tests (a)-(e) drive the implementation in-process via the
//! `overdrive_control_plane::openapi` library surface — determinism
//! and drift detection are pure-Rust properties. Test (f) is the
//! subprocess smoke check against the real workspace: it runs the
//! compiled `openapi` binary and asserts the checked-in
//! `api/openapi.yaml` matches the live schema.
//!
//! Driving port for (a)-(e):
//! `overdrive_control_plane::openapi::{generate_yaml, check_against_disk}`
//! — a pure function pair the binary calls. Tests assert on return
//! values and error messages; no file I/O happens inside the pure-core
//! functions.

use std::path::PathBuf;
use std::process::Command;

use overdrive_control_plane::openapi as openapi_lib;

// -----------------------------------------------------------------------------
// (a) Determinism — repeat generation produces byte-identical YAML.
// -----------------------------------------------------------------------------

#[test]
fn openapi_gen_writes_deterministic_yaml_in_repeat_runs() {
    let first = openapi_lib::generate_yaml().expect("generate_yaml must succeed");
    let second = openapi_lib::generate_yaml().expect("generate_yaml must succeed");
    assert_eq!(
        first, second,
        "openapi-gen output must be byte-identical across invocations \
         (utoipa 5.x sorts paths/schemas; any non-determinism is a bug)"
    );
    assert!(!first.is_empty(), "generated YAML must not be empty");
}

// -----------------------------------------------------------------------------
// (b) All ADR-0008 paths are present in the generated YAML.
// -----------------------------------------------------------------------------

#[test]
fn openapi_gen_output_contains_every_adr_0008_path() {
    let yaml = openapi_lib::generate_yaml().expect("generate_yaml must succeed");
    for expected in ["/v1/jobs", "/v1/jobs/{id}", "/v1/allocs", "/v1/nodes", "/v1/cluster/info"] {
        assert!(
            yaml.contains(expected),
            "generated YAML must include path {expected}; got:\n{yaml}"
        );
    }
}

// -----------------------------------------------------------------------------
// (c) All API DTO schemas from §02-03 are present.
// -----------------------------------------------------------------------------

#[test]
fn openapi_gen_output_contains_every_api_type_schema() {
    let yaml = openapi_lib::generate_yaml().expect("generate_yaml must succeed");
    for expected in [
        "SubmitWorkloadRequest",
        "SubmitWorkloadResponse",
        "WorkloadDescription",
        "ClusterStatus",
        "BrokerCountersBody",
        "AllocStatusResponse",
        "AllocStatusRowBody",
        "NodeList",
        "NodeRowBody",
        "ErrorBody",
    ] {
        assert!(
            yaml.contains(expected),
            "generated YAML must include schema {expected}; got:\n{yaml}"
        );
    }
}

// -----------------------------------------------------------------------------
// (d) `openapi-check` succeeds when the on-disk YAML matches the live schema.
// -----------------------------------------------------------------------------

#[test]
fn openapi_check_exits_0_when_yaml_matches_disk() {
    let yaml = openapi_lib::generate_yaml().expect("generate_yaml must succeed");
    let tmp = tempfile::NamedTempFile::new().expect("tempfile creation must succeed");
    std::fs::write(tmp.path(), &yaml).expect("write tempfile must succeed");
    openapi_lib::check_against_disk(tmp.path())
        .expect("check_against_disk must succeed on byte-identical input");
}

// -----------------------------------------------------------------------------
// (e) `openapi-check` surfaces drift with an actionable message naming the
//     first drifted schema + the regenerate hint.
// -----------------------------------------------------------------------------

#[test]
fn openapi_check_exits_non_zero_with_drift_message_when_yaml_is_stale() {
    let yaml = openapi_lib::generate_yaml().expect("generate_yaml must succeed");
    // Mutate the on-disk YAML: remove one schema name everywhere. The
    // `openapi-check` layer must surface the drift with an actionable
    // message identifying the schema + the regenerate suggestion.
    let drifted = yaml.replace("AllocStatusRowBody", "AllocStatusRowBodyWRONG");
    assert_ne!(drifted, yaml, "mutation must actually change the YAML");

    let tmp = tempfile::NamedTempFile::new().expect("tempfile creation must succeed");
    std::fs::write(tmp.path(), &drifted).expect("write tempfile must succeed");

    let err = openapi_lib::check_against_disk(tmp.path())
        .expect_err("check_against_disk must fail on drift");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("AllocStatusRowBody"),
        "drift message must name the drifted schema; got: {msg}"
    );
    assert!(
        msg.contains("cargo openapi-gen"),
        "drift message must suggest regenerating via `cargo openapi-gen`; got: {msg}"
    );
}

// -----------------------------------------------------------------------------
// (f) Subprocess smoke — running the `openapi check` binary against the
//     real workspace exits 0. This confirms the checked-in
//     `api/openapi.yaml` is up to date and the full binary wiring works.
// -----------------------------------------------------------------------------

/// Absolute path to the compiled `openapi` binary for this test run.
fn openapi_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_openapi"))
}

/// The workspace root — `crates/overdrive-control-plane/` lives two
/// levels below it.
fn workspace_root() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir
        .parent()
        .expect("overdrive-control-plane crate dir must have a parent")
        .parent()
        .expect("crates/ must have a parent (the workspace root)")
        .to_path_buf()
}

#[test]
fn openapi_check_subprocess_exits_0_against_checked_in_yaml() {
    let output = Command::new(openapi_bin())
        .arg("check")
        .current_dir(workspace_root())
        .output()
        .expect("openapi binary must be invokable");

    assert!(
        output.status.success(),
        "`cargo openapi-check` must exit 0 against the checked-in \
         api/openapi.yaml (run `cargo openapi-gen` to regenerate).\n\
         stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
