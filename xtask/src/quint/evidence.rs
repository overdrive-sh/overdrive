//! `specs/quint/<subsystem>/evidence/` — the recorded evidence of the latest
//! full run of a subsystem's checks against its current specs.
//!
//! `cargo xtask quint check --subsystem <dir> --record` is the only writer.
//! At the end of a completed run (whatever the verdicts) it builds the new
//! evidence in a hidden staging directory beside `evidence/` and swaps it in,
//! so `evidence/` never holds a mix of two runs. A counterexample whose trace
//! cannot be read, parsed or staged does not stop the recording: the check is
//! recorded with its outcome and a typed [`TraceRecordError`], its full log
//! goes to `failures/`, and every other check's evidence is still swapped in.
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
//!                             for checks whose outcome missed `expect` or
//!                             whose counterexample trace was not recorded
//! ```
//!
//! Full logs of passing checks stay under `target/quint/` only. Earlier runs
//! live in git history, not beside the current one.
//!
//! `cargo xtask quint verify-evidence` recomputes the input hashes and fails
//! when `summary.json` is stale, any recorded outcome misses its `expect`, or
//! a counterexample was recorded without its trace.

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
    /// Why the counterexample's trace was not recorded (`trace` and
    /// `condensed` are then `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_error: Option<TraceRecordError>,
}

/// Why a counterexample's trace was not recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum TraceRecordError {
    /// The checker's trace output (Apalache ITF file, TLC log) could not be read.
    #[error("cannot read {file}: {error}")]
    Unreadable {
        /// The file.
        file: String,
        /// The I/O error.
        error: String,
    },
    /// The output holds no counterexample this runner can parse.
    #[error("no parsable counterexample in {file}: {error}")]
    Unparsable {
        /// The file.
        file: String,
        /// The parse error.
        error: String,
    },
    /// The trace could not be written into the staged evidence.
    #[error("cannot write {file} into the staged evidence: {error}")]
    Unwritable {
        /// Path relative to `evidence/`.
        file: String,
        /// The I/O error.
        error: String,
    },
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
    let untraced: Vec<&CheckEvidence> =
        s.checks.iter().filter(|c| c.trace_error.is_some()).collect();
    let _ = writeln!(out, "# Quint evidence — `{}`\n", s.subsystem);
    let _ = writeln!(
        out,
        "Written by `cargo xtask quint check --subsystem {} --record`; do not edit. \
         `cargo xtask quint verify-evidence --subsystem {}` checks it against the specs.\n",
        s.subsystem, s.subsystem
    );
    let _ = writeln!(out, "| | |\n|---|---|");
    let _ = writeln!(
        out,
        "| Result | {passed} of {} check(s) matched `expect`{} |",
        s.checks.len(),
        if untraced.is_empty() {
            String::new()
        } else {
            format!("; **{} counterexample(s) without a recorded trace**", untraced.len())
        }
    );
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
        if c.trace_error.is_some() {
            files.push("**no trace**".to_owned());
        }
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
    if !untraced.is_empty() {
        let _ = writeln!(out, "\n## Counterexamples without a recorded trace\n");
        for c in untraced {
            let _ = writeln!(
                out,
                "- `{}`: {}{}",
                c.name,
                c.trace_error.as_ref().map_or(String::new(), |e| cell(&e.to_string())),
                c.failure_log
                    .as_deref()
                    .map_or(String::new(), |f| format!(" — full log: [{f}]({f})"))
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
    /// A recorded counterexample has no trace and no recorded reason.
    #[error("check `{name}`: counterexample recorded without a trace")]
    MissingTrace {
        /// Check name.
        name: String,
    },
    /// The recording run could not record a counterexample's trace.
    #[error(
        "check `{name}`: counterexample trace NOT recorded — {cause}; a reviewer cannot \
         inspect this counterexample (fix the cause, then re-record)"
    )]
    TraceNotRecorded {
        /// Check name.
        name: String,
        /// The recorded [`TraceRecordError`].
        cause: String,
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
        if let Some(err) = &c.trace_error {
            problems.push(EvidenceProblem::TraceNotRecorded {
                name: c.name.clone(),
                cause: err.to_string(),
            });
        } else if c.outcome == Outcome::Violation && (c.trace.is_none() || c.condensed.is_none()) {
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

/// The full logs of a check: every attempt's log, then `events.log`. A log
/// that cannot be read is named in the output with its error, so the copy
/// still carries whatever survived.
fn attempt_logs(job: &Job, attempts: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut append = |label: &str, path: &Path| {
        let _ = writeln!(Fmt(&mut out), "===== {label}: {} =====", path.display());
        match fs::read(path) {
            Ok(bytes) => out.extend(bytes),
            Err(e) => {
                let _ = writeln!(Fmt(&mut out), "(could not read {}: {e})", path.display());
            }
        }
        out.push(b'\n');
    };
    for n in 1..=attempts {
        append(&format!("attempt {n}"), &job.out_dir.join(format!("attempt-{n}.log")));
    }
    append("events", &job.out_dir.join("events.log"));
    out
}

/// `fmt::Write` over a byte buffer.
struct Fmt<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for Fmt<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

/// A hidden staging directory beside `<dir>/evidence/` that the new evidence
/// is written into and then swapped in. Removed on drop unless installed.
struct Stage {
    dir: PathBuf,
    old: PathBuf,
    target: PathBuf,
    installed: bool,
}

impl Stage {
    fn create(dir: &Path, run_id: &str) -> Result<Self> {
        let tag: String =
            run_id.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        let stage = Self {
            dir: dir.join(format!(".evidence.new-{tag}")),
            old: dir.join(format!(".evidence.old-{tag}")),
            target: dir.join(EVIDENCE_DIR),
            installed: false,
        };
        if stage.dir.exists() {
            fs::remove_dir_all(&stage.dir)
                .wrap_err_with(|| format!("clearing {}", stage.dir.display()))?;
        }
        fs::create_dir_all(&stage.dir)
            .wrap_err_with(|| format!("creating {}", stage.dir.display()))?;
        Ok(stage)
    }

    /// Write one file (path relative to `evidence/`).
    fn write(&self, rel: &str, bytes: &[u8]) -> std::io::Result<()> {
        let path = self.dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, bytes)
    }

    /// Write one file; a failure ends the recording.
    fn write_or_fail(&self, rel: &str, bytes: &[u8]) -> Result<()> {
        self.write(rel, bytes).wrap_err_with(|| format!("writing {}", self.dir.join(rel).display()))
    }

    /// Remove one staged file written before a later write of the same check
    /// failed. An absent file is fine (it was never written).
    fn unwrite(&self, rel: &str) -> std::io::Result<()> {
        match fs::remove_file(self.dir.join(rel)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Swap the staged directory in place of `evidence/`.
    fn install(mut self) -> Result<()> {
        if self.target.exists() {
            fs::rename(&self.target, &self.old)
                .wrap_err_with(|| format!("moving {} aside", self.target.display()))?;
            if let Err(err) = fs::rename(&self.dir, &self.target) {
                fs::rename(&self.old, &self.target).wrap_err_with(|| {
                    format!("restoring {} after a failed swap ({err})", self.target.display())
                })?;
                return Err(eyre!("installing {}: {err}", self.target.display()));
            }
            self.installed = true;
            fs::remove_dir_all(&self.old)
                .wrap_err_with(|| format!("removing {}", self.old.display()))?;
        } else {
            fs::rename(&self.dir, &self.target)
                .wrap_err_with(|| format!("installing {}", self.target.display()))?;
            self.installed = true;
        }
        Ok(())
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if !self.installed
            && let Err(e) = fs::remove_dir_all(&self.dir)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!(
                "xtask quint check: could not remove the staging dir {}: {e}",
                self.dir.display()
            );
        }
    }
}

/// Read, parse and stage one counterexample's raw and condensed trace.
/// Returns `(raw, condensed)` paths relative to `evidence/`.
fn stage_trace(
    job: &Job,
    rec: &CheckRecord,
    stage: &Stage,
) -> Result<(String, String), TraceRecordError> {
    let c = &job.check;
    let read = |path: &Path| {
        fs::read_to_string(path).map_err(|e| TraceRecordError::Unreadable {
            file: path.display().to_string(),
            error: e.to_string(),
        })
    };
    let unparsable = |path: &Path, e: trace::TraceError| TraceRecordError::Unparsable {
        file: path.display().to_string(),
        error: e.to_string(),
    };
    let (raw_name, raw, parsed, source) = match c.backend {
        Backend::Apalache => {
            let itf = job.out_dir.join("trace.itf.json");
            let body = read(&itf)?;
            let parsed = trace::parse_itf(&body).map_err(|e| unparsable(&itf, e))?;
            (format!("traces/{}.itf.json", c.name), body, parsed, "ITF")
        }
        Backend::Tlc => {
            let log = read(&rec.log)?;
            let text = trace::extract_tlc_trace(&log).map_err(|e| unparsable(&rec.log, e))?;
            let parsed = trace::parse_tlc(&text).map_err(|e| unparsable(&rec.log, e))?;
            (format!("traces/{}.tlc.txt", c.name), text, parsed, "TLC")
        }
    };
    let condensed_name = format!("traces/{}.txt", c.name);
    let condensed = trace::condense(&c.name, source, &parsed);
    for (rel, bytes) in [(&condensed_name, condensed.as_bytes()), (&raw_name, raw.as_bytes())] {
        if let Err(e) = stage.write(rel, bytes) {
            let mut error = e.to_string();
            // Leave no half-staged trace behind; say so if even that fails.
            for done in [&condensed_name, &raw_name] {
                if let Err(e) = stage.unwrite(done) {
                    let _ = write!(error, "; and could not remove staged {done}: {e}");
                }
            }
            return Err(TraceRecordError::Unwritable { file: rel.clone(), error });
        }
    }
    Ok((raw_name, condensed_name))
}

fn check_evidence(job: &Job, rec: &CheckRecord, stage: &Stage) -> Result<CheckEvidence> {
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
        trace_error: None,
    };
    if rec.outcome == Outcome::Violation {
        match stage_trace(job, rec, stage) {
            Ok((raw, condensed)) => {
                ev.trace = Some(raw);
                ev.condensed = Some(condensed);
            }
            Err(err) => ev.trace_error = Some(err),
        }
    }
    if !rec.pass || ev.trace_error.is_some() {
        let name = format!("failures/{}.log", c.name);
        stage.write_or_fail(&name, &attempt_logs(job, rec.attempts))?;
        ev.failure_log = Some(name);
    }
    Ok(ev)
}

/// One [`CheckEvidence`] per check, staging its trace and failure log. A
/// trace that cannot be recorded becomes that check's `trace_error`; only a
/// failure to stage a failure log is an `Err`.
fn stage_checks(
    jobs: &[Job],
    records: &[CheckRecord],
    stage: &Stage,
) -> Result<Vec<CheckEvidence>> {
    jobs.iter().zip(records).map(|(job, rec)| check_evidence(job, rec, stage)).collect()
}

/// What [`record`] did.
#[derive(Debug)]
pub(super) struct RecordReport {
    /// The installed `evidence/` directory.
    pub evidence_dir: PathBuf,
    /// Checks recorded.
    pub checks: usize,
    /// `(check, error, failure log relative to evidence/)` for every
    /// counterexample recorded without its trace.
    pub trace_errors: Vec<(String, TraceRecordError, Option<String>)>,
}

/// Build the evidence of a completed run and swap it into
/// `<dir>/evidence/`.
///
/// A per-check trace failure is recorded in that check's
/// [`CheckEvidence::trace_error`] and reported in the returned
/// [`RecordReport`]; only a failure that affects the whole evidence (inputs
/// changed, tool versions unavailable, the staging dir or `summary.json`
/// unwritable, the swap) is an `Err`.
pub(super) fn record(
    meta: &RunMeta<'_>,
    jobs: &[Job],
    records: &[CheckRecord],
) -> Result<RecordReport> {
    let after = hash_inputs(meta.dir)?;
    if after != meta.inputs {
        bail!(
            "{} changed while the checks ran; evidence not recorded (re-run on unchanged specs)",
            meta.dir.display()
        );
    }
    let stage = Stage::create(meta.dir, meta.run_id)?;
    let checks = stage_checks(jobs, records, &stage)?;
    let mut apalache = None;
    for rec in records {
        if apalache.is_none() {
            match fs::read_to_string(&rec.log) {
                Ok(log) => apalache = apalache_version(&log),
                Err(e) => eprintln!(
                    "xtask quint check: reading {} for the Apalache version: {e}",
                    rec.log.display()
                ),
            }
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
    stage.write_or_fail(SUMMARY_JSON, json.as_bytes())?;
    stage.write_or_fail(SUMMARY_MD, render_md(&summary).as_bytes())?;
    stage.install()?;
    Ok(RecordReport {
        evidence_dir: meta.dir.join(EVIDENCE_DIR),
        checks: summary.checks.len(),
        trace_errors: summary
            .checks
            .into_iter()
            .filter_map(|c| c.trace_error.map(|e| (c.name, e, c.failure_log)))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::resources::Reservation;
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
            trace_error: None,
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

    fn job(dir: &Path, name: &str, backend: Backend) -> Job {
        Job {
            subsystem: "sub".into(),
            dir: dir.to_path_buf(),
            check: Check { backend, max_steps: None, ..check(name, Expect::Violation) },
            out_dir: dir.join("runs").join(name),
            timeout: std::time::Duration::from_secs(900),
            sizing: super::super::resources::Sizing { heap_mb: 1024, workers: 1 },
            reservation: Reservation { mem_mb: 1024, cpus: 1 },
        }
    }

    fn check_record(j: &Job, outcome: Outcome) -> CheckRecord {
        CheckRecord {
            name: j.check.name.clone(),
            backend: j.check.backend,
            expect: j.check.expect,
            outcome,
            pass: judge(j.check.expect, outcome),
            seconds: 1.0,
            attempts: 1,
            timeout_secs: 900,
            cause: None,
            log: j.out_dir.join("attempt-1.log"),
            heap_mb: 1024,
            workers: 1,
            reserved_mem_mb: 1024,
            reserved_cpus: 1,
            peak_rss_mb: 0,
        }
    }

    /// One check's log as the runner leaves it: attempt log + events.log.
    fn write_logs(j: &Job, log: &str) {
        fs::create_dir_all(&j.out_dir).expect("mkdir");
        fs::write(j.out_dir.join("attempt-1.log"), log).expect("write");
        fs::write(j.out_dir.join("events.log"), "1.000 attempt 1 start\n").expect("write");
    }

    #[test]
    fn a_trace_that_cannot_be_recorded_spares_every_other_check() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        // An initial-state TLC violation (the shape that once lost a whole
        // run) is recorded like any other counterexample.
        let init = job(dir, "init", Backend::Tlc);
        write_logs(&init, include_str!("testdata/tlc/init-inv.log"));
        // A TLC violation whose log holds no counterexample text.
        let garbled = job(dir, "garbled", Backend::Tlc);
        write_logs(&garbled, "[violation] Found an issue (1ms).\nerror: found a counterexample\n");
        // An Apalache violation whose ITF file is missing.
        let no_itf = job(dir, "no-itf", Backend::Apalache);
        write_logs(&no_itf, "error: found a counterexample\n");
        // A trace the stage cannot write (its path is taken by a directory).
        let blocked = job(dir, "blocked", Backend::Tlc);
        write_logs(&blocked, include_str!("testdata/tlc/multi-inv.log"));
        let jobs = [init, garbled, no_itf, blocked];
        let records: Vec<CheckRecord> =
            jobs.iter().map(|j| check_record(j, Outcome::Violation)).collect();

        let stage = Stage::create(dir, "7-1.0").expect("stage");
        fs::create_dir_all(stage.dir.join("traces/blocked.txt")).expect("block the path");
        let checks = stage_checks(&jobs, &records, &stage).expect("staging never fails per trace");
        fs::remove_dir_all(stage.dir.join("traces/blocked.txt")).expect("unblock");

        assert_eq!(checks[0].trace.as_deref(), Some("traces/init.tlc.txt"));
        assert_eq!(checks[0].condensed.as_deref(), Some("traces/init.txt"));
        assert_eq!((checks[0].trace_error.as_ref(), checks[0].failure_log.as_ref()), (None, None));

        assert!(matches!(
            &checks[1].trace_error,
            Some(TraceRecordError::Unparsable { file, error })
                if file.ends_with("garbled/attempt-1.log") && error.contains("no TLC counterexample")
        ));
        assert!(matches!(
            &checks[2].trace_error,
            Some(TraceRecordError::Unreadable { file, .. }) if file.ends_with("no-itf/trace.itf.json")
        ));
        assert!(matches!(
            &checks[3].trace_error,
            Some(TraceRecordError::Unwritable { file, .. }) if file == "traces/blocked.txt"
        ));
        for c in &checks[1..] {
            // Outcome and verdict stand; the trace does not; the full log is kept.
            assert_eq!(
                (c.outcome, c.pass, &c.trace, &c.condensed),
                (Outcome::Violation, true, &None, &None)
            );
            assert_eq!(c.failure_log, Some(format!("failures/{}.log", c.name)));
        }

        let s = summary(checks);
        stage.write_or_fail(SUMMARY_MD, render_md(&s).as_bytes()).expect("md");
        stage.install().expect("install");
        let ev = dir.join(EVIDENCE_DIR);
        let present = list_files(&ev).expect("list");
        assert_eq!(
            present,
            BTreeSet::from(
                [
                    SUMMARY_MD,
                    "traces/init.tlc.txt",
                    "traces/init.txt",
                    "failures/garbled.log",
                    "failures/no-itf.log",
                    "failures/blocked.log",
                ]
                .map(str::to_owned)
            ),
            "no half-written trace of `blocked` survives"
        );
        let garbled_log = fs::read_to_string(ev.join("failures/garbled.log")).expect("log");
        assert!(garbled_log.contains("error: found a counterexample"), "{garbled_log}");
        assert!(garbled_log.contains("attempt 1 start"), "{garbled_log}");
        assert_eq!(
            fs::read_to_string(ev.join("traces/init.txt")).expect("trace"),
            "# init: counterexample, 1 state(s) (from the TLC trace)\n\nState 1\n  x = 0\n"
        );

        let md = render_md(&s);
        assert!(md.contains("3 counterexample(s) without a recorded trace"), "{md}");
        assert!(md.contains("## Counterexamples without a recorded trace"), "{md}");
        assert!(md.contains("- `garbled`: no parsable counterexample in "), "{md}");
        assert!(md.contains("| **no trace** [failures/no-itf.log](failures/no-itf.log) |"), "{md}");

        // verify-evidence names each untraced counterexample.
        let checks: Vec<Check> =
            s.checks.iter().map(|c| check(&c.name, Expect::Violation)).collect();
        let problems = verify_summary(&s, &s.inputs.clone(), &checks, &present);
        let untraced: Vec<&str> = problems
            .iter()
            .filter_map(|p| match p {
                EvidenceProblem::TraceNotRecorded { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(untraced, ["garbled", "no-itf", "blocked"]);
        assert!(!problems.iter().any(|p| matches!(p, EvidenceProblem::MissingTrace { .. })));
        assert!(
            problems[0].to_string().contains("counterexample trace NOT recorded"),
            "{}",
            problems[0]
        );
    }

    #[test]
    fn trace_error_roundtrips_and_is_omitted_when_absent() {
        let mut c = rec("w", Expect::Violation, Outcome::Violation);
        let plain = serde_json::to_value(&c).expect("encode");
        assert!(plain.get("trace_error").is_none(), "{plain}");
        c.trace = None;
        c.condensed = None;
        c.trace_error = Some(TraceRecordError::Unparsable {
            file: "a.log".into(),
            error: "no TLC counterexample".into(),
        });
        let v = serde_json::to_value(&c).expect("encode");
        assert_eq!(v["trace_error"]["kind"], "unparsable");
        assert_eq!(serde_json::from_value::<CheckEvidence>(v).expect("decode"), c);
    }

    #[test]
    fn missing_logs_are_named_in_the_failure_log_copy() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let j = job(tmp.path(), "gone", Backend::Tlc);
        let copy = String::from_utf8(attempt_logs(&j, 1)).expect("utf8");
        assert!(copy.contains("===== attempt 1: "), "{copy}");
        assert!(copy.contains("(could not read "), "{copy}");
        assert!(copy.contains("===== events: "), "{copy}");
    }

    #[test]
    fn install_replaces_the_whole_evidence_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ev = tmp.path().join(EVIDENCE_DIR);
        fs::create_dir_all(ev.join("round-2")).expect("mkdir");
        fs::write(ev.join("round-2/copied.sh"), "#!/bin/sh").expect("write");
        let stage = Stage::create(tmp.path(), "12-3.4").expect("stage");
        stage.write_or_fail(SUMMARY_JSON, b"{}").expect("write");
        stage.write_or_fail("traces/w.txt", b"trace").expect("write");
        stage.install().expect("install");
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
