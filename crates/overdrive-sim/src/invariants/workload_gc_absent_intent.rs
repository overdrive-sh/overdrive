//! workload-gc-absent-stale-allocs step 01-03 — DST scenarios for
//! the absent-intent workload GC arm AND the resubmit-after-GC race.
//!
//! Two scenarios drive end-to-end through the live `SimIntentStore +
//! SimObservationStore + WorkloadLifecycle` runtime stack via the
//! public `submit` (intent put) / `tick` (run_convergence_tick)
//! harness driving ports. Assertions land at the
//! `ObservationStore::alloc_status_rows()` driven port boundary.
//! No reconciler internals are exercised directly.
//!
//! ## Scenario 1 — `orphan_workload_converges_to_terminal_gc`
//!
//! 1. Submit `Job(id=X)` (driving port: IntentStore::put + the
//!    workload-kind discriminator key).
//! 2. Drive convergence ticks until at least one `AllocStatusRow`
//!    for X reaches `Running`.
//! 3. **Fault inject**: `IntentStore::delete("jobs/X")` — primitive
//!    already exists at
//!    `crates/overdrive-core/src/traits/intent_store.rs:193`.
//! 4. Drive up to `MAX_TICKS_GC = 3` more ticks (per architecture.md
//!    § 7 bound).
//! 5. Assert three invariants:
//!    - **gc.converges**: every alloc row for X is terminal
//!      (`Terminated | Failed`).
//!    - **gc.terminal_claim**: every alloc row for X carries
//!      `terminal == Some(Stopped { by: SystemGc })`.
//!    - **gc.no_fresh_alloc**: `assert_always!` — no alloc row for
//!      X with `alloc_id` outside the pre-fault snapshot is
//!      created during the post-fault tick window.
//!
//! ## Scenario 2 — `resubmit_after_gc_creates_fresh_alloc`
//!
//! 1. Sets up the same harness and runs scenario 1's flow to
//!    quiescence.
//! 2. **Action**: resubmit `Job(id=X)` to intent.
//! 3. Drive up to `MAX_TICKS_RESUBMIT = 5` more ticks (per
//!    architecture.md § 7 bound).
//! 4. Assert two invariants:
//!    - **resubmit.places_fresh**: ≥1 alloc row for X reaches
//!      `Running` AND its `alloc_id != original_alloc_id`
//!      (durable distinctness — the GC'd row is not resurrected).
//!    - **resubmit.preserves_prior_gc_terminal**: the original
//!      alloc row's `terminal` field stays
//!      `Some(Stopped { by: SystemGc })` for every tick after
//!      resubmit.
//!
//! ## Mutation-killability targets (per step 01-03 AC)
//!
//! - A mutant flipping the `SystemGc` stamp to `Operator` fails
//!   gc.terminal_claim AND resubmit.preserves_prior_gc_terminal.
//! - A mutant skipping the `StopAllocation` emission entirely
//!   (returning `Vec::new()`) fails gc.converges.
//! - A mutant turning `actual.allocations` filter into bare
//!   `.collect_vec()` (no `Running` predicate) produces stray
//!   actions that violate the existing reconciler-purity / no-
//!   duplicate-stop invariants in `overdrive-core` tests.
//!
//! ## Wiring
//!
//! Sibling pattern: `exit_event_observable_outcome.rs`. The harness
//! lives at the simulation-harness layer because `overdrive-sim`
//! already depends on `overdrive-control-plane` (for `SimViewStore`
//! per ADR-0035 §3) — driving the production action-shim +
//! reconciler-runtime wiring against `Sim*` adapters is structurally
//! supported here. No additional dep introduced.

#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
// Module-level narrative docstrings reference scenario sub-invariant
// names (gc.converges, gc.terminal_claim, gc.no_fresh_alloc,
// resubmit.places_fresh, resubmit.preserves_prior_gc_terminal),
// `StoppedBy` variant words used as concepts (`SystemGc`, `Operator`),
// driver method names embedded in narrative prose, and the
// `IntentStore::put` / `IntentStore::delete` driving-port references.
// Forcing every occurrence into backticks degrades readability of the
// scenario specifications without changing meaning. Scoped expect, not
// crate-wide allow; lifts the moment the docstrings stop firing the
// lint (typically when the narrative is replaced by structured
// reference tables in COMMIT-phase polish).
#![expect(
    clippy::doc_markdown,
    reason = "narrative scenario docstrings reference concept words (gc.converges, SystemGc, Operator, IntentStore::delete) — see module header"
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, noop_heartbeat, workload_lifecycle};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput, WorkloadIntent,
};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_core::transition_reason::StoppedBy;
use overdrive_core::{TerminalCondition, WorkloadId};
use tempfile::TempDir;

use crate::adapters::clock::SimClock;
use crate::adapters::dataplane::SimDataplane;
use crate::adapters::driver::SimDriver;
use crate::adapters::observation_store::SimObservationStore;
use crate::harness::{InvariantResult, InvariantStatus};
use overdrive_store_local::LocalIntentStore;

/// Tick budget for the orphan-converges scenario. Per architecture.md
/// § 7: at one tick the GC arm emits StopAllocation per Running row;
/// the action shim writes terminal + state on the next tick boundary;
/// by the third tick all rows for X are terminal. Anything slower is
/// a bug in the GC arm or the shim's terminal-write ordering.
const MAX_TICKS_GC: u64 = 3;

/// Tick budget for the resubmit-creates-fresh scenario. Per
/// architecture.md § 7: resubmit writes intent; one tick to hydrate,
/// one to schedule, one for the driver to mark Running. Five ticks
/// is a generous ceiling that accommodates the broker re-enqueue
/// settling.
const MAX_TICKS_RESUBMIT: u64 = 5;

/// Convergence-to-Running budget. Same shape as the
/// `exit_event_observable_outcome` harness's `drive_to_running`
/// budget — the test bails Fail if the alloc never reaches Running,
/// which is a different bug class than the GC behavior under test.
const MAX_TICKS_TO_RUNNING: u64 = 30;

const WORKLOAD_NAME: &str = "workload-gc-absent-intent";
const RECONCILER_NAME: &str = "job-lifecycle";

/// Drive scenario 1 and return an `InvariantResult` pinned to the
/// canonical kebab-case name.
pub async fn evaluate_orphan_workload_converges_to_terminal_gc() -> InvariantResult {
    const NAME: &str = "workload-gc-orphan-converges";

    match drive_orphan_converges().await {
        Ok(()) => pass(NAME),
        Err(cause) => fail(NAME, cause),
    }
}

/// Drive scenario 2 and return an `InvariantResult` pinned to the
/// canonical kebab-case name.
pub async fn evaluate_resubmit_after_gc_creates_fresh_alloc() -> InvariantResult {
    const NAME: &str = "workload-gc-resubmit-creates-fresh";

    match drive_resubmit_creates_fresh().await {
        Ok(()) => pass(NAME),
        Err(cause) => fail(NAME, cause),
    }
}

/// Body of scenario 1. Returns `Err(cause)` on any invariant
/// violation; the cause string names the violated sub-invariant
/// (gc.converges / gc.terminal_claim / gc.no_fresh_alloc) and
/// the offending tick / row for triage.
async fn drive_orphan_converges() -> Result<(), String> {
    let tmp = TempDir::new().map_err(|e| format!("tempdir: {e}"))?;
    let h = build_harness(&tmp).await?;

    // Phase A — drive to Running.
    let pre_fault_alloc_ids = drive_to_running(&h).await?;

    // Phase B — fault inject: delete intent.
    fault_inject_delete_intent(&h).await?;

    // Phase C — drive ≤ MAX_TICKS_GC ticks; assert always
    // gc.no_fresh_alloc per tick; once gc.converges + gc.terminal_claim
    // hold within the budget, scenario passes.
    let start_tick = h.next_tick.load(std::sync::atomic::Ordering::Relaxed);
    let mut converged = false;
    for tick_n in start_tick..(start_tick + MAX_TICKS_GC) {
        run_convergence_tick(
            &h.state,
            &h.reconciler_name,
            &h.target,
            h.start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            h.deadline,
        )
        .await
        .map_err(|e| format!("tick {tick_n}: {e:?}"))?;
        yield_a_few().await;

        let rows = read_rows_for_workload(&h).await?;

        // gc.no_fresh_alloc — assert_always: no alloc row for X
        // outside the pre-fault snapshot may exist.
        for row in &rows {
            if !pre_fault_alloc_ids.contains(&row.alloc_id) {
                return Err(format!(
                    "gc.no_fresh_alloc violated at tick {tick_n}: a fresh alloc \
                     `{alloc}` was placed for workload `{wid}` while intent was absent. \
                     Pre-fault allocs: {pre:?}",
                    alloc = row.alloc_id,
                    wid = row.workload_id,
                    pre = pre_fault_alloc_ids,
                ));
            }
        }

        // gc.converges + gc.terminal_claim — assert_eventually
        // (within MAX_TICKS_GC).
        let all_terminal = !rows.is_empty()
            && rows.iter().all(|r| matches!(r.state, AllocState::Terminated | AllocState::Failed));
        let all_system_gc_stamped = !rows.is_empty()
            && rows.iter().all(|r| {
                matches!(r.terminal, Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc }))
            });
        if all_terminal && all_system_gc_stamped {
            converged = true;
            break;
        }
    }

    if !converged {
        let rows = read_rows_for_workload(&h).await?;
        let budget = MAX_TICKS_GC;
        let wid = WORKLOAD_NAME;
        return Err(format!(
            "gc.converges / gc.terminal_claim violated: after {budget} ticks, not every \
             alloc for workload `{wid}` reached a terminal state with \
             `Some(Stopped {{ by: SystemGc }})`. Rows snapshot: {rows:?}",
        ));
    }

    Ok(())
}

/// Body of scenario 2. Sets up the same harness, runs scenario 1's
/// flow to quiescence, then resubmits `Job(X)` and asserts the
/// resubmit invariants.
async fn drive_resubmit_creates_fresh() -> Result<(), String> {
    let tmp = TempDir::new().map_err(|e| format!("tempdir: {e}"))?;
    let h = build_harness(&tmp).await?;

    // Phase A — drive to Running, then through scenario 1's GC
    // convergence to quiescence. Capture the original alloc id(s).
    let pre_fault_alloc_ids = drive_to_running(&h).await?;
    fault_inject_delete_intent(&h).await?;

    let start_tick = h.next_tick.load(std::sync::atomic::Ordering::Relaxed);
    let mut converged = false;
    for tick_n in start_tick..(start_tick + MAX_TICKS_GC) {
        run_convergence_tick(
            &h.state,
            &h.reconciler_name,
            &h.target,
            h.start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            h.deadline,
        )
        .await
        .map_err(|e| format!("tick {tick_n}: {e:?}"))?;
        yield_a_few().await;
        let rows = read_rows_for_workload(&h).await?;
        if !rows.is_empty()
            && rows.iter().all(|r| matches!(r.state, AllocState::Terminated | AllocState::Failed))
            && rows.iter().all(|r| {
                matches!(r.terminal, Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc }))
            })
        {
            converged = true;
            h.next_tick.store(tick_n.saturating_add(1), std::sync::atomic::Ordering::Relaxed);
            break;
        }
    }
    if !converged {
        return Err("scenario 2 setup: scenario 1's GC convergence did not complete \
                    within budget — cannot proceed to resubmit phase"
            .to_owned());
    }

    // Phase B — resubmit Job(X) to intent.
    resubmit_intent(&h).await?;

    // Phase C — drive ≤ MAX_TICKS_RESUBMIT ticks; assert
    // resubmit.places_fresh (eventually) AND
    // resubmit.preserves_prior_gc_terminal (always per tick).
    let resubmit_start_tick = h.next_tick.load(std::sync::atomic::Ordering::Relaxed);
    let mut placed_fresh = false;
    for tick_n in resubmit_start_tick..(resubmit_start_tick + MAX_TICKS_RESUBMIT) {
        run_convergence_tick(
            &h.state,
            &h.reconciler_name,
            &h.target,
            h.start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            h.deadline,
        )
        .await
        .map_err(|e| format!("tick {tick_n}: {e:?}"))?;
        yield_a_few().await;

        let rows = read_rows_for_workload(&h).await?;

        // resubmit.preserves_prior_gc_terminal — assert_always: the
        // original alloc rows MUST stay terminal-stamped with
        // SystemGc for every tick after resubmit. The SystemGc stamp
        // is durable through the resubmit cycle.
        for row in rows.iter().filter(|r| pre_fault_alloc_ids.contains(&r.alloc_id)) {
            if !matches!(row.terminal, Some(TerminalCondition::Stopped { by: StoppedBy::SystemGc }))
            {
                return Err(format!(
                    "resubmit.preserves_prior_gc_terminal violated at tick {tick_n}: \
                     original alloc `{alloc}` lost its SystemGc terminal stamp. \
                     Current row: {row:?}",
                    alloc = row.alloc_id,
                ));
            }
        }

        // resubmit.places_fresh — assert_eventually: ≥1 alloc with a
        // fresh alloc_id (NOT in pre_fault_alloc_ids) reaches Running.
        // The "alloc_id distinctness" half is the durable-distinctness
        // claim from architecture.md § 7 + step 01-03 prompt: the GC'd
        // row is NOT resurrected. A reconciler that reuses the prior
        // alloc_id and overwrites the SystemGc terminal row with a
        // fresh Running row violates BOTH this invariant AND
        // preserves_prior_gc_terminal — the structural defence
        // against the resurrection class.
        let fresh_running = rows
            .iter()
            .any(|r| !pre_fault_alloc_ids.contains(&r.alloc_id) && r.state == AllocState::Running);
        if fresh_running {
            placed_fresh = true;
            break;
        }
    }

    if !placed_fresh {
        let rows = read_rows_for_workload(&h).await?;
        let budget = MAX_TICKS_RESUBMIT;
        let pre = &pre_fault_alloc_ids;
        return Err(format!(
            "resubmit.places_fresh violated: after {budget} ticks, no fresh alloc \
             (alloc_id NOT in pre-fault set {pre:?}) reached Running. Rows snapshot: \
             {rows:?}",
        ));
    }

    Ok(())
}

/// Drive convergence ticks until at least one alloc row for X reaches
/// `Running`. Returns the set of `alloc_id`s observed in the
/// pre-fault snapshot (used to defend against fresh-alloc placement
/// during the post-fault window).
async fn drive_to_running(h: &Harness) -> Result<Vec<AllocationId>, String> {
    let mut reached_running = false;
    let start_tick = h.next_tick.load(std::sync::atomic::Ordering::Relaxed);
    let mut last_tick = start_tick;
    for tick_n in start_tick..(start_tick + MAX_TICKS_TO_RUNNING) {
        last_tick = tick_n;
        run_convergence_tick(
            &h.state,
            &h.reconciler_name,
            &h.target,
            h.start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            h.deadline,
        )
        .await
        .map_err(|e| format!("drive_to_running tick {tick_n}: {e:?}"))?;
        yield_a_few().await;
        let rows = read_rows_for_workload(h).await?;
        if rows.iter().any(|r| matches!(r.state, AllocState::Running)) {
            reached_running = true;
            break;
        }
    }
    if !reached_running {
        return Err(format!(
            "alloc never reached Running within {MAX_TICKS_TO_RUNNING} ticks — \
             the GC scenario cannot test the absent-intent shape without first \
             observing a Running alloc to be GC'd"
        ));
    }
    h.next_tick.store(last_tick.saturating_add(1), std::sync::atomic::Ordering::Relaxed);
    let rows = read_rows_for_workload(h).await?;
    Ok(rows.into_iter().map(|r| r.alloc_id).collect())
}

/// Snapshot the obs store and return rows whose `workload_id`
/// matches the test's workload.
async fn read_rows_for_workload(h: &Harness) -> Result<Vec<AllocStatusRow>, String> {
    let workload_id = WorkloadId::new(WORKLOAD_NAME).map_err(|e| format!("workload id: {e:?}"))?;
    let rows = h.state.obs.alloc_status_rows().await.map_err(|e| format!("read rows: {e:?}"))?;
    Ok(rows.into_iter().filter(|r| r.workload_id == workload_id).collect())
}

/// Fault inject — delete the `jobs/X` intent record. Mirrors the
/// hard-delete shape from architecture.md § 7. Drive port:
/// `IntentStore::delete`.
async fn fault_inject_delete_intent(h: &Harness) -> Result<(), String> {
    let workload_id = WorkloadId::new(WORKLOAD_NAME).map_err(|e| format!("workload id: {e:?}"))?;
    let key = IntentKey::for_workload(&workload_id);
    h.state.store.delete(key.as_bytes()).await.map_err(|e| format!("intent delete: {e:?}"))?;
    Ok(())
}

/// Resubmit `Job(X)` to intent — same shape as initial submit.
async fn resubmit_intent(h: &Harness) -> Result<(), String> {
    let job = build_job_spec()?;
    let intent = WorkloadIntent::Job(job.clone());
    let archived =
        intent.archive_for_store().map_err(|e| format!("rkyv archive (resubmit): {e:?}"))?;
    let key = IntentKey::for_workload(&job.id);
    h.state
        .store
        .put(key.as_bytes(), archived.as_ref())
        .await
        .map_err(|e| format!("intent put (resubmit): {e:?}"))?;
    Ok(())
}

/// Yield a few times to give the spawned tasks chances to run before
/// the next tick. Mirrors the convention in
/// `exit_event_observable_outcome.rs`.
async fn yield_a_few() {
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
}

// ---------------------------------------------------------------------------
// Harness — mirrors `exit_event_observable_outcome::build_harness` but
// without the exit_observer wiring (this scenario does not inject
// driver exits; the GC arm is exercised by deleting intent).
// ---------------------------------------------------------------------------

struct Harness {
    state: AppState,
    target: TargetResource,
    reconciler_name: ReconcilerName,
    start: Instant,
    deadline: Instant,
    /// Monotonic tick counter shared across the scenario phases. Each
    /// `run_convergence_tick` call uses this value and the harness
    /// bumps it after the call. `AtomicU64` (not `Cell`) because the
    /// futures returned by the evaluator must be `Send` so the harness
    /// can run them via `tokio::spawn` / Send-bound combinators —
    /// `Cell<u64>` is `!Sync` and propagates to the future as
    /// `!Send`.
    next_tick: std::sync::atomic::AtomicU64,
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

fn build_job_spec() -> Result<Job, String> {
    Job::from_submit(JobSpecInput {
        id: WORKLOAD_NAME.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/sleep".to_owned(),
            args: vec!["3600".to_owned()],
        }),
    })
    .map_err(|e| format!("valid job spec: {e:?}"))
}

async fn build_harness(tmp: &TempDir) -> Result<Harness, String> {
    let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path())
        .map_err(|e| format!("runtime: {e:?}"))?;
    runtime.register(noop_heartbeat()).await.map_err(|e| format!("register noop: {e:?}"))?;
    runtime
        .register(workload_lifecycle())
        .await
        .map_err(|e| format!("register {RECONCILER_NAME}: {e:?}"))?;

    let store = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb"))
            .map_err(|e| format!("open store: {e:?}"))?,
    );
    let node_id = NodeId::new("local").map_err(|e| format!("node id: {e:?}"))?;
    let sim_obs = Arc::new(SimObservationStore::single_peer(node_id.clone(), 0));
    let obs: Arc<dyn ObservationStore> = sim_obs;
    let sim_clock = Arc::new(SimClock::new());
    let sim_driver = Arc::new(SimDriver::with_clock(DriverType::Exec, sim_clock.clone()));
    let driver: Arc<dyn Driver> = sim_driver;

    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    let state = AppState::new(
        store,
        tmp.path().join("intent.redb"),
        obs,
        Arc::new(runtime),
        driver,
        sim_clock.clone(),
        Arc::new(SimDataplane::new()),
        NodeId::new("writer-1").map_err(|e| format!("writer node id: {e:?}"))?,
        allocator,
        std::net::Ipv4Addr::LOCALHOST,
    );

    // Initial submit — Job(X). The kind discriminator key is omitted;
    // `WorkloadKind::from_discriminator_byte` defaults to Service for
    // an absent byte, which the WorkloadLifecycle reconciler handles
    // as the kind-agnostic shape (the GC arm is kind-agnostic per
    // architecture.md § 6).
    let job = build_job_spec()?;
    let intent = WorkloadIntent::Job(job.clone());
    let archived = intent.archive_for_store().map_err(|e| format!("rkyv archive: {e:?}"))?;
    let key = IntentKey::for_workload(&job.id);
    state
        .store
        .put(key.as_bytes(), archived.as_ref())
        .await
        .map_err(|e| format!("put job: {e:?}"))?;

    let target_str = format!("job/{WORKLOAD_NAME}");
    let target = TargetResource::new(&target_str).map_err(|e| format!("valid target: {e:?}"))?;
    let reconciler_name =
        ReconcilerName::new(RECONCILER_NAME).map_err(|e| format!("reconciler name: {e:?}"))?;

    let start = Instant::now();
    let deadline = start + Duration::from_secs(120);

    // Background ticker — same convention as
    // `exit_event_observable_outcome.rs`.
    let ticker_clock = sim_clock.clone();
    let ticker_handle = tokio::spawn(async move {
        loop {
            ticker_clock.tick(Duration::from_millis(50));
            tokio::task::yield_now().await;
        }
    });

    Ok(Harness {
        state,
        target,
        reconciler_name,
        start,
        deadline,
        next_tick: std::sync::atomic::AtomicU64::new(0),
        sim_clock,
        ticker_handle,
    })
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
