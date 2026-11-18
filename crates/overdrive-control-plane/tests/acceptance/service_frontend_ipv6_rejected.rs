//! S-01-F — IPv6 VIP rejected at the action-shim with a logged failure event, BEFORE the dataplane is ever called
//! (udp-service-support US-01; ADR-0060 D1a; criterion 6).
//!
//! Scenario SSOT:
//! `docs/feature/udp-service-support/distill/test-scenarios.md` S-01-F.
//!
//! Driving port: the action-shim `dispatch`. The distinct property this
//! test pins (vs the integration-tier `dispatch_rejects_ipv6_vip_with_a_failure_event`,
//! which uses a SimDataplane): the IPv6 rejection happens at the
//! operator-visible `ServiceFrontend::new` seam and is **NOT demoted to
//! a late opaque `DataplaneError`** — the dataplane adapter is never
//! reached. A panicking mock dataplane proves the short-circuit: if the
//! shim ever called `update_service` the test would panic.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tracing::subscriber::set_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

use overdrive_control_plane::action_shim::dataplane_update_service;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::fingerprint::fingerprint;
use overdrive_core::id::{ContentHash, CorrelationKey, ServiceId, ServiceVip, SpiffeId};
use overdrive_core::reconcilers::Action;
use overdrive_core::traits::dataplane::{
    Backend, Dataplane, DataplaneError, FlowEvent, PolicyKey, Verdict,
};

use overdrive_testing::placement;

placement!(host_pure);

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
}

#[tokio::test]
async fn ipv6_vip_rejected_at_action_shim_before_the_dataplane_is_reached() {
    let service_id = ServiceId::new(42).expect("ServiceId");
    let ipv6_vip = ServiceVip::new(IpAddr::V6(Ipv6Addr::LOCALHOST)).expect("v6 VIP");
    let backends = vec![Backend {
        alloc: SpiffeId::new("spiffe://overdrive.local/workload/dns/alloc/dns-0")
            .expect("SpiffeId"),
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

    let captured = CapturedEvents::default();
    let _guard = set_default(Registry::default().with(captured.clone()));

    // The panicking mock proves the dataplane is never reached.
    dataplane_update_service::dispatch(&action, dataplane.as_ref()).await;

    let failures: Vec<String> = captured
        .entries()
        .into_iter()
        .filter(|e| e.starts_with("dataplane.update_service.failed"))
        .collect();
    assert_eq!(
        failures.len(),
        1,
        "S-01-F: an IPv6 VIP must emit one dataplane.update_service.failed event: {failures:?}"
    );
}
