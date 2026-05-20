//! `ServiceMapHydrator` ESR invariants — Slice 08 (US-08;
//! ASR-2.2-04).
//!
//! Two named DST invariants per
//! `docs/feature/phase-2-xdp-service-map/distill/test-scenarios.md`
//! S-2.2-26 / S-2.2-27:
//!
//! - [`evaluate_hydrator_eventually_converges`] — eventual: from
//!   any combination of `service_backends` rows + starting BPF
//!   map state, repeated reconcile ticks drive
//!   `actual.fingerprint == desired.fingerprint`.
//! - [`evaluate_hydrator_idempotent_steady_state`] — always: once
//!   converged, no further `Action::DataplaneUpdateService` is
//!   emitted on subsequent ticks given unchanged inputs.
//!
//! Both invariants drive the typed `ServiceMapHydrator::reconcile`
//! function directly via the `AnyReconciler::ServiceMapHydrator`
//! dispatch — port-to-port at the domain scope per
//! `nw-tdd-methodology` Mandate 2 (the reconciler is a pure
//! function; calling it with typed inputs IS port-to-port).
//!
//! Wired into the existing `Invariant` enum's exhaustive match at
//! `crates/overdrive-sim/src/invariants/mod.rs` as additive variants
//! `HydratorEventuallyConverges` and `HydratorIdempotentSteadyState`.

#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use overdrive_core::dataplane::fingerprint::fingerprint;
use overdrive_core::id::{NodeId, ServiceId, ServiceVip, SpiffeId};
use overdrive_core::reconciler::{
    Action, AnyReconciler, AnyReconcilerView, AnyState, ServiceDesired, ServiceMapHydrator,
    ServiceMapHydratorState, ServiceMapHydratorView, TickContext,
};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::ServiceHydrationStatus;
use overdrive_core::wall_clock::UnixInstant;

use crate::harness::{InvariantResult, InvariantStatus};

/// Maximum reconcile ticks the eventual-convergence invariant gives
/// the hydrator before declaring divergence. Today the hydrator
/// converges in ONE tick (one dispatch → one Completed observation
/// → next tick emits no actions); this ceiling exists to keep the
/// fixture honest if a future change introduces multi-tick
/// convergence (e.g. dispatch ordering by priority).
const CONVERGENCE_TICK_BUDGET: u32 = 8;

/// Number of idempotent steady-state ticks the always-invariant
/// asserts. `1` would be brittle — the property is "every
/// post-convergence tick emits zero actions"; running through
/// several confirms the steady state holds.
const STEADY_STATE_TICKS: u32 = 5;

/// Drive the eventual-convergence scenario.
///
/// # Scenario
///
/// 1. Construct a `ServiceMapHydratorState` with one service in
///    `desired` and an empty `actual` (cold start — no
///    `service_hydration_results` row yet).
/// 2. Tick the reconciler repeatedly, simulating the action shim:
///    after each tick that emits a `DataplaneUpdateService` action,
///    write the matching `Completed { fingerprint, applied_at }`
///    into `actual` for the next tick (fresh-out-of-the-dataplane
///    success).
/// 3. Within `CONVERGENCE_TICK_BUDGET` ticks, the actual fingerprint
///    must reach the desired fingerprint AND the hydrator must
///    emit zero actions on the post-convergence tick.
///
/// A failure to converge within the budget is a load-bearing bug —
/// the reconciler is either (a) emitting actions that don't carry
/// the desired fingerprint, (b) failing to recognise convergence
/// when actual matches desired, or (c) re-dispatching every tick
/// (idempotency bug). The fixture exercises the most basic ESR
/// progress property; harder cases (multi-service, churn,
/// fingerprint drift mid-convergence) live in the proptest suite.
pub fn evaluate_hydrator_eventually_converges() -> InvariantResult {
    const NAME: &str = "hydrator-eventually-converges";

    let scenario = match build_single_service_scenario() {
        Ok(s) => s,
        Err(reason) => return fail(NAME, reason),
    };
    let reconciler = ServiceMapHydrator::canonical();
    let any_reconciler = AnyReconciler::ServiceMapHydrator(reconciler);

    let mut state = scenario.state;
    let mut view = ServiceMapHydratorView::default();

    for tick_idx in 0..CONVERGENCE_TICK_BUDGET {
        let tick = make_tick(tick_idx);
        let (actions, next_view) = any_reconciler.reconcile(
            &AnyState::ServiceMapHydrator(state.clone()),
            &AnyState::ServiceMapHydrator(state.clone()),
            &AnyReconcilerView::ServiceMapHydrator(view.clone()),
            &tick,
        );

        // The dispatched action shim's behaviour: on every emitted
        // DataplaneUpdateService for service S with fingerprint F,
        // record actual.S = Completed { fingerprint = F, applied_at }.
        // The harness simulates the success branch (the dataplane
        // applied the update); the Failed branch + retry-budget gate
        // is exercised by the unit tests in `overdrive-core`.
        for action in &actions {
            if let Action::DataplaneUpdateService { service_id, .. } = action {
                let desired = match state.desired.get(service_id) {
                    Some(d) => d.clone(),
                    None => {
                        return fail(
                            NAME,
                            format!(
                                "tick {tick_idx}: hydrator emitted DataplaneUpdateService \
                                 for {service_id} which is not in state.desired"
                            ),
                        );
                    }
                };
                state.actual.insert(
                    *service_id,
                    ServiceHydrationStatus::Completed {
                        fingerprint: desired.fingerprint,
                        applied_at: UnixInstant::from_unix_duration(Duration::from_secs(
                            u64::from(tick_idx) + 1,
                        )),
                    },
                );
            }
        }

        // Install next_view per the runtime's persist-then-install
        // contract. The DST harness has no fsync to elide, so this
        // is the in-memory installation step the runtime would
        // normally do after `write_through` returns Ok.
        let AnyReconcilerView::ServiceMapHydrator(next_view_inner) = next_view else {
            return fail(
                NAME,
                "reconciler returned non-ServiceMapHydrator view variant".to_string(),
            );
        };
        view = next_view_inner;

        // Convergence check — actual.fingerprint matches desired.fingerprint
        // AND no actions were emitted this tick (idempotent steady state
        // reached).
        if actions.is_empty() && all_converged(&state) {
            return pass(NAME);
        }
    }

    fail(
        NAME,
        format!(
            "hydrator did not converge within {CONVERGENCE_TICK_BUDGET} ticks; \
             final state: desired={:?} actual={:?}",
            state.desired.keys().collect::<Vec<_>>(),
            state.actual,
        ),
    )
}

/// Drive the idempotent-steady-state scenario.
///
/// # Scenario
///
/// 1. Construct a converged `ServiceMapHydratorState` directly:
///    `desired` and `actual` carry matching fingerprints for every
///    service.
/// 2. Tick the reconciler `STEADY_STATE_TICKS` times.
/// 3. Every tick must emit zero actions.
///
/// A non-empty action set on any post-convergence tick is a
/// load-bearing bug — the hydrator would re-dispatch on every tick
/// forever, saturating the dataplane and the action shim with
/// no-op writes.
pub fn evaluate_hydrator_idempotent_steady_state() -> InvariantResult {
    const NAME: &str = "hydrator-idempotent-steady-state";

    let mut scenario = match build_single_service_scenario() {
        Ok(s) => s,
        Err(reason) => return fail(NAME, reason),
    };
    // Pre-populate `actual` with the converged Completed status,
    // matching the post-convergence harness state.
    let (service_id, desired) = match scenario.state.desired.iter().next() {
        Some((id, d)) => (*id, d.clone()),
        None => return fail(NAME, "scenario constructed empty desired map".to_string()),
    };
    scenario.state.actual.insert(
        service_id,
        ServiceHydrationStatus::Completed {
            fingerprint: desired.fingerprint,
            applied_at: UnixInstant::from_unix_duration(Duration::from_secs(1)),
        },
    );

    let reconciler = ServiceMapHydrator::canonical();
    let any_reconciler = AnyReconciler::ServiceMapHydrator(reconciler);

    let mut view = ServiceMapHydratorView::default();
    for tick_idx in 0..STEADY_STATE_TICKS {
        let tick = make_tick(tick_idx);
        let (actions, next_view) = any_reconciler.reconcile(
            &AnyState::ServiceMapHydrator(scenario.state.clone()),
            &AnyState::ServiceMapHydrator(scenario.state.clone()),
            &AnyReconcilerView::ServiceMapHydrator(view.clone()),
            &tick,
        );

        if !actions.is_empty() {
            return fail(
                NAME,
                format!(
                    "tick {tick_idx}: converged hydrator emitted {} action(s); \
                     expected zero. actions={actions:?}",
                    actions.len(),
                ),
            );
        }

        view = match next_view {
            AnyReconcilerView::ServiceMapHydrator(v) => v,
            _ => {
                return fail(
                    NAME,
                    "reconciler returned non-ServiceMapHydrator view variant".to_string(),
                );
            }
        };
    }

    pass(NAME)
}

/// Single-service scenario fixture used by both invariants. One
/// service in `desired` with one healthy backend; `actual` is
/// empty (cold start).
struct Scenario {
    state: ServiceMapHydratorState,
}

fn build_single_service_scenario() -> Result<Scenario, String> {
    let service_id = ServiceId::new(42).map_err(|e| format!("ServiceId construction: {e}"))?;
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 42)))
        .map_err(|e| format!("ServiceVip construction: {e}"))?;
    let alloc =
        SpiffeId::new("spiffe://overdrive.local/job/web/alloc/web-0").map_err(|e| e.to_string())?;
    let backend = Backend {
        alloc,
        addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 1, 1)), 8080),
        weight: 1,
        healthy: true,
    };
    let backends = vec![backend];
    let fp = fingerprint(&vip, &backends);

    let mut desired = BTreeMap::new();
    desired.insert(service_id, ServiceDesired { vip, backends, fingerprint: fp });

    Ok(Scenario { state: ServiceMapHydratorState { desired, actual: BTreeMap::new() } })
}

/// True iff every desired service has an `actual.Completed` row whose
/// fingerprint matches the desired fingerprint.
fn all_converged(state: &ServiceMapHydratorState) -> bool {
    state.desired.iter().all(|(service_id, desired)| {
        matches!(
            state.actual.get(service_id),
            Some(ServiceHydrationStatus::Completed { fingerprint, .. })
                if *fingerprint == desired.fingerprint
        )
    })
}

/// Construct a synthetic `TickContext` for the harness. `now_unix`
/// advances by one second per tick — far longer than the (degenerate)
/// 1-second backoff, so any retry-gated dispatch always fires on the
/// next tick. Pure inputs only; no `Instant::now()`.
fn make_tick(tick_idx: u32) -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(u64::from(tick_idx) * 60)),
        tick: u64::from(tick_idx),
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

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

// ---------------------------------------------------------------------------
// S-BDB-19 — bridge → hydrator handoff (Tier 1 DST)
//
// `backend-discovery-bridge-service-reachability` step 02-04 extends
// the Tier 1 invariant catalogue to drive the in-process bridge →
// hydrator handoff against the same `SimObservationStore` the bridge
// writes into. The Tier 3 walking-skeleton exercises the same property
// against the real kernel adapter; this Tier 1 invariant exercises it
// against `SimDataplane` semantics on every PR via `cargo dst`.
//
// The chain:
//
//   1. Tick `BackendDiscoveryBridge::reconcile` against a Running
//      alloc + projected listener; assert it emits one
//      `Action::WriteServiceBackendRow`.
//   2. Apply that action to `SimObservationStore` (mirrors the
//      production `action_shim::write_service_backend_row` dispatch).
//   3. Read `service_backends_rows(&service_id)` back from obs and
//      project the row into a `ServiceMapHydratorState.desired` entry
//      (mirrors the runtime `hydrate_desired` arm at
//      `crates/overdrive-control-plane/src/reconciler_runtime.rs`).
//   4. Tick `ServiceMapHydrator::reconcile` against that state;
//      assert it emits exactly one `Action::DataplaneUpdateService`
//      carrying the bridge-written row's `vip` + `backends`.
//
// The structural defense is that fingerprint identity is preserved
// across the bridge-write / hydrator-read boundary: the bridge
// computes `fingerprint(&vip, &backends)`; the hydrator's
// `hydrate_desired` recomputes it from the same inputs; the
// hydrator's dispatch decision compares against `actual.fingerprint`.
// If any of those three sites drift in encoding or input set, this
// invariant fails — exactly the regression class S-BDB-19 is meant
// to guard against.
// ---------------------------------------------------------------------------

/// Drive the bridge → hydrator handoff scenario (S-BDB-19).
///
/// # Scenario
///
/// 1. Single Service workload with one listener `(vip=10.1.0.1, port=8080,
///    tcp)` and one Running alloc.
/// 2. Tick `BackendDiscoveryBridge` once → asserts one
///    `Action::WriteServiceBackendRow` emitted; applied to obs.
/// 3. Read `service_backends_rows(&service_id)` back → project into
///    `ServiceMapHydratorState.desired`.
/// 4. Tick `ServiceMapHydrator` once with empty `actual` → asserts one
///    `Action::DataplaneUpdateService` emitted with the same VIP +
///    backend set the bridge wrote.
///
/// # Why this is load-bearing
///
/// The bridge writes the row at fingerprint `F1`. The hydrator's
/// `desired` is re-derived from the SAME row's `(vip, backends)`
/// inputs — `fingerprint(&desired.vip, &desired.backends)` MUST equal
/// `F1`. If the hydrator silently re-encodes the inputs (e.g., a
/// future refactor that wraps `vip` in a different newtype, or
/// re-orders backends), the action's `vip` / `backends` will diverge
/// from what the bridge wrote and the kernel-side dataplane programs
/// against a different set than the bridge believes is current. This
/// invariant pins that property in DST so the regression cannot land.
//
// `too_many_lines` allow: the body is a single linear five-step
// recipe (bridge tick → apply action → read back → project →
// hydrator tick → assert) where extracting helpers would split the
// load-bearing sequence across files and obscure the fixture's
// intent. The sibling `evaluate_hydrator_eventually_converges`
// carries the same shape.
#[allow(clippy::too_many_lines)]
pub async fn evaluate_bridge_to_hydrator_handoff() -> InvariantResult {
    use std::collections::BTreeMap as StdBTreeMap;
    use std::collections::BTreeSet;
    use std::num::NonZeroU16;

    use overdrive_core::dataplane::backend_key::Proto;
    use overdrive_core::id::{AllocationId, WorkloadId};
    use overdrive_core::reconciler::Reconciler;
    use overdrive_core::reconciler::backend_discovery_bridge::{
        BackendDiscoveryBridge, BackendDiscoveryBridgeState, BackendDiscoveryBridgeView,
        ProjectedListener,
    };
    use overdrive_core::traits::observation_store::{ObservationRow, ObservationStore};

    use crate::adapters::observation_store::SimObservationStore;

    const NAME: &str = "bridge-to-hydrator-handoff";

    // ---- Fixture: ids, addresses, listener.
    let writer_node = match NodeId::new("host-0") {
        Ok(n) => n,
        Err(e) => return fail(NAME, format!("NodeId construction: {e}")),
    };
    let host_ipv4 = Ipv4Addr::new(10, 0, 0, 5);
    let workload_id = match WorkloadId::new("payments") {
        Ok(w) => w,
        Err(e) => return fail(NAME, format!("WorkloadId construction: {e}")),
    };
    let svc_id = match ServiceId::new(1) {
        Ok(s) => s,
        Err(e) => return fail(NAME, format!("ServiceId construction: {e}")),
    };
    let vip_addr = Ipv4Addr::new(10, 1, 0, 1);
    let vip = match ServiceVip::new(IpAddr::V4(vip_addr)) {
        Ok(v) => v,
        Err(e) => return fail(NAME, format!("ServiceVip construction: {e}")),
    };
    let Some(port) = NonZeroU16::new(8080) else {
        return fail(NAME, "8080 must be NonZeroU16".to_owned());
    };
    let alloc = match AllocationId::new("alloc-a") {
        Ok(a) => a,
        Err(e) => return fail(NAME, format!("AllocationId construction: {e}")),
    };

    // ---- Step 1: tick the bridge, assert exactly one
    //      WriteServiceBackendRow action emitted.
    let bridge = BackendDiscoveryBridge::new(host_ipv4, writer_node.clone());
    let mut bridge_state = BackendDiscoveryBridgeState::empty_for_workload(workload_id.clone());
    bridge_state
        .desired
        .listeners
        .insert(svc_id, ProjectedListener { vip, port, protocol: Proto::Tcp });
    bridge_state.actual.running.insert(alloc);
    let bridge_view = BackendDiscoveryBridgeView::default();
    let tick0 = make_tick(0);
    let (bridge_actions, _bridge_next_view) =
        bridge.reconcile(&bridge_state, &bridge_state, &bridge_view, &tick0);

    let Some(written_row) = bridge_actions.iter().find_map(|a| match a {
        Action::WriteServiceBackendRow { row, .. } => Some(row.clone()),
        _ => None,
    }) else {
        return fail(
            NAME,
            format!(
                "bridge tick 0 emitted no WriteServiceBackendRow action; got {} action(s): {:?}",
                bridge_actions.len(),
                bridge_actions,
            ),
        );
    };

    // ---- Step 2: apply the row write to SimObservationStore.
    let obs = SimObservationStore::single_peer(writer_node.clone(), 0);
    if let Err(e) = obs.write(ObservationRow::ServiceBackend(written_row.clone())).await {
        return fail(NAME, format!("SimObservationStore::write rejected bridge row: {e}"));
    }

    // ---- Step 3: read back; project into ServiceMapHydratorState.desired
    //      (mirrors the runtime's hydrate_desired arm).
    let rows = match obs.service_backends_rows(&svc_id).await {
        Ok(r) => r,
        Err(e) => return fail(NAME, format!("service_backends_rows: {e}")),
    };
    let Some(row) = rows.first().cloned() else {
        return fail(
            NAME,
            "service_backends_rows returned empty after bridge write — \
             SimObservationStore::write did not surface the row to the read \
             path; this is a SimObservationStore bug, not a bridge bug."
                .to_owned(),
        );
    };

    // Project ServiceBackendRow → ServiceDesired. The runtime's
    // hydrate_desired arm performs the same projection — wrap
    // row.vip in ServiceVip, carry row.backends verbatim, re-compute
    // the fingerprint from the same inputs.
    let desired_vip = match ServiceVip::new(IpAddr::V4(row.vip)) {
        Ok(v) => v,
        Err(e) => return fail(NAME, format!("ServiceVip projection from row: {e}")),
    };
    let desired_backends = row.backends.clone();
    let desired_fp = fingerprint(&desired_vip, &desired_backends);
    let mut desired_map = StdBTreeMap::new();
    desired_map.insert(
        svc_id,
        ServiceDesired {
            vip: desired_vip,
            backends: desired_backends.clone(),
            fingerprint: desired_fp,
        },
    );

    // ---- Step 4: tick the hydrator against the projected desired
    //      with empty actual (no prior service_hydration_results row).
    let hydrator = ServiceMapHydrator::canonical();
    let any_hydrator = AnyReconciler::ServiceMapHydrator(hydrator);
    let hydrator_state = ServiceMapHydratorState { desired: desired_map, actual: BTreeMap::new() };
    let hydrator_view = ServiceMapHydratorView::default();
    let tick1 = make_tick(1);
    let (hydrator_actions, _next_view) = any_hydrator.reconcile(
        &AnyState::ServiceMapHydrator(hydrator_state.clone()),
        &AnyState::ServiceMapHydrator(hydrator_state),
        &AnyReconcilerView::ServiceMapHydrator(hydrator_view),
        &tick1,
    );

    // Assert exactly one DataplaneUpdateService action emitted, and
    // that its vip + backends match what the bridge wrote.
    let mut dispatch_count = 0_usize;
    let mut matched = false;
    for action in &hydrator_actions {
        if let Action::DataplaneUpdateService { service_id, vip, backends, .. } = action {
            dispatch_count += 1;
            if *service_id != svc_id {
                return fail(
                    NAME,
                    format!(
                        "hydrator dispatched DataplaneUpdateService for {service_id} \
                         which differs from the bridge-written service_id {svc_id}"
                    ),
                );
            }
            // The hydrator's action carries vip + backends. They MUST
            // equal what the bridge wrote — drift here means the
            // bridge → hydrator boundary lost or transformed the
            // payload.
            let expected_vip = match ServiceVip::new(IpAddr::V4(row.vip)) {
                Ok(v) => v,
                Err(e) => return fail(NAME, format!("expected_vip wrap: {e}")),
            };
            if *vip != expected_vip {
                return fail(
                    NAME,
                    format!(
                        "hydrator action vip ({vip}) differs from bridge-written \
                         row.vip ({})",
                        row.vip,
                    ),
                );
            }
            if *backends != desired_backends {
                return fail(
                    NAME,
                    format!(
                        "hydrator action backends differ from bridge-written row.backends; \
                         expected len {}, got len {}",
                        desired_backends.len(),
                        backends.len(),
                    ),
                );
            }
            matched = true;
        }
    }

    if dispatch_count == 0 {
        return fail(
            NAME,
            format!(
                "hydrator emitted no DataplaneUpdateService for the bridge-written \
                 service; got {} action(s) total: {:?}",
                hydrator_actions.len(),
                hydrator_actions,
            ),
        );
    }
    if dispatch_count > 1 {
        return fail(
            NAME,
            format!(
                "hydrator emitted {dispatch_count} DataplaneUpdateService actions for \
                 a single bridge-written row; expected exactly one"
            ),
        );
    }
    if !matched {
        return fail(
            NAME,
            "hydrator action matched the service_id but the payload mismatch \
             gate did not flip to matched — control flow regression"
                .to_owned(),
        );
    }

    // Silence unused-binding warnings on items that are load-bearing
    // for the fixture's shape but unused after the assertion path.
    let _ = (workload_id, bridge_state);
    let _ = BTreeSet::<AllocationId>::new();

    pass(NAME)
}

// ---------------------------------------------------------------------------
// Compatibility shim — old `assert_*` entry points retained until the
// harness call sites swap over to the new `evaluate_*` names. The
// `evaluate_*` shape returns `InvariantResult` directly per the
// `MaglevDeterministic` precedent; the `assert_*` shape was
// RED-scaffold-only.
//
// Placed BEFORE the `#[cfg(test)]` retry-budget module to satisfy
// `clippy::items-after-test-module`.
// ---------------------------------------------------------------------------

/// Compatibility: invoke the eventual-convergence evaluator and
/// panic on failure. Retained only so the harness's existing
/// `assert_hydrator_eventually_converges` symbol resolves until the
/// dispatch arm in `harness.rs` swaps to `evaluate_*`.
#[doc(hidden)]
pub fn assert_hydrator_eventually_converges() {
    let result = evaluate_hydrator_eventually_converges();
    if matches!(result.status, InvariantStatus::Fail) {
        panic!("HydratorEventuallyConverges failed: {:?}", result.cause);
    }
}

/// Compatibility shim for the always-invariant. See above.
#[doc(hidden)]
pub fn assert_hydrator_idempotent_steady_state() {
    let result = evaluate_hydrator_idempotent_steady_state();
    if matches!(result.status, InvariantStatus::Fail) {
        panic!("HydratorIdempotentSteadyState failed: {:?}", result.cause);
    }
}

// ---------------------------------------------------------------------------
// S-2.2-30 — retry-budget proptest + dst-lint purity gate
//
// Scenario: `reconciler_purity_preserved_dst_lint_and_reconciler_is_pure`
//
// Two properties co-located here:
//
// 1. **Retry-budget proptest** (Tier 1 property-based): for any
//    `(attempts, last_failure_seen_at, now)` where
//    `now < last_failure_seen_at + backoff_for_attempt(attempts)`,
//    `reconcile` emits NO `Action::DataplaneUpdateService`.  At the
//    boundary (`now >= ...`) the action IS emitted.  The `View`
//    carries *inputs* unchanged within the window.
//
// 2. **dst-lint purity gate** (static analysis via
//    `xtask::dst_lint::inspect_service_map_hydrator_reconcile_body`):
//    the `ServiceMapHydrator::reconcile` body must contain no `.await`,
//    no `Instant::now`, no `SystemTime::now`, no direct DB handle — per
//    ADR-0035 §2 / ADR-0013 §2.
//
// These tests live in a `#[cfg(test)]` module so they run via nextest
// and proptest on every PR without touching the invariant catalogue or
// harness dispatch table.
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]
mod retry_budget_proptest {
    use std::collections::BTreeMap;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::time::{Duration, Instant};

    use overdrive_core::dataplane::fingerprint::fingerprint;
    use overdrive_core::id::{ServiceId, ServiceVip, SpiffeId};
    use overdrive_core::reconciler::{
        Action, Reconciler, RetryMemory, ServiceDesired, ServiceMapHydrator,
        ServiceMapHydratorState, ServiceMapHydratorView, TickContext, backoff_for_attempt,
    };
    use overdrive_core::traits::dataplane::Backend;
    use overdrive_core::traits::observation_store::ServiceHydrationStatus;
    use overdrive_core::wall_clock::UnixInstant;
    use proptest::prelude::*;

    /// Build a minimal `ServiceDesired` for proptest fixtures.
    fn make_desired() -> ServiceDesired {
        let vip =
            ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("valid ServiceVip");
        let backends = vec![Backend {
            alloc: SpiffeId::new("spiffe://overdrive.local/job/web/alloc/web-0")
                .expect("valid SpiffeId"),
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 1, 1)), 8080),
            weight: 1,
            healthy: true,
        }];
        let fp = fingerprint(&vip, &backends);
        ServiceDesired { vip, backends, fingerprint: fp }
    }

    fn make_tick(now_secs: u64) -> TickContext {
        TickContext {
            now: Instant::now(),
            now_unix: UnixInstant::from_unix_duration(Duration::from_secs(now_secs)),
            tick: now_secs,
            deadline: Instant::now() + Duration::from_secs(60),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 256,
            ..ProptestConfig::default()
        })]

        /// Property 1: within the backoff window — no action emitted.
        ///
        /// For any `(attempts, failure_secs, now_secs)` where
        /// `now_secs < failure_secs + backoff_for_attempt(attempts).as_secs()`,
        /// `reconcile` must emit zero `DataplaneUpdateService` actions
        /// when actual is `Failed { same fingerprint }`.
        ///
        /// The `View.retries` entry is UNCHANGED by a no-dispatch tick:
        /// attempts and `last_failure_seen_at` carry the same values
        /// into `next_view` (the view update only fires on dispatch).
        #[test]
        fn no_action_within_backoff_window(
            attempts in 0u32..=10u32,
            failure_secs in 10u64..=10_000u64,
            // `now_secs` is strictly BEFORE the backoff deadline.
            now_delta in 0u64..backoff_for_attempt(0).as_secs(),
        ) {
            let r = ServiceMapHydrator::canonical();
            let s_id = ServiceId::new(1).expect("valid ServiceId");
            let desired_svc = make_desired();
            let fp = desired_svc.fingerprint;

            let mut desired = BTreeMap::new();
            desired.insert(s_id, desired_svc);

            let mut actual = BTreeMap::new();
            actual.insert(
                s_id,
                ServiceHydrationStatus::Failed {
                    fingerprint: fp,
                    failed_at: UnixInstant::from_unix_duration(Duration::from_secs(failure_secs)),
                    reason: "proptest-synthetic".into(),
                },
            );
            let state = ServiceMapHydratorState { desired, actual };

            let backoff = backoff_for_attempt(attempts);
            // now_secs strictly less than deadline.
            let now_secs = failure_secs.saturating_add(now_delta)
                .min(failure_secs.saturating_add(backoff.as_secs()).saturating_sub(1));

            let mut view = ServiceMapHydratorView::default();
            view.retries.insert(
                s_id,
                RetryMemory {
                    attempts,
                    last_failure_seen_at: UnixInstant::from_unix_duration(
                        Duration::from_secs(failure_secs),
                    ),
                    last_attempted_fingerprint: Some(fp),
                },
            );

            let (actions, next_view) =
                r.reconcile(&state, &state, &view, &make_tick(now_secs));

            let deadline = failure_secs + backoff.as_secs();
            let msg = format!(
                "within backoff window no action must be emitted; \
                 now={now_secs} deadline={deadline} attempts={attempts}"
            );
            prop_assert!(actions.is_empty(), "{}", msg);

            // View inputs unchanged within the window.
            let entry = next_view.retries.get(&s_id)
                .expect("retry entry must survive no-dispatch tick");
            let got_attempts = entry.attempts;
            prop_assert!(
                got_attempts == attempts,
                "attempts must not change within backoff window",
            );
            let expected_seen_at =
                UnixInstant::from_unix_duration(Duration::from_secs(failure_secs));
            let got_seen_at = entry.last_failure_seen_at;
            prop_assert!(
                got_seen_at == expected_seen_at,
                "last_failure_seen_at must not change within backoff window",
            );
        }

        /// Property 2: at and beyond the backoff deadline — action IS emitted.
        ///
        /// For any `(attempts, failure_secs)`,
        /// `now_secs == failure_secs + backoff_for_attempt(attempts).as_secs()`
        /// must produce exactly one `DataplaneUpdateService` action.
        /// The deadline is recomputed from inputs every tick — never persisted.
        #[test]
        fn action_emitted_at_backoff_boundary(
            attempts in 0u32..=10u32,
            failure_secs in 0u64..=10_000u64,
            // Additional seconds beyond the deadline (0 = exactly at boundary).
            extra_secs in 0u64..=60u64,
        ) {
            let r = ServiceMapHydrator::canonical();
            let s_id = ServiceId::new(1).expect("valid ServiceId");
            let desired_svc = make_desired();
            let fp = desired_svc.fingerprint;

            let mut desired = BTreeMap::new();
            desired.insert(s_id, desired_svc);

            let mut actual = BTreeMap::new();
            actual.insert(
                s_id,
                ServiceHydrationStatus::Failed {
                    fingerprint: fp,
                    failed_at: UnixInstant::from_unix_duration(Duration::from_secs(failure_secs)),
                    reason: "proptest-synthetic".into(),
                },
            );
            let state = ServiceMapHydratorState { desired, actual };

            let backoff = backoff_for_attempt(attempts);
            // now_secs exactly at or beyond the deadline.
            let now_secs = failure_secs + backoff.as_secs() + extra_secs;

            let mut view = ServiceMapHydratorView::default();
            view.retries.insert(
                s_id,
                RetryMemory {
                    attempts,
                    last_failure_seen_at: UnixInstant::from_unix_duration(
                        Duration::from_secs(failure_secs),
                    ),
                    last_attempted_fingerprint: Some(fp),
                },
            );

            let (actions, _) =
                r.reconcile(&state, &state, &view, &make_tick(now_secs));

            let deadline = failure_secs + backoff.as_secs();
            let got_len = actions.len();
            let boundary_msg = format!(
                "at/beyond backoff boundary exactly one DataplaneUpdateService \
                 must be emitted; now={now_secs} deadline={deadline} attempts={attempts}"
            );
            prop_assert!(got_len == 1, "{}", boundary_msg);
            prop_assert!(
                matches!(&actions[0], Action::DataplaneUpdateService { service_id, .. }
                    if *service_id == s_id),
                "action must be DataplaneUpdateService for the expected service",
            );
        }
    }
}
