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
    BpfStatsMode, Canary, CanaryPhase, Clock, ClockSource, Declaration, EvidenceClass, Host, Line,
    METHODOLOGY_VERSION, MetricDecl, Outcome, ProfileDecl, RecordWriter, ReferenceDecl,
    ReplicateLine, RunFinished, RunRefused, RunStarted, SCHEMA_VERSION, Substrate, Unit,
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
        Error::StatsEnable { .. } | Error::Record { .. } | Error::InvalidRecord { .. } => 70,
    }
}

/// The one stderr line that explains `error`.
fn diagnostic(error: &Error) -> String {
    match error {
        Error::Refused { setting, found, pinned } => {
            format!("bench: refused: {setting}: found {found}, pinned {pinned}")
        }
        Error::Unreadable { setting, source } => format!("bench: refused: {setting}: {source}"),
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

        self.canary_samples(&mut record, CanaryPhase::Start, &dir)?;
        let counts = execute(decl, environment.seed, hold.as_ref(), &mut record)?;
        self.canary_samples(&mut record, CanaryPhase::End, &dir)?;
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
            .map_err(|source| Error::Record { path: path.clone(), source })?;
        Ok(RecordWriter::new(out, path))
    }

    fn canary_samples(
        &mut self,
        record: &mut RecordWriter<Box<dyn Write>>,
        phase: CanaryPhase,
        dir: &Path,
    ) -> Result<()> {
        let mut samples_ns = Vec::with_capacity(CANARY_SAMPLES);
        for _ in 0..CANARY_SAMPLES {
            let sample = self.canary.sample().map_err(|source| Error::Record {
                path: dir.join(RECORD_FILE),
                source: io::Error::new(
                    source.kind(),
                    format!("the loopback canary failed: {source}"),
                ),
            })?;
            samples_ns.push(sample);
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
            substrate: Substrate::Native,
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

/// Run every profile's schedule, appending one `replicate` line per run.
fn execute(
    decl: &BenchmarkDecl,
    seed: u64,
    hold: Option<&Arc<BpfStatsHold>>,
    record: &mut RecordWriter<Box<dyn Write>>,
) -> Result<Counts> {
    let mut counts = Counts { ok: 0, failed: 0 };
    let profiles: Vec<&str> = if decl.profiles.is_empty() {
        vec![IMPLICIT_PROFILE]
    } else {
        decl.profiles.iter().map(|profile| profile.name).collect()
    };
    for (index, profile) in (0_u32..).zip(profiles) {
        for planned in plan(decl.arms, decl.replicates, index, seed) {
            let line = run_one(decl, profile, &planned, hold);
            match line.outcome {
                Outcome::Ok => counts.ok += 1,
                Outcome::Failed => counts.failed += 1,
            }
            record.append(&Line::Replicate(Box::new(line)))?;
        }
    }
    Ok(counts)
}

/// One replicate: a fresh handle, the body once, and what it recorded.
fn run_one(
    decl: &BenchmarkDecl,
    profile: &str,
    planned: &crate::schedule::Planned,
    hold: Option<&Arc<BpfStatsHold>>,
) -> ReplicateLine {
    if let Some(hold) = hold {
        // Reads of an earlier replicate are not this one's evidence of effectiveness.
        drop(hold.take_reads());
    }
    let mut replicate = Replicate::new(decl, &planned.arm, profile, hold.cloned());
    let body = (decl.run)(&mut replicate);
    let finished = replicate.finish();
    let ineffective = hold.and_then(|hold| ineffective_program(&hold.take_reads()));
    let failure = match body {
        Err(failure) => Some(failure.to_string()),
        Ok(()) => finished.failure.or_else(|| {
            ineffective.map(|id| ReplicateFailure::stats_not_effective(id).to_string())
        }),
    };
    ReplicateLine {
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
    }
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
            outcome.result.expect("the run finishes");
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
        outcome.result.expect("the run finishes");
        assert_eq!(outcome.sink.lines()[0]["bpf_stats"], "per_process");
    }

    #[test]
    fn a_failed_replicate_is_counted_and_the_run_continues() {
        let decl = decl(&["fine", "broken", "typo"], false);
        let (outcome, _dir) = run(&decl, &Fixture::eligible());
        outcome.result.expect("the run finishes");
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
        outcome.result.expect("the run finishes");
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
}
