//! workload-identity-manager step 01-07 (Slice 01 CAPSTONE) — the North-Star
//! held-SVID convergence invariant (ADR-0067 D9, O1 / K1).
//!
//! This is where the held-identity walking skeleton is PROVEN converged: the
//! riskiest Slice-01 assumption — "identity warrants its own convergence
//! target" — is validated by driving the REAL convergence loop (the pure
//! `SvidLifecycle` reconciler + the `issue_svid` / `drop_svid` action-shim
//! executors over `SimCa` + `SimObservationStore` + `IdentityMgr`) through
//! allocations churning Running↔Stopped, and asserting the held `BTreeMap`
//! converges against the running-allocation set:
//!
//! - **K1/O1** — eventually every Running alloc holds a valid SVID (the
//!   `issue_svid` executor minted + held it, an `issued_certificates` audit
//!   row exists, the held `spiffe_id` is the pure-derived
//!   `SpiffeId::for_allocation` identity);
//! - **K2/O2** — no Stopped alloc holds an SVID (the `drop_svid` executor
//!   removed the held leaf key — leak resistance on stop).
//!
//! The invariant is `assert_eventually!`, not `assert_always!`: the bounded
//! convergence window (an alloc Running but its `IssueSvid` not yet executed)
//! is fail-CLOSED, not an exposure — the held-vs-running relation is the
//! steady-state target reached within a tick budget, not a per-tick
//! invariant (ADR-0067 D9 / the held-vs-running relation framing).
//!
//! # TEETH (ADR-0067 D9, load-bearing)
//!
//! A scenario test without teeth is a smell (`.claude/rules/testing.md` §
//! Tier 1). [`evaluate_running_set_holds_valid_svid`] proves the HAPPY path
//! converges; [`drive_churn_with_executor_defect`] proves the invariant has
//! TEETH — under a deliberately-broken executor (one that DROPS the hold the
//! real executor just made, or FAILS to drop on stop) the held-vs-running
//! relation is violated and the bounded-tick eventual check returns `Err`.
//! The acceptance test asserts BOTH: the healthy run converges AND each
//! broken variant fails — the falsifiability gate that a neutral stub would
//! silently pass.
//!
//! # Twin-run determinism (K5)
//!
//! The scenario reproduces bit-identically from a seed: the held `BTreeMap`
//! iterates in deterministic `AllocationId` order, `SimCa` draws serials from
//! the seeded [`SimEntropy`], and the fixture cert/key bytes are `const`. Two
//! runs at the same seed produce the same verdict — surfaced through the
//! harness's `RunReport` (`cargo dst --seed <N>` reproduces). Flaky DST is a
//! sim-layer bug, never a rerun (`.claude/rules/testing.md` § Tier 1).
//!
//! # Wiring
//!
//! Sibling pattern: [`crate::invariants::workload_gc_absent_intent`]. The
//! harness lives at the simulation layer because `overdrive-sim` already
//! depends on `overdrive-control-plane` (for `SimViewStore` per ADR-0035 §3),
//! so driving the production action-shim + reconciler-runtime wiring against
//! `Sim*` adapters is structurally supported here — no new dep. The driving
//! port is `run_convergence_tick` for the `svid-lifecycle` reconciler against
//! a `job/<workload>` target; the observable assertions land at the
//! `IdentityMgr::held_snapshot` + `ObservationStore::issued_certificate_rows`
//! + `ObservationStore::alloc_status_rows` driven-port boundaries. No
//! reconciler / executor internals are exercised directly.

use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, noop_heartbeat, svid_lifecycle};
use overdrive_core::SpiffeId;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
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

/// Tick budget for the held-vs-running relation to converge. Each Running
/// alloc converges in ONE svid-lifecycle tick (`running ∧ ¬held → IssueSvid`
/// → executor mints + holds), each Stopped alloc in one tick
/// (`¬running ∧ held → DropSvid` → executor drops). The budget is kept loose
/// so a future multi-tick convergence shape has headroom; a regression that
/// drops convergence entirely still fails within the budget.
const CONVERGENCE_TICK_BUDGET: u64 = 8;

/// The workload every alloc in the churn scenario belongs to. The
/// svid-lifecycle reconciler ticks against `job/<WORKLOAD_NAME>` (the
/// `workload_id_from_target` shape the runtime's `hydrate_svid_desired`
/// arm requires).
const WORKLOAD_NAME: &str = "workload-identity-capstone";

/// The node every alloc runs on (single-node, Phase 1).
const NODE_NAME: &str = "host-0";

/// Which executor-defect to inject after the production executor runs, to
/// prove the invariant has teeth (ADR-0067 D9). `None` is the healthy path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutorDefect {
    /// A broken `IssueSvid` executor that mints the SVID but FAILS to hold it
    /// — the held leaf key never enters the held map, so a Running alloc reads
    /// `¬held` forever. Modelled by dropping the hold the real executor just
    /// made. Violates K1/O1.
    DropsTheHold,
    /// A broken `DropSvid` executor that FAILS to drop on stop — the held leaf
    /// key for a Stopped alloc stays reachable in the held map. Modelled by
    /// re-holding the entry the real executor just dropped. Violates K2/O2.
    FailsToDropOnStop,
}

/// North-Star evaluator (ADR-0067 D9, O1 / K1).
///
/// Drives the HEALTHY churn scenario and asserts the held-vs-running relation
/// converges within the tick budget. Registered in the `Invariant` catalogue
/// and on the `cargo dst` critical path.
///
/// Returns `Pass` iff, after churning allocs Running↔Stopped and ticking the
/// real svid-lifecycle convergence loop, EVERY Running alloc holds a valid
/// SVID (held `spiffe_id == SpiffeId::for_allocation(workload, alloc)` AND an
/// `issued_certificates` audit row exists for it) AND NO Stopped alloc holds
/// one. `Fail` with a triage cause otherwise.
pub async fn evaluate_running_set_holds_valid_svid() -> InvariantResult {
    const NAME: &str = "svid-running-set-holds-valid-svid";

    match drive_churn_with_executor_defect(None).await {
        Ok(()) => pass(NAME),
        Err(cause) => fail(NAME, cause),
    }
}

/// Drive the churn scenario through the REAL convergence loop, optionally
/// injecting `defect` after each production-executor tick to prove teeth.
///
/// Scenario (a fixed three-alloc churn):
///
/// 1. `alloc-a`, `alloc-b` reach Running → tick → both held with valid SVIDs.
/// 2. `alloc-a` stops (its `alloc_status` row leaves Running) → tick →
///    `alloc-a` dropped, `alloc-b` still held.
/// 3. `alloc-c` reaches Running, `alloc-b` stops → tick → steady state:
///    `alloc-b`, `alloc-c` are the running set; `alloc-c` held, `alloc-b`
///    dropped, `alloc-a` still dropped.
///
/// After EACH tick the held-vs-running relation is checked against the
/// current running set; convergence must hold within
/// [`CONVERGENCE_TICK_BUDGET`] ticks per churn step. With `defect = None`
/// the relation converges (the North-Star). With a defect injected the
/// relation is violated and this returns `Err` — the teeth proof.
///
/// # Errors
///
/// Returns `Err(cause)` naming the violated sub-relation (`running ∧ ¬held`,
/// or `¬running ∧ held`, or `held identity mismatch`, or `no audit row`) and
/// the offending alloc / tick for triage.
pub async fn drive_churn_with_executor_defect(
    defect: Option<ExecutorDefect>,
) -> Result<(), String> {
    let tmp = TempDir::new().map_err(|e| format!("tempdir: {e}"))?;
    let h = build_harness(&tmp).await?;

    // Churn step 1 — alloc-a, alloc-b reach Running.
    write_alloc_state(&h, "alloc-a", AllocState::Running).await?;
    write_alloc_state(&h, "alloc-b", AllocState::Running).await?;
    converge_and_assert(&h, defect, &["alloc-a", "alloc-b"]).await?;

    // Churn step 2 — alloc-a stops (leaves Running); alloc-b still Running.
    write_alloc_state(&h, "alloc-a", AllocState::Terminated).await?;
    converge_and_assert(&h, defect, &["alloc-b"]).await?;

    // Churn step 3 — alloc-c reaches Running, alloc-b stops.
    write_alloc_state(&h, "alloc-c", AllocState::Running).await?;
    write_alloc_state(&h, "alloc-b", AllocState::Terminated).await?;
    converge_and_assert(&h, defect, &["alloc-c"]).await?;

    Ok(())
}

/// Tick the svid-lifecycle convergence loop up to [`CONVERGENCE_TICK_BUDGET`]
/// times, applying `defect` after each tick, until the held-vs-running
/// relation holds against `expected_running` (the Running alloc ids). On a
/// healthy run the relation converges; under a defect it never does and this
/// returns `Err` once the budget is exhausted.
async fn converge_and_assert(
    h: &Harness,
    defect: Option<ExecutorDefect>,
    expected_running: &[&str],
) -> Result<(), String> {
    let start_tick = h.next_tick.load(std::sync::atomic::Ordering::Relaxed);
    let mut last_cause = String::from("scenario did not run any ticks");
    for tick_n in start_tick..(start_tick + CONVERGENCE_TICK_BUDGET) {
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

        // TEETH (ADR-0067 D9): model a broken executor as a mutation on the
        // observable held set AFTER the real executor ran this tick. The
        // production executor's hold/drop is genuine (the SVID was minted by
        // SimCa and audited); the defect then breaks the held-vs-running
        // relation exactly as a broken executor would, so the assertion below
        // must catch it. `None` leaves the genuine production outcome intact.
        apply_executor_defect(h, defect, expected_running);

        match check_held_vs_running(h, expected_running).await {
            Ok(()) => {
                h.next_tick.store(tick_n.saturating_add(1), std::sync::atomic::Ordering::Relaxed);
                return Ok(());
            }
            Err(cause) => last_cause = cause,
        }
    }
    h.next_tick.store(
        start_tick.saturating_add(CONVERGENCE_TICK_BUDGET),
        std::sync::atomic::Ordering::Relaxed,
    );
    Err(format!(
        "held-vs-running relation did not converge within {CONVERGENCE_TICK_BUDGET} ticks \
         (expected running={expected_running:?}): {last_cause}"
    ))
}

/// Model a broken executor as a mutation on the observable held set — the
/// teeth mechanism (ADR-0067 D9). The mutation breaks the SAME observable
/// surface (`IdentityMgr::held_snapshot`) the invariant asserts on, so a
/// genuine broken executor would surface identically.
fn apply_executor_defect(h: &Harness, defect: Option<ExecutorDefect>, expected_running: &[&str]) {
    match defect {
        None => {}
        Some(ExecutorDefect::DropsTheHold) => {
            // A broken IssueSvid that minted but never held: drop the hold the
            // real executor just made for each Running alloc.
            for raw in expected_running {
                if let Ok(alloc) = AllocationId::new(raw) {
                    h.state.identity.drop_svid(&alloc);
                }
            }
        }
        Some(ExecutorDefect::FailsToDropOnStop) => {
            // A broken DropSvid that never removed the held entry: re-hold any
            // alloc that is held-or-was-held but is NOT in the running set.
            // We re-hold every non-running alloc id the scenario uses so a
            // Stopped alloc stays reachable in the held map.
            for raw in ["alloc-a", "alloc-b", "alloc-c"] {
                if expected_running.contains(&raw)
                    && let Ok(_alloc) = AllocationId::new(raw)
                {
                    continue;
                }
                if let Ok(alloc) = AllocationId::new(raw) {
                    h.state.identity.hold(alloc, defect_svid(raw));
                }
            }
        }
    }
}

/// Assert the held-vs-running relation: every alloc in `expected_running` is
/// held with a valid SVID (correct identity + an `issued_certificates` audit
/// row), and no alloc outside the running set is held.
///
/// This is the `assert_eventually!("running allocs hold a valid SVID")`
/// body — the K1/O1 + K2/O2 acceptance surface. All assertions read the
/// observable driven-port boundaries: `IdentityMgr::held_snapshot` (the held
/// `BTreeMap`), `ObservationStore::issued_certificate_rows` (the audit
/// surface), never internal reachability.
async fn check_held_vs_running(h: &Harness, expected_running: &[&str]) -> Result<(), String> {
    let held = h.state.identity.held_snapshot();
    let workload = WorkloadId::new(WORKLOAD_NAME).map_err(|e| format!("workload id: {e:?}"))?;

    // K1/O1 — every Running alloc holds a valid SVID.
    for raw in expected_running {
        let alloc = AllocationId::new(raw).map_err(|e| format!("alloc id {raw:?}: {e:?}"))?;
        let Some(facts) = held.get(&alloc) else {
            return Err(format!(
                "running ∧ ¬held: Running alloc `{raw}` holds NO SVID — the issue executor \
                 did not mint+hold (held set: {:?})",
                held.keys().collect::<Vec<_>>()
            ));
        };
        // The held identity is the pure-derived SpiffeId::for_allocation.
        let expected_id = SpiffeId::for_allocation(&workload, &alloc);
        if facts.spiffe_id != expected_id {
            return Err(format!(
                "held identity mismatch for `{raw}`: held {:?}, expected {:?}",
                facts.spiffe_id, expected_id
            ));
        }
    }

    // K1/O1 audit binding — every Running alloc has an issued_certificates
    // audit row for its identity (the issue executor wrote it via
    // ca_issuance::issue_and_audit; audit-before-hold, ADR-0063 D6).
    let audit_rows = h
        .state
        .obs
        .issued_certificate_rows()
        .await
        .map_err(|e| format!("read audit rows: {e:?}"))?;
    for raw in expected_running {
        let alloc = AllocationId::new(raw).map_err(|e| format!("alloc id {raw:?}: {e:?}"))?;
        let expected_id = SpiffeId::for_allocation(&workload, &alloc);
        if !audit_rows.iter().any(|r| r.spiffe_id == expected_id) {
            return Err(format!(
                "no audit row for Running alloc `{raw}` (identity {expected_id}): the issue \
                 executor must write an issued_certificates row before holding"
            ));
        }
    }

    // K2/O2 — no Stopped alloc holds an SVID. Every held alloc must be in the
    // running set; a held alloc outside it is a leaked leaf key on stop.
    for held_alloc in held.keys() {
        if !expected_running.iter().any(|raw| held_alloc.as_str() == *raw) {
            return Err(format!(
                "¬running ∧ held: alloc `{held_alloc}` is held but NOT in the running set \
                 {expected_running:?} — the drop executor did not remove the held leaf key \
                 (leak resistance on stop, O2/K2)"
            ));
        }
    }

    Ok(())
}

/// Write an `AllocStatusRow` for `alloc_raw` in `state` through the
/// `ObservationStore` port — the churn driver. A `Running` row puts the alloc
/// in the svid reconciler's `desired` (Running) set; a `Terminated` row drops
/// it out (leaving the running set), so the next tick's
/// `¬running ∧ held → DropSvid` fires. Newer writes win under LWW (the
/// counter advances with the tick), so a later state overwrites an earlier one
/// for the same alloc.
async fn write_alloc_state(h: &Harness, alloc_raw: &str, state: AllocState) -> Result<(), String> {
    let counter = h.next_tick.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let writer = NodeId::new(NODE_NAME).map_err(|e| format!("writer node id: {e:?}"))?;
    let row = AllocStatusRow {
        alloc_id: AllocationId::new(alloc_raw).map_err(|e| format!("alloc id: {e:?}"))?,
        workload_id: WorkloadId::new(WORKLOAD_NAME).map_err(|e| format!("workload id: {e:?}"))?,
        node_id: NodeId::new(NODE_NAME).map_err(|e| format!("node id: {e:?}"))?,
        state,
        // LWW timestamp — counter advances per write so a later state wins.
        updated_at: LogicalTimestamp { counter: counter.saturating_add(1), writer },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Job,
        listeners: Vec::new(),
        started_at: None,
    };
    h.state
        .obs
        .write(ObservationRow::AllocStatus(Box::new(row)))
        .await
        .map_err(|e| format!("write alloc_status row for {alloc_raw}: {e:?}"))?;
    Ok(())
}

/// Yield a few times to let spawned action-shim tasks settle before the next
/// tick. Mirrors `workload_gc_absent_intent::yield_a_few`.
async fn yield_a_few() {
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
}

/// Build a placeholder `SvidMaterial` for the `FailsToDropOnStop` teeth
/// variant — the held-set assertion reads only the `held_snapshot`
/// projection (presence + identity), so the cert/key bytes are placeholders.
fn defect_svid(alloc_raw: &str) -> SvidMaterial {
    use overdrive_core::CertSerial;
    use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem};
    use overdrive_core::wall_clock::UnixInstant;

    let workload =
        WorkloadId::new(WORKLOAD_NAME).unwrap_or_else(|_| unreachable!("WORKLOAD_NAME is valid"));
    let alloc =
        AllocationId::new(alloc_raw).unwrap_or_else(|_| unreachable!("scenario alloc id is valid"));
    SvidMaterial::new(
        CaCertPem::new("-----BEGIN CERTIFICATE-----\nLEAKED\n-----END CERTIFICATE-----\n".into()),
        CaCertDer::new(vec![0xDE, 0xAD]),
        CertSerial::new("0badc0de").unwrap_or_else(|_| unreachable!("serial is valid hex")),
        SpiffeId::for_allocation(&workload, &alloc),
        CaKeyPem::new("-----BEGIN PRIVATE KEY-----\nLEAKED\n-----END PRIVATE KEY-----\n".into()),
        UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000)),
    )
}

// ---------------------------------------------------------------------------
// Harness — mirrors `workload_gc_absent_intent::build_harness` but registers
// the svid-lifecycle reconciler and ticks it against the workload target.
// ---------------------------------------------------------------------------

struct Harness {
    state: AppState,
    target: TargetResource,
    reconciler_name: ReconcilerName,
    start: Instant,
    deadline: Instant,
    /// Monotonic counter shared across churn phases — used both for the
    /// `run_convergence_tick` `tick_n` and the LWW `updated_at.counter` on
    /// written alloc rows. `AtomicU64` because the evaluator's futures must be
    /// `Send`.
    next_tick: std::sync::atomic::AtomicU64,
    #[allow(dead_code)]
    sim_clock: Arc<SimClock>,
}

async fn build_harness(tmp: &TempDir) -> Result<Harness, String> {
    let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path())
        .map_err(|e| format!("runtime: {e:?}"))?;
    runtime.register(noop_heartbeat()).await.map_err(|e| format!("register noop: {e:?}"))?;
    runtime
        .register(svid_lifecycle())
        .await
        .map_err(|e| format!("register svid-lifecycle: {e:?}"))?;

    let store = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb"))
            .map_err(|e| format!("open store: {e:?}"))?,
    );
    let node_id = NodeId::new(NODE_NAME).map_err(|e| format!("node id: {e:?}"))?;
    let sim_obs = Arc::new(SimObservationStore::single_peer(node_id.clone(), 0));
    let obs: Arc<dyn ObservationStore> = sim_obs;
    let sim_clock = Arc::new(SimClock::new());
    let sim_driver = Arc::new(SimDriver::with_clock(DriverType::Exec, sim_clock.clone()));
    let driver: Arc<dyn Driver> = sim_driver;

    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);

    // Boot trust bundle — the IdentityMgr starts with the SimCa's bundle so
    // the read surface is current (the issue executor refreshes it, D6).
    let boot_bundle: Option<TrustBundle> = None;
    let state = AppState::new(
        store,
        tmp.path().join("intent.redb"),
        obs,
        Arc::new(runtime),
        driver,
        sim_clock.clone(),
        Arc::new(SimDataplane::new()),
        // SimCa over the seeded entropy — serials are drawn deterministically
        // from the seed (K5).
        Arc::new(SimCa::new(Arc::new(SimEntropy::new(0)))),
        Arc::new(IdentityMgr::new(boot_bundle)),
        node_id,
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    );

    // The svid-lifecycle reconciler ticks against a `job/<workload>` target —
    // the `workload_id_from_target` shape its `hydrate_svid_desired` arm
    // requires (the reconciler is keyed by name, the target is the workload).
    let target_str = format!("job/{WORKLOAD_NAME}");
    let target = TargetResource::new(&target_str).map_err(|e| format!("valid target: {e:?}"))?;
    let reconciler_name = ReconcilerName::new(
        <overdrive_core::reconcilers::svid_lifecycle::SvidLifecycle as overdrive_core::reconcilers::Reconciler>::NAME,
    )
    .map_err(|e| format!("reconciler name: {e:?}"))?;

    let start = Instant::now();
    let deadline = start + Duration::from_secs(120);

    Ok(Harness {
        state,
        target,
        reconciler_name,
        start,
        deadline,
        next_tick: std::sync::atomic::AtomicU64::new(0),
        sim_clock,
    })
}

// ---------------------------------------------------------------------------
// Helpers — pin the canonical name + host string in `InvariantResult`.
// ---------------------------------------------------------------------------

fn pass(name: &str) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Pass,
        tick: 1,
        host: NODE_NAME.to_owned(),
        cause: None,
    }
}

fn fail(name: &str, cause: String) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Fail,
        tick: 1,
        host: NODE_NAME.to_owned(),
        cause: Some(cause),
    }
}
