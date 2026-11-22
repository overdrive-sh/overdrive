//! S-2.2-28 — Action shim's per-arm `Action::DataplaneUpdateService`
//! dispatch calls `Dataplane::update_service` and reports the outcome per
//! architecture.md § 7 *Failure surface*.
//!
//! Tags: `@US-08` `@K8` `@slice-08` `@in-memory`.
//!
//! Tier 1 — calls the typed per-arm dispatch fn directly with
//! `Arc<SimDataplane>`. Pins each branch:
//!
//! - `Ok(())` from `Dataplane::update_service`
//!     → no event.
//! - `Err(DataplaneError::*)`
//!     → one `dataplane.update_service.failed`
//!       event carrying the service id, the backend-set fingerprint and
//!       `Display::to_string(&err)`.
//!
//! Per architecture.md § 7: the failure surface is a log event, NOT a
//! `TerminalCondition` claim — service hydration cannot terminate an
//! allocation. The dispatch records no observation row; the hydrator
//! learns whether the update landed from the programmed service map.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tracing::subscriber::set_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

use overdrive_control_plane::action_shim::dataplane_update_service;
use overdrive_core::dataplane::fingerprint::fingerprint;
use overdrive_core::id::{ContentHash, CorrelationKey, ServiceId, ServiceVip, SpiffeId};
use overdrive_core::reconcilers::Action;
use overdrive_core::traits::dataplane::{
    Backend, Dataplane, DataplaneError, FlowEvent, PolicyKey, Verdict,
};
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_testing::overdrive_test;

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
}

#[derive(Clone, Default)]
struct CapturedEvents {
    inner: Arc<Mutex<Vec<String>>>,
}

impl CapturedEvents {
    fn entries(&self) -> Vec<String> {
        self.inner.lock().expect("captured events mutex").clone()
    }
}

struct Fields<'a>(&'a mut String);

impl tracing::field::Visit for Fields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        write!(self.0, " {}={:?}", field.name(), value).expect("write to String");
    }
}

impl<S> Layer<S> for CapturedEvents
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut line = String::from(event.metadata().name());
        event.record(&mut Fields(&mut line));
        self.inner.lock().expect("captured events mutex").push(line);
    }
}

fn sample_action() -> (ServiceId, ServiceVip, Vec<Backend>, Action) {
    let service_id = ServiceId::new(42).expect("ServiceId");
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("ServiceVip");
    let backends = vec![Backend {
        workload_identity: SpiffeId::new("spiffe://overdrive.local/workload/payments")
            .expect("SpiffeId"),
        addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 8080),
        weight: 100,
        healthy: true,
    }];
    let fp = fingerprint(&vip, &backends);
    let target = format!("service-map-hydrator/{service_id}");
    let spec_hash = ContentHash::of(fp.to_le_bytes());
    let correlation = CorrelationKey::derive(&target, &spec_hash, "update-service");
    let action = Action::DataplaneUpdateService {
        service_id,
        vip,
        port: std::num::NonZeroU16::new(8080).expect("non-zero"),
        proto: overdrive_core::dataplane::backend_key::Proto::Tcp,
        backends: backends.clone(),
        correlation,
    };
    (service_id, vip, backends, action)
}

#[overdrive_test(guest = "cgroup-default")]
#[tokio::test]
async fn dispatch_completes_without_an_event_on_dataplane_ok() {
    let (_service_id, _vip, _backends, action) = sample_action();
    let dataplane: Arc<dyn Dataplane> = Arc::new(SimDataplane::new());
    let captured = CapturedEvents::default();
    let _guard = set_default(Registry::default().with(captured.clone()));

    dataplane_update_service::dispatch(&action, dataplane.as_ref()).await;

    let failures: Vec<String> = captured
        .entries()
        .into_iter()
        .filter(|e| e.starts_with("dataplane.update_service.failed"))
        .collect();
    assert!(failures.is_empty(), "a landed update must not emit a failure event: {failures:?}");
}

#[overdrive_test(guest = "cgroup-default")]
#[tokio::test]
async fn dispatch_logs_one_failure_event_on_dataplane_err() {
    let (service_id, vip, backends, action) = sample_action();
    let dataplane: Arc<dyn Dataplane> = Arc::new(FailingUpdateService);
    let captured = CapturedEvents::default();
    let _guard = set_default(Registry::default().with(captured.clone()));

    dataplane_update_service::dispatch(&action, dataplane.as_ref()).await;

    let failures: Vec<String> = captured
        .entries()
        .into_iter()
        .filter(|e| e.starts_with("dataplane.update_service.failed"))
        .collect();
    assert_eq!(failures.len(), 1, "exactly one failure event expected: {failures:?}");
    let event = &failures[0];
    assert!(event.contains(&format!("service_id={service_id}")), "names the service: {event}");
    assert!(
        event.contains(&format!("fingerprint={}", fingerprint(&vip, &backends))),
        "carries the backend-set fingerprint: {event}"
    );
    // `Display` of `DataplaneError::Busy` is fixed by the
    // `#[error("dataplane busy, retry later")]` attribute on the variant.
    assert!(
        event.contains(&DataplaneError::Busy.to_string()),
        "carries the dataplane error's Display: {event}"
    );
}

/// Regression: an IPv6 VIP must NOT silently fall through to
/// `Dataplane::update_service(0.0.0.0, ...)`. The dispatch must
/// short-circuit with a failure event whose
/// `reason` names the unsupported address family, and never call the
/// dataplane at all.
#[overdrive_test(guest = "cgroup-default")]
#[tokio::test]
async fn dispatch_rejects_ipv6_vip_with_a_failure_event() {
    let service_id = ServiceId::new(99).expect("ServiceId");
    let ipv6_vip =
        ServiceVip::new(IpAddr::V6(Ipv6Addr::new(0xfd, 0xc2, 0, 0, 0, 0, 0, 1))).expect("v6 VIP");
    let backends = vec![Backend {
        workload_identity: SpiffeId::new("spiffe://overdrive.local/workload/payments")
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
        backends,
        correlation,
    };
    let dataplane = Arc::new(SimDataplane::new());
    let captured = CapturedEvents::default();
    let _guard = set_default(Registry::default().with(captured.clone()));

    dataplane_update_service::dispatch(&action, dataplane.as_ref()).await;

    assert!(
        dataplane.service_vip_keys().is_empty(),
        "the dataplane must not have been programmed for an IPv6 VIP"
    );
    let failures: Vec<String> = captured
        .entries()
        .into_iter()
        .filter(|e| e.starts_with("dataplane.update_service.failed"))
        .collect();
    assert_eq!(failures.len(), 1, "exactly one failure event expected: {failures:?}");
    assert!(
        failures[0].contains("IPv4"),
        "reason should name the IPv4-only precondition, got: {}",
        failures[0]
    );
}
