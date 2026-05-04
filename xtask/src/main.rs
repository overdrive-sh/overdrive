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

    /// ADR-0019 gate — assert no `serde_yaml` / `serde_yml` appears in
    /// the `overdrive-cli` resolved dependency graph. Scoped to
    /// non-dev dependencies; test-only YAML is out of scope.
    /// See `docs/product/architecture/adr-0019-operator-config-format-toml.md`.
    YamlFreeCli {
        /// Path to the workspace `Cargo.toml` to scan. Defaults to the
        /// enclosing workspace root (cwd-relative).
        #[arg(long, default_value = "Cargo.toml")]
        manifest_path: std::path::PathBuf,
    },

    /// Compile `crates/overdrive-bpf` against `bpfel-unknown-none` and
    /// copy the produced ELF to the load-bearing stable path
    /// `target/xtask/bpf-objects/overdrive_bpf.o` that the loader's
    /// `include_bytes!` references.
    ///
    /// Per ADR-0038 §3.1 the build is a child-process invocation of
    /// `cargo +nightly build --release --target bpfel-unknown-none -Z
    /// build-std=core --features build-bpf-target --manifest-path
    /// crates/overdrive-bpf/Cargo.toml` — no recursive cargo from
    /// `build.rs`. The `--features build-bpf-target` flag is required
    /// to gate-in the kernel-side `[[bin]]` (host workflows skip it
    /// via `required-features` to avoid the `#![no_std]` lang-item
    /// conflict on the host triple — see crates/overdrive-bpf/Cargo.toml).
    BpfBuild,

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

    /// One-shot developer bootstrap: installs the CLI tools this
    /// workspace depends on (cargo-nextest), runs `lefthook install`
    /// when lefthook is present, and prints install hints for anything
    /// that cannot be auto-installed.
    ///
    /// Idempotent — running it against an already-set-up checkout is a
    /// no-op modulo the `lefthook install` step (which itself is a
    /// no-op when the hooks are already wired).
    ///
    /// Rationale: Cargo has no `[tool-deps]` concept, so the canonical
    /// way to pin the project's tool versions is to treat "install the
    /// tools" as a repo artifact. This subcommand IS that artifact.
    DevSetup,

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

    /// Regenerate `api/openapi.yaml` from the live `OverdriveApi`
    /// schema. Invoked by developers after adding or changing a DTO or
    /// handler path. Per ADR-0009, the checked-in YAML is the contract;
    /// drift is caught by `openapi-check` in CI.
    OpenapiGen,

    /// Verify `api/openapi.yaml` matches the live `OverdriveApi`
    /// schema. Exits 0 on match; non-zero with a message naming the
    /// first drifted schema/path and suggesting `cargo xtask
    /// openapi-gen` to regenerate. CI gate per ADR-0009.
    OpenapiCheck,
}

#[derive(Debug, Parser)]
#[command(
    about = "Mutation testing (cargo-mutants) — diff or workspace mode",
    long_about = "Exactly one of --diff or --workspace must be given. Writes \
                  target/xtask/mutants-summary.json; exit status is zero iff \
                  the gate passed (≥80% kill rate for --diff; ≥60% absolute \
                  floor for --workspace, with drift ≤ -2pp as a soft-warn). \
                  Narrow further with --file, --package, and --features. \
                  --package defaults to --test-workspace=false for speed. \
                  Pass --features integration-tests explicitly when you want \
                  acceptance tests gated behind that cfg to participate — \
                  the workspace convention requires every member to declare \
                  the feature (see .claude/rules/testing.md §\"Integration \
                  vs unit gating\"), so the bare flag resolves uniformly."
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

    /// Files to mutate (repeatable). Passed through to cargo-mutants
    /// as `--file <GLOB>`. Use to narrow a diff-scoped run to a
    /// specific file, or a workspace run to a subset of files.
    #[arg(long, value_name = "GLOB")]
    file: Vec<std::path::PathBuf>,

    /// Cargo package to mutate (repeatable). Passed through to
    /// cargo-mutants as `--package <CRATE>`. When set,
    /// `--test-workspace=false` is added automatically — mutation
    /// reruns only the selected package's tests. Pass
    /// `--test-whole-workspace` to opt out.
    #[arg(long, value_name = "CRATE")]
    package: Vec<String>,

    /// Features to enable when building mutated code. Comma- or
    /// space-separated; multiple `--features` flags append. Passed
    /// through to cargo-mutants as `--features <LIST>` verbatim — the
    /// wrapper does not add or rewrite anything.
    ///
    /// To exercise acceptance tests gated behind `#[cfg(feature =
    /// "integration-tests")]` (see `.claude/rules/testing.md`
    /// §"Integration vs unit gating"), pass `--features
    /// integration-tests` explicitly. Every workspace member declares
    /// the feature (no-op `[]` for crates without integration tests),
    /// so the bare flag resolves uniformly under cargo-mutants v27's
    /// per-package scoping.
    #[arg(long, value_name = "LIST", value_delimiter = ',')]
    features: Vec<String>,

    /// Force `--test-workspace=true` even with `--package`. Rare; use
    /// when mutations in the selected package can only be killed by
    /// tests in another crate.
    #[arg(long)]
    test_whole_workspace: bool,
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
    /// Open an interactive shell in the VM (runs as the unprivileged
    /// `lima` user; use `sudo -i` inside if you need root).
    Shell,
    /// Run a one-off command inside the VM (remaining args forwarded).
    ///
    /// Default behaviour wraps the command in
    /// `sudo -E env "PATH=$PATH" "CARGO_TARGET_DIR=$CARGO_TARGET_DIR" ...`
    /// so the test process runs as root — the same permission surface
    /// CI's LVH VM sees. Pass `--no-sudo` to run as the unprivileged
    /// `lima` user instead.
    Run {
        /// Run the command as the `lima` user instead of wrapping in
        /// `sudo -E ...`. Use when the command does not need cgroup
        /// writes or other root-only operations.
        #[arg(long)]
        no_sudo: bool,
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
        Task::YamlFreeCli { manifest_path } => xtask::yaml_free_cli::run(&manifest_path),
        Task::BpfBuild => bpf_build(),
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
        Task::DevSetup => dev_setup(),
        Task::OpenapiGen => xtask::openapi::openapi_gen(),
        Task::OpenapiCheck => xtask::openapi::openapi_check(),
    }
}

/// One-shot developer bootstrap — installs the tools this workspace
/// depends on. Keep the list here in sync with `.config/nextest.toml`
/// and the install hints in `xtask::mutants` / `lefthook.yml`.
fn dev_setup() -> Result<()> {
    // 1. cargo-nextest — the project-wide test runner per
    //    `.claude/rules/testing.md` §"Running tests — foreground, always".
    //    Idempotent: `cargo install --locked` no-ops when the exact
    //    locked version is already installed.
    if Command::new("sh")
        .arg("-c")
        .arg("command -v cargo-nextest")
        .status()
        .is_ok_and(|s| s.success())
    {
        eprintln!("xtask dev-setup: cargo-nextest already on PATH");
    } else {
        sh(
            "cargo install cargo-nextest --locked",
            Command::new(cargo()).args(["install", "cargo-nextest", "--locked"]),
        )?;
    }

    // 2. lefthook — cannot be installed via cargo (Go binary). Hint
    //    and skip if absent; otherwise run `lefthook install` so the
    //    repo's pre-commit / pre-push hooks are wired on this checkout.
    let lefthook_present =
        Command::new("sh").arg("-c").arg("command -v lefthook").status().is_ok_and(|s| s.success());
    if lefthook_present {
        sh("lefthook install", Command::new("lefthook").arg("install"))?;
    } else {
        eprintln!(
            "xtask dev-setup: lefthook not found on PATH. Install it with:\n  \
             brew install lefthook  # or see https://lefthook.dev/installation/\n  \
             Then re-run `cargo xtask dev-setup` to wire the git hooks."
        );
    }

    eprintln!("xtask dev-setup: done");
    Ok(())
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

fn render_mcp_json(github_pat: &str, greptile_api_key: &str) -> Result<String> {
    let doc = serde_json::json!({
        "mcpServers": {
            "github": {
                "type": "http",
                "url": "https://api.githubcopilot.com/mcp/",
                "headers": {
                    "Authorization": format!("Bearer {github_pat}"),
                    "X-MCP-Toolsets": GITHUB_MCP_TOOLSETS
                }
            },
            "greptile": {
                "type": "http",
                "url": "https://api.greptile.com/mcp",
                "headers": {
                    "Authorization": format!("Bearer {greptile_api_key}")
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
    let greptile_api_key = lookup_required(
        &env_file,
        &["GREPTILE_API_KEY"],
        "create one at https://app.greptile.com (Settings → API Keys) \
         and either `export GREPTILE_API_KEY=...` or add it to `.env`",
    )?;

    let rendered = render_mcp_json(&github_pat, &greptile_api_key)?;
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
        LimaAction::Run { no_sudo, args } => {
            if args.is_empty() {
                bail!("no command given; use `cargo xtask lima run -- cargo xtask dst` etc.");
            }
            let mut cmd = Command::new("limactl");
            if no_sudo {
                cmd.args(["shell", LIMA_INSTANCE]).args(&args);
            } else {
                // Default: run the test process as root inside the VM
                // so cgroup writes and other privileged ops succeed —
                // the same permission shape CI's LVH harness uses.
                // `sudo -E` preserves env; `env "PATH=$PATH"
                // "CARGO_TARGET_DIR=$CARGO_TARGET_DIR"` re-injects the
                // two vars sudo's `secure_path` would otherwise scrub
                // so cargo and its target dir resolve under the
                // `lima` user's home (where rustup is installed).
                let joined = args.iter().map(|a| sh_escape(a)).collect::<Vec<_>>().join(" ");
                let inner = format!(
                    r#"sudo -E env "PATH=$PATH" "CARGO_TARGET_DIR=$CARGO_TARGET_DIR" {joined}"#
                );
                cmd.args(["shell", LIMA_INSTANCE, "bash", "-lc", &inner]);
            }
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

/// Single-quote-wrap an argument so it survives `bash -lc` re-parsing
/// inside the Lima guest. POSIX single quotes preserve every byte
/// except `'` itself, which closes the quoted span; we close, escape
/// the literal quote, and reopen.
fn sh_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    let safe = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '=' | ',' | ':'));
    if safe {
        return s.into();
    }
    let escaped = s.replace('\'', r"'\''");
    format!("'{escaped}'")
}

fn which_or_hint(binary: &str, install_hint: &str) -> Result<()> {
    let found = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {binary}"))
        .status()
        .is_ok_and(|s| s.success());
    if !found {
        // If the hint already starts with the canonical
        // "`<binary>` not found on PATH." prefix, surface it verbatim
        // so callers like `bpf_build` can supply a multi-line hint
        // without the prefix being doubled. Otherwise fall back to the
        // single-line shape used by every other call site.
        let canonical_prefix = format!("`{binary}` not found on PATH");
        if install_hint.starts_with(&canonical_prefix) {
            bail!("{install_hint}");
        }
        bail!("`{binary}` not found on PATH. Install it with: {install_hint}");
    }
    Ok(())
}

/// Compile `crates/overdrive-bpf` against `bpfel-unknown-none` and
/// copy the resulting ELF to the load-bearing stable path
/// `target/xtask/bpf-objects/overdrive_bpf.o` that the loader's
/// `include_bytes!` references (see ADR-0038 §3.1, architecture.md
/// §3.1, wave-decisions.md D3).
///
/// Three failure modes, all surface as a structured `eyre::Report`
/// with non-zero exit:
///
/// 1. `bpf-linker` is not on PATH — caught by `which_or_hint` with a
///    hint listing the three install paths (`cargo install --locked
///    bpf-linker`, `cargo xtask dev-setup`, Lima re-provision).
/// 2. The child `cargo +nightly build` exits non-zero — captured
///    stderr is propagated.
/// 3. File I/O on the copy step (parent dir creation, `fs::copy`) —
///    propagated with the source/destination paths.
///
/// The copy is `fs::copy`, not move — keep the cargo-target ELF in
/// place so subsequent rebuilds short-circuit on no-change.
fn bpf_build() -> Result<()> {
    which_or_hint("bpf-linker", &bpf_linker_install_hint())?;

    let workspace_root = workspace_root_dir()?;
    let manifest = workspace_root.join("crates/overdrive-bpf/Cargo.toml");

    // Invoke through `rustup run nightly cargo …` rather than the
    // bare `cargo +nightly` form. The `$CARGO` env var that
    // `cargo()` resolves to is populated by cargo itself with the
    // direct cargo binary (not rustup's shim), and the direct
    // binary does not parse `+toolchain` directives. Going through
    // rustup is the canonical way to pin a non-default toolchain
    // when the parent process was launched by stable cargo (rustup
    // book § "Channels and Toolchain Specifiers"). The
    // `-Z build-std=core` flag requires nightly per
    // `wave-decisions.md` D3 / ADR-0038 §3.1; nightly is provisioned
    // alongside stable on the dev surfaces (Lima, dev-setup).
    sh(
        "rustup run nightly cargo build (overdrive-bpf, bpfel-unknown-none)",
        Command::new("rustup")
            .args([
                "run",
                "nightly",
                "cargo",
                "build",
                "--release",
                "--target",
                "bpfel-unknown-none",
                "-Z",
                "build-std=core",
                "--features",
                "build-bpf-target",
                "--manifest-path",
            ])
            .arg(&manifest)
            .current_dir(&workspace_root),
    )?;

    // Copy the produced ELF to the stable path the loader's
    // `include_bytes!` references. The `bpfel-unknown-none/release/`
    // directory is cargo-target-dir-relative; respect $CARGO_TARGET_DIR
    // when set so the copy still lands when the target dir is
    // redirected (e.g. Lima's `/home/marcus.guest/.cargo-target-lima`).
    let target_dir = cargo_target_dir(&workspace_root);
    let src = target_dir.join("bpfel-unknown-none/release/overdrive-bpf");
    let dst_dir = workspace_root.join("target/xtask/bpf-objects");
    let dst = dst_dir.join("overdrive_bpf.o");

    std::fs::create_dir_all(&dst_dir)
        .map_err(|e| color_eyre::eyre::eyre!("failed to create {}: {e}", dst_dir.display()))?;
    std::fs::copy(&src, &dst).map_err(|e| {
        color_eyre::eyre::eyre!(
            "failed to copy BPF ELF {} -> {}: {e}",
            src.display(),
            dst.display()
        )
    })?;

    eprintln!("xtask: bpf-build wrote {}", dst.display());
    Ok(())
}

/// Hint string returned to the operator when `bpf-linker` is missing.
/// Per ADR-0038 §4 / wave-decisions.md D4 the hint MUST name all three
/// install paths so the operator picks the one matching their dev
/// surface — Lima users re-provision; non-Lima Linux developers run
/// `cargo xtask dev-setup` (step 02-03); anyone else uses the raw
/// `cargo install --locked` form. `--locked` is mandatory across every
/// install site for reproducibility (ADR-0038 §4).
fn bpf_linker_install_hint() -> String {
    "`bpf-linker` not found on PATH. Install with one of:\n  \
     • `cargo install --locked bpf-linker`\n  \
     • `cargo xtask dev-setup` (non-Lima Linux dev surface)\n  \
     • re-provision the Lima VM (`cargo xtask lima delete && cargo xtask lima up`)\n\
     See ADR-0038 §4 for toolchain provisioning."
        .to_string()
}

/// Resolve the workspace root. Uses `cargo_metadata` (already a build
/// dep) so the path is correct even when xtask is launched from a
/// nested working directory.
fn workspace_root_dir() -> Result<std::path::PathBuf> {
    let metadata = cargo_metadata::MetadataCommand::new().no_deps().exec()?;
    Ok(metadata.workspace_root.into_std_path_buf())
}

/// Resolve the cargo target dir, honouring `$CARGO_TARGET_DIR` when
/// set. Lima dev sets this to `/home/marcus.guest/.cargo-target-lima`
/// so the same workspace can be built from macOS host and Linux guest
/// without colliding fingerprints.
fn cargo_target_dir(workspace_root: &std::path::Path) -> std::path::PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| workspace_root.join("target"), std::path::PathBuf::from)
}

fn bpf_unit() -> Result<()> {
    // Placeholder — `crates/overdrive-bpf` lands in Phase 2. This will
    // invoke `cargo nextest run -p overdrive-bpf --test '*'` against the
    // BPF_PROG_TEST_RUN harness. Nextest is the project-wide runner
    // (see `.config/nextest.toml`); this subcommand keeps the same
    // invariant.
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

    let scope = xtask::mutants::Scope {
        files: args.file,
        packages: args.package,
        features: args.features,
        test_whole_workspace: args.test_whole_workspace,
    };

    xtask::mutants::run(&mode, &scope)
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
    // nextest for the main suite, separate `cargo test --doc` for rustdoc
    // examples. Nextest does not execute doctests — see `.config/nextest.toml`
    // and `.github/workflows/ci.yml`'s `test` job for the paired structure.
    which_or_hint(
        "cargo-nextest",
        "cargo install cargo-nextest --locked  # or: brew install cargo-nextest",
    )?;
    sh(
        "cargo nextest run",
        Command::new(cargo()).args(["nextest", "run", "--workspace", "--all-targets"]),
    )?;
    sh("cargo test --doc", Command::new(cargo()).args(["test", "--doc", "--workspace"]))
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
