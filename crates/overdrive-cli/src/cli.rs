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

    /// Start the Phase 1 control-plane server on a TLS-bound listener.
    Serve {
        /// Socket address to bind (default `127.0.0.1:7001` per
        /// ADR-0008). Pass `127.0.0.1:0` to request an ephemeral port.
        #[arg(long, default_value = "127.0.0.1:7001")]
        bind: String,
        /// Data directory — parent of the redb file, per-primitive
        /// libSQL files, and the trust-triple config. Default:
        /// `dirs::data_dir()/overdrive` per ADR-0013 §5.
        #[arg(long)]
        data_dir: Option<std::path::PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub enum JobCommand {
    /// Submit a job spec — positional path per US-05 AC.
    Submit {
        /// Path to a TOML job-spec file.
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
    /// Read canonical `spec_digest` + `commit_index` for a job and the
    /// number of live allocations for it. Named after ADR-0014's
    /// `GET /v1/jobs/{id}` + `GET /v1/allocs` composition — the CLI
    /// surface is a single command even though it spans two handlers.
    Status {
        /// Canonical `JobId` to describe.
        #[arg(long)]
        job: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ClusterCommand {
    /// Mint a fresh ephemeral CA and write the trust triple to the
    /// config directory (`$OVERDRIVE_CONFIG_DIR` or `~/.overdrive/`).
    /// Re-invoking always re-mints per ADR-0010 §R4; `--force` is
    /// reserved for future non-destructive modes.
    Init {
        #[arg(long)]
        force: bool,
    },
    Upgrade {
        #[arg(long, value_parser = ["single", "ha"])]
        mode: String,
        #[arg(long, value_delimiter = ',')]
        peers: Vec<String>,
    },
    Status,
}
