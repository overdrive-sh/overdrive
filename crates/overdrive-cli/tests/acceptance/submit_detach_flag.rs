//! S-CLI-01 — `overdrive deploy --detach` argv surface.
//!
//! Per `docs/feature/cli-submit-vs-deploy-and-alloc-status/deliver/03-01`
//! step 03-01 acceptance criteria:
//!
//! > Add `--detach` boolean to the `Deploy` clap struct in
//! > `crates/overdrive-cli/src/cli.rs`. When set, CLI sends
//! > `Accept: application/json` and consumes the JSON ack only (today's
//! > pre-feature shape) — does NOT engage the NDJSON consumer regardless
//! > of stdout being a TTY. Exit code 0 on Inserted/Unchanged outcome;
//! > exit code 2 on transport / server-validation error per ADR-0015 (no
//! > NDJSON terminal-mapping path active).
//!
//! Slice 03 step 03-01 lands the explicit operator escape valve. The
//! reference class is `docker run -d`, `nomad job run --detach`. The
//! companion auto-detect (`std::io::IsTerminal`) is step 03-02; this
//! step is the explicit-flag side only.
//!
//! Per `crates/overdrive-cli/CLAUDE.md` § *Integration tests — no
//! subprocess*, this file exercises the argv surface in-process via
//! `Cli::try_parse_from(...)`. The full wire-level assertion (Accept
//! header on the reqwest client, exit code 0 on `Inserted`, KPI-05
//! wall-clock ≤ 200ms p95) is exercised by the integration suite that
//! already covers the JSON-ack path
//! (`tests/integration/deploy.rs::submit_with_valid_toml_against_in_process_server_returns_submit_output_with_intent_key_and_next_command`)
//! — the `--detach` flag dispatches to that same path through main.rs's
//! match arm, so the existing JSON-path coverage is the wire-level
//! witness.

use clap::Parser as _;
use overdrive_cli::cli::{Cli, Command};

// ---------------------------------------------------------------------
// (A) `--detach` is recognised by clap and lands as `true`
// ---------------------------------------------------------------------

#[test]
fn deploy_with_detach_flag_parses_to_detach_true() {
    let cli = Cli::try_parse_from(["overdrive", "deploy", "--detach", "payments.toml"])
        .expect("`--detach` must be a recognised flag on `deploy`");

    match cli.command {
        Command::Deploy { spec, detach } => {
            assert_eq!(
                spec.to_string_lossy(),
                "payments.toml",
                "spec positional must round-trip unchanged",
            );
            assert!(
                detach,
                "S-CLI-01: `--detach` on the argv must produce `detach = true` on the parsed Deploy struct",
            );
        }
        other => panic!("expected Command::Deploy, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// (B) Without `--detach`, the parsed struct carries `detach = false`
// ---------------------------------------------------------------------

#[test]
fn deploy_without_detach_flag_parses_to_detach_false() {
    let cli = Cli::try_parse_from(["overdrive", "deploy", "payments.toml"])
        .expect("`deploy <spec>` without --detach is the streaming default and must parse");

    match cli.command {
        Command::Deploy { spec, detach } => {
            assert_eq!(spec.to_string_lossy(), "payments.toml");
            assert!(
                !detach,
                "S-CLI-01: absent `--detach` must produce `detach = false` (streaming-default lane)",
            );
        }
        other => panic!("expected Command::Deploy, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// (C) `--detach` accepts no value — it is a boolean flag, not a string
// ---------------------------------------------------------------------

#[test]
fn detach_flag_does_not_accept_a_value() {
    let result =
        Cli::try_parse_from(["overdrive", "deploy", "--detach=somevalue", "payments.toml"]);
    assert!(
        result.is_err(),
        "S-CLI-01: `--detach` must be a boolean flag with no attached value; \
         a future refactor that adds `value_parser` would break the contract",
    );
}

// ---------------------------------------------------------------------
// (D) `--detach` is documented in the help output for `deploy`
// ---------------------------------------------------------------------

#[test]
fn deploy_help_output_documents_detach_flag() {
    let err = Cli::try_parse_from(["overdrive", "deploy", "--help"])
        .expect_err("`--help` exits with a non-zero clap kind carrying the help text");

    let rendered = err.render().to_string();

    assert!(
        rendered.contains("--detach"),
        "S-CLI-01: `overdrive deploy --help` must document the `--detach` flag; got:\n{rendered}",
    );
}
