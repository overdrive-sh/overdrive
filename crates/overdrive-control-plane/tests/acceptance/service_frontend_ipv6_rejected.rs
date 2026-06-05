//! S-01-F — IPv6 VIP rejected at the action-shim with the existing
//! operator-visible `Failed` row, BEFORE the dataplane is ever called
//! (udp-service-support US-01; ADR-0060 D1a; criterion 6).
//!
//! Scenario SSOT:
//! `docs/feature/udp-service-support/distill/test-scenarios.md` S-01-F.
//!
//! Driving port: the action-shim `dispatch`. The distinct property this
//! test pins (vs the integration-tier `dispatch_rejects_ipv6_vip_with_failed_row`,
//! which uses a SimDataplane): the IPv6 rejection happens at the
//! operator-visible `ServiceFrontend::new` seam and is **NOT demoted to
//! a late opaque `DataplaneError`** — the dataplane adapter is never
//! reached. A panicking mock dataplane proves the short-circuit: if the
//! shim ever called `update_service` the test would panic.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use overdrive_control_plane::action_shim::dataplane_update_service::{self, DispatchOutcome};
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::fingerprint::fingerprint;
use overdrive_core::id::{ContentHash, CorrelationKey, NodeId, ServiceId, ServiceVip, SpiffeId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::dataplane::{
    Backend, Dataplane, DataplaneError, FlowEvent, PolicyKey, Verdict,
};
use overdrive_core::traits::observation_store::{ObservationStore, ServiceHydrationStatus};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_sim::adapters::observation_store::SimObservationStore;

/// A dataplane that panics if `update_service` is ever called. The
/// operator-visible IPv6 rejection MUST short-circuit before reaching
/// the adapter (ADR-0060 D1a — not a late opaque `DataplaneError`).
struct PanicOnUpdateService;

#[async_trait]
impl Dataplane for PanicOnUpdateService {
    async fn update_policy(&self, _k: PolicyKey, _v: Verdict) -> Result<(), DataplaneError> {
        Ok(())
    }
    async fn update_service(
        &self,
        _frontend: ServiceFrontend,
        _backends: Vec<Backend>,
    ) -> Result<(), DataplaneError> {
        panic!(
            "update_service must NOT be reached for an IPv6 VIP — the action-shim \
                rejects it at the operator-visible ServiceFrontend::new seam (ADR-0060 D1a)"
        );
    }
    async fn drain_flow_events(&self) -> Result<Vec<FlowEvent>, DataplaneError> {
        Ok(Vec::new())
    }
    async fn register_local_backend(
        &self,
        _vip: Ipv4Addr,
        _vip_port: u16,
        _backend: SocketAddrV4,
        _proto: overdrive_core::dataplane::backend_key::Proto,
    ) -> Result<(), DataplaneError> {
        Ok(())
    }
    async fn deregister_local_backend(
        &self,
        _vip: Ipv4Addr,
        _vip_port: u16,
        _backend: SocketAddrV4,
        _proto: overdrive_core::dataplane::backend_key::Proto,
    ) -> Result<(), DataplaneError> {
        Ok(())
    }
}

fn fixed_tick() -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1)),
        tick: 1,
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

#[tokio::test]
async fn ipv6_vip_rejected_at_action_shim_as_operator_visible_failed() {
    let service_id = ServiceId::new(42).expect("ServiceId");
    let ipv6_vip = ServiceVip::new(IpAddr::V6(Ipv6Addr::LOCALHOST)).expect("v6 VIP");
    let backends = vec![Backend {
        alloc: SpiffeId::new("spiffe://overdrive.local/job/dns/alloc/dns-0").expect("SpiffeId"),
        addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 1, 0, 1)), 5353),
        weight: 1,
        healthy: true,
    }];
    let fp = fingerprint(&ipv6_vip, &backends);
    let target = format!("service-map-hydrator/{service_id}");
    let spec_hash = ContentHash::of(fp.to_le_bytes());
    let correlation = CorrelationKey::derive(&target, &spec_hash, "update-service");

    let action = Action::DataplaneUpdateService {
        service_id,
        vip: ipv6_vip,
        port: std::num::NonZeroU16::new(5353).expect("non-zero"),
        proto: overdrive_core::dataplane::backend_key::Proto::Udp,
        backends: backends.clone(),
        correlation,
    };

    let dataplane: Arc<dyn Dataplane> = Arc::new(PanicOnUpdateService);
    let writer_node = NodeId::new("writer-1").expect("NodeId");
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(writer_node.clone(), 1));
    let tick = fixed_tick();

    // The panicking mock proves the dataplane is never reached.
    let outcome = dataplane_update_service::dispatch(
        &action,
        dataplane.as_ref(),
        obs.as_ref(),
        &tick,
        &writer_node,
    )
    .await
    .expect("dispatch returns Ok (IPv6 rejection is an operator-visible Failed row, not Err)");

    assert!(
        matches!(outcome, DispatchOutcome::Failed),
        "S-01-F: an IPv6 VIP must produce DispatchOutcome::Failed, got {outcome:?}"
    );

    // The operator-visible Failed row carries the IPv6-unsupported reason.
    let rows = obs.service_hydration_results_rows(&service_id).await.expect("read rows");
    assert_eq!(rows.len(), 1, "exactly one operator-visible Failed row");
    match &rows[0].status {
        ServiceHydrationStatus::Failed { reason, .. } => {
            assert!(
                reason.contains("IPv6") || reason.contains("not supported"),
                "S-01-F: Failed reason must name the IPv6-unsupported cause, got: {reason}"
            );
        }
        other => panic!("S-01-F: expected operator-visible Failed row, got {other:?}"),
    }
}
