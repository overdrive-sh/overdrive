//! `ExitEventObservableOutcome` — fix-exit-observer-running-gate step
//! 01-05 (Solution 4) — DST invariant: every consumed `ExitEvent`
//! produces a visible outcome.
//!
//! **Eventually invariant**: for every `ExitEvent` the worker
//! `exit_observer::run_with_retry → handle_exit_event` consumes, at
//! least one of the following is produced before the harness stops
//! driving ticks:
//!
//! 1. **(a) Obs-row write** — an `AllocStatusRow` for the same
//!    `alloc_id` exists with `state ∈ {Failed, Terminated}`.
//! 2. **(b) Degraded `LifecycleEvent`** — a broadcast on the
//!    lifecycle bus for the same `alloc_id` carrying
//!    `TransitionReason::DriverInternalError` (the May-2 retry-
//!    exhaustion-degraded escalation path) OR a terminal `to`
//!    state (the canonical happy-path emission).
//! 3. **(c) Structured error log** — a `tracing::error!` event
//!    naming the `alloc_id` and the underlying error.
//!
//! With Solution 1' (oneshot-gated watcher emission) landed in steps
//! 01-02 and 01-03, this invariant does NOT fire under the canonical
//! flow — the producer-ordering happens-before edge prevents the
//! silent-drop race entirely. The invariant's load-bearing role is
//! preventing future regressions of B/C/D in the RCA tree (consumer
//! one-shot read silent verdict, missing DST invariant) through any
//! emission path that bypasses the gate.
//!
//! Closes the gap predecessor RCA
//! `fix-exit-observer-write-retry/deliver/rca.md:107-109` named and
//! `docs/evolution/2026-05-02-fix-exit-observer-write-retry.md:64`
//! left open.
//!
//! ## Scenarios
//!
//! Both scenarios drive the live `action_shim + exit_observer +
//! SimDriver + SimObservationStore` wiring end-to-end (the same
//! production code paths the integration tests at
//! `crates/overdrive-control-plane/tests/integration/workload_lifecycle/
//! exit_observer_running_gate.rs` exercise) and assert the three-
//! outcome disjunction at the end.
//!
//! 1. **Happy path** — `submit → Running → exit → Terminated`. The
//!    invariant holds via outcome (a) (the obs row reaches `Failed`
//!    or `Terminated`) AND outcome (b) (a `LifecycleEvent` with a
//!    terminal `to` state is broadcast).
//! 2. **May-2 degraded** — drive to `Running`, then inject a
//!    saturating run of non-retryable
//!    `ObservationStoreError::PermissionDenied` failures via
//!    `SimObservationStore::inject_write_failure`, then inject the
//!    exit. The exit-observer's `run_with_retry` exhausts retries on
//!    every post-exit `obs.write`; the `RetryOutcome::Failed` arm
//!    emits a degraded `LifecycleEvent` carrying
//!    `TransitionReason::DriverInternalError` AND fires a structured
//!    `tracing::error!` AND fires the Running-confirmed gate as a
//!    liveness rail. The invariant accepts outcome (b) (the degraded
//!    `LifecycleEvent` broadcast) — the load-bearing AC4 assertion.
//!
//! ## Detection scope
//!
//! The evaluator observes outcomes (a) and (b) directly via the
//! `ObservationStore` snapshot and the lifecycle-event broadcast. It
//! does NOT capture outcome (c) — the structured `tracing::error!`
//! site at `crates/overdrive-control-plane/src/worker/exit_observer.rs`
//! lines 271–278 is co-located with the degraded `LifecycleEvent`
//! emission immediately following it. Under current code, outcome
//! (c) is a strict structural superset of outcome (b)'s degraded
//! arm: the `RetryOutcome::Failed` arm emits BOTH the
//! `tracing::error!` AND the degraded `LifecycleEvent`. The
//! invariant's three-outcome OR is therefore SATISFIED whenever (b)
//! is observed; capturing (c) explicitly would be redundant under
//! the current emission shape and would couple the DST harness to
//! `tracing` global-subscriber state, which is hostile to multi-
//! evaluator dispatch under `cargo dst`. The contract documented
//! above remains the canonical AC1 disjunction; (c) is a permissive
//! third arm that future emission paths may take in lieu of (a) or
//! (b) — the evaluator's failure message names all three outcomes
//! so a future regression that relies solely on (c) is properly
//! attributed.
//!
//! ## Wiring
//!
//! The invariant lives at the simulation-harness layer because
//! `overdrive-sim` already depends on `overdrive-control-plane`
//! (for `SimViewStore` per ADR-0035 §3) — the dep graph for this
//! crate already permits driving the production action-shim +
//! exit-observer wiring against `Sim*` adapters. No additional dep
//! introduced.

#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::api::AllocStateWire;
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::worker::exit_observer;
use overdrive_control_plane::{AppState, noop_heartbeat, workload_lifecycle};
use overdrive_core::TransitionReason;
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput,
};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::driver::{Driver, DriverType, ExitKind};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, ObservationStore, ObservationStoreError,
};
use tempfile::TempDir;

use crate::adapters::ca::SimCa;
use crate::adapters::clock::SimClock;
use crate::adapters::dataplane::SimDataplane;
use crate::adapters::driver::SimDriver;
use crate::adapters::entropy::SimEntropy;
use crate::adapters::observation_store::SimObservationStore;
use crate::harness::{InvariantResult, InvariantStatus};
use overdrive_store_local::LocalIntentStore;

/// Drive both scenarios and return an `InvariantResult` pinned to the
/// canonical kebab-case name.
///
/// The two scenarios are run sequentially against fresh harnesses so
/// no fixture state leaks. Each scenario:
///
/// 1. Builds an end-to-end harness identical in shape to the
///    `exit_observer_running_gate.rs` integration tests.
/// 2. Drives convergence ticks until either an outcome is observed or
///    the per-scenario tick budget is exhausted.
/// 3. Inspects the captured event log for the three-outcome
///    disjunction.
///
/// A single missing outcome on either scenario is a load-bearing
/// failure: it means a future emission path produced a consumed
/// `ExitEvent` with no visible outcome (the very silent-drop class
/// the RCA's Solution 4 was written to defend).
pub async fn evaluate_exit_event_observable_outcome() -> InvariantResult {
    const NAME: &str = "exit-event-observable-outcome";

    if let Err(cause) =
        drive(Scenario::HappyPath, "happy-path-submit-running-exit-terminated").await
    {
        return fail(NAME, cause);
    }

    if let Err(cause) =
        drive(Scenario::DegradedEscalation, "degraded-escalation-via-inject-write-failure").await
    {
        return fail(NAME, cause);
    }

    pass(NAME)
}

/// Sub-scenario tag controlling fixture wiring.
#[derive(Debug, Clone, Copy)]
enum Scenario {
    /// Happy path: `submit → Running → exit → Terminated`. Outcome
    /// (a) and outcome (b) are both produced.
    HappyPath,
    /// May-2 degraded: drive to Running, inject saturating run of
    /// `PermissionDenied` failures, then inject the exit. Outcome
    /// (b) is produced via the degraded `LifecycleEvent`.
    DegradedEscalation,
}

/// Captured outcomes per allocation id.
#[derive(Default)]
struct OutcomeReport {
    /// Outcome (a): obs row with state ∈ {Failed, Terminated}.
    obs_terminal: bool,
    /// Outcome (b): lifecycle event with terminal `to` AND/OR with
    /// `TransitionReason::DriverInternalError` (degraded path).
    lifecycle_terminal_or_degraded: bool,
}

impl OutcomeReport {
    const fn satisfied(&self) -> bool {
        self.obs_terminal || self.lifecycle_terminal_or_degraded
    }
}

/// Drive a single scenario and return Ok if every `ExitEvent`
/// consumed by the observer produced at least one of (a)/(b)/(c).
async fn drive(scenario: Scenario, label: &str) -> Result<(), String> {
    let tmp = TempDir::new().map_err(|e| format!("scenario `{label}`: tempdir: {e}"))?;
    let h = build_harness(&tmp).await.map_err(|e| format!("scenario `{label}`: {e}"))?;

    let workload_lifecycle_name = ReconcilerName::new("job-lifecycle")
        .map_err(|e| format!("scenario `{label}`: reconciler name: {e}"))?;
    let mut events = h.state.lifecycle_events.subscribe();
    let start = Instant::now();
    let deadline = start + Duration::from_secs(120);

    let mut report = OutcomeReport::default();

    match scenario {
        Scenario::HappyPath => {
            drive_happy_path(
                &h,
                &workload_lifecycle_name,
                start,
                deadline,
                &mut events,
                &mut report,
            )
            .await
            .map_err(|e| format!("scenario `{label}`: {e}"))?;
        }
        Scenario::DegradedEscalation => {
            drive_degraded_escalation(
                &h,
                &workload_lifecycle_name,
                start,
                deadline,
                &mut events,
                &mut report,
            )
            .await
            .map_err(|e| format!("scenario `{label}`: {e}"))?;
        }
    }

    if !report.satisfied() {
        return Err(format!(
            "scenario `{label}`: alloc {alloc} consumed an ExitEvent but produced no visible \
             outcome: obs_terminal={a}, lifecycle_terminal_or_degraded={b}. Per RCA Solution 4, \
             every ExitEvent the exit_observer consumes MUST produce at least one of (a) \
             AllocStatusRow with state ∈ {{Failed, Terminated}}, (b) degraded LifecycleEvent \
             carrying TransitionReason::DriverInternalError or a terminal `to` state, or (c) \
             structured tracing::error! naming the alloc_id. Outcomes (a) and (b) are checked \
             directly here; outcome (c) is documented at the trait surface as a permissive third \
             arm — see the module rustdoc.",
            alloc = h.alloc_id,
            a = report.obs_terminal,
            b = report.lifecycle_terminal_or_degraded,
        ));
    }

    Ok(())
}

/// Drive the happy-path scenario: inject a sub-budget exit BEFORE
/// convergence runs, then drive ticks until the obs row reaches a
/// terminal state and a terminal `LifecycleEvent` is broadcast.
///
/// With Solution 1' the watcher parks on the gate; the obs row
/// reaches Terminated/Failed and a terminal `LifecycleEvent` is
/// broadcast. Outcomes (a) AND (b) are produced.
async fn drive_happy_path(
    h: &Harness,
    workload_lifecycle_name: &ReconcilerName,
    start: Instant,
    deadline: Instant,
    events: &mut tokio::sync::broadcast::Receiver<
        overdrive_control_plane::action_shim::LifecycleEvent,
    >,
    report: &mut OutcomeReport,
) -> Result<(), String> {
    h.sim_driver.inject_exit_after(
        &h.alloc_id,
        Duration::from_micros(1),
        ExitKind::Crashed { exit_code: Some(1), signal: None },
    );

    for tick_n in 0_u64..60 {
        run_convergence_tick(
            &h.state,
            workload_lifecycle_name,
            &h.target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .map_err(|e| format!("tick {tick_n}: {e:?}"))?;
        drain_events(events, &h.alloc_id, report);
        yield_a_few().await;
        if report.obs_terminal && report.lifecycle_terminal_or_degraded {
            break;
        }
    }

    check_obs_terminal(h, report).await
}

/// Drive the May-2 degraded-escalation scenario: drive to Running,
/// inject a saturating run of non-retryable `PermissionDenied`
/// failures, then inject the exit. The exit-observer's
/// `run_with_retry` exhausts retries on every post-exit `obs.write`;
/// the `RetryOutcome::Failed` arm emits a degraded `LifecycleEvent`
/// carrying `TransitionReason::DriverInternalError`. Outcome (b) is
/// produced; the invariant accepts (b) alone — the load-bearing AC4
/// assertion.
async fn drive_degraded_escalation(
    h: &Harness,
    workload_lifecycle_name: &ReconcilerName,
    start: Instant,
    deadline: Instant,
    events: &mut tokio::sync::broadcast::Receiver<
        overdrive_control_plane::action_shim::LifecycleEvent,
    >,
    report: &mut OutcomeReport,
) -> Result<(), String> {
    let tick_n = drive_to_running(h, workload_lifecycle_name, start, deadline).await?;

    // Drain pre-degraded events so the assertion sees the
    // degraded-path event specifically.
    while events.try_recv().is_ok() {}

    // Inject saturating run of non-retryable failures so the
    // post-exit obs.write exhausts retries.
    for _ in 0..16 {
        h.sim_obs.inject_write_failure(ObservationStoreError::Io(io::Error::from(
            io::ErrorKind::PermissionDenied,
        )));
    }

    h.sim_driver.inject_exit_after(
        &h.alloc_id,
        Duration::from_millis(500),
        ExitKind::Crashed { exit_code: Some(1), signal: None },
    );

    for tick in tick_n..(tick_n + 60) {
        run_convergence_tick(
            &h.state,
            workload_lifecycle_name,
            &h.target,
            start + Duration::from_millis(tick.saturating_mul(100)),
            tick,
            deadline,
        )
        .await
        .map_err(|e| format!("tick {tick}: {e:?}"))?;
        drain_events(events, &h.alloc_id, report);
        yield_a_few().await;
        if report.lifecycle_terminal_or_degraded {
            break;
        }
    }
    // The degraded path explicitly avoids writing the obs row (the
    // May-2 contract — the obs store is rejecting writes outright),
    // so outcome (a) is structurally absent here. The invariant must
    // accept outcome (b) alone — that's the load-bearing AC4
    // assertion.
    Ok(())
}

/// Drive convergence ticks until the alloc reaches `Running`. Returns
/// the next tick number (callers continue from this value). Errors if
/// `Running` is not reached within the budget.
async fn drive_to_running(
    h: &Harness,
    workload_lifecycle_name: &ReconcilerName,
    start: Instant,
    deadline: Instant,
) -> Result<u64, String> {
    let mut tick_n: u64 = 0;
    let mut reached_running = false;
    while tick_n < 30 && !reached_running {
        run_convergence_tick(
            &h.state,
            workload_lifecycle_name,
            &h.target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .map_err(|e| format!("tick {tick_n}: {e:?}"))?;
        let rows =
            h.state.obs.alloc_status_rows().await.map_err(|e| format!("read rows: {e:?}"))?;
        reached_running =
            rows.iter().any(|r| r.alloc_id == h.alloc_id && matches!(r.state, AllocState::Running));
        tick_n += 1;
    }
    if reached_running {
        Ok(tick_n)
    } else {
        Err("alloc must reach Running before injecting exit + write failures (the post-Running \
             gate fire is required to unpark the watcher)"
            .to_owned())
    }
}

/// Drain pending `LifecycleEvent`s and update the outcome report.
fn drain_events(
    events: &mut tokio::sync::broadcast::Receiver<
        overdrive_control_plane::action_shim::LifecycleEvent,
    >,
    alloc_id: &AllocationId,
    report: &mut OutcomeReport,
) {
    while let Ok(ev) = events.try_recv() {
        if &ev.alloc_id != alloc_id {
            continue;
        }
        let terminal_to = matches!(ev.to, AllocStateWire::Failed | AllocStateWire::Terminated);
        let degraded_reason = matches!(ev.reason, TransitionReason::DriverInternalError { .. });
        if terminal_to || degraded_reason {
            report.lifecycle_terminal_or_degraded = true;
        }
    }
}

/// Snapshot the obs store and update the outcome report.
async fn check_obs_terminal(h: &Harness, report: &mut OutcomeReport) -> Result<(), String> {
    let rows = h.state.obs.alloc_status_rows().await.map_err(|e| format!("read rows: {e:?}"))?;
    if rows.iter().any(|r| {
        r.alloc_id == h.alloc_id && matches!(r.state, AllocState::Failed | AllocState::Terminated)
    }) {
        report.obs_terminal = true;
    }
    Ok(())
}

/// Yield a few times to give the spawned `inject_exit_after` task and
/// the observer task chances to run before the next tick. Mirrors the
/// convention in `exit_observer_running_gate.rs`.
async fn yield_a_few() {
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
}

// ---------------------------------------------------------------------------
// Harness — mirrors `exit_observer_running_gate.rs::build_harness` so the
// surface under test is identical.
// ---------------------------------------------------------------------------

struct Harness {
    state: AppState,
    sim_obs: Arc<SimObservationStore>,
    sim_driver: Arc<SimDriver>,
    target: TargetResource,
    alloc_id: AllocationId,
    #[allow(dead_code)]
    sim_clock: Arc<SimClock>,
    /// Background ticker task. Aborted on drop so the tokio runtime
    /// can shut down cleanly between scenarios.
    ticker_handle: tokio::task::JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.ticker_handle.abort();
    }
}

async fn build_harness(tmp: &TempDir) -> Result<Harness, String> {
    let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path())
        .map_err(|e| format!("runtime: {e:?}"))?;
    runtime.register(noop_heartbeat()).await.map_err(|e| format!("register noop: {e:?}"))?;
    runtime
        .register(workload_lifecycle())
        .await
        .map_err(|e| format!("register job-lifecycle: {e:?}"))?;

    let store = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb"))
            .map_err(|e| format!("open store: {e:?}"))?,
    );
    let node_id = NodeId::new("local").map_err(|e| format!("node id: {e:?}"))?;
    let sim_obs = Arc::new(SimObservationStore::single_peer(node_id.clone(), 0));
    let obs: Arc<dyn ObservationStore> = sim_obs.clone();
    let sim_clock = Arc::new(SimClock::new());
    let sim_driver = Arc::new(SimDriver::with_clock(DriverType::Exec, sim_clock.clone()));
    let driver: Arc<dyn Driver> = sim_driver.clone();

    let allocator = overdrive_control_plane::test_default_allocator(
        Arc::clone(&store) as Arc<dyn overdrive_core::traits::intent_store::IntentStore>
    );
    let state = AppState::new(
        store,
        tmp.path().join("intent.redb"),
        obs,
        Arc::new(runtime),
        driver,
        sim_clock.clone(),
        Arc::new(SimDataplane::new()),
        Arc::new(SimCa::new(Arc::new(SimEntropy::new(0)))),
        Arc::new(IdentityMgr::new(None)),
        NodeId::new("writer-1").map_err(|e| format!("writer node id: {e:?}"))?,
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    );

    exit_observer::spawn(
        state.obs.clone(),
        state.driver.clone(),
        state.lifecycle_events.clone(),
        sim_clock.clone(),
    );

    let job = Job::from_submit(JobSpecInput {
        id: "exit-event-observable-outcome".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/sleep".to_string(),
            args: vec!["3600".to_string()],
        }),
    })
    .map_err(|e| format!("valid job spec: {e:?}"))?;
    // Per ADR-0050 single-cut migration: wrap into the kind-agnostic
    // `WorkloadIntent` aggregate before archival; persist at
    // `workloads/<id>` keying.
    let intent = overdrive_core::aggregate::WorkloadIntent::Job(job.clone());
    let archived = intent.archive_for_store().map_err(|e| format!("rkyv archive: {e:?}"))?;
    let key = IntentKey::for_workload(&job.id);
    state
        .store
        .put(key.as_bytes(), archived.as_ref())
        .await
        .map_err(|e| format!("put job: {e:?}"))?;

    let target = TargetResource::new("job/exit-event-observable-outcome")
        .map_err(|e| format!("valid target: {e:?}"))?;
    let alloc_id = AllocationId::new("alloc-exit-event-observable-outcome-0")
        .map_err(|e| format!("alloc id: {e:?}"))?;

    // Background ticker — same convention as `exit_observer_running_gate.rs`.
    let ticker_clock = sim_clock.clone();
    let ticker_handle = tokio::spawn(async move {
        loop {
            ticker_clock.tick(Duration::from_millis(50));
            tokio::task::yield_now().await;
        }
    });

    Ok(Harness { state, sim_obs, sim_driver, target, alloc_id, sim_clock, ticker_handle })
}

// ---------------------------------------------------------------------------
// Helpers — pin the canonical name + the host string used in
// `InvariantResult` to mirror sibling invariants in this catalogue.
// ---------------------------------------------------------------------------

fn pass(name: &str) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Pass,
        tick: 1,
        host: cluster_host(),
        cause: None,
    }
}

fn fail(name: &str, cause: String) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Fail,
        tick: 1,
        host: cluster_host(),
        cause: Some(cause),
    }
}

fn cluster_host() -> String {
    NodeId::new("cluster").map_or_else(|_| "cluster".to_owned(), |id| id.to_string())
}
