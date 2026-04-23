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
        /// Run exactly one invariant by its canonical kebab-case name.
        /// Unknown names fail fast before the harness is built.
        #[arg(long)]
        only: Option<String>,
    },

    /// Tier 1 — banned-API lint gate over `crate_class = "core"` crates.
    /// See `docs/product/architecture/adr-0003-core-crate-labelling.md`
    /// and `.claude/rules/development.md`.
    DstLint {
        /// Path to the workspace `Cargo.toml` to scan. Defaults to the
        /// enclosing workspace root (cwd-relative).
        #[arg(long, default_value = "Cargo.toml")]
        manifest_path: std::path::PathBuf,
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

    /// Mutation testing (`cargo-mutants`) — diff-scoped per PR or
    /// full-workspace (nightly).
    ///
    /// Exactly one of `--diff` or `--workspace` must be given. Both
    /// write `target/xtask/mutants-summary.json` with the gate verdict
    /// and kill-rate figures; exit status is zero iff the gate passed.
    ///
    /// Thresholds match `.claude/rules/testing.md`:
    ///
    /// - `--diff`: kill rate ≥ 80% (hard fail below).
    /// - `--workspace`: kill rate ≥ 60% absolute floor (hard fail);
    ///   drift ≤ -2pp vs. baseline is a soft-warn.
    Mutants(MutantsArgs),

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

    /// Manage MCP server configuration for this project (`.mcp.json`).
    ///
    /// Claude Code does not expand environment variables inside `.mcp.json`,
    /// so secrets must be materialised at setup time. This subcommand reads
    /// the required tokens from the process environment (or a local `.env`)
    /// and writes a ready-to-use `.mcp.json` at the workspace root.
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
}

#[derive(Debug, Parser)]
#[command(
    about = "Mutation testing (cargo-mutants) — diff or workspace mode",
    long_about = "Exactly one of --diff or --workspace must be given. Writes \
                  target/xtask/mutants-summary.json; exit status is zero iff \
                  the gate passed (≥80% kill rate for --diff; ≥60% absolute \
                  floor for --workspace, with drift ≤ -2pp as a soft-warn)."
)]
struct MutantsArgs {
    /// Diff-scoped: git ref to diff against (e.g. `origin/main`).
    /// Produces a diff file and passes it to `cargo mutants --in-diff`.
    #[arg(long, group = "mutants_mode", value_name = "BASE_REF")]
    diff: Option<String>,

    /// Full-workspace mode. Compares the run against the baseline at
    /// the path given by `--baseline` (default:
    /// `mutants-baseline/main/kill_rate.txt`).
    #[arg(long, group = "mutants_mode")]
    workspace: bool,

    /// Path to the stored baseline kill rate for `--workspace`
    /// (percent as a float, e.g. `75.0`). Seeded if missing.
    #[arg(
        long,
        value_name = "BASELINE_PATH",
        default_value = "mutants-baseline/main/kill_rate.txt",
        requires = "workspace"
    )]
    baseline: std::path::PathBuf,
}

#[derive(Debug, Clone, Copy, Subcommand)]
enum McpAction {
    /// Render `.mcp.json` from the built-in template, injecting tokens
    /// from the process environment or `.env` at the workspace root.
    Setup {
        /// Overwrite an existing `.mcp.json` without prompting.
        #[arg(long)]
        force: bool,
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
        Task::Dst { seed, only } => xtask::dst::run(seed, only.as_deref()),
        Task::DstLint { manifest_path } => xtask::dst_lint::run(&manifest_path),
        Task::BpfUnit => bpf_unit(),
        Task::IntegrationTest { scope } => match scope {
            IntegrationScope::Vm { cache_dir, kernels } => integration_vm(&cache_dir, &kernels),
        },
        Task::VerifierRegress => verifier_regress(),
        Task::XdpPerf => xdp_perf(),
        Task::Mutants(args) => mutants(args),
        Task::Ci => ci(),
        Task::Lima { action } => lima(action),
        Task::Hooks { action } => hooks(action),
        Task::Mcp { action } => mcp(action),
    }
}

fn mcp(action: McpAction) -> Result<()> {
    match action {
        McpAction::Setup { force } => mcp_setup(force),
    }
}

/// Project-root `.mcp.json` — rendered from the template below.
const MCP_JSON: &str = ".mcp.json";

/// Template for `.mcp.json`. Tokens are injected from the environment at
/// setup time because Claude Code does not expand env vars at load time.
/// Toolsets enabled on the remote GitHub MCP server. `default` preserves
/// the server's built-in set (context, repos, issues, `pull_requests`,
/// users); the rest extend it.
const GITHUB_MCP_TOOLSETS: &str = "default,projects,discussions,labels";

fn render_mcp_json(github_pat: &str) -> Result<String> {
    let doc = serde_json::json!({
        "mcpServers": {
            "github": {
                "type": "http",
                "url": "https://api.githubcopilot.com/mcp/",
                "headers": {
                    "Authorization": format!("Bearer {github_pat}"),
                    "X-MCP-Toolsets": GITHUB_MCP_TOOLSETS
                }
            }
        }
    });
    Ok(serde_json::to_string_pretty(&doc)? + "\n")
}

fn mcp_setup(force: bool) -> Result<()> {
    let workspace_root = std::env::current_dir()?;
    let out_path = workspace_root.join(MCP_JSON);

    if out_path.exists() && !force {
        bail!("{} already exists; re-run with `--force` to overwrite", out_path.display());
    }

    let env_file = load_env_file(&workspace_root.join(".env"))?;
    let github_pat = lookup_required(
        &env_file,
        &["GITHUB_PAT", "GITHUB_PERSONAL_ACCESS_TOKEN"],
        "create one at https://github.com/settings/personal-access-tokens/new \
         and either `export GITHUB_PAT=...` or add it to `.env`",
    )?;

    let rendered = render_mcp_json(&github_pat)?;
    std::fs::write(&out_path, rendered)?;
    eprintln!("xtask: wrote {}", out_path.display());
    eprintln!("xtask: restart Claude Code and run `/mcp` to pick up the new server");
    Ok(())
}

/// Parse a `.env` file into `(key, value)` pairs via `dotenvy`. Missing
/// file is not an error — the process environment may still satisfy the
/// lookup. Parse errors (malformed lines, IO) are propagated so the
/// operator sees why setup refused to proceed.
fn load_env_file(path: &std::path::Path) -> Result<Vec<(String, String)>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    dotenvy::from_path_iter(path)?.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
}

/// Look up the first matching key in the process environment, falling
/// back to the parsed `.env` file. Returns an error with the install
/// hint when no source provides a value.
fn lookup_required(
    env_file: &[(String, String)],
    keys: &[&str],
    install_hint: &str,
) -> Result<String> {
    for key in keys {
        if let Ok(val) = std::env::var(key) {
            if !val.is_empty() {
                return Ok(val);
            }
        }
    }
    for key in keys {
        if let Some((_, val)) = env_file.iter().find(|(k, _)| k == key) {
            if !val.is_empty() {
                return Ok(val.clone());
            }
        }
    }
    bail!("none of {:?} set in the environment or `.env`. {}", keys, install_hint)
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
        .is_ok_and(|s| s.success());
    if !found {
        bail!("`{binary}` not found on PATH. Install it with: {install_hint}");
    }
    Ok(())
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

fn mutants(args: MutantsArgs) -> Result<()> {
    let mode = match (args.diff, args.workspace) {
        (Some(base), false) => xtask::mutants::Mode::Diff { base },
        (None, true) => xtask::mutants::Mode::Workspace { baseline_path: args.baseline },
        (Some(_), true) => {
            // clap's `group` should prevent this, but defence in depth.
            bail!("--diff and --workspace are mutually exclusive")
        }
        (None, false) => bail!("must give exactly one of --diff <BASE_REF> or --workspace"),
    };
    xtask::mutants::run(&mode)
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
