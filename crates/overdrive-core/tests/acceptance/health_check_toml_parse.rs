//! Tier 1 acceptance — `[[health_check.*]]` TOML parse rules per
//! ADR-0057.
//!
//! Slices 02 / 03 / 07. RED scaffolds.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::struct_field_names,
    reason = "DISTILL RED scaffold; per `.claude/rules/testing.md` § 'RED scaffolds' lints land when DELIVER replaces todo!() bodies + rewrites docs"
)]

/// S-SHCP-PARSE-01 (US-02 / K1 / ADR-0057 §1) — `[[health_check.
/// startup]] type = "http", path = "/healthz", port = 8080` parses
/// to a `ProbeDescriptor` with `mechanic: ProbeMechanic::Http
/// { path: "/healthz", port: 8080, host: None }` and defaults
/// applied (timeout 5s, interval 2s, max_attempts 30).
#[test]
#[should_panic(expected = "RED scaffold")]
fn health_check_startup_http_parses_with_defaults() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-PARSE-01 / [[health_check.startup]] HTTP parses with defaults)"
    );
}

/// S-SHCP-PARSE-02 (US-02 / K5 / ADR-0057 §3) — `[[health_check.
/// startup]] type = "http"` WITHOUT `path` field yields
/// `ParseError::HttpProbeMissingPath { probe_idx: 0 }`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn health_check_http_missing_path_yields_named_parse_error() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-PARSE-02 / HTTP probe missing path → ParseError::HttpProbeMissingPath)"
    );
}

/// S-SHCP-PARSE-03 (US-03 / K1 / ADR-0057 §1) — `[[health_check.
/// startup]] type = "exec", command = ["/bin/healthcheck.sh"]`
/// parses to `ProbeMechanic::Exec { command: vec![...] }`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn health_check_startup_exec_parses_with_defaults() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-PARSE-03 / [[health_check.startup]] Exec parses with defaults)"
    );
}

/// S-SHCP-PARSE-04 (US-03 / K5 / ADR-0057 §3) — `type = "exec",
/// command = []` (empty array) yields `ParseError::ExecProbeMissing
/// Command { probe_idx: 0 }`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn health_check_exec_empty_command_yields_named_parse_error() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-PARSE-04 / Exec probe empty command → ParseError::ExecProbeMissingCommand)"
    );
}

/// S-SHCP-PARSE-05 (US-07 / K5 / ADR-0057 §3) — `[job]` block with
/// `[[health_check.startup]]` section yields
/// `ParseError::ProbesNotAllowedOnKind { kind: "job", guidance:
/// "Job has no readiness question; on completion is enough." }`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn probes_under_job_kind_yields_named_parse_error_with_guidance() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-PARSE-05 / [job] + [[health_check.startup]] → ParseError::ProbesNotAllowedOnKind ( kind: \"job\" ))"
    );
}

/// S-SHCP-PARSE-06 (US-07 / K5 / ADR-0057 §3) — `[schedule]` block
/// with any `[[health_check.*]]` yields `ParseError::ProbesNotAllowedOnKind
/// { kind: "schedule", guidance: "Schedule composes per-fire ..." }`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn probes_under_schedule_kind_yields_named_parse_error_with_guidance() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-PARSE-06 / [schedule] + [[health_check.*]] → ParseError::ProbesNotAllowedOnKind ( kind: \"schedule\" ))"
    );
}

/// S-SHCP-PARSE-07 (US-07 regression guard) — `[service]` block
/// with `[[health_check.startup]]` parses without error.
#[test]
#[should_panic(expected = "RED scaffold")]
fn probes_under_service_kind_parses_successfully() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-PARSE-07 / [service] + [[health_check.*]] parses successfully)"
    );
}

/// S-SHCP-INFER-01 (US-01 WS / DDD-15 / ADR-0058) — Service spec
/// with at least one `[[listener]]` AND zero `[[health_check.*]]`
/// sections synthesises a single startup probe with `mechanic:
/// ProbeMechanic::Tcp { host: "0.0.0.0", port: listeners[0].port }`
/// and `inferred: true`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn service_without_probes_with_listener_infers_default_tcp_startup_probe() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INFER-01 / no-probe Service infers TCP startup against listener[0])"
    );
}

/// S-SHCP-INFER-02 (DDD-16 / ADR-0058) — `[[health_check.startup]]
/// = []` (explicit empty array) is the explicit opt-out: preserves
/// Phase-1 first-Running semantics; no probe is inferred.
#[test]
#[should_panic(expected = "RED scaffold")]
fn service_with_empty_startup_array_opts_out_of_default_inference() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INFER-02 / [[health_check.startup]] = [] opts out of default inference)"
    );
}

/// S-SHCP-PARSE-08 (US-02 AC / research Pitfall 5) — parse rejects
/// `https://` scheme in HTTP probe (Phase 1 plain HTTP only per
/// C6); yields `ParseError::HttpsProbeNotSupported { probe_idx }`
/// or equivalent named error.
#[test]
#[should_panic(expected = "RED scaffold")]
fn health_check_http_https_scheme_yields_named_parse_error() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-PARSE-08 / https:// scheme rejected at parse time)"
    );
}
