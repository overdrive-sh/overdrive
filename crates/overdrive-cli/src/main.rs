//! `overdrive` — the command-line client.
//!
//! This binary is a thin boundary layer: it parses args, configures logging
//! and error reporting, and hands off to library crates. Error handling
//! uses `eyre` + `color-eyre` here because this is a binary boundary; the
//! libraries below return typed `thiserror` enums.
//!
//! Per `crates/overdrive-cli/CLAUDE.md`, the argv surface lives in the
//! `overdrive_cli::cli` library module so integration tests can invoke
//! `Cli::try_parse_from(...)` in-process, without spawning `overdrive`
//! as a subprocess.

#![allow(clippy::expect_used)] // `expect` is the correct shape at bin boundaries.

use clap::Parser;
use color_eyre::eyre::Result;
use overdrive_cli::cli::Cli;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    color_eyre::install().expect("color-eyre installs once at startup");

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("building the Tokio runtime cannot fail here");

    runtime.block_on(run(cli))
}

#[allow(clippy::unused_async)] // Real subcommand handlers will `.await` a client RPC.
async fn run(cli: Cli) -> Result<()> {
    // Every subcommand will gain a real handler as the control-plane API
    // lands in Phase 1. Until then we acknowledge the command and exit 0
    // so `overdrive --help` and smoke-tests work end-to-end.
    let _ = cli.command;
    tracing::warn!(endpoint = %cli.endpoint, "command not yet wired to control plane");
    Ok(())
}
