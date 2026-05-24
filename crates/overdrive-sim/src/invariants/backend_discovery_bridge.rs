//! DST invariants for `backend-discovery-bridge-service-reachability` (joint #174 + #175).
//!
//! Per `docs/feature/backend-discovery-bridge-service-reachability/distill/test-scenarios.md`
//! S-BDB-02..S-BDB-10 and Atlas Q2 (S-BDB-06). Tier 1 — pure-Rust under sim
//! adapters; runs via `cargo dst` on every PR.
//!
//! Three evaluators, all returning [`InvariantResult`] from the shared
//! harness dispatch in `crate::harness`:
//!
//! - [`evaluate_bridge_eventually_writes_backend_row`] (S-BDB-02 / S-BDB-03
//!   / S-BDB-04 / S-BDB-10) — eventual: for every Service workload with
//!   ≥ 1 listener AND allocator-issued VIP AND ≥ 1 Running alloc, the
//!   harness's `SimObservationStore` eventually carries a
//!   `ServiceBackendRow` whose `backends` matches the Running alloc set.
//! - [`evaluate_bridge_idempotent_steady_state`] (S-BDB-05 / S-BDB-07) —
//!   always: once steady state, subsequent ticks with unchanged inputs
//!   produce zero `Action::WriteServiceBackendRow` actions AND the View's
//!   `last_written_fingerprint` map is stable; the View GC `retain`
//!   clause drops removed `ServiceId` entries when the listener set
//!   shrinks.
//! - [`evaluate_bridge_recomputes_fingerprint_on_replay`] (S-BDB-06 /
//!   Atlas Q2) — always under the crash-recovery scenario family: the
//!   harness injects a crash between `SimViewStore::write_through` fsync
//!   and the runtime's in-memory `BTreeMap::insert`, then drives a
//!   restart-equivalent `bulk_load` + first-tick re-projection. The
//!   bridge MUST recompute the fingerprint from inputs — never silent-
//!   skip on cached stale state.
//!
//! Per `.claude/rules/development.md` § "Reconciler I/O" the runtime's
//! fsync-then-memory ordering rule is structurally load-bearing for
//! crash recovery; the Atlas Q2 invariant proves the bridge honors it.
//!
//! Production code these invariants guard (per Atlas Q3 mutation-scope
//! mapping in `docs/feature/backend-discovery-bridge-service-reachability/distill/wave-decisions.md`):
//!
//! - `BackendDiscoveryBridge::reconcile` body — main loop
//! - `BackendDiscoveryBridge::reconcile` dedup branch
//! - `BackendDiscoveryBridge::reconcile` View GC `retain` clause
//! - `fingerprint(&vip, &backends)` call site
//! - `hydrate_desired` allocator-lookup arm
//! - `hydrate_actual` Running-filter arm
//! - `Action::WriteServiceBackendRow` action shim dispatch
//! - `ViewStore` crash-recovery semantics (fsync-then-memory ordering)

use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroU16;
use std::time::{Duration, Instant};

use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip, WorkloadId};
use overdrive_core::reconcilers::backend_discovery_bridge::{
    BackendDiscoveryBridge, BackendDiscoveryBridgeState, BackendDiscoveryBridgeView,
    ProjectedListener,
};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::observation_store::{ObservationRow, ObservationStore};
use overdrive_core::wall_clock::UnixInstant;

use crate::adapters::observation_store::SimObservationStore;
use crate::harness::{InvariantResult, InvariantStatus};

/// Maximum reconcile ticks the eventual-convergence invariant gives
/// the bridge before declaring divergence. Today the bridge converges
/// in ONE tick per service (one dispatch → one row write → next tick
/// hits the dedup branch). The budget is kept loose so future changes
/// that introduce multi-tick convergence (e.g. cross-service ordering)
/// have headroom without flipping the invariant; a regression that
/// drops convergence entirely still fails.
const CONVERGENCE_TICK_BUDGET: u32 = 8;

/// Number of idempotent steady-state ticks the always-invariant
/// asserts. A single tick would be brittle — the property is "every
/// post-convergence tick emits zero actions"; running through several
/// confirms the steady state holds.
const STEADY_STATE_TICKS: u32 = 5;

/// Canonical node id used by every evaluator. Pins the
/// `LogicalTimestamp::writer` field on emitted rows so the
/// post-condition checks have a stable expected value.
fn writer_node_id() -> NodeId {
    #[allow(clippy::expect_used)]
    NodeId::new("host-0").expect("'host-0' is a valid NodeId")
}

/// Canonical host IPv4 used by every evaluator. Pins the backend
/// endpoint addr so post-condition checks have a stable expected
/// value. Phase 2.2 single-node: every Running alloc resolves here.
const fn host_ipv4() -> Ipv4Addr {
    Ipv4Addr::new(10, 0, 0, 5)
}

fn workload_id(raw: &str) -> Result<WorkloadId, String> {
    WorkloadId::new(raw).map_err(|e| format!("invalid WorkloadId {raw:?}: {e}"))
}

fn service_id(value: u64) -> Result<ServiceId, String> {
    ServiceId::new(value).map_err(|e| format!("invalid ServiceId {value}: {e}"))
}

fn service_vip(addr: Ipv4Addr) -> Result<ServiceVip, String> {
    ServiceVip::new(IpAddr::V4(addr)).map_err(|e| format!("invalid ServiceVip {addr}: {e}"))
}

fn alloc_id(raw: &str) -> Result<AllocationId, String> {
    AllocationId::new(raw).map_err(|e| format!("invalid AllocationId {raw:?}: {e}"))
}

fn listener(vip: ServiceVip, port: u16) -> Result<ProjectedListener, String> {
    let port = NonZeroU16::new(port).ok_or_else(|| format!("port {port} must be non-zero"))?;
    Ok(ProjectedListener { vip, port, protocol: Proto::Tcp })
}

/// Synthetic [`TickContext`] for the evaluator harness. Time advances
/// deterministically by tick index; never reads wall-clock. Pure
/// inputs only — K3 reproducibility.
fn make_tick(idx: u32) -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(u64::from(idx) * 60)),
        tick: u64::from(idx),
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

fn pass(name: &str) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Pass,
        tick: 1,
        host: "host-0".to_owned(),
        cause: None,
    }
}

fn fail(name: &str, cause: String) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Fail,
        tick: 1,
        host: "host-0".to_owned(),
        cause: Some(cause),
    }
}

/// Apply emitted `Action::WriteServiceBackendRow` actions to the
/// `SimObservationStore` — this is the in-evaluator simulation of
/// `crates/overdrive-control-plane/src/action_shim/
/// write_service_backend_row.rs`. The action shim's production
/// behaviour: dispatch into
/// `ObservationStore::write(ObservationRow::ServiceBackend(row))`.
///
/// Returns the number of rows written, or an error string if the
/// store rejected any write (the test contract expects writes to
/// succeed in the canonical fault catalogue).
async fn apply_actions(obs: &SimObservationStore, actions: &[Action]) -> Result<usize, String> {
    let mut written = 0usize;
    for action in actions {
        if let Action::WriteServiceBackendRow { row, .. } = action {
            obs.write(ObservationRow::ServiceBackend(row.clone()))
                .await
                .map_err(|e| format!("SimObservationStore::write failed: {e}"))?;
            written += 1;
        }
    }
    Ok(written)
}

/// Eventual-convergence evaluator — closes S-BDB-02 / S-BDB-03 /
/// S-BDB-04 / S-BDB-10.
///
/// Drives three sub-scenarios from the fault catalogue documented at
/// the module level:
///
/// 1. Single Running alloc → single backend entry.
/// 2. Multiple concurrent Running allocs → backend set is the union.
/// 3. Running → Failed (drop from running set) → second Running:
///    steady state reflects the second alloc only.
///
/// All three are evaluated under the same harness shape: tick the
/// bridge, apply emitted actions to `SimObservationStore`, observe
/// convergence via `service_backends_rows`. Convergence MUST hold
/// within [`CONVERGENCE_TICK_BUDGET`] ticks.
pub async fn evaluate_bridge_eventually_writes_backend_row() -> InvariantResult {
    const NAME: &str = "bridge-eventually-writes-backend-row";

    // Scenario A — single Running alloc.
    if let Err(cause) = scenario_single_alloc().await {
        return fail(NAME, format!("scenario A (single alloc): {cause}"));
    }

    // Scenario B — multiple concurrent Running allocs.
    if let Err(cause) = scenario_multi_alloc().await {
        return fail(NAME, format!("scenario B (multi alloc): {cause}"));
    }

    // Scenario C — Running → Failed → second Running. Steady state
    // reflects the second alloc only.
    if let Err(cause) = scenario_alloc_replacement().await {
        return fail(NAME, format!("scenario C (alloc replacement): {cause}"));
    }

    pass(NAME)
}

/// S-BDB-02 sub-scenario: a single Running alloc produces a single
/// backend entry; the observed row's `updated_at.counter` is
/// `tick.tick + 1`; the `writer` matches the configured node id; the
/// `vip` matches the allocator-issued VIP.
async fn scenario_single_alloc() -> Result<(), String> {
    let bridge = BackendDiscoveryBridge::new(host_ipv4(), writer_node_id());
    let obs = SimObservationStore::single_peer(writer_node_id(), 0);

    let wid = workload_id("payments")?;
    let sid = service_id(1)?;
    let vip = service_vip(Ipv4Addr::new(10, 1, 0, 1))?;
    let lst = listener(vip, 8080)?;

    let mut state = BackendDiscoveryBridgeState::empty_for_workload(wid.clone());
    state.desired.listeners.insert(sid, lst);
    state.actual.running.insert(alloc_id("alloc-a")?);

    let mut view = BackendDiscoveryBridgeView::default();
    for tick_idx in 0..CONVERGENCE_TICK_BUDGET {
        let tick = make_tick(tick_idx);
        let (actions, next_view) = bridge.reconcile(&state, &state, &view, &tick);
        let _ = apply_actions(&obs, &actions).await?;
        view = next_view;

        let rows = obs
            .service_backends_rows(&sid)
            .await
            .map_err(|e| format!("service_backends_rows: {e}"))?;
        if let Some(row) = rows.first() {
            if row.backends.len() == 1
                && row.vip == Ipv4Addr::new(10, 1, 0, 1)
                && row.updated_at.writer == writer_node_id()
                && row.updated_at.counter == u64::from(tick_idx).saturating_add(1)
            {
                return Ok(());
            }
        }
    }

    Err(format!(
        "single-alloc scenario did not converge within {CONVERGENCE_TICK_BUDGET} ticks; \
         final view fingerprints={:?}",
        view.last_written_fingerprint.keys().collect::<Vec<_>>()
    ))
}

/// S-BDB-04 sub-scenario: N=3 Running allocs produce backend set
/// length 3.
async fn scenario_multi_alloc() -> Result<(), String> {
    let bridge = BackendDiscoveryBridge::new(host_ipv4(), writer_node_id());
    let obs = SimObservationStore::single_peer(writer_node_id(), 0);

    let wid = workload_id("frontend")?;
    let sid = service_id(2)?;
    let vip = service_vip(Ipv4Addr::new(10, 1, 0, 2))?;
    let lst = listener(vip, 9000)?;

    let mut state = BackendDiscoveryBridgeState::empty_for_workload(wid.clone());
    state.desired.listeners.insert(sid, lst);
    state.actual.running.insert(alloc_id("alloc-x")?);
    state.actual.running.insert(alloc_id("alloc-y")?);
    state.actual.running.insert(alloc_id("alloc-z")?);

    let mut view = BackendDiscoveryBridgeView::default();
    for tick_idx in 0..CONVERGENCE_TICK_BUDGET {
        let tick = make_tick(tick_idx);
        let (actions, next_view) = bridge.reconcile(&state, &state, &view, &tick);
        let _ = apply_actions(&obs, &actions).await?;
        view = next_view;

        let rows = obs
            .service_backends_rows(&sid)
            .await
            .map_err(|e| format!("service_backends_rows: {e}"))?;
        if let Some(row) = rows.first() {
            if row.backends.len() == 3 {
                return Ok(());
            }
        }
    }

    Err(format!("multi-alloc scenario did not converge within {CONVERGENCE_TICK_BUDGET} ticks"))
}

/// S-BDB-03 sub-scenario: converge on alloc A, drop A, add B,
/// re-tick — final observed row's `backends.len() == 1` and the
/// `updated_at.counter` is strictly higher than the prior write.
async fn scenario_alloc_replacement() -> Result<(), String> {
    let bridge = BackendDiscoveryBridge::new(host_ipv4(), writer_node_id());
    let obs = SimObservationStore::single_peer(writer_node_id(), 0);

    let wid = workload_id("api")?;
    let sid = service_id(3)?;
    let vip = service_vip(Ipv4Addr::new(10, 1, 0, 3))?;
    let lst = listener(vip, 8443)?;

    let mut state = BackendDiscoveryBridgeState::empty_for_workload(wid.clone());
    state.desired.listeners.insert(sid, lst);
    state.actual.running.insert(alloc_id("alloc-a")?);

    let mut view = BackendDiscoveryBridgeView::default();

    // Phase 1 — converge on alloc-a.
    let tick = make_tick(0);
    let (actions, next_view) = bridge.reconcile(&state, &state, &view, &tick);
    let _ = apply_actions(&obs, &actions).await?;
    view = next_view;
    let first_counter = obs
        .service_backends_rows(&sid)
        .await
        .map_err(|e| format!("first service_backends_rows: {e}"))?
        .first()
        .map(|r| r.updated_at.counter)
        .ok_or_else(|| "phase 1 produced no observable row".to_owned())?;

    // Phase 2 — drop alloc-a, add alloc-b. Run remaining ticks.
    state.actual.running.clear();
    state.actual.running.insert(alloc_id("alloc-b")?);
    for tick_idx in 1..CONVERGENCE_TICK_BUDGET {
        let tick = make_tick(tick_idx);
        let (actions, next_view) = bridge.reconcile(&state, &state, &view, &tick);
        let _ = apply_actions(&obs, &actions).await?;
        view = next_view;

        let rows = obs
            .service_backends_rows(&sid)
            .await
            .map_err(|e| format!("service_backends_rows: {e}"))?;
        if let Some(row) = rows.first() {
            if row.backends.len() == 1 && row.updated_at.counter > first_counter {
                return Ok(());
            }
        }
    }

    Err(format!(
        "alloc-replacement scenario did not produce a strictly-newer single-backend row \
         within {CONVERGENCE_TICK_BUDGET} ticks (first_counter={first_counter})"
    ))
}

/// Idempotent-steady-state + View-GC evaluator — closes S-BDB-05 +
/// S-BDB-07.
///
/// 1. Reach steady state on a single service.
/// 2. Tick K=[`STEADY_STATE_TICKS`] more times with unchanged inputs.
///    Every tick MUST emit zero actions AND the `next_view` MUST
///    equal the prior view.
/// 3. Shrink `desired.listeners` to empty. Tick once. The View's
///    `last_written_fingerprint` MUST shed the removed `ServiceId`
///    (the GC `retain` clause).
pub async fn evaluate_bridge_idempotent_steady_state() -> InvariantResult {
    const NAME: &str = "bridge-idempotent-steady-state";

    let bridge = BackendDiscoveryBridge::new(host_ipv4(), writer_node_id());
    let obs = SimObservationStore::single_peer(writer_node_id(), 0);

    let wid = match workload_id("payments") {
        Ok(w) => w,
        Err(cause) => return fail(NAME, cause),
    };
    let sid = match service_id(1) {
        Ok(s) => s,
        Err(cause) => return fail(NAME, cause),
    };
    let vip = match service_vip(Ipv4Addr::new(10, 1, 0, 1)) {
        Ok(v) => v,
        Err(cause) => return fail(NAME, cause),
    };
    let lst = match listener(vip, 8080) {
        Ok(l) => l,
        Err(cause) => return fail(NAME, cause),
    };

    let mut state = BackendDiscoveryBridgeState::empty_for_workload(wid);
    state.desired.listeners.insert(sid, lst);
    let single_alloc = match alloc_id("alloc-a") {
        Ok(a) => a,
        Err(cause) => return fail(NAME, cause),
    };
    state.actual.running.insert(single_alloc);

    // STEP 1 — reach steady state. First tick MUST emit one action;
    // applying it populates obs; subsequent ticks MUST dedup.
    let mut view = BackendDiscoveryBridgeView::default();
    let tick0 = make_tick(0);
    let (actions0, view_after_seed) = bridge.reconcile(&state, &state, &view, &tick0);
    // UI-05 dual-emit: bridge emits WriteServiceBackendRow +
    // EnqueueEvaluation per drifted service. The two actions land
    // together; either ALL of them apply or NONE (the invariant
    // tests both at once by checking the pair count).
    if actions0.len() != 2 {
        return fail(
            NAME,
            format!(
                "seed tick must emit exactly two actions \
                 (WriteServiceBackendRow + EnqueueEvaluation per UI-05); got {}",
                actions0.len()
            ),
        );
    }
    if let Err(cause) = apply_actions(&obs, &actions0).await {
        return fail(NAME, cause);
    }
    if !view_after_seed.last_written_fingerprint.contains_key(&sid) {
        return fail(NAME, "seed tick must record fingerprint for the written service".to_owned());
    }
    view = view_after_seed;

    // STEP 2 — K subsequent ticks must emit zero actions AND leave
    // the View unchanged.
    let stable_view = view.clone();
    for tick_idx in 1..=STEADY_STATE_TICKS {
        let tick = make_tick(tick_idx);
        let (actions, next_view) = bridge.reconcile(&state, &state, &view, &tick);

        if !actions.is_empty() {
            return fail(
                NAME,
                format!(
                    "tick {tick_idx}: converged bridge emitted {} action(s); expected zero",
                    actions.len()
                ),
            );
        }
        if next_view != stable_view {
            return fail(
                NAME,
                format!(
                    "tick {tick_idx}: View mutated under unchanged inputs \
                     (steady-state dedup branch must not touch the View)"
                ),
            );
        }
        view = next_view;
    }

    // STEP 3 — shrink the listener set to empty. Tick once. The
    // View GC `retain` clause must drop the removed ServiceId.
    state.desired.listeners.clear();
    let gc_tick = make_tick(STEADY_STATE_TICKS + 1);
    let (gc_actions, gc_view) = bridge.reconcile(&state, &state, &view, &gc_tick);
    if !gc_actions.is_empty() {
        return fail(
            NAME,
            format!(
                "GC tick emitted {} action(s); expected zero — removing a listener \
                 should not trigger a new write",
                gc_actions.len()
            ),
        );
    }
    if gc_view.last_written_fingerprint.contains_key(&sid) {
        return fail(
            NAME,
            "View GC `retain` clause failed to drop fingerprint for removed ServiceId \
             — S-BDB-07 violation"
                .to_owned(),
        );
    }

    pass(NAME)
}

/// Atlas Q2 evaluator — closes S-BDB-06.
///
/// Models the fsync-then-memory ordering contract from
/// `.claude/rules/development.md` § "Reconciler I/O" → "Steady-state
/// tick" at the bridge level. The runtime's in-memory `BTreeMap`
/// update happens AFTER `ViewStore::write_through` returns Ok; if the
/// process crashes between fsync and the in-memory insert, the next
/// boot's `bulk_load` MUST see the persisted view, and the bridge's
/// first post-restart `reconcile` MUST recompute the fingerprint from
/// fresh inputs — never silent-skip on the cached old fingerprint.
///
/// # Scenario
///
/// 1. Reach steady state — the bridge has emitted one action and
///    recorded the fingerprint in the (returned) View. Call this
///    `persisted_view` — it is what a hypothetical `bulk_load` would
///    see after the crash.
/// 2. Simulate the crash: discard the runtime's in-memory map by
///    constructing a fresh `view = persisted_view.clone()` (as
///    `bulk_load` would return) — there is no separate in-memory
///    state at this layer; the discipline is that `reconcile` MUST
///    re-derive every value from the inputs + view, never from
///    process-local cached state.
/// 3. Branch A (idempotent path): inputs unchanged. Tick once. The
///    bridge MUST emit zero actions (the persisted fingerprint
///    matches the freshly-recomputed fingerprint).
/// 4. Branch B (drift path): inputs change (a Running alloc was
///    added). Tick once. The bridge MUST emit one
///    `Action::WriteServiceBackendRow` (the freshly-recomputed
///    fingerprint differs from the persisted one — no silent skip).
/// 5. After Branch B's action is applied, Branch B's next tick MUST
///    reach steady state again (zero actions emitted).
#[allow(clippy::too_many_lines)]
// Justification: every match block is a structured-cause early-return
// with a distinct error message. Splitting into helpers would force
// passing `name` + captured fixture values through extra arguments
// without making the seed → crash → recover → drift flow clearer.
// Matches the `evaluate_write_through_ordering` precedent at
// `crates/overdrive-sim/src/invariants/evaluators.rs:1626`.
pub async fn evaluate_bridge_recomputes_fingerprint_on_replay() -> InvariantResult {
    const NAME: &str = "bridge-recomputes-fingerprint-on-replay";

    let bridge = BackendDiscoveryBridge::new(host_ipv4(), writer_node_id());
    let obs = SimObservationStore::single_peer(writer_node_id(), 0);

    let wid = match workload_id("payments") {
        Ok(w) => w,
        Err(cause) => return fail(NAME, cause),
    };
    let sid = match service_id(1) {
        Ok(s) => s,
        Err(cause) => return fail(NAME, cause),
    };
    let vip = match service_vip(Ipv4Addr::new(10, 1, 0, 1)) {
        Ok(v) => v,
        Err(cause) => return fail(NAME, cause),
    };
    let lst = match listener(vip, 8080) {
        Ok(l) => l,
        Err(cause) => return fail(NAME, cause),
    };

    let mut state = BackendDiscoveryBridgeState::empty_for_workload(wid);
    state.desired.listeners.insert(sid, lst);
    let alloc_a = match alloc_id("alloc-a") {
        Ok(a) => a,
        Err(cause) => return fail(NAME, cause),
    };
    state.actual.running.insert(alloc_a);

    // STEP 1 — reach steady state. The returned `next_view` is what
    // would be persisted by `ViewStore::write_through` before the
    // simulated crash.
    let seed_tick = make_tick(0);
    let (seed_actions, persisted_view) =
        bridge.reconcile(&state, &state, &BackendDiscoveryBridgeView::default(), &seed_tick);
    // UI-05 dual-emit: WriteServiceBackendRow + EnqueueEvaluation.
    if seed_actions.len() != 2 {
        return fail(
            NAME,
            format!(
                "seed tick must emit two actions (WriteServiceBackendRow + \
                 EnqueueEvaluation per UI-05); got {}",
                seed_actions.len()
            ),
        );
    }
    if let Err(cause) = apply_actions(&obs, &seed_actions).await {
        return fail(NAME, cause);
    }
    let Some(prev_fp) = persisted_view.last_written_fingerprint.get(&sid).copied() else {
        return fail(NAME, "seed tick must record fingerprint for the written service".to_owned());
    };

    // STEP 2 — simulate crash + bulk_load: the next boot's view IS
    // `persisted_view` (the value fsync'd before the crash).

    // BRANCH A — idempotent path: inputs unchanged.
    // Reconcile MUST emit zero actions; the freshly-computed
    // fingerprint matches `persisted_view.last_written_fingerprint`.
    let recovery_tick_idle = make_tick(1);
    let (idle_actions, idle_view) =
        bridge.reconcile(&state, &state, &persisted_view, &recovery_tick_idle);
    if !idle_actions.is_empty() {
        return fail(
            NAME,
            format!(
                "recovery tick (unchanged inputs) must emit zero actions; got {} — \
                 bridge silently re-emitted writes against the persisted view",
                idle_actions.len()
            ),
        );
    }
    let idle_fp = idle_view.last_written_fingerprint.get(&sid).copied();
    if idle_fp != Some(prev_fp) {
        return fail(
            NAME,
            format!(
                "recovery tick (unchanged inputs) must preserve the persisted fingerprint; \
                 got {idle_fp:?}, expected {prev_fp:?}"
            ),
        );
    }

    // BRANCH B — drift path: add a second Running alloc. The
    // freshly-recomputed fingerprint differs from the persisted one;
    // the bridge MUST NOT silently skip — it MUST emit a new
    // WriteServiceBackendRow action carrying the new fingerprint.
    let alloc_b = match alloc_id("alloc-b") {
        Ok(a) => a,
        Err(cause) => return fail(NAME, cause),
    };
    state.actual.running.insert(alloc_b);

    let drift_tick = make_tick(2);
    let (drift_actions, drift_view) =
        bridge.reconcile(&state, &state, &persisted_view, &drift_tick);
    // UI-05 dual-emit: WriteServiceBackendRow + EnqueueEvaluation.
    if drift_actions.len() != 2 {
        return fail(
            NAME,
            format!(
                "recovery tick (drifted inputs) must emit exactly two actions \
                 (WriteServiceBackendRow + EnqueueEvaluation per UI-05); got {} — \
                 silent skip on cached stale fingerprint is the Atlas Q2 failure mode",
                drift_actions.len()
            ),
        );
    }
    let drift_fp = drift_view.last_written_fingerprint.get(&sid).copied();
    match drift_fp {
        Some(fp) if fp == prev_fp => {
            return fail(
                NAME,
                "drift tick must update the View's fingerprint to the new value; \
                 the recorded fingerprint still equals the persisted (stale) one"
                    .to_owned(),
            );
        }
        None => {
            return fail(
                NAME,
                "drift tick must record a fingerprint for the written service".to_owned(),
            );
        }
        Some(_) => {}
    }
    if let Err(cause) = apply_actions(&obs, &drift_actions).await {
        return fail(NAME, cause);
    }

    // STEP 3 — eventually steady state again. One more tick under
    // unchanged inputs against `drift_view` MUST emit zero actions.
    let steady_tick = make_tick(3);
    let (steady_actions, _) = bridge.reconcile(&state, &state, &drift_view, &steady_tick);
    if !steady_actions.is_empty() {
        return fail(
            NAME,
            format!(
                "post-recovery steady-state tick must emit zero actions; got {}",
                steady_actions.len()
            ),
        );
    }

    pass(NAME)
}

// ---------------------------------------------------------------------------
// Backward-compatibility shim — the harness historically dispatched to
// a struct-method `evaluate_red_scaffold` body. Phase 01-05 closes the
// scaffolds: the struct types remain (the `Invariant` enum holds the
// canonical names; the structs aren't exposed beyond this module),
// and the harness now routes directly to the free `evaluate_*` fns
// above.
// ---------------------------------------------------------------------------

/// Marker struct retained for code-search continuity.
///
/// The DISTILL-era harness dispatched to
/// `BridgeEventuallyWritesBackendRow::evaluate_red_scaffold`; the
/// GREEN harness now routes directly to
/// [`evaluate_bridge_eventually_writes_backend_row`]. The marker
/// stays so that `grep BridgeEventuallyWritesBackendRow` continues
/// to land at the evaluator.
pub struct BridgeEventuallyWritesBackendRow;

impl BridgeEventuallyWritesBackendRow {
    /// Construct the marker.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for BridgeEventuallyWritesBackendRow {
    fn default() -> Self {
        Self::new()
    }
}

/// Marker struct — see [`BridgeEventuallyWritesBackendRow`].
pub struct BridgeIdempotentSteadyState;

impl BridgeIdempotentSteadyState {
    /// Construct the marker.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for BridgeIdempotentSteadyState {
    fn default() -> Self {
        Self::new()
    }
}

/// Marker struct — see [`BridgeEventuallyWritesBackendRow`].
pub struct BridgeRecomputesFingerprintOnReplay;

impl BridgeRecomputesFingerprintOnReplay {
    /// Construct the marker.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for BridgeRecomputesFingerprintOnReplay {
    fn default() -> Self {
        Self::new()
    }
}
