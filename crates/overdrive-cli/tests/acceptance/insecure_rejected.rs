//! @clap-config — argv parsing exercised via `Cli::try_parse_from`,
//! no subprocess.
//!
//! Step 02-05 — two concerns:
//!
//! A) `--insecure` rejection (ADR-0010 §R4): clap must reject
//!    `overdrive --insecure ...` as an unknown argument. The test
//!    pins clap-tree shape so a future refactor cannot silently
//!    introduce `--insecure` without also touching this test.
//!
//! B) Malformed trust triple: `load_trust_triple` surfaces a
//!    structured `ControlPlaneError` naming the file and the field
//!    that failed to decode. No panic, no unwrap.
//!
//! Per `crates/overdrive-cli/CLAUDE.md`, the tests in this module
//! exercise the binary-wrapper argv surface in-process — they are
//! the Exception scope carved out for "argv parsing for the binary
//! wrapper itself", NOT a subprocess smoke test.

use std::fs;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use clap::Parser as _;
use clap::error::ErrorKind;
use overdrive_cli::cli::Cli;
use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::tls_bootstrap::{
    load_trust_triple, mint_ephemeral_ca, write_trust_triple,
};
use tempfile::TempDir;

// -----------------------------------------------------------------------
// Part A — `--insecure` rejection
// -----------------------------------------------------------------------

#[test]
fn insecure_flag_is_rejected_as_unknown_argument() {
    let result = Cli::try_parse_from(["overdrive", "--insecure", "cluster", "status"]);

    let err = result.expect_err(
        "ADR-0010 §R4 forbids `--insecure` — clap MUST reject it as an unknown argument",
    );

    assert_eq!(
        err.kind(),
        ErrorKind::UnknownArgument,
        "clap must classify `--insecure` as UnknownArgument, not InvalidValue or \
         DisplayHelp — got {:?}",
        err.kind(),
    );

    let rendered = err.render().to_string();
    assert!(
        rendered.contains("--insecure"),
        "clap error text must name the offending flag `--insecure`; got: {rendered}",
    );
}

#[test]
fn legitimate_top_level_commands_parse_successfully() {
    // Positive control: without --insecure, the argv parses cleanly.
    // If this fails, the UnknownArgument assertion above is not a
    // reliable signal (clap might be rejecting for an unrelated
    // reason).
    let parsed = Cli::try_parse_from(["overdrive", "cluster", "status"])
        .expect("positive control: `cluster status` is a real subcommand");

    // A smoke check — we just want to prove parse succeeded; we don't
    // care which variant of `command` clap produced here.
    let _ = parsed.command;
}

// -----------------------------------------------------------------------
// Part B — malformed trust triple
// -----------------------------------------------------------------------

fn write_config_with_malformed_field(dir: &std::path::Path, field: &str) -> std::path::PathBuf {
    let overdrive_dir = dir.join(".overdrive");
    fs::create_dir_all(&overdrive_dir).expect("mkdir .overdrive");
    let config_path = overdrive_dir.join("config");

    // Mint REAL material for the other two fields so only one field is
    // malformed — that isolates the assertion to the specific field.
    let material = mint_ephemeral_ca().expect("mint_ephemeral_ca");

    let mut ca = BASE64.encode(material.ca_cert_pem.as_bytes());
    let mut crt = BASE64.encode(material.client_leaf_cert_pem.as_bytes());
    let mut key = BASE64.encode(material.client_leaf_key_pem.as_bytes());

    // Corrupt only the named field with a string that is NOT valid
    // base64. `@` and `#` are outside the base64 alphabet. We quote
    // the YAML scalar so the garbage arrives at `BASE64.decode`
    // byte-for-byte — without quoting, a leading `!` would be parsed
    // as a YAML tag directive and the malformed-field test would
    // collide with YAML's tag resolver rather than exercise the
    // base64 decode path we care about.
    let garbage = "@@@not-valid-base64###".to_string();
    match field {
        "ca" => ca = garbage,
        "crt" => crt = garbage,
        "key" => key = garbage,
        other => panic!("test bug: unknown field {other}"),
    }

    let yaml = format!(
        "context: local\n\
         contexts:\n  \
           local:\n    \
             endpoint: https://127.0.0.1:7001\n    \
             ca: \"{ca}\"\n    \
             crt: \"{crt}\"\n    \
             key: \"{key}\"\n",
    );
    fs::write(&config_path, yaml).expect("write malformed config");
    config_path
}

#[test]
fn load_trust_triple_rejects_malformed_base64_ca_field_with_control_plane_error_internal() {
    let tmp = TempDir::new().expect("TempDir");
    let config_path = write_config_with_malformed_field(tmp.path(), "ca");

    let err = load_trust_triple(&config_path)
        .expect_err("malformed base64 in the `ca` field must surface a structured error, not Ok");

    match &err {
        ControlPlaneError::Internal(msg) => {
            assert!(
                msg.contains(&config_path.display().to_string()),
                "error message must name the config file path `{}` so operators \
                 can locate the bad file; got: {msg}",
                config_path.display(),
            );
            assert!(
                msg.contains("ca"),
                "error message must name the offending field `ca`; got: {msg}",
            );
        }
        other => {
            panic!("malformed trust triple must map to ControlPlaneError::Internal, got {other:?}",)
        }
    }
}

#[test]
fn load_trust_triple_rejects_malformed_base64_crt_field_with_control_plane_error_internal() {
    let tmp = TempDir::new().expect("TempDir");
    let config_path = write_config_with_malformed_field(tmp.path(), "crt");

    let err = load_trust_triple(&config_path)
        .expect_err("malformed `crt` base64 must surface a structured error");

    match &err {
        ControlPlaneError::Internal(msg) => {
            assert!(
                msg.contains(&config_path.display().to_string()),
                "error must name path; got: {msg}",
            );
            assert!(msg.contains("crt"), "error must name field `crt`; got: {msg}");
        }
        other => panic!("expected Internal variant, got {other:?}"),
    }
}

#[test]
fn load_trust_triple_rejects_malformed_base64_key_field_with_control_plane_error_internal() {
    let tmp = TempDir::new().expect("TempDir");
    let config_path = write_config_with_malformed_field(tmp.path(), "key");

    let err = load_trust_triple(&config_path)
        .expect_err("malformed `key` base64 must surface a structured error");

    match &err {
        ControlPlaneError::Internal(msg) => {
            assert!(
                msg.contains(&config_path.display().to_string()),
                "error must name path; got: {msg}",
            );
            assert!(msg.contains("key"), "error must name field `key`; got: {msg}");
        }
        other => panic!("expected Internal variant, got {other:?}"),
    }
}

#[test]
fn load_trust_triple_rejects_missing_file_with_control_plane_error_internal_naming_path() {
    let tmp = TempDir::new().expect("TempDir");
    let missing = tmp.path().join(".overdrive").join("config");

    let err = load_trust_triple(&missing)
        .expect_err("nonexistent config path must surface a structured error");

    match &err {
        ControlPlaneError::Internal(msg) => {
            assert!(
                msg.contains(&missing.display().to_string()),
                "error must name the missing path `{}`; got: {msg}",
                missing.display(),
            );
        }
        other => panic!("expected Internal variant for missing file, got {other:?}"),
    }
}

#[test]
fn load_trust_triple_on_well_formed_config_returns_ok_with_populated_fields() {
    // Positive control — proves load_trust_triple is not a
    // constant-error stub. When the file IS valid, it must succeed
    // and surface the decoded bytes + endpoint.
    let tmp = TempDir::new().expect("TempDir");
    let material = mint_ephemeral_ca().expect("mint_ephemeral_ca");
    write_trust_triple(tmp.path(), "https://127.0.0.1:7001", &material)
        .expect("write_trust_triple");

    let config_path = tmp.path().join(".overdrive").join("config");
    let triple =
        load_trust_triple(&config_path).expect("well-formed config must load successfully");

    assert_eq!(triple.endpoint(), "https://127.0.0.1:7001");
    assert_eq!(triple.ca_cert_pem(), material.ca_cert_pem.as_bytes());
    assert_eq!(triple.client_cert_pem(), material.client_leaf_cert_pem.as_bytes());
    assert_eq!(triple.client_key_pem(), material.client_leaf_key_pem.as_bytes());
}
