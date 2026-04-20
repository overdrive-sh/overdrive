//! `overdrive` — the command-line client.
//!
//! This binary is a thin boundary layer: it parses args, configures logging
//! and error reporting, and hands off to library crates. Error handling
//! uses `eyre` + `color-eyre` here because this is a binary boundary; the
//! libraries below return typed `thiserror` enums.

#![allow(clippy::expect_used)] // `expect` is the correct shape at bin boundaries.

use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;
use tracing_subscriber::EnvFilter;

/// Overdrive — a next-generation workload orchestration platform.
#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Control-plane endpoint (defaults to `OVERDRIVE_ENDPOINT` env var).
    #[arg(long, env = "OVERDRIVE_ENDPOINT", default_value = "http://127.0.0.1:7001")]
    endpoint: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Job lifecycle — submit, list, stop.
    #[command(subcommand)]
    Job(JobCommand),

    /// Node inspection.
    #[command(subcommand)]
    Node(NodeCommand),

    /// Allocation inspection.
    #[command(subcommand)]
    Alloc(AllocCommand),

    /// Cluster bootstrap and membership.
    #[command(subcommand)]
    Cluster(ClusterCommand),
}

#[derive(Debug, Subcommand)]
enum JobCommand {
    Submit {
        #[arg(long)]
        spec: std::path::PathBuf,
    },
    List,
    Stop {
        id: String,
    },
}

#[derive(Debug, Subcommand)]
enum NodeCommand {
    List,
}

#[derive(Debug, Subcommand)]
enum AllocCommand {
    Status { id: String },
}

#[derive(Debug, Subcommand)]
enum ClusterCommand {
    Upgrade {
        #[arg(long, value_parser = ["single", "ha"])]
        mode: String,
        #[arg(long, value_delimiter = ',')]
        peers: Vec<String>,
    },
}

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
