//! `cargo xtask <cmd>` — the one place CI entry points live.
//!
//! Every gate in `.claude/rules/testing.md` corresponds to a subcommand
//! here. Each subcommand is a stub until the underlying subsystem lands;
//! filling them in is the job of each phase of the roadmap.

#![allow(clippy::expect_used, clippy::print_stderr, clippy::unnecessary_wraps)]

use std::process::{Command, ExitCode};

use clap::{Parser, Subcommand};
use color_eyre::eyre::{Result, bail};

#[derive(Debug, Parser)]
#[command(about = "Overdrive developer / CI tasks", version)]
struct Args {
    #[command(subcommand)]
    cmd: Task,
}

#[derive(Debug, Subcommand)]
enum Task {
    /// Tier 1 — deterministic simulation tests (`turmoil` + `Sim*` traits).
    Dst {
        /// Seed for reproducible runs. Defaults to a fresh random seed.
        #[arg(long)]
        seed: Option<u64>,
    },

    /// Tier 2 — BPF unit tests via `BPF_PROG_TEST_RUN`.
    BpfUnit,

    /// Tier 3 — real-kernel integration tests. Reuses aya's
    /// `cargo xtask integration-test vm` harness.
    IntegrationTest {
        #[command(subcommand)]
        scope: IntegrationScope,
    },

    /// Tier 4 — verifier complexity regression (`veristat`).
    VerifierRegress,

    /// Tier 4 — XDP throughput / p99 regression (`xdp-bench`).
    XdpPerf,

    /// Lint + format check (mirrors CI).
    Ci,

    /// Manage the `overdrive` Lima VM used for Linux-specific builds and
    /// BPF/integration tests from a macOS host. No-op on Linux.
    Lima {
        #[command(subcommand)]
        action: LimaAction,
    },

    /// Manage git hooks via lefthook — see `lefthook.yml`.
    Hooks {
        #[command(subcommand)]
        action: HooksAction,
    },
}

#[derive(Debug, Subcommand)]
enum HooksAction {
    /// Install `.git/hooks/*` from `lefthook.yml`.
    Install,
    /// Remove Overdrive-managed git hooks.
    Uninstall,
    /// Validate `lefthook.yml` without installing.
    Validate,
    /// Run a named hook manually (e.g. `pre-commit`, `pre-push`).
    Run { hook: String },
}

#[derive(Debug, Subcommand)]
enum LimaAction {
    /// Create & start the VM (or start an existing one).
    Up,
    /// Open an interactive shell in the VM.
    Shell,
    /// Run a one-off command inside the VM (remaining args forwarded).
    Run {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
        args: Vec<String>,
    },
    /// Stop the VM (state preserved).
    Stop,
    /// Delete the VM (destroys persisted state).
    Delete,
    /// Validate the template without starting the VM.
    Validate,
}

#[derive(Debug, Subcommand)]
enum IntegrationScope {
    /// Full kernel matrix inside QEMU via `little-vm-helper`.
    Vm {
        #[arg(long, default_value = "target/xtask/lvh-cache")]
        cache_dir: std::path::PathBuf,
        /// One or more kernels from the matrix (5.10, 5.15, 6.1, 6.6, latest, bpf-next).
        kernels: Vec<String>,
    },
}

fn main() -> ExitCode {
    if let Err(err) = color_eyre::install() {
        eprintln!("failed to install color-eyre: {err}");
        return ExitCode::FAILURE;
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xtask failed: {err:?}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    match Args::parse().cmd {
        Task::Dst { seed } => dst(seed),
        Task::BpfUnit => bpf_unit(),
        Task::IntegrationTest { scope } => match scope {
            IntegrationScope::Vm { cache_dir, kernels } => integration_vm(&cache_dir, &kernels),
        },
        Task::VerifierRegress => verifier_regress(),
        Task::XdpPerf => xdp_perf(),
        Task::Ci => ci(),
        Task::Lima { action } => lima(action),
        Task::Hooks { action } => hooks(action),
    }
}

fn hooks(action: HooksAction) -> Result<()> {
    which_or_hint(
        "lefthook",
        "brew install lefthook  # or see https://lefthook.dev/installation/",
    )?;
    match action {
        HooksAction::Install => sh("lefthook install", Command::new("lefthook").arg("install")),
        HooksAction::Uninstall => {
            sh("lefthook uninstall", Command::new("lefthook").arg("uninstall"))
        }
        HooksAction::Validate => sh("lefthook validate", Command::new("lefthook").arg("validate")),
        HooksAction::Run { hook } => {
            sh("lefthook run", Command::new("lefthook").args(["run", &hook]))
        }
    }
}

const LIMA_INSTANCE: &str = "overdrive";
const LIMA_TEMPLATE: &str = "infra/lima/overdrive-dev.yaml";

fn lima(action: LimaAction) -> Result<()> {
    if !cfg!(target_os = "macos") {
        eprintln!("xtask: lima target is macOS-only; skipping on {}", std::env::consts::OS);
        return Ok(());
    }
    which_or_hint("limactl", "brew install lima")?;

    match action {
        LimaAction::Up => sh(
            "limactl start",
            Command::new("limactl").args([
                "start",
                "--name",
                LIMA_INSTANCE,
                "--tty=false",
                LIMA_TEMPLATE,
            ]),
        ),
        LimaAction::Shell => {
            sh("limactl shell", Command::new("limactl").args(["shell", LIMA_INSTANCE]))
        }
        LimaAction::Run { args } => {
            if args.is_empty() {
                bail!("no command given; use `cargo xtask lima run -- cargo xtask dst` etc.");
            }
            let mut cmd = Command::new("limactl");
            cmd.args(["shell", LIMA_INSTANCE]).args(&args);
            sh("limactl shell <cmd>", &mut cmd)
        }
        LimaAction::Stop => {
            sh("limactl stop", Command::new("limactl").args(["stop", LIMA_INSTANCE]))
        }
        LimaAction::Delete => {
            sh("limactl delete", Command::new("limactl").args(["delete", "--force", LIMA_INSTANCE]))
        }
        LimaAction::Validate => {
            sh("limactl validate", Command::new("limactl").args(["validate", LIMA_TEMPLATE]))
        }
    }
}

fn which_or_hint(binary: &str, install_hint: &str) -> Result<()> {
    let found = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {binary}"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !found {
        bail!("`{binary}` not found on PATH. Install it with: {install_hint}");
    }
    Ok(())
}

fn dst(seed: Option<u64>) -> Result<()> {
    let mut cmd = Command::new(cargo());
    cmd.args(["test", "--workspace", "--features", "dst", "--", "--include-ignored"]);
    if let Some(s) = seed {
        cmd.env("OVERDRIVE_DST_SEED", s.to_string());
    }
    sh("cargo test (dst)", &mut cmd)
}

fn bpf_unit() -> Result<()> {
    // Placeholder — `crates/overdrive-bpf` lands in Phase 2. This will
    // invoke `cargo test --package overdrive-bpf --test '*'` against the
    // BPF_PROG_TEST_RUN harness.
    tracing_placeholder("bpf-unit: overdrive-bpf crate lands in Phase 2")
}

fn integration_vm(cache_dir: &std::path::Path, kernels: &[String]) -> Result<()> {
    if kernels.is_empty() {
        bail!("specify at least one kernel (e.g. 5.15, 6.1, 6.6, latest, bpf-next)");
    }
    // Placeholder — Tier 3 harness lands in Phase 2. Will reuse aya's
    // `cargo xtask integration-test vm --cache-dir <dir> <KERNEL>...`.
    let summary = format!(
        "integration-test vm: Phase 2. cache={}, kernels={}",
        cache_dir.display(),
        kernels.join(",")
    );
    tracing_placeholder(&summary)
}

fn verifier_regress() -> Result<()> {
    tracing_placeholder("verifier-regress: veristat harness lands in Phase 2")
}

fn xdp_perf() -> Result<()> {
    tracing_placeholder("xdp-perf: xdp-bench harness lands in Phase 2")
}

fn ci() -> Result<()> {
    sh("cargo fmt --check", Command::new(cargo()).args(["fmt", "--all", "--", "--check"]))?;
    sh(
        "cargo clippy",
        Command::new(cargo()).args([
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ]),
    )?;
    sh("cargo test", Command::new(cargo()).args(["test", "--workspace", "--all-targets"]))
}

fn sh(label: &str, cmd: &mut Command) -> Result<()> {
    eprintln!("xtask: running {label}");
    let status = cmd.status()?;
    if !status.success() {
        bail!("{label} failed with {status}");
    }
    Ok(())
}

fn cargo() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

fn tracing_placeholder(msg: &str) -> Result<()> {
    eprintln!("xtask: {msg}");
    Ok(())
}
