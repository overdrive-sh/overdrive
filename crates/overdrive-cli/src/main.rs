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
#![allow(clippy::print_stdout)] // Operator-facing CLI output is the intended use of stdout.
#![allow(clippy::print_stderr)] // Error output on failing subcommands is the intended use of stderr.

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

async fn run(cli: Cli) -> Result<()> {
    use overdrive_cli::cli::{AllocCommand, ClusterCommand, Command, JobCommand, NodeCommand};

    match cli.command {
        Command::Cluster(ClusterCommand::Status) => {
            let config_path = default_config_path();
            let args = overdrive_cli::commands::cluster::StatusArgs { config_path };
            let out = overdrive_cli::commands::cluster::status(args).await?;
            print!("{}", overdrive_cli::render::cluster_status(&out));
            Ok(())
        }
        Command::Job(JobCommand::Submit { spec }) => {
            let config_path = default_config_path();
            let args = overdrive_cli::commands::job::SubmitArgs { spec, config_path };
            match overdrive_cli::commands::job::submit(args).await {
                Ok(out) => {
                    print!("{}", overdrive_cli::render::job_submit_accepted(&out));
                    Ok(())
                }
                Err(err) => {
                    // Render through the CLI-side error formatter so
                    // operators see actionable next steps on
                    // `CliError::Transport`, not the raw Display form.
                    eprint!("{}", overdrive_cli::render::cli_error(&err));
                    Err(color_eyre::eyre::eyre!("job submit failed"))
                }
            }
        }
        Command::Job(JobCommand::Stop { id }) => {
            let config_path = default_config_path();
            let args = overdrive_cli::commands::job::StopArgs { id, config_path };
            match overdrive_cli::commands::job::stop(args).await {
                Ok(out) => {
                    print!("{}", overdrive_cli::render::job_stop_accepted(&out));
                    Ok(())
                }
                Err(err) => {
                    eprint!("{}", overdrive_cli::render::cli_error(&err));
                    Err(color_eyre::eyre::eyre!("job stop failed"))
                }
            }
        }
        Command::Alloc(AllocCommand::Status { job }) => {
            let config_path = default_config_path();
            let args = overdrive_cli::commands::alloc::StatusArgs { job, config_path };
            match overdrive_cli::commands::alloc::status(args).await {
                Ok(out) => {
                    print!("{}", overdrive_cli::render::alloc_status(&out));
                    Ok(())
                }
                Err(err) => {
                    eprint!("{}", overdrive_cli::render::cli_error(&err));
                    Err(color_eyre::eyre::eyre!("alloc status failed"))
                }
            }
        }
        Command::Node(NodeCommand::List) => {
            let config_path = default_config_path();
            let args = overdrive_cli::commands::node::ListArgs { config_path };
            let out = overdrive_cli::commands::node::list(args).await?;
            print!("{}", overdrive_cli::render::node_list(&out));
            Ok(())
        }
        Command::Serve { bind, data_dir } => {
            let bind_addr = bind
                .parse()
                .map_err(|e| color_eyre::eyre::eyre!("invalid --bind address `{bind}`: {e}"))?;
            let data_dir = data_dir.unwrap_or_else(default_data_dir);
            // Operator config dir is the canonical write target for
            // the trust triple — must equal what `default_config_path`
            // resolves to on the read side, so `serve` and `job submit`
            // share the same file (`fix-cli-cannot-reach-control-plane`).
            let config_dir = overdrive_cli::commands::cluster::default_operator_config_dir();
            let args =
                overdrive_cli::commands::serve::ServeArgs { bind: bind_addr, data_dir, config_dir };
            let handle = overdrive_cli::commands::serve::run(args).await?;
            tracing::info!(endpoint = %handle.endpoint(), "control plane listening");

            // SIGINT handling per `crates/overdrive-cli/CLAUDE.md`: the
            // binary selects on the shutdown signal and the server task.
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("SIGINT received; shutting down");
                }
            }
            handle.shutdown().await?;
            Ok(())
        }
        other => {
            // Remaining subcommands (Job, Alloc, Cluster::Upgrade) still
            // land in later Phase 1 steps. Log and exit 0 so smoke-tests
            // keep working.
            tracing::warn!(command = ?other, "command not yet wired");
            Ok(())
        }
    }
}

/// Default operator config path per ADR-0010 / ADR-0014 / ADR-0019
/// and whitepaper §8: `~/.overdrive/config` (single `.overdrive`
/// segment). Resolves the base directory from `$OVERDRIVE_CONFIG_DIR`
/// first, then `$HOME`, then `.` as a last resort; the `.overdrive`
/// segment and `config` filename are appended exactly once by the
/// shared helper. Both `$OVERDRIVE_CONFIG_DIR` and `$HOME` are BASE
/// directories — callers do not pre-suffix with `.overdrive`.
///
/// The CLI binary resolves this once; library tests always pass an
/// explicit path. Delegates to
/// `overdrive_cli::commands::cluster::default_operator_config_path`
/// so the read side of the CLI computes the same canonical path as
/// the write side (`serve::run` + `write_trust_triple`) — the two
/// sites previously drifted (`fix-overdrive-config-path-doubled`).
/// `serve` is the sole cert-minting site in Phase 1 per
/// `fix-remove-phase-1-cluster-init` (#81).
fn default_config_path() -> std::path::PathBuf {
    overdrive_cli::commands::cluster::default_operator_config_path()
}

/// Default data directory per ADR-0013 §5 — XDG `data_dir()/overdrive`.
/// Falls back to `./overdrive` if `$XDG_DATA_HOME` and `$HOME` are
/// both unset (this is a bin-layer concern; library tests always pass
/// an explicit `data_dir`).
fn default_data_dir() -> std::path::PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return std::path::PathBuf::from(xdg).join("overdrive");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return std::path::PathBuf::from(home).join(".local/share/overdrive");
    }
    std::path::PathBuf::from("./overdrive")
}
