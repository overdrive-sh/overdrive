//! The runner (DDD-5, DDD-6, DDD-7, DDD-8, DDD-9): `run_main` end to end.
//!
//! Wiring order: read the arguments and the checkout marker, probe the host, read every setting
//! and decide eligibility, create the result directory and announce it, enable statistics (only
//! when eligible and declared), write `run_started`, then, when refused, write `run_refused` and
//! stop; else canary, run the schedule, canary, finish. The run's only effects go through the
//! ports of [`Runner`], so the whole flow runs against fixtures in unit tests.

use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use rand::RngCore;
use rand::rngs::OsRng;
use time::OffsetDateTime;
use time::macros::format_description;

use crate::args::Args;
use crate::host::{HostProbe, HostReading, ProcSysProbe};
use crate::marker::{self, MARKER_SETTING, Marker};
use crate::record::{
    BpfStatsMode, Canary, CanaryPhase, Clock, ClockSource, Declaration, EvidenceClass, GuestBlock,
    GuestObserved, GuestView, Host, Line, METHODOLOGY_VERSION, MetricDecl, ObservedArtifacts,
    Outcome, ProfileDecl, RecordWriter, ReferenceDecl, ReplicateLine, RunFinished, RunRefused,
    RunStarted, SCHEMA_VERSION, Substrate, Unit,
};
use crate::registry::{self, BenchmarkDecl};
use crate::replicate::{HostClock, Replicate, ReplicateFailure};
use crate::report::render;
use crate::schedule::plan;
use crate::stats_hold::{AyaStatsEnabler, BpfStatsHold, StatsEnabler, ineffective_program};
use crate::{Error, Evidence, Result};

const IMPLICIT_PROFILE: &str = "default";
const RECORD_FILE: &str = "record.jsonl";
const REPORT_FILE: &str = "report.md";
const RESULT_DIR_LINE: &str = "OVERDRIVE_BENCH_RESULT_DIR=";
const CANARY_SAMPLES: usize = 16;
const CANARY_WARMUP_CONNECTS: u32 = 200;
const CANARY_TIMED_CONNECTS: u32 = 2_000;
const RUN_ID_HEX_DIGITS: usize = 24;
const SOURCE_ROOT_VARIABLE: &str = "OVERDRIVE_METAL_SOURCE_ROOT";
const RUN_ID_VARIABLE: &str = "OVERDRIVE_METAL_RUN_ID";

// ---------------------------------------------------------------------------------------
// Exit statuses and diagnostics (DDD-9).

/// The runner's exit status for `error`: 64 usage, 78 refused, 70 internal.
pub const fn exit_status(error: &Error) -> u8 {
    match error {
        Error::Usage { .. } | Error::FamilyNotDeclared { .. } => 64,
        Error::Refused { .. } | Error::Unreadable { .. } => 78,
        Error::StatsEnable { .. }
        | Error::Record { .. }
        | Error::Canary { .. }
        | Error::Stdout { .. }
        | Error::InvalidRecord { .. } => 70,
    }
}

/// The one stderr line that explains `error`.
fn diagnostic(error: &Error) -> String {
    match error {
        Error::Refused { setting, found, pinned } => {
            format!("bench: refused: {setting}: found {found}, pinned {pinned}")
        }
        Error::Unreadable { setting, source } => format!("bench: refused: {setting}: {source}"),
        // These Display texts already carry their cause.
        Error::Record { .. } | Error::Canary { .. } | Error::Stdout { .. } => {
            format!("bench: {error}")
        }
        other => std::error::Error::source(other)
            .map_or_else(|| format!("bench: {other}"), |cause| format!("bench: {other}: {cause}")),
    }
}

// ---------------------------------------------------------------------------------------
// Process entry.

/// Run the benchmark binary: the process's arguments and environment, the real host.
pub fn main(evidence: Evidence) -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mut stdout = io::stdout().lock();
    let mut canary = LoopbackCanary::default();
    let outcome = Environment::from_process().and_then(|environment| {
        Runner {
            evidence,
            debug_assertions: cfg!(debug_assertions),
            declared: &registry::BENCHMARKS,
            probe: &ProcSysProbe,
            enabler: &AyaStatsEnabler,
            sink: &FileSink,
            canary: &mut canary,
            stdout: &mut stdout,
        }
        .run(&arguments, &environment)
    });
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // A refused run's reason is the only thing the operator reads; stderr is its channel.
            let _ = writeln!(io::stderr(), "{}", diagnostic(&error));
            ExitCode::from(exit_status(&error))
        }
    }
}

/// What the process supplies to a run: the checkout, the run id and the seed.
pub struct Environment {
    pub(crate) source_root: Option<PathBuf>,
    pub(crate) run_id: String,
    pub(crate) seed: u64,
}

impl Environment {
    fn from_process() -> Result<Self> {
        let run_id = match std::env::var_os(RUN_ID_VARIABLE) {
            Some(value) => {
                value.into_string().ok().filter(|id| is_run_id(id)).ok_or_else(|| Error::Usage {
                    reason: format!(
                        "{RUN_ID_VARIABLE} is not {RUN_ID_HEX_DIGITS} lowercase hex digits"
                    ),
                })?
            }
            None => generated_run_id(),
        };
        Ok(Self {
            source_root: std::env::var_os(SOURCE_ROOT_VARIABLE).map(PathBuf::from),
            run_id,
            seed: OsRng.next_u64(),
        })
    }
}

fn is_run_id(text: &str) -> bool {
    text.len() == RUN_ID_HEX_DIGITS
        && text.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// 96 bits of OS entropy as 24 lowercase hex digits.
fn generated_run_id() -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    let bits = u128::from_le_bytes(bytes) >> 32;
    format!("{bits:024x}")
}

// ---------------------------------------------------------------------------------------
// Driven ports not owned by another module.

/// Where a run's record and report go: an append-only file in a result directory.
pub trait RecordSink {
    /// Create the run's directory (it must not exist) and open its record for appending.
    fn create(&self, dir: &Path) -> io::Result<Box<dyn Write>>;
    /// The record as written so far.
    fn read_record(&self, dir: &Path) -> io::Result<String>;
    /// Write the derived report next to the record.
    fn write_report(&self, dir: &Path, text: &str) -> io::Result<()>;
    /// Write `bytes` at `dir`/`relative`, create-new, creating the parent directories (DDD-9).
    fn write_artifact(&self, dir: &Path, relative: &Path, bytes: &[u8]) -> io::Result<()>;
}

/// The result directory on disk.
pub struct FileSink;

impl RecordSink for FileSink {
    fn create(&self, dir: &Path) -> io::Result<Box<dyn Write>> {
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::create_dir(dir)?;
        let record = std::fs::OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(dir.join(RECORD_FILE))?;
        Ok(Box::new(record))
    }

    fn read_record(&self, dir: &Path) -> io::Result<String> {
        std::fs::read_to_string(dir.join(RECORD_FILE))
    }

    fn write_report(&self, dir: &Path, text: &str) -> io::Result<()> {
        std::fs::write(dir.join(REPORT_FILE), text)
    }

    fn write_artifact(&self, dir: &Path, relative: &Path, bytes: &[u8]) -> io::Result<()> {
        let path = dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(bytes)
    }
}

/// One canary sample: the mean connect-and-close time of a loopback connection, in ns.
pub trait CanarySampler {
    fn sample(&mut self) -> io::Result<f64>;
}

/// The loopback connect-and-close loop (DDD-5), against a listener it starts on first use.
#[derive(Default)]
pub struct LoopbackCanary {
    server: Option<AcceptLoop>,
}

impl CanarySampler for LoopbackCanary {
    fn sample(&mut self) -> io::Result<f64> {
        let address = match &self.server {
            Some(server) => server.address,
            None => self.server.insert(AcceptLoop::start()?).address,
        };
        for _ in 0..CANARY_WARMUP_CONNECTS {
            connect_and_close(address)?;
        }
        let clock = HostClock;
        let begin = clock.now();
        for _ in 0..CANARY_TIMED_CONNECTS {
            connect_and_close(address)?;
        }
        let elapsed = clock.now().duration_since(begin);
        Ok(elapsed.as_secs_f64() * 1e9 / f64::from(CANARY_TIMED_CONNECTS))
    }
}

fn connect_and_close(address: SocketAddr) -> io::Result<()> {
    TcpStream::connect(address).map(drop)
}

/// A listener that accepts and drops every connection until it is dropped.
struct AcceptLoop {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl AcceptLoop {
    fn start() -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let address = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&stop);
        let thread = std::thread::Builder::new().name("bench-canary-accept".to_owned()).spawn(
            move || {
                for connection in listener.incoming() {
                    if observed.load(Ordering::Acquire) {
                        return;
                    }
                    drop(connection);
                }
            },
        )?;
        Ok(Self { address, stop, thread: Some(thread) })
    }
}

impl Drop for AcceptLoop {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Wake the blocked `accept` so the thread observes the flag and returns.
        drop(TcpStream::connect(self.address));
        if let Some(thread) = self.thread.take() {
            drop(thread.join());
        }
    }
}

// ---------------------------------------------------------------------------------------
// The run.

/// One run's ports and the benchmarks it may select from.
pub struct Runner<'a> {
    pub(crate) evidence: Evidence,
    /// Whether this binary was built with debug assertions (`cfg!(debug_assertions)`).
    pub(crate) debug_assertions: bool,
    pub(crate) declared: &'a [BenchmarkDecl],
    pub(crate) probe: &'a dyn HostProbe,
    pub(crate) enabler: &'a dyn StatsEnabler,
    pub(crate) sink: &'a dyn RecordSink,
    pub(crate) canary: &'a mut dyn CanarySampler,
    pub(crate) stdout: &'a mut dyn Write,
}

impl Runner<'_> {
    pub fn run(&mut self, arguments: &[String], environment: &Environment) -> Result<()> {
        let args = Args::parse(arguments)?;
        let decl = registry::select(&args.family, self.declared)?;
        let source_root = environment.source_root.as_deref().ok_or_else(missing_source_root)?;
        let results_root = args.results_root_within(source_root)?;
        let marker = marker::read(source_root)?;

        let reading = HostReading::read(self.probe);
        let host = reading.host_block();
        let eligibility = reading.decide(self.debug_assertions);

        let dir = results_root.join(decl.family).join(&environment.run_id);
        let mut record = self.open_record(&dir, source_root)?;
        let start = Start {
            decl,
            marker: &marker,
            environment,
            host,
            evidence: self.evidence,
            debug_assertions: self.debug_assertions,
        };

        if let Err(refusal) = eligibility {
            record.append(&Line::RunStarted(Box::new(start.line(BpfStatsMode::Off))))?;
            record.append(&Line::RunRefused(refused_line(&refusal)))?;
            self.write_report(&dir)?;
            return Err(refusal);
        }
        let hold = if decl.bpf_stats {
            match self.enabler.enable() {
                Ok(descriptor) => Some(Arc::new(BpfStatsHold::new(descriptor))),
                Err(source) => {
                    record.append(&Line::RunStarted(Box::new(start.line(BpfStatsMode::Off))))?;
                    record.append(&Line::RunRefused(RunRefused {
                        setting: "bpf_stats".to_owned(),
                        found: None,
                        pinned: None,
                        reason: Some(source.to_string()),
                    }))?;
                    self.write_report(&dir)?;
                    return Err(Error::StatsEnable { source });
                }
            }
        } else {
            None
        };
        let mode = if hold.is_some() { BpfStatsMode::PerProcess } else { BpfStatsMode::Off };
        record.append(&Line::RunStarted(Box::new(start.line(mode))))?;

        self.canary_samples(&mut record, CanaryPhase::Start)?;
        let counts = execute(
            decl,
            environment.seed,
            hold.as_ref(),
            &mut record,
            (self.sink, dir.as_path()),
        )?;
        self.canary_samples(&mut record, CanaryPhase::End)?;
        record.append(&Line::RunFinished(RunFinished {
            finished: now(),
            replicates_ok: counts.ok,
            replicates_failed: counts.failed,
        }))?;
        self.write_report(&dir)
    }

    /// Create the result directory, announce it (repo-relative) and open the record.
    fn open_record(
        &mut self,
        dir: &Path,
        source_root: &Path,
    ) -> Result<RecordWriter<Box<dyn Write>>> {
        let path = dir.join(RECORD_FILE);
        let out =
            self.sink.create(dir).map_err(|source| Error::Record { path: path.clone(), source })?;
        let relative = dir.strip_prefix(source_root).unwrap_or(dir);
        writeln!(self.stdout, "{RESULT_DIR_LINE}{}", relative.display())
            .and_then(|()| self.stdout.flush())
            .map_err(|source| Error::Stdout { source })?;
        Ok(RecordWriter::new(out, path))
    }

    fn canary_samples(
        &mut self,
        record: &mut RecordWriter<Box<dyn Write>>,
        phase: CanaryPhase,
    ) -> Result<()> {
        let mut samples_ns = Vec::with_capacity(CANARY_SAMPLES);
        for _ in 0..CANARY_SAMPLES {
            samples_ns.push(self.canary.sample().map_err(|source| Error::Canary { source })?);
        }
        record.append(&Line::Canary(Canary { phase, samples_ns }))
    }

    /// Render the record that is on disk into `report.md`: one renderer, no subprocess.
    fn write_report(&self, dir: &Path) -> Result<()> {
        let failed = |source| Error::Record { path: dir.join(REPORT_FILE), source };
        let text = self.sink.read_record(dir).map_err(failed)?;
        let report = render(&text)?;
        self.sink.write_report(dir, &report).map_err(failed)
    }
}

fn missing_source_root() -> Error {
    Error::Unreadable {
        setting: MARKER_SETTING.to_owned(),
        source: io::Error::new(
            io::ErrorKind::NotFound,
            format!("{SOURCE_ROOT_VARIABLE} is not set, so the checkout marker cannot be located"),
        ),
    }
}

/// The terminal line that states `refusal`.
fn refused_line(refusal: &Error) -> RunRefused {
    match refusal {
        Error::Refused { setting, found, pinned } => RunRefused {
            setting: setting.clone(),
            found: Some(found.clone()),
            pinned: Some(pinned.clone()),
            reason: None,
        },
        Error::Unreadable { setting, source } => RunRefused {
            setting: setting.clone(),
            found: None,
            pinned: None,
            reason: Some(source.to_string()),
        },
        other => {
            unreachable!("host eligibility only refuses with Refused or Unreadable, not {other:?}")
        }
    }
}

/// What `run_started` is built from.
struct Start<'a> {
    decl: &'a BenchmarkDecl,
    marker: &'a Marker,
    environment: &'a Environment,
    host: std::collections::BTreeMap<String, crate::record::HostValue>,
    evidence: Evidence,
    debug_assertions: bool,
}

impl Start<'_> {
    fn guest(&self) -> bool {
        self.decl.substrate == Substrate::Guest.as_str()
    }

    fn line(&self, bpf_stats: BpfStatsMode) -> RunStarted {
        RunStarted {
            schema: SCHEMA_VERSION,
            methodology: METHODOLOGY_VERSION,
            run_id: self.environment.run_id.clone(),
            family: self.decl.family.to_owned(),
            benchmark: self.decl.benchmark.to_owned(),
            evidence: match self.evidence {
                Evidence::Maintained => EvidenceClass::Maintained,
                Evidence::Spike => EvidenceClass::Spike,
            },
            substrate: if self.guest() { Substrate::Guest } else { Substrate::Native },
            commit: self.marker.commit.clone(),
            dirty: self.marker.dirty,
            debug_assertions: self.debug_assertions,
            seed: self.environment.seed,
            started: now(),
            host: Host(self.host.clone()),
            clock: Clock {
                host: ClockSource::MonotonicRaw,
                service: self.decl.service_clock,
                guest: self.decl.guest_clock,
            },
            guest: self.guest().then_some(GuestBlock),
            bpf_stats,
            declaration: declaration(self.decl),
        }
    }
}

/// The echo of the attribute (DDD-6): no profile declared echoes one `default` profile.
fn declaration(decl: &BenchmarkDecl) -> Declaration {
    let profiles = if decl.profiles.is_empty() {
        vec![ProfileDecl { name: IMPLICIT_PROFILE.to_owned(), preparation: String::new() }]
    } else {
        decl.profiles
            .iter()
            .map(|profile| ProfileDecl {
                name: profile.name.to_owned(),
                preparation: profile.preparation.to_owned(),
            })
            .collect()
    };
    Declaration {
        arms: decl.arms.iter().map(|arm| (*arm).to_owned()).collect(),
        replicates: decl.replicates,
        profiles,
        metrics: decl
            .metrics
            .iter()
            .map(|metric| MetricDecl {
                name: metric.name.to_owned(),
                unit: Unit::parse(metric.unit).unwrap_or_else(|| {
                    unreachable!("the attribute rejects unit `{}`", metric.unit)
                }),
                table: metric.table.to_owned(),
                series: metric.series,
            })
            .collect(),
        references: decl
            .references
            .iter()
            .map(|reference| ReferenceDecl {
                metric: reference.metric.to_owned(),
                value: reference.value,
                label: reference.label.to_owned(),
                source: reference.source.to_owned(),
            })
            .collect(),
    }
}

/// How many replicates finished and failed.
struct Counts {
    ok: u64,
    failed: u64,
}

/// Where a run's artifacts go: the sink and the result directory.
type Artifacts<'a> = (&'a dyn RecordSink, &'a Path);

/// Run every profile's schedule, appending one `replicate` line per run. The first view a
/// replicate reports becomes the run's one `guest_observed` line, right before that replicate's
/// line.
fn execute(
    decl: &BenchmarkDecl,
    seed: u64,
    hold: Option<&Arc<BpfStatsHold>>,
    record: &mut RecordWriter<Box<dyn Write>>,
    artifacts: Artifacts<'_>,
) -> Result<Counts> {
    let mut counts = Counts { ok: 0, failed: 0 };
    let mut view_written = false;
    let profiles: Vec<&str> = if decl.profiles.is_empty() {
        vec![IMPLICIT_PROFILE]
    } else {
        decl.profiles.iter().map(|profile| profile.name).collect()
    };
    for (index, profile) in (0_u32..).zip(profiles) {
        for planned in plan(decl.arms, decl.replicates, index, seed) {
            let (line, view) = run_one(decl, profile, &planned, hold, artifacts);
            match line.outcome {
                Outcome::Ok => counts.ok += 1,
                Outcome::Failed => counts.failed += 1,
            }
            if let (false, Some(view)) = (view_written, view) {
                view_written = true;
                record.append(&Line::GuestObserved(observed(view)))?;
            }
            record.append(&Line::Replicate(Box::new(line)))?;
        }
    }
    Ok(counts)
}

/// The record's form of the view a replicate reported.
fn observed(view: GuestView) -> GuestObserved {
    GuestObserved {
        vcpus: view.vcpus,
        memory_mib: view.memory_mib,
        kernel: view.kernel,
        clocksource: view.clocksource,
        artifacts: ObservedArtifacts {
            overdrive_init: view.artifacts.overdrive_init,
            overdrive_guest_peer: view.artifacts.overdrive_guest_peer,
            ovd_fwd: view.artifacts.ovd_fwd,
        },
    }
}

/// The console's place under the result directory (DDD-9).
fn console_path(profile: &str, planned: &crate::schedule::Planned) -> PathBuf {
    Path::new("artifacts")
        .join(profile)
        .join(format!("{}-{}-{}", planned.block, planned.position, planned.arm))
        .join("console.log")
}

/// One replicate: a fresh handle, the body once, and what it recorded; with the view it reported.
fn run_one(
    decl: &BenchmarkDecl,
    profile: &str,
    planned: &crate::schedule::Planned,
    hold: Option<&Arc<BpfStatsHold>>,
    (sink, dir): Artifacts<'_>,
) -> (ReplicateLine, Option<GuestView>) {
    if let Some(hold) = hold {
        // Reads of an earlier replicate are not this one's evidence of effectiveness.
        drop(hold.take_reads());
    }
    let mut replicate = Replicate::new(decl, &planned.arm, profile, hold.cloned());
    let body = (decl.run)(&mut replicate);
    let finished = replicate.finish();
    let console_failure = finished.console.as_deref().and_then(|text| {
        sink.write_artifact(dir, &console_path(profile, planned), text.as_bytes())
            .err()
            .map(|source| format!("console not retained: {source}"))
    });
    let ineffective = hold.and_then(|hold| ineffective_program(&hold.take_reads()));
    let failure = match body {
        Err(failure) => Some(console_failure.map_or_else(
            || failure.to_string(),
            |not_retained| format!("{failure}; {not_retained}"),
        )),
        Ok(()) => finished.failure.or(console_failure).or_else(|| {
            ineffective.map(|id| ReplicateFailure::stats_not_effective(id).to_string())
        }),
    };
    let line = ReplicateLine {
        profile: profile.to_owned(),
        arm: planned.arm.clone(),
        block: planned.block,
        position: planned.position,
        role: planned.role,
        outcome: if failure.is_some() { Outcome::Failed } else { Outcome::Ok },
        failure,
        metrics: finished.metrics,
        notes: finished.notes,
        commands: finished.commands,
    };
    (line, finished.view)
}

fn now() -> String {
    OffsetDateTime::now_utc()
        .format(format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z"))
        .unwrap_or_else(|error| unreachable!("a fixed UTC format description formats: {error}"))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "tests assert their own preconditions and compare exact stored floats"
)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::sync::Mutex;

    use super::*;
    use crate::host::tests::Fixture;
    use crate::record::validate;
    use crate::registry::{BenchmarkDecl, MetricDecl};

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
    const RUN_ID: &str = "00112233445566778899aabb";
    const METRICS: [MetricDecl; 1] =
        [MetricDecl { name: "ns_per_op", unit: "ns", table: "cost", series: false }];

    fn body(replicate: &mut Replicate) -> Result<(), ReplicateFailure> {
        match replicate.arm() {
            "broken" => Err(ReplicateFailure::fixture("the fixture did not build")),
            "typo" => replicate.record("ns_per_opp", 1.0),
            _ => replicate.record("ns_per_op", 100.0),
        }
    }

    fn decl(arms: &'static [&'static str], bpf_stats: bool) -> BenchmarkDecl {
        BenchmarkDecl {
            family: "unit-family",
            benchmark: "unit_bench",
            substrate: "native",
            arms,
            replicates: 9,
            profiles: &[],
            metrics: &METRICS,
            bpf_stats,
            references: &[],
            service_clock: None,
            guest_clock: None,
            run: body,
        }
    }

    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("lock").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct MemorySink {
        created: RefCell<Vec<PathBuf>>,
        record: Arc<Mutex<Vec<u8>>>,
        report: RefCell<Option<String>>,
        /// Every artifact written: the directory, the relative path and the bytes.
        artifacts: RefCell<Vec<(PathBuf, PathBuf, Vec<u8>)>>,
        /// Makes every artifact write fail (a full disk).
        artifact_writes_fail: Cell<bool>,
    }

    impl MemorySink {
        fn record(&self) -> String {
            String::from_utf8(self.record.lock().expect("lock").clone()).expect("utf-8")
        }

        fn lines(&self) -> Vec<serde_json::Value> {
            self.record().lines().map(|line| serde_json::from_str(line).expect("json")).collect()
        }
    }

    impl RecordSink for MemorySink {
        fn create(&self, dir: &Path) -> io::Result<Box<dyn Write>> {
            self.created.borrow_mut().push(dir.to_owned());
            Ok(Box::new(Shared(Arc::clone(&self.record))))
        }

        fn read_record(&self, _dir: &Path) -> io::Result<String> {
            Ok(self.record())
        }

        fn write_report(&self, _dir: &Path, text: &str) -> io::Result<()> {
            *self.report.borrow_mut() = Some(text.to_owned());
            Ok(())
        }

        fn write_artifact(&self, dir: &Path, relative: &Path, bytes: &[u8]) -> io::Result<()> {
            if self.artifact_writes_fail.get() {
                return Err(io::Error::other("disk full"));
            }
            self.artifacts.borrow_mut().push((dir.to_owned(), relative.to_owned(), bytes.to_vec()));
            Ok(())
        }
    }

    #[derive(Default)]
    struct CountingCanary {
        samples: Cell<usize>,
    }

    impl CanarySampler for CountingCanary {
        fn sample(&mut self) -> io::Result<f64> {
            self.samples.set(self.samples.get() + 1);
            Ok(1000.0 + f64::from(u32::try_from(self.samples.get() % 3).unwrap_or(0)))
        }
    }

    struct Enabler(Option<&'static str>);

    impl StatsEnabler for Enabler {
        fn enable(&self) -> std::result::Result<std::os::fd::OwnedFd, aya::sys::SyscallError> {
            match self.0 {
                None => Ok(std::fs::File::open("/dev/null").expect("open").into()),
                Some(_) => Err(aya::sys::SyscallError {
                    call: "bpf",
                    io_error: io::Error::from(io::ErrorKind::PermissionDenied),
                }),
            }
        }
    }

    struct Outcome {
        result: Result<()>,
        sink: MemorySink,
        canary_samples: usize,
        stdout: String,
    }

    fn marker_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(".overdrive-metal-source"),
            format!("commit={COMMIT}\nworkspace=/w\nsource_digest=ab\n"),
        )
        .expect("marker");
        dir
    }

    fn run_with(
        decl: &BenchmarkDecl,
        probe: &Fixture,
        enabler: &Enabler,
        debug_assertions: bool,
        root: &Path,
        line: &str,
    ) -> Outcome {
        let declared = std::slice::from_ref(decl);
        let sink = MemorySink::default();
        let mut canary = CountingCanary::default();
        let mut stdout = Vec::new();
        let environment =
            Environment { source_root: Some(root.to_owned()), run_id: RUN_ID.to_owned(), seed: 42 };
        let arguments: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        let result = Runner {
            evidence: Evidence::Maintained,
            debug_assertions,
            declared,
            probe,
            enabler,
            sink: &sink,
            canary: &mut canary,
            stdout: &mut stdout,
        }
        .run(&arguments, &environment);
        Outcome {
            result,
            sink,
            canary_samples: canary.samples.get(),
            stdout: String::from_utf8(stdout).expect("utf-8"),
        }
    }

    fn run(decl: &BenchmarkDecl, probe: &Fixture) -> (Outcome, tempfile::TempDir) {
        let dir = marker_dir();
        let line = format!(
            "run --family unit-family --results-root {}/benchmarks/results --bench",
            dir.path().display()
        );
        (run_with(decl, probe, &Enabler(None), false, dir.path(), &line), dir)
    }

    #[test]
    fn exit_statuses_follow_the_error_taxonomy() {
        let io = || io::Error::from(io::ErrorKind::Other);
        let statuses = [
            (Error::Usage { reason: String::new() }, 64),
            (Error::FamilyNotDeclared { family: String::new(), found: Vec::new() }, 64),
            (
                Error::Refused {
                    setting: String::new(),
                    found: String::new(),
                    pinned: String::new(),
                },
                78,
            ),
            (Error::Unreadable { setting: String::new(), source: io() }, 78),
            (
                Error::StatsEnable {
                    source: aya::sys::SyscallError { call: "bpf", io_error: io() },
                },
                70,
            ),
            (Error::Record { path: PathBuf::new(), source: io() }, 70),
            (Error::Canary { source: io() }, 70),
            (Error::Stdout { source: io() }, 70),
        ];
        for (error, status) in statuses {
            assert_eq!(exit_status(&error), status, "{error:?}");
        }
    }

    #[test]
    fn an_eligible_two_arm_run_writes_a_valid_record_and_its_report() {
        let decl = decl(&["fast", "slow"], false);
        let (outcome, dir) = run(&decl, &Fixture::eligible());
        outcome.result.expect("the run finishes");
        validate(&outcome.sink.record()).expect("the record validates");
        let lines = outcome.sink.lines();
        assert_eq!(lines[0]["t"], "run_started");
        assert_eq!(lines[0]["bpf_stats"], "off");
        assert_eq!(lines[0]["commit"], COMMIT);
        assert_eq!(lines[0]["dirty"], true);
        assert_eq!(lines[0]["seed"], 42);
        assert_eq!(lines[1]["t"], "canary");
        assert_eq!(lines[1]["phase"], "start");
        assert_eq!(lines.iter().filter(|line| line["t"] == "replicate").count(), 27);
        let last = lines.last().expect("lines");
        assert_eq!(
            (last["t"].as_str(), last["replicates_ok"].as_u64()),
            (Some("run_finished"), Some(27))
        );
        assert_eq!(outcome.canary_samples, 32, "16 samples at the start and 16 at the end");
        assert_eq!(
            outcome.stdout,
            format!("OVERDRIVE_BENCH_RESULT_DIR=benchmarks/results/unit-family/{RUN_ID}\n")
        );
        let report = outcome.sink.report.borrow().clone().expect("report written");
        assert!(report.contains("- status: finished"), "{report}");
        drop(dir);
    }

    #[test]
    fn run_started_names_the_declared_clock_sources_and_null_for_an_omitted_one() {
        let cases = [
            (None, None, serde_json::Value::Null, serde_json::Value::Null),
            (
                Some(registry::ServiceClock::InjectedMonotonic),
                None,
                serde_json::json!("Clock::now (monotonic)"),
                serde_json::Value::Null,
            ),
            (
                None,
                Some(registry::GuestClock::InitElapsedMs),
                serde_json::Value::Null,
                serde_json::json!("CLOCK_MONOTONIC (overdrive-init Instant::elapsed, 1 ms)"),
            ),
        ];
        for (service, guest, expected_service, expected_guest) in cases {
            let mut decl = decl(&["fast", "slow"], false);
            decl.service_clock = service;
            decl.guest_clock = guest;
            let (outcome, _dir) = run(&decl, &Fixture::eligible());
            outcome.result.as_ref().expect("the run finishes");
            validate(&outcome.sink.record()).expect("the record validates");
            let clock = &outcome.sink.lines()[0]["clock"];
            assert_eq!(clock["host"], "CLOCK_MONOTONIC_RAW");
            assert_eq!(clock["service"], expected_service);
            assert_eq!(clock["guest"], expected_guest);
        }
    }

    #[test]
    fn a_drifted_host_leaves_run_started_then_run_refused_and_takes_no_sample() {
        let decl = decl(&["fast", "slow"], true);
        let (outcome, _dir) =
            run(&decl, &Fixture::eligible().with("/sys/devices/system/cpu/smt/control", "off\n"));
        assert!(matches!(&outcome.result, Err(Error::Refused { setting, .. }) if setting == "smt"));
        let lines = outcome.sink.lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            (lines[0]["t"].as_str(), lines[0]["bpf_stats"].as_str()),
            (Some("run_started"), Some("off"))
        );
        assert_eq!(lines[1]["t"], "run_refused");
        assert_eq!(lines[1]["setting"], "smt");
        assert_eq!(
            (lines[1]["found"].as_str(), lines[1]["pinned"].as_str()),
            (Some("off"), Some("on"))
        );
        assert_eq!(outcome.canary_samples, 0);
        assert!(
            outcome.stdout.starts_with("OVERDRIVE_BENCH_RESULT_DIR="),
            "printed for a refused run too"
        );
        let report = outcome.sink.report.borrow().clone().expect("report written");
        assert!(report.contains("- status: refused"), "{report}");
        validate(&outcome.sink.record()).expect("a refused record validates");
    }

    #[test]
    fn a_failed_probe_refuses_as_host_probe() {
        let mut probe = Fixture::eligible();
        probe.probe_fails = true;
        let (outcome, _dir) = run(&decl(&["a"], false), &probe);
        assert!(
            matches!(&outcome.result, Err(Error::Refused { setting, .. }) if setting == "host_probe")
        );
        assert_eq!(outcome.sink.lines()[1]["setting"], "host_probe");
    }

    #[test]
    fn a_debug_build_is_refused_naming_debug_assertions() {
        let decl = decl(&["a"], false);
        let dir = marker_dir();
        let line = format!("run --family unit-family --results-root {}/r", dir.path().display());
        let outcome =
            run_with(&decl, &Fixture::eligible(), &Enabler(None), true, dir.path(), &line);
        assert!(
            matches!(&outcome.result, Err(Error::Refused { setting, found, .. }) if setting == "debug_assertions" && found == "on")
        );
        assert_eq!(outcome.sink.lines()[0]["debug_assertions"], true);
    }

    #[test]
    fn a_marker_that_cannot_be_read_refuses_before_any_directory_or_output() {
        let decl = decl(&["a"], false);
        let dir = tempfile::tempdir().expect("tempdir");
        let line = format!("run --family unit-family --results-root {}/r", dir.path().display());
        let outcome =
            run_with(&decl, &Fixture::eligible(), &Enabler(None), false, dir.path(), &line);
        assert!(
            matches!(&outcome.result, Err(Error::Unreadable { setting, .. }) if setting == ".overdrive-metal-source")
        );
        assert!(outcome.sink.created.borrow().is_empty(), "no result directory");
        assert!(outcome.stdout.is_empty(), "no result directory line");
        assert!(outcome.sink.record().is_empty());
    }

    #[test]
    fn a_failed_statistics_enable_refuses_as_bpf_stats_with_a_reason() {
        let decl = decl(&["a"], true);
        let dir = marker_dir();
        let line = format!("run --family unit-family --results-root {}/r", dir.path().display());
        let outcome = run_with(
            &decl,
            &Fixture::eligible(),
            &Enabler(Some("denied")),
            false,
            dir.path(),
            &line,
        );
        assert!(matches!(outcome.result, Err(Error::StatsEnable { .. })));
        let lines = outcome.sink.lines();
        assert_eq!(lines[0]["bpf_stats"], "off");
        assert_eq!(lines[1]["setting"], "bpf_stats");
        assert!(lines[1]["reason"].is_string() && lines[1].get("found").is_none());
        assert_eq!(outcome.canary_samples, 0);
    }

    #[test]
    fn a_declared_statistics_benchmark_holds_the_descriptor_and_says_so() {
        let decl = decl(&["a"], true);
        let (outcome, _dir) = run(&decl, &Fixture::eligible());
        outcome.result.as_ref().expect("the run finishes");
        assert_eq!(outcome.sink.lines()[0]["bpf_stats"], "per_process");
    }

    #[test]
    fn a_failed_replicate_is_counted_and_the_run_continues() {
        let decl = decl(&["fine", "broken", "typo"], false);
        let (outcome, _dir) = run(&decl, &Fixture::eligible());
        outcome.result.as_ref().expect("the run finishes");
        let lines = outcome.sink.lines();
        let failed: Vec<&serde_json::Value> =
            lines.iter().filter(|line| line["outcome"] == "failed").collect();
        assert_eq!(failed.len(), 18, "the broken and the mistyped arm fail every replicate");
        assert!(failed.iter().any(|line| line["failure"] == "the fixture did not build"));
        assert!(
            failed.iter().any(|line| line["failure"]
                .as_str()
                .is_some_and(|text| text.contains("ns_per_opp")))
        );
        let last = lines.last().expect("lines");
        assert_eq!(last["replicates_failed"], 18);
        validate(&outcome.sink.record()).expect("the record validates");
    }

    #[test]
    fn a_declared_metric_the_body_never_recorded_is_one_not_recorded_failure() {
        #[allow(clippy::unnecessary_wraps, reason = "the signature is the benchmark body's")]
        fn silent(_: &mut Replicate) -> Result<(), ReplicateFailure> {
            Ok(())
        }
        let mut quiet = decl(&["a"], false);
        quiet.run = silent;
        let (outcome, _dir) = run(&quiet, &Fixture::eligible());
        outcome.result.as_ref().expect("the run finishes");
        let replicate = &outcome.sink.lines()[2];
        assert_eq!(replicate["outcome"], "ok");
        assert_eq!(replicate["metrics"]["ns_per_op"]["failures"][0], "not recorded");
    }

    #[test]
    fn usage_errors_leave_no_trace() {
        let decl = decl(&["a"], false);
        let dir = marker_dir();
        let outside = run_with(
            &decl,
            &Fixture::eligible(),
            &Enabler(None),
            false,
            dir.path(),
            "run --family unit-family --results-root /elsewhere",
        );
        assert!(matches!(outside.result, Err(Error::Usage { .. })));
        let relative = run_with(
            &decl,
            &Fixture::eligible(),
            &Enabler(None),
            false,
            dir.path(),
            "run --family unit-family --results-root rel",
        );
        assert!(matches!(relative.result, Err(Error::Usage { .. })));
        let undeclared = run_with(
            &decl,
            &Fixture::eligible(),
            &Enabler(None),
            false,
            dir.path(),
            "run --family other --results-root /x",
        );
        match &undeclared.result {
            Err(Error::FamilyNotDeclared { family, found }) => {
                assert_eq!((family.as_str(), found), ("other", &vec!["unit-family".to_owned()]));
            }
            other => panic!("expected FamilyNotDeclared, got {other:?}"),
        }
        for outcome in [&outside, &relative, &undeclared] {
            assert!(outcome.sink.created.borrow().is_empty() && outcome.stdout.is_empty());
        }
    }

    #[test]
    fn the_undeclared_family_diagnostic_names_requested_found_and_the_usual_cause() {
        let error =
            Error::FamilyNotDeclared { family: "x".to_owned(), found: vec!["y".to_owned()] };
        let text = diagnostic(&error);
        assert!(
            text.contains("`x`") && text.contains("\"y\"") && text.contains("use <crate> as _;"),
            "{text}"
        );
    }

    #[test]
    fn a_refusal_diagnostic_is_one_line_naming_the_setting_and_reason() {
        let marker = Error::Unreadable {
            setting: MARKER_SETTING.to_owned(),
            source: io::Error::new(io::ErrorKind::NotFound, "gone"),
        };
        assert_eq!(diagnostic(&marker), "bench: refused: .overdrive-metal-source: gone");
    }

    #[test]
    fn the_file_sink_creates_a_new_directory_once_and_writes_record_and_report() {
        let root = tempfile::tempdir().expect("tempdir");
        let dir = root.path().join("family").join(RUN_ID);
        let mut out = FileSink.create(&dir).expect("created");
        out.write_all(b"line\n").expect("write");
        out.flush().expect("flush");
        assert_eq!(FileSink.read_record(&dir).expect("read"), "line\n");
        assert!(FileSink.create(&dir).is_err(), "a run directory is never reused");
        FileSink.write_report(&dir, "report").expect("report");
        assert_eq!(std::fs::read_to_string(dir.join(REPORT_FILE)).expect("read"), "report");
    }

    #[test]
    fn the_generated_run_id_is_24_lowercase_hex_digits() {
        let id = generated_run_id();
        assert!(is_run_id(&id), "{id}");
        assert_ne!(id, generated_run_id());
        assert!(!is_run_id("ABCDEF012345678901234567"));
        assert!(!is_run_id("abc"));
    }

    // ---- The guest substrate (guest-benchmark-substrate, DDD-2, DDD-4, DDD-7, DDD-9, G-1, G-2) ----
    //
    // The runner is driven with a benchmark declaration whose `substrate` is `"guest"` and whose
    // body is scripted per replicate (the family's host half, replaced by a script): each
    // replicate reports a view, fails before or after reporting, or returns `Ok` without a view.
    // Evidence lane: unit (the runner's own ports, in-memory sink and canary). Layers 1-2: the
    // ordering invariant is a proptest (Mandate 9). CONTRACT_SHAPE per test: bounded-change
    // (the record the runner appends).

    use std::num::{NonZeroU8, NonZeroU32};

    use proptest::prelude::*;

    use crate::{GuestArtifacts, GuestMemoryMib, GuestView, HostValue};

    const GUEST_METRICS: [MetricDecl; 1] = [MetricDecl {
        name: "forwarded_round_trip_ns",
        unit: "ns",
        table: "forwarded-path",
        series: true,
    }];

    /// What one replicate of the scripted family body does.
    #[derive(Clone, Copy, Debug)]
    enum Step {
        /// Reports a view with this guest kernel, then records its series.
        Report(&'static str),
        /// Reports a view with this guest kernel, then fails with this reason.
        ReportThenFail(&'static str, &'static str),
        /// Records its series and returns `Ok` without reporting a view.
        Silent,
        /// Fails with this reason before reporting a view.
        Fail(&'static str),
    }

    thread_local! {
        static SCRIPT: RefCell<Vec<Step>> = const { RefCell::new(Vec::new()) };
        static NEXT: Cell<usize> = const { Cell::new(0) };
    }

    fn script(steps: &[Step]) {
        SCRIPT.with(|script| *script.borrow_mut() = steps.to_vec());
        NEXT.with(|next| next.set(0));
    }

    fn view(kernel: &str) -> GuestView {
        GuestView {
            vcpus: NonZeroU8::new(2).expect("two"),
            memory_mib: GuestMemoryMib::new(NonZeroU32::new(512).expect("512")),
            kernel: HostValue::Text(kernel.to_owned()),
            clocksource: HostValue::Text("kvm-clock".to_owned()),
            artifacts: GuestArtifacts {
                overdrive_init: HostValue::Text("a".repeat(64)),
                overdrive_guest_peer: HostValue::Text("b".repeat(64)),
                ovd_fwd: HostValue::Text("c".repeat(64)),
            },
        }
    }

    fn view_json(kernel: &str) -> serde_json::Value {
        serde_json::json!({
            "t": "guest_observed", "vcpus": 2, "memory_mib": 512, "kernel": kernel,
            "clocksource": "kvm-clock",
            "artifacts": {
                "overdrive_init": "a".repeat(64),
                "overdrive_guest_peer": "b".repeat(64),
                "ovd_fwd": "c".repeat(64)
            }
        })
    }

    /// The `guest` block of `run_started` for a guest benchmark (DDD-2).
    fn guest_block() -> serde_json::Value {
        let unseen = serde_json::json!({ "absent": "not visible from a guest" });
        serde_json::json!({
            "launch": "metal_host", "boot": "per_replicate", "vcpu_scheduling": "unpinned",
            "settings": { "governor": unseen, "smt": unseen, "boost": unseen }
        })
    }

    fn series_failure(reason: &str) -> ReplicateFailure {
        ReplicateFailure::fixture(reason)
    }

    /// The scripted family body: the next step of the script, one per replicate in run order.
    fn scripted(replicate: &mut Replicate) -> Result<(), ReplicateFailure> {
        let index = NEXT.with(|next| {
            let index = next.get();
            next.set(index + 1);
            index
        });
        let step = SCRIPT.with(|script| script.borrow()[index]);
        let values = [100.0, 200.0, 300.0];
        match step {
            Step::Report(kernel) => {
                replicate.report_guest_view(view(kernel))?;
                replicate.record_series("forwarded_round_trip_ns", &values)
            }
            Step::ReportThenFail(kernel, reason) => {
                replicate.report_guest_view(view(kernel))?;
                Err(series_failure(reason))
            }
            Step::Silent => replicate.record_series("forwarded_round_trip_ns", &values),
            Step::Fail(reason) => Err(series_failure(reason)),
        }
    }

    fn guest_decl(run: fn(&mut Replicate) -> Result<(), ReplicateFailure>) -> BenchmarkDecl {
        BenchmarkDecl {
            family: "unit-family",
            benchmark: "unit_bench",
            substrate: "guest",
            arms: &["forwarding"],
            replicates: 9,
            profiles: &[],
            metrics: &GUEST_METRICS,
            bpf_stats: false,
            references: &[],
            service_clock: None,
            guest_clock: None,
            run,
        }
    }

    fn lines_of(outcome: &Outcome, kind: &str) -> Vec<serde_json::Value> {
        outcome.sink.lines().into_iter().filter(|line| line["t"] == kind).collect()
    }

    /// The first replicate that reports a view makes the runner append one `guest_observed` line
    /// right before that replicate's line, after the start canary; `run_started` carries the
    /// static `guest` block and states substrate `guest`; the record validates.
    /// Boundary scenario (1): a conforming host and a prepared kit proceed to samples.
    #[test]
    fn the_first_reported_guest_view_is_written_once_before_that_replicates_line() {
        script(&[Step::Report("7.0.0-38-generic"); 9]);
        let (outcome, _dir) = run(&guest_decl(scripted), &Fixture::eligible());
        outcome.result.as_ref().expect("the run finishes");
        let lines = outcome.sink.lines();
        assert_eq!(
            lines[0]["substrate"], "guest",
            "run_started must state substrate guest for a guest benchmark"
        );
        assert_eq!(
            lines[0]["guest"],
            guest_block(),
            "run_started must carry the static guest block"
        );
        assert_eq!(
            (lines[1]["t"].as_str(), lines[1]["phase"].as_str()),
            (Some("canary"), Some("start"))
        );
        assert_eq!(
            lines[2],
            view_json("7.0.0-38-generic"),
            "the guest_observed line follows the start canary and holds the reported view"
        );
        assert_eq!(lines[3]["t"], "replicate", "the first replicate's line follows it");
        assert_eq!(
            lines_of(&outcome, "guest_observed").len(),
            1,
            "exactly one guest_observed line"
        );
        validate(&outcome.sink.record()).expect("the record validates");
    }

    /// A later replicate's view is not compared and changes nothing: one line, the first view.
    /// The operator fixes the kernel, the artifacts and the shape (ADR-0328).
    #[test]
    fn a_later_guest_view_writes_no_second_line_and_changes_nothing() {
        let mut steps = [Step::Report("7.0.0-38-generic"); 9];
        steps[1] = Step::Report("6.1.0-another");
        steps[4] = Step::Report("6.1.0-another");
        script(&steps);
        let (outcome, _dir) = run(&guest_decl(scripted), &Fixture::eligible());
        outcome.result.as_ref().expect("the run finishes");
        let observed = lines_of(&outcome, "guest_observed");
        assert_eq!(observed.len(), 1, "exactly one guest_observed line, later views write none");
        assert_eq!(observed[0], view_json("7.0.0-38-generic"), "the first reported view stays");
        assert!(
            lines_of(&outcome, "replicate").iter().all(|line| line["outcome"] == "ok"),
            "a later, different view fails nothing"
        );
        validate(&outcome.sink.record()).expect("the record validates");
    }

    /// M-1 (boundary scenario 8): a guest-substrate replicate whose body returns `Ok` without
    /// reporting a view is failed by the runner, and the record still validates; with no replicate
    /// ever reporting there is no `guest_observed` line.
    #[test]
    fn a_guest_replicate_that_returns_ok_without_a_view_is_failed_and_the_record_still_validates() {
        script(&[Step::Silent; 9]);
        let (outcome, _dir) = run(&guest_decl(scripted), &Fixture::eligible());
        outcome.result.as_ref().expect("the run finishes");
        let replicates = lines_of(&outcome, "replicate");
        assert_eq!(replicates.len(), 9);
        for line in &replicates {
            assert_eq!(
                (line["outcome"].as_str(), line["failure"].as_str()),
                (Some("failed"), Some("the replicate reported no guest view")),
                "a guest replicate that returned Ok without a view must be failed with the reported no guest view reason"
            );
        }
        assert!(lines_of(&outcome, "guest_observed").is_empty(), "no view, so no line");
        let finished = outcome.sink.lines().pop().expect("last line");
        assert_eq!(
            (finished["replicates_ok"].as_u64(), finished["replicates_failed"].as_u64()),
            (Some(0), Some(9))
        );
        validate(&outcome.sink.record())
            .expect("a guest record whose replicates all failed validates");
    }

    /// A view reported by a replicate that then fails stays the run's view; the replicate stays
    /// failed with its own reason (a failure after reporting is not "no view").
    #[test]
    fn a_view_reported_by_a_replicate_that_then_fails_stays_the_runs_view() {
        let mut steps = [Step::Report("6.1.0-another"); 9];
        steps[0] = Step::ReportThenFail("7.0.0-38-generic", "the in-guest program exited 3");
        script(&steps);
        let (outcome, _dir) = run(&guest_decl(scripted), &Fixture::eligible());
        outcome.result.as_ref().expect("the run finishes");
        let lines = outcome.sink.lines();
        let observed = lines_of(&outcome, "guest_observed");
        assert_eq!(observed.len(), 1, "exactly one guest_observed line");
        assert_eq!(
            observed[0],
            view_json("7.0.0-38-generic"),
            "the failed replicate's view is the run's view"
        );
        assert_eq!(lines[2]["t"], "guest_observed");
        assert_eq!(
            (lines[3]["outcome"].as_str(), lines[3]["failure"].as_str()),
            (Some("failed"), Some("the in-guest program exited 3")),
            "the replicate keeps its own failure"
        );
        validate(&outcome.sink.record()).expect("the record validates");
    }

    /// Boundary scenarios (2) and (5): a replicate whose guest never boots or dies is failed and
    /// counted, the run continues, and the `guest_observed` line is written right before the line
    /// of the first replicate that reports a view (after the failed one), preceding every `ok` line.
    #[test]
    fn a_replicate_that_fails_before_reporting_is_counted_and_later_replicates_still_run() {
        let mut steps = [Step::Report("7.0.0-38-generic"); 9];
        steps[0] = Step::Fail("guest boot: no READY within 60s");
        steps[5] = Step::Fail("guest run: no EXIT within the bound");
        script(&steps);
        let (outcome, _dir) = run(&guest_decl(scripted), &Fixture::eligible());
        outcome.result.as_ref().expect("the run finishes");
        let lines = outcome.sink.lines();
        assert_eq!(
            (lines[2]["t"].as_str(), lines[2]["failure"].as_str()),
            (Some("replicate"), Some("guest boot: no READY within 60s"))
        );
        assert_eq!(
            lines[3],
            view_json("7.0.0-38-generic"),
            "the guest_observed line comes right before the replicate that reported the view"
        );
        assert_eq!(lines[4]["outcome"], "ok");
        let finished = lines.last().expect("last line");
        assert_eq!(
            (finished["replicates_ok"].as_u64(), finished["replicates_failed"].as_u64()),
            (Some(7), Some(2)),
            "both failures are counted and the other replicates ran"
        );
        validate(&outcome.sink.record()).expect("the record validates");
    }

    /// G-1 and boundary scenario (6): with a drifted host the family body is never called and no
    /// guest boots; `run_started` still carries the static `guest` block, `run_refused` follows,
    /// no `guest_observed` line is written and the refused record validates.
    #[test]
    fn a_drifted_host_never_calls_a_guest_benchmarks_body_and_writes_no_view() {
        fn never_called(_: &mut Replicate) -> Result<(), ReplicateFailure> {
            panic!("a refused host must not call the family body");
        }
        let (outcome, _dir) = run(
            &guest_decl(never_called),
            &Fixture::eligible().with("/sys/devices/system/cpu/smt/control", "off\n"),
        );
        assert!(matches!(&outcome.result, Err(Error::Refused { setting, .. }) if setting == "smt"));
        let lines = outcome.sink.lines();
        assert_eq!(lines.len(), 2, "run_started and run_refused only");
        assert_eq!(
            lines[0]["guest"],
            guest_block(),
            "a refused guest benchmark's run_started must carry the static guest block"
        );
        assert_eq!(lines[1]["t"], "run_refused");
        assert!(lines_of(&outcome, "guest_observed").is_empty());
        assert_eq!(outcome.canary_samples, 0);
        validate(&outcome.sink.record()).expect("a refused guest record validates");
    }

    /// Feature disabled (boundary scenario 6): a native benchmark's record has no `guest` block and
    /// no `guest_observed` line, and a native replicate that tries to report a view is failed (a
    /// native record may not carry one).
    #[test]
    fn a_native_benchmark_writes_no_guest_block_and_refuses_a_reported_view() {
        fn reports(replicate: &mut Replicate) -> Result<(), ReplicateFailure> {
            replicate.report_guest_view(view("7.0.0-38-generic"))?;
            replicate.record_series("forwarded_round_trip_ns", &[100.0])
        }
        let mut decl = guest_decl(reports);
        decl.substrate = "native";
        let (outcome, _dir) = run(&decl, &Fixture::eligible());
        outcome.result.as_ref().expect("the run finishes");
        let lines = outcome.sink.lines();
        assert!(lines[0].get("guest").is_none(), "a native run_started has no guest block");
        assert_eq!(lines[0]["substrate"], "native");
        assert!(lines_of(&outcome, "guest_observed").is_empty());
        for line in lines_of(&outcome, "replicate") {
            assert_eq!(
                line["outcome"], "failed",
                "a native replicate that reports a guest view must fail"
            );
        }
        validate(&outcome.sink.record()).expect("the native record validates");
    }

    /// The relative path DDD-9 pins for a replicate's console.
    fn console_path(line: &serde_json::Value) -> PathBuf {
        PathBuf::from(format!(
            "artifacts/{}/{}-{}-{}/console.log",
            line["profile"].as_str().expect("profile"),
            line["block"],
            line["position"],
            line["arm"].as_str().expect("arm"),
        ))
    }

    fn retaining(replicate: &mut Replicate) -> Result<(), ReplicateFailure> {
        let index = NEXT.with(|next| {
            let index = next.get();
            next.set(index + 1);
            index
        });
        replicate.retain_console(&format!("console of replicate {index}"))?;
        replicate.report_guest_view(view("7.0.0-38-generic"))?;
        if index % 3 == 2 {
            return Err(series_failure("guest run: the program exited 2"));
        }
        replicate.record_series("forwarded_round_trip_ns", &[100.0, 200.0, 300.0])
    }

    /// DDD-9: every replicate's console is written, in every outcome, to
    /// `artifacts/<profile>/<block>-<position>-<arm>/console.log` under the result directory.
    #[test]
    fn a_replicates_console_is_retained_beside_the_record_in_every_outcome() {
        script(&[]);
        let (outcome, dir) = run(&guest_decl(retaining), &Fixture::eligible());
        outcome.result.as_ref().expect("the run finishes");
        let replicates = lines_of(&outcome, "replicate");
        let written = outcome.sink.artifacts.borrow();
        assert_eq!(
            written.len(),
            replicates.len(),
            "every replicate's console must be written as an artifact in every outcome"
        );
        let result_dir = dir.path().join("benchmarks/results/unit-family").join(RUN_ID);
        for (index, line) in replicates.iter().enumerate() {
            let relative = console_path(line);
            let found = written
                .iter()
                .find(|(_, path, _)| *path == relative)
                .unwrap_or_else(|| panic!("no artifact at {}", relative.display()));
            assert_eq!(found.0, result_dir, "artifacts are written under the result directory");
            assert_eq!(found.2, format!("console of replicate {index}").into_bytes());
        }
        assert!(
            replicates.iter().any(|line| line["outcome"] == "failed"),
            "the failed replicates' consoles are among them"
        );
    }

    /// A second `retain_console` by the same replicate fails it (create-new), and only the first
    /// console is written.
    #[test]
    fn retaining_a_console_twice_in_one_replicate_fails_it_and_keeps_the_first() {
        fn twice(replicate: &mut Replicate) -> Result<(), ReplicateFailure> {
            replicate.report_guest_view(view("7.0.0-38-generic"))?;
            replicate.retain_console("first")?;
            replicate.retain_console("second")?;
            replicate.record_series("forwarded_round_trip_ns", &[100.0])
        }
        let (outcome, _dir) = run(&guest_decl(twice), &Fixture::eligible());
        outcome.result.as_ref().expect("the run finishes");
        for line in lines_of(&outcome, "replicate") {
            assert_eq!(
                line["outcome"], "failed",
                "a second retain_console must fail the replicate"
            );
        }
        let written = outcome.sink.artifacts.borrow();
        assert_eq!(written.len(), 9, "the first console of each replicate is written");
        assert!(written.iter().all(|(_, _, bytes)| bytes == b"first"));
    }

    /// A console write that fails (a full disk) fails the replicate; the run continues.
    #[test]
    fn a_console_write_that_fails_fails_the_replicate_and_the_run_continues() {
        fn retains(replicate: &mut Replicate) -> Result<(), ReplicateFailure> {
            replicate.report_guest_view(view("7.0.0-38-generic"))?;
            replicate.retain_console("console")?;
            replicate.record_series("forwarded_round_trip_ns", &[100.0])
        }
        let decl = guest_decl(retains);
        let dir = marker_dir();
        let declared = std::slice::from_ref(&decl);
        let sink = MemorySink::default();
        sink.artifact_writes_fail.set(true);
        let mut canary = CountingCanary::default();
        let mut stdout = Vec::new();
        let environment = Environment {
            source_root: Some(dir.path().to_owned()),
            run_id: RUN_ID.to_owned(),
            seed: 42,
        };
        let arguments: Vec<String> = format!(
            "run --family unit-family --results-root {}/benchmarks/results --bench",
            dir.path().display()
        )
        .split_whitespace()
        .map(str::to_owned)
        .collect();
        let probe = Fixture::eligible();
        let enabler = Enabler(None);
        let result = Runner {
            evidence: Evidence::Maintained,
            debug_assertions: false,
            declared,
            probe: &probe,
            enabler: &enabler,
            sink: &sink,
            canary: &mut canary,
            stdout: &mut stdout,
        }
        .run(&arguments, &environment);
        result.expect("a failed console write does not end the run");
        let lines = sink.lines();
        let replicates: Vec<_> = lines.iter().filter(|line| line["t"] == "replicate").collect();
        assert_eq!(replicates.len(), 9, "every replicate still ran");
        for line in replicates {
            assert_eq!(line["outcome"], "failed", "a failed console write must fail the replicate");
        }
    }

    /// A body that fails and whose console write also fails keeps its own reason first and carries
    /// the write failure after it.
    #[test]
    fn a_failed_body_with_a_failed_console_write_reports_both_reasons() {
        fn retains(replicate: &mut Replicate) -> Result<(), ReplicateFailure> {
            replicate.report_guest_view(view("7.0.0-38-generic"))?;
            replicate.retain_console("console")?;
            Err(ReplicateFailure::measurement("the in-guest program exited 3"))
        }
        let decl = guest_decl(retains);
        let dir = marker_dir();
        let declared = std::slice::from_ref(&decl);
        let sink = MemorySink::default();
        sink.artifact_writes_fail.set(true);
        let mut canary = CountingCanary::default();
        let mut stdout = Vec::new();
        let environment = Environment {
            source_root: Some(dir.path().to_owned()),
            run_id: RUN_ID.to_owned(),
            seed: 42,
        };
        let arguments: Vec<String> = format!(
            "run --family unit-family --results-root {}/benchmarks/results --bench",
            dir.path().display()
        )
        .split_whitespace()
        .map(str::to_owned)
        .collect();
        let probe = Fixture::eligible();
        let enabler = Enabler(None);
        let result = Runner {
            evidence: Evidence::Maintained,
            debug_assertions: false,
            declared,
            probe: &probe,
            enabler: &enabler,
            sink: &sink,
            canary: &mut canary,
            stdout: &mut stdout,
        }
        .run(&arguments, &environment);
        result.expect("a failed console write does not end the run");
        let lines = sink.lines();
        let replicates: Vec<_> = lines.iter().filter(|line| line["t"] == "replicate").collect();
        assert_eq!(replicates.len(), 9, "every replicate still ran");
        for line in replicates {
            let reason = line["failure"].as_str().expect("a failed replicate carries its reason");
            assert!(
                reason.starts_with("the in-guest program exited 3; console not retained: "),
                "{reason}"
            );
        }
    }

    /// The directory-backed sink writes an artifact create-new, creating its parent directories;
    /// writing the same path again is `AlreadyExists` and leaves the first bytes.
    #[test]
    fn the_file_sink_writes_an_artifact_create_new_and_creates_its_parents() {
        let root = tempfile::tempdir().expect("tempdir");
        let dir = root.path().join("family").join(RUN_ID);
        drop(FileSink.create(&dir).expect("created"));
        let relative = Path::new("artifacts/default/0-0-forwarding/console.log");
        FileSink.write_artifact(&dir, relative, b"console").expect("the artifact is written");
        assert_eq!(
            std::fs::read(dir.join(relative)).ok().as_deref(),
            Some(b"console".as_slice()),
            "the file sink must write the artifact and its parents"
        );
        let again = FileSink.write_artifact(&dir, relative, b"other");
        assert_eq!(again.err().map(|error| error.kind()), Some(io::ErrorKind::AlreadyExists));
        assert_eq!(std::fs::read(dir.join(relative)).expect("read"), b"console");
    }

    fn step_strategy() -> impl Strategy<Value = Step> {
        prop_oneof![
            Just(Step::Report("7.0.0-38-generic")),
            Just(Step::Report("6.1.0-another")),
            Just(Step::ReportThenFail("7.0.0-38-generic", "guest run: the program exited 1")),
            Just(Step::ReportThenFail("6.1.0-another", "guest run: the program exited 1")),
            Just(Step::Silent),
            Just(Step::Fail("guest boot: no READY within 60s")),
        ]
    }

    fn reported_kernel(step: &Step) -> Option<&'static str> {
        match step {
            Step::Report(kernel) | Step::ReportThenFail(kernel, _) => Some(kernel),
            Step::Silent | Step::Fail(_) => None,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, failure_persistence: None, ..ProptestConfig::default() })]

        /// The seeded ordering invariant of the runner over a guest benchmark (G-2 evidence lane):
        /// whatever each replicate does, the record validates; there is at most one
        /// `guest_observed` line and it exists exactly when a replicate reported a view; it holds the
        /// first reported view and sits right before the first reporting replicate's line, so before
        /// every `ok` line; a replicate that returned `Ok` without a view is failed with the
        /// not-reported reason; the terminal line comes last.
        #[test]
        fn the_guest_record_ordering_holds_for_every_script_of_replicates(
            steps in proptest::collection::vec(step_strategy(), 9),
        ) {
            script(&steps);
            let (outcome, _dir) = run(&guest_decl(scripted), &Fixture::eligible());
            let lines = outcome.sink.lines();
            // Every breach is collected, so a failing script names all of them in one message.
            let mut violations: Vec<String> = Vec::new();
            if outcome.result.is_err() {
                violations.push("the run did not finish".to_owned());
            }
            if let Err(error) = validate(&outcome.sink.record()) {
                violations.push(format!("the record does not validate: {error}"));
            }
            let observed: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line["t"] == "guest_observed")
                .map(|(at, _)| at)
                .collect();
            let first_reporter = steps.iter().position(|step| reported_kernel(step).is_some());
            if observed.len() != usize::from(first_reporter.is_some()) {
                violations.push(format!(
                    "{} guest_observed lines, but a line exists exactly when a replicate reported a view",
                    observed.len()
                ));
            }
            let replicate_at: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line["t"] == "replicate")
                .map(|(at, _)| at)
                .collect();
            if let (Some(index), Some(&at)) = (first_reporter, observed.first()) {
                let kernel = reported_kernel(&steps[index]).expect("a reporter");
                if lines[at] != view_json(kernel) {
                    violations.push("the line is not the first reported view".to_owned());
                }
                if replicate_at.get(index) != Some(&(at + 1)) {
                    violations.push("the line is not right before the first reporting replicate".to_owned());
                }
            }
            for (&line_at, step) in replicate_at.iter().zip(&steps) {
                if matches!(step, Step::Silent)
                    && lines[line_at]["failure"].as_str() != Some("the replicate reported no guest view")
                {
                    violations.push("a replicate without a view was not failed for it".to_owned());
                }
            }
            if lines.last().map(|line| line["t"].as_str()) != Some(Some("run_finished")) {
                violations.push("the terminal line is not last".to_owned());
            }
            prop_assert!(
                violations.is_empty(),
                "guest record ordering invariant violated: {violations:?}"
            );
        }
    }
}
