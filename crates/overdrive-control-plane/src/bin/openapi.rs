#![allow(clippy::print_stderr)]
//! `cargo openapi-gen` / `cargo openapi-check` binary — `OpenAPI` schema
//! CI gate per ADR-0009.
//!
//! Thin CLI plumbing over
//! [`overdrive_control_plane::openapi::{generate_yaml, check_against_disk}`].
//! User-facing invocations resolve through cargo aliases in
//! `.cargo/config.toml`:
//!
//! - `cargo openapi-gen` writes the live schema to `api/openapi.yaml`.
//! - `cargo openapi-check` fails non-zero if `api/openapi.yaml` does not
//!   match the live schema, naming the first drifted schema or path.
//!
//! Direct invocation: `cargo run -p overdrive-control-plane --bin openapi
//! -- {gen,check}`.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use color_eyre::eyre::{Result, WrapErr};

use overdrive_control_plane::openapi::{OPENAPI_YAML_PATH, check_against_disk, generate_yaml};

#[derive(Parser, Debug)]
#[command(about = "Overdrive OpenAPI schema CI gate", version)]
enum Cmd {
    /// Regenerate `api/openapi.yaml` at the workspace root.
    Gen,
    /// Verify `api/openapi.yaml` matches the live schema.
    Check,
}

fn main() -> ExitCode {
    if let Err(err) = color_eyre::install() {
        eprintln!("failed to install color-eyre: {err}");
        return ExitCode::FAILURE;
    }
    match run(&Cmd::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("openapi failed: {err:?}");
            ExitCode::FAILURE
        }
    }
}

fn run(cmd: &Cmd) -> Result<()> {
    // mutants: skip — binary entry point tested via subprocess in openapi_gate.rs
    let workspace_root = workspace_root()?;
    let path = workspace_root.join(OPENAPI_YAML_PATH);
    match cmd {
        Cmd::Gen => {
            let yaml = generate_yaml().wrap_err("render OverdriveApi::openapi() to YAML")?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .wrap_err_with(|| format!("create parent directory {}", parent.display()))?;
            }
            std::fs::write(&path, yaml)
                .wrap_err_with(|| format!("write OpenAPI YAML to {}", path.display()))?;
            eprintln!("openapi: wrote {}", path.display());
        }
        Cmd::Check => check_against_disk(&path)?,
    }
    Ok(())
}

/// Resolve the workspace root from the current working directory. The
/// binary is invoked from the workspace root (both via `cargo
/// openapi-{gen,check}` and via the subprocess smoke test); the helper
/// exists so downstream error messages name the discovered root rather
/// than the opaque `.` form of the path.
fn workspace_root() -> Result<PathBuf> {
    std::env::current_dir().wrap_err("determine workspace root from current_dir")
}
