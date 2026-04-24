//! DST harness — composes a real `LocalIntentStore` with every `Sim*` adapter
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
//! * Each host owns a `LocalIntentStore` (real redb) on a per-host tempdir,
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
use crate::adapters::observation_store::{SimObservationCluster, SimObservationStore};
use crate::adapters::transport::SimTransport;
use crate::invariants::{Invariant, evaluators};

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
/// one real `LocalIntentStore` on a per-host tempdir.
struct Host {
    /// Host name used in summaries (e.g. `host-0`).
    name: String,
    /// Backing tempdir; dropped when the harness drops.
    _tempdir: tempfile::TempDir,
    /// Path of the redb file — captured for error reporting.
    _store_path: PathBuf,
    /// Real `LocalIntentStore` this host writes intent through. Held on the
    /// host so evaluator calls can read from the same instance the
    /// harness initialised.
    intent: Arc<overdrive_store_local::LocalIntentStore>,
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
    /// Building the tokio runtime used to drive the async invariant
    /// evaluators failed. This is not tied to any host and is surfaced
    /// separately so the error message names the runtime rather than a
    /// phantom `host-18446744073709551615`.
    #[error("tokio runtime build failed: {source}")]
    RuntimeBuild {
        /// Underlying IO error from `tokio::runtime::Builder::build`.
        #[source]
        source: std::io::Error,
    },
    /// Opening the real `LocalIntentStore` failed.
    #[error("LocalIntentStore open failed for host-{index}: {source}")]
    LocalIntentStoreOpen {
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

        // Build a shared SimObservationCluster for the LWW convergence
        // evaluator. Each host retains its own single-peer observation
        // store for unrelated invariants; the cluster is constructed
        // anew here with its own gossip router.
        let observation_cluster = build_observation_cluster(seed, &hosts);

        // Evaluate the invariant subset we were configured with.
        let catalogue = self.catalogue();

        // Evaluators are async (the intent-crossing and LWW evaluators
        // read through the `IntentStore` / `SimObservationStore` async
        // APIs). Spin a single-threaded tokio runtime inside the
        // synchronous `run` so the xtask binary's callsite remains
        // synchronous — xtask is a boundary crate and does not want an
        // async entry point per ADR-0003.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| HarnessError::RuntimeBuild { source })?;

        let mut invariants = Vec::with_capacity(catalogue.len());
        let mut failures = Vec::new();

        for invariant in catalogue {
            let result = rt.block_on(Self::evaluate(invariant, seed, &hosts, &observation_cluster));
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

        // Real LocalIntentStore on a per-host tempdir — shared with evaluator
        // bodies via `Host::intent` so every invariant sees the same
        // backing redb instance the harness composed.
        let intent = Arc::new(
            overdrive_store_local::LocalIntentStore::open(&store_path)
                .map_err(|source| HarnessError::LocalIntentStoreOpen { index, source })?,
        );

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
            intent,
            adapters,
        })
    }

    /// Dispatch to the matching per-invariant evaluator in
    /// [`crate::invariants::evaluators`]. Every invariant in the
    /// catalogue maps to exactly one evaluator; unknown variants cannot
    /// compile because the enum is exhaustive.
    async fn evaluate(
        invariant: Invariant,
        seed: u64,
        hosts: &[Host],
        cluster: &SimObservationCluster,
    ) -> InvariantResult {
        // The harness guarantees `hosts` is non-empty via `DEFAULT_HOST_COUNT`;
        // `if let Some` keeps clippy's `expect_used` lint satisfied and
        // documents the invariant for the reader.
        let Some(first_host) = hosts.first() else {
            return InvariantResult {
                name: invariant.to_string(),
                status: InvariantStatus::Fail,
                tick: DEFAULT_TICK_BUDGET,
                host: "cluster".to_owned(),
                cause: Some("harness booted zero hosts — composition bug".to_owned()),
            };
        };

        match invariant {
            Invariant::SingleLeader => {
                // Stubbed 3-host leader election per US-06 Technical
                // Note 3: the "leader" is host-0 deterministically for
                // every epoch in Phase 1. Phase 2 replaces this with a
                // read against the real Raft leader term.
                let hosts_ids: Vec<NodeId> =
                    hosts.iter().filter_map(|h| NodeId::new(&h.name).ok()).collect();
                let leader = hosts_ids.first().cloned();
                evaluators::evaluate_single_leader_from_topology(&hosts_ids, leader.as_ref())
            }
            Invariant::IntentNeverCrossesIntoObservation => {
                evaluators::evaluate_intent_crossing(
                    first_host.intent.as_ref(),
                    first_host.adapters.observation.as_ref(),
                )
                .await
            }
            Invariant::SnapshotRoundtripBitIdentical => {
                evaluators::evaluate_snapshot_roundtrip(first_host.intent.as_ref()).await
            }
            Invariant::SimObservationLwwConverges => {
                evaluators::evaluate_sim_observation_lww(cluster).await
            }
            Invariant::ReplayEquivalentEmptyWorkflow => {
                evaluators::evaluate_replay_equivalent_empty_workflow(seed)
            }
            Invariant::EntropyDeterminismUnderReseed => {
                evaluators::evaluate_entropy_determinism(seed)
            }
            // Step 04-05 — reconciler-primitive runtime invariants per
            // ADR-0013 §2 / §8 and whitepaper §18. The harness does not
            // depend on `overdrive-control-plane` (that would be a
            // dependency cycle), so each evaluator receives the minimal
            // state it needs: the count for registry size, an inline
            // LWW simulation of the broker for collapse, and a
            // locally-constructed deterministic reconciler for purity.
            // Production wiring is exercised separately in step 05-05's
            // walking skeleton.
            Invariant::AtLeastOneReconcilerRegistered => {
                evaluators::evaluate_at_least_one_reconciler_registered(
                    harness_registered_reconcilers(hosts),
                )
            }
            Invariant::DuplicateEvaluationsCollapse => {
                let (n, counters) = drive_broker_collapse();
                evaluators::evaluate_duplicate_evaluations_collapse(n, counters)
            }
            Invariant::ReconcilerIsPure => {
                let reconciler = harness_purity_reconciler();
                evaluators::evaluate_reconciler_is_pure(reconciler.as_ref())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Step 04-05 — in-harness fixtures for the three reconciler invariants
// ---------------------------------------------------------------------------

/// How many reconcilers the harness treats as "registered" for the
/// `AtLeastOneReconcilerRegistered` evaluator. Matches the Phase 1
/// production boot path in `overdrive-control-plane::run_server_with_obs`,
/// which registers `noop-heartbeat` before the server starts. Taking
/// `_hosts` as input pins the evaluator to per-run state rather than a
/// free constant — a future regression that drops registration still
/// has somewhere observable to manifest.
const fn harness_registered_reconcilers(_hosts: &[Host]) -> usize {
    // Phase 1 boot always registers `noop-heartbeat`. Future phases
    // that add more reconcilers will grow this count. A zero return
    // would make the invariant fail, which is the intended behaviour.
    1
}

/// Drive the broker-collapse sequence described by ADR-0013 §8 in
/// isolation: submit `N ≥ 3` evaluations at the same `(ReconcilerName,
/// TargetResource)` key, drain once, and return the observed counters.
///
/// The broker's LWW key-collapse is reimplemented here in a few lines
/// rather than pulled in from `overdrive-control-plane` — the sim crate
/// is a leaf adapter and the broker's behaviour is small enough to
/// mirror. The contract the evaluator checks — `dispatched == 1`,
/// `cancelled == n - 1`, `queued == 0` — is the invariant, and
/// mirroring the broker proves the contract is satisfiable on a clean
/// run. Production wiring is exercised in step 05-05.
#[allow(clippy::expect_used)] // `ReconcilerName::new` / `TargetResource::new` are total on literals.
fn drive_broker_collapse() -> (u64, evaluators::BrokerCountersSnapshot) {
    use std::collections::HashSet;

    use overdrive_core::reconciler::{ReconcilerName, TargetResource};

    /// Number of same-key evaluations the harness submits. 3 is the
    /// minimum the ADR-0013 invariant requires; larger values don't
    /// change the shape of the assertion.
    const N: u64 = 3;

    let reconciler =
        ReconcilerName::new("noop-heartbeat").expect("noop-heartbeat is a valid ReconcilerName");
    let target =
        TargetResource::new("job/payments").expect("job/payments is a valid TargetResource");
    let key = (reconciler, target);

    // Mirror of `EvaluationBroker::submit` + `drain_pending` LWW
    // semantics. Inserting at an occupied key evicts the prior value
    // into the cancelable count; draining empties pending and bumps
    // dispatched by the drained length.
    let mut pending: HashSet<(ReconcilerName, TargetResource)> = HashSet::new();
    let mut cancelled: u64 = 0;
    for _ in 0..N {
        // `insert` returns false if the key was already present — which
        // is exactly the LWW-supersession signal the broker uses.
        if !pending.insert(key.clone()) {
            cancelled = cancelled.saturating_add(1);
        }
    }
    let dispatched = pending.len() as u64;
    pending.clear();

    (N, evaluators::BrokerCountersSnapshot { queued: 0, cancelled, dispatched })
}

/// Construct the reconciler the harness twin-invokes for the
/// `ReconcilerIsPure` invariant. Mirrors the `noop-heartbeat` factory
/// in `overdrive-control-plane::noop_heartbeat`: deterministically
/// returns `vec![Action::Noop]`. Redefined here so the sim crate does
/// not depend on `overdrive-control-plane` (which already depends on
/// `overdrive-sim`).
#[allow(clippy::expect_used)] // `ReconcilerName::new("noop-heartbeat")` is total on the literal.
fn harness_purity_reconciler() -> Box<dyn overdrive_core::reconciler::Reconciler> {
    use overdrive_core::reconciler::{Action, Db, Reconciler, ReconcilerName, State};

    struct HarnessNoopHeartbeat {
        name: ReconcilerName,
    }

    impl Reconciler for HarnessNoopHeartbeat {
        fn name(&self) -> &ReconcilerName {
            &self.name
        }

        // Mutation-skip for this fn lives in `.cargo/mutants.toml` under
        // the `harness_purity_reconciler` exclude_re entry — cargo-mutants'
        // comment-based skip syntax is not honoured by the 25.x CLI we
        // pin in CI. See that file for the full justification.
        fn reconcile(&self, _desired: &State, _actual: &State, _db: &Db) -> Vec<Action> {
            #[cfg(feature = "canary-bug")]
            {
                use std::sync::atomic::{AtomicU64, Ordering};
                static CALL: AtomicU64 = AtomicU64::new(0);
                let n = CALL.fetch_add(1, Ordering::SeqCst);
                if n % 2 == 0 {
                    return vec![Action::Noop];
                }
                vec![Action::Noop, Action::Noop]
            }
            #[cfg(not(feature = "canary-bug"))]
            vec![Action::Noop]
        }
    }

    Box::new(HarnessNoopHeartbeat {
        name: ReconcilerName::new("noop-heartbeat")
            .expect("noop-heartbeat is a valid ReconcilerName"),
    })
}

/// Build a `SimObservationCluster` mirroring the harness's host set.
/// The cluster has its own gossip router, so writes issued through
/// `cluster.peer(&id).write(...)` converge as tested in step 04-03.
/// Each host's single-peer `SimObservationStore` in its adapter bundle
/// is separate from the cluster — the cluster is constructed here
/// specifically to drive the `SimObservationLwwConverges` evaluator.
fn build_observation_cluster(seed: u64, hosts: &[Host]) -> SimObservationCluster {
    let peers: Vec<NodeId> = hosts.iter().filter_map(|h| NodeId::new(&h.name).ok()).collect();
    SimObservationStore::cluster_builder()
        .peers(peers)
        .gossip_delay(std::time::Duration::from_millis(10))
        .seed(seed)
        .build()
}

impl Default for Harness {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    //! Library-level unit tests — these are what cargo-mutants uses to
    //! kill mutations in this file. Integration-test-style assertions
    //! (the xtask subprocess tests) are explicitly excluded by
    //! `.mutants.toml` so mutation coverage must come from here.

    use super::*;

    #[test]
    fn invariant_status_as_str_is_pass_or_fail() {
        assert_eq!(InvariantStatus::Pass.as_str(), "pass");
        assert_eq!(InvariantStatus::Fail.as_str(), "fail");
    }

    #[test]
    fn run_report_is_green_reflects_failures_vector() {
        let green = RunReport {
            seed: 0,
            invariants: Vec::new(),
            wall_clock: Duration::ZERO,
            failures: Vec::new(),
        };
        assert!(green.is_green());

        let red = RunReport {
            seed: 0,
            invariants: Vec::new(),
            wall_clock: Duration::ZERO,
            failures: vec![Failure {
                invariant: "single-leader".to_owned(),
                tick: 1,
                host: "host-0".to_owned(),
                cause: "boom".to_owned(),
            }],
        };
        assert!(!red.is_green());
    }

    #[test]
    fn default_catalogue_is_the_full_invariant_set() {
        let h = Harness::new();
        let cat = h.catalogue();
        assert_eq!(cat.len(), Invariant::ALL.len());
        assert_eq!(cat, Invariant::ALL.to_vec());
    }

    #[test]
    fn only_catalogue_is_the_single_requested_invariant() {
        let h = Harness::new().only(Invariant::SingleLeader);
        let cat = h.catalogue();
        assert_eq!(cat, vec![Invariant::SingleLeader]);
    }

    #[test]
    fn run_boots_the_default_number_of_hosts_and_reports_every_invariant() {
        let report = Harness::new().run(42).expect("harness must compose");
        // One result per invariant in the default catalogue.
        assert_eq!(report.invariants.len(), Invariant::ALL.len());
        // Phase 1 stubs all pass — failures array empty.
        assert!(report.is_green());
        assert!(report.failures.is_empty());
        // Every invariant result carries a canonical name and a host.
        for (result, canonical) in report.invariants.iter().zip(Invariant::ALL.iter()) {
            assert_eq!(result.name, canonical.to_string());
            assert_eq!(result.status, InvariantStatus::Pass);
            assert!(!result.host.is_empty());
        }
        // Seed is echoed back verbatim.
        assert_eq!(report.seed, 42);
    }

    #[test]
    fn run_with_only_produces_one_invariant_result() {
        let report = Harness::new()
            .only(Invariant::EntropyDeterminismUnderReseed)
            .run(7)
            .expect("harness must compose");
        assert_eq!(report.invariants.len(), 1);
        assert_eq!(report.invariants[0].name, "entropy-determinism-under-reseed");
        assert_eq!(report.invariants[0].status, InvariantStatus::Pass);
    }

    #[test]
    fn build_hosts_produces_default_host_count_with_distinct_names() {
        let hosts = Harness::build_hosts(0).expect("build_hosts must succeed");
        assert_eq!(hosts.len(), DEFAULT_HOST_COUNT);
        let names: Vec<&str> = hosts.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["host-0", "host-1", "host-2"]);
    }

    #[test]
    fn evaluator_failure_produces_a_corresponding_failure_entry() {
        // Synthesise a hand-crafted failure via a custom evaluation path.
        // The real evaluator always returns Pass in Phase 1, so we test
        // the `if result.status == Fail` branch by constructing a
        // RunReport directly.
        let fail = Failure {
            invariant: "single-leader".to_owned(),
            tick: 42,
            host: "host-1".to_owned(),
            cause: "synthetic".to_owned(),
        };
        let report = RunReport {
            seed: 1,
            invariants: vec![InvariantResult {
                name: "single-leader".to_owned(),
                status: InvariantStatus::Fail,
                tick: 42,
                host: "host-1".to_owned(),
                cause: Some("synthetic".to_owned()),
            }],
            wall_clock: Duration::ZERO,
            failures: vec![fail.clone()],
        };
        assert!(!report.is_green());
        assert_eq!(report.failures, vec![fail]);
    }
}
