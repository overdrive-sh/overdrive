//! UI-05 acceptance — the `backend-discovery-bridge` reconciler emits
//! `Action::EnqueueEvaluation { reconciler: "service-map-hydrator",
//! target: "service/<id>" }` alongside its `WriteServiceBackendRow`
//! so the `ServiceMapHydrator` ticks against the bridge-written row
//! on the next convergence cycle.
//!
//! Pre-UI-05 the bridge emitted only `WriteServiceBackendRow`; the
//! hydrator never ticked in production because no cross-reconciler
//! handoff mechanism existed. This test pins the dual emission at
//! the bridge's reconcile surface — the load-bearing structural
//! property the architectural remediation introduces.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::num::NonZeroU16;

use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip, WorkloadId};
use overdrive_core::reconcilers::Reconciler;
use overdrive_core::reconcilers::backend_discovery_bridge::{
    BackendDiscoveryBridge, BackendDiscoveryBridgeState, BackendDiscoveryBridgeView,
    ProjectedListener,
};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::wall_clock::UnixInstant;
use std::net::IpAddr;
use std::time::{Duration, Instant};

fn tick(t: u64) -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(t)),
        tick: t,
        deadline: now + Duration::from_secs(1),
    }
}

/// GIVEN a `BackendDiscoveryBridge` with one Service listener and
/// one Running alloc —
/// WHEN `reconcile` ticks once with an empty `View` (forcing the
/// dedup branch to fire) —
/// THEN exactly two actions are emitted:
///   1. `Action::WriteServiceBackendRow` carrying the bridge's
///      computed row.
///   2. `Action::EnqueueEvaluation { reconciler:
///      "service-map-hydrator", target: "service/<service_id>" }`.
///
/// The pairing is the UI-05 cross-reconciler handoff. Without it, the
/// hydrator would never tick on the bridge's write — the production
/// gap the architectural remediation closes.
#[test]
fn bridge_reconcile_emits_paired_write_and_enqueue_for_hydrator() {
    let writer_node = NodeId::new("host-0").expect("valid NodeId");
    let host_ipv4 = Ipv4Addr::new(10, 0, 0, 5);
    let workload_id = WorkloadId::new("payments").expect("valid WorkloadId");
    let service_id = ServiceId::new(42).expect("ServiceId accepts u64");
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 1, 0, 1))).expect("valid VIP");
    let port = NonZeroU16::new(8080).expect("non-zero port");
    let alloc = AllocationId::new("alloc-a").expect("valid AllocationId");

    let bridge = BackendDiscoveryBridge::new(host_ipv4, writer_node);
    let mut state = BackendDiscoveryBridgeState::empty_for_workload(workload_id);
    state
        .desired
        .listeners
        .insert(service_id, ProjectedListener { vip, port, protocol: Proto::Tcp });
    state.actual.running.insert(alloc, None);
    let view = BackendDiscoveryBridgeView::default();

    let (actions, _next_view) = bridge.reconcile(&state, &state, &view, &tick(1));

    assert_eq!(
        actions.len(),
        2,
        "UI-05: bridge MUST emit dual actions (WriteServiceBackendRow + \
         EnqueueEvaluation); got {} action(s): {:?}",
        actions.len(),
        actions
    );

    // First action — WriteServiceBackendRow.
    let Action::WriteServiceBackendRow { row, .. } = &actions[0] else {
        panic!("action[0] must be WriteServiceBackendRow, got {:?}", actions[0]);
    };
    assert_eq!(
        row.service_id, service_id,
        "WriteServiceBackendRow.service_id must match the listener's ServiceId"
    );

    // Second action — EnqueueEvaluation routed at the hydrator.
    let Action::EnqueueEvaluation { reconciler, target } = &actions[1] else {
        panic!("action[1] must be EnqueueEvaluation, got {:?}", actions[1]);
    };
    assert_eq!(
        reconciler.as_str(),
        "service-map-hydrator",
        "EnqueueEvaluation MUST route at the service-map-hydrator reconciler"
    );
    assert_eq!(
        target.as_str(),
        &format!("service/{service_id}"),
        "EnqueueEvaluation target MUST be service/<service_id>"
    );
}

/// GIVEN a `BackendDiscoveryBridge` with the dedup branch primed
/// (the View already carries the fingerprint matching the
/// current `(vip, backends)` shape) —
/// WHEN `reconcile` ticks against the same state —
/// THEN zero actions are emitted — the dedup branch suppresses
/// both the `WriteServiceBackendRow` AND the paired
/// `EnqueueEvaluation`. The handoff is paired with the WRITE, not
/// emitted unconditionally per tick.
#[test]
fn bridge_dedup_branch_emits_zero_actions_including_no_enqueue() {
    let writer_node = NodeId::new("host-0").expect("valid NodeId");
    let host_ipv4 = Ipv4Addr::new(10, 0, 0, 5);
    let workload_id = WorkloadId::new("payments").expect("valid WorkloadId");
    let service_id = ServiceId::new(99).expect("valid ServiceId");
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 1, 0, 2))).expect("valid VIP");
    let port = NonZeroU16::new(9000).expect("non-zero port");
    let alloc = AllocationId::new("alloc-b").expect("valid AllocationId");

    let bridge = BackendDiscoveryBridge::new(host_ipv4, writer_node);
    let mut state = BackendDiscoveryBridgeState::empty_for_workload(workload_id);
    state
        .desired
        .listeners
        .insert(service_id, ProjectedListener { vip, port, protocol: Proto::Tcp });
    state.actual.running.insert(alloc, None);

    // First tick — write happens; expect dual emission.
    let (actions_first, view_after_first) =
        bridge.reconcile(&state, &state, &BackendDiscoveryBridgeView::default(), &tick(1));
    assert_eq!(actions_first.len(), 2, "first tick must emit dual actions");

    // Second tick — feed prior view back in; dedup must fire and
    // suppress BOTH the write AND the paired enqueue.
    let (actions_second, _) = bridge.reconcile(&state, &state, &view_after_first, &tick(2));
    assert!(
        actions_second.is_empty(),
        "dedup branch must suppress both WriteServiceBackendRow AND the paired \
         EnqueueEvaluation; got {} action(s): {:?}",
        actions_second.len(),
        actions_second
    );
}

/// Compile-time presence smoke: the `Action::EnqueueEvaluation`
/// variant is constructible from the public re-exports —
/// pins the public Action surface for downstream consumers (the
/// action-shim dispatch wrapper, the DST evaluator).
#[test]
fn enqueue_evaluation_action_variant_is_publicly_constructible() {
    use overdrive_core::reconcilers::{ReconcilerName, TargetResource};

    let reconciler = ReconcilerName::new("service-map-hydrator").expect("valid name");
    let target = TargetResource::new("service/1").expect("valid target");
    let _ = Action::EnqueueEvaluation { reconciler, target };

    // BTreeSet just to silence "unused import" in alternative
    // refactors that need to enumerate emitted actions; load-bearing
    // shape pin only.
    let _ = BTreeSet::<u64>::new();
}
