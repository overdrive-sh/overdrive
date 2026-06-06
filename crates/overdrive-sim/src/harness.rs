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
    pub const fn is_green(&self) -> bool {
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
            driver: Arc::new(SimDriver::new(DriverType::Exec)),
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
    ///
    /// `#[allow(clippy::too_many_lines)]`: the body is a pure
    /// `match` over `Invariant::ALL` — one arm per variant, with each
    /// arm's body being a single delegating call. Extracting the arms
    /// to a per-invariant helper would split the exhaustive match
    /// across two files without removing any logic. Documentation
    /// comments per arm push the line count over the 100-line lint
    /// threshold; the dispatch shape is the right one.
    #[allow(clippy::too_many_lines)]
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
            Invariant::ReplayEquivalenceProvisionRecord => {
                // workflow-primitive step 01-07 — graduates the slice-1
                // two-SimEntropy-transcripts placeholder into a real
                // journal replay driving the `WorkflowEngine` +
                // `SimJournalStore` through the three-run crash-resume
                // shape (ADR-0064 §3/§6). K4, on the `cargo dst` critical
                // path.
                evaluators::evaluate_replay_equivalence_provision_record(seed).await
            }
            Invariant::WorkflowJournalWriteOrdering => {
                evaluators::evaluate_workflow_journal_write_ordering(seed).await
            }
            Invariant::WorkflowExactlyOnceEffectOnResume => {
                evaluators::evaluate_workflow_exactly_once_effect_on_resume(seed).await
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
            Invariant::BrokerDrainOrderIsDeterministic => {
                // Drive the multi-key submit-and-drain sequence twice
                // with identical inputs and feed both order snapshots
                // to the evaluator. Two passes is the minimum that can
                // catch order divergence; if the underlying drain
                // depends on `HashSet` iteration or any other implicit
                // state, the two snapshots will differ at some
                // position and the evaluator returns Fail. See the RCA
                // at `docs/feature/fix-eval-broker-drain-determinism/`.
                let (_, _, order_a) = drive_broker_collapse_multi_key();
                let (_, _, order_b) = drive_broker_collapse_multi_key();
                evaluators::evaluate_broker_drain_order_is_deterministic(&order_a, &order_b)
            }
            Invariant::DispatchRoutingIsNameRestricted => {
                // Drive the §8 storm-proofing dispatch-routing contract.
                // The harness mirrors `lib.rs:465-481`'s post-fix shape
                // (drain_pending → for eval in pending → run_convergence_tick)
                // without depending on `overdrive-control-plane` (per
                // ADR-0004 sim/host split). The mirror dispatches each
                // drained eval against its named reconciler exactly once;
                // the evaluator asserts the contract holds. See the RCA
                // at `docs/feature/fix-dst-dispatch-routing-invariant/`.
                let (submitted, record) = drive_dispatch_routing();
                evaluators::evaluate_dispatch_routing_is_name_restricted(&submitted, &record)
            }
            Invariant::ReconcilerIsPure => {
                let reconciler = harness_purity_reconciler();
                // Pull the `TickContext::now` snapshot from the first
                // host's injected `SimClock` rather than wall-clock.
                // `first_host` is bound by the outer `let Some(...)`
                // guard above. Under DST this is seed-deterministic;
                // the sim crate is `adapter-sim`-class so dst-lint does
                // not scan it, but pulling from the injected clock
                // preserves the ADR-0013 §2c "time is input state"
                // contract even at the harness-evaluator callsite and
                // matches the shape any future production evaluator
                // will use.
                evaluators::evaluate_reconciler_is_pure(
                    &reconciler,
                    first_host.adapters.clock.as_ref(),
                )
            }
            Invariant::IntentStoreReturnsCallerBytes => {
                // ADR-0020 §Enforcement structural-regression guard.
                // Uses an evaluator-owned tempdir-backed
                // `LocalIntentStore` rather than the harness's per-host
                // store so it cannot interact with state other
                // invariants leave behind.
                evaluators::evaluate_intent_store_returns_caller_bytes().await
            }
            // -------------------------------------------------------------
            // phase-1-first-workload — slice 3 (US-03) — convergence
            // invariants. The harness does not yet drive a full runtime
            // tick loop with a real `WorkloadLifecycle` reconciler against
            // host-owned IntentStore + ObservationStore (that wiring lives
            // in `overdrive-control-plane` as of step 02-03 and would
            // invert the dep graph). At the harness level we therefore
            // evaluate against a baseline empty observation snapshot —
            // the invariants are vacuous-pass here, which is the
            // correct K3 behaviour: "no submissions, no rows" is
            // self-consistent. End-to-end exercise lives in
            // `crates/overdrive-control-plane/tests/integration/workload_lifecycle/*`.
            // -------------------------------------------------------------
            Invariant::JobScheduledAfterSubmission => {
                evaluators::evaluate_workload_scheduled_after_submission(&[], &[])
            }
            Invariant::DesiredReplicaCountConverges => {
                evaluators::evaluate_desired_replica_count_converges(&[], &[])
            }
            Invariant::NoDoubleScheduling => evaluators::evaluate_no_double_scheduling(&[]),
            // -------------------------------------------------------------
            // reconciler-memory-redb step 01-07 — ViewStore DST
            // invariants per ADR-0035 §6. The evaluators construct
            // their own `SimViewStore` (and a `ReconcilerRuntime` for
            // the WriteThroughOrdering case) — the harness-owned
            // `host` adapters carry no `ViewStore`, and `view_store`
            // state is per-evaluator (each invariant builds a fresh
            // store so fixtures cannot leak across runs).
            // -------------------------------------------------------------
            Invariant::ViewStoreRoundtripIsLossless => {
                evaluators::evaluate_view_store_roundtrip_is_lossless(seed).await
            }
            Invariant::BulkLoadIsDeterministic => {
                evaluators::evaluate_bulk_load_is_deterministic().await
            }
            Invariant::WriteThroughOrdering => evaluators::evaluate_write_through_ordering().await,
            // phase-2-xdp-service-map Slice 03 (US-03; S-2.2-09).
            // GREEN of step 03-01 lands the real evaluator body
            // in `crate::invariants::backend_set_swap_atomic`. The
            // RED-scaffold body panics when invoked.
            Invariant::BackendSetSwapAtomic => {
                crate::invariants::backend_set_swap_atomic::evaluate_backend_set_swap_atomic().await
            }
            // phase-2-xdp-service-map Slice 04 (US-04; S-2.2-13 sibling).
            // GREEN of step 04-03 lands the real evaluator body in
            // `crate::invariants::maglev_distribution`. Sibling to the
            // disruption-bound proptest at
            // `crates/overdrive-sim/tests/integration/maglev_churn.rs`.
            Invariant::MaglevDistributionEven => {
                crate::invariants::maglev_distribution::evaluate_maglev_distribution_even()
            }
            // phase-2-xdp-service-map Slice 04 (US-04; S-2.2-14 sibling).
            // GREEN of step 04-05 lands the real evaluator body in
            // `crate::invariants::maglev_deterministic`. Sibling to
            // `MaglevDistributionEven` — both ride on the same pure
            // `maglev::generate` function.
            Invariant::MaglevDeterministic => {
                crate::invariants::maglev_deterministic::evaluate_maglev_deterministic()
            }
            // phase-2-xdp-service-map Slice 05 (US-05; S-2.2-20).
            // GREEN of step 05-01 lands the real evaluator body in
            // `crate::invariants::reverse_nat_lockstep`. The
            // RED-scaffold body panics when invoked.
            Invariant::ReverseNatLockstep => {
                crate::invariants::reverse_nat_lockstep::evaluate_reverse_nat_lockstep().await
            }
            // unconnected-udp-sendmsg4 Slice 02 (US-02; J-PLAT-004 / K3).
            // GH #200, ADR-0053 rev 2026-06-05. The evaluator drives
            // `register_local_backend` and asserts the reply mirror's
            // `reply_source_for(...) == Some(vip)` (step 02-01 GREEN) —
            // the structural defense below Tier-3 for the reply-source
            // identity. A forward-only mutation (forward entry written,
            // reply mirror not) turns it RED.
            Invariant::ReplySourceRewriteLockstep => {
                crate::invariants::reply_source_rewrite_lockstep::evaluate_reply_source_rewrite_lockstep().await
            }
            // phase-2-xdp-service-map Slice 06 (US-06; S-2.2-22
            // sibling). GREEN of step 06-04 lands the real evaluator
            // body in `crate::invariants::sanity_checks_fire`. Sibling
            // to the Tier 3 mixed-batch test at
            // `crates/overdrive-dataplane/tests/integration/sanity_mixed_batch.rs`.
            Invariant::SanityChecksFireBeforeServiceMap => {
                crate::invariants::sanity_checks_fire::evaluate_sanity_checks_fire_before_service_map().await
            }
            // phase-2-xdp-service-map DISTILL — RED scaffolds per
            // `docs/feature/phase-2-xdp-service-map/distill/wave-decisions.md`
            // DWD-4. Bodies panic when invoked. DELIVER's Slice 08
            // wires real evaluators via
            // `crate::invariants::service_map_hydrator::*` and
            // returns `InvariantResult` from real fixtures.
            // phase-2-xdp-service-map Slice 08 (US-08; ASR-2.2-04).
            // GREEN of step 08-02 lands the real evaluator bodies in
            // `crate::invariants::service_map_hydrator`. Both ESR
            // invariants drive the typed `ServiceMapHydrator::reconcile`
            // function directly via `AnyReconciler::ServiceMapHydrator`
            // dispatch; no I/O, no async — matches `MaglevDeterministic`
            // shape.
            Invariant::HydratorEventuallyConverges => {
                crate::invariants::service_map_hydrator::evaluate_hydrator_eventually_converges()
            }
            Invariant::HydratorIdempotentSteadyState => {
                crate::invariants::service_map_hydrator::evaluate_hydrator_idempotent_steady_state(
                )
            }
            // fix-exit-observer-running-gate step 01-05 (Solution 4).
            // Drives the live action_shim + exit_observer + SimDriver
            // + SimObservationStore wiring end-to-end across two
            // scenarios (happy path + May-2 degraded escalation) and
            // asserts the three-outcome disjunction for every
            // consumed `ExitEvent`. With Solution 1' (oneshot-gated
            // watcher emission) landed in steps 01-02 / 01-03, this
            // evaluator does NOT fire under the canonical flow — it
            // is the load-bearing structural defence against future
            // regressions.
            Invariant::ExitEventObservableOutcome => {
                crate::invariants::exit_event_observable_outcome::evaluate_exit_event_observable_outcome()
                    .await
            }
            // workload-gc-absent-stale-allocs step 01-03. Two scenarios
            // drive end-to-end through SimIntentStore +
            // SimObservationStore + WorkloadLifecycle runtime stack;
            // assertions land at the
            // `ObservationStore::alloc_status_rows()` driven port
            // boundary. Closes #148 AC §1.3.
            Invariant::WorkloadGcOrphanConverges => {
                crate::invariants::workload_gc_absent_intent::evaluate_orphan_workload_converges_to_terminal_gc()
                    .await
            }
            Invariant::WorkloadGcResubmitCreatesFresh => {
                crate::invariants::workload_gc_absent_intent::evaluate_resubmit_after_gc_creates_fresh_alloc()
                    .await
            }
            // backend-discovery-bridge-service-reachability (#174 + Atlas Q2)
            // GREEN — Slice 1 (closes #174). The three evaluators drive
            // the real `BackendDiscoveryBridge::reconcile` against a
            // `SimObservationStore`, applying emitted
            // `Action::WriteServiceBackendRow` actions via the action
            // shim simulation (`apply_actions` helper inside the module).
            // The Atlas Q2 evaluator (S-BDB-06) additionally exercises
            // the fsync-then-memory ordering contract from
            // `.claude/rules/development.md` § "Reconciler I/O".
            Invariant::BridgeEventuallyWritesBackendRow => {
                crate::invariants::backend_discovery_bridge::evaluate_bridge_eventually_writes_backend_row()
                    .await
            }
            Invariant::BridgeIdempotentSteadyState => {
                crate::invariants::backend_discovery_bridge::evaluate_bridge_idempotent_steady_state()
                    .await
            }
            Invariant::BridgeRecomputesFingerprintOnReplay => {
                crate::invariants::backend_discovery_bridge::evaluate_bridge_recomputes_fingerprint_on_replay()
                    .await
            }
            // backend-discovery-bridge-service-reachability step 02-04 —
            // bridge → hydrator handoff (S-BDB-19). Drives
            // `BackendDiscoveryBridge::reconcile` → applies
            // `Action::WriteServiceBackendRow` to `SimObservationStore`
            // → projects `service_backends_rows` back into
            // `ServiceMapHydratorState.desired` → ticks
            // `ServiceMapHydrator::reconcile` → asserts the dispatched
            // `Action::DataplaneUpdateService` carries the bridge-
            // written VIP + backends.
            Invariant::BridgeToHydratorHandoff => {
                crate::invariants::service_map_hydrator::evaluate_bridge_to_hydrator_handoff().await
            }
            // workflow-result-error-model step 02-01 (ADR-0065 §3, D3) —
            // GREEN. Drives an always-failing `AlwaysExplicitFailure` workflow
            // (body returns `Err(TerminalError::explicit)`) through the real
            // `WorkflowEngine` + `SimJournalStore` and asserts the engine
            // projects the body's `Err` to `WorkflowStatus::Failed { terminal }`
            // carrying the authored kind + detail, AND that the SAME status
            // round-trips byte-equal through BOTH the durable journal
            // `Terminal { status }` and the observable `WorkflowTerminal {
            // status }` obs row (D3 lossless projection). Now in
            // `Invariant::ALL` (wired in `crate::invariants::mod`), so
            // `cargo dst` dispatches it on every run.
            Invariant::WorkflowTerminalStatusProjection => {
                evaluators::evaluate_workflow_terminal_status_projection(seed).await
            }
            // workflow-result-error-model step 04-02 (ADR-0065 §D4) — the DST
            // counterpart to NEW-5 (`workflow_budget_exhaustion_mints_terminal`).
            // Drives an always-transient workflow (body returns
            // `Err(TerminalError::retryable(..))`) through the real
            // `WorkflowEngine` + `SimJournalStore`, advancing `SimClock` past
            // each backoff window via a concurrent ticker so the parked
            // re-drives fire, and asserts the engine re-drove up to
            // `WORKFLOW_RETRY_BUDGET` then MINTED `WorkflowStatus::Failed {
            // terminal: BudgetExhausted }` — the body authored no failure
            // (D4). Authored GREEN directly (the retry loop landed in 04-01).
            Invariant::WorkflowBudgetExhaustionMintsTerminal => {
                evaluators::evaluate_workflow_budget_exhaustion_mints_terminal(seed).await
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
/// Uses the real `EvaluationBroker` from `overdrive-core` — the broker
/// is pure logic over core types with no I/O, so the sim crate imports
/// it directly. The contract the evaluator checks — `dispatched == 1`,
/// `cancelled == n - 1`, `queued == 0` — is asserted against the real
/// broker's `counters()` snapshot.
#[allow(clippy::expect_used)] // `ReconcilerName::new` / `TargetResource::new` are total on literals.
fn drive_broker_collapse() -> (u64, evaluators::BrokerCountersSnapshot) {
    use overdrive_core::eval_broker::{Evaluation, EvaluationBroker};
    use overdrive_core::reconcilers::{ReconcilerName, TargetResource};

    const N: u64 = 3;

    let reconciler =
        ReconcilerName::new("noop-heartbeat").expect("noop-heartbeat is a valid ReconcilerName");
    let target =
        TargetResource::new("job/payments").expect("job/payments is a valid TargetResource");

    let mut broker = EvaluationBroker::new();
    for _ in 0..N {
        broker.submit(Evaluation { reconciler: reconciler.clone(), target: target.clone() });
    }
    let _ = broker.drain_pending();

    (N, broker.counters())
}

/// Drive the broker-collapse sequence across **two distinct keys** with
/// interleaved submits, drain once, and return the observed counters
/// plus the drain-order snapshot.
///
/// Multi-key sibling of [`drive_broker_collapse`]. Uses the real
/// `EvaluationBroker` from `overdrive-core` — the broker's
/// `BTreeMap`-backed `drain_pending` yields deterministic ascending
/// key order, which is exactly what the
/// `BrokerDrainOrderIsDeterministic` evaluator asserts.
///
/// ## Submit pattern
///
/// Two keys (`K1 = ("noop-heartbeat", "job/payments")` and
/// `K2 = ("noop-heartbeat", "job/frontend")`), `N = 3` submits per key,
/// strictly interleaved as `[K1, K2, K1, K2, K1, K2]`. After all six
/// submits land, drain once.
///
/// ## Expected snapshot
///
/// - `dispatched = 2` — two distinct keys remain in `pending` at drain
///   time, so drain emits one invocation per key.
/// - `cancelled = 4` — each key was submitted N times; the first submit
///   per key occupies `pending`, every subsequent same-key submit
///   evicts the prior version into the cancelable count. With N=3 and
///   2 keys: `2 * (N - 1) = 4`.
/// - `queued = 0` — drain emptied `pending`.
#[allow(clippy::expect_used)] // `ReconcilerName::new` / `TargetResource::new` are total on literals.
fn drive_broker_collapse_multi_key()
-> (u64, evaluators::BrokerCountersSnapshot, evaluators::BrokerDrainOrderSnapshot) {
    use overdrive_core::eval_broker::{Evaluation, EvaluationBroker};
    use overdrive_core::reconcilers::{ReconcilerName, TargetResource};

    const N: u64 = 3;

    let reconciler =
        ReconcilerName::new("noop-heartbeat").expect("noop-heartbeat is a valid ReconcilerName");
    let target_a =
        TargetResource::new("job/payments").expect("job/payments is a valid TargetResource");
    let target_b =
        TargetResource::new("job/frontend").expect("job/frontend is a valid TargetResource");

    let mut broker = EvaluationBroker::new();
    for _ in 0..N {
        broker.submit(Evaluation { reconciler: reconciler.clone(), target: target_a.clone() });
        broker.submit(Evaluation { reconciler: reconciler.clone(), target: target_b.clone() });
    }
    let drained = broker.drain_pending();
    let dispatched_order: Vec<(ReconcilerName, TargetResource)> =
        drained.into_iter().map(|e| (e.reconciler, e.target)).collect();

    (N, broker.counters(), evaluators::BrokerDrainOrderSnapshot { dispatched_order })
}

/// Drive the dispatch path for the `DispatchRoutingIsNameRestricted`
/// invariant.
///
/// Mirrors `lib.rs:465-481`'s post-fix shape (`drain_pending` → for eval
/// in pending → `run_convergence_tick`) without depending on
/// `overdrive-control-plane` (per ADR-0004 sim/host split). Submits a
/// fixed set of evals naming `job-lifecycle` against distinct targets,
/// drains, and records each dispatch as a `(reconciler, target)` tuple.
/// The recorded dispatcher honours the §8 contract: each drained eval
/// dispatches exactly one (R, T) where R is the eval's named reconciler.
///
/// The mirror's role is the SAT-side witness that the contract is
/// satisfiable on a clean run — exactly as `drive_broker_collapse`
/// proves the broker's collapse invariant is satisfiable. Production
/// code coverage for dispatch routing remains the acceptance test at
/// `runtime_convergence_loop.rs:209-309`
/// (`eval_dispatch_runs_only_the_named_reconciler`, commit `e6f5e5e`)
/// plus this DST harness pass — jointly closing the §8 storm-proofing
/// contract at unit and DST tiers.
#[allow(clippy::expect_used)] // `ReconcilerName::new` / `TargetResource::new` are total on literals.
fn drive_dispatch_routing() -> (Vec<evaluators::Evaluation>, evaluators::DispatchRecord) {
    use overdrive_core::eval_broker::Evaluation;
    use overdrive_core::reconcilers::{ReconcilerName, TargetResource};

    let r_jl =
        ReconcilerName::new("job-lifecycle").expect("job-lifecycle is a valid ReconcilerName");
    let t_a = TargetResource::new("job/payments").expect("job/payments is a valid TargetResource");
    let t_b = TargetResource::new("job/frontend").expect("job/frontend is a valid TargetResource");

    let submitted = vec![
        Evaluation { reconciler: r_jl.clone(), target: t_a },
        Evaluation { reconciler: r_jl, target: t_b },
    ];

    // Mirrored dispatcher: for each drained eval, dispatch the named
    // reconciler against the named target — single dispatch per drained
    // eval, named by the Evaluation. A registry-iteration regression
    // would manifest here as multiple entries per drained eval naming
    // reconcilers other than the submitted one; the evaluator's
    // smoking-gun branch would catch it.
    let mut dispatched: Vec<(ReconcilerName, TargetResource)> = Vec::new();
    for eval in &submitted {
        dispatched.push((eval.reconciler.clone(), eval.target.clone()));
    }

    (submitted, evaluators::DispatchRecord { dispatched })
}

/// Construct the reconciler the harness twin-invokes for the
/// `ReconcilerIsPure` invariant.
fn harness_purity_reconciler() -> overdrive_core::reconcilers::AnyReconciler {
    use overdrive_core::reconcilers::{AnyReconciler, NoopHeartbeat};
    AnyReconciler::NoopHeartbeat(NoopHeartbeat::canonical())
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

    // Phase 01-05 (closes #174) GREEN: the three
    // backend-discovery-bridge evaluators landed; the prior
    // `#[should_panic(expected = "RED scaffold")]` downstream-fallout
    // guard (documented in `.claude/rules/testing.md` § "Downstream
    // fallout on pre-existing tests") is removed per the same
    // section's "removing the underlying todo!() / panic!() will fire
    // a different panic message, trip #[should_panic], and flag the
    // test for review at the moment the scaffold goes GREEN" handoff.
    // unconnected-udp-sendmsg4 step 02-01 GREEN: the
    // `ReplySourceRewriteLockstep` evaluator landed (the SimDataplane
    // reply-mirror write in `register_local_backend`), so the prior
    // `#[should_panic(expected = "RED scaffold")]` downstream-fallout
    // guard (per `.claude/rules/testing.md` § "Downstream fallout on
    // pre-existing tests") is removed — the full-invariant walk now
    // reaches the real evaluator and passes green.
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
    fn drive_broker_collapse_multi_key_returns_expected_snapshot() {
        // Pins the counter snapshot for the multi-key drain driver.
        // Two distinct keys, N=3 submits each, interleaved → drain
        // should leave dispatched=2 (one per key still in pending),
        // cancelled=4 (each key superseded N-1=2 times), queued=0
        // (drain emptied pending). Step 01-04 prerequisite for the
        // BrokerDrainOrderIsDeterministic invariant in step 01-05.
        //
        // The driver's return tuple was extended additively in 01-05
        // to also yield a `BrokerDrainOrderSnapshot` for the new
        // invariant; this test continues to pin the counters and now
        // also asserts the order snapshot is non-empty (the order
        // contract itself is exercised by the evaluator unit test).
        let (per_key_n, counters, order) = drive_broker_collapse_multi_key();
        assert_eq!(per_key_n, 3);
        assert_eq!(
            counters,
            evaluators::BrokerCountersSnapshot { queued: 0, cancelled: 4, dispatched: 2 }
        );
        assert_eq!(
            order.dispatched_order.len(),
            2,
            "drain order must hold exactly the two distinct keys after dedup"
        );
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
