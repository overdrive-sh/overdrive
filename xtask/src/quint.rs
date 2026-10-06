//! `cargo xtask quint` — typecheck and model-check the Quint specs under
//! `specs/quint/<subsystem>/`.
//!
//! Each subsystem directory may carry a `checks.toml` listing the model
//! checks to run:
//!
//! ```toml
//! [[check]]
//! name = "owner-one-session-per-cid"
//! spec = "owner_flows.qnt"          # relative to the subsystem dir
//! main = "flows_ok"
//! invariant = "Safety"              # or temporal = "<prop>"
//! backend = "apalache"              # "apalache" | "tlc"
//! max_steps = 14                    # apalache only
//! expect = "holds"                  # "holds" | "violation"
//! ci = true                         # run under --ci
//! timeout_secs = 1800               # optional; default DEFAULT_TIMEOUT_SECS
//! ```
//!
//! `expect = "holds"` passes only on a clean result; `expect = "violation"`
//! passes only when the checker reports a counterexample. A tool error
//! (parse / type / name-resolution failure, Apalache or TLC crash, an
//! unrecognised result) or a timeout fails the check regardless of
//! `expect` — neither is ever mistaken for a violation.
//!
//! The runner owns process lifetime, timeouts and parallelism so callers
//! never script their own watchdogs:
//!
//! - **Process groups.** Each check runs in its own process group (Quint
//!   plus the Apalache / TLC JVM it starts). The group is killed when the
//!   check ends on any path — clean exit, timeout, start-up hang, runner
//!   error, or SIGINT / SIGTERM / SIGHUP of xtask. After the run the runner
//!   verifies that no process tagged with this run's id (an inherited
//!   environment marker) remains, and at start it
//!   sweeps processes left by a runner that was itself `SIGKILL`ed.
//! - **Timeouts.** Each attempt has a wall-clock budget (`--timeout`, else
//!   the check's `timeout_secs`, else [`DEFAULT_TIMEOUT_SECS`]); exceeding it
//!   yields [`Outcome::TimedOut`].
//! - **Server start-up hangs.** Quint compiles every check (both backends)
//!   through an Apalache server it starts on a private port. If Apalache has
//!   not begun its first pass ([`started_work`]) within
//!   [`STARTUP_BOUND_SECS`] — the server never came up, or came up but Quint
//!   never handed it the job — the attempt is killed and retried once on a
//!   fresh port; a second hang is a tool error naming the cause. Quint
//!   exiting after starting the server but before the first pass (a gRPC
//!   handshake failure under load) is the same failure and gets the same
//!   single retry. A server
//!   that is merely listening is not progress: the observed hang leaves the
//!   server up and idle.
//! - **Parallelism.** `--jobs N` checks run concurrently (default
//!   [`default_jobs`]), each with a private server port, a bounded Apalache
//!   server heap ([`JVM_HEAP_MB`]) and a private `TMPDIR` / `java.io.tmpdir`
//!   (which also holds TLC's state queue) under `target/quint/` — never
//!   `/tmp`, a RAM disk in the Lima VM. Quint itself launches TLC with a
//!   fixed `-Xmx8G -workers auto`, which no environment setting overrides.
//!
//! Every attempt writes its full checker log to
//! `target/quint/<subsystem>/<check>/attempt-<n>.log` (attempts are never
//! overwritten within a run), an append-only `events.log` records each
//! attempt's start, end, kill and verdict, and an Apalache counterexample's
//! ITF trace lands beside them. Each subsystem gets a
//! `target/quint/<subsystem>/summary.json`.
//!
//! `--record` (one full subsystem only) additionally replaces
//! `specs/quint/<subsystem>/evidence/` with the evidence of this run — see
//! [`evidence`] — and `cargo xtask quint verify-evidence` fails when that
//! evidence is stale against the current specs or records a missed `expect`.
//! Nothing else writes `evidence/`.
//!
//! The pure parts — [`parse_checks`], [`classify`] / [`judge`],
//! [`started_work`], [`deadline_check`] / [`attempt_verdict`], [`effective_timeout`],
//! [`default_jobs`], [`estimate_secs`] and [`parse_stat_state`] — are unit-tested here;
//! [`typecheck`] and [`check`] shell out to `quint`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::net::TcpListener;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use color_eyre::eyre::{Context, Result, bail, eyre};
use serde::{Deserialize, Serialize};

pub mod evidence;
pub mod trace;

/// Root of the Quint spec tree, relative to the workspace root.
pub const SPECS_DIR: &str = "specs/quint";
/// Per-subsystem check manifest file name.
pub const CHECKS_FILE: &str = "checks.toml";
/// Output root for logs and traces, relative to the workspace root.
pub const OUT_DIR: &str = "target/quint";

/// Marker Quint prints on a clean `verify` result (both backends).
const OK_MARKER: &str = "[ok] No violation found";
/// Marker Quint prints when the checker found a counterexample.
const VIOLATION_MARKER: &str = "error: found a counterexample";
/// Line Apalache prints as its server starts (before it is handed a job).
const SERVER_START_MARKER: &str = "Starting checker server on port";
/// Prefix of the line Apalache prints when it begins its first pass on a
/// check — the earliest evidence the server received and started the job.
const FIRST_PASS_MARKER: &str = "PASS #0:";

/// Per-attempt wall-clock budget when neither `--timeout` nor the check's
/// `timeout_secs` sets one.
pub const DEFAULT_TIMEOUT_SECS: u64 = 900;
/// How long the Apalache server may take to become ready before the attempt
/// counts as a start-up hang.
pub const STARTUP_BOUND_SECS: u64 = 120;
/// Maximum heap of each check's Apalache server JVM (Apalache's own
/// default). TLC's JVM heap is fixed by Quint's command line.
pub const JVM_HEAP_MB: u64 = 4096;
/// Memory budgeted per concurrent check: the JVM heap plus headroom for Z3,
/// JVM metaspace and Quint itself.
pub const PER_CHECK_MEM_MB: u64 = JVM_HEAP_MB + 1024;
/// Upper bound on the default `--jobs`.
pub const MAX_DEFAULT_JOBS: usize = 4;
/// Environment variable carrying the run id into every child process, so
/// leftovers can be found even after re-parenting.
const RUN_MARKER_ENV: &str = "OVERDRIVE_XTASK_QUINT_RUN";
/// Supervisor poll interval.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Errors from reading and validating a `checks.toml`.
#[derive(Debug, thiserror::Error)]
pub enum ChecksError {
    /// The manifest is not valid TOML or does not match the schema.
    #[error("{path}: invalid checks manifest: {source}")]
    Parse {
        /// Manifest path.
        path: PathBuf,
        /// Underlying TOML error.
        #[source]
        source: toml::de::Error,
    },
    /// A `[[check]]` entry is structurally valid TOML but violates a rule.
    #[error("{path}: check `{name}`: {reason}")]
    InvalidCheck {
        /// Manifest path.
        path: PathBuf,
        /// The offending check's name.
        name: String,
        /// What is wrong with it.
        reason: String,
    },
}

/// Model-checking backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// Bounded symbolic model checking (`quint verify`, the default).
    Apalache,
    /// Exhaustive explicit-state model checking (`quint verify --backend tlc`).
    Tlc,
}

impl Backend {
    /// Lowercase label, as written in `checks.toml` and passed to `--backend`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Apalache => "apalache",
            Self::Tlc => "tlc",
        }
    }
}

/// The result the check author expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Expect {
    /// The property holds (no counterexample within the search).
    Holds,
    /// The checker must produce a counterexample (witness / hazard check).
    Violation,
}

impl Expect {
    /// Lowercase label, as written in `checks.toml`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Holds => "holds",
            Self::Violation => "violation",
        }
    }
}

/// The property a check verifies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Property {
    /// A state invariant (`--invariant`).
    Invariant(String),
    /// A temporal property (`--temporal`).
    Temporal(String),
}

/// One validated `[[check]]` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// Unique (per manifest) check name; also the output directory name.
    pub name: String,
    /// Spec file, relative to the subsystem directory.
    pub spec: PathBuf,
    /// Main module (`--main`).
    pub main: String,
    /// Property to check.
    pub property: Property,
    /// Checker backend.
    pub backend: Backend,
    /// Apalache step bound; `None` uses Quint's default. Always `None` for TLC.
    pub max_steps: Option<u32>,
    /// Expected outcome.
    pub expect: Expect,
    /// Whether `--ci` selects this check.
    pub ci: bool,
    /// Per-attempt wall-clock budget in seconds; `None` uses the default.
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    #[serde(default)]
    check: Vec<RawCheck>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCheck {
    name: String,
    spec: PathBuf,
    main: String,
    invariant: Option<String>,
    temporal: Option<String>,
    backend: Backend,
    max_steps: Option<u32>,
    expect: Expect,
    #[serde(default)]
    ci: bool,
    timeout_secs: Option<u64>,
}

/// Parse and validate a `checks.toml` body. `path` is used for error
/// messages only.
pub fn parse_checks(path: &Path, body: &str) -> Result<Vec<Check>, ChecksError> {
    let raw: RawManifest = toml::from_str(body)
        .map_err(|source| ChecksError::Parse { path: path.to_path_buf(), source })?;
    let invalid = |name: &str, reason: &str| ChecksError::InvalidCheck {
        path: path.to_path_buf(),
        name: name.to_owned(),
        reason: reason.to_owned(),
    };

    let mut checks: Vec<Check> = Vec::with_capacity(raw.check.len());
    for c in raw.check {
        if c.name.is_empty()
            || !c.name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        {
            return Err(invalid(
                &c.name,
                "name must be non-empty [A-Za-z0-9_-] (it names the output directory)",
            ));
        }
        if checks.iter().any(|existing| existing.name == c.name) {
            return Err(invalid(&c.name, "duplicate check name"));
        }
        if c.spec.is_absolute()
            || c.spec.components().any(|comp| matches!(comp, std::path::Component::ParentDir))
        {
            return Err(invalid(
                &c.name,
                "spec must be a relative path inside the subsystem directory",
            ));
        }
        let property = match (c.invariant, c.temporal) {
            (Some(inv), None) => Property::Invariant(inv),
            (None, Some(temp)) => Property::Temporal(temp),
            (Some(_), Some(_)) => {
                return Err(invalid(
                    &c.name,
                    "set exactly one of `invariant` or `temporal`, not both",
                ));
            }
            (None, None) => {
                return Err(invalid(&c.name, "set exactly one of `invariant` or `temporal`"));
            }
        };
        match c.backend {
            Backend::Tlc if c.max_steps.is_some() => {
                return Err(invalid(
                    &c.name,
                    "`max_steps` applies to backend = \"apalache\" only (TLC is exhaustive)",
                ));
            }
            // Quint 0.32 prompts interactively ("proceed with Apalache
            // anyway? (y/N)") for temporal properties on Apalache and
            // reports no result non-interactively.
            Backend::Apalache if matches!(property, Property::Temporal(_)) => {
                return Err(invalid(&c.name, "temporal properties require backend = \"tlc\""));
            }
            Backend::Apalache | Backend::Tlc => {}
        }
        if c.timeout_secs == Some(0) {
            return Err(invalid(&c.name, "`timeout_secs` must be at least 1"));
        }
        checks.push(Check {
            name: c.name,
            spec: c.spec,
            main: c.main,
            property,
            backend: c.backend,
            max_steps: c.max_steps,
            expect: c.expect,
            ci: c.ci,
            timeout_secs: c.timeout_secs,
        });
    }
    Ok(checks)
}

/// What a check's run produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Clean result: no counterexample within the search.
    Holds,
    /// The checker reported a counterexample.
    Violation,
    /// Anything else — a parse/type/name error, a checker crash, output
    /// the runner does not recognise, or a repeated server start-up hang.
    ToolError,
    /// The attempt exceeded its wall-clock budget and was killed.
    TimedOut,
    /// The runner was interrupted (SIGINT / SIGTERM / SIGHUP) before the
    /// check finished.
    Interrupted,
}

impl Outcome {
    /// Lowercase label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Holds => "holds",
            Self::Violation => "violation",
            Self::ToolError => "tool-error",
            Self::TimedOut => "timed-out",
            Self::Interrupted => "interrupted",
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Classify a `quint verify` run from its exit status and combined output.
///
/// Quint exits 0 on a clean result and 1 on BOTH a counterexample and a tool
/// error, so the exit code alone cannot separate them. A result counts only
/// with positive evidence: `Holds` needs exit 0 AND the `[ok]` marker (a
/// zero exit without it — e.g. an unanswered interactive prompt — is a tool
/// error); `Violation` needs a non-zero exit AND the counterexample marker.
pub fn classify(exit_success: bool, output: &str) -> Outcome {
    if exit_success {
        if output.contains(OK_MARKER) { Outcome::Holds } else { Outcome::ToolError }
    } else if output.contains(VIOLATION_MARKER) {
        Outcome::Violation
    } else {
        Outcome::ToolError
    }
}

/// Whether `outcome` satisfies `expect`. A tool error, timeout or
/// interruption never passes.
pub const fn judge(expect: Expect, outcome: Outcome) -> bool {
    matches!(
        (expect, outcome),
        (Expect::Holds, Outcome::Holds) | (Expect::Violation, Outcome::Violation)
    )
}

/// Locate the workspace root by walking up from the current directory to the
/// first `Cargo.toml` declaring `[workspace]`.
fn workspace_root() -> Result<PathBuf> {
    let cwd = std::env::current_dir().wrap_err("reading current directory")?;
    for dir in cwd.ancestors() {
        let manifest = dir.join("Cargo.toml");
        if manifest.is_file()
            && fs::read_to_string(&manifest)
                .wrap_err_with(|| format!("reading {}", manifest.display()))?
                .lines()
                .any(|l| l.trim() == "[workspace]")
        {
            return Ok(dir.to_path_buf());
        }
    }
    bail!("no workspace Cargo.toml found above {}", cwd.display())
}

/// Recursively collect `*.qnt` files under `dir`, skipping hidden and
/// `_`-prefixed directories (tool caches such as `.tools/`, `_apalache-out/`).
fn collect_qnt(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .wrap_err_with(|| format!("reading {}", dir.display()))?
        .collect::<std::io::Result<_>>()
        .wrap_err_with(|| format!("reading {}", dir.display()))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let file_type = entry.file_type().wrap_err_with(|| format!("stat {}", path.display()))?;
        if file_type.is_dir() {
            if !name.starts_with('.') && !name.starts_with('_') {
                collect_qnt(&path, out)?;
            }
        } else if path.extension().is_some_and(|e| e == "qnt") {
            out.push(path);
        }
    }
    Ok(())
}

fn quint_command() -> Command {
    Command::new("quint")
}

fn spawn_error(err: &std::io::Error) -> color_eyre::eyre::Report {
    if err.kind() == std::io::ErrorKind::NotFound {
        eyre!(
            "`quint` not found on PATH — the Lima VM provisions it \
             (infra/lima/overdrive-dev.yaml); CI installs it via \
             infra/provision/install-quint.sh. Run via `cargo xtask lima run -- cargo xtask quint …`"
        )
    } else {
        eyre!("spawning quint: {err}")
    }
}

/// `cargo xtask quint typecheck` — typecheck every `specs/quint/**/*.qnt`.
pub fn typecheck() -> Result<()> {
    let root = workspace_root()?;
    let specs = root.join(SPECS_DIR);
    if !specs.is_dir() {
        eprintln!("xtask quint typecheck: no {SPECS_DIR}/ directory; nothing to typecheck");
        return Ok(());
    }
    let mut files = Vec::new();
    collect_qnt(&specs, &mut files)?;
    let mut failed = Vec::new();
    for file in &files {
        let rel = file.strip_prefix(&root).unwrap_or(file);
        let output = quint_command()
            .arg("typecheck")
            .arg(file)
            .current_dir(&root)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| spawn_error(&e))?;
        if output.status.success() {
            eprintln!("  ok    {}", rel.display());
        } else {
            eprintln!("  FAIL  {}", rel.display());
            eprintln!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            failed.push(rel.display().to_string());
        }
    }
    if failed.is_empty() {
        eprintln!("xtask quint typecheck: {} file(s) typecheck", files.len());
        Ok(())
    } else {
        bail!(
            "{} of {} Quint file(s) failed to typecheck: {}",
            failed.len(),
            files.len(),
            failed.join(", ")
        )
    }
}

/// Selection filters for [`check`].
#[derive(Debug, Clone, Default)]
pub struct CheckFilter {
    /// Only checks marked `ci = true`.
    pub ci: bool,
    /// Only this subsystem: a directory name under `specs/quint/`, or a
    /// path (containing `/`) to a subsystem directory elsewhere.
    pub subsystem: Option<String>,
    /// Only the check with this name.
    pub name: Option<String>,
}

/// Execution options for [`check`].
#[derive(Debug, Clone, Copy, Default)]
pub struct CheckOptions {
    /// Concurrent checks; `None` uses [`default_jobs`].
    pub jobs: Option<usize>,
    /// Per-attempt timeout override in seconds (wins over `timeout_secs`).
    pub timeout_secs: Option<u64>,
    /// Replace the subsystem's `evidence/` with this run's evidence.
    pub record: bool,
}

/// The wall-clock budget of one attempt: the `--timeout` override, else the
/// check's `timeout_secs`, else [`DEFAULT_TIMEOUT_SECS`].
pub fn effective_timeout(cli: Option<u64>, check: Option<u64>) -> Duration {
    Duration::from_secs(cli.or(check).unwrap_or(DEFAULT_TIMEOUT_SECS))
}

/// Default concurrency for `--jobs`.
///
/// Half the CPUs (one JVM keeps about two cores busy), capped at
/// [`MAX_DEFAULT_JOBS`] and by how many [`PER_CHECK_MEM_MB`] budgets fit in
/// total memory (when known). Always at least 1.
pub fn default_jobs(cpus: usize, mem_total_mb: Option<u64>) -> usize {
    let by_cpu = cpus / 2;
    let by_mem = mem_total_mb
        .map_or(usize::MAX, |mb| usize::try_from(mb / PER_CHECK_MEM_MB).unwrap_or(usize::MAX));
    by_cpu.min(by_mem).clamp(1, MAX_DEFAULT_JOBS)
}

/// How a running attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptEnd {
    /// Quint exited on its own.
    Exited {
        /// Whether the exit status was zero.
        success: bool,
    },
    /// The attempt exceeded its wall-clock budget and was killed.
    TimedOut,
    /// Apalache did not begin its first pass within the start-up bound; the
    /// attempt was killed.
    StartupHang,
}

/// Whether a check's output shows Apalache began work on it.
pub fn started_work(output: &str) -> bool {
    output.lines().any(|l| l.starts_with(FIRST_PASS_MARKER))
}

/// Decide whether a still-running attempt must be killed. The overall
/// timeout takes precedence over the start-up bound.
pub fn deadline_check(
    elapsed: Duration,
    started: bool,
    timeout: Duration,
    startup_bound: Duration,
) -> Option<AttemptEnd> {
    if elapsed >= timeout {
        Some(AttemptEnd::TimedOut)
    } else if !started && elapsed >= startup_bound {
        Some(AttemptEnd::StartupHang)
    } else {
        None
    }
}

/// What to do after an attempt ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptVerdict {
    /// The check is finished with this outcome.
    Final {
        /// The check's outcome.
        outcome: Outcome,
        /// Human-readable cause for a non-result outcome.
        cause: Option<String>,
    },
    /// Retry the check on a fresh server port.
    Retry {
        /// Why the attempt was abandoned.
        cause: String,
    },
}

/// Turn an ended attempt (1-based `attempt`) into a verdict.
///
/// A start-up failure — a hang, or Quint exiting after starting its server but before
/// the first pass — is retried exactly once; a second one is a tool error.
pub fn attempt_verdict(
    attempt: u32,
    end: AttemptEnd,
    output: &str,
    timeout: Duration,
    startup_bound: Duration,
) -> AttemptVerdict {
    match end {
        AttemptEnd::Exited { success } => {
            let outcome = classify(success, output);
            // Quint started its Apalache server but failed before the server
            // began the job (e.g. a gRPC reflection DEADLINE_EXCEEDED under
            // load): the same start-up failure as a hang, so it gets the same
            // single retry. A spec error never gets as far as the server.
            if outcome == Outcome::ToolError
                && output.contains(SERVER_START_MARKER)
                && !started_work(output)
            {
                return if attempt <= 1 {
                    AttemptVerdict::Retry {
                        cause: "Apalache server started but Quint failed before the first pass; \
                                retrying on a fresh port"
                            .to_owned(),
                    }
                } else {
                    AttemptVerdict::Final {
                        outcome,
                        cause: Some(format!(
                            "Apalache server start-up failed before the first pass on {attempt} \
                             attempts (fresh port each); see the attempt logs"
                        )),
                    }
                };
            }
            let cause = (outcome == Outcome::ToolError).then(|| {
                format!(
                    "quint exited {} without a recognised result (see the attempt log)",
                    if success { "0" } else { "non-zero" }
                )
            });
            AttemptVerdict::Final { outcome, cause }
        }
        AttemptEnd::TimedOut => AttemptVerdict::Final {
            outcome: Outcome::TimedOut,
            cause: Some(format!(
                "exceeded the {}s timeout; process group killed",
                timeout.as_secs()
            )),
        },
        AttemptEnd::StartupHang if attempt <= 1 => AttemptVerdict::Retry {
            cause: format!(
                "Apalache server not ready (no first pass) within {}s; process group killed, \
                 retrying on a fresh port",
                startup_bound.as_secs()
            ),
        },
        AttemptEnd::StartupHang => AttemptVerdict::Final {
            outcome: Outcome::ToolError,
            cause: Some(format!(
                "Apalache server never became ready (no first pass) within {}s on {attempt} \
                 attempts (fresh port each)",
                startup_bound.as_secs()
            )),
        },
    }
}

/// Estimated wall-clock seconds for a run, plus the count without history.
///
/// `durations` are previous per-check seconds (`None` = no history) run on
/// `jobs` workers. The estimate is the larger of the total work spread over the workers and
/// the single longest check; checks without history are not included.
pub fn estimate_secs(durations: &[Option<f64>], jobs: usize) -> (f64, usize) {
    let known: Vec<f64> = durations.iter().flatten().copied().collect();
    let unknown = durations.len() - known.len();
    let total: f64 = known.iter().sum();
    let longest = known.iter().copied().fold(0.0_f64, f64::max);
    #[allow(clippy::cast_precision_loss)]
    let spread = total / jobs.max(1) as f64;
    (spread.max(longest), unknown)
}

/// The state letter of a `/proc/<pid>/stat` line (`R`, `S`, `Z`, …).
pub fn parse_stat_state(stat: &str) -> Option<char> {
    // `pid (comm) state …` — `comm` may contain spaces and `)`, so split
    // after the LAST `)`.
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().next()?.chars().next()
}

fn subsystems(specs: &Path, only: Option<&str>) -> Result<Vec<(String, PathBuf)>> {
    if let Some(only) = only.filter(|o| o.contains('/')) {
        let dir = fs::canonicalize(only).wrap_err_with(|| format!("resolving {only}"))?;
        if !dir.join(CHECKS_FILE).is_file() {
            bail!("no {} found", dir.join(CHECKS_FILE).display());
        }
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| eyre!("{} has no directory name", dir.display()))?;
        return Ok(vec![(name, dir)]);
    }
    if let Some(name) = only {
        let dir = specs.join(name);
        if !dir.join(CHECKS_FILE).is_file() {
            bail!("no {} found for subsystem `{name}`", dir.join(CHECKS_FILE).display());
        }
        return Ok(vec![(name.to_owned(), dir)]);
    }
    let mut found = Vec::new();
    for entry in fs::read_dir(specs).wrap_err_with(|| format!("reading {}", specs.display()))? {
        let entry = entry.wrap_err_with(|| format!("reading {}", specs.display()))?;
        let path = entry.path();
        if path.join(CHECKS_FILE).is_file() {
            found.push((entry.file_name().to_string_lossy().into_owned(), path));
        }
    }
    found.sort();
    Ok(found)
}

// ---------------------------------------------------------------------------
// Signals
// ---------------------------------------------------------------------------

/// Signal number received by the runner, or 0.
static SIGNALLED: AtomicI32 = AtomicI32::new(0);

extern "C" fn on_signal(sig: libc::c_int) {
    // Only an atomic store: async-signal-safe.
    SIGNALLED.store(sig, Ordering::SeqCst);
}

/// Route SIGINT / SIGTERM / SIGHUP to [`SIGNALLED`] so the supervisor loop
/// can kill every process group before exiting.
fn install_signal_handlers() -> Result<()> {
    for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        // SAFETY: `on_signal` performs a single atomic store, which is
        // async-signal-safe; the handler stays valid for the process lifetime.
        let prev = unsafe { libc::signal(sig, handler) };
        if prev == libc::SIG_ERR {
            bail!("installing handler for signal {sig}: {}", std::io::Error::last_os_error());
        }
    }
    Ok(())
}

fn signalled() -> Option<i32> {
    match SIGNALLED.load(Ordering::SeqCst) {
        0 => None,
        sig => Some(sig),
    }
}

// ---------------------------------------------------------------------------
// Process groups
// ---------------------------------------------------------------------------

/// `SIGKILL` every process in group `pgid`. An already-empty group is fine.
fn kill_group(pgid: i32) -> Result<()> {
    if pgid <= 1 {
        bail!("refusing to signal process group {pgid}");
    }
    // SAFETY: plain syscall; a negative pid addresses the process group, and
    // `pgid > 1` excludes the "every process" (-1) and own-group (0) forms.
    let rc = unsafe { libc::kill(-pgid, libc::SIGKILL) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            bail!("killing process group {pgid}: {err}");
        }
    }
    Ok(())
}

/// `SIGKILL` one process. An already-gone process is fine.
fn kill_pid(pid: i32) -> Result<()> {
    if pid <= 1 {
        bail!("refusing to signal pid {pid}");
    }
    // SAFETY: plain syscall on a positive pid.
    let rc = unsafe { libc::kill(pid, libc::SIGKILL) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            bail!("killing pid {pid}: {err}");
        }
    }
    Ok(())
}

/// A spawned check: Quint is the leader of its own process group, and every
/// process it starts (the Apalache / TLC JVM) stays in that group. Dropping
/// the guard kills the whole group and reaps Quint, so no exit path —
/// including an early `?` return — leaves the group running.
struct Group {
    pgid: i32,
    child: Child,
    reaped: bool,
}

impl Group {
    fn spawn(cmd: &mut Command) -> Result<Self> {
        let child = cmd.process_group(0).spawn().map_err(|e| spawn_error(&e))?;
        let pgid = i32::try_from(child.id()).wrap_err("child pid out of range")?;
        Ok(Self { pgid, child, reaped: false })
    }

    /// Kill the group (the leader may already have exited while its JVM
    /// lingers) and reap the leader.
    fn terminate(&mut self) -> Result<()> {
        kill_group(self.pgid)?;
        if !self.reaped {
            self.child.wait().wrap_err("reaping quint")?;
            self.reaped = true;
        }
        Ok(())
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        if let Err(err) = self.terminate() {
            eprintln!("xtask quint: cleanup of process group {}: {err:#}", self.pgid);
        }
    }
}

/// Whether a `/proc/<pid>/…` read failed because the process is gone (or
/// is a kernel thread, whose `environ` reads fail with `ESRCH`).
fn vanished(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::NotFound || err.raw_os_error() == Some(libc::ESRCH)
}

/// Every live (non-zombie) process carrying a run marker, as
/// `(pid, marker value)`. Processes whose
/// `/proc` entries vanish or are unreadable mid-scan are skipped: a racing
/// exit is not a leftover, and an unreadable `environ` belongs to another
/// user, which this runner (spawning as its own user) cannot have started.
/// Returns an empty list where `/proc` does not exist (non-Linux hosts).
fn scan_processes() -> Result<Vec<(i32, String)>> {
    let proc_dir = Path::new("/proc");
    if !proc_dir.is_dir() {
        return Ok(Vec::new());
    }
    let own = i32::try_from(std::process::id()).wrap_err("own pid out of range")?;
    let mut out = Vec::new();
    for entry in fs::read_dir(proc_dir).wrap_err("reading /proc")? {
        let entry = entry.wrap_err("reading /proc")?;
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else {
            continue;
        };
        if pid == own {
            continue;
        }
        let stat = match fs::read_to_string(entry.path().join("stat")) {
            Ok(s) => s,
            Err(e) if vanished(&e) => continue,
            Err(e) => return Err(eyre!("reading /proc/{pid}/stat: {e}")),
        };
        let marker = match fs::read(entry.path().join("environ")) {
            Ok(env) => env.split(|b| *b == 0).find_map(|kv| {
                let kv = std::str::from_utf8(kv).ok()?;
                kv.strip_prefix(RUN_MARKER_ENV)?.strip_prefix('=').map(str::to_owned)
            }),
            Err(e) if vanished(&e) || e.kind() == std::io::ErrorKind::PermissionDenied => None,
            Err(e) => return Err(eyre!("reading /proc/{pid}/environ: {e}")),
        };
        // A zombie / dead task has already released everything it held.
        if matches!(parse_stat_state(&stat), Some('Z' | 'X' | 'x')) {
            continue;
        }
        if let Some(marker) = marker {
            out.push((pid, marker));
        }
    }
    Ok(out)
}

/// Kill processes left behind by an earlier runner that died without
/// cleaning up (e.g. `SIGKILL`): tagged with a run marker whose runner pid is
/// no longer alive. Returns how many were killed.
fn sweep_stale(run_id: &str) -> Result<usize> {
    let mut killed = 0;
    for (pid, marker) in scan_processes()? {
        if marker == run_id {
            continue;
        }
        let runner_alive = marker
            .split('-')
            .next()
            .and_then(|p| p.parse::<i32>().ok())
            .is_some_and(|p| Path::new(&format!("/proc/{p}")).exists());
        if !runner_alive {
            kill_pid(pid)?;
            killed += 1;
        }
    }
    Ok(killed)
}

/// Processes still tagged with this run's marker.
fn leftovers(run_id: &str) -> Result<Vec<i32>> {
    Ok(scan_processes()?
        .into_iter()
        .filter(|(_, marker)| marker == run_id)
        .map(|(pid, _)| pid)
        .collect())
}

/// Wait up to `bound` for `leftovers` to become empty; returns what remains.
fn await_gone(run_id: &str, bound: Duration) -> Result<Vec<i32>> {
    let deadline = Instant::now() + bound;
    loop {
        let still = leftovers(run_id)?;
        if still.is_empty() || Instant::now() >= deadline {
            return Ok(still);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn cmdline(pid: i32) -> String {
    fs::read(format!("/proc/{pid}/cmdline")).map_or_else(
        |e| format!("<cmdline unreadable: {e}>"),
        |b| {
            let s = String::from_utf8_lossy(&b).replace('\0', " ");
            s.trim().chars().take(160).collect()
        },
    )
}

/// Verify no process tagged with this run's marker remains. Processes are
/// tracked by the inherited environment marker rather than by process-group
/// id, so a recycled pgid can never select an unrelated process. Every
/// group was already `SIGKILL`ed, so first allow the kernel a few seconds
/// to finish tearing members down; anything still present after that is a
/// genuine escapee — reported, killed, and an error if it survives.
fn verify_no_orphans(run_id: &str) -> Result<()> {
    let found = await_gone(run_id, Duration::from_secs(5))?;
    if found.is_empty() {
        eprintln!("xtask quint check: verified no Quint/JVM process from this run remains");
        return Ok(());
    }
    for pid in &found {
        eprintln!(
            "xtask quint check: leftover pid {pid} outside its process group: {}",
            cmdline(*pid)
        );
        kill_pid(*pid)?;
    }
    let still = await_gone(run_id, Duration::from_secs(5))?;
    if still.is_empty() {
        eprintln!("xtask quint check: killed {} leftover process(es)", found.len());
        Ok(())
    } else {
        bail!("process(es) from this run survived SIGKILL: {still:?}")
    }
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Pick a free loopback port for this attempt's Apalache server, so Quint
/// spawns a private server rather than attaching to one another process
/// already runs (whose working directory, heap, and lifetime this run does
/// not control). Ports already handed out in this run are never reused.
fn free_port(used: &mut BTreeSet<u16>) -> Result<u16> {
    for _ in 0..64 {
        let listener = TcpListener::bind("127.0.0.1:0").wrap_err("binding a probe port")?;
        let port = listener.local_addr().wrap_err("reading probe port")?.port();
        if used.insert(port) {
            return Ok(port);
        }
    }
    bail!("could not find an unused loopback port")
}

fn unix_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// Append one timestamped line to a check's `events.log`.
fn event(out_dir: &Path, line: &str) -> Result<()> {
    let path = out_dir.join("events.log");
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .wrap_err_with(|| format!("opening {}", path.display()))?;
    writeln!(f, "{:.3} {line}", unix_now()).wrap_err_with(|| format!("writing {}", path.display()))
}

/// One selected check.
struct Job {
    subsystem: String,
    dir: PathBuf,
    check: Check,
    out_dir: PathBuf,
    timeout: Duration,
}

/// One attempt in flight.
struct Running {
    job: usize,
    attempt: u32,
    port: u16,
    started: Instant,
    started_work: bool,
    log_path: PathBuf,
    group: Group,
}

/// Final per-check record (also the `summary.json` row).
#[derive(Debug, Clone, Serialize)]
struct CheckRecord {
    name: String,
    backend: Backend,
    expect: Expect,
    outcome: Outcome,
    pass: bool,
    seconds: f64,
    attempts: u32,
    timeout_secs: u64,
    cause: Option<String>,
    log: PathBuf,
}

#[derive(Debug, Serialize)]
struct Summary<'a> {
    subsystem: &'a str,
    run_id: &'a str,
    started_unix: f64,
    finished_unix: f64,
    jobs: usize,
    interrupted_by_signal: Option<i32>,
    checks: Vec<&'a CheckRecord>,
}

/// File (per subsystem) holding the last measured seconds of every check
/// that produced a result, merged across runs; feeds the estimate only.
const DURATIONS_FILE: &str = "durations.json";

/// Read a subsystem's `durations.json`. A missing file means no history; an
/// unreadable or malformed one is reported and treated as empty (it only
/// feeds the estimate).
fn read_durations(path: &Path) -> BTreeMap<String, f64> {
    let body = match fs::read_to_string(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return BTreeMap::new(),
        Err(e) => {
            eprintln!("xtask quint check: ignoring {} for the estimate: {e}", path.display());
            return BTreeMap::new();
        }
    };
    serde_json::from_str(&body).unwrap_or_else(|e| {
        eprintln!("xtask quint check: ignoring {} for the estimate: {e}", path.display());
        BTreeMap::new()
    })
}

/// Previous per-check durations, keyed by `(subsystem, check)`.
fn prior_durations(root: &Path, subsystems: &BTreeSet<&str>) -> BTreeMap<(String, String), f64> {
    let mut out = BTreeMap::new();
    for sub in subsystems {
        for (name, secs) in read_durations(&root.join(OUT_DIR).join(sub).join(DURATIONS_FILE)) {
            out.insert(((*sub).to_owned(), name), secs);
        }
    }
    out
}

fn mem_total_mb() -> Option<u64> {
    let info = fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = info
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some(kb / 1024)
}

fn start_attempt(
    jobs: &[Job],
    job: usize,
    attempt: u32,
    run_id: &str,
    ports: &mut BTreeSet<u16>,
) -> Result<Running> {
    let j = &jobs[job];
    let out_dir = &j.out_dir;
    if attempt == 1 {
        // Fresh output dir so a stale trace or log is never mistaken for
        // this run's. Later attempts keep earlier attempts' files.
        if out_dir.exists() {
            fs::remove_dir_all(out_dir)
                .wrap_err_with(|| format!("clearing {}", out_dir.display()))?;
        }
        fs::create_dir_all(out_dir).wrap_err_with(|| format!("creating {}", out_dir.display()))?;
    }
    let tmp = out_dir.join(format!("tmp-{attempt}"));
    fs::create_dir_all(&tmp).wrap_err_with(|| format!("creating {}", tmp.display()))?;
    let tmp_str = tmp.to_str().ok_or_else(|| eyre!("non-UTF-8 path {}", tmp.display()))?;
    if tmp_str.contains(char::is_whitespace) {
        bail!("TMPDIR {tmp_str} contains whitespace, which JAVA_TOOL_OPTIONS cannot carry");
    }
    let log_path = out_dir.join(format!("attempt-{attempt}.log"));
    let log =
        File::create(&log_path).wrap_err_with(|| format!("creating {}", log_path.display()))?;
    let log_err = log.try_clone().wrap_err("duplicating log handle")?;
    let port = free_port(ports)?;

    let c = &j.check;
    let mut cmd = quint_command();
    cmd.arg("verify")
        .arg(j.dir.join(&c.spec))
        .arg(format!("--main={}", c.main))
        .arg(format!("--backend={}", c.backend.as_str()))
        // TLC prints its counterexample only from verbosity 3 (Quint 0.32
        // writes no ITF for the TLC backend); the evidence keeps it.
        .arg(format!("--verbosity={}", if c.backend == Backend::Tlc { 3 } else { 2 }))
        .arg(format!("--server-endpoint=localhost:{port}"));
    match &c.property {
        Property::Invariant(inv) => cmd.arg(format!("--invariant={inv}")),
        Property::Temporal(prop) => cmd.arg(format!("--temporal={prop}")),
    };
    if let Some(steps) = c.max_steps {
        cmd.arg(format!("--max-steps={steps}"));
    }
    if c.backend == Backend::Apalache {
        cmd.arg(format!("--out-itf={}", out_dir.join("trace.itf.json").display()));
    }
    // Apalache writes `_apalache-out/` into its working directory; its
    // launcher honours JVM_ARGS (heap) and puts `java.io.tmpdir` under
    // TMPDIR; Quint puts TLC's metadir under TMPDIR. JAVA_TOOL_OPTIONS
    // covers every other JVM temp use and disables the hsperfdata file the
    // JVM otherwise writes to /tmp. All of these default to /tmp, a
    // RAM-backed tmpfs in the Lima VM that a large state space can exhaust.
    cmd.current_dir(out_dir)
        .env("TMPDIR", &tmp)
        .env("JVM_ARGS", format!("-Xmx{JVM_HEAP_MB}m"))
        .env("JAVA_TOOL_OPTIONS", format!("-XX:-UsePerfData -Djava.io.tmpdir={tmp_str}"))
        .env(RUN_MARKER_ENV, run_id)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_err);
    let group = Group::spawn(&mut cmd)?;
    event(
        out_dir,
        &format!(
            "attempt {attempt} start pgid={} port={port} timeout={}s log={}",
            group.pgid,
            j.timeout.as_secs(),
            log_path.display()
        ),
    )?;
    Ok(Running {
        job,
        attempt,
        port,
        started: Instant::now(),
        started_work: false,
        log_path,
        group,
    })
}

/// Poll one attempt; `Some(end)` once it has ended (and been killed).
fn poll_attempt(r: &mut Running, timeout: Duration) -> Result<Option<AttemptEnd>> {
    if let Some(status) = r.group.child.try_wait().wrap_err("polling quint")? {
        r.group.reaped = true;
        return Ok(Some(AttemptEnd::Exited { success: status.success() }));
    }
    if !r.started_work {
        let log =
            fs::read(&r.log_path).wrap_err_with(|| format!("reading {}", r.log_path.display()))?;
        r.started_work = started_work(&String::from_utf8_lossy(&log));
    }
    Ok(deadline_check(
        r.started.elapsed(),
        r.started_work,
        timeout,
        Duration::from_secs(STARTUP_BOUND_SECS),
    ))
}

/// Read and filter every selected `[[check]]` into a [`Job`].
fn select_jobs(
    root: &Path,
    specs: &Path,
    filter: &CheckFilter,
    opts: CheckOptions,
) -> Result<Vec<Job>> {
    let mut jobs: Vec<Job> = Vec::new();
    for (subsystem, dir) in subsystems(specs, filter.subsystem.as_deref())? {
        let manifest = dir.join(CHECKS_FILE);
        let body = fs::read_to_string(&manifest)
            .wrap_err_with(|| format!("reading {}", manifest.display()))?;
        for c in parse_checks(&manifest, &body)? {
            if filter.ci && !c.ci {
                continue;
            }
            if filter.name.as_deref().is_some_and(|n| n != c.name) {
                continue;
            }
            jobs.push(Job {
                out_dir: root.join(OUT_DIR).join(&subsystem).join(&c.name),
                timeout: effective_timeout(opts.timeout_secs, c.timeout_secs),
                subsystem: subsystem.clone(),
                dir: dir.clone(),
                check: c,
            });
        }
    }
    Ok(jobs)
}

/// Print the run header and the duration estimate.
fn announce(root: &Path, jobs: &[Job], workers: usize, run_id: &str) {
    let subsystem_names: BTreeSet<&str> = jobs.iter().map(|j| j.subsystem.as_str()).collect();
    let prior = prior_durations(root, &subsystem_names);
    let history: Vec<Option<f64>> = jobs
        .iter()
        .map(|j| prior.get(&(j.subsystem.clone(), j.check.name.clone())).copied())
        .collect();
    let (estimate, no_history) = estimate_secs(&history, workers);
    let worst_unknown = jobs
        .iter()
        .zip(&history)
        .filter(|(_, h)| h.is_none())
        .fold(Duration::ZERO, |acc, (j, _)| acc + j.timeout);
    let worst_unknown_min =
        worst_unknown.as_secs_f64() / 60.0 / f64::from(u32::try_from(workers).unwrap_or(u32::MAX));
    eprintln!(
        "xtask quint check: {} check(s), --jobs {workers}, Apalache heap {JVM_HEAP_MB} MiB, \
         start-up bound {STARTUP_BOUND_SECS}s, run id {run_id}",
        jobs.len()
    );
    let unknown_note = if no_history == 0 {
        String::new()
    } else {
        format!(
            "; {no_history} check(s) without history (worst case +{worst_unknown_min:.1} min \
             of timeouts over {workers} job(s))"
        )
    };
    eprintln!(
        "xtask quint check: estimate ~{:.1} min from previous durations{unknown_note}",
        estimate / 60.0
    );
}

/// The supervisor: a single-threaded poll loop over up to `workers`
/// concurrent attempts.
struct Supervisor<'a> {
    jobs: &'a [Job],
    workers: usize,
    run_id: &'a str,
    run_start: Instant,
    ports: BTreeSet<u16>,
    queue: VecDeque<(usize, u32)>,
    running: Vec<Running>,
    records: Vec<Option<CheckRecord>>,
    first_started: Vec<Option<Instant>>,
}

impl<'a> Supervisor<'a> {
    fn new(jobs: &'a [Job], workers: usize, run_id: &'a str) -> Self {
        Self {
            jobs,
            workers,
            run_id,
            run_start: Instant::now(),
            ports: BTreeSet::new(),
            queue: (0..jobs.len()).map(|i| (i, 1)).collect(),
            running: Vec::new(),
            records: vec![None; jobs.len()],
            first_started: vec![None; jobs.len()],
        }
    }

    fn stamp(&self, t: Instant) -> String {
        format!("[{:>7.1}s]", t.duration_since(self.run_start).as_secs_f64())
    }

    /// Run until every check has a final verdict (`None`) or a signal
    /// arrives (`Some(signal)`).
    fn run(&mut self) -> Result<Option<i32>> {
        loop {
            if let Some(sig) = signalled() {
                return Ok(Some(sig));
            }
            self.fill()?;
            if self.running.is_empty() {
                return Ok(None);
            }
            std::thread::sleep(POLL_INTERVAL);
            let mut i = 0;
            while i < self.running.len() {
                let timeout = self.jobs[self.running[i].job].timeout;
                if let Some(end) = poll_attempt(&mut self.running[i], timeout)? {
                    let r = self.running.remove(i);
                    self.finish_attempt(r, end)?;
                } else {
                    i += 1;
                }
            }
        }
    }

    /// Start queued attempts up to the worker limit.
    fn fill(&mut self) -> Result<()> {
        while self.running.len() < self.workers {
            let Some((job, attempt)) = self.queue.pop_front() else { break };
            let r = start_attempt(self.jobs, job, attempt, self.run_id, &mut self.ports)?;
            self.first_started[job].get_or_insert(r.started);
            let j = &self.jobs[job];
            eprintln!(
                "{} start  {}/{} [{}] attempt {attempt} port {} timeout {}s ({} running, {} queued)",
                self.stamp(r.started),
                j.subsystem,
                j.check.name,
                j.check.backend.as_str(),
                r.port,
                j.timeout.as_secs(),
                self.running.len() + 1,
                self.queue.len(),
            );
            self.running.push(r);
        }
        Ok(())
    }

    /// Kill an ended attempt's group, judge it, and either requeue (start-up
    /// hang retry) or record the final verdict.
    fn finish_attempt(&mut self, mut r: Running, end: AttemptEnd) -> Result<()> {
        // Always kill the group: Quint may exit while its JVM lingers.
        r.group.terminate()?;
        let j = &self.jobs[r.job];
        let output =
            fs::read(&r.log_path).wrap_err_with(|| format!("reading {}", r.log_path.display()))?;
        let verdict = attempt_verdict(
            r.attempt,
            end,
            &String::from_utf8_lossy(&output),
            j.timeout,
            Duration::from_secs(STARTUP_BOUND_SECS),
        );
        let attempt_secs = r.started.elapsed().as_secs_f64();
        event(
            &j.out_dir,
            &format!(
                "attempt {} end {end:?} after {attempt_secs:.1}s; group {} killed; verdict {verdict:?}",
                r.attempt, r.group.pgid
            ),
        )?;
        let now = self.stamp(Instant::now());
        match verdict {
            AttemptVerdict::Retry { cause } => {
                eprintln!("{now} retry  {}/{}: {cause}", j.subsystem, j.check.name);
                self.queue.push_front((r.job, r.attempt + 1));
            }
            AttemptVerdict::Final { outcome, cause } => {
                let pass = judge(j.check.expect, outcome);
                let seconds =
                    self.first_started[r.job].map_or(attempt_secs, |t| t.elapsed().as_secs_f64());
                eprintln!(
                    "{now} {}   {}/{} expect={} got={outcome} ({seconds:.1}s){}",
                    if pass { "PASS" } else { "FAIL" },
                    j.subsystem,
                    j.check.name,
                    j.check.expect.as_str(),
                    cause.as_deref().map_or(String::new(), |c| format!(" — {c}")),
                );
                if !pass {
                    eprintln!("            log: {}", r.log_path.display());
                }
                self.records[r.job] = Some(CheckRecord {
                    name: j.check.name.clone(),
                    backend: j.check.backend,
                    expect: j.check.expect,
                    outcome,
                    pass,
                    seconds,
                    attempts: r.attempt,
                    timeout_secs: j.timeout.as_secs(),
                    cause,
                    log: r.log_path,
                });
            }
        }
        Ok(())
    }

    /// Signal received: kill every in-flight group and mark every
    /// unfinished check `interrupted`.
    fn interrupt(&mut self, sig: i32) -> Result<()> {
        eprintln!(
            "xtask quint check: signal {sig} received; killing {} running check(s)",
            self.running.len()
        );
        for r in &mut self.running {
            r.group.terminate()?;
            let j = &self.jobs[r.job];
            event(&j.out_dir, &format!("attempt {} killed: runner got signal {sig}", r.attempt))?;
        }
        for (i, j) in self.jobs.iter().enumerate() {
            if self.records[i].is_none() {
                self.records[i] = Some(CheckRecord {
                    name: j.check.name.clone(),
                    backend: j.check.backend,
                    expect: j.check.expect,
                    outcome: Outcome::Interrupted,
                    pass: false,
                    seconds: self.first_started[i].map_or(0.0, |t| t.elapsed().as_secs_f64()),
                    attempts: self.running.iter().find(|r| r.job == i).map_or(0, |r| r.attempt),
                    timeout_secs: j.timeout.as_secs(),
                    cause: Some(format!("runner received signal {sig}")),
                    log: j.out_dir.clone(),
                });
            }
        }
        Ok(())
    }
}

/// Write one `summary.json` per subsystem.
fn write_summaries(
    root: &Path,
    jobs: &[Job],
    records: &[CheckRecord],
    meta: (&str, f64, usize, Option<i32>),
) -> Result<()> {
    let (run_id, started_unix, workers, interrupted) = meta;
    let finished_unix = unix_now();
    let subsystem_names: BTreeSet<&str> = jobs.iter().map(|j| j.subsystem.as_str()).collect();
    for sub in subsystem_names {
        let summary = Summary {
            subsystem: sub,
            run_id,
            started_unix,
            finished_unix,
            jobs: workers,
            interrupted_by_signal: interrupted,
            checks: jobs
                .iter()
                .zip(records)
                .filter(|(j, _)| j.subsystem == sub)
                .map(|(_, r)| r)
                .collect(),
        };
        let path = root.join(OUT_DIR).join(sub).join("summary.json");
        let body = serde_json::to_string_pretty(&summary).wrap_err("encoding summary.json")?;
        fs::write(&path, body).wrap_err_with(|| format!("writing {}", path.display()))?;
        eprintln!("xtask quint check: wrote {}", path.display());

        // Merge this run's measured durations into the estimate history; a
        // tool error, timeout or interruption measured nothing representative.
        let dpath = root.join(OUT_DIR).join(sub).join(DURATIONS_FILE);
        let mut durations = read_durations(&dpath);
        for r in &summary.checks {
            if matches!(r.outcome, Outcome::Holds | Outcome::Violation) {
                durations.insert(r.name.clone(), r.seconds);
            }
        }
        let body = serde_json::to_string_pretty(&durations).wrap_err("encoding durations")?;
        fs::write(&dpath, body).wrap_err_with(|| format!("writing {}", dpath.display()))?;
    }
    Ok(())
}

/// `cargo xtask quint check` — run the selected `checks.toml` entries and fail
/// unless every outcome matches its `expect`.
///
/// With `opts.record`, the selection must be one full subsystem
/// ([`evidence::validate_record_args`]); a completed run then replaces that
/// subsystem's `evidence/` whatever the verdicts (an interrupted run records
/// nothing).
pub fn check(filter: &CheckFilter, opts: CheckOptions) -> Result<()> {
    if opts.record {
        evidence::validate_record_args(filter)?;
    }
    let root = workspace_root()?;
    let specs = root.join(SPECS_DIR);
    let subsystem_path = filter.subsystem.as_deref().is_some_and(|s| s.contains('/'));
    if !specs.is_dir() && !subsystem_path {
        if filter.subsystem.is_some() || filter.name.is_some() {
            bail!("no {SPECS_DIR}/ directory");
        }
        eprintln!("xtask quint check: no {SPECS_DIR}/ directory; nothing to check");
        return Ok(());
    }
    let jobs = select_jobs(&root, &specs, filter, opts)?;
    if jobs.is_empty() {
        if let Some(name) = &filter.name {
            bail!("no check named `{name}` matches the selection");
        }
        eprintln!("xtask quint check: no checks selected");
        return Ok(());
    }

    let record_inputs = if opts.record { Some(evidence::hash_inputs(&jobs[0].dir)?) } else { None };

    let cpus = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let workers = opts.jobs.unwrap_or_else(|| default_jobs(cpus, mem_total_mb())).max(1);
    let started_unix = unix_now();
    let run_id = format!("{}-{started_unix:.3}", std::process::id());

    install_signal_handlers()?;
    let swept = sweep_stale(&run_id)?;
    if swept > 0 {
        eprintln!("xtask quint check: killed {swept} process(es) left by an earlier, dead runner");
    }
    announce(&root, &jobs, workers, &run_id);

    let mut sup = Supervisor::new(&jobs, workers, &run_id);
    let interrupted = sup.run()?;
    if let Some(sig) = interrupted {
        sup.interrupt(sig)?;
    }
    let wall = sup.run_start.elapsed();
    let records: Vec<CheckRecord> = std::mem::take(&mut sup.records)
        .into_iter()
        .map(|r| {
            r.unwrap_or_else(|| unreachable!("every job ends in a final verdict or interrupt"))
        })
        .collect();
    drop(sup);
    verify_no_orphans(&run_id)?;

    write_summaries(&root, &jobs, &records, (&run_id, started_unix, workers, interrupted))?;
    print_table(&jobs, &records, wall);
    if let Some(inputs) = record_inputs {
        record_evidence(
            &root,
            &jobs,
            &records,
            (&run_id, started_unix, workers, interrupted),
            inputs,
        )?;
    }

    let failures: Vec<String> = jobs
        .iter()
        .zip(&records)
        .filter(|(_, r)| !r.pass)
        .map(|(j, r)| {
            format!(
                "{}/{} (expected {}, got {})",
                j.subsystem,
                r.name,
                r.expect.as_str(),
                r.outcome
            )
        })
        .collect();
    if let Some(sig) = interrupted {
        bail!(
            "interrupted by signal {sig}; {} of {} check(s) unfinished or failed",
            failures.len(),
            jobs.len()
        );
    }
    if failures.is_empty() {
        eprintln!("xtask quint check: {} check(s) passed", jobs.len());
        Ok(())
    } else {
        bail!(
            "{} of {} Quint check(s) failed:\n  {}",
            failures.len(),
            jobs.len(),
            failures.join("\n  ")
        )
    }
}

/// `--record`: replace the subsystem's `evidence/` after a completed run.
fn record_evidence(
    root: &Path,
    jobs: &[Job],
    records: &[CheckRecord],
    meta: (&str, f64, usize, Option<i32>),
    inputs: BTreeMap<String, String>,
) -> Result<()> {
    let (run_id, started_unix, workers, interrupted) = meta;
    if let Some(sig) = interrupted {
        eprintln!("xtask quint check: interrupted by signal {sig}; evidence not recorded");
        return Ok(());
    }
    evidence::record(
        &evidence::RunMeta {
            root,
            subsystem: &jobs[0].subsystem,
            dir: &jobs[0].dir,
            run_id,
            started_unix,
            finished_unix: unix_now(),
            jobs: workers,
            inputs,
        },
        jobs,
        records,
    )
}

/// `cargo xtask quint verify-evidence` — fail unless recorded evidence is current and passing.
///
/// Every selected subsystem's `evidence/summary.json` must have been
/// recorded against its current `.qnt` files and `checks.toml`, and every
/// recorded outcome must match its `expect`. Reads files only; needs no
/// Quint toolchain.
pub fn verify_evidence(subsystem: Option<&str>) -> Result<()> {
    let root = workspace_root()?;
    let specs = root.join(SPECS_DIR);
    let subsystem_path = subsystem.is_some_and(|s| s.contains('/'));
    if !specs.is_dir() && !subsystem_path {
        if subsystem.is_some() {
            bail!("no {SPECS_DIR}/ directory");
        }
        eprintln!("xtask quint verify-evidence: no {SPECS_DIR}/ directory; nothing to verify");
        return Ok(());
    }
    let mut failed = Vec::new();
    let selected = subsystems(&specs, subsystem)?;
    for (name, dir) in &selected {
        let manifest = dir.join(CHECKS_FILE);
        let body = fs::read_to_string(&manifest)
            .wrap_err_with(|| format!("reading {}", manifest.display()))?;
        let checks = parse_checks(&manifest, &body)?;
        let problems = evidence::verify_dir(dir, &checks)?;
        if problems.is_empty() {
            eprintln!("  ok    {name}: evidence matches the specs; every outcome met `expect`");
        } else {
            eprintln!("  FAIL  {name}:");
            for p in &problems {
                eprintln!("          {p}");
            }
            failed.push(name.clone());
        }
    }
    if failed.is_empty() {
        eprintln!("xtask quint verify-evidence: {} subsystem(s) verified", selected.len());
        Ok(())
    } else {
        bail!(
            "Quint evidence is stale or failing for {}; re-record with \
             `cargo xtask quint check --subsystem <dir> --record`",
            failed.join(", ")
        )
    }
}

fn print_table(jobs: &[Job], records: &[CheckRecord], wall: Duration) {
    let width =
        jobs.iter().map(|j| j.subsystem.len() + 1 + j.check.name.len()).max().unwrap_or(5).max(5);
    eprintln!();
    eprintln!(
        "{:<width$}  {:<9}  {:<11}  {:>8}  {:>3}  RESULT",
        "CHECK", "EXPECT", "OUTCOME", "SECONDS", "TRY"
    );
    for (j, r) in jobs.iter().zip(records) {
        eprintln!(
            "{:<width$}  {:<9}  {:<11}  {:>8.1}  {:>3}  {}",
            format!("{}/{}", j.subsystem, r.name),
            r.expect.as_str(),
            r.outcome.as_str(),
            r.seconds,
            r.attempts,
            if r.pass { "PASS" } else { "FAIL" },
        );
    }
    eprintln!("wall clock: {:.1}s", wall.as_secs_f64());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(body: &str) -> Result<Vec<Check>, ChecksError> {
        parse_checks(Path::new("checks.toml"), body)
    }

    fn reason(body: &str) -> String {
        match parse(body) {
            Err(ChecksError::InvalidCheck { reason, .. }) => reason,
            other => panic!("expected InvalidCheck, got {other:?}"),
        }
    }

    const APALACHE_INV: &str = r#"
[[check]]
name = "owner-one-session-per-cid"
spec = "owner_flows.qnt"
main = "flows_ok"
invariant = "OneLiveSession"
backend = "apalache"
max_steps = 14
expect = "holds"
ci = true
"#;

    #[test]
    fn parses_apalache_invariant_check() {
        let checks = parse(APALACHE_INV).expect("valid manifest");
        assert_eq!(
            checks,
            vec![Check {
                name: "owner-one-session-per-cid".into(),
                spec: "owner_flows.qnt".into(),
                main: "flows_ok".into(),
                property: Property::Invariant("OneLiveSession".into()),
                backend: Backend::Apalache,
                max_steps: Some(14),
                expect: Expect::Holds,
                ci: true,
                timeout_secs: None,
            }]
        );
    }

    #[test]
    fn parses_tlc_temporal_violation_and_defaults_ci_false() {
        let checks = parse(
            r#"
[[check]]
name = "hazard"
spec = "hazard/x.qnt"
main = "m"
temporal = "Live"
backend = "tlc"
expect = "violation"
"#,
        )
        .expect("valid manifest");
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].property, Property::Temporal("Live".into()));
        assert_eq!(checks[0].backend, Backend::Tlc);
        assert_eq!(checks[0].expect, Expect::Violation);
        assert_eq!(checks[0].max_steps, None);
        assert!(!checks[0].ci);
    }

    #[test]
    fn empty_manifest_has_no_checks() {
        assert!(parse("").expect("empty is valid").is_empty());
    }

    #[test]
    fn rejects_both_invariant_and_temporal() {
        let body = APALACHE_INV.replace("expect", "temporal = \"T\"\nexpect");
        assert!(reason(&body).contains("not both"));
    }

    #[test]
    fn rejects_neither_invariant_nor_temporal() {
        let body = APALACHE_INV.replace("invariant = \"OneLiveSession\"\n", "");
        assert!(reason(&body).contains("exactly one"));
    }

    #[test]
    fn rejects_max_steps_on_tlc() {
        let body = APALACHE_INV.replace("\"apalache\"", "\"tlc\"");
        assert!(reason(&body).contains("max_steps"));
    }

    #[test]
    fn rejects_temporal_on_apalache() {
        let body = APALACHE_INV.replace("invariant =", "temporal =");
        assert!(reason(&body).contains("require backend = \"tlc\""));
    }

    #[test]
    fn rejects_duplicate_names() {
        let body = format!("{APALACHE_INV}{APALACHE_INV}");
        assert!(reason(&body).contains("duplicate"));
    }

    #[test]
    fn rejects_spec_escaping_subsystem_dir() {
        assert!(
            reason(&APALACHE_INV.replace("\"owner_flows.qnt\"", "\"../x.qnt\""))
                .contains("relative path")
        );
        assert!(
            reason(&APALACHE_INV.replace("\"owner_flows.qnt\"", "\"/abs/x.qnt\""))
                .contains("relative path")
        );
    }

    #[test]
    fn rejects_name_unsafe_as_directory() {
        assert!(
            reason(&APALACHE_INV.replace("owner-one-session-per-cid", "a/b"))
                .contains("name must be")
        );
        assert!(
            reason(&APALACHE_INV.replace("\"owner-one-session-per-cid\"", "\"\""))
                .contains("name must be")
        );
    }

    #[test]
    fn rejects_unknown_backend_expect_and_field() {
        for body in [
            APALACHE_INV.replace("\"apalache\"", "\"z3\""),
            APALACHE_INV.replace("\"holds\"", "\"fails\""),
            APALACHE_INV.replace("ci = true", "ci = true\ntimeout = 3"),
            APALACHE_INV.replace("ci = true", "ci = true\ntimeout_secs = -3"),
            APALACHE_INV.replace("ci = true", "ci = true\ntimeout_secs = \"30\""),
        ] {
            assert!(matches!(parse(&body), Err(ChecksError::Parse { .. })), "{body}");
        }
    }

    #[test]
    fn rejects_missing_required_field() {
        let body = APALACHE_INV.replace("main = \"flows_ok\"\n", "");
        assert!(matches!(parse(&body), Err(ChecksError::Parse { .. })));
    }

    // Output excerpts captured from Quint v0.32.0 (Apalache 0.56.1 / TLC).
    const OK_OUT: &str = "The outcome is: NoError\n[ok] No violation found (2824ms).\n";
    const VIOLATION_OUT: &str =
        "[violation] Found an issue (626ms).\nerror: found a counterexample\n";
    const NAME_ERROR_OUT: &str =
        "error: [QNT404] Name 'nosuch' not found\nerror: name resolution failed\n";
    const TLC_ERROR_OUT: &str =
        "[failure] TLC encountered an error (193ms).\nerror: TLC error (see output above)\n";
    const PROMPT_OUT: &str = "Consider using --backend tlc, which fully supports temporal properties.\n\
                              Do you want to proceed with Apalache anyway? (y/N) ";

    #[test]
    fn classify_requires_positive_evidence() {
        assert_eq!(classify(true, OK_OUT), Outcome::Holds);
        assert_eq!(classify(false, VIOLATION_OUT), Outcome::Violation);
        // Exit 1 without the counterexample marker is a tool error, not a violation.
        assert_eq!(classify(false, NAME_ERROR_OUT), Outcome::ToolError);
        assert_eq!(classify(false, TLC_ERROR_OUT), Outcome::ToolError);
        assert_eq!(classify(false, ""), Outcome::ToolError);
        // Exit 0 without the [ok] marker (e.g. an unanswered prompt) is a tool error.
        assert_eq!(classify(true, PROMPT_OUT), Outcome::ToolError);
        assert_eq!(classify(true, ""), Outcome::ToolError);
        // Markers are honoured only with the matching exit status.
        assert_eq!(classify(true, VIOLATION_OUT), Outcome::ToolError);
        assert_eq!(classify(false, OK_OUT), Outcome::ToolError);
    }

    #[test]
    fn judge_matches_expectation_and_never_passes_tool_error() {
        assert!(judge(Expect::Holds, Outcome::Holds));
        assert!(judge(Expect::Violation, Outcome::Violation));
        assert!(!judge(Expect::Holds, Outcome::Violation));
        assert!(!judge(Expect::Violation, Outcome::Holds));
        assert!(!judge(Expect::Holds, Outcome::ToolError));
        assert!(!judge(Expect::Violation, Outcome::ToolError));
    }

    #[test]
    fn parses_optional_timeout_secs_and_rejects_zero() {
        let body = APALACHE_INV.replace("ci = true", "ci = true\ntimeout_secs = 1800");
        assert_eq!(parse(&body).expect("valid manifest")[0].timeout_secs, Some(1800));
        assert_eq!(parse(APALACHE_INV).expect("valid manifest")[0].timeout_secs, None);
        let zero = APALACHE_INV.replace("ci = true", "ci = true\ntimeout_secs = 0");
        assert!(reason(&zero).contains("timeout_secs"));
    }

    #[test]
    fn every_existing_manifest_still_parses() {
        let specs = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(SPECS_DIR);
        let Ok(entries) = fs::read_dir(&specs) else { return };
        for entry in entries {
            let manifest = entry.expect("dir entry").path().join(CHECKS_FILE);
            if manifest.is_file() {
                let body = fs::read_to_string(&manifest).expect("read manifest");
                parse_checks(&manifest, &body).expect("existing manifest parses");
            }
        }
    }

    #[test]
    fn effective_timeout_prefers_cli_then_check_then_default() {
        assert_eq!(effective_timeout(Some(5), Some(60)), Duration::from_secs(5));
        assert_eq!(effective_timeout(None, Some(60)), Duration::from_secs(60));
        assert_eq!(effective_timeout(None, None), Duration::from_secs(DEFAULT_TIMEOUT_SECS));
    }

    #[test]
    fn default_jobs_is_bounded_by_cpu_memory_and_cap() {
        assert_eq!(default_jobs(8, Some(16 * 1024)), 3); // memory-bound: 16 GiB / 5 GiB
        assert_eq!(default_jobs(8, Some(64 * 1024)), 4); // cpu/2 = 4
        assert_eq!(default_jobs(32, Some(256 * 1024)), MAX_DEFAULT_JOBS);
        assert_eq!(default_jobs(4, None), 2);
        assert_eq!(default_jobs(1, None), 1);
        assert_eq!(default_jobs(8, Some(1024)), 1); // never zero
    }

    const SECS: fn(u64) -> Duration = Duration::from_secs;

    #[test]
    fn deadline_check_orders_timeout_before_startup_hang() {
        let (t, s) = (SECS(900), SECS(120));
        assert_eq!(deadline_check(SECS(10), false, t, s), None);
        assert_eq!(deadline_check(SECS(119), false, t, s), None);
        assert_eq!(deadline_check(SECS(120), false, t, s), Some(AttemptEnd::StartupHang));
        // A check Apalache has started is never a start-up hang.
        assert_eq!(deadline_check(SECS(800), true, t, s), None);
        assert_eq!(deadline_check(SECS(900), true, t, s), Some(AttemptEnd::TimedOut));
        // A timeout shorter than the start-up bound fires as a timeout.
        assert_eq!(deadline_check(SECS(5), false, SECS(5), s), Some(AttemptEnd::TimedOut));
    }

    #[test]
    fn started_work_needs_the_first_pass_not_a_listening_server() {
        // Captured from the hung attempt: server up and idle, job never sent.
        let hung = "[TLC] Compiling to TLA+ (via Apalache)...\n\
                    Starting checker server on port 45655...   I@22:21:49.830\n\
                    The Apalache server is running on port 45655. Press Ctrl-C to stop.\n";
        assert!(!started_work(hung));
        assert!(started_work(&format!(
            "{hung}PASS #0: SanyParser                                I@22:10:03.506\n"
        )));
        assert!(!started_work(""));
    }

    #[test]
    fn attempt_verdict_classifies_every_end() {
        let (t, s) = (SECS(900), SECS(120));
        assert_eq!(
            attempt_verdict(1, AttemptEnd::Exited { success: true }, OK_OUT, t, s),
            AttemptVerdict::Final { outcome: Outcome::Holds, cause: None }
        );
        assert_eq!(
            attempt_verdict(2, AttemptEnd::Exited { success: false }, VIOLATION_OUT, t, s),
            AttemptVerdict::Final { outcome: Outcome::Violation, cause: None }
        );
        match attempt_verdict(1, AttemptEnd::Exited { success: false }, NAME_ERROR_OUT, t, s) {
            AttemptVerdict::Final { outcome: Outcome::ToolError, cause: Some(_) } => {}
            other => panic!("expected tool error with cause, got {other:?}"),
        }
        // A timeout is its own outcome, even when the log already holds a
        // counterexample marker — never a violation.
        match attempt_verdict(1, AttemptEnd::TimedOut, VIOLATION_OUT, t, s) {
            AttemptVerdict::Final { outcome: Outcome::TimedOut, cause: Some(c) } => {
                assert!(c.contains("900s"), "{c}");
            }
            other => panic!("expected timed-out, got {other:?}"),
        }
        assert!(matches!(
            attempt_verdict(1, AttemptEnd::StartupHang, "", t, s),
            AttemptVerdict::Retry { .. }
        ));
        // Captured: Quint exited after the server started, before any pass.
        let reflection = "Starting checker server on port 36103...   I@22:42:50.782\n\
                          The Apalache server is running on port 36103. Press Ctrl-C to stop.\n\
                          error: Error querying reflection endpoint: Error: 4 DEADLINE_EXCEEDED\n";
        let failed = AttemptEnd::Exited { success: false };
        assert!(matches!(
            attempt_verdict(1, failed, reflection, t, s),
            AttemptVerdict::Retry { .. }
        ));
        match attempt_verdict(2, failed, reflection, t, s) {
            AttemptVerdict::Final { outcome: Outcome::ToolError, cause: Some(c) } => {
                assert!(c.contains("start-up failed"), "{c}");
            }
            other => panic!("expected tool error, got {other:?}"),
        }
        // A failure after the first pass, or one that never started the
        // server (a spec error), is final on the first attempt.
        let crashed = format!("{reflection}PASS #0: SanyParser\nerror: boom\n");
        assert!(matches!(
            attempt_verdict(1, failed, &crashed, t, s),
            AttemptVerdict::Final { outcome: Outcome::ToolError, .. }
        ));
        assert!(matches!(
            attempt_verdict(1, failed, NAME_ERROR_OUT, t, s),
            AttemptVerdict::Final { outcome: Outcome::ToolError, .. }
        ));
        // A counterexample is never retried, whatever precedes it.
        assert!(matches!(
            attempt_verdict(1, failed, &format!("{reflection}{VIOLATION_OUT}"), t, s),
            AttemptVerdict::Final { outcome: Outcome::Violation, .. }
        ));
        match attempt_verdict(2, AttemptEnd::StartupHang, "", t, s) {
            AttemptVerdict::Final { outcome: Outcome::ToolError, cause: Some(c) } => {
                assert!(c.contains("never became ready"), "{c}");
            }
            other => panic!("expected tool error, got {other:?}"),
        }
    }

    #[test]
    fn judge_never_passes_timeout_or_interrupt() {
        for expect in [Expect::Holds, Expect::Violation] {
            assert!(!judge(expect, Outcome::TimedOut));
            assert!(!judge(expect, Outcome::Interrupted));
        }
    }

    #[test]
    fn estimate_spreads_known_work_and_counts_unknown() {
        assert_eq!(estimate_secs(&[Some(60.0), Some(60.0), None], 2), (60.0, 1));
        assert_eq!(estimate_secs(&[Some(10.0), Some(10.0), Some(10.0), Some(10.0)], 2), (20.0, 0));
        // The longest check bounds the estimate from below.
        assert_eq!(estimate_secs(&[Some(300.0), Some(10.0)], 4), (300.0, 0));
        assert_eq!(estimate_secs(&[None, None], 3), (0.0, 2));
    }

    #[test]
    fn parse_stat_state_handles_spaces_and_parens_in_comm() {
        assert_eq!(parse_stat_state("1234 (java) S 1 1200 1200 0 -1"), Some('S'));
        assert_eq!(parse_stat_state("77 (a b) c)) Z 5 66 66 0"), Some('Z'));
        assert_eq!(parse_stat_state("garbage"), None);
    }

    #[test]
    fn outcome_labels_are_distinct() {
        let all = [
            Outcome::Holds,
            Outcome::Violation,
            Outcome::ToolError,
            Outcome::TimedOut,
            Outcome::Interrupted,
        ];
        let labels: BTreeSet<&str> = all.iter().map(|o| o.as_str()).collect();
        assert_eq!(labels.len(), all.len());
        assert_eq!(serde_json::to_string(&Outcome::TimedOut).expect("json"), "\"timed-out\"");
    }

    #[test]
    fn collect_qnt_skips_hidden_and_underscore_dirs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        for p in ["a.qnt", "sub/b.qnt", "sub/notes.md", ".tools/c.qnt", "_apalache-out/d.qnt"] {
            let path = tmp.path().join(p);
            fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            fs::write(&path, "").expect("write");
        }
        let mut found = Vec::new();
        collect_qnt(tmp.path(), &mut found).expect("walk");
        let rel: Vec<_> = found
            .iter()
            .map(|p| p.strip_prefix(tmp.path()).expect("prefix").to_path_buf())
            .collect();
        assert_eq!(rel, vec![PathBuf::from("a.qnt"), PathBuf::from("sub/b.qnt")]);
    }
}
