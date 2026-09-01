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
    /// `target/bpf/overdrive_bpf.o` that the loader's
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

    /// Run clippy against the kernel-side `overdrive-bpf` bin under
    /// the same toolchain `bpf-build` uses (`+nightly`,
    /// `--target bpfel-unknown-none`, `-Z build-std=core`,
    /// `--features build-bpf-target`). The host workspace clippy
    /// run cannot lint this bin: it is `#![no_std] #![no_main]`
    /// and rustc rejects it on the host triple with "unwinding
    /// panics are not supported without std".
    BpfClippy,

    /// Tier 2 — BPF unit tests via `BPF_PROG_TEST_RUN`.
    BpfUnit,

    /// Tier 3 — real-kernel integration tests. Reuses aya's
    /// `cargo xtask integration-test vm` harness.
    IntegrationTest {
        #[command(subcommand)]
        scope: IntegrationScope,
    },

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
    /// BPF/integration tests. Required on all host platforms — macOS and
    /// Linux developers both use Lima for reproducibility and to avoid
    /// polluting the host with kernel toolchains.
    Lima {
        #[command(subcommand)]
        action: LimaAction,
    },

    /// Run tests / commands on the bare-metal `x86_64` KVM box.
    ///
    /// The Cloud-Hypervisor microVM `kvm-tests` require native,
    /// nonvirtualized `x86_64` hardware with usable KVM, so their Tier-3 boot
    /// surface runs on a qualified bare-metal box reached over ssh. The target host
    /// (`user@host`) comes from `OVERDRIVE_METAL_TARGET` in the process
    /// environment or `.env` at the workspace root (see `.env.example`).
    /// This is the metal sibling of `Lima` — `run` rsyncs the tree up
    /// (via `infra/metal/bootstrap.sh --sync-only`) and then ssh-executes
    /// under a `bash -lc` login shell (so rustup/cargo are on PATH),
    /// wrapping in `sudo … env "HOME=$HOME" "PATH=$PATH" …` by default so
    /// the KVM / cgroup tests get the root permission surface they need.
    Metal {
        #[command(subcommand)]
        action: MetalAction,
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
    /// `sudo -E env "HOME=$HOME" "PATH=$PATH" "CARGO_TARGET_DIR=$CARGO_TARGET_DIR" ...`
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
enum MetalAction {
    /// rsync the working tree up to the metal box (bootstrap.sh --sync-only).
    Sync,
    /// Open an interactive shell on the metal box in `~/overdrive`.
    Shell,
    /// Sync, then run a command on the metal box (remaining args forwarded).
    ///
    /// Default wraps the command in `sudo -E env "PATH=$PATH" ...` so it
    /// runs as root — the permission surface the KVM / cgroup tests need
    /// (`$PATH` is expanded on the remote host, not locally). Pass
    /// `--no-sudo` to run as the login user, `--no-sync` to skip the
    /// pre-run rsync (e.g. to re-run a suite against an already-synced tree).
    Run {
        /// Skip the pre-run rsync; run against whatever is already on the box.
        #[arg(long)]
        no_sync: bool,
        /// Run as the login user instead of wrapping in `sudo -E ...`.
        #[arg(long)]
        no_sudo: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
        args: Vec<String>,
    },
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
        Task::DstLint { manifest_path } => xtask::dst_lint::run(&manifest_path),
        Task::YamlFreeCli { manifest_path } => xtask::yaml_free_cli::run(&manifest_path),
        Task::BpfBuild => bpf_build(),
        Task::BpfClippy => bpf_clippy(),
        Task::BpfUnit => bpf_unit(),
        Task::IntegrationTest { scope } => match scope {
            IntegrationScope::Vm { cache_dir, kernels } => integration_vm(&cache_dir, &kernels),
        },
        Task::Mutants(args) => mutants(args),
        Task::Ci => ci(),
        Task::Lima { action } => lima(action),
        Task::Metal { action } => metal(action),
        Task::Hooks { action } => hooks(action),
        Task::Mcp { action } => mcp(action),
        Task::DevSetup => dev_setup(),
    }
}

/// One-shot developer bootstrap — installs the tools this workspace
/// depends on. Keep the list here in sync with `.config/nextest.toml`
/// and the install hints in `xtask::mutants` / `lefthook.yml`.
///
/// Phase coverage:
///
/// 1. `cargo-nextest` — workspace test runner.
/// 2. lefthook — git hook manager (probed; skipped if absent because
///    it cannot be installed via cargo).
/// 3. **bpf-build toolchain** (`bpf-linker`, `nightly` rustup
///    toolchain, `rust-src` component on nightly) — delegated to
///    [`xtask::dev_setup::run`] which is itself test-covered against
///    the four `ProbeContext` permutations. macOS short-circuits with a
///    warn per AC7 of step 02-03; Linux installs whatever is missing.
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

    // 3. bpf-build toolchain — bpf-linker, nightly rustup toolchain,
    //    rust-src component on nightly. Per ADR-0038 §4 / step 02-03 /
    //    upstream-issue A1; planning + execution split lives in
    //    `xtask::dev_setup` so the argv shapes are unit-testable.
    xtask::dev_setup::run()?;

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
        if let Ok(val) = std::env::var(key)
            && !val.is_empty()
        {
            return Ok(val);
        }
    }
    for key in keys {
        if let Some((_, val)) = env_file.iter().find(|(k, _)| k == key)
            && !val.is_empty()
        {
            return Ok(val.clone());
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

/// Returns `true` when the current process is running inside the Lima
/// guest VM. Lima names every guest `lima-<instance>`, so the hostname
/// prefix is a reliable signal that survives across shell sessions and
/// sudo escalation. The `OVERDRIVE_LIMA_VM` env var in the Lima
/// template (`infra/lima/overdrive-dev.yaml`) is the secondary signal
/// for newly-provisioned VMs.
fn inside_lima() -> bool {
    if std::env::var_os("OVERDRIVE_LIMA_VM").is_some() {
        return true;
    }
    std::fs::read_to_string("/etc/hostname").is_ok_and(|h| h.trim().starts_with("lima-"))
}

/// Returns `true` when the caller has provisioned the pinned BPF
/// toolchain (`BPF_NIGHTLY_TOOLCHAIN` + `rust-src` + `bpf-linker`)
/// directly on this host and wants `bpf-build` / `bpf-clippy` to
/// cross-compile here rather than re-dispatch into the Lima VM.
///
/// The BPF build is a pure `bpfel-unknown-none` cross-compile — it runs
/// nothing, and needs no kernel, KVM, or QEMU — so on a Linux host that
/// already has the toolchain (the CI `bpf-build` job) the Lima round-trip
/// is pure overhead, and specifically drags in the flaky apt-based QEMU
/// install inside `lima-vm/lima-actions/setup` that has repeatedly hung
/// the linchpin job to its timeout. Opt in by exporting
/// `OVERDRIVE_BPF_NATIVE=1`. Unset (every dev surface, macOS included)
/// preserves the transparent Lima dispatch.
fn bpf_build_native() -> bool {
    std::env::var_os("OVERDRIVE_BPF_NATIVE").is_some()
}

/// Ensures the current xtask subcommand is running inside the Lima VM.
/// If already inside (detected via `OVERDRIVE_LIMA_VM` env var), returns
/// `Ok(())` and the caller proceeds with the real work. If outside, re-
/// dispatches the given `args` through `cargo xtask lima run --` and
/// exits with the child's exit status.
fn ensure_in_lima(args: &[&str]) -> Result<()> {
    if inside_lima() {
        return Ok(());
    }
    let mut cmd_args: Vec<String> =
        vec!["cargo".into(), "xtask".into(), "lima".into(), "run".into(), "--".into()];
    cmd_args.extend(args.iter().map(|s| (*s).to_string()));
    lima(LimaAction::Run { no_sudo: false, args: cmd_args })
}

fn lima(action: LimaAction) -> Result<()> {
    // When already inside the Lima guest and the action is `Run`,
    // execute the command directly — `limactl` is not installed inside
    // the VM so the `which_or_hint` below would fail. This passthrough
    // lets cargo aliases (e.g. `cargo verifier-regress`) that route
    // through `xtask lima run --` work transparently from both the host
    // and from inside the VM.
    if let LimaAction::Run { no_sudo, ref args } = action
        && inside_lima()
    {
        if args.is_empty() {
            bail!("no command given; use `cargo xtask lima run -- cargo dst` etc.");
        }
        if no_sudo {
            return sh("(lima passthrough)", Command::new(&args[0]).args(&args[1..]));
        }
        return sh(
            "(lima passthrough, sudo)",
            Command::new("sudo")
                .args(["-E", "env"])
                .arg(format!("PATH={}", std::env::var("PATH").unwrap_or_default()))
                .arg(format!(
                    "CARGO_TARGET_DIR={}",
                    std::env::var("CARGO_TARGET_DIR").unwrap_or_default()
                ))
                .args(args),
        );
    }

    which_or_hint(
        "limactl",
        if cfg!(target_os = "macos") {
            "brew install lima"
        } else {
            "brew install lima  # or: curl -fsSL https://lima-vm.io/install.sh | bash"
        },
    )?;

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
                bail!("no command given; use `cargo xtask lima run -- cargo dst` etc.");
            }
            let mut cmd = Command::new("limactl");
            if no_sudo {
                cmd.args(["shell", LIMA_INSTANCE]).args(&args);
            } else {
                // Default: run the test process as root inside the VM
                // so cgroup writes and other privileged ops succeed —
                // the same permission shape CI's LVH harness uses.
                //
                // `env "HOME=$HOME" "PATH=$PATH"
                // "CARGO_TARGET_DIR=$CARGO_TARGET_DIR"` re-injects these
                // explicitly so cargo, rustup, and the target dir all
                // resolve under the `lima` user's home — where rustup,
                // the `nightly` toolchain, and the cargo registry cache
                // live. `HOME` is load-bearing and must NOT be left to
                // `sudo -E`: Ubuntu 26.04's sudoers refuses `-E`
                // ("preserving the entire environment is not supported,
                // '-E' is ignored"), so without the explicit
                // `HOME=$HOME` a root rustup resolves
                // `RUSTUP_HOME=/root/.rustup` and `rustup run nightly`
                // fails "toolchain not installed". `-E` is kept for the
                // older-sudo case where it still works; the explicit
                // `env` vars are the load-bearing path.
                let joined = args.iter().map(|a| sh_escape(a)).collect::<Vec<_>>().join(" ");
                let inner = format!(
                    r#"sudo -E env "HOME=$HOME" "PATH=$PATH" "CARGO_TARGET_DIR=$CARGO_TARGET_DIR" {joined}"#
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

/// Process-env / `.env` key naming the bare-metal test host (`user@host`).
const METAL_TARGET_ENV: &str = "OVERDRIVE_METAL_TARGET";
/// The one rsync definition — reused so metal `sync` and the `lima` VM
/// path never diverge on excludes.
const METAL_BOOTSTRAP: &str = "infra/metal/bootstrap.sh";
/// Resolve the bare-metal test host (`user@host`) from
/// `OVERDRIVE_METAL_TARGET` in the process environment, falling back to
/// `.env` at the workspace root. This is the native, nonvirtualized `x86_64`
/// hardware KVM box the Cloud-Hypervisor microVM `kvm-tests` run on
/// (see infra/metal/bootstrap.sh).
fn metal_target() -> Result<String> {
    let workspace_root = std::env::current_dir()?;
    let env_file = load_env_file(&workspace_root.join(".env"))?;
    lookup_required(
        &env_file,
        &[METAL_TARGET_ENV],
        "set the bare-metal test host, e.g. `export OVERDRIVE_METAL_TARGET=ubuntu@1.2.3.4` \
         or add it to `.env` (see .env.example). It is the x86_64 KVM box the \
         Cloud-Hypervisor `kvm-tests` run on — native nonvirtualized x86_64 hardware with KVM.",
    )
}

/// rsync the working tree to the metal box via the existing bootstrap
/// script's `--sync-only` mode, so metal and its excludes have exactly
/// one definition.
fn metal_sync(target: &str) -> Result<()> {
    sh(
        "metal sync (bootstrap.sh --sync-only)",
        Command::new("bash").args([METAL_BOOTSTRAP, target, "--sync-only"]),
    )
}

/// `cargo xtask metal …` — the metal sibling of `lima`. Resolves the
/// target host, then syncs / shells / runs over ssh. `run` rsyncs first
/// (unless `--no-sync`) and wraps in `sudo -E env "PATH=$PATH" …` by
/// default (unless `--no-sudo`) so the KVM / cgroup tests get root.
fn metal(action: MetalAction) -> Result<()> {
    which_or_hint(
        "ssh",
        "install an OpenSSH client (ships with macOS; `apt-get install openssh-client` on Linux)",
    )?;
    let target = metal_target()?;

    match action {
        MetalAction::Sync => metal_sync(&target),
        MetalAction::Shell => sh(
            "metal shell (bootstrap lease session)",
            Command::new("bash").args([METAL_BOOTSTRAP, &target, "--shell"]),
        ),
        MetalAction::Run { no_sync, no_sudo, args } => {
            if args.is_empty() {
                bail!(
                    "no command given; use `cargo xtask metal run -- cargo nextest run \
                     -p overdrive-cli --features integration-tests,kvm-tests`"
                );
            }
            let mut command = Command::new("bash");
            command.args([METAL_BOOTSTRAP, &target, "--run"]);
            if no_sync {
                command.arg("--no-sync");
            }
            if no_sudo {
                command.arg("--no-sudo");
            }
            command.arg("--").args(args);
            sh("metal run (one bootstrap lease session)", &mut command)
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
/// `target/bpf/overdrive_bpf.o` that the loader's
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
    // The kernel-side toolchain (nightly + `rust-src` + `bpf-linker` +
    // `bpfel-unknown-none` target) is provisioned in the Lima VM. Re-
    // dispatch through Lima so callers (lefthook, devs, CI) can invoke
    // `cargo xtask bpf-build` unconditionally — inside the VM the
    // guard returns Ok(()) and falls through to the direct path below.
    // `--no-sudo` because the build is unprivileged; the ELF copy lands
    // in the workspace's virtiofs-mounted `target/bpf/` owned by the
    // `lima` user. `OVERDRIVE_BPF_NATIVE=1` (see `bpf_build_native`) opts
    // out of the Lima dispatch to cross-compile on the host directly.
    if !inside_lima() && !bpf_build_native() {
        return lima(LimaAction::Run {
            no_sudo: true,
            args: vec!["cargo".into(), "xtask".into(), "bpf-build".into()],
        });
    }

    which_or_hint("bpf-linker", &bpf_linker_install_hint())?;

    let workspace_root = workspace_root_dir()?;
    let manifest = workspace_root.join("crates/overdrive-bpf/Cargo.toml");

    // Invoke through `rustup run <BPF_NIGHTLY_TOOLCHAIN> cargo …`
    // rather than the bare `cargo +nightly` form. The `$CARGO` env var
    // that `cargo()` resolves to is populated by cargo itself with the
    // direct cargo binary (not rustup's shim), and the direct
    // binary does not parse `+toolchain` directives. Going through
    // rustup is the canonical way to pin a non-default toolchain
    // when the parent process was launched by stable cargo (rustup
    // book § "Channels and Toolchain Specifiers"). The
    // `-Z build-std=core` flag requires nightly per
    // `wave-decisions.md` D3 / ADR-0038 §3.1; nightly is provisioned
    // alongside stable on the dev surfaces (Lima, dev-setup).
    //
    // The toolchain is the *dated* `BPF_NIGHTLY_TOOLCHAIN`, not the
    // floating `nightly` channel — see that constant's doc comment
    // for why (bpf-linker/rustc LLVM major-version skew).
    let nightly = xtask::BPF_NIGHTLY_TOOLCHAIN;
    sh(
        &format!("rustup run {nightly} cargo build (overdrive-bpf, bpfel-unknown-none)"),
        Command::new("rustup")
            .args([
                "run",
                nightly,
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
    let dst_dir = workspace_root.join("target/bpf");
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

/// Run clippy against `crates/overdrive-bpf` under the same toolchain
/// triple `bpf_build` uses (`+nightly`, `--target bpfel-unknown-none`,
/// `-Z build-std=core`, `--features build-bpf-target`). The kernel-side
/// `[[bin]]` is `#![no_std] #![no_main]`; the host workspace clippy
/// run cannot lint it (rustc rejects it on the host triple with
/// "unwinding panics are not supported without std"), so this is the
/// dedicated path.
///
/// `bpf-linker` is not strictly required for `cargo clippy` (no link
/// step), but the rest of the toolchain (nightly + rust-src) is — same
/// failure mode as `bpf_build` if missing.
fn bpf_clippy() -> Result<()> {
    // The kernel-side toolchain (nightly + `rust-src` + `bpf-linker` +
    // `bpfel-unknown-none` target) is provisioned in the Lima VM. Re-
    // dispatch through Lima so callers (lefthook, devs, CI) can invoke
    // `cargo xtask bpf-clippy` unconditionally — inside the VM the
    // guard returns Ok(()) and falls through to the direct path below.
    // `--no-sudo` because clippy is a build, not a privileged op.
    // `OVERDRIVE_BPF_NATIVE=1` (see `bpf_build_native`) opts out of the
    // Lima dispatch to lint on the host directly.
    if !inside_lima() && !bpf_build_native() {
        return lima(LimaAction::Run {
            no_sudo: true,
            args: vec!["cargo".into(), "xtask".into(), "bpf-clippy".into()],
        });
    }

    let workspace_root = workspace_root_dir()?;
    let manifest = workspace_root.join("crates/overdrive-bpf/Cargo.toml");

    // Pinned nightly — see `xtask::BPF_NIGHTLY_TOOLCHAIN`'s doc
    // comment (bpf-linker/rustc LLVM major-version skew).
    let nightly = xtask::BPF_NIGHTLY_TOOLCHAIN;
    sh(
        &format!("rustup run {nightly} cargo clippy (overdrive-bpf, bpfel-unknown-none)"),
        Command::new("rustup")
            .args([
                "run",
                nightly,
                "cargo",
                "clippy",
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
            .args(["--", "-D", "warnings"])
            .current_dir(&workspace_root),
    )?;

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

/// Tier 2 — invoke `cargo nextest run -p overdrive-bpf --features
/// integration-tests --test integration` to drive the
/// PKTGEN/SETUP/CHECK triptych under
/// `crates/overdrive-bpf/tests/integration/`.
///
/// Per architecture.md §6.1 / `.claude/rules/testing.md` § "Tier 2 —
/// BPF Unit Tests": each program ships a triptych that loads the BPF
/// object, drives `BPF_PROG_TEST_RUN` via aya, and asserts on
/// observable kernel side effects (verdict + map state).
///
/// The test target binary is named `integration` (the `tests/
/// integration.rs` entrypoint per § Layout convention); we pass it
/// explicitly via `--test integration` rather than the wildcard
/// `--test '*'` because nextest's CLI does not glob — the wildcard
/// would be passed verbatim and miss the binary. Architecture.md §6.1
/// notes the `--test '*'` shape mirrors the stub's documented intent;
/// the concrete invocation lands the binary name as the integration
/// suite's single entrypoint per the testing.md Layout convention.
///
fn bpf_unit() -> Result<()> {
    ensure_in_lima(&["cargo", "xtask", "bpf-unit"])?;

    which_or_hint(
        "cargo-nextest",
        "cargo install cargo-nextest --locked  # or: brew install cargo-nextest",
    )?;
    let workspace_root = workspace_root_dir()?;

    // Tier 2 BPF unit tests under
    // `crates/overdrive-bpf/tests/integration/xdp_service_map_redirect_neigh.rs`
    // call `bpf_fib_lookup` from the XDP program with input-mode
    // semantics (flags=0). For `RET_SUCCESS` the kernel needs both a
    // route AND a populated neighbour entry for the destination —
    // and rp_filter must be relaxed because `BPF_PROG_TEST_RUN`
    // synthesises an XDP context with `ingress_ifindex = lo`. Lima
    // happens to satisfy these via routine boot traffic on the
    // default route, so the suite passes there; CI runners
    // (`ubuntu-latest`, LVH) make no such guarantee.
    //
    // Install a deterministic FIB-hit topology before `nextest` runs
    // and tear it down on Drop so a panicking test still cleans up.
    // The topology lives once per `bpf-unit` invocation, not per
    // test (mirrors the env-wide nature of host network state, and
    // avoids per-test sudo dances).
    let _topology = bpf_fib_topology::install()?;

    sh(
        "cargo nextest run -p overdrive-bpf --features integration-tests --test integration",
        Command::new(cargo())
            .args([
                "nextest",
                "run",
                "-p",
                "overdrive-bpf",
                "--features",
                "integration-tests",
                "--test",
                "integration",
            ])
            .current_dir(&workspace_root),
    )
}

/// Tier 2 BPF unit-test FIB topology: a dummy interface with a
/// directly-connected `/32` route to the well-known backend IP +
/// permanent neighbour entry, so `bpf_fib_lookup` returns
/// `RET_SUCCESS` from the XDP program independent of host network
/// state. See `bpf_unit()` for the rationale.
///
/// Magic constants are duplicated by name in the consuming test
/// file (`crates/overdrive-bpf/tests/integration/
/// xdp_service_map_redirect_neigh.rs` — `ROUTABLE_BACKEND_OCTETS`
/// and the synthesised packet's source IP). xtask hard-codes the
/// dotted-quad strings rather than importing the test crate, per
/// the `overdrive-*`-out-of-xtask-deps rule (CLAUDE.md §
/// "xtask is build / test / dev orchestration"). If the test
/// constants ever change, this module changes alongside them.
mod bpf_fib_topology {
    use super::{Result, sh};
    use eyre::bail;
    use std::process::Command;

    // 15-char IFNAMSIZ ceiling; this is well under.
    const IFACE: &str = "odt-bpf-fib";
    // /24 covers the test's source IP (`10.0.0.100` in
    // `synthesise_tcp_syn`) so the input-mode FIB lookup has a
    // valid reverse-path candidate via this iface.
    const IFACE_ADDR_CIDR: &str = "10.0.0.254/24";
    // Matches `ROUTABLE_BACKEND_OCTETS = [10, 1, 0, 5]` in the
    // forward-path consuming test (`xdp_service_map_redirect_neigh`).
    // Outside the IFACE_ADDR_CIDR subnet, so it needs an explicit
    // `/32` route plus permanent neigh entry.
    const BACKEND_IP: &str = "10.1.0.5";
    // Matches `ROUTABLE_CLIENT_OCTETS = [10, 0, 0, 100]` in the
    // reverse-path consuming test (`xdp_reverse_nat_redirect_neigh`).
    // Inside the IFACE_ADDR_CIDR `/24` so the route is auto-added
    // (directly connected); only the permanent neigh entry is needed
    // (without it the FIB lookup gates on ARP and returns
    // RET_NO_NEIGH, falling through to XDP_PASS).
    const CLIENT_IP: &str = "10.0.0.100";
    // Synthetic next-hop MAC; the tests assert only that the
    // post-FIB-lookup `h_dest` differs from the PKTGEN sentinel,
    // so any well-formed unicast MAC works. Same for both neigh
    // entries — neither test inspects the value.
    const NEIGH_LLADDR: &str = "02:00:00:00:00:01";
    const RP_FILTER_ALL: &str = "/proc/sys/net/ipv4/conf/all/rp_filter";

    pub struct Topology {
        rp_filter_prior: Option<String>,
    }

    pub fn install() -> Result<Topology> {
        // Best-effort module load — already present on most modern
        // kernels including Lima's 6.8 image.
        let _ = Command::new("modprobe").arg("dummy").status();

        // Defensive cleanup: if a prior `bpf-unit` was killed
        // mid-run, the iface may still exist. Removing here is
        // idempotent.
        let _ = Command::new("ip").args(["link", "del", IFACE]).status();

        // `BPF_PROG_TEST_RUN` synthesises an XDP context with
        // `ingress_ifindex = lo`. Input-mode `bpf_fib_lookup` then
        // does a route lookup with `flowi4_iif = lo`; on kernels
        // with `all.rp_filter >= 1` (Ubuntu's default) the kernel
        // rejects the lookup because the source IP is not routable
        // back through `lo`. Drop the global to 0 for the duration
        // of the suite and restore on Drop. Lima's image already
        // ships with `all.rp_filter = 0`; this branch is the
        // CI-runner backstop.
        let rp_filter_prior = std::fs::read_to_string(RP_FILTER_ALL).ok();
        if let Err(err) = std::fs::write(RP_FILTER_ALL, "0\n") {
            bail!(
                "failed to relax {RP_FILTER_ALL} for FIB-hit topology: {err}; \
                 `cargo xtask bpf-unit` requires NET_ADMIN — invoke via \
                 `cargo xtask lima run -- cargo xtask bpf-unit` or `sudo` in CI"
            );
        }

        sh(
            &format!("ip link add {IFACE} type dummy"),
            Command::new("ip").args(["link", "add", IFACE, "type", "dummy"]),
        )?;
        sh(
            &format!("ip link set {IFACE} up"),
            Command::new("ip").args(["link", "set", IFACE, "up"]),
        )?;
        sh(
            &format!("ip addr add {IFACE_ADDR_CIDR} dev {IFACE}"),
            Command::new("ip").args(["addr", "add", IFACE_ADDR_CIDR, "dev", IFACE]),
        )?;
        sh(
            &format!("ip route add {BACKEND_IP}/32 dev {IFACE}"),
            Command::new("ip").args(["route", "add", &format!("{BACKEND_IP}/32"), "dev", IFACE]),
        )?;
        sh(
            &format!("ip neigh add {BACKEND_IP} lladdr {NEIGH_LLADDR} dev {IFACE} nud permanent"),
            Command::new("ip").args([
                "neigh",
                "add",
                BACKEND_IP,
                "lladdr",
                NEIGH_LLADDR,
                "dev",
                IFACE,
                "nud",
                "permanent",
            ]),
        )?;
        sh(
            &format!("ip neigh add {CLIENT_IP} lladdr {NEIGH_LLADDR} dev {IFACE} nud permanent"),
            Command::new("ip").args([
                "neigh",
                "add",
                CLIENT_IP,
                "lladdr",
                NEIGH_LLADDR,
                "dev",
                IFACE,
                "nud",
                "permanent",
            ]),
        )?;

        Ok(Topology { rp_filter_prior })
    }

    impl Drop for Topology {
        fn drop(&mut self) {
            // `ip link del` cascades the route and neigh entry.
            let _ = Command::new("ip").args(["link", "del", IFACE]).status();
            if let Some(prior) = &self.rp_filter_prior {
                let _ = std::fs::write(RP_FILTER_ALL, prior);
            }
        }
    }
}

fn integration_vm(cache_dir: &std::path::Path, kernels: &[String]) -> Result<()> {
    ensure_in_lima(&["cargo", "xtask", "integration-test", "vm"])?;

    if kernels.is_empty() {
        bail!("specify at least one kernel (e.g. 5.15, 6.1, 6.6, latest, bpf-next)");
    }
    // Placeholder — Tier 3 nested-VM kernel-matrix harness is queued
    // for issue #152 (split out of #23 during DELIVER per
    // `docs/feature/phase-2-aya-rs-scaffolding/deliver/upstream-issues.md`
    // § A3). The original architecture (architecture.md §6.2) wired
    // `cargo xtask integration-test vm latest` to LVH; that was
    // dropped from #23 because (a) for a no-op `xdp_pass` the real-
    // attach path adds zero coverage over Tier 2's
    // `BPF_PROG_TEST_RUN`, and (b) nested-VM machinery only earns
    // its keep when running against a kernel different from the
    // host environment, which is the deferred kernel-matrix scope.
    let summary = format!(
        "integration-test vm: nested-VM harness deferred to #152. cache={}, kernels={}",
        cache_dir.display(),
        kernels.join(",")
    );
    tracing_placeholder(&summary)
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
    // `--features integration-tests` (NOT `--all-features`) on the host:
    // `overdrive-bpf` declares `build-bpf-target` to gate its
    // `#![no_std] #![no_main]` kernel-side bin (see ADR-0038 §3.1 /
    // `cargo xtask bpf-build`). Enabling that feature on the host target
    // makes rustc reject the bin with "unwinding panics are not supported
    // without std". The dedicated `bpf-build` job exercises the kernel-side
    // compile path with the right toolchain. Mirrors the `fmt-clippy` job
    // in `.github/workflows/ci.yml`.
    sh(
        "cargo clippy",
        Command::new(cargo()).args([
            "clippy",
            "--workspace",
            "--all-targets",
            "--features",
            "integration-tests",
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

#[cfg(test)]
mod metal_qualification_tests {
    #![allow(clippy::doc_markdown)]

    use std::os::unix::fs::PermissionsExt as _;
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Duration;

    use tempfile::TempDir;

    fn workspace_file(relative: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(relative)
    }

    fn holder(
        lock: &std::path::Path,
        owner: &std::path::Path,
        token: &str,
        action: &str,
    ) -> Command {
        let mut command = Command::new("bash");
        command
            .arg(workspace_file("infra/metal/lease-holder.sh"))
            .arg(lock)
            .arg(owner)
            .arg("1")
            .arg(token)
            .args([action, "C-GTI-METAL-LEASE", "/workspace", "deadbeef"]);
        command
    }

    fn wait_for_owner(owner: &std::path::Path) {
        for _ in 0..100 {
            if owner.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("lease holder did not publish owner metadata");
    }

    /// CONTRACT_SHAPE: bounded-change (contention times out before mutation and cancellation releases).
    #[test]
    fn metal_run_sync_and_bootstrap_share_one_pre_mutation_lease() {
        let temp = TempDir::new().expect("temporary lease directory");
        let lock = temp.path().join("shared.lock");
        let owner = temp.path().join("shared.owner");
        let mutation = temp.path().join("must-not-exist");

        let mut first = holder(&lock, &owner, "first-token", "run")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("start first lease holder");
        wait_for_owner(&owner);
        let owner_metadata = std::fs::read_to_string(&owner).expect("owner metadata");
        for field in [
            "pid=",
            "started_at=",
            "action=run",
            "scenario=C-GTI-METAL-LEASE",
            "workspace=/workspace",
            "commit=deadbeef",
            "token=first-token",
        ] {
            assert!(owner_metadata.contains(field), "missing owner field {field}");
        }
        let fake_bin = temp.path().join("bin");
        let fake_home = temp.path().join("remote-home");
        std::fs::create_dir_all(&fake_bin).expect("fake command directory");
        std::fs::create_dir_all(&fake_home).expect("fake remote home");
        write_executable(
            &fake_bin.join("ssh"),
            r#"#!/usr/bin/env bash
set -euo pipefail
command="${!#}"
case "${command}" in
  'echo ok') exit 0 ;;
  'echo $HOME') printf '%s\n' "${OVERDRIVE_FAKE_REMOTE_HOME}"; exit 0 ;;
  'id -un') printf 'root\n'; exit 0 ;;
esac
if [[ "${command}" == lease_script=* ]]; then
  exec bash -c "${command}"
fi
printf 'mutation-before-lease\n' > "${OVERDRIVE_FAKE_MUTATION}"
exit 0
"#,
        );
        let inherited_path = std::env::var_os("PATH").expect("PATH");
        let mut fake_paths = vec![fake_bin];
        fake_paths.extend(std::env::split_paths(&inherited_path));
        let fake_path = std::env::join_paths(fake_paths).expect("fake PATH");
        for (action, args) in [
            ("run", vec!["fake@metal", "--run", "--", "true"]),
            ("sync", vec!["fake@metal", "--sync-only"]),
            ("bootstrap", vec!["fake@metal"]),
        ] {
            let blocked = Command::new("bash")
                .arg(workspace_file("infra/metal/bootstrap.sh"))
                .args(args)
                .env("PATH", &fake_path)
                .env("RSYNC_BIN", "/bin/true")
                .env("OVERDRIVE_METAL_LOCK_PATH", &lock)
                .env("OVERDRIVE_METAL_OWNER_PATH", &owner)
                .env("OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS", "1")
                .env("OVERDRIVE_FAKE_REMOTE_HOME", &fake_home)
                .env("OVERDRIVE_FAKE_MUTATION", &mutation)
                .output()
                .expect("execute canonical bootstrap writer route");
            assert_eq!(blocked.status.code(), Some(75), "{action} must time out at the same lock");
            let diagnostic = String::from_utf8(blocked.stderr).expect("UTF-8 diagnostic");
            assert!(diagnostic.contains("timed out after 1s"));
            assert!(diagnostic.contains("token=first-token"));
            assert!(!mutation.exists(), "a timed-out {action} writer must perform no mutation");
        }

        Command::new("kill")
            .args(["-TERM", &first.id().to_string()])
            .status()
            .expect("signal first holder");
        let _ = first.wait().expect("reap first holder");
        assert!(!owner.exists(), "signal cancellation must remove this lease's metadata");

        let reacquired = holder(&lock, &owner, "replacement-token", "bootstrap")
            .stdin(Stdio::null())
            .output()
            .expect("reacquire after cancellation");
        assert!(reacquired.status.success());
    }

    fn write_executable(path: &std::path::Path, body: &str) {
        std::fs::write(path, body).expect("write executable fixture");
        let mut permissions = std::fs::metadata(path).expect("stat fixture").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("chmod fixture");
    }

    fn preflight_command(root: &std::path::Path) -> Command {
        let mut command = Command::new("bash");
        command
            .arg(workspace_file("infra/metal/native-preflight.sh"))
            .env("OVERDRIVE_PREFLIGHT_ARCH", "x86_64")
            .env("OVERDRIVE_PREFLIGHT_CPUINFO", root.join("cpuinfo"))
            .env("OVERDRIVE_PREFLIGHT_HYPERVISOR_TYPE", root.join("hypervisor-type"))
            .env("OVERDRIVE_PREFLIGHT_KVM_DEVICE", root.join("kvm"))
            .env("OVERDRIVE_PREFLIGHT_KVM_CHARACTER", "yes")
            .env("OVERDRIVE_PREFLIGHT_KVM_PROBE", root.join("kvm-ok"))
            .env("OVERDRIVE_PREFLIGHT_CGROUP_CONTROLLERS", root.join("cgroup.controllers"))
            .env("OVERDRIVE_PREFLIGHT_DETECT_VIRT", root.join("detect-none"))
            .env("OVERDRIVE_PREFLIGHT_CLOUD_HYPERVISOR", root.join("cloud-hypervisor"))
            .env("OVERDRIVE_METAL_KERNEL", root.join("kernel"))
            .env("OVERDRIVE_METAL_ROOTFS", root.join("rootfs"))
            .env("OVERDRIVE_METAL_OWNER_PATH", root.join("owner"))
            .env("OVERDRIVE_EXPECTED_TOKEN", "fixture-token")
            .env("OVERDRIVE_EXPECTED_COMMIT", "deadbeef")
            .env("OVERDRIVE_EXPECTED_WORKSPACE", "/workspace")
            .env("OVERDRIVE_EXPECTED_SOURCE", "source-digest")
            .env("OVERDRIVE_REMOTE_DIR", root.join("remote"));
        command
    }

    fn assert_preflight_failure(mut command: Command, diagnostic: &str) {
        let output = command.output().expect("execute native preflight");
        assert!(!output.status.success(), "partition unexpectedly passed: {diagnostic}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(diagnostic),
            "missing diagnostic {diagnostic:?}: {}",
            String::from_utf8_lossy(&output.stderr),
        );
    }

    fn assert_artifact_preflight_partitions(root: &std::path::Path) {
        let mut unset_kernel = preflight_command(root);
        unset_kernel.env_remove("OVERDRIVE_METAL_KERNEL");
        assert_preflight_failure(unset_kernel, "selected guest kernel is required");
        let mut empty_kernel = preflight_command(root);
        empty_kernel.env("OVERDRIVE_METAL_KERNEL", "");
        assert_preflight_failure(empty_kernel, "selected guest kernel is required");
        let mut unset_rootfs = preflight_command(root);
        unset_rootfs.env_remove("OVERDRIVE_METAL_ROOTFS");
        assert_preflight_failure(unset_rootfs, "selected guest rootfs is required");
        let mut empty_rootfs = preflight_command(root);
        empty_rootfs.env("OVERDRIVE_METAL_ROOTFS", "");
        assert_preflight_failure(empty_rootfs, "selected guest rootfs is required");

        std::fs::create_dir(root.join("kernel-directory")).expect("kernel directory fixture");
        let mut kernel_directory = preflight_command(root);
        kernel_directory.env("OVERDRIVE_METAL_KERNEL", root.join("kernel-directory"));
        assert_preflight_failure(
            kernel_directory,
            "selected guest kernel must be a readable regular file",
        );
        std::fs::create_dir(root.join("rootfs-directory")).expect("rootfs directory fixture");
        let mut rootfs_directory = preflight_command(root);
        rootfs_directory.env("OVERDRIVE_METAL_ROOTFS", root.join("rootfs-directory"));
        assert_preflight_failure(
            rootfs_directory,
            "selected guest rootfs must be a readable regular file",
        );

        for (variable, path, diagnostic) in [
            (
                "OVERDRIVE_METAL_KERNEL",
                "absent-kernel",
                "selected guest kernel must be a readable regular file",
            ),
            (
                "OVERDRIVE_METAL_ROOTFS",
                "absent-rootfs",
                "selected guest rootfs must be a readable regular file",
            ),
        ] {
            let mut missing = preflight_command(root);
            missing.env(variable, root.join(path));
            assert_preflight_failure(missing, diagnostic);
        }
    }

    /// CONTRACT_SHAPE: bounded-change (native preflight rejects every absent or virtualized signal).
    #[test]
    fn metal_run_rejects_virtualized_or_unusable_kvm_hosts() {
        let temp = TempDir::new().expect("preflight fixture");
        let root = temp.path();
        std::fs::write(root.join("cpuinfo"), "flags : fpu vmx sse\n").expect("cpuinfo");
        std::fs::write(root.join("hypervisor-type"), "").expect("hypervisor type");
        for file in ["kvm", "cgroup.controllers", "kernel", "rootfs"] {
            std::fs::write(root.join(file), "fixture").expect("fixture file");
        }
        std::fs::create_dir(root.join("remote")).expect("remote dir");
        std::fs::write(root.join("owner"), "token=fixture-token\n").expect("owner");
        std::fs::write(
            root.join("remote/.overdrive-metal-source"),
            "commit=deadbeef\nworkspace=/workspace\nsource_digest=source-digest\n",
        )
        .expect("source marker");
        write_executable(&root.join("detect-none"), "#!/bin/sh\necho none\nexit 1\n");
        write_executable(&root.join("detect-virt"), "#!/bin/sh\necho kvm\nexit 0\n");
        write_executable(&root.join("kvm-ok"), "#!/bin/sh\nexit 0\n");
        write_executable(
            &root.join("kvm-permission-denied"),
            "#!/bin/sh\necho 'KVM open permission denied' >&2\nexit 1\n",
        );
        write_executable(
            &root.join("kvm-api-mismatch"),
            "#!/bin/sh\necho 'KVM API version mismatch' >&2\nexit 1\n",
        );
        write_executable(
            &root.join("kvm-create-failed"),
            "#!/bin/sh\necho 'KVM create VM failed' >&2\nexit 1\n",
        );
        write_executable(&root.join("cloud-hypervisor"), "#!/bin/sh\nexit 0\n");

        assert!(preflight_command(root).status().expect("baseline preflight").success());
        assert_artifact_preflight_partitions(root);

        let mut arch = preflight_command(root);
        arch.env("OVERDRIVE_PREFLIGHT_ARCH", "aarch64");
        assert_preflight_failure(arch, "architecture must be literal x86_64");
        let mut detector_missing = preflight_command(root);
        detector_missing.env("OVERDRIVE_PREFLIGHT_DETECT_VIRT", root.join("absent-detector"));
        assert_preflight_failure(detector_missing, "systemd-detect-virt is required");
        let mut virtualized = preflight_command(root);
        virtualized.env("OVERDRIVE_PREFLIGHT_DETECT_VIRT", root.join("detect-virt"));
        assert_preflight_failure(virtualized, "host reports virtualization");

        std::fs::write(root.join("cpu-hypervisor"), "flags : vmx hypervisor\n").expect("cpu");
        let mut hypervisor_flag = preflight_command(root);
        hypervisor_flag.env("OVERDRIVE_PREFLIGHT_CPUINFO", root.join("cpu-hypervisor"));
        assert_preflight_failure(hypervisor_flag, "CPU hypervisor flag is present");
        std::fs::write(root.join("hypervisor-present"), "kvm\n").expect("hypervisor");
        let mut hypervisor_type = preflight_command(root);
        hypervisor_type.env("OVERDRIVE_PREFLIGHT_HYPERVISOR_TYPE", root.join("hypervisor-present"));
        assert_preflight_failure(hypervisor_type, "/sys/hypervisor/type reports a hypervisor");
        std::fs::write(root.join("cpu-no-kvm"), "flags : fpu sse\n").expect("cpu");
        let mut no_cpu_kvm = preflight_command(root);
        no_cpu_kvm.env("OVERDRIVE_PREFLIGHT_CPUINFO", root.join("cpu-no-kvm"));
        assert_preflight_failure(no_cpu_kvm, "CPU exposes neither vmx nor svm");
        let mut not_character = preflight_command(root);
        not_character.env_remove("OVERDRIVE_PREFLIGHT_KVM_CHARACTER");
        assert_preflight_failure(not_character, "/dev/kvm is not a character device");
        for (probe, diagnostic) in [
            ("kvm-permission-denied", "KVM open permission denied"),
            ("kvm-api-mismatch", "KVM API version mismatch"),
            ("kvm-create-failed", "KVM create VM failed"),
        ] {
            let mut bad_kvm = preflight_command(root);
            bad_kvm.env("OVERDRIVE_PREFLIGHT_KVM_PROBE", root.join(probe));
            assert_preflight_failure(bad_kvm, diagnostic);
        }
        let mut no_cgroup = preflight_command(root);
        no_cgroup.env("OVERDRIVE_PREFLIGHT_CGROUP_CONTROLLERS", root.join("absent-cgroup"));
        assert_preflight_failure(no_cgroup, "cgroup v2 controllers are unavailable");
        let mut no_cloud_hypervisor = preflight_command(root);
        no_cloud_hypervisor.env("OVERDRIVE_PREFLIGHT_CLOUD_HYPERVISOR", root.join("absent-cloud"));
        assert_preflight_failure(no_cloud_hypervisor, "cloud-hypervisor is unavailable");
        let mut wrong_owner = preflight_command(root);
        wrong_owner.env("OVERDRIVE_EXPECTED_TOKEN", "wrong-token");
        assert_preflight_failure(wrong_owner, "the active lease owner token changed");
        let mut stale_source = preflight_command(root);
        stale_source.env("OVERDRIVE_EXPECTED_SOURCE", "wrong-source");
        assert_preflight_failure(stale_source, "runtime source marker is stale or mismatched");
    }
}
