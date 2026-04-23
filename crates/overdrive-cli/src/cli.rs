//! clap-derive argv surface for the `overdrive` binary.
//!
//! Kept in the library crate so integration tests can exercise argv
//! parsing via `Cli::try_parse_from([...])` without a subprocess —
//! see `crates/overdrive-cli/CLAUDE.md` (Exception: argv parsing for
//! the binary wrapper itself).
//!
//! Per ADR-0010 §R4 there is NO `--insecure` flag — an operator
//! invoking `overdrive --insecure ...` must be rejected as an unknown
//! argument. Clap's default behaviour does this; the test in
//! `tests/acceptance/insecure_rejected.rs` pins that behaviour
//! against future refactors.

use clap::{Parser, Subcommand};

/// Overdrive — a next-generation workload orchestration platform.
#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Control-plane endpoint (defaults to `OVERDRIVE_ENDPOINT` env var).
    #[arg(long, env = "OVERDRIVE_ENDPOINT", default_value = "http://127.0.0.1:7001")]
    pub endpoint: String,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
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
pub enum JobCommand {
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
pub enum NodeCommand {
    List,
}

#[derive(Debug, Subcommand)]
pub enum AllocCommand {
    Status { id: String },
}

#[derive(Debug, Subcommand)]
pub enum ClusterCommand {
    Upgrade {
        #[arg(long, value_parser = ["single", "ha"])]
        mode: String,
        #[arg(long, value_delimiter = ',')]
        peers: Vec<String>,
    },
    Status,
}
