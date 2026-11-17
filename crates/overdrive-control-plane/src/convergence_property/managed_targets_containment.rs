//! WK-4 — per-reconciler `managed_targets` containment (feature delta D-1a,
//! D-12 f; `docs/feature/reconciler-convergence-guarantees/distill/
//! test-scenarios.md` § "WK-4").
//!
//! For every world a test builds, every target whose `hydrate_desired` or
//! `hydrate_actual` projection is non-empty is in the set `managed_targets`
//! returns, both sides read through the reconciler's own methods (never the
//! world's bookkeeping); a read failure returns `Err`, never a partial set;
//! `svid-lifecycle`'s projection excludes `ever_issued` (desired plus held); a
//! reconciler whose hydrate ignores the target manages exactly one target. The
//! conformance driver cannot discharge this (it sets `has` from the same set),
//! so this is the obligation that detects an omitted target.
//!
//! CONTRACT_SHAPE: pure-function.

use std::collections::BTreeSet;

use overdrive_core::aggregate::{IntentKey, WorkloadKind};
use overdrive_core::dataplane::fingerprint::fingerprint;
use overdrive_core::id::SpiffeId;
use overdrive_core::id::{ContentHash, CorrelationKey};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::observation_store::{
    AllocState, LogicalTimestamp, ObservationStore, ObservationWrite, ServiceBackendRow,
    ServiceHydrationResultRow, ServiceHydrationStatus,
};
use overdrive_core::workflow::{WorkflowName, WorkflowStart};
use overdrive_reconcilers::AnyState;
use overdrive_testing::placement;
use std::net::Ipv4Addr;

use super::{Composed, name_of, node, runtime, target_of, workload_id};
use crate::reconciler_runtime::{hydrate_actual_for_test, hydrate_desired_for_test};

placement!(host_pure);

/// Does `(desired, actual)` project anything for the target? `svid-lifecycle`
/// excludes the global `ever_issued` audit set (D-1a containment note).
fn projection_is_non_empty(desired: &AnyState, actual: &AnyState) -> bool {
    match (desired, actual) {
        (AnyState::WorkloadLifecycle(d), AnyState::WorkloadLifecycle(a)) => {
            d.job.is_some() || !a.allocations.is_empty()
        }
        (AnyState::ServiceLifecycle(d), AnyState::ServiceLifecycle(a)) => {
            !d.allocs.is_empty()
                || !a.allocs.is_empty()
                || !a.service_dataplane.is_empty()
                || !d.service_dataplane.is_empty()
        }
        (AnyState::SvidLifecycle(d), AnyState::SvidLifecycle(a)) => {
            !d.desired.is_empty() || !a.actual.is_empty()
        }
        (AnyState::ServiceMapHydrator(d), AnyState::ServiceMapHydrator(a)) => {
            !d.desired.is_empty() || !a.actual.is_empty()
        }
        (AnyState::WorkflowLifecycle(d), AnyState::WorkflowLifecycle(a)) => {
            !d.instances.is_empty() || !a.instances.is_empty()
        }
        (AnyState::Unit, AnyState::Unit) => false,
        other => panic!("projection predicate has no arm for {other:?}"),
    }
}

/// Every candidate whose projection (read through the reconciler's own
/// hydration) is non-empty must be in `managed`.
async fn assert_non_empty_projections_are_managed(
    world: &Composed,
    reconciler: &ReconcilerName,
    candidates: &[TargetResource],
    managed: &BTreeSet<TargetResource>,
) {
    let registered = world.state.runtime.get(reconciler).expect("registered");
    let mut required = 0usize;
    for target in candidates {
        let desired = hydrate_desired_for_test(registered, target, &world.state)
            .await
            .expect("hydrate desired");
        let actual = hydrate_actual_for_test(registered, target, &world.state)
            .await
            .expect("hydrate actual");
        if projection_is_non_empty(&desired, &actual) {
            required += 1;
            assert!(
                managed.contains(target),
                "{reconciler} managed_targets omitted {target}, whose hydrate projection is \
                 non-empty (managed set: {managed:?})"
            );
        }
    }
    assert!(required > 0, "fixture: the world has no target with a non-empty projection");
}

#[test]
fn workload_lifecycle_manages_every_workload_with_intent_or_an_allocation_row() {
    let rt = runtime();
    rt.block_on(async {
        let name = name_of("workload-lifecycle");
        let world = Composed::new(vec![crate::workload_lifecycle()]).await;
        // intent-only, both, row-only; `neither` has nothing.
        world.put_job("intent-only").await;
        world.put_job("both").await;
        world.write_alloc_row("both", "both-0", AllocState::Running, WorkloadKind::Job, None).await;
        world
            .write_alloc_row("row-only", "row-only-0", AllocState::Running, WorkloadKind::Job, None)
            .await;
        // The sub-key sentinels of a workload are not workloads.
        world
            .state
            .store
            .put(IntentKey::for_workload_stop(&workload_id("intent-only")).as_bytes(), b"")
            .await
            .expect("stop");
        world
            .state
            .store
            .put(
                IntentKey::for_workload_generation(&workload_id("intent-only")).as_bytes(),
                &7u64.to_be_bytes(),
            )
            .await
            .expect("generation");
        let candidates: Vec<TargetResource> = ["intent-only", "both", "row-only", "neither"]
            .map(|id| target_of("workload", id))
            .to_vec();

        let managed = world.managed_targets(&name).await.expect("managed_targets");
        assert_non_empty_projections_are_managed(&world, &name, &candidates, &managed).await;
        // A target that has left desired and actual (no intent, no row) is not
        // enumerated: this is what stops a withdrawn workload being evaluated.
        let neither = target_of("workload", "neither");
        assert!(
            !managed.contains(&neither),
            "workload-lifecycle managed a workload with neither intent nor an allocation row: \
             {managed:?}"
        );
        for target in &managed {
            let id = target.as_str().strip_prefix("workload/").expect("a workload target");
            assert!(!id.contains('/'), "a sub-key sentinel leaked into the set as {target}");
        }
    });
}

#[test]
fn workload_lifecycle_enumeration_returns_an_error_not_the_intent_half_when_allocation_rows_cannot_be_read()
 {
    let rt = runtime();
    rt.block_on(async {
        let name = name_of("workload-lifecycle");
        let world = Composed::new(vec![crate::workload_lifecycle()]).await;
        world.put_job("intent-only").await;
        world.obs.script_alloc_status_rows_read_failure(true);
        let outcome = world.managed_targets(&name).await;
        assert!(
            outcome.is_err(),
            "workload-lifecycle managed_targets returned a set instead of Err when the \
             alloc_status read failed: {outcome:?}"
        );
    });
}

#[test]
fn service_lifecycle_manages_services_and_not_jobs() {
    let rt = runtime();
    rt.block_on(async {
        let name = name_of("service-lifecycle");
        let world = Composed::new(vec![crate::service_lifecycle()]).await;
        world.put_service("web").await;
        world.put_job("batch").await;
        world
            .write_alloc_row(
                "web",
                "web-0",
                AllocState::Running,
                WorkloadKind::Service,
                Some(Ipv4Addr::new(100, 95, 0, 2)),
            )
            .await;
        world
            .write_alloc_row("batch", "batch-0", AllocState::Running, WorkloadKind::Job, None)
            .await;
        let candidates: Vec<TargetResource> =
            ["web", "batch"].map(|id| target_of("workload", id)).to_vec();

        let managed = world.managed_targets(&name).await.expect("managed_targets");
        assert_non_empty_projections_are_managed(&world, &name, &candidates, &managed).await;
        assert!(
            !managed.contains(&target_of("workload", "batch")),
            "service-lifecycle managed a Job workload"
        );
    });
}

/// Mint an SVID for `alloc` of `workload` through the production issuance path
/// (audit row first, then the leaf), as `IssueSvid` does.
async fn issue(world: &Composed, spiffe: &SpiffeId) -> overdrive_core::traits::ca::SvidMaterial {
    crate::ca_issuance::issue_and_audit(
        world.state.ca.as_ref(),
        world.obs.as_ref(),
        world.clock.as_ref(),
        &node("local"),
        spiffe,
    )
    .await
    .expect("issue and audit")
}

#[test]
fn svid_lifecycle_manages_workloads_with_running_allocations_or_a_held_svid_and_nothing_else() {
    let rt = runtime();
    rt.block_on(async {
        let name = name_of("svid-lifecycle");
        let world = Composed::new(vec![crate::svid_lifecycle()]).await;
        // desired: a Running allocation.
        world
            .write_alloc_row("running", "running-0", AllocState::Running, WorkloadKind::Job, None)
            .await;
        // held with no row: the hold outlived a failed row write.
        let held_alloc = overdrive_core::id::AllocationId::new("held-only-0").expect("alloc");
        let held_spiffe = SpiffeId::for_allocation(&workload_id("held-only"), &held_alloc);
        let material = issue(&world, &held_spiffe).await;
        world.state.identity.hold(held_alloc, material);
        // issued once, no longer held, no row: `ever_issued` alone manages nothing.
        let ever_alloc = overdrive_core::id::AllocationId::new("ever-only-0").expect("alloc");
        issue(&world, &SpiffeId::for_allocation(&workload_id("ever-only"), &ever_alloc)).await;
        // a held entry whose identity does not invert to a workload.
        let gateway_like = SpiffeId::new("spiffe://overdrive.local/gateway/local").expect("spiffe");
        let material = issue(&world, &gateway_like).await;
        world
            .state
            .identity
            .hold(overdrive_core::id::AllocationId::new("odd-0").expect("alloc"), material);
        let candidates: Vec<TargetResource> =
            ["running", "held-only", "ever-only"].map(|id| target_of("workload", id)).to_vec();

        let managed = world.managed_targets(&name).await.expect("managed_targets");
        assert_non_empty_projections_are_managed(&world, &name, &candidates, &managed).await;
        assert!(
            !managed.contains(&target_of("workload", "ever-only")),
            "svid-lifecycle managed a workload only because it was once issued"
        );
        assert!(
            managed.iter().all(|t| !t.as_str().contains("gateway") && !t.as_str().contains("odd")),
            "svid-lifecycle returned a target for a held entry that does not invert: {managed:?}"
        );
    });
}

#[test]
fn svid_lifecycle_enumeration_returns_an_error_when_allocation_rows_cannot_be_read() {
    let rt = runtime();
    rt.block_on(async {
        let name = name_of("svid-lifecycle");
        let world = Composed::new(vec![crate::svid_lifecycle()]).await;
        world
            .write_alloc_row("running", "running-0", AllocState::Running, WorkloadKind::Job, None)
            .await;
        world.obs.script_alloc_status_rows_read_failure(true);
        let outcome = world.managed_targets(&name).await;
        assert!(
            outcome.is_err(),
            "svid-lifecycle managed_targets returned a set instead of Err when the \
             alloc_status read failed: {outcome:?}"
        );
    });
}

/// D-21: the hydrator manages a Service that has a workload intent or a
/// `service_backends` row. A `service_hydration_results` row alone adds no
/// target: the hydrator no longer reads it (D-20), and an orphaned row (intent
/// and backend row both gone) is the deletion case, #211.
#[cfg(test)]
#[test]
fn service_map_hydrator_manages_services_with_an_intent_or_a_backend_row_but_not_a_result_row_alone()
 {
    let rt = runtime();
    rt.block_on(async {
        let name = name_of("service-map-hydrator");
        let world = Composed::new(vec![crate::service_map_hydrator()]).await;
        // A deployed Service with an intent and no row of any kind.
        let (intent_only, _vip) = world.put_service("svc-intent").await;
        // A Service id with a `service_backends` row and no intent.
        let backend_only = orphan_service_id(7);
        let backend_vip = orphan_vip(7);
        world
            .obs
            .write(ObservationWrite::ServiceBackend(ServiceBackendRow {
                service_id: backend_only,
                vip: backend_vip.try_as_ipv4().expect("v4"),
                backends: vec![],
                updated_at: LogicalTimestamp { counter: 1, writer: node("writer-1") },
            }))
            .await
            .expect("backend row");
        // A Service id with only a result row: intent and backend row both gone.
        let result_only = orphan_service_id(8);
        world
            .obs
            .write(ObservationWrite::ServiceHydration(ServiceHydrationResultRow {
                service_id: result_only,
                fingerprint: fingerprint(&orphan_vip(8), &[]),
                status: ServiceHydrationStatus::Pending,
                updated_at: LogicalTimestamp { counter: 2, writer: node("writer-1") },
            }))
            .await
            .expect("result row");

        let managed = world.managed_targets(&name).await.expect("managed_targets");
        for (service, why) in [(intent_only, "a workload intent"), (backend_only, "a backend row")]
        {
            assert!(
                managed.contains(&target_of("service", &service.to_string())),
                "service-map-hydrator managed_targets omitted the service with {why}: {managed:?}"
            );
        }
        assert!(
            !managed.contains(&target_of("service", &result_only.to_string())),
            "service-map-hydrator managed a service with only a result row, the deletion case \
             (#211): {managed:?}"
        );
    });
}

/// A frontend `10.96.9.<octet>:8080/tcp` no deployed Service owns.
fn orphan_vip(octet: u8) -> overdrive_core::id::ServiceVip {
    overdrive_core::id::ServiceVip::new(std::net::IpAddr::V4(Ipv4Addr::new(10, 96, 9, octet)))
        .expect("IPv4 service VIP")
}

fn orphan_service_id(octet: u8) -> overdrive_core::id::ServiceId {
    overdrive_core::id::ServiceId::derive(
        &orphan_vip(octet),
        std::num::NonZeroU16::new(8080).expect("non-zero port"),
        overdrive_core::dataplane::backend_key::Proto::Tcp,
        "service-map",
    )
}

#[test]
fn service_map_hydrator_enumeration_returns_an_error_when_service_rows_cannot_be_read() {
    let rt = runtime();
    rt.block_on(async {
        let name = name_of("service-map-hydrator");
        let world = Composed::new(vec![crate::service_map_hydrator()]).await;
        world.put_service("svc-a").await;
        world.obs.script_service_rows_read_failure(true);
        let outcome = world.managed_targets(&name).await;
        assert!(
            outcome.is_err(),
            "service-map-hydrator managed_targets returned a set instead of Err when the \
             service rows could not be read: {outcome:?}"
        );
    });
}

#[test]
fn workflow_lifecycle_manages_its_one_conventional_target_once_an_instance_exists() {
    let rt = runtime();
    rt.block_on(async {
        let name = name_of("workflow-lifecycle");
        let world = Composed::new(vec![crate::workflow_lifecycle()]).await;
        let before = world.managed_targets(&name).await.expect("managed_targets");
        assert!(before.is_empty(), "workflow-lifecycle managed {before:?} with no instance");
        let spec = WorkflowStart {
            name: WorkflowName::new("provision-record").expect("workflow name"),
            input: Vec::new(),
        };
        let correlation = CorrelationKey::derive(
            "wf-provision-0001",
            &ContentHash::of(spec.name.as_str().as_bytes()),
            "start-workflow",
        );
        let archived = spec.archive_for_store().expect("archive");
        world
            .state
            .store
            .put(IntentKey::for_workflow_instance(&correlation).as_bytes(), archived.as_ref())
            .await
            .expect("put workflow intent");
        let candidates = vec![target_of("workflow", "all")];

        let managed = world.managed_targets(&name).await.expect("managed_targets");
        assert_non_empty_projections_are_managed(&world, &name, &candidates, &managed).await;
        assert_eq!(managed.len(), 1, "workflow-lifecycle manages exactly its one target");
    });
}

#[test]
fn vm_reclamation_manages_exactly_its_node_target() {
    let rt = runtime();
    rt.block_on(async {
        let name = name_of("vm-reclamation");
        let world = Composed::new(vec![crate::vm_reclamation()]).await;
        let managed = world.managed_targets(&name).await.expect("managed_targets");
        let expected: BTreeSet<TargetResource> = [target_of("node", "writer-1")].into();
        assert_eq!(
            managed, expected,
            "vm-reclamation managed_targets omitted its node target (it hydrates from the \
             host, so the one conventional target is always managed)"
        );
    });
}

#[test]
fn gateway_svid_lifecycle_manages_exactly_its_node_target() {
    let rt = runtime();
    rt.block_on(async {
        let name = name_of("gateway-svid-lifecycle");
        let world =
            Composed::new(vec![overdrive_reconcilers::AnyReconciler::GatewaySvidLifecycle(
                overdrive_reconcilers::GatewaySvidLifecycle::canonical(),
            )])
            .await;
        let managed = world.managed_targets(&name).await.expect("managed_targets");
        let expected: BTreeSet<TargetResource> = [target_of("node", "writer-1")].into();
        assert_eq!(
            managed, expected,
            "gateway-svid-lifecycle managed_targets omitted its node target (always managed \
             once the reconciler is registered)"
        );
    });
}
