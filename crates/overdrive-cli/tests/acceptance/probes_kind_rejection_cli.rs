//! Tier 1 acceptance — CLI surface for `ParseError::
//! ProbesNotAllowedOnKind` per US-07 / K5.
//!
//! Slice 07 — step 03-01 GREEN landing.
//!
//! Per `crates/overdrive-cli/CLAUDE.md`: tests call command handlers
//! directly with injected adapters (NOT subprocess). The kind-
//! rejection fires inside `job::submit` at spec-parse time, BEFORE any
//! HTTP call (`ApiClient::from_config` is step 4; parse is step 2), so
//! these tests need no running server — the `config_path` is never
//! reached on the rejection path.
//!
//! Universe (observable surface of the driving port): the `Result`
//! returned by `job::submit` — its `CliError` variant, the wrapped
//! `ParseError`'s `kind` / `guidance` fields, the rendered multi-line
//! error string, and the `cli_error_to_exit_code` mapping. None of the
//! assertions reach into handler internals.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc
)]

use std::path::PathBuf;

use overdrive_cli::commands::job::{self, SubmitArgs};
use overdrive_cli::http_client::CliError;
use overdrive_cli::render::cli_error_to_exit_code;
use overdrive_core::aggregate::{
    JOB_PROBES_GUIDANCE, ParseError, SCHEDULE_PROBES_GUIDANCE, WorkloadSpecInput,
};
use tempfile::TempDir;

/// Write `body` to a `*.toml` fixture in a fresh tempdir and return the
/// path plus the `TempDir` guard (kept alive by the caller).
fn write_fixture(body: &str) -> (PathBuf, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("spec.toml");
    std::fs::write(&path, body).expect("write fixture");
    (path, tmp)
}

const JOB_WITH_STARTUP_PROBE: &str = r#"
[job]
id = "batch"

[exec]
command = "/usr/bin/server"
args = []

[resources]
cpu_milli = 100
memory_bytes = 134217728

[[health_check.startup]]
type = "tcp"
port = 8080
"#;

const SCHEDULE_WITH_READINESS_PROBE: &str = r#"
[job]
id = "nightly"

[schedule]
cron = "0 0 * * *"

[exec]
command = "/usr/bin/server"
args = []

[resources]
cpu_milli = 100
memory_bytes = 134217728

[[health_check.readiness]]
type = "tcp"
port = 8080
"#;

const SERVICE_WITH_READINESS_PROBE: &str = r#"
[service]
id = "svc"
replicas = 1

[[listener]]
port = 8080
protocol = "tcp"

[exec]
command = "/usr/bin/server"
args = []

[resources]
cpu_milli = 100
memory_bytes = 134217728

[[health_check.readiness]]
type = "tcp"
port = 8080
"#;

/// S-SHCP-CLI-12 (US-07 / K5) — Job + `[[health_check.startup]]`
/// submitted via the `job::submit` driving port returns
/// `CliError::ParseError(ProbesNotAllowedOnKind { kind: "job",
/// guidance: <job guidance> })`, the rendered error contains the job
/// guidance text, and the exit-code mapping is 1.
///
/// bypass: single-example — the rejection is a single observable
/// outcome (one kind, one guidance constant); the PARSE-05 proptest at
/// `overdrive-core` already quantifies it over every probe role × port.
/// This test verifies the CLI surface (variant + exit code) on one
/// representative fixture.
#[tokio::test]
async fn given_job_with_probe_section_when_submit_then_named_error_with_job_guidance_exit_one() {
    let (spec, _tmp) = write_fixture(JOB_WITH_STARTUP_PROBE);
    let args = SubmitArgs { spec, config_path: PathBuf::from("/nonexistent/config") };

    let err = job::submit(args).await.expect_err("Job + probe must be rejected");
    match &err {
        CliError::ParseError(ParseError::ProbesNotAllowedOnKind { kind, guidance }) => {
            assert_eq!(*kind, "job");
            assert_eq!(*guidance, JOB_PROBES_GUIDANCE);
        }
        other => panic!("expected ParseError(ProbesNotAllowedOnKind(job)), got {other:?}"),
    }
    assert!(
        err.to_string().contains(JOB_PROBES_GUIDANCE),
        "rendered error must surface the job guidance text: {err}"
    );
    assert_eq!(cli_error_to_exit_code(&err), 1, "spec rejection exits 1");
}

/// S-SHCP-CLI-13 (US-07 / K5) — Schedule + `[[health_check.readiness]]`
/// surfaces `ProbesNotAllowedOnKind { kind: "schedule" }` with the
/// schedule guidance, exit 1.
///
/// bypass: single-example — see CLI-12; PARSE-06 quantifies the parser
/// side.
#[tokio::test]
async fn given_schedule_with_probe_section_when_submit_then_named_error_with_schedule_guidance() {
    let (spec, _tmp) = write_fixture(SCHEDULE_WITH_READINESS_PROBE);
    let args = SubmitArgs { spec, config_path: PathBuf::from("/nonexistent/config") };

    let err = job::submit(args).await.expect_err("Schedule + probe must be rejected");
    match &err {
        CliError::ParseError(ParseError::ProbesNotAllowedOnKind { kind, guidance }) => {
            assert_eq!(*kind, "schedule");
            assert_eq!(*guidance, SCHEDULE_PROBES_GUIDANCE);
        }
        other => panic!("expected ParseError(ProbesNotAllowedOnKind(schedule)), got {other:?}"),
    }
    assert!(
        err.to_string().contains(SCHEDULE_PROBES_GUIDANCE),
        "rendered error must surface the schedule guidance text: {err}"
    );
    assert_eq!(cli_error_to_exit_code(&err), 1, "spec rejection exits 1");
}

/// S-SHCP-CLI-14 (US-07 / K5 regression guard) — Service +
/// `[[health_check.readiness]]` is ACCEPTED: the kind-discriminating
/// parser does NOT raise `ProbesNotAllowedOnKind`. Asserted at the
/// parse driving port (`WorkloadSpecInput::from_toml_str`) — a full
/// `job::submit` would fall through to the legacy flat-`JobSpecInput`
/// path and fail on the unrelated absence of flat job fields, which
/// would conflate "kind rejection" with "wrong legacy shape". The
/// regression this guards is precisely "Service probes must NOT trip
/// the kind-rejection added in this step".
///
/// bypass: single-example — the property "Service + probe parses" is a
/// negative-of-rejection guard; PARSE-07 quantifies the positive
/// (readiness probe survives) over arbitrary ports.
#[tokio::test]
async fn given_service_with_probe_section_when_submit_then_accepted_no_parse_error() {
    let parsed = WorkloadSpecInput::from_toml_str(SERVICE_WITH_READINESS_PROBE);
    match parsed {
        Ok(WorkloadSpecInput::Service(spec)) => {
            assert_eq!(
                spec.readiness_probes.len(),
                1,
                "the Service readiness probe must survive the parse"
            );
        }
        Err(ParseError::ProbesNotAllowedOnKind { .. }) => {
            panic!("Service + probe must NOT trip the kind-rejection (regression guard)");
        }
        other => panic!("expected Service kind to parse, got {other:?}"),
    }
}
