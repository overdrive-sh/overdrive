//! S-2.2-28 — Action shim's per-arm `Action::DataplaneUpdateService`
//! dispatch writes a `service_hydration_results` row keyed on
//! `(service_id, fingerprint)` per architecture.md § 7 *Failure
//! surface* and § 12 *Schema*.
//!
//! Tags: `@US-08` `@K8` `@slice-08` `@in-memory`.
//!
//! Tier 1 — calls the typed per-arm dispatch fn directly with
//! `Arc<SimDataplane>` + `Arc<SimObservationStore>`. Pins both branches:
//!
//! - `Ok(())` from `Dataplane::update_service`
//!     → row `status: Completed { fingerprint, applied_at: tick.now }`.
//! - `Err(DataplaneError::*)`
//!     → row `status: Failed { fingerprint, failed_at: tick.now,
//!       reason: Display::to_string(&err) }`.
//!
//! Per architecture.md § 7: the failure surface is observation, NOT a
//! `TerminalCondition` claim — service hydration cannot terminate an
//! allocation; the row's `status` enum carries every dispatch outcome.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use overdrive_control_plane::action_shim::dataplane_update_service::{self, DispatchOutcome};
use overdrive_core::dataplane::fingerprint::fingerprint;
use overdrive_core::id::{ContentHash, CorrelationKey, NodeId, ServiceId, ServiceVip, SpiffeId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::dataplane::{
    Backend, Dataplane, DataplaneError, FlowEvent, PolicyKey, Verdict,
};
use overdrive_core::traits::observation_store::{ObservationStore, ServiceHydrationStatus};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::observation_store::SimObservationStore;

/// Minimal failing-on-update dataplane wrapper for the `Err(...)`
/// branch. The sim dataplane always succeeds — composing a thin wrapper
/// here is preferable to widening `SimDataplane`'s public surface for a
/// single test case.
struct FailingUpdateService;

#[async_trait]
impl Dataplane for FailingUpdateService {
    async fn update_policy(
        &self,
        _key: PolicyKey,
        _verdict: Verdict,
    ) -> Result<(), DataplaneError> {
        Ok(())
    }

    async fn update_service(
        &self,
        _frontend: overdrive_core::dataplane::ServiceFrontend,
        _backends: Vec<Backend>,
    ) -> Result<(), DataplaneError> {
        Err(DataplaneError::Busy)
    }

    async fn drain_flow_events(&self) -> Result<Vec<FlowEvent>, DataplaneError> {
        Ok(Vec::new())
    }

    async fn register_local_backend(
        &self,
        _vip: Ipv4Addr,
        _vip_port: u16,
        _backend: std::net::SocketAddrV4,
        _proto: overdrive_core::dataplane::backend_key::Proto,
    ) -> Result<(), DataplaneError> {
        Ok(())
    }

    async fn deregister_local_backend(
        &self,
        _vip: Ipv4Addr,
        _vip_port: u16,
        _proto: overdrive_core::dataplane::backend_key::Proto,
    ) -> Result<(), DataplaneError> {
        Ok(())
    }
}

fn fixed_tick() -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000)),
        tick: 7,
        deadline: now + Duration::from_millis(100),
    }
}

fn sample_action() -> (ServiceId, ServiceVip, Vec<Backend>, CorrelationKey) {
    let service_id = ServiceId::new(42).expect("ServiceId");
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("ServiceVip");
    let backends = vec![Backend {
        alloc: SpiffeId::new("spiffe://overdrive.local/job/payments/alloc/a1b2c3")
            .expect("SpiffeId"),
        addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 8080),
        weight: 100,
        healthy: true,
    }];
    let fp = fingerprint(&vip, &backends);
    let target = format!("service-map-hydrator/{service_id}");
    let spec_hash = ContentHash::of(fp.to_le_bytes());
    let correlation = CorrelationKey::derive(&target, &spec_hash, "update-service");
    (service_id, vip, backends, correlation)
}

#[tokio::test]
async fn dispatch_writes_completed_row_on_dataplane_ok() {
    let (service_id, vip, backends, correlation) = sample_action();
    let action = Action::DataplaneUpdateService {
        service_id,
        vip,
        port: std::num::NonZeroU16::new(8080).expect("non-zero"),
        proto: overdrive_core::dataplane::backend_key::Proto::Tcp,
        backends: backends.clone(),
        correlation,
    };
    let dataplane: Arc<dyn Dataplane> = Arc::new(SimDataplane::new());
    let writer_node = NodeId::new("writer-1").expect("NodeId");
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(writer_node.clone(), 1));
    let tick = fixed_tick();

    let outcome = dataplane_update_service::dispatch(
        &action,
        dataplane.as_ref(),
        obs.as_ref(),
        &tick,
        &writer_node,
    )
    .await
    .expect("dispatch returns Ok on Dataplane::update_service success");

    assert!(matches!(outcome, DispatchOutcome::Completed));

    let rows = obs.service_hydration_results_rows(&service_id).await.expect("read rows");
    assert_eq!(rows.len(), 1, "exactly one row written");
    let row = &rows[0];
    assert_eq!(row.service_id, service_id);
    let expected_fp = fingerprint(&vip, &backends);
    assert_eq!(row.fingerprint, expected_fp);
    match &row.status {
        ServiceHydrationStatus::Completed { fingerprint: fp, applied_at } => {
            assert_eq!(*fp, expected_fp);
            assert_eq!(*applied_at, tick.now_unix);
        }
        other => panic!("expected Completed, got {other:?}"),
    }
}

#[tokio::test]
async fn dispatch_writes_failed_row_on_dataplane_err() {
    let (service_id, vip, backends, correlation) = sample_action();
    let action = Action::DataplaneUpdateService {
        service_id,
        vip,
        port: std::num::NonZeroU16::new(8080).expect("non-zero"),
        proto: overdrive_core::dataplane::backend_key::Proto::Tcp,
        backends: backends.clone(),
        correlation,
    };
    let dataplane: Arc<dyn Dataplane> = Arc::new(FailingUpdateService);
    let writer_node = NodeId::new("writer-1").expect("NodeId");
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(writer_node.clone(), 2));
    let tick = fixed_tick();

    let outcome = dataplane_update_service::dispatch(
        &action,
        dataplane.as_ref(),
        obs.as_ref(),
        &tick,
        &writer_node,
    )
    .await
    .expect("dispatch returns Ok even on Dataplane::update_service Err — outcome is Failed");

    assert!(matches!(outcome, DispatchOutcome::Failed));

    let rows = obs.service_hydration_results_rows(&service_id).await.expect("read rows");
    assert_eq!(rows.len(), 1, "exactly one row written");
    let row = &rows[0];
    assert_eq!(row.service_id, service_id);
    let expected_fp = fingerprint(&vip, &backends);
    assert_eq!(row.fingerprint, expected_fp);
    match &row.status {
        ServiceHydrationStatus::Failed { fingerprint: fp, failed_at, reason } => {
            assert_eq!(*fp, expected_fp);
            assert_eq!(*failed_at, tick.now_unix);
            // `Display` of `DataplaneError::Busy` is fixed by the
            // `#[error("dataplane busy, retry later")]` attribute on
            // the variant.
            assert_eq!(reason, &DataplaneError::Busy.to_string());
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

/// Regression: an IPv6 VIP must NOT silently fall through to
/// `Dataplane::update_service(0.0.0.0, ...)`. The dispatch must
/// short-circuit with a `Failed` observation row whose `reason`
/// names the unsupported address family, and never call the
/// dataplane at all.
#[tokio::test]
async fn dispatch_rejects_ipv6_vip_with_failed_row() {
    let service_id = ServiceId::new(99).expect("ServiceId");
    let ipv6_vip =
        ServiceVip::new(IpAddr::V6(Ipv6Addr::new(0xfd, 0xc2, 0, 0, 0, 0, 0, 1))).expect("v6 VIP");
    let backends = vec![Backend {
        alloc: SpiffeId::new("spiffe://overdrive.local/job/payments/alloc/a1b2c3")
            .expect("SpiffeId"),
        addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 8080),
        weight: 100,
        healthy: true,
    }];
    let fp = fingerprint(&ipv6_vip, &backends);
    let target = format!("service-map-hydrator/{service_id}");
    let spec_hash = ContentHash::of(fp.to_le_bytes());
    let correlation = CorrelationKey::derive(&target, &spec_hash, "update-service");

    let action = Action::DataplaneUpdateService {
        service_id,
        vip: ipv6_vip,
        port: std::num::NonZeroU16::new(8080).expect("non-zero"),
        proto: overdrive_core::dataplane::backend_key::Proto::Tcp,
        backends: backends.clone(),
        correlation,
    };

    let dataplane: Arc<dyn Dataplane> = Arc::new(SimDataplane::new());
    let writer_node = NodeId::new("writer-1").expect("NodeId");
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(writer_node.clone(), 3));
    let tick = fixed_tick();

    let outcome = dataplane_update_service::dispatch(
        &action,
        dataplane.as_ref(),
        obs.as_ref(),
        &tick,
        &writer_node,
    )
    .await
    .expect("dispatch returns Ok (IPv6 rejection is Failed, not Err)");

    assert!(
        matches!(outcome, DispatchOutcome::Failed),
        "IPv6 VIP must produce DispatchOutcome::Failed, got {outcome:?}"
    );

    let rows = obs.service_hydration_results_rows(&service_id).await.expect("read rows");
    assert_eq!(rows.len(), 1, "exactly one Failed row written");
    let row = &rows[0];
    assert_eq!(row.service_id, service_id);
    assert_eq!(row.fingerprint, fp);
    match &row.status {
        ServiceHydrationStatus::Failed { fingerprint: row_fp, failed_at, reason } => {
            assert_eq!(*row_fp, fp);
            assert_eq!(*failed_at, tick.now_unix);
            assert!(
                reason.contains("IPv6") || reason.contains("not supported"),
                "reason should mention IPv6 rejection, got: {reason}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}
