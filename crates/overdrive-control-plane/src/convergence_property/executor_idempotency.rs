//! The executor-idempotency companion to the convergence property (US-05 S4,
//! GH #351): applying a reconciler's actions twice through the production action
//! shim leaves the same observable state as applying them once. A reconciler
//! re-emits an action every pass until the world shows it landed, so an executor
//! that is not idempotent turns the pass into a source of duplicate effects.
//!
//! Each case reconciles a non-converged world through the reconciler's own
//! hydration and `reconcile`, then dispatches the resulting actions through
//! `dispatch_with_workflow_intent` (the shim every tick uses) once, and twice
//! over an identical world. The universe is the port-exposed observable of the
//! executor's effect (the held set, the dataplane's backends, the published
//! backend row, the allocation rows, the host), never a row timestamp or a
//! logical counter, which a second write legitimately advances.
//!
//! Each case first proves the comparison is not vacuous: the first application
//! must succeed and its effect must land (the observable differs from the world
//! before dispatch). A dispatch that failed both times, or changed nothing, would
//! otherwise make `once == twice` hold trivially. The second application may
//! legitimately refuse (an executor that rejects a repeat); what must hold is
//! the state.
//!
//! Two executors are not repeated here because an existing test already pins
//! their repeat-safety through the same production path:
//! - VM reclamation: `action_shim::reclamation::tests::is_idempotent_on_an_already_absent_allocation`
//!   (the discard of an already-absent allocation succeeds) and the
//!   `vm_reclamation` steady-state invariant in `overdrive-sim`.
//! - Workload start: `tests/acceptance/lifecycle_broadcast.rs`
//!   `barriered_starting_and_live_duplicate_actions_preserve_the_real_shim_row_and_event`
//!   (a duplicate `StartAllocation` through the real shim, while the original is
//!   Starting and after it is Live, preserves the row and emits no event).
//!
//! The remaining executors are existing production code, so the cases are green
//! today and stay green: they guard the pass against an executor that cannot be
//! repeated. CONTRACT_SHAPE: bounded-change.

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::SpiffeId;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::AllocationId;
use overdrive_core::reconcilers::{Action, ReconcilerName, TargetResource, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    AllocState, LogicalTimestamp, ObservationStore, ObservationWrite, ServiceBackendRow,
};
use overdrive_reconcilers::AnyReconcilerView;
use overdrive_reconcilers::GatewaySvidLifecycleView;
use overdrive_reconcilers::service_lifecycle::ServiceLifecycleView;
use overdrive_reconcilers::service_map_hydrator::ServiceMapHydratorView;
use overdrive_reconcilers::svid_lifecycle::SvidLifecycleView;
use overdrive_testing::placement;

use super::{Composed, name_of, node, runtime, target_of, workload_id};
use crate::action_shim::{ShimError, dispatch_with_workflow_intent};
use crate::reconciler_runtime::{hydrate_actual_for_test, hydrate_desired_for_test};

placement!(host_pure);

/// The actions `name` emits for `target` over the world as hydrated now.
async fn emitted(
    world: &Composed,
    name: &ReconcilerName,
    target: &TargetResource,
    view: AnyReconcilerView,
) -> (Vec<Action>, TickContext) {
    let reconciler = world.state.runtime.get(name).expect("registered");
    let desired =
        hydrate_desired_for_test(reconciler, target, &world.state).await.expect("hydrate desired");
    let actual =
        hydrate_actual_for_test(reconciler, target, &world.state).await.expect("hydrate actual");
    let now = Clock::now(&*world.clock);
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_clock(&*world.clock),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };
    let (actions, _next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);
    assert!(
        actions.iter().any(|action| !matches!(action, Action::Noop)),
        "fixture: {name} emits nothing over this world, so there is no executor to repeat"
    );
    (actions, tick)
}

/// Dispatch `actions` `times` times through the production shim and return each
/// dispatch's result in order.
async fn apply(
    world: &Composed,
    actions: &[Action],
    tick: &TickContext,
    times: usize,
) -> Vec<Result<(), ShimError>> {
    let mut results = Vec::with_capacity(times);
    for _ in 0..times {
        results.push(dispatch_with_workflow_intent(actions.to_vec(), &world.state, tick).await);
    }
    results
}

/// Build the world twice, apply once to the first and twice to the second, and
/// compare the executor's observable effect. The first application must succeed
/// and change the observable, so the equality is never a comparison of two
/// untouched worlds.
async fn assert_apply_twice_equals_apply_once<W, FutW, O, FutO>(
    reconciler: &str,
    build: W,
    view: fn() -> AnyReconcilerView,
    observe: O,
) where
    W: Fn() -> FutW,
    FutW: std::future::Future<Output = (Composed, TargetResource)>,
    O: Fn(Arc<Composed>) -> FutO,
    FutO: std::future::Future<Output = String>,
{
    let name = name_of(reconciler);

    let (once_world, target) = build().await;
    let once_world = Arc::new(once_world);
    let before = observe(Arc::clone(&once_world)).await;
    let (actions, tick) = emitted(&once_world, &name, &target, view()).await;
    let first = apply(&once_world, &actions, &tick, 1).await;
    assert!(
        first[0].is_ok(),
        "{reconciler}: the first application of its actions failed: {:?}",
        first[0]
    );
    let once = observe(Arc::clone(&once_world)).await;
    assert_ne!(
        before, once,
        "{reconciler}: the first application succeeded but its effect did not land"
    );

    let (twice_world, target) = build().await;
    let twice_world = Arc::new(twice_world);
    let (actions, tick) = emitted(&twice_world, &name, &target, view()).await;
    let both = apply(&twice_world, &actions, &tick, 2).await;
    assert!(
        both[0].is_ok(),
        "{reconciler}: the first application of its actions failed: {:?}",
        both[0]
    );
    let twice = observe(Arc::clone(&twice_world)).await;

    assert_eq!(
        once, twice,
        "{reconciler}: applying its actions twice changed observable state beyond applying them once"
    );
}

fn addr(index: u8) -> Ipv4Addr {
    Ipv4Addr::new(100, 95, 0, index)
}

#[test]
fn applying_the_hydrators_dataplane_update_twice_equals_applying_it_once() {
    #[cfg(test)]
    async fn build() -> (Composed, TargetResource) {
        let world = Composed::new(vec![crate::service_map_hydrator()]).await;
        let (sid, vip) = world.put_service("payments").await;
        world
            .obs
            .write(ObservationWrite::ServiceBackend(ServiceBackendRow {
                service_id: sid,
                vip: vip.try_as_ipv4().expect("v4"),
                backends: vec![Backend {
                    alloc: SpiffeId::from_str(
                        "spiffe://overdrive.local/workload/payments/alloc/a0",
                    )
                    .expect("spiffe"),
                    addr: SocketAddr::new(Ipv4Addr::new(10, 1, 2, 3).into(), 8080),
                    weight: 100,
                    healthy: true,
                }],
                updated_at: LogicalTimestamp { counter: 1, writer: node("writer-1") },
            }))
            .await
            .expect("backend row");
        (world, target_of("service", &sid.to_string()))
    }
    async fn observe(world: Arc<Composed>) -> String {
        let rows = world.obs.all_service_backends_rows().await.expect("rows");
        let row = rows.first().expect("backend row").clone();
        let results: Vec<String> = world
            .obs
            .service_hydration_results_rows(&row.service_id)
            .await
            .expect("results")
            .into_iter()
            .map(|r| format!("{:?}/{:?}", r.fingerprint, std::mem::discriminant(&r.status)))
            .collect();
        format!("{:?} | {results:?}", world.dataplane.service_backends(row.vip))
    }
    runtime().block_on(assert_apply_twice_equals_apply_once(
        "service-map-hydrator",
        build,
        || AnyReconcilerView::ServiceMapHydrator(ServiceMapHydratorView::default()),
        observe,
    ));
}

#[test]
fn applying_the_svid_issuance_twice_equals_applying_it_once() {
    async fn build() -> (Composed, TargetResource) {
        let world = Composed::new(vec![crate::svid_lifecycle()]).await;
        world
            .write_alloc_row("payments", "payments-0", AllocState::Running, WorkloadKind::Job, None)
            .await;
        (world, target_of("workload", "payments"))
    }
    async fn observe(world: Arc<Composed>) -> String {
        let held: Vec<_> = world
            .state
            .identity
            .held_snapshot()
            .into_iter()
            .map(|(alloc, facts)| (alloc.to_string(), facts.spiffe_id.to_string()))
            .collect();
        format!("{held:?}")
    }
    runtime().block_on(assert_apply_twice_equals_apply_once(
        "svid-lifecycle",
        build,
        || AnyReconcilerView::SvidLifecycle(SvidLifecycleView::default()),
        observe,
    ));
}

#[test]
fn applying_the_svid_drop_twice_equals_applying_it_once() {
    async fn build() -> (Composed, TargetResource) {
        let world = Composed::new(vec![crate::svid_lifecycle()]).await;
        world
            .write_alloc_row(
                "payments",
                "payments-0",
                AllocState::Terminated,
                WorkloadKind::Job,
                None,
            )
            .await;
        let alloc = AllocationId::new("payments-0").expect("alloc");
        let spiffe = SpiffeId::for_allocation(&workload_id("payments"), &alloc);
        let material = crate::ca_issuance::issue_and_audit(
            world.state.ca.as_ref(),
            world.obs.as_ref(),
            world.clock.as_ref(),
            &world.state.node_id,
            &spiffe,
        )
        .await
        .expect("pre-issue");
        world.state.identity.hold(alloc, material);
        (world, target_of("workload", "payments"))
    }
    async fn observe(world: Arc<Composed>) -> String {
        let held: BTreeSet<String> =
            world.state.identity.held_snapshot().keys().map(ToString::to_string).collect();
        format!("{held:?}")
    }
    runtime().block_on(assert_apply_twice_equals_apply_once(
        "svid-lifecycle",
        build,
        || AnyReconcilerView::SvidLifecycle(SvidLifecycleView::default()),
        observe,
    ));
}

#[test]
fn applying_the_service_backend_publication_twice_equals_applying_it_once() {
    async fn build() -> (Composed, TargetResource) {
        let world = Composed::new(vec![crate::service_lifecycle()]).await;
        world.put_service("web").await;
        world
            .write_alloc_row(
                "web",
                "web-0",
                AllocState::Running,
                WorkloadKind::Service,
                Some(addr(2)),
            )
            .await;
        (world, target_of("workload", "web"))
    }
    async fn observe(world: Arc<Composed>) -> String {
        let rows = world.obs.all_service_backends_rows().await.expect("rows");
        let published: Vec<BTreeSet<SocketAddr>> = rows
            .into_iter()
            .map(|row| row.backends.into_iter().map(|b| b.addr).collect())
            .collect();
        format!("{published:?} | {:?}", world.alloc_rows().await)
    }
    runtime().block_on(assert_apply_twice_equals_apply_once(
        "service-lifecycle",
        build,
        || AnyReconcilerView::ServiceLifecycle(ServiceLifecycleView::default()),
        observe,
    ));
}

#[test]
fn applying_the_gateway_svid_issuance_twice_equals_applying_it_once() {
    async fn build() -> (Composed, TargetResource) {
        let world = Composed::new_with_gateway(vec![super::gateway::reconciler()], None).await;
        (world, target_of("node", "writer-1"))
    }
    async fn observe(world: Arc<Composed>) -> String {
        // The held identity's epoch and subject, and the subjects the audit
        // trail names; never the serial or validity, which a second leaf
        // legitimately replaces.
        let slot = world.gateway_slot.as_ref().expect("gateway is composed");
        let held = slot.current().map(|facts| (facts.epoch, facts.spiffe_id.to_string()));
        let audited: BTreeSet<String> = world
            .obs
            .issued_certificate_rows()
            .await
            .expect("audit rows")
            .into_iter()
            .map(|row| row.spiffe_id.to_string())
            .collect();
        format!("{held:?} | {audited:?}")
    }
    runtime().block_on(assert_apply_twice_equals_apply_once(
        "gateway-svid-lifecycle",
        build,
        || AnyReconcilerView::GatewaySvidLifecycle(GatewaySvidLifecycleView::default()),
        observe,
    ));
}
