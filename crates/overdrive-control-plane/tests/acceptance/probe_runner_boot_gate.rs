//! S-SHCP-01-03d — composition-root `ProbeRunner` Earned-Trust
//! gate per ADR-0054 § 7.
//!
//! Asserts the structural composition-root invariant: the helper
//! `compose_and_probe_runner_gate` runs the
//! `ProbeRunner::probe()` Earned-Trust check after construction and
//! returns the typed `ControlPlaneError::ProbeRunnerBoot` (with the
//! source `ProbeRunnerError` preserved through `#[from]`) when the
//! sacrificial-loopback probe fails.
//!
//! Port-to-port shape: the AT enters through the public
//! `crate::probe_runner_boot::compose_and_probe_runner_gate` driving
//! port and asserts on the observable outcome — either an
//! `Arc<ProbeRunner>` is returned, or a typed
//! `ControlPlaneError::ProbeRunnerBoot { source: ProbeRunnerError }`
//! flows back. The structured `tracing::error!(name:
//! "health.startup.refused", ...)` event is captured via a
//! tracing-subscriber layer per the existing pattern in
//! `tests/integration/backend_discovery_bridge/boot_composition.rs`.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::sync::{Arc, Mutex};

use overdrive_control_plane::error::{ControlPlaneError, ProbeRunnerBootError};
use overdrive_control_plane::probe_runner_boot::compose_and_probe_runner_gate;
use overdrive_core::id::NodeId;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::prober::ProbeOutcome;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimExecProber, SimHttpProber, SimTcpProber};
use overdrive_worker::probe_runner::ProbeRunnerError;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;

// ---------------------------------------------------------------------------
// Tracing capture — minimal layer that records every event's
// `name:` + visited field values into a shared Vec so the test can
// assert on the structured event surface.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct EventRow {
    name: String,
    fields: std::collections::BTreeMap<String, String>,
}

#[derive(Default)]
struct FieldVisitor {
    fields: std::collections::BTreeMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}").trim_matches('"').to_owned());
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields.insert(field.name().to_owned(), value.to_owned());
    }
}

#[derive(Clone, Default)]
struct EventCollector {
    inner: Arc<Mutex<Vec<EventRow>>>,
}

impl EventCollector {
    fn snapshot(&self) -> Vec<EventRow> {
        self.inner.lock().expect("collector lock").clone()
    }
}

impl<S> Layer<S> for EventCollector
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.inner
            .lock()
            .expect("collector lock")
            .push(EventRow { name: event.metadata().name().to_owned(), fields: visitor.fields });
    }
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// S-SHCP-01-03d-PASS — the composition root runs the
/// `ProbeRunner::probe()` Earned-Trust gate against the wired TCP
/// adapter; when the adapter returns Pass (sim default with empty
/// queue, against the sacrificial loopback listener bound by the
/// gate itself) the helper returns the live `Arc<ProbeRunner>` and
/// the runtime is free to serve.
#[tokio::test]
async fn given_passing_tcp_prober_when_probe_gate_runs_then_returns_probe_runner() {
    let tcp = Arc::new(SimTcpProber::new()); // empty queue → Pass
    let http = Arc::new(SimHttpProber::new());
    let exec = Arc::new(SimExecProber::new());
    let clock: Arc<dyn Clock> = Arc::new(SimClock::default());
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("probe-gate-pass-test").expect("valid NodeId"),
        0,
    ));

    let result = compose_and_probe_runner_gate(tcp, http, exec, clock, obs).await;

    let runner = result.expect("probe-gate must succeed when TCP adapter returns Pass");
    assert_eq!(
        runner.active_alloc_count(),
        0,
        "freshly constructed ProbeRunner must hold zero alloc supervisors"
    );
}

/// S-SHCP-01-03d-REFUSE — when the TCP adapter returns Fail (sim
/// adapter pre-enqueued with `ProbeOutcome::Fail`), the helper:
///   1. returns `ControlPlaneError::ProbeRunnerBoot { source:
///      ProbeRunnerError::EarnedTrustFailure { .. } }`
///   2. emits the canonical structured `tracing::error!(name:
///      "health.startup.refused", reason = "probe_runner.earned_trust",
///      ...)` event.
///
/// The typed `#[from]` chain is the load-bearing contract per
/// `.claude/rules/development.md` § "Distinct failure modes get
/// distinct error variants" — callers can `matches!` on the
/// structured variant without `Display`-grepping a stringified
/// `Internal` message.
#[tokio::test]
async fn given_failing_tcp_prober_when_probe_gate_runs_then_returns_typed_refusal() {
    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let _guard = tracing::subscriber::set_default(subscriber);

    let tcp = Arc::new(SimTcpProber::new());
    tcp.enqueue_outcome(ProbeOutcome::Fail {
        reason: "synthetic injection: sacrificial loopback refused".to_owned(),
    });
    let http = Arc::new(SimHttpProber::new());
    let exec = Arc::new(SimExecProber::new());
    let clock: Arc<dyn Clock> = Arc::new(SimClock::default());
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("probe-gate-fail-test").expect("valid NodeId"),
        0,
    ));

    let result = compose_and_probe_runner_gate(tcp, http, exec, clock, obs).await;

    let Err(err) = result else {
        panic!("probe-gate must refuse when TCP adapter returns Fail");
    };
    let is_probe_boot = matches!(
        &err,
        ControlPlaneError::ProbeRunnerBoot(ProbeRunnerBootError::Probe {
            source: ProbeRunnerError::EarnedTrustFailure { .. },
        }),
    );
    assert!(
        is_probe_boot,
        "expected ControlPlaneError::ProbeRunnerBoot(Probe {{ EarnedTrustFailure }}); got: {err:?}",
    );

    let events = collector.snapshot();
    let refusal_events: Vec<_> = events
        .iter()
        .filter(|e| {
            e.name == "health.startup.refused"
                || e.fields.get("name").map(String::as_str) == Some("health.startup.refused")
        })
        .collect();
    let probe_refusal = refusal_events.iter().any(|row| {
        // tracing's `name:` slot for `event!(name: "...", ...)` lands
        // on metadata().name(); the synthetic `reason` field is on
        // the event payload.
        row.name == "health.startup.refused"
            && row.fields.get("reason").map(String::as_str) == Some("probe_runner.earned_trust")
    });
    assert!(
        probe_refusal,
        "expected health.startup.refused event with reason=probe_runner.earned_trust; got: {events:?}",
    );
}
