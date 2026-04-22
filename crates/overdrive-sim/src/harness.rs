//! DST harness — composes a real `LocalStore` with every `Sim*` adapter
//! on each of three turmoil-style hosts, evaluates a catalogue of
//! invariants, and returns a structured [`RunReport`].
//!
//! # Phase 1 scope
//!
//! This step (06-01) ships the harness *composition* + CLI plumbing.
//! Each invariant evaluator returns a pass stub — step 06-02 fills in
//! the real bodies (e.g. the `SimObservationStore` LWW convergence check,
//! the entropy twin-run identity check). The contract surface — input
//! `seed`, output `RunReport` — is stable so that 06-02 can swap the
//! stubs for real evaluators without touching the xtask wiring or the
//! JSON summary schema.
//!
//! # What composition looks like
//!
//! Per the roadmap and `docs/product/architecture/brief.md` §7:
//!
//! * Three hosts, named `host-0`, `host-1`, `host-2` — the baseline
//!   three-peer DST cluster shape.
//! * Each host owns a `LocalStore` (real redb) on a per-host tempdir,
//!   plus one instance of every `Sim*` adapter.
//! * `Harness::run(seed)` iterates the invariant catalogue (or a
//!   filtered subset) and collects one [`InvariantResult`] per entry.
//!
//! `turmoil` itself is kept deliberately thin in Phase 1 — the full
//! `turmoil::Builder` scheduler lands in 06-02 together with the
//! invariants whose evaluators actually need a scheduled tick loop.
//! `Harness::run` here is synchronous so the xtask binary can call it
//! without imposing an async boundary on CI.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::id::{IdParseError, NodeId};
use overdrive_core::traits::driver::DriverType;
use overdrive_core::traits::intent_store::IntentStoreError;

use crate::adapters::clock::SimClock;
use crate::adapters::dataplane::SimDataplane;
use crate::adapters::driver::SimDriver;
use crate::adapters::entropy::SimEntropy;
use crate::adapters::llm::SimLlm;
use crate::adapters::observation_store::SimObservationStore;
use crate::adapters::transport::SimTransport;
use crate::invariants::Invariant;

/// Default number of hosts the harness boots when constructed via
/// `Harness::default()`.
pub const DEFAULT_HOST_COUNT: usize = 3;

/// A single tick budget placeholder — 06-02 refines this once the
/// turmoil-driven evaluators land.
const DEFAULT_TICK_BUDGET: u64 = 1_000;

/// Outcome of a single invariant evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvariantStatus {
    /// The invariant held for the entire run.
    Pass,
    /// The invariant was violated.
    Fail,
}

impl InvariantStatus {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
        }
    }
}

/// One invariant's result in a [`RunReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvariantResult {
    /// Canonical kebab-case name of the invariant (matches [`Invariant::as_canonical`]).
    pub name: String,
    /// Whether the invariant held.
    pub status: InvariantStatus,
    /// Simulated tick at which the evaluator concluded. Phase 1 stubs
    /// always report the end-of-run tick; 06-02 replaces this with the
    /// violating tick on failure.
    pub tick: u64,
    /// Host the evaluator ran against. Phase 1 stubs always report
    /// `host-0`; 06-02 refines per-invariant.
    pub host: String,
    /// Optional cause description on failure.
    pub cause: Option<String>,
}

/// A single reported failure, suitable for the JSON summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Invariant name that failed.
    pub invariant: String,
    /// Tick at which the failure was observed.
    pub tick: u64,
    /// Host the failure occurred on.
    pub host: String,
    /// Human-readable cause.
    pub cause: String,
}

/// Structured result of a harness run — the single value the xtask
/// binary renders to both the console and `dst-summary.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    /// The seed this run was parameterised by.
    pub seed: u64,
    /// Per-invariant results, in the order they were evaluated.
    pub invariants: Vec<InvariantResult>,
    /// Total wall-clock time the harness consumed.
    pub wall_clock: Duration,
    /// Failures, in the order they were observed (also represented in
    /// `invariants` via `status == Fail`).
    pub failures: Vec<Failure>,
}

impl RunReport {
    /// `true` iff every invariant passed.
    #[must_use]
    pub fn is_green(&self) -> bool {
        self.failures.is_empty()
    }
}

/// A single host in the harness cluster — owns one of each adapter and
/// one real `LocalStore` on a per-host tempdir.
///
/// Phase 1 instantiates the adapters to prove composition; the actual
/// invariant evaluators plug in at 06-02. We keep the adapters alive
/// via `_adapters` so that Drop does not run before the invariants
/// observe them.
struct Host {
    /// Host name used in summaries (e.g. `host-0`).
    name: String,
    /// Backing tempdir; dropped when the harness drops. We keep it on
    /// the host so that the `LocalStore`'s file handle remains valid
    /// across the whole run.
    _tempdir: tempfile::TempDir,
    /// Path of the redb file — captured for error reporting; the store
    /// itself is held behind an `Arc` so that 06-02 evaluators can share
    /// it across async tasks without moving the host.
    _store_path: PathBuf,
    /// Adapter bundle — constructed for composition; 06-02 consumes.
    #[allow(dead_code)]
    adapters: HostAdapters,
}

/// Every `Sim*` adapter composed on a single host. Phase 1 constructs
/// them to prove the harness wires every port; 06-02 consumes them in
/// the evaluator bodies.
#[allow(dead_code)]
struct HostAdapters {
    clock: Arc<SimClock>,
    transport: Arc<SimTransport>,
    entropy: Arc<SimEntropy>,
    dataplane: Arc<SimDataplane>,
    driver: Arc<SimDriver>,
    llm: Arc<SimLlm>,
    observation: Arc<SimObservationStore>,
}

/// Harness composition errors.
///
/// Invariant *evaluation* failures land in [`RunReport::failures`].
/// [`HarnessError`] is exclusively for "we could not even stand the
/// harness up" failures — tempdir refused, redb refused to open,
/// `NodeId::new` rejected a synthesised host id.
#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    /// Per-host tempdir creation failed.
    #[error("tempdir for host-{index} failed: {source}")]
    TempDir {
        /// Host index that failed.
        index: usize,
        /// Underlying IO error.
        #[source]
        source: std::io::Error,
    },
    /// Opening the real `LocalStore` failed.
    #[error("LocalStore open failed for host-{index}: {source}")]
    LocalStoreOpen {
        /// Host index that failed.
        index: usize,
        /// Underlying intent-store error.
        #[source]
        source: IntentStoreError,
    },
    /// Constructing the per-host `NodeId` failed. This is effectively
    /// unreachable (the synthesised "host-<i>" form is always valid)
    /// but we surface the error rather than panic so the DST harness
    /// never crashes a CI run with an unexplained panic.
    #[error("NodeId for host-{index} rejected: {source}")]
    NodeId {
        /// Host index that failed.
        index: usize,
        /// Underlying id-parse error.
        #[source]
        source: IdParseError,
    },
}

/// DST harness entry point. Construct via [`Harness::new`], run via
/// [`Harness::run`].
pub struct Harness {
    /// Invariants this run will evaluate. `None` means "the default
    /// catalogue" — constructed lazily in `run` to avoid pinning the
    /// order at construction time.
    only: Option<Invariant>,
}

impl Harness {
    /// Construct a default harness — runs the full invariant
    /// catalogue against three hosts.
    #[must_use]
    pub const fn new() -> Self {
        Self { only: None }
    }

    /// Narrow this run to a single invariant. The xtask `--only <NAME>`
    /// flag calls this after resolving the name via
    /// [`Invariant::from_str`].
    #[must_use]
    pub const fn only(mut self, invariant: Invariant) -> Self {
        self.only = Some(invariant);
        self
    }

    /// Run the harness against `seed`. Returns the full [`RunReport`];
    /// callers decide whether to fail based on `report.is_green()`.
    ///
    /// Errors only on harness-composition failure (e.g. redb refuses to
    /// open). Invariant failures are reported through [`RunReport`] so
    /// that the xtask binary can still write a summary JSON that names
    /// the failure.
    pub fn run(self, seed: u64) -> Result<RunReport, HarnessError> {
        let started = Instant::now();

        // Compose three hosts. We build them up-front so that
        // composition failures (e.g. redb refuses to open) show up
        // before any invariant evaluator runs.
        let hosts = Self::build_hosts(seed)?;

        // Evaluate the invariant subset we were configured with.
        let catalogue = self.catalogue();

        let mut invariants = Vec::with_capacity(catalogue.len());
        let mut failures = Vec::new();

        for invariant in catalogue {
            let result = Self::evaluate(invariant, seed, &hosts);
            if result.status == InvariantStatus::Fail {
                failures.push(Failure {
                    invariant: result.name.clone(),
                    tick: result.tick,
                    host: result.host.clone(),
                    cause: result
                        .cause
                        .clone()
                        .unwrap_or_else(|| "invariant evaluator reported failure".to_owned()),
                });
            }
            invariants.push(result);
        }

        Ok(RunReport { seed, invariants, wall_clock: started.elapsed(), failures })
    }

    fn catalogue(&self) -> Vec<Invariant> {
        self.only.map_or_else(|| Invariant::ALL.to_vec(), |only| vec![only])
    }

    fn build_hosts(seed: u64) -> Result<Vec<Host>, HarnessError> {
        (0..DEFAULT_HOST_COUNT).map(|i| Self::build_host(seed, i)).collect()
    }

    fn build_host(seed: u64, index: usize) -> Result<Host, HarnessError> {
        let tempdir = tempfile::Builder::new()
            .prefix(&format!("overdrive-dst-host-{index}-"))
            .tempdir()
            .map_err(|source| HarnessError::TempDir { index, source })?;

        let store_path = tempdir.path().join("intent.redb");

        // Real LocalStore on a per-host tempdir — proves the adapter
        // wiring works with real redb I/O. We drop the store at the end
        // of the function; 06-02 holds it in the HostAdapters struct
        // once evaluators consume it. For now the presence of a working
        // `open` on every host is what this step ships.
        let store = overdrive_store_local::LocalStore::open(&store_path)
            .map_err(|source| HarnessError::LocalStoreOpen { index, source })?;
        drop(store);

        // Per-host entropy — each host gets a deterministically-derived
        // seed so that a single global seed parameterises the whole
        // run but hosts do not share an RNG stream.
        let host_seed = seed.wrapping_add(index as u64);

        let node_id = NodeId::new(&format!("host-{index}"))
            .map_err(|source| HarnessError::NodeId { index, source })?;

        let adapters = HostAdapters {
            clock: Arc::new(SimClock::new()),
            transport: Arc::new(SimTransport::new()),
            entropy: Arc::new(SimEntropy::new(host_seed)),
            dataplane: Arc::new(SimDataplane::new()),
            driver: Arc::new(SimDriver::new(DriverType::Process)),
            llm: Arc::new(SimLlm::new(Vec::new())),
            observation: Arc::new(SimObservationStore::single_peer(node_id, host_seed)),
        };

        Ok(Host {
            name: format!("host-{index}"),
            _tempdir: tempdir,
            _store_path: store_path,
            adapters,
        })
    }

    /// Phase 1 evaluator: every invariant passes on the stub. 06-02
    /// replaces this with a per-invariant evaluator dispatch.
    fn evaluate(invariant: Invariant, _seed: u64, hosts: &[Host]) -> InvariantResult {
        let host = hosts.first().map_or_else(|| "host-0".to_owned(), |h| h.name.clone());
        // TODO(06-02): per-invariant evaluator bodies. Phase 1 stubs
        // return pass so that the CLI, artifact shape, and --only
        // filter are independently testable before the evaluators land.
        InvariantResult {
            name: invariant.to_string(),
            status: InvariantStatus::Pass,
            tick: DEFAULT_TICK_BUDGET,
            host,
            cause: None,
        }
    }
}

impl Default for Harness {
    fn default() -> Self {
        Self::new()
    }
}
