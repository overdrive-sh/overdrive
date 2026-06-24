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

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    // The four-shape fixtures construct ids/addresses that are infallible
    // by construction (ServiceId::new(42), a literal valid SpiffeId, a
    // const NonZeroU16). `.expect` documents the construction contract,
    // consistent with the `#[cfg(test)]` retry-budget module below and the
    // sibling invariant fixtures.
    clippy::expect_used
)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use overdrive_core::dataplane::fingerprint::fingerprint;
use overdrive_core::id::{NodeId, ServiceId, ServiceVip, SpiffeId};
use overdrive_core::reconcilers::{
    Action, AnyReconciler, AnyReconcilerView, AnyState, ServiceDesired, ServiceMapHydrator,
    ServiceMapHydratorState, ServiceMapHydratorView, TickContext,
};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::ServiceHydrationStatus;
use overdrive_core::wall_clock::UnixInstant;

// The Path-A/mesh workload subnet the hydrator gates against — the
// canonical `10.99.0.0/16` `WORKLOAD_SUBNET_BASE`. The `canonical(..)`
// constructor takes the same value the provisioner carves `/30`s from.
// Aliased (not a `fn`) to avoid naming the `ipnet` crate directly —
// `overdrive-sim` does not depend on it.
use overdrive_control_plane::veth_provisioner::WORKLOAD_SUBNET_BASE as MESH_SUBNET;

/// Host primary IPv4 for the LOCAL-arm classifier. Distinct from every
/// mesh and remote address used in the four-shape fixtures; itself NOT in
/// the mesh subnet so the `local-only` shape's backend (== `host_ipv4`)
/// is partitioned LOCAL, not gated as mesh.
const fn host_ipv4() -> Ipv4Addr {
    Ipv4Addr::new(10, 0, 0, 9)
}

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

    // Exercise ALL FOUR service shapes as sub-cases (convergence-model.md
    // § 10.2). The MIXED shape is the load-bearing addition — it is the
    // face the faithless full-set echo made structurally undetectable
    // (RCA § 10.4); a regression of the fingerprint-domain mismatch
    // re-fails it here within the budget.
    for shape in shape_fixtures() {
        if let Err(reason) = drive_one_shape_to_convergence(&shape) {
            return fail(NAME, format!("shape '{}': {reason}", shape.label));
        }
    }
    pass(NAME)
}

/// Drive a single service shape from cold start to convergence, modelling
/// the REAL action-shim's write-back: each emitted `DataplaneUpdateService`
/// records `Completed { fingerprint: fingerprint(vip, action.backends) }`
/// — the PROGRAMMED-subset fingerprint, NOT the desired full-set echo
/// (convergence-model.md § 10.1). The honest, shape-agnostic convergence
/// predicate (§ 10.1) is: within the budget, a tick exists where
/// `actions.is_empty()` AND every desired service has a `Completed` row.
fn drive_one_shape_to_convergence(shape: &ShapeFixture) -> Result<(), String> {
    let any_reconciler =
        AnyReconciler::ServiceMapHydrator(ServiceMapHydrator::canonical(host_ipv4(), MESH_SUBNET));
    let mut state = shape.state.clone();
    let mut view = ServiceMapHydratorView::default();

    for tick_idx in 0..CONVERGENCE_TICK_BUDGET {
        let tick = make_tick(tick_idx);
        let (actions, next_view) = any_reconciler.reconcile(
            &AnyState::ServiceMapHydrator(state.clone()),
            &AnyState::ServiceMapHydrator(state.clone()),
            &AnyReconcilerView::ServiceMapHydrator(view.clone()),
            &tick,
        );

        // Model the REAL shim (dataplane_update_service::dispatch):
        // fingerprint over the ACTION's backends (the programmed subset,
        // possibly EMPTY = the per-proto purge), NOT the desired full-set
        // echo. RegisterLocalBackend emits NO row — the harness must NOT
        // synthesise one (model the real cgroup path's silence).
        for action in &actions {
            if let Action::DataplaneUpdateService { service_id, vip, backends, .. } = action {
                if !state.desired.contains_key(service_id) {
                    return Err(format!(
                        "tick {tick_idx}: hydrator emitted DataplaneUpdateService for \
                         {service_id} which is not in state.desired"
                    ));
                }
                let applied_fp = fingerprint(vip, backends);
                state.actual.insert(
                    *service_id,
                    ServiceHydrationStatus::Completed {
                        fingerprint: applied_fp,
                        applied_at: UnixInstant::from_unix_duration(Duration::from_secs(
                            u64::from(tick_idx) + 1,
                        )),
                    },
                );
            }
        }

        let AnyReconcilerView::ServiceMapHydrator(next_view_inner) = next_view else {
            return Err("reconciler returned non-ServiceMapHydrator view variant".to_string());
        };
        view = next_view_inner;

        // Shape-agnostic convergence (§ 10.1): the loop quiesced AND every
        // desired service has a confirmed Completed row. This holds for
        // remote-only (Completed{fp(vip,[remote])}), all-mesh / local-only
        // (Completed{fp(vip,[])} via the empty purge), and mixed alike.
        if actions.is_empty() && every_service_has_completed_row(&state) {
            if tick_idx >= shape.max_converge_ticks {
                return Err(format!(
                    "converged at tick {tick_idx} but the shape budget is \
                     {} ticks — convergence is slower than the model allows",
                    shape.max_converge_ticks
                ));
            }
            return Ok(());
        }
    }

    Err(format!(
        "did not converge within {CONVERGENCE_TICK_BUDGET} ticks; \
         final actual={:?}",
        state.actual,
    ))
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

    // EACH of the four shapes must emit ZERO actions per tick once
    // converged (convergence-model.md § 10.2) — the all-mesh and
    // local-only shapes must NOT re-emit the empty purge once
    // `Completed{fp(vip,[])}` is observed.
    for shape in shape_fixtures() {
        if let Err(reason) = drive_one_shape_steady_state(&shape) {
            return fail(NAME, format!("shape '{}': {reason}", shape.label));
        }
    }
    pass(NAME)
}

/// Drive one shape to its converged `actual` (the same programmed-subset
/// `Completed` row the shim writes), then tick `STEADY_STATE_TICKS` times
/// and assert every tick emits zero actions.
fn drive_one_shape_steady_state(shape: &ShapeFixture) -> Result<(), String> {
    let any_reconciler =
        AnyReconciler::ServiceMapHydrator(ServiceMapHydrator::canonical(host_ipv4(), MESH_SUBNET));

    // Build the converged `actual`: for each service, the Completed row
    // carries `fingerprint(vip, programmed_subset)` — the value the real
    // shim writes for this shape (the empty purge for all-mesh / local-only,
    // the remote survivor for remote-only / mixed).
    let mut state = shape.state.clone();
    for (service_id, desired) in &shape.state.desired {
        let programmed = programmed_subset(desired);
        let applied_fp = fingerprint(&desired.vip, &programmed);
        state.actual.insert(
            *service_id,
            ServiceHydrationStatus::Completed {
                fingerprint: applied_fp,
                applied_at: UnixInstant::from_unix_duration(Duration::from_secs(1)),
            },
        );
    }

    let mut view = ServiceMapHydratorView::default();
    for tick_idx in 0..STEADY_STATE_TICKS {
        let tick = make_tick(tick_idx);
        let (actions, next_view) = any_reconciler.reconcile(
            &AnyState::ServiceMapHydrator(state.clone()),
            &AnyState::ServiceMapHydrator(state.clone()),
            &AnyReconcilerView::ServiceMapHydrator(view.clone()),
            &tick,
        );

        if !actions.is_empty() {
            return Err(format!(
                "tick {tick_idx}: converged hydrator emitted {} action(s); \
                 expected zero. actions={actions:?}",
                actions.len(),
            ));
        }

        let AnyReconcilerView::ServiceMapHydrator(v) = next_view else {
            return Err("reconciler returned non-ServiceMapHydrator view variant".to_string());
        };
        view = v;
    }

    Ok(())
}

/// One service-shape fixture: a label, its `desired`-only state (cold
/// `actual`), and the convergence-tick budget for the shape.
struct ShapeFixture {
    label: &'static str,
    state: ServiceMapHydratorState,
    /// The shape converges at or before this 0-based tick index.
    max_converge_ticks: u32,
}

/// The PROGRAMMED-remote subset of a service's backends — the exact set
/// the emitted `DataplaneUpdateService` carries and the shim hashes back.
/// Mirrors the hydrator's gate: drop mesh (`∈ workload_subnet`) and LOCAL
/// (`== host_ipv4`) backends.
fn programmed_subset(desired: &ServiceDesired) -> Vec<Backend> {
    let subnet = MESH_SUBNET;
    let host = host_ipv4();
    desired
        .backends
        .iter()
        .filter(|b| match b.addr.ip() {
            IpAddr::V4(v4) => !subnet.contains(&v4) && v4 != host,
            IpAddr::V6(_) => true,
        })
        .cloned()
        .collect()
}

/// The four canonical shapes (convergence-model.md § 10.2). Each is a
/// single V4-VIP service with one backend in the named address class.
fn shape_fixtures() -> Vec<ShapeFixture> {
    vec![
        // remote-only: [10.96.0.50] — survives; Completed{fp(vip,[remote])}.
        build_shape("remote-only", &[Ipv4Addr::new(10, 96, 0, 50)]),
        // all-mesh: [10.99.0.6] — gated out; empty purge → Completed{fp(vip,[])}.
        build_shape("all-mesh", &[Ipv4Addr::new(10, 99, 0, 6)]),
        // mixed: the load-bearing case — one mesh (gated out) + one remote
        // (survives into the programmed subset). This is the face the
        // faithless full-set echo made structurally undetectable (RCA § 10.4).
        build_shape(
            "mixed-mesh-remote",
            &[Ipv4Addr::new(10, 99, 0, 6), Ipv4Addr::new(10, 96, 0, 50)],
        ),
        // local-only: [host_ipv4] — partitioned LOCAL; empty purge → Completed{fp(vip,[])}.
        build_shape("local-only", &[host_ipv4()]),
    ]
}

/// Build a shape fixture carrying the given V4 backend addresses.
fn build_shape(label: &'static str, backend_ips: &[Ipv4Addr]) -> ShapeFixture {
    ShapeFixture { label, state: single_service_state(backend_ips), max_converge_ticks: 2 }
}

/// Construct a one-service `ServiceMapHydratorState` carrying the given V4
/// backend addresses (each port 8080) on a single non-mesh V4 VIP, with an
/// empty `actual` (cold start). The VIP (10.0.0.1) is itself NOT in the
/// mesh subnet, so only the backends' classes are under test.
fn single_service_state(backend_ips: &[Ipv4Addr]) -> ServiceMapHydratorState {
    let service_id = ServiceId::new(42).expect("ServiceId accepts any u64");
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("valid ServiceVip");
    let backends: Vec<Backend> = backend_ips
        .iter()
        .map(|ip| Backend {
            alloc: SpiffeId::new("spiffe://overdrive.local/job/web/alloc/web-0")
                .expect("valid SpiffeId"),
            addr: SocketAddr::new(IpAddr::V4(*ip), 8080),
            weight: 1,
            healthy: true,
        })
        .collect();
    let fp = fingerprint(&vip, &backends);

    let mut desired = BTreeMap::new();
    desired.insert(
        service_id,
        ServiceDesired {
            vip,
            port: const { std::num::NonZeroU16::new(8080).expect("8080 is non-zero") },
            proto: overdrive_core::dataplane::backend_key::Proto::Tcp,
            backends,
            fingerprint: fp,
        },
    );
    ServiceMapHydratorState { desired, actual: BTreeMap::new() }
}

/// True iff every desired service has a `Completed` `actual` row. This is
/// the shape-agnostic convergence predicate (convergence-model.md § 10.1):
/// "the loop quiesced AND a confirmed row exists per service." It does NOT
/// re-derive the programmable fingerprint — it observes that a `Completed`
/// row was produced, which (paired with `actions.is_empty()` at the call
/// site) is the honest convergence signal for every shape.
fn every_service_has_completed_row(state: &ServiceMapHydratorState) -> bool {
    state.desired.keys().all(|service_id| {
        matches!(state.actual.get(service_id), Some(ServiceHydrationStatus::Completed { .. }))
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
    use overdrive_core::reconcilers::Reconciler;
    use overdrive_core::reconcilers::backend_discovery_bridge::{
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
    bridge_state.actual.running.insert(alloc, None);
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
            port: const { std::num::NonZeroU16::new(8080).expect("8080 is non-zero") },
            proto: overdrive_core::dataplane::backend_key::Proto::Tcp,
            backends: desired_backends.clone(),
            fingerprint: desired_fp,
        },
    );

    // ---- Step 4: tick the hydrator against the projected desired
    //      with empty actual (no prior service_hydration_results row).
    let hydrator = ServiceMapHydrator::canonical(
        std::net::Ipv4Addr::UNSPECIFIED,
        overdrive_control_plane::veth_provisioner::WORKLOAD_SUBNET_BASE,
    );
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
    use overdrive_core::reconcilers::{
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
        ServiceDesired {
            vip,
            port: const { std::num::NonZeroU16::new(8080).expect("8080 is non-zero") },
            proto: overdrive_core::dataplane::backend_key::Proto::Tcp,
            backends,
            fingerprint: fp,
        }
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
            let r = ServiceMapHydrator::canonical(
                std::net::Ipv4Addr::UNSPECIFIED,
                overdrive_control_plane::veth_provisioner::WORKLOAD_SUBNET_BASE,
            );
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
            let r = ServiceMapHydrator::canonical(
                std::net::Ipv4Addr::UNSPECIFIED,
                overdrive_control_plane::veth_provisioner::WORKLOAD_SUBNET_BASE,
            );
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
