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
        Command::Cluster(ClusterCommand::Init { force }) => {
            let args = overdrive_cli::commands::cluster::InitArgs { config_dir: None, force };
            let out = overdrive_cli::commands::cluster::init(args).await?;
            println!("wrote trust triple to {}", out.config_path.display());
            println!("endpoint: {}", out.endpoint);
            Ok(())
        }
        Command::Cluster(ClusterCommand::Status) => {
            let endpoint = cli.endpoint.parse().map_err(|e| {
                color_eyre::eyre::eyre!("invalid --endpoint `{}`: {e}", cli.endpoint)
            })?;
            let config_path = default_config_path();
            let args = overdrive_cli::commands::cluster::StatusArgs { endpoint, config_path };
            let out = overdrive_cli::commands::cluster::status(args).await?;
            print!("{}", overdrive_cli::render::cluster_status(&out));
            Ok(())
        }
        Command::Job(JobCommand::Submit { spec }) => {
            let endpoint = cli.endpoint.parse().map_err(|e| {
                color_eyre::eyre::eyre!("invalid --endpoint `{}`: {e}", cli.endpoint)
            })?;
            let config_path = default_config_path();
            let args = overdrive_cli::commands::job::SubmitArgs { spec, endpoint, config_path };
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
        Command::Alloc(AllocCommand::Status { job }) => {
            let endpoint = cli.endpoint.parse().map_err(|e| {
                color_eyre::eyre::eyre!("invalid --endpoint `{}`: {e}", cli.endpoint)
            })?;
            let config_path = default_config_path();
            let args = overdrive_cli::commands::alloc::StatusArgs { job, endpoint, config_path };
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
            let endpoint = cli.endpoint.parse().map_err(|e| {
                color_eyre::eyre::eyre!("invalid --endpoint `{}`: {e}", cli.endpoint)
            })?;
            let config_path = default_config_path();
            let args = overdrive_cli::commands::node::ListArgs { endpoint, config_path };
            let out = overdrive_cli::commands::node::list(args).await?;
            print!("{}", overdrive_cli::render::node_list(&out));
            Ok(())
        }
        Command::Serve { bind, data_dir } => {
            let bind_addr = bind
                .parse()
                .map_err(|e| color_eyre::eyre::eyre!("invalid --bind address `{bind}`: {e}"))?;
            let data_dir = data_dir.unwrap_or_else(default_data_dir);
            let args = overdrive_cli::commands::serve::ServeArgs { bind: bind_addr, data_dir };
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
            tracing::warn!(endpoint = %cli.endpoint, command = ?other, "command not yet wired");
            Ok(())
        }
    }
}

/// Default config path per ADR-0010: `$OVERDRIVE_CONFIG_DIR/.overdrive/config`
/// if set, `$HOME/.overdrive/.overdrive/config` otherwise. The CLI binary
/// resolves this once; library tests always pass an explicit path.
fn default_config_path() -> std::path::PathBuf {
    if let Some(cfg) = std::env::var_os("OVERDRIVE_CONFIG_DIR") {
        return std::path::PathBuf::from(cfg).join(".overdrive").join("config");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return std::path::PathBuf::from(home).join(".overdrive").join("config");
    }
    std::path::PathBuf::from("./.overdrive/config")
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
