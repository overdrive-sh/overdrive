//! `specs/quint/<subsystem>/evidence/` — the recorded evidence of the latest
//! full run of a subsystem's checks against its current specs.
//!
//! `cargo xtask quint check --subsystem <dir> --record` is the only writer.
//! At the end of a completed run (whatever the verdicts) it builds the new
//! evidence in a hidden staging directory beside `evidence/` and swaps it in,
//! so `evidence/` never holds a mix of two runs:
//!
//! ```text
//! evidence/
//!   summary.json              tool versions, kernel, git HEAD + dirty flag,
//!                             sha256 of every .qnt and checks.toml, times,
//!                             jobs, and one record per check
//!   summary.md                the same as a table
//!   traces/<check>.itf.json   Apalache counterexample (ITF), every check
//!                             whose outcome is a counterexample
//!   traces/<check>.tlc.txt    TLC counterexample (TLC's own text; Quint
//!                             0.32 writes no ITF for the TLC backend)
//!   traces/<check>.txt        condensed state sequence of either
//!   failures/<check>.log      every attempt's full log + events.log, only
//!                             for checks whose outcome missed `expect`
//! ```
//!
//! Full logs of passing checks stay under `target/quint/` only. Earlier runs
//! live in git history, not beside the current one.
//!
//! `cargo xtask quint verify-evidence` recomputes the input hashes and fails
//! when `summary.json` is stale or any recorded outcome misses its `expect`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use color_eyre::eyre::{Context, Result, bail, eyre};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::trace;
use super::{
    Backend, CHECKS_FILE, Check, CheckFilter, CheckRecord, Expect, Job, Outcome, Property,
    collect_qnt, judge,
};

/// Evidence directory name inside a subsystem directory.
pub const EVIDENCE_DIR: &str = "evidence";
/// Machine-readable summary file inside [`EVIDENCE_DIR`].
pub const SUMMARY_JSON: &str = "summary.json";
/// Human-readable summary file inside [`EVIDENCE_DIR`].
pub const SUMMARY_MD: &str = "summary.md";
/// Version of the `summary.json` schema this runner writes and reads.
pub const SCHEMA_VERSION: u32 = 1;

/// `--record` combined with a selection that is not one full subsystem.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecordArgsError {
    /// No `--subsystem`.
    #[error("--record needs --subsystem <dir>: evidence belongs to exactly one subsystem")]
    NoSubsystem,
    /// `--name` given.
    #[error("--record records a full run of the subsystem; it cannot be combined with --name")]
    Name,
    /// `--ci` given.
    #[error("--record records a full run of the subsystem; it cannot be combined with --ci")]
    Ci,
}

/// `--record` is allowed only for a full run of one subsystem.
pub const fn validate_record_args(filter: &CheckFilter) -> Result<(), RecordArgsError> {
    if filter.subsystem.is_none() {
        Err(RecordArgsError::NoSubsystem)
    } else if filter.name.is_some() {
        Err(RecordArgsError::Name)
    } else if filter.ci {
        Err(RecordArgsError::Ci)
    } else {
        Ok(())
    }
}

/// `evidence/summary.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceSummary {
    /// [`SCHEMA_VERSION`] at write time.
    pub schema: u32,
    /// Subsystem directory name.
    pub subsystem: String,
    /// The runner's run id.
    pub run_id: String,
    /// Run start, RFC 3339 UTC.
    pub started: String,
    /// Run end, RFC 3339 UTC.
    pub finished: String,
    /// Concurrent checks (`--jobs`).
    pub jobs: usize,
    /// Tool versions and kernel.
    pub tools: Tools,
    /// Repository state.
    pub git: GitState,
    /// sha256 (hex) of `checks.toml` and every `.qnt`, keyed by path
    /// relative to the subsystem directory.
    pub inputs: BTreeMap<String, String>,
    /// One record per check, in `checks.toml` order.
    pub checks: Vec<CheckEvidence>,
}

/// Tool versions of a recorded run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
    /// `quint --version`.
    pub quint: String,
    /// Apalache version from the check logs; `None` when no check reached
    /// the Apalache server.
    pub apalache: Option<String>,
    /// First line of `java -version`.
    pub java: String,
    /// `uname -r`.
    pub kernel: String,
}

/// Repository state of a recorded run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitState {
    /// `git rev-parse HEAD`.
    pub head: String,
    /// Whether the worktree had uncommitted changes (`git status --porcelain`).
    pub dirty: bool,
}

/// One check of a recorded run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckEvidence {
    /// Check name.
    pub name: String,
    /// Spec file, relative to the subsystem directory.
    pub spec: String,
    /// Main module.
    pub main: String,
    /// `invariant` or `temporal`.
    pub property_kind: String,
    /// Property name.
    pub property: String,
    /// Checker backend.
    pub backend: Backend,
    /// `max_steps=N`, `max_steps=quint-default`, or `exhaustive` (TLC).
    pub bound: String,
    /// Expected outcome.
    pub expect: Expect,
    /// Observed outcome.
    pub outcome: Outcome,
    /// Whether `outcome` matched `expect`.
    pub pass: bool,
    /// Attempts made (a start-up failure is retried once).
    pub attempts: u32,
    /// Wall-clock seconds from the first attempt's start.
    pub seconds: f64,
    /// Per-attempt timeout in seconds.
    pub timeout_secs: u64,
    /// Cause of a non-result outcome.
    pub cause: Option<String>,
    /// Counterexample trace (relative to `evidence/`).
    pub trace: Option<String>,
    /// Condensed counterexample (relative to `evidence/`).
    pub condensed: Option<String>,
    /// Full logs of a failed check (relative to `evidence/`).
    pub failure_log: Option<String>,
}

/// sha256 (hex) of `checks.toml` and every `.qnt` under `dir` (skipping
/// hidden and `_`-prefixed directories and `evidence/`), keyed by the path
/// relative to `dir` with `/` separators.
pub fn hash_inputs(dir: &Path) -> Result<BTreeMap<String, String>> {
    let mut files = Vec::new();
    collect_qnt(dir, &mut files)?;
    files.push(dir.join(CHECKS_FILE));
    let mut out = BTreeMap::new();
    for path in files {
        let rel = path.strip_prefix(dir).wrap_err("input outside the subsystem dir")?;
        if rel.components().next().is_some_and(|c| c.as_os_str() == EVIDENCE_DIR) {
            continue;
        }
        let key = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let bytes = fs::read(&path).wrap_err_with(|| format!("reading {}", path.display()))?;
        out.insert(key, hex::encode(Sha256::digest(&bytes)));
    }
    Ok(out)
}

/// The `# APALACHE version: …` banner of a check log.
pub fn apalache_version(log: &str) -> Option<String> {
    log.lines().find_map(|l| {
        let rest = l.strip_prefix("# APALACHE version:")?;
        // The banner ends in padding plus a log timestamp (`I@hh:mm:ss`).
        Some(rest.split("  ").next().unwrap_or(rest).trim().to_owned())
    })
}

/// Escape a Markdown table cell.
fn cell(s: &str) -> String {
    s.replace('|', "\\|")
}

/// Render `summary.md`.
pub fn render_md(s: &EvidenceSummary) -> String {
    let mut out = String::new();
    let passed = s.checks.iter().filter(|c| c.pass).count();
    let _ = writeln!(out, "# Quint evidence — `{}`\n", s.subsystem);
    let _ = writeln!(
        out,
        "Written by `cargo xtask quint check --subsystem {} --record`; do not edit. \
         `cargo xtask quint verify-evidence --subsystem {}` checks it against the specs.\n",
        s.subsystem, s.subsystem
    );
    let _ = writeln!(out, "| | |\n|---|---|");
    let _ = writeln!(out, "| Result | {passed} of {} check(s) matched `expect` |", s.checks.len());
    let _ = writeln!(out, "| Run | `{}`, {} → {} |", s.run_id, s.started, s.finished);
    let _ = writeln!(out, "| Jobs | {} |", s.jobs);
    let _ = writeln!(
        out,
        "| Git | `{}`{} |",
        s.git.head,
        if s.git.dirty { " (dirty worktree)" } else { "" }
    );
    let _ = writeln!(out, "| Quint | {} |", cell(&s.tools.quint));
    let _ = writeln!(out, "| Apalache | {} |", cell(s.tools.apalache.as_deref().unwrap_or("—")));
    let _ = writeln!(out, "| Java | {} |", cell(&s.tools.java));
    let _ = writeln!(out, "| Kernel | {} |", cell(&s.tools.kernel));
    let _ = writeln!(
        out,
        "\n| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | \
         Attempts | Seconds | Evidence |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|---:|---:|---|");
    for c in &s.checks {
        let mut files: Vec<String> = Vec::new();
        for f in [&c.condensed, &c.trace, &c.failure_log].into_iter().flatten() {
            files.push(format!("[{f}]({f})"));
        }
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} `{}` | {} | {} | {} | {} | {} | {} | {:.1} | {} |",
            c.name,
            c.spec,
            c.main,
            c.property_kind,
            c.property,
            c.backend.as_str(),
            c.bound,
            c.expect.as_str(),
            c.outcome.as_str(),
            if c.pass { "PASS" } else { "**FAIL**" },
            c.attempts,
            c.seconds,
            files.join(" "),
        );
    }
    let failed: Vec<&CheckEvidence> = s.checks.iter().filter(|c| !c.pass).collect();
    if !failed.is_empty() {
        let _ = writeln!(out, "\n## Checks that missed `expect`\n");
        for c in failed {
            let _ = writeln!(
                out,
                "- `{}`: expected {}, got {}{}",
                c.name,
                c.expect.as_str(),
                c.outcome.as_str(),
                c.cause.as_deref().map_or(String::new(), |cause| format!(" — {cause}"))
            );
        }
    }
    let _ = writeln!(out, "\n## Inputs (sha256)\n");
    for (file, sha) in &s.inputs {
        let _ = writeln!(out, "- `{file}` `{sha}`");
    }
    out
}

/// Why recorded evidence does not stand for the current specs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvidenceProblem {
    /// `summary.json` uses another schema.
    #[error("summary.json has schema {found}, this runner reads {SCHEMA_VERSION}")]
    Schema {
        /// Schema found.
        found: u32,
    },
    /// An input changed since the recorded run.
    #[error("stale: `{file}` changed since the recorded run")]
    InputChanged {
        /// Input path.
        file: String,
    },
    /// An input appeared since the recorded run.
    #[error("stale: `{file}` was added after the recorded run")]
    InputAdded {
        /// Input path.
        file: String,
    },
    /// An input disappeared since the recorded run.
    #[error("stale: `{file}` was removed after the recorded run")]
    InputRemoved {
        /// Input path.
        file: String,
    },
    /// A check in `checks.toml` has no record.
    #[error("check `{name}` in checks.toml has no recorded outcome")]
    CheckNotRecorded {
        /// Check name.
        name: String,
    },
    /// A record names a check `checks.toml` no longer has.
    #[error("recorded check `{name}` is not in checks.toml")]
    CheckNotInManifest {
        /// Check name.
        name: String,
    },
    /// A recorded outcome does not match its `expect`.
    #[error("check `{name}`: expected {expect}, recorded outcome {outcome}")]
    OutcomeMismatch {
        /// Check name.
        name: String,
        /// Expected label.
        expect: &'static str,
        /// Outcome label.
        outcome: &'static str,
    },
    /// A recorded counterexample has no trace.
    #[error("check `{name}`: counterexample recorded without a trace")]
    MissingTrace {
        /// Check name.
        name: String,
    },
    /// A file the summary references is absent.
    #[error("`{file}` is referenced by summary.json but missing")]
    MissingFile {
        /// Path relative to `evidence/`.
        file: String,
    },
}

/// Compare a recorded summary with the current inputs, manifest and evidence files.
///
/// `files` are paths relative to `evidence/`. Empty means the evidence
/// stands for the current specs and every recorded outcome matched.
pub fn verify_summary(
    summary: &EvidenceSummary,
    inputs: &BTreeMap<String, String>,
    checks: &[Check],
    files: &BTreeSet<String>,
) -> Vec<EvidenceProblem> {
    let mut problems = Vec::new();
    if summary.schema != SCHEMA_VERSION {
        problems.push(EvidenceProblem::Schema { found: summary.schema });
    }
    for (file, sha) in inputs {
        match summary.inputs.get(file) {
            None => problems.push(EvidenceProblem::InputAdded { file: file.clone() }),
            Some(rec) if rec != sha => {
                problems.push(EvidenceProblem::InputChanged { file: file.clone() });
            }
            Some(_) => {}
        }
    }
    for file in summary.inputs.keys().filter(|f| !inputs.contains_key(*f)) {
        problems.push(EvidenceProblem::InputRemoved { file: file.clone() });
    }
    let recorded: BTreeSet<&str> = summary.checks.iter().map(|c| c.name.as_str()).collect();
    for c in checks.iter().filter(|c| !recorded.contains(c.name.as_str())) {
        problems.push(EvidenceProblem::CheckNotRecorded { name: c.name.clone() });
    }
    for c in &summary.checks {
        if !checks.iter().any(|m| m.name == c.name) {
            problems.push(EvidenceProblem::CheckNotInManifest { name: c.name.clone() });
        }
        // Recompute rather than trust the recorded `pass`.
        if !judge(c.expect, c.outcome) {
            problems.push(EvidenceProblem::OutcomeMismatch {
                name: c.name.clone(),
                expect: c.expect.as_str(),
                outcome: c.outcome.as_str(),
            });
        }
        if c.outcome == Outcome::Violation && (c.trace.is_none() || c.condensed.is_none()) {
            problems.push(EvidenceProblem::MissingTrace { name: c.name.clone() });
        }
        for f in [&c.trace, &c.condensed, &c.failure_log].into_iter().flatten() {
            if !files.contains(f) {
                problems.push(EvidenceProblem::MissingFile { file: f.clone() });
            }
        }
    }
    problems
}

/// Every file under `dir`, relative, with `/` separators.
fn list_files(dir: &Path) -> Result<BTreeSet<String>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeSet<String>) -> Result<()> {
        for entry in fs::read_dir(dir).wrap_err_with(|| format!("reading {}", dir.display()))? {
            let entry = entry.wrap_err_with(|| format!("reading {}", dir.display()))?;
            let path = entry.path();
            if entry.file_type().wrap_err_with(|| format!("stat {}", path.display()))?.is_dir() {
                walk(base, &path, out)?;
            } else {
                let rel = path.strip_prefix(base).wrap_err("path outside evidence dir")?;
                out.insert(
                    rel.components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/"),
                );
            }
        }
        Ok(())
    }
    let mut out = BTreeSet::new();
    walk(dir, dir, &mut out)?;
    Ok(out)
}

/// Check one subsystem's evidence against its current specs; returns the
/// problems found (empty = current and passing).
pub fn verify_dir(dir: &Path, checks: &[Check]) -> Result<Vec<String>> {
    let evidence = dir.join(EVIDENCE_DIR);
    let summary_path = evidence.join(SUMMARY_JSON);
    let body = match fs::read_to_string(&summary_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(vec![format!(
                "no {} — record it with `cargo xtask quint check --subsystem <dir> --record`",
                summary_path.display()
            )]);
        }
        Err(e) => return Err(eyre!("reading {}: {e}", summary_path.display())),
    };
    let summary: EvidenceSummary = match serde_json::from_str(&body) {
        Ok(s) => s,
        Err(e) => {
            return Ok(vec![format!("{} is not a valid summary: {e}", summary_path.display())]);
        }
    };
    let problems = verify_summary(&summary, &hash_inputs(dir)?, checks, &list_files(&evidence)?);
    Ok(problems.iter().map(ToString::to_string).collect())
}

// ---------------------------------------------------------------------------
// Recording
// ---------------------------------------------------------------------------

fn run_tool(cmd: &mut Command, what: &str) -> Result<std::process::Output> {
    let out = cmd.output().map_err(|e| eyre!("running {what}: {e}"))?;
    if !out.status.success() {
        bail!(
            "{what} exited with {}: {}{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(out)
}

fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).lines().next().unwrap_or_default().trim().to_owned()
}

fn git(root: &Path, args: &[&str]) -> Result<std::process::Output> {
    // `cargo xtask lima run` runs as root against a tree the lima user owns;
    // without `safe.directory` git refuses the repository.
    let safe = format!("safe.directory={}", root.display());
    run_tool(
        Command::new("git").arg("-c").arg(&safe).arg("-C").arg(root).args(args),
        &format!("git {}", args.join(" ")),
    )
}

fn rfc3339(unix: f64) -> Result<String> {
    #[allow(clippy::cast_possible_truncation)]
    let nanos = (unix * 1e9) as i128;
    time::OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .wrap_err("timestamp out of range")?
        .format(&time::format_description::well_known::Rfc3339)
        .wrap_err("formatting timestamp")
}

/// Inputs of [`record`] that describe the run as a whole.
pub(super) struct RunMeta<'a> {
    pub root: &'a Path,
    pub subsystem: &'a str,
    pub dir: &'a Path,
    pub run_id: &'a str,
    pub started_unix: f64,
    pub finished_unix: f64,
    pub jobs: usize,
    /// Input hashes taken before the run.
    pub inputs: BTreeMap<String, String>,
}

/// One file of the new evidence, relative to the evidence dir.
type Staged = Vec<(String, Vec<u8>)>;

fn attempt_logs(job: &Job, attempts: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for n in 1..=attempts {
        let path = job.out_dir.join(format!("attempt-{n}.log"));
        let _ = writeln!(Fmt(&mut out), "===== attempt {n}: {} =====", path.display());
        out.extend(fs::read(&path).wrap_err_with(|| format!("reading {}", path.display()))?);
        out.push(b'\n');
    }
    let events = job.out_dir.join("events.log");
    let _ = writeln!(Fmt(&mut out), "===== {} =====", events.display());
    out.extend(fs::read(&events).wrap_err_with(|| format!("reading {}", events.display()))?);
    Ok(out)
}

/// `fmt::Write` over a byte buffer.
struct Fmt<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for Fmt<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

fn check_evidence(job: &Job, rec: &CheckRecord, staged: &mut Staged) -> Result<CheckEvidence> {
    let c = &job.check;
    let (property_kind, property) = match &c.property {
        Property::Invariant(p) => ("invariant", p.clone()),
        Property::Temporal(p) => ("temporal", p.clone()),
    };
    let bound = match (c.backend, c.max_steps) {
        (Backend::Tlc, _) => "exhaustive".to_owned(),
        (Backend::Apalache, Some(n)) => format!("max_steps={n}"),
        (Backend::Apalache, None) => "max_steps=quint-default".to_owned(),
    };
    let mut ev = CheckEvidence {
        name: c.name.clone(),
        spec: c.spec.to_string_lossy().replace('\\', "/"),
        main: c.main.clone(),
        property_kind: property_kind.to_owned(),
        property,
        backend: c.backend,
        bound,
        expect: c.expect,
        outcome: rec.outcome,
        pass: rec.pass,
        attempts: rec.attempts,
        seconds: (rec.seconds * 1000.0).round() / 1000.0,
        timeout_secs: rec.timeout_secs,
        cause: rec.cause.clone(),
        trace: None,
        condensed: None,
        failure_log: None,
    };
    if rec.outcome == Outcome::Violation {
        let (raw_name, raw, parsed, source) = match c.backend {
            Backend::Apalache => {
                let itf = job.out_dir.join("trace.itf.json");
                let body = fs::read_to_string(&itf).wrap_err_with(|| {
                    format!("check `{}`: counterexample without {}", c.name, itf.display())
                })?;
                let parsed = trace::parse_itf(&body)
                    .wrap_err_with(|| format!("check `{}`: {}", c.name, itf.display()))?;
                (format!("traces/{}.itf.json", c.name), body, parsed, "ITF")
            }
            Backend::Tlc => {
                let log = fs::read_to_string(&rec.log)
                    .wrap_err_with(|| format!("reading {}", rec.log.display()))?;
                let text = trace::extract_tlc_trace(&log).ok_or_else(|| {
                    eyre!("check `{}`: no TLC counterexample in {}", c.name, rec.log.display())
                })?;
                let parsed = trace::parse_tlc(&text)
                    .wrap_err_with(|| format!("check `{}`: {}", c.name, rec.log.display()))?;
                (format!("traces/{}.tlc.txt", c.name), text, parsed, "TLC")
            }
        };
        let condensed_name = format!("traces/{}.txt", c.name);
        staged.push((condensed_name.clone(), trace::condense(&c.name, source, &parsed).into()));
        staged.push((raw_name.clone(), raw.into_bytes()));
        ev.trace = Some(raw_name);
        ev.condensed = Some(condensed_name);
    }
    if !rec.pass {
        let name = format!("failures/{}.log", c.name);
        staged.push((name.clone(), attempt_logs(job, rec.attempts)?));
        ev.failure_log = Some(name);
    }
    Ok(ev)
}

/// Build the evidence of a completed run and swap it into
/// `<dir>/evidence/`.
pub(super) fn record(meta: &RunMeta<'_>, jobs: &[Job], records: &[CheckRecord]) -> Result<()> {
    let after = hash_inputs(meta.dir)?;
    if after != meta.inputs {
        bail!(
            "{} changed while the checks ran; evidence not recorded (re-run on unchanged specs)",
            meta.dir.display()
        );
    }
    let mut staged: Staged = Vec::new();
    let mut checks = Vec::with_capacity(jobs.len());
    let mut apalache = None;
    for (job, rec) in jobs.iter().zip(records) {
        checks.push(check_evidence(job, rec, &mut staged)?);
        if apalache.is_none() {
            let log = fs::read_to_string(&rec.log)
                .wrap_err_with(|| format!("reading {}", rec.log.display()))?;
            apalache = apalache_version(&log);
        }
    }
    let quint =
        first_line(&run_tool(Command::new("quint").arg("--version"), "quint --version")?.stdout);
    let java_out = run_tool(Command::new("java").arg("-version"), "java -version")?;
    let java =
        first_line(if java_out.stderr.is_empty() { &java_out.stdout } else { &java_out.stderr });
    let kernel = first_line(&run_tool(Command::new("uname").arg("-r"), "uname -r")?.stdout);
    let head = first_line(&git(meta.root, &["rev-parse", "HEAD"])?.stdout);
    let dirty = !git(meta.root, &["status", "--porcelain"])?.stdout.is_empty();

    let summary = EvidenceSummary {
        schema: SCHEMA_VERSION,
        subsystem: meta.subsystem.to_owned(),
        run_id: meta.run_id.to_owned(),
        started: rfc3339(meta.started_unix)?,
        finished: rfc3339(meta.finished_unix)?,
        jobs: meta.jobs,
        tools: Tools { quint, apalache, java, kernel },
        git: GitState { head, dirty },
        inputs: after,
        checks,
    };
    let json = serde_json::to_string_pretty(&summary).wrap_err("encoding summary.json")? + "\n";
    staged.push((SUMMARY_JSON.to_owned(), json.into_bytes()));
    staged.push((SUMMARY_MD.to_owned(), render_md(&summary).into_bytes()));
    install(meta.dir, meta.run_id, &staged)?;
    eprintln!(
        "xtask quint check: recorded {} check(s) into {}",
        summary.checks.len(),
        meta.dir.join(EVIDENCE_DIR).display()
    );
    Ok(())
}

/// Write `files` into a hidden staging dir beside `<dir>/evidence/`, then
/// swap it in place of the old evidence.
fn install(dir: &Path, run_id: &str, files: &Staged) -> Result<()> {
    let tag: String =
        run_id.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let stage = dir.join(format!(".evidence.new-{tag}"));
    let old = dir.join(format!(".evidence.old-{tag}"));
    let target = dir.join(EVIDENCE_DIR);
    if stage.exists() {
        fs::remove_dir_all(&stage).wrap_err_with(|| format!("clearing {}", stage.display()))?;
    }
    let written = (|| -> Result<()> {
        for (rel, bytes) in files {
            let path: PathBuf = stage.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .wrap_err_with(|| format!("creating {}", parent.display()))?;
            }
            fs::write(&path, bytes).wrap_err_with(|| format!("writing {}", path.display()))?;
        }
        Ok(())
    })();
    if let Err(err) = written {
        let _ = fs::remove_dir_all(&stage);
        return Err(err);
    }
    if target.exists() {
        fs::rename(&target, &old).wrap_err_with(|| format!("moving {} aside", target.display()))?;
        if let Err(err) = fs::rename(&stage, &target) {
            fs::rename(&old, &target).wrap_err_with(|| {
                format!("restoring {} after a failed swap ({err})", target.display())
            })?;
            return Err(eyre!("installing {}: {err}", target.display()));
        }
        fs::remove_dir_all(&old).wrap_err_with(|| format!("removing {}", old.display()))?;
    } else {
        fs::rename(&stage, &target).wrap_err_with(|| format!("installing {}", target.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(name: &str, expect: Expect) -> Check {
        Check {
            name: name.into(),
            spec: "a.qnt".into(),
            main: "m".into(),
            property: Property::Invariant("Inv".into()),
            backend: Backend::Apalache,
            max_steps: Some(4),
            expect,
            ci: false,
            timeout_secs: None,
            heap_mb: None,
            workers: None,
        }
    }

    fn rec(name: &str, expect: Expect, outcome: Outcome) -> CheckEvidence {
        let violation = outcome == Outcome::Violation;
        let pass = judge(expect, outcome);
        CheckEvidence {
            name: name.into(),
            spec: "a.qnt".into(),
            main: "m".into(),
            property_kind: "invariant".into(),
            property: "Inv".into(),
            backend: Backend::Apalache,
            bound: "max_steps=4".into(),
            expect,
            outcome,
            pass,
            attempts: 1,
            seconds: 1.5,
            timeout_secs: 900,
            cause: None,
            trace: violation.then(|| format!("traces/{name}.itf.json")),
            condensed: violation.then(|| format!("traces/{name}.txt")),
            failure_log: (!pass).then(|| format!("failures/{name}.log")),
        }
    }

    fn summary(checks: Vec<CheckEvidence>) -> EvidenceSummary {
        EvidenceSummary {
            schema: SCHEMA_VERSION,
            subsystem: "sub".into(),
            run_id: "1-2.000".into(),
            started: "2026-10-06T22:00:00Z".into(),
            finished: "2026-10-06T22:01:00Z".into(),
            jobs: 2,
            tools: Tools {
                quint: "0.32.0".into(),
                apalache: Some("0.56.1 | build: 70cdaf4".into()),
                java: "openjdk version \"21\"".into(),
                kernel: "7.0.0".into(),
            },
            git: GitState { head: "abc".into(), dirty: true },
            inputs: BTreeMap::from([
                ("a.qnt".into(), "11".into()),
                ("checks.toml".into(), "22".into()),
            ]),
            checks,
        }
    }

    fn files(s: &EvidenceSummary) -> BTreeSet<String> {
        s.checks
            .iter()
            .flat_map(|c| [&c.trace, &c.condensed, &c.failure_log].into_iter().flatten().cloned())
            .collect()
    }

    #[test]
    fn record_args_need_one_full_subsystem() {
        let ok = CheckFilter { ci: false, subsystem: Some("s".into()), name: None };
        assert_eq!(validate_record_args(&ok), Ok(()));
        assert_eq!(
            validate_record_args(&CheckFilter::default()),
            Err(RecordArgsError::NoSubsystem)
        );
        assert_eq!(
            validate_record_args(&CheckFilter { name: Some("x".into()), ..ok.clone() }),
            Err(RecordArgsError::Name)
        );
        assert_eq!(validate_record_args(&CheckFilter { ci: true, ..ok }), Err(RecordArgsError::Ci));
        assert!(RecordArgsError::NoSubsystem.to_string().contains("--subsystem"));
    }

    #[test]
    fn summary_json_roundtrips_and_pins_field_names() {
        let s = summary(vec![
            rec("w", Expect::Violation, Outcome::Violation),
            rec("t", Expect::Holds, Outcome::TimedOut),
        ]);
        let json = serde_json::to_string_pretty(&s).expect("encode");
        assert_eq!(serde_json::from_str::<EvidenceSummary>(&json).expect("decode"), s);
        let v: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(v["checks"][0]["outcome"], "violation");
        assert_eq!(v["checks"][1]["outcome"], "timed-out");
        assert_eq!(v["checks"][0]["backend"], "apalache");
        assert_eq!(v["checks"][0]["trace"], "traces/w.itf.json");
        assert_eq!(v["git"]["dirty"], true);
        assert_eq!(v["inputs"]["checks.toml"], "22");
        // Unknown fields are refused, so a hand-edited summary cannot slip through.
        let tampered = json.replacen("\"jobs\"", "\"note\": 1,\n  \"jobs\"", 1);
        assert!(serde_json::from_str::<EvidenceSummary>(&tampered).is_err());
    }

    #[test]
    fn summary_md_lists_every_check_and_input() {
        let s = summary(vec![
            rec("w", Expect::Violation, Outcome::Violation),
            rec("h", Expect::Holds, Outcome::ToolError),
        ]);
        let md = render_md(&s);
        assert!(md.contains("| Result | 1 of 2 check(s) matched `expect` |"), "{md}");
        assert!(md.contains("| w | a.qnt | m | invariant `Inv` | apalache | max_steps=4 | violation | violation | PASS | 1 | 1.5 | [traces/w.txt](traces/w.txt) [traces/w.itf.json](traces/w.itf.json) |"), "{md}");
        assert!(md.contains("| h | a.qnt | m | invariant `Inv` | apalache | max_steps=4 | holds | tool-error | **FAIL** | 1 | 1.5 | [failures/h.log](failures/h.log) |"), "{md}");
        assert!(md.contains("- `checks.toml` `22`"), "{md}");
        assert!(md.contains("(dirty worktree)"), "{md}");
        // A `|` inside a value must not split the table cell.
        assert!(md.contains("| Apalache | 0.56.1 \\| build: 70cdaf4 |"), "{md}");
        assert!(md.contains("- `h`: expected holds, got tool-error"), "{md}");
    }

    #[test]
    fn current_passing_evidence_has_no_problems() {
        let s = summary(vec![
            rec("w", Expect::Violation, Outcome::Violation),
            rec("h", Expect::Holds, Outcome::Holds),
        ]);
        let checks = [check("w", Expect::Violation), check("h", Expect::Holds)];
        assert_eq!(verify_summary(&s, &s.inputs.clone(), &checks, &files(&s)), vec![]);
    }

    #[test]
    fn stale_inputs_are_reported_per_file() {
        let s = summary(vec![rec("h", Expect::Holds, Outcome::Holds)]);
        let mut now = s.inputs.clone();
        now.insert("a.qnt".into(), "changed".into());
        now.insert("hazard/b.qnt".into(), "33".into());
        now.remove("checks.toml");
        assert_eq!(
            verify_summary(&s, &now, &[check("h", Expect::Holds)], &files(&s)),
            vec![
                EvidenceProblem::InputChanged { file: "a.qnt".into() },
                EvidenceProblem::InputAdded { file: "hazard/b.qnt".into() },
                EvidenceProblem::InputRemoved { file: "checks.toml".into() },
            ]
        );
    }

    #[test]
    fn mismatched_outcomes_and_check_sets_are_reported() {
        let mut lying = rec("x", Expect::Holds, Outcome::Violation);
        lying.pass = true; // the recorded flag is not trusted
        let s = summary(vec![lying, rec("t", Expect::Violation, Outcome::TimedOut)]);
        let checks = [check("t", Expect::Violation), check("new", Expect::Holds)];
        assert_eq!(
            verify_summary(&s, &s.inputs.clone(), &checks, &files(&s)),
            vec![
                EvidenceProblem::CheckNotRecorded { name: "new".into() },
                EvidenceProblem::CheckNotInManifest { name: "x".into() },
                EvidenceProblem::OutcomeMismatch {
                    name: "x".into(),
                    expect: "holds",
                    outcome: "violation"
                },
                EvidenceProblem::OutcomeMismatch {
                    name: "t".into(),
                    expect: "violation",
                    outcome: "timed-out"
                },
            ]
        );
    }

    #[test]
    fn missing_trace_files_and_schema_are_reported() {
        let mut s = summary(vec![rec("w", Expect::Violation, Outcome::Violation)]);
        s.schema = 99;
        let mut present = files(&s);
        present.remove("traces/w.itf.json");
        let mut no_trace = s.clone();
        no_trace.checks[0].trace = None;
        let checks = [check("w", Expect::Violation)];
        assert_eq!(
            verify_summary(&s, &s.inputs.clone(), &checks, &present),
            vec![
                EvidenceProblem::Schema { found: 99 },
                EvidenceProblem::MissingFile { file: "traces/w.itf.json".into() },
            ]
        );
        assert!(
            verify_summary(&no_trace, &s.inputs.clone(), &checks, &files(&s))
                .contains(&EvidenceProblem::MissingTrace { name: "w".into() })
        );
    }

    #[test]
    fn hash_inputs_covers_specs_and_manifest_but_not_evidence() {
        let tmp = tempfile::tempdir().expect("tempdir");
        for (p, body) in [
            ("checks.toml", "x"),
            ("a.qnt", "module a {}"),
            ("hazard/b.qnt", "module b {}"),
            ("evidence/old.qnt", "stale copy"),
            ("_apalache-out/c.qnt", "cache"),
            ("notes.md", "not an input"),
        ] {
            let path = tmp.path().join(p);
            fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            fs::write(&path, body).expect("write");
        }
        let hashes = hash_inputs(tmp.path()).expect("hash");
        assert_eq!(hashes.keys().collect::<Vec<_>>(), ["a.qnt", "checks.toml", "hazard/b.qnt"]);
        // sha256("x")
        assert_eq!(
            hashes["checks.toml"],
            "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881"
        );
        fs::write(tmp.path().join("a.qnt"), "module a { val x = 1 }").expect("write");
        assert_ne!(hash_inputs(tmp.path()).expect("hash")["a.qnt"], hashes["a.qnt"]);
    }

    #[test]
    fn apalache_version_reads_the_banner() {
        let log = "Output directory: /x\n\
                   # APALACHE version: 0.56.1 | build: 70cdaf4                       I@22:58:25.447\n";
        assert_eq!(apalache_version(log).as_deref(), Some("0.56.1 | build: 70cdaf4"));
        assert_eq!(apalache_version("no banner\n"), None);
    }

    #[test]
    fn install_replaces_the_whole_evidence_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ev = tmp.path().join(EVIDENCE_DIR);
        fs::create_dir_all(ev.join("round-2")).expect("mkdir");
        fs::write(ev.join("round-2/copied.sh"), "#!/bin/sh").expect("write");
        let files: Staged =
            vec![(SUMMARY_JSON.into(), b"{}".to_vec()), ("traces/w.txt".into(), b"trace".to_vec())];
        install(tmp.path(), "12-3.4", &files).expect("install");
        assert_eq!(
            list_files(&ev).expect("list"),
            BTreeSet::from([SUMMARY_JSON.to_owned(), "traces/w.txt".to_owned()])
        );
        let leftovers: Vec<_> = fs::read_dir(tmp.path())
            .expect("read")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert_eq!(leftovers, [std::ffi::OsString::from(EVIDENCE_DIR)]);
    }
}
