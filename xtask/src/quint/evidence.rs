//! `specs/quint/<subsystem>/evidence/` — the recorded evidence of a
//! subsystem's checks against its current specs.
//!
//! `cargo xtask quint check --subsystem <dir> --record` is the only writer.
//! It is incremental: each recorded check carries an *input key* (see
//! [`check_input_key`]) and a recording run re-runs only the checks whose key
//! changed (or that have no usable recorded result), carrying every other
//! check's recorded verdict and trace over unchanged. `--full` re-runs every
//! check. At the end of a completed run (whatever the verdicts) it builds the
//! new evidence in a hidden staging directory beside `evidence/` and swaps it
//! in, so `evidence/` is never half-written. A counterexample whose trace
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
//! `cargo xtask quint verify-evidence` recomputes every check's input key and
//! fails when a recorded check's key is stale, any recorded outcome misses its
//! `expect`, or a counterexample was recorded without its trace. Evidence
//! whose checks carry no input key (recorded before incremental recording) is
//! verified the old way: every input hash must match.

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
    collect_qnt, judge, reachable_specs, spec_index,
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
    #[error("--record records the whole subsystem; it cannot be combined with --name")]
    Name,
    /// `--ci` given.
    #[error("--record records the whole subsystem; it cannot be combined with --ci")]
    Ci,
}

/// `--record` is allowed only for a full selection of one subsystem (the run
/// itself re-runs only the stale checks of it).
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
    /// Attempts made (a stall is retried once).
    pub attempts: u32,
    /// The check's own time: run seconds of the final attempt. Evidence
    /// recorded without `retry_seconds` holds the wall time from the first
    /// attempt's start here instead.
    pub seconds: f64,
    /// Run seconds of the attempts abandoned to stall retries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_seconds: Option<f64>,
    /// Seconds from the first attempt's start to the verdict, including any
    /// wait for the budget between attempts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_seconds: Option<f64>,
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
    /// The check's input key when it was run ([`check_input_key`]). Evidence
    /// recorded before incremental recording has none, and is then verified
    /// against the global input hashes and re-run by the next `--record`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_key: Option<String>,
    /// The run that produced this record. Differs from the summary's
    /// `run_id` for a check carried over unchanged from an earlier recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
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

/// Index every `.qnt` file under `dir`.
///
/// Same selection and keys as [`hash_inputs`]. A file the lexer cannot index is left out: a check whose
/// import closure reaches it then has no input key and counts as stale.
pub fn index_specs(dir: &Path) -> Result<BTreeMap<String, spec_index::QntIndex>> {
    let mut paths = Vec::new();
    collect_qnt(dir, &mut paths)?;
    let mut out = BTreeMap::new();
    for path in paths {
        let rel = path.strip_prefix(dir).wrap_err("spec outside the subsystem dir")?;
        let key = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let body =
            fs::read_to_string(&path).wrap_err_with(|| format!("reading {}", path.display()))?;
        if let Ok(idx) = spec_index::index(&body) {
            out.insert(key, idx);
        }
    }
    Ok(out)
}

/// The tool versions whose change invalidates every recorded check.
///
/// Quint (which pins the TLC and Apalache it downloads) and the JVM they run on.
/// The kernel is recorded for provenance but does not invalidate anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolKey {
    /// `quint --version`.
    pub quint: String,
    /// First line of `java -version`.
    pub java: String,
}

impl From<&Tools> for ToolKey {
    fn from(t: &Tools) -> Self {
        Self { quint: t.quint.clone(), java: t.java.clone() }
    }
}

/// Why a check has no input key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// A file in the check's import closure is not an input hashed by
    /// [`hash_inputs`] (missing, or under a hidden or `_`-prefixed directory).
    #[error("`{file}` (loaded by the check's spec) is not a hashed input")]
    Unhashed {
        /// Path relative to the subsystem directory.
        file: String,
    },
    /// A file in the check's import closure could not be indexed, so its own
    /// imports are unknown.
    #[error("`{file}` (loaded by the check's spec) cannot be indexed")]
    Unindexed {
        /// Path relative to the subsystem directory.
        file: String,
    },
}

/// The input key of one check: sha256 (hex) over everything that can change
/// its verdict —
///
/// - the tool versions ([`ToolKey`]);
/// - the check's own `checks.toml` entry: name, spec, main, property,
///   backend, `max_steps`, `expect`, `timeout_secs`, `heap_mb` and `workers`
///   (`ci` only selects checks, it does not change a verdict);
/// - the sha256 of its spec and of every file the spec imports, transitively
///   (`hashes` from [`hash_inputs`], `files` from [`index_specs`]).
///
/// Other `.qnt` files of the subsystem and the other `[[check]]` entries are
/// not part of the key. Fails closed: a closure file with no hash or no index
/// is an error, never silently ignored.
pub fn check_input_key(
    check: &Check,
    hashes: &BTreeMap<String, String>,
    files: &BTreeMap<String, spec_index::QntIndex>,
    tools: &ToolKey,
) -> Result<String, KeyError> {
    let spec = check.spec.to_string_lossy().replace('\\', "/");
    let mut h = Sha256::new();
    let mut line = |s: String| {
        h.update(s.as_bytes());
        h.update(b"\n");
    };
    line("quint-check-input-key v1".to_owned());
    line(format!("tool quint {}", tools.quint));
    line(format!("tool java {}", tools.java));
    line(format!("name {}", check.name));
    line(format!("spec {spec}"));
    line(format!("main {}", check.main));
    line(match &check.property {
        Property::Invariant(p) => format!("invariant {p}"),
        Property::Temporal(p) => format!("temporal {p}"),
        Property::Run(p) => format!("run {p}"),
    });
    line(format!("backend {}", check.backend.as_str()));
    line(format!("max_steps {:?}", check.max_steps));
    line(format!("expect {}", check.expect.as_str()));
    line(format!("timeout_secs {:?}", check.timeout_secs));
    line(format!("heap_mb {:?}", check.heap_mb));
    line(format!("workers {:?}", check.workers));
    for file in reachable_specs(files, &spec) {
        let sha = hashes.get(&file).ok_or_else(|| KeyError::Unhashed { file: file.clone() })?;
        if !files.contains_key(&file) {
            return Err(KeyError::Unindexed { file });
        }
        line(format!("file {file} {sha}"));
    }
    Ok(hex::encode(h.finalize()))
}

/// [`check_input_key`] of every check, by name.
pub fn check_keys(
    checks: &[Check],
    hashes: &BTreeMap<String, String>,
    files: &BTreeMap<String, spec_index::QntIndex>,
    tools: &ToolKey,
) -> BTreeMap<String, Result<String, KeyError>> {
    checks.iter().map(|c| (c.name.clone(), check_input_key(c, hashes, files, tools))).collect()
}

/// The recorded evidence a recording run starts from.
#[derive(Debug, Clone, PartialEq)]
pub enum Prior {
    /// No `evidence/summary.json`.
    Absent,
    /// `summary.json` exists but cannot be used (undecodable).
    Unreadable(String),
    /// A decoded summary and the files under `evidence/` (relative).
    Found(Box<EvidenceSummary>, BTreeSet<String>),
}

/// Read the recorded evidence of `dir`. An I/O error other than a missing
/// `summary.json` is an `Err`; an undecodable summary is [`Prior::Unreadable`].
pub fn load_prior(dir: &Path) -> Result<Prior> {
    let evidence = dir.join(EVIDENCE_DIR);
    let path = evidence.join(SUMMARY_JSON);
    let body = match fs::read_to_string(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Prior::Absent),
        Err(e) => return Err(eyre!("reading {}: {e}", path.display())),
    };
    match serde_json::from_str::<EvidenceSummary>(&body) {
        Ok(s) => Ok(Prior::Found(Box::new(s), list_files(&evidence)?)),
        Err(e) => Ok(Prior::Unreadable(e.to_string())),
    }
}

/// What a recording run does with each check of the manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordPlan {
    /// Manifest order of every check.
    pub order: Vec<String>,
    /// Checks whose recorded result stands, carried over unchanged.
    pub reuse: BTreeMap<String, CheckEvidence>,
    /// Checks to run, each with the reason (in manifest order).
    pub rerun: Vec<(String, String)>,
    /// Current input key of every check that has one.
    pub keys: BTreeMap<String, String>,
    /// Apalache version of the prior evidence (checked against the run's own
    /// when some results are reused).
    pub prior_apalache: Option<String>,
}

impl RecordPlan {
    /// Whether `name` is to be run.
    pub fn runs(&self, name: &str) -> bool {
        self.rerun.iter().any(|(n, _)| n == name)
    }
}

/// Decide which checks a recording run must run.
///
/// A check's recorded result is reused only when every one of these holds,
/// and re-run otherwise (fail closed):
///
/// - the prior summary decodes and uses [`SCHEMA_VERSION`], and was recorded
///   under the current tool versions (any tool change re-runs everything);
/// - the check has a record carrying an input key equal to its current one;
/// - the record's outcome meets its `expect`, has no trace error, and every
///   evidence file it references is present.
///
/// A recorded check no longer in the manifest is simply not carried over.
pub fn plan_record(
    prior: &Prior,
    full: bool,
    tools: &ToolKey,
    checks: &[Check],
    hashes: &BTreeMap<String, String>,
    files: &BTreeMap<String, spec_index::QntIndex>,
) -> RecordPlan {
    let keys = check_keys(checks, hashes, files, tools);
    let order: Vec<String> = checks.iter().map(|c| c.name.clone()).collect();
    let all = |reason: String| RecordPlan {
        order: order.clone(),
        reuse: BTreeMap::new(),
        rerun: order.iter().map(|n| (n.clone(), reason.clone())).collect(),
        keys: keys
            .iter()
            .filter_map(|(n, k)| k.as_ref().ok().map(|k| (n.clone(), k.clone())))
            .collect(),
        prior_apalache: None,
    };
    if full {
        return all("--full".to_owned());
    }
    let (summary, present) = match prior {
        Prior::Absent => return all("no recorded evidence".to_owned()),
        Prior::Unreadable(e) => return all(format!("recorded summary.json is unusable: {e}")),
        Prior::Found(s, present) => (s, present),
    };
    if summary.schema != SCHEMA_VERSION {
        return all(format!("recorded schema {} differs from {SCHEMA_VERSION}", summary.schema));
    }
    if ToolKey::from(&summary.tools) != *tools {
        return all("tool versions changed since the recorded run".to_owned());
    }
    let recorded: BTreeMap<&str, &CheckEvidence> =
        summary.checks.iter().map(|c| (c.name.as_str(), c)).collect();
    let mut reuse = BTreeMap::new();
    let mut rerun = Vec::new();
    for c in checks {
        let verdict = match (recorded.get(c.name.as_str()), keys.get(&c.name)) {
            (None, _) => Err("not in the recorded evidence".to_owned()),
            (_, Some(Err(e))) => Err(format!("no input key: {e}")),
            (Some(rec), Some(Ok(key))) => match rec.input_key.as_deref() {
                None => Err("recorded without an input key".to_owned()),
                Some(old) if old != key => {
                    Err("its spec, an import or its checks.toml entry changed".to_owned())
                }
                Some(_) if !judge(rec.expect, rec.outcome) => {
                    Err("its recorded outcome missed `expect`".to_owned())
                }
                Some(_) if rec.trace_error.is_some() => {
                    Err("its counterexample trace was not recorded".to_owned())
                }
                Some(_) => {
                    let absent = [&rec.trace, &rec.condensed, &rec.failure_log]
                        .into_iter()
                        .flatten()
                        .find(|f| !present.contains(*f));
                    absent.map_or_else(
                        || Ok((*rec).clone()),
                        |f| Err(format!("its recorded file `{f}` is missing")),
                    )
                }
            },
            (Some(_), None) => Err("no input key".to_owned()),
        };
        match verdict {
            Ok(rec) => {
                reuse.insert(c.name.clone(), rec);
            }
            Err(reason) => rerun.push((c.name.clone(), reason)),
        }
    }
    RecordPlan {
        order,
        reuse,
        rerun,
        keys: keys.into_iter().filter_map(|(n, k)| k.ok().map(|k| (n, k))).collect(),
        prior_apalache: summary.tools.apalache.clone(),
    }
}

/// The tool versions of this machine.
pub fn current_tools() -> Result<Tools> {
    let quint =
        first_line(&run_tool(Command::new("quint").arg("--version"), "quint --version")?.stdout);
    let java_out = run_tool(Command::new("java").arg("-version"), "java -version")?;
    let java =
        first_line(if java_out.stderr.is_empty() { &java_out.stdout } else { &java_out.stderr });
    let kernel = first_line(&run_tool(Command::new("uname").arg("-r"), "uname -r")?.stdout);
    Ok(Tools { quint, apalache: None, java, kernel })
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

/// One check's row of the `summary.md` table.
fn check_row(c: &CheckEvidence) -> String {
    let mut files: Vec<String> = Vec::new();
    if c.trace_error.is_some() {
        files.push("**no trace**".to_owned());
    }
    for f in [&c.condensed, &c.trace, &c.failure_log].into_iter().flatten() {
        files.push(format!("[{f}]({f})"));
    }
    let opt = |v: Option<f64>| v.map_or_else(|| "—".to_owned(), |v| format!("{v:.1}"));
    format!(
        "| {} | {} | {} | {} `{}` | {} | {} | {} | {} | {} | {} | {:.1} | {} | {} | {} |\n",
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
        opt(c.retry_seconds),
        opt(c.wall_seconds),
        files.join(" "),
    )
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
    let carried =
        s.checks.iter().filter(|c| c.run_id.as_deref().is_some_and(|r| r != s.run_id)).count();
    if carried > 0 {
        let _ = writeln!(
            out,
            "| Carried over | {carried} check(s) were not re-run by this run: their inputs are \
             unchanged since the run named in their `run_id` in summary.json |"
        );
    }
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
         Attempts | Seconds | Retry s | Wall s | Evidence |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|");
    for c in &s.checks {
        out.push_str(&check_row(c));
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
    /// A recorded check's input key no longer matches its current inputs.
    #[error("stale: check `{name}`: {reason}")]
    CheckStale {
        /// Check name.
        name: String,
        /// What differs.
        reason: String,
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

/// Legacy staleness: any input file whose hash differs from the summary's.
fn stale_inputs(
    summary: &EvidenceSummary,
    inputs: &BTreeMap<String, String>,
) -> Vec<EvidenceProblem> {
    let mut problems = Vec::new();
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
    problems
}

/// Compare a recorded summary with the current inputs, manifest and evidence files.
///
/// `files` are paths relative to `evidence/`; `keys` are the current input
/// keys of `checks` under the summary's tool versions ([`check_keys`]). Empty
/// means the evidence stands for the current specs and every recorded outcome
/// matched.
///
/// When every recorded check carries an input key, each must equal its
/// current key; files no check loads do not matter. Otherwise (evidence from
/// before incremental recording) every input hash must match.
pub fn verify_summary(
    summary: &EvidenceSummary,
    inputs: &BTreeMap<String, String>,
    keys: &BTreeMap<String, Result<String, KeyError>>,
    checks: &[Check],
    files: &BTreeSet<String>,
) -> Vec<EvidenceProblem> {
    let mut problems = Vec::new();
    if summary.schema != SCHEMA_VERSION {
        problems.push(EvidenceProblem::Schema { found: summary.schema });
    }
    let keyed = !summary.checks.is_empty() && summary.checks.iter().all(|c| c.input_key.is_some());
    if keyed {
        for c in &summary.checks {
            let reason = match (keys.get(&c.name), c.input_key.as_deref()) {
                (Some(Ok(now)), Some(rec)) if now == rec => continue,
                (Some(Ok(_)), _) => "its spec, an import or its checks.toml entry changed".into(),
                (Some(Err(e)), _) => format!("its inputs cannot be keyed: {e}"),
                // Not in the manifest: reported as `CheckNotInManifest` below.
                (None, _) => continue,
            };
            problems.push(EvidenceProblem::CheckStale { name: c.name.clone(), reason });
        }
    } else {
        problems.extend(stale_inputs(summary, inputs));
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
    let hashes = hash_inputs(dir)?;
    let keys = check_keys(checks, &hashes, &index_specs(dir)?, &ToolKey::from(&summary.tools));
    let problems = verify_summary(&summary, &hashes, &keys, checks, &list_files(&evidence)?);
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
    /// Which checks the run ran and which recorded results it carries over.
    pub plan: &'a RecordPlan,
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

    /// Copy one file of the current `evidence/` into the stage (same path).
    fn carry_over(&self, rel: &str) -> Result<()> {
        let from = self.target.join(rel);
        let bytes = fs::read(&from).wrap_err_with(|| format!("reading {}", from.display()))?;
        self.write_or_fail(rel, &bytes)
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
    let (raw_name, raw, condensed) = match c.backend {
        Backend::Apalache => {
            let itf = job.out_dir.join("trace.itf.json");
            let body = read(&itf)?;
            let parsed = trace::parse_itf(&body).map_err(|e| unparsable(&itf, e))?;
            let condensed = trace::condense(&c.name, "ITF", &parsed);
            (format!("traces/{}.itf.json", c.name), body, condensed)
        }
        Backend::Tlc => {
            let log = read(&rec.log)?;
            let text = trace::extract_tlc_trace(&log).map_err(|e| unparsable(&rec.log, e))?;
            let parsed = trace::parse_tlc(&text).map_err(|e| unparsable(&rec.log, e))?;
            let condensed = trace::condense(&c.name, "TLC", &parsed);
            (format!("traces/{}.tlc.txt", c.name), text, condensed)
        }
        // A failed run's evidence is Quint's own failure report: which
        // `expect` failed, where, and the seed that repeats it.
        Backend::Evaluator => {
            let log = read(&rec.log)?;
            (format!("traces/{}.test.txt", c.name), log.clone(), log)
        }
    };
    let condensed_name = format!("traces/{}.txt", c.name);
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

/// Round seconds to milliseconds for the evidence.
fn millis(secs: f64) -> f64 {
    (secs * 1000.0).round() / 1000.0
}

fn check_evidence(job: &Job, rec: &CheckRecord, stage: &Stage) -> Result<CheckEvidence> {
    let c = &job.check;
    let (property_kind, property) = match &c.property {
        Property::Invariant(p) => ("invariant", p.clone()),
        Property::Temporal(p) => ("temporal", p.clone()),
        Property::Run(p) => ("run", p.clone()),
    };
    let bound = match (c.backend, c.max_steps) {
        (Backend::Tlc, _) => "exhaustive".to_owned(),
        (Backend::Apalache, Some(n)) => format!("max_steps={n}"),
        (Backend::Apalache, None) => "max_steps=quint-default".to_owned(),
        (Backend::Evaluator, _) => "run".to_owned(),
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
        seconds: millis(rec.seconds),
        retry_seconds: Some(millis(rec.retry_seconds)),
        wall_seconds: Some(millis(rec.wall_seconds)),
        timeout_secs: rec.timeout_secs,
        cause: rec.cause.clone(),
        trace: None,
        condensed: None,
        failure_log: None,
        trace_error: None,
        input_key: None,
        run_id: None,
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
    /// Checks recorded (run now plus carried over).
    pub checks: usize,
    /// Of those, checks whose recorded result was carried over unchanged.
    pub carried_over: usize,
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
    let mut ran: BTreeMap<String, CheckEvidence> = BTreeMap::new();
    for mut ev in stage_checks(jobs, records, &stage)? {
        ev.input_key = meta.plan.keys.get(&ev.name).cloned();
        ev.run_id = Some(meta.run_id.to_owned());
        ran.insert(ev.name.clone(), ev);
    }
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
    let mut tools = current_tools()?;
    if !meta.plan.reuse.is_empty() {
        // Carried-over results were produced by the prior run's Apalache.
        if let (Some(now), Some(then)) = (&apalache, &meta.plan.prior_apalache)
            && now != then
        {
            bail!(
                "Apalache changed from `{then}` to `{now}` while {} recorded check(s) were \
                 carried over; evidence not recorded (re-record with --full)",
                meta.plan.reuse.len()
            );
        }
        if apalache.is_none() {
            apalache.clone_from(&meta.plan.prior_apalache);
        }
    }
    tools.apalache = apalache;
    let head = first_line(&git(meta.root, &["rev-parse", "HEAD"])?.stdout);
    let dirty = !git(meta.root, &["status", "--porcelain"])?.stdout.is_empty();

    let mut checks = Vec::with_capacity(meta.plan.order.len());
    for name in &meta.plan.order {
        if let Some(ev) = ran.remove(name) {
            checks.push(ev);
        } else if let Some(ev) = meta.plan.reuse.get(name) {
            for f in [&ev.trace, &ev.condensed, &ev.failure_log].into_iter().flatten() {
                stage.carry_over(f)?;
            }
            checks.push(ev.clone());
        } else {
            bail!("check `{name}` was neither run nor carried over; evidence not recorded");
        }
    }
    let summary = EvidenceSummary {
        schema: SCHEMA_VERSION,
        subsystem: meta.subsystem.to_owned(),
        run_id: meta.run_id.to_owned(),
        started: rfc3339(meta.started_unix)?,
        finished: rfc3339(meta.finished_unix)?,
        jobs: meta.jobs,
        tools,
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
        carried_over: meta.plan.reuse.len(),
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
            retry_seconds: Some(0.0),
            wall_seconds: Some(1.5),
            timeout_secs: 900,
            cause: None,
            trace: violation.then(|| format!("traces/{name}.itf.json")),
            condensed: violation.then(|| format!("traces/{name}.txt")),
            failure_log: (!pass).then(|| format!("failures/{name}.log")),
            trace_error: None,
            input_key: None,
            run_id: None,
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
        assert!(md.contains("| w | a.qnt | m | invariant `Inv` | apalache | max_steps=4 | violation | violation | PASS | 1 | 1.5 | 0.0 | 1.5 | [traces/w.txt](traces/w.txt) [traces/w.itf.json](traces/w.itf.json) |"), "{md}");
        assert!(md.contains("| h | a.qnt | m | invariant `Inv` | apalache | max_steps=4 | holds | tool-error | **FAIL** | 1 | 1.5 | 0.0 | 1.5 | [failures/h.log](failures/h.log) |"), "{md}");
        assert!(md.contains("- `checks.toml` `22`"), "{md}");
        assert!(md.contains("(dirty worktree)"), "{md}");
        // A `|` inside a value must not split the table cell.
        assert!(md.contains("| Apalache | 0.56.1 \\| build: 70cdaf4 |"), "{md}");
        assert!(md.contains("- `h`: expected holds, got tool-error"), "{md}");
    }

    #[test]
    fn evidence_without_retry_and_wall_seconds_still_decodes_and_verifies() {
        // Evidence recorded before the split timing fields existed.
        let s = summary(vec![rec("w", Expect::Violation, Outcome::Violation)]);
        let mut v: serde_json::Value = serde_json::to_value(&s).expect("encode");
        let row = v["checks"][0].as_object_mut().expect("object");
        row.remove("retry_seconds");
        row.remove("wall_seconds");
        let legacy: EvidenceSummary = serde_json::from_value(v).expect("legacy decodes");
        assert_eq!(legacy.checks[0].retry_seconds, None);
        assert_eq!(legacy.checks[0].wall_seconds, None);
        assert!(
            verify_summary(
                &legacy,
                &legacy.inputs,
                &BTreeMap::new(),
                &[check("w", Expect::Violation)],
                &files(&legacy)
            )
            .is_empty()
        );
        // Re-encoding keeps the legacy shape (no null fields appear).
        let json = serde_json::to_string(&legacy).expect("encode");
        assert!(!json.contains("retry_seconds") && !json.contains("wall_seconds"), "{json}");
        assert!(render_md(&legacy).contains("| 1 | 1.5 | — | — |"));
    }

    #[test]
    fn current_passing_evidence_has_no_problems() {
        let s = summary(vec![
            rec("w", Expect::Violation, Outcome::Violation),
            rec("h", Expect::Holds, Outcome::Holds),
        ]);
        let checks = [check("w", Expect::Violation), check("h", Expect::Holds)];
        assert_eq!(
            verify_summary(&s, &s.inputs.clone(), &BTreeMap::new(), &checks, &files(&s)),
            vec![]
        );
    }

    #[test]
    fn stale_inputs_are_reported_per_file() {
        let s = summary(vec![rec("h", Expect::Holds, Outcome::Holds)]);
        let mut now = s.inputs.clone();
        now.insert("a.qnt".into(), "changed".into());
        now.insert("hazard/b.qnt".into(), "33".into());
        now.remove("checks.toml");
        assert_eq!(
            verify_summary(&s, &now, &BTreeMap::new(), &[check("h", Expect::Holds)], &files(&s)),
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
            verify_summary(&s, &s.inputs.clone(), &BTreeMap::new(), &checks, &files(&s)),
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
            verify_summary(&s, &s.inputs.clone(), &BTreeMap::new(), &checks, &present),
            vec![
                EvidenceProblem::Schema { found: 99 },
                EvidenceProblem::MissingFile { file: "traces/w.itf.json".into() },
            ]
        );
        assert!(
            verify_summary(&no_trace, &s.inputs.clone(), &BTreeMap::new(), &checks, &files(&s))
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
            retry_seconds: 0.0,
            wall_seconds: 1.0,
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
        let problems = verify_summary(&s, &s.inputs.clone(), &BTreeMap::new(), &checks, &present);
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

    // ---- incremental recording: input keys and the record plan ----

    fn tools() -> ToolKey {
        ToolKey { quint: "0.32.0".into(), java: "openjdk version \"21\"".into() }
    }

    /// Spec files by relative path, from their sources.
    fn specs(srcs: &[(&str, &str)]) -> BTreeMap<String, spec_index::QntIndex> {
        srcs.iter().map(|(p, b)| ((*p).to_owned(), spec_index::index(b).expect("index"))).collect()
    }

    fn hashes(h: &[(&str, &str)]) -> BTreeMap<String, String> {
        h.iter().map(|(p, s)| ((*p).to_owned(), (*s).to_owned())).collect()
    }

    fn check_on(name: &str, spec: &str) -> Check {
        Check { spec: spec.into(), ..check(name, Expect::Holds) }
    }

    /// `a.qnt` imports `lib/b.qnt`, which imports `lib/c.qnt`; `d.qnt` stands alone.
    fn world() -> (BTreeMap<String, spec_index::QntIndex>, BTreeMap<String, String>, Vec<Check>) {
        let files = specs(&[
            ("a.qnt", "module A { import B.* from \"./lib/b\" }"),
            ("lib/b.qnt", "module B { import C.* from \"./c\" }"),
            ("lib/c.qnt", "module C { }"),
            ("d.qnt", "module D { }"),
        ]);
        let h = hashes(&[
            ("a.qnt", "a1"),
            ("lib/b.qnt", "b1"),
            ("lib/c.qnt", "c1"),
            ("d.qnt", "d1"),
            ("checks.toml", "t1"),
        ]);
        (files, h, vec![check_on("on_a", "a.qnt"), check_on("on_d", "d.qnt")])
    }

    /// A summary recorded for `checks` under `tools()` with their current keys.
    fn recorded(
        checks: &[Check],
        h: &BTreeMap<String, String>,
        f: &BTreeMap<String, spec_index::QntIndex>,
    ) -> EvidenceSummary {
        let keys = check_keys(checks, h, f, &tools());
        let mut s = summary(
            checks
                .iter()
                .map(|c| {
                    let mut r = rec(&c.name, c.expect, Outcome::Holds);
                    r.input_key = keys[&c.name].clone().ok();
                    r.run_id = Some("old".into());
                    r
                })
                .collect(),
        );
        s.tools.quint = tools().quint;
        s.tools.java = tools().java;
        s
    }

    fn plan(
        s: &EvidenceSummary,
        checks: &[Check],
        h: &BTreeMap<String, String>,
        f: &BTreeMap<String, spec_index::QntIndex>,
    ) -> RecordPlan {
        let present = files(s);
        plan_record(&Prior::Found(Box::new(s.clone()), present), false, &tools(), checks, h, f)
    }

    fn rerun_names(p: &RecordPlan) -> Vec<&str> {
        p.rerun.iter().map(|(n, _)| n.as_str()).collect()
    }

    #[test]
    fn unchanged_inputs_run_nothing() {
        let (f, h, checks) = world();
        let s = recorded(&checks, &h, &f);
        let p = plan(&s, &checks, &h, &f);
        assert!(p.rerun.is_empty(), "{:?}", p.rerun);
        assert_eq!(p.reuse.len(), 2);
    }

    #[test]
    fn a_spec_edit_reruns_only_the_checks_on_that_spec() {
        let (f, mut h, checks) = world();
        let s = recorded(&checks, &h, &f);
        h.insert("d.qnt".into(), "d2".into());
        assert_eq!(rerun_names(&plan(&s, &checks, &h, &f)), ["on_d"]);
    }

    #[test]
    fn an_edit_of_a_transitive_import_reruns_its_dependents() {
        let (f, mut h, checks) = world();
        let s = recorded(&checks, &h, &f);
        h.insert("lib/c.qnt".into(), "c2".into());
        let p = plan(&s, &checks, &h, &f);
        assert_eq!(rerun_names(&p), ["on_a"]);
        assert!(p.reuse.contains_key("on_d"));
    }

    #[test]
    fn an_edit_of_a_file_no_check_loads_runs_nothing() {
        let (mut f, mut h, checks) = world();
        let s = recorded(&checks, &h, &f);
        f.insert("hazard/x.qnt".into(), spec_index::index("module X { }").expect("index"));
        h.insert("hazard/x.qnt".into(), "x1".into());
        h.insert("checks.toml".into(), "t2".into());
        assert!(plan(&s, &checks, &h, &f).rerun.is_empty());
    }

    #[test]
    fn every_execution_field_of_the_checks_toml_entry_reruns_that_check() {
        let (f, h, checks) = world();
        let s = recorded(&checks, &h, &f);
        let edits: [fn(&mut Check); 9] = [
            |c| c.main = "other".into(),
            |c| c.property = Property::Invariant("Other".into()),
            |c| c.backend = Backend::Tlc,
            |c| c.max_steps = Some(9),
            |c| c.expect = Expect::Violation,
            |c| c.timeout_secs = Some(5),
            |c| c.heap_mb = Some(512),
            |c| c.workers = Some(2),
            |c| c.spec = "d.qnt".into(),
        ];
        for edit in edits {
            let mut edited = checks.clone();
            edit(&mut edited[0]);
            let p = plan(&s, &edited, &h, &f);
            assert_eq!(rerun_names(&p), ["on_a"], "{edited:?}");
            assert!(p.reuse.contains_key("on_d"));
        }
        // `ci` only selects checks; it cannot change a verdict.
        let mut ci = checks;
        ci[0].ci = true;
        assert!(plan(&s, &ci, &h, &f).rerun.is_empty());
    }

    #[test]
    fn a_tool_version_change_reruns_every_check() {
        let (f, h, checks) = world();
        let s = recorded(&checks, &h, &f);
        let present = files(&s);
        for newer in [
            ToolKey { quint: "0.33.0".into(), ..tools() },
            ToolKey { java: "openjdk version \"25\"".into(), ..tools() },
        ] {
            let p = plan_record(
                &Prior::Found(Box::new(s.clone()), present.clone()),
                false,
                &newer,
                &checks,
                &h,
                &f,
            );
            assert_eq!(rerun_names(&p), ["on_a", "on_d"]);
            assert!(p.reuse.is_empty());
        }
    }

    #[test]
    fn an_added_check_runs_and_a_removed_check_is_dropped() {
        let (f, h, checks) = world();
        let s = recorded(&checks, &h, &f);
        let mut now = vec![checks[0].clone(), check_on("fresh", "d.qnt")];
        let p = plan(&s, &now, &h, &f);
        assert_eq!(rerun_names(&p), ["fresh"]);
        assert_eq!(p.order, ["on_a", "fresh"]);
        assert_eq!(p.reuse.keys().collect::<Vec<_>>(), ["on_a"]);
        now.clear();
        assert!(plan(&s, &now, &h, &f).reuse.is_empty());
    }

    #[test]
    fn staleness_that_cannot_be_determined_reruns_the_check() {
        let (f, h, checks) = world();
        let s = recorded(&checks, &h, &f);
        // A closure file that is not a hashed input.
        let mut unhashed = h.clone();
        unhashed.remove("lib/c.qnt");
        assert_eq!(rerun_names(&plan(&s, &checks, &unhashed, &f)), ["on_a"]);
        // A closure file the lexer could not index.
        let mut unindexed = f.clone();
        unindexed.remove("lib/b.qnt");
        assert_eq!(rerun_names(&plan(&s, &checks, &h, &unindexed)), ["on_a"]);
        // A record without a key (recorded before incremental recording).
        let mut keyless = s.clone();
        keyless.checks[1].input_key = None;
        assert_eq!(rerun_names(&plan(&keyless, &checks, &h, &f)), ["on_d"]);
        // Another schema.
        let mut schema = s;
        schema.schema += 1;
        assert_eq!(rerun_names(&plan(&schema, &checks, &h, &f)), ["on_a", "on_d"]);
        // No usable summary.
        for prior in [Prior::Absent, Prior::Unreadable("x".into())] {
            let p = plan_record(&prior, false, &tools(), &checks, &h, &f);
            assert_eq!(rerun_names(&p), ["on_a", "on_d"]);
        }
    }

    #[test]
    fn unusable_recorded_results_are_not_carried_over() {
        let (f, h, checks) = world();
        let s = recorded(&checks, &h, &f);
        let mut missed = s.clone();
        missed.checks[0].outcome = Outcome::TimedOut;
        assert_eq!(rerun_names(&plan(&missed, &checks, &h, &f)), ["on_a"]);
        let mut untraced = s.clone();
        untraced.checks[0].trace_error =
            Some(TraceRecordError::Unreadable { file: "x".into(), error: "e".into() });
        assert_eq!(rerun_names(&plan(&untraced, &checks, &h, &f)), ["on_a"]);
        // A referenced evidence file that is absent from `evidence/`.
        let mut gone = s.clone();
        gone.checks[1].trace = Some("traces/on_d.itf.json".into());
        let p =
            plan_record(&Prior::Found(Box::new(gone), files(&s)), false, &tools(), &checks, &h, &f);
        assert_eq!(rerun_names(&p), ["on_d"]);
    }

    #[test]
    fn full_reruns_everything_even_when_current() {
        let (f, h, checks) = world();
        let s = recorded(&checks, &h, &f);
        let present = files(&s);
        let p = plan_record(&Prior::Found(Box::new(s), present), true, &tools(), &checks, &h, &f);
        assert_eq!(rerun_names(&p), ["on_a", "on_d"]);
        assert!(p.reuse.is_empty());
    }

    #[test]
    fn verify_accepts_keyed_evidence_while_an_unrelated_file_changes() {
        let (f, mut h, checks) = world();
        let s = recorded(&checks, &h, &f);
        h.insert("hazard/x.qnt".into(), "x1".into());
        let keys = check_keys(&checks, &h, &f, &tools());
        assert_eq!(verify_summary(&s, &h, &keys, &checks, &files(&s)), vec![]);
    }

    #[test]
    fn verify_rejects_a_check_whose_inputs_changed_after_recording() {
        let (f, mut h, checks) = world();
        let s = recorded(&checks, &h, &f);
        h.insert("lib/b.qnt".into(), "b2".into());
        let keys = check_keys(&checks, &h, &f, &tools());
        let problems = verify_summary(&s, &h, &keys, &checks, &files(&s));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(matches!(&problems[0], EvidenceProblem::CheckStale { name, .. } if name == "on_a"));
    }

    #[test]
    fn verify_falls_back_to_global_hashes_without_input_keys() {
        let (f, mut h, checks) = world();
        let mut s = recorded(&checks, &h, &f);
        s.inputs = h.clone();
        s.checks[0].input_key = None;
        h.insert("d.qnt".into(), "d2".into());
        let keys = check_keys(&checks, &h, &f, &tools());
        assert_eq!(
            verify_summary(&s, &h, &keys, &checks, &files(&s)),
            vec![EvidenceProblem::InputChanged { file: "d.qnt".into() }]
        );
    }

    #[test]
    fn an_input_key_roundtrips_through_summary_json_and_is_omitted_when_absent() {
        let (f, h, checks) = world();
        let s = recorded(&checks, &h, &f);
        let back: EvidenceSummary =
            serde_json::from_str(&serde_json::to_string(&s).expect("encode")).expect("decode");
        assert_eq!(back, s);
        let mut bare = s;
        bare.checks[0].input_key = None;
        bare.checks[0].run_id = None;
        let json = serde_json::to_string(&bare.checks[0]).expect("encode");
        assert!(!json.contains("input_key") && !json.contains("run_id"), "{json}");
    }
}
