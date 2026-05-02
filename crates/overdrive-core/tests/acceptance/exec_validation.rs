//! Acceptance scenarios for `wire-exec-spec-end-to-end` — the new
//! `Job::from_spec` validation rule on `exec.command` (per ADR-0031 §4)
//! and the schema-level rejections driven by serde
//! (`deny_unknown_fields` + tagged-enum dispatch on `DriverInput`).
//!
//! Covers `docs/feature/wire-exec-spec-end-to-end/distill/test-scenarios.md`
//! §2 *Aggregate validation rules* and §3 *Schema-level rejections*.
//!
//! Per `.claude/rules/development.md` § Errors and the existing
//! `aggregate_validation.rs` pattern: tests assert on the structured
//! `AggregateError::Validation { field, message }` variant — NOT a
//! stringified `Display` form. The HTTP layer (ADR-0015) consumes the
//! variant shape via `#[from]` pass-through; the variant is the
//! contract. The one literal-message assertion (`"command must be
//! non-empty"`) is justified because Ana reads it on the operator
//! surface (per DWD-6 in `wave-decisions.md`).

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use overdrive_core::aggregate::{
    AggregateError, DriverInput, ExecInput, Job, JobSpecInput, ResourcesInput,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Canonical valid `JobSpecInput` carrying a non-empty exec block.
/// Used as the base for "tweak one field, expect rejection" tests.
fn canonical_spec() -> JobSpecInput {
    JobSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 134_217_728 },
        driver: DriverInput::Exec(ExecInput {
            command: "/opt/payments/bin/server".to_string(),
            args: vec!["--port".to_string(), "8080".to_string()],
        }),
    }
}

/// Helper — produce a spec whose `exec.command` is `raw`, leaving every
/// other field at its canonical-valid value.
fn spec_with_command(raw: &str) -> JobSpecInput {
    let mut spec = canonical_spec();
    spec.driver = DriverInput::Exec(ExecInput {
        command: raw.to_string(),
        args: vec!["--port".to_string(), "8080".to_string()],
    });
    spec
}

// ---------------------------------------------------------------------------
// §2 — Validation rules at `Job::from_spec`
// ---------------------------------------------------------------------------

#[test]
fn job_from_spec_rejects_empty_exec_command_with_structured_field_name() {
    // Given a JobSpecInput whose driver is Exec with command = "".
    let spec = spec_with_command("");

    // When Ana calls the validating constructor.
    let err = Job::from_spec(spec).expect_err("empty exec.command must be rejected");

    // Then the error is the Validation variant naming the exec.command field.
    match err {
        AggregateError::Validation { field, message } => {
            assert_eq!(field, "exec.command", "field must name `exec.command`; got {field:?}");
            // Per DWD-6 the message is part of the operator-facing
            // contract; the literal is pinned.
            assert_eq!(
                message, "command must be non-empty",
                "message must be the canonical operator-facing string per ADR-0031 §4; got {message:?}",
            );
        }
        other => panic!("expected AggregateError::Validation, got {other:?}"),
    }
}

#[test]
fn job_from_spec_rejects_whitespace_only_exec_command_via_trim_rule() {
    // Given a JobSpecInput whose `exec.command` is "   " (three spaces).
    // The trim-then-is_empty predicate (ADR-0031 §4) rejects this where
    // a bare `is_empty()` would not — pinning the trim rule.
    let spec = spec_with_command("   ");

    let err = Job::from_spec(spec).expect_err("whitespace-only command must be rejected");

    match err {
        AggregateError::Validation { field, .. } => {
            assert_eq!(field, "exec.command", "field must name `exec.command`; got {field:?}");
        }
        other => panic!("expected AggregateError::Validation, got {other:?}"),
    }
}

#[test]
fn job_from_spec_rejects_mixed_whitespace_exec_command() {
    // Given an exec.command whose entirety is mixed Unicode whitespace
    // (tab, newline, space). The `str::trim` predicate covers Unicode
    // whitespace, not only ASCII space.
    let spec = spec_with_command("\t\n ");

    let err = Job::from_spec(spec).expect_err("mixed-whitespace command must be rejected");

    match err {
        AggregateError::Validation { field, .. } => {
            assert_eq!(field, "exec.command");
        }
        other => panic!("expected AggregateError::Validation, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// §3 — Schema-level rejections (driven by serde, not the constructor)
// ---------------------------------------------------------------------------

#[test]
fn toml_missing_exec_table_fails_to_parse_with_serde_error() {
    // TOML lacking the `[exec]` table. The tagged-enum `DriverInput`
    // requires exactly one driver variant (today: only `Exec`); an
    // absent driver table is a parse error, not a "default Exec".
    let body = r#"
id = "payments"
replicas = 1

[resources]
cpu_milli = 500
memory_bytes = 134217728
"#;

    let result: Result<JobSpecInput, _> = toml::from_str(body);
    let err = result.expect_err("TOML lacking `[exec]` must fail to parse");
    let msg = err.to_string();
    // The diagnostic must reference the `exec` field / variant — the
    // exact wording is serde-version-sensitive but the substring is
    // stable enough for an assertion.
    assert!(
        msg.contains("exec") || msg.contains("missing field") || msg.contains("variant"),
        "serde error must reference the missing exec driver; got: {msg}",
    );
}

#[test]
fn toml_missing_resources_table_fails_to_parse_with_serde_error() {
    let body = r#"
id = "payments"
replicas = 1

[exec]
command = "/opt/payments/bin/server"
args = ["--port", "8080"]
"#;

    let result: Result<JobSpecInput, _> = toml::from_str(body);
    let err = result.expect_err("TOML lacking `[resources]` must fail to parse");
    let msg = err.to_string();
    assert!(
        msg.contains("resources") || msg.contains("missing field"),
        "serde error must reference the missing resources field; got: {msg}",
    );
}

#[test]
fn toml_with_unknown_top_level_driver_table_fails_to_parse_via_deny_unknown_fields() {
    // TOML containing `[exec]` plus an unknown sibling table at top
    // level. Today only one variant exists in `DriverInput`
    // (`Exec(...)`); a sibling unknown table is the analogue of "two
    // driver tables" until a second variant lands. `deny_unknown_fields`
    // on `JobSpecInput` rejects.
    let body = r#"
id = "payments"
replicas = 1

[resources]
cpu_milli = 500
memory_bytes = 134217728

[exec]
command = "/opt/payments/bin/server"
args = ["--port", "8080"]

[bogus]
some_field = "wat"
"#;

    let result: Result<JobSpecInput, _> = toml::from_str(body);
    let err = result.expect_err("TOML with unknown top-level table must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("bogus") || msg.contains("unknown"),
        "serde error must name the unknown table; got: {msg}",
    );
}

#[test]
fn toml_with_typo_in_exec_field_fails_via_deny_unknown_fields() {
    // `commando` (typo for `command`) inside `[exec]`. The
    // `deny_unknown_fields` directive on `ExecInput` rejects unknown
    // fields — operators see a serde diagnostic, not a silently-
    // ignored typo.
    let body = r#"
id = "payments"
replicas = 1

[resources]
cpu_milli = 500
memory_bytes = 134217728

[exec]
commando = "/opt/payments/bin/server"
args = []
"#;

    let result: Result<JobSpecInput, _> = toml::from_str(body);
    let err = result.expect_err("typo in exec field must fail to parse");
    let msg = err.to_string();
    assert!(
        msg.contains("commando") || msg.contains("unknown"),
        "serde error must name the unknown field `commando`; got: {msg}",
    );
}

// ---------------------------------------------------------------------------
// §2 — Property: empty / whitespace command always yields exec.command Validation
// ---------------------------------------------------------------------------

mod property {
    use super::*;
    use proptest::prelude::*;

    /// Generator — strings whose `trim()` is empty. Includes `""`, pure
    /// ASCII whitespace, and pure Unicode whitespace (tab, newline,
    /// vertical tab, form feed).
    fn empty_or_whitespace_string() -> impl Strategy<Value = String> {
        prop_oneof![
            Just(String::new()),
            // ASCII whitespace runs of length 1..=10
            prop::collection::vec(proptest::sample::select(vec![' ', '\t', '\n', '\r']), 1..=10)
                .prop_map(|chars| chars.into_iter().collect::<String>()),
        ]
    }

    proptest! {
        /// For any `exec.command` that is empty or pure whitespace, with
        /// every other field otherwise valid, `from_spec` must always
        /// return `Validation { field: "exec.command", .. }`. Closes the
        /// mutation gap on the trim guard per `.claude/rules/testing.md`
        /// mutation target "Newtype FromStr and validators".
        #[test]
        fn empty_or_whitespace_command_always_yields_exec_command_validation(
            command in empty_or_whitespace_string(),
            cpu in 0u32..10_000,
            mem in 1u64..=u64::MAX,
        ) {
            let spec = JobSpecInput {
                id: "payments".to_string(),
                replicas: 1,
                resources: ResourcesInput { cpu_milli: cpu, memory_bytes: mem },
                driver: DriverInput::Exec(ExecInput {
                    command,
                    args: vec![],
                }),
            };
            match Job::from_spec(spec) {
                Err(AggregateError::Validation { field, .. }) => {
                    prop_assert_eq!(field, "exec.command");
                }
                other => prop_assert!(
                    false,
                    "expected Validation{{field: \"exec.command\"}}, got {:?}",
                    other,
                ),
            }
        }
    }
}
