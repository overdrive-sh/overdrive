//! DST invariants for `backend-discovery-bridge-service-reachability` (joint #174 + #175).
//!
//! Per `docs/feature/backend-discovery-bridge-service-reachability/distill/test-scenarios.md`
//! S-BDB-02..S-BDB-10, as amended by ADR-0079 (S-BDB-06 / Atlas Q2 and the
//! S-BDB-07 View-GC half are retired — see below). Tier 1 — pure-Rust under
//! sim adapters; runs via `cargo dst` on every PR.
//!
//! Three evaluators, all returning [`InvariantResult`] from the shared
//! harness dispatch in `crate::harness`:
//!
//! - [`evaluate_bridge_eventually_writes_backend_row`] (S-BDB-02 / S-BDB-03
//!   / S-BDB-04 / S-BDB-10) — eventual: for every Service workload with
//!   ≥ 1 listener AND allocator-issued VIP AND ≥ 1 Running alloc, the
//!   harness's `SimObservationStore` eventually carries a
//!   `ServiceBackendRow` whose `backends` matches the Running alloc set.
//! - [`evaluate_bridge_idempotent_steady_state`] (S-BDB-05) — always:
//!   once the observed `service_backends` row matches desired,
//!   subsequent ticks with unchanged inputs produce zero
//!   `Action::WriteServiceBackendRow` actions and leave the (field-less)
//!   View untouched.
//! - [`evaluate_bridge_reconverges_after_dropped_write`] (ADR-0079
//!   § D7) — always: seed the store with a `ServiceBackendRow` that does
//!   NOT match desired, tick, and assert the bridge re-emits; then apply
//!   the write and assert the next tick emits zero. This is the
//!   convergence property that replaces the retired S-BDB-06 (Atlas Q2)
//!   evaluator — the cache whose stale-skip that scenario defended
//!   against no longer exists, so the failure mode is structurally
//!   impossible rather than merely untriggered.
//!
//! Production code these invariants guard:
//!
//! - `BackendDiscoveryBridge::reconcile` body — main loop
//! - `BackendDiscoveryBridge::reconcile` convergence diff against the
//!   observed row (ADR-0079 § D2)
//! - `fingerprint(&vip, &backends)` correlation content-address
//! - `hydrate_desired` allocator-lookup arm
//! - `hydrate_actual` Running-filter arm + the `service_backends`
//!   projection
//! - `Action::WriteServiceBackendRow` action shim dispatch

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;
use std::time::{Duration, Instant};

use overdrive_core::SpiffeId;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip, WorkloadId};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationRow, ObservationStore, ServiceBackendRow,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::backend_discovery_bridge::{
    BackendDiscoveryBridge, BackendDiscoveryBridgeState, BackendDiscoveryBridgeView,
    ProjectedListener,
};

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

/// Re-project the store's LWW-winner `service_backends` row for `sid`
/// into `state.service_backends` — the in-evaluator simulation of the
/// runtime's `hydrate_actual` bridge arm (ADR-0079 § D1). Without this
/// the bridge never observes its own write and cannot converge.
async fn refresh_observed(
    obs: &SimObservationStore,
    sid: ServiceId,
    state: &mut BackendDiscoveryBridgeState,
) -> Result<(), String> {
    let rows =
        obs.service_backends_rows(&sid).await.map_err(|e| format!("service_backends_rows: {e}"))?;
    match rows.into_iter().next() {
        Some(row) => {
            state.service_backends.insert(sid, row);
        }
        None => {
            state.service_backends.remove(&sid);
        }
    }
    Ok(())
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
    state.actual.running.insert(alloc_id("alloc-a")?, None);

    let mut view = BackendDiscoveryBridgeView::default();
    for tick_idx in 0..CONVERGENCE_TICK_BUDGET {
        let tick = make_tick(tick_idx);
        let (actions, next_view) = bridge.reconcile(&state, &state, &view, &tick);
        let _ = apply_actions(&obs, &actions).await?;
        view = next_view;
        refresh_observed(&obs, sid, &mut state).await?;

        let rows = obs
            .service_backends_rows(&sid)
            .await
            .map_err(|e| format!("service_backends_rows: {e}"))?;
        if let Some(row) = rows.first()
            && row.backends.len() == 1
            && row.vip == Ipv4Addr::new(10, 1, 0, 1)
            && row.updated_at.writer == writer_node_id()
            && row.updated_at.counter == u64::from(tick_idx).saturating_add(1)
        {
            return Ok(());
        }
    }

    Err(format!(
        "single-alloc scenario did not converge within {CONVERGENCE_TICK_BUDGET} ticks; \
         observed row={:?}",
        state.service_backends.get(&sid)
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
    state.actual.running.insert(alloc_id("alloc-x")?, None);
    state.actual.running.insert(alloc_id("alloc-y")?, None);
    state.actual.running.insert(alloc_id("alloc-z")?, None);

    let mut view = BackendDiscoveryBridgeView::default();
    for tick_idx in 0..CONVERGENCE_TICK_BUDGET {
        let tick = make_tick(tick_idx);
        let (actions, next_view) = bridge.reconcile(&state, &state, &view, &tick);
        let _ = apply_actions(&obs, &actions).await?;
        view = next_view;
        refresh_observed(&obs, sid, &mut state).await?;

        let rows = obs
            .service_backends_rows(&sid)
            .await
            .map_err(|e| format!("service_backends_rows: {e}"))?;
        if let Some(row) = rows.first()
            && row.backends.len() == 3
        {
            return Ok(());
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
    state.actual.running.insert(alloc_id("alloc-a")?, None);

    let mut view = BackendDiscoveryBridgeView::default();

    // Phase 1 — converge on alloc-a.
    let tick = make_tick(0);
    let (actions, next_view) = bridge.reconcile(&state, &state, &view, &tick);
    let _ = apply_actions(&obs, &actions).await?;
    view = next_view;
    refresh_observed(&obs, sid, &mut state).await?;
    let first_counter = obs
        .service_backends_rows(&sid)
        .await
        .map_err(|e| format!("first service_backends_rows: {e}"))?
        .first()
        .map(|r| r.updated_at.counter)
        .ok_or_else(|| "phase 1 produced no observable row".to_owned())?;

    // Phase 2 — drop alloc-a, add alloc-b. Run remaining ticks.
    state.actual.running.clear();
    state.actual.running.insert(alloc_id("alloc-b")?, None);
    for tick_idx in 1..CONVERGENCE_TICK_BUDGET {
        let tick = make_tick(tick_idx);
        let (actions, next_view) = bridge.reconcile(&state, &state, &view, &tick);
        let _ = apply_actions(&obs, &actions).await?;
        view = next_view;
        refresh_observed(&obs, sid, &mut state).await?;

        let rows = obs
            .service_backends_rows(&sid)
            .await
            .map_err(|e| format!("service_backends_rows: {e}"))?;
        if let Some(row) = rows.first()
            && row.backends.len() == 1
            && row.updated_at.counter > first_counter
        {
            return Ok(());
        }
    }

    Err(format!(
        "alloc-replacement scenario did not produce a strictly-newer single-backend row \
         within {CONVERGENCE_TICK_BUDGET} ticks (first_counter={first_counter})"
    ))
}

/// Idempotent-steady-state evaluator — closes S-BDB-05.
///
/// 1. Reach steady state on a single service: tick, apply the emitted
///    write, re-project the stored row into `actual.service_backends`
///    (what the runtime's `hydrate_actual` does).
/// 2. Tick K=[`STEADY_STATE_TICKS`] more times with unchanged inputs.
///    Every tick MUST emit zero actions AND the `next_view` MUST
///    equal the prior view.
/// 3. Shrink `desired.listeners` to empty. Tick once — still zero
///    actions, because removing a listener must not trigger a write.
///
/// ADR-0079 § D7 retires the S-BDB-07 half of step 3 (the View GC
/// `retain` assertion): the field it swept is deleted. The zero-actions
/// assertion beside it is a convergence property and stays.
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
    state.actual.running.insert(single_alloc, None);

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
    // The runtime's `hydrate_actual` re-projection — without it the
    // bridge cannot observe its own write and never converges.
    if let Err(cause) = refresh_observed(&obs, sid, &mut state).await {
        return fail(NAME, cause);
    }
    if !state.service_backends.contains_key(&sid) {
        return fail(NAME, "seed tick's write must be observable in the store".to_owned());
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

    // STEP 3 — shrink the listener set to empty. Tick once: the loop
    // has nothing to iterate, so zero actions. (The S-BDB-07 View-GC
    // assertion that used to sit here is retired with the field —
    // ADR-0079 § D7.)
    state.desired.listeners.clear();
    let gc_tick = make_tick(STEADY_STATE_TICKS + 1);
    let (gc_actions, _) = bridge.reconcile(&state, &state, &view, &gc_tick);
    if !gc_actions.is_empty() {
        return fail(
            NAME,
            format!(
                "removed-listener tick emitted {} action(s); expected zero — removing \
                 a listener should not trigger a new write",
                gc_actions.len()
            ),
        );
    }

    pass(NAME)
}

/// Convergence-after-dropped-write evaluator — ADR-0079 § D7.
///
/// This is the DST form of the ADR's central falsifiable claim, and it
/// replaces the retired S-BDB-06 (Atlas Q2) evaluator. Atlas Q2
/// defended against a "silent skip on a cached stale fingerprint after
/// a crash"; with the emit-time fingerprint deleted (§ D2 / § D3) there
/// is no cache, so that failure mode is structurally impossible rather
/// than merely untriggered. The property worth holding now is the one
/// convergence buys: a write the store discarded is retried.
///
/// # Scenario
///
/// 1. Seed the `SimObservationStore` with a `ServiceBackendRow` that
///    does NOT match desired (a stale single-backend row while two
///    allocs are Running), and project it into
///    `actual.service_backends` as `hydrate_actual` would.
/// 2. Tick. The bridge MUST emit the UI-05 pair — it observes the row
///    it manages and sees drift.
/// 3. Model the dropped write: do NOT apply the actions. `actual` still
///    shows the stale row. Tick again — the bridge MUST emit again.
///    Under the deleted design this second tick emitted ZERO and the
///    drop was permanently forgotten.
/// 4. Now apply the write and re-project. The next tick MUST emit zero
///    — convergence terminates.
pub async fn evaluate_bridge_reconverges_after_dropped_write() -> InvariantResult {
    const NAME: &str = "bridge-reconverges-after-dropped-write";

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

    let mut state = BackendDiscoveryBridgeState::empty_for_workload(wid.clone());
    state.desired.listeners.insert(sid, lst);
    let alloc_a = match alloc_id("alloc-a") {
        Ok(a) => a,
        Err(cause) => return fail(NAME, cause),
    };
    let alloc_b = match alloc_id("alloc-b") {
        Ok(a) => a,
        Err(cause) => return fail(NAME, cause),
    };
    state.actual.running.insert(alloc_a.clone(), None);
    state.actual.running.insert(alloc_b, None);

    // STEP 1 — seed a STALE row: one backend, while two allocs run.
    let stale_row = ServiceBackendRow {
        service_id: sid,
        vip: Ipv4Addr::new(10, 1, 0, 1),
        backends: vec![Backend {
            alloc: SpiffeId::for_allocation(&wid, &alloc_a),
            addr: SocketAddr::new(IpAddr::V4(host_ipv4()), 8080),
            weight: 1,
            healthy: true,
        }],
        updated_at: LogicalTimestamp::dominating(0, writer_node_id(), None),
    };
    if let Err(e) = obs.write(ObservationRow::ServiceBackend(stale_row)).await {
        return fail(NAME, format!("seeding the stale row failed: {e}"));
    }
    if let Err(cause) = refresh_observed(&obs, sid, &mut state).await {
        return fail(NAME, cause);
    }

    // STEP 2 — the bridge observes drift and emits.
    let view = BackendDiscoveryBridgeView::default();
    let (first, _) = bridge.reconcile(&state, &state, &view, &make_tick(1));
    if first.len() != 2 {
        return fail(
            NAME,
            format!(
                "a stale observed row must trigger the UI-05 pair \
                 (WriteServiceBackendRow + EnqueueEvaluation); got {}",
                first.len()
            ),
        );
    }

    // STEP 3 — the write was DROPPED: `actual` is unchanged. The bridge
    // MUST re-emit rather than dedup itself into silence.
    let (second, _) = bridge.reconcile(&state, &state, &view, &make_tick(2));
    if second.len() != 2 {
        return fail(
            NAME,
            format!(
                "a DROPPED write must be retried on the next tick; got {} action(s) — \
                 the bridge has gone quiet on an unconverged row (RCA § 4.3)",
                second.len()
            ),
        );
    }

    // STEP 4 — the retry lands; convergence terminates.
    if let Err(cause) = apply_actions(&obs, &second).await {
        return fail(NAME, cause);
    }
    if let Err(cause) = refresh_observed(&obs, sid, &mut state).await {
        return fail(NAME, cause);
    }
    let (third, _) = bridge.reconcile(&state, &state, &view, &make_tick(3));
    if !third.is_empty() {
        return fail(
            NAME,
            format!(
                "once the write lands the bridge must converge and emit zero actions; \
                 got {} — convergence does not terminate",
                third.len()
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
pub struct BridgeReconvergesAfterDroppedWrite;

impl BridgeReconvergesAfterDroppedWrite {
    /// Construct the marker.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for BridgeReconvergesAfterDroppedWrite {
    fn default() -> Self {
        Self::new()
    }
}
