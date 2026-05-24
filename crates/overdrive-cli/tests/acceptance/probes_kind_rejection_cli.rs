//! Tier 1 acceptance — CLI surface for `ParseError::
//! ProbesNotAllowedOnKind` per US-07 / K5.
//!
//! Slice 07 — RED scaffolds.
//!
//! Per `crates/overdrive-cli/CLAUDE.md`: tests call command
//! handlers directly with injected adapters (NOT subprocess).
//! Parser-side rejection lives in `overdrive-core::aggregate`;
//! CLI-side surface is the error rendering + exit code 1.

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

/// S-SHCP-CLI-12 (US-07 / K5) — Job + `[[health_check.startup]]` TOML
/// submitted via `job::submit` handler returns
/// `CliError::ParseError(ParseError::ProbesNotAllowedOnKind {
/// kind: "job", guidance: "Job has no readiness question; ..." })`,
/// exit code 1, error text contains the named guidance.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_job_with_probe_section_when_submit_then_named_error_with_job_guidance_exit_one() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-CLI-12 / [job] + probe → ProbesNotAllowedOnKind ( kind: \"job\" ) + exit 1)"
    );
}

/// S-SHCP-CLI-13 (US-07 / K5) — Schedule + `[[health_check.*]]` TOML
/// submitted via `job::submit` handler returns
/// `ProbesNotAllowedOnKind { kind: "schedule" }` with the
/// schedule-specific guidance.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_schedule_with_probe_section_when_submit_then_named_error_with_schedule_guidance() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-CLI-13 / [schedule] + probe → ProbesNotAllowedOnKind ( kind: \"schedule\" ) + exit 1)"
    );
}

/// S-SHCP-CLI-14 (US-07 / K5 regression guard) — Service +
/// `[[health_check.startup]]` is accepted; no parse error.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_service_with_probe_section_when_submit_then_accepted_no_parse_error() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-CLI-14 / [service] + probe accepted — regression guard)"
    );
}
