//! Bugfix regression witness — a `D6` trust-bundle refresh failure inside the
//! `IssueSvid` action-shim executor must be SURFACED (structured `tracing::warn!`)
//! rather than silently swallowed, while staying NON-FATAL: the SVID was already
//! minted, audited, and held (K4), so the hold is not unwound and the dispatch
//! still returns `Ok`.
//!
//! Root cause (pre-fix): `dispatch_issue` did `if let Ok(bundle) =
//! ca.trust_bundle() { identity.set_bundle(bundle); }` — the `Err` arm was
//! silently discarded, directly contradicting the function's own comment
//! ("Surface it so a persistent refresh failure is not silently swallowed").
//! A `Ca::trust_bundle()` failure left `IdentityRead::current_bundle()`
//! stale/`None` with ZERO operator-visible signal.
//!
//! The fix keeps the refresh non-fatal (ADR-0067 D6 / K4: failing the dispatch
//! would unwind an already-audited hold AND, given the re-enqueue-on-dispatch-
//! failure machinery, cause a re-issue/audit storm every tick) but emits a
//! structured `issue_svid.trust_bundle_refresh_failed` `warn!` event. A richer
//! ObservationStore surface is deferred to issue #223.
//!
//! Port-to-port: the driving port is `run_convergence_tick` for `svid-lifecycle`
//! (driven with a test-only `FailingBundleCa` that issues + audits successfully
//! but fails `trust_bundle`). Observable outcomes:
//!   1. the tick returns `Ok` (the bundle failure is non-fatal),
//!   2. the alloc is HELD (K4 — the audited hold survives the bundle failure),
//!   3. `current_bundle()` is `None` (the failed refresh installed no bundle —
//!      the stale-bundle symptom is real and observable in state),
//!   4. the structured `issue_svid.trust_bundle_refresh_failed` event is emitted.
//!
//! Assertion 4 is the RED witness: pre-fix the warning is never emitted.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, noop_heartbeat, svid_lifecycle};
use overdrive_core::eval_broker::Evaluation;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::svid_lifecycle::SvidLifecycle;
use overdrive_core::reconcilers::{Reconciler, ReconcilerName, TargetResource};
use overdrive_core::traits::ca::{
    Ca, CaError, IntermediateHandle, RootCaHandle, SvidMaterial, SvidRequest, TrustBundle,
};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::identity_read::IdentityRead;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;

const SVID_LIFECYCLE: &str = "svid-lifecycle";
const WORKLOAD_NAME: &str = "payments";
const NODE_NAME: &str = "host-0";

// ---------------------------------------------------------------------------
// Tracing capture — minimal layer recording every event's `name:` + visited
// field values, mirroring the pattern in `probe_runner_boot_gate.rs`. Reused,
// not reinvented: `tracing-subscriber` is already a dev-dependency.
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
// Test double
// ---------------------------------------------------------------------------

/// A test-only `Ca` whose `trust_bundle` always fails — every other method
/// delegates to a real `SimCa` (so issuance + audit + hold all SUCCEED). This is
/// a legitimate test double (a failing driven-port adapter), NOT production
/// surface: it drives the `IssueSvid` action-shim executor onto the D6
/// bundle-refresh `Err` path while leaving the issuance itself green, so the
/// convergence tick still returns `Ok`.
struct FailingBundleCa {
    delegate: SimCa,
}

impl FailingBundleCa {
    fn new() -> Self {
        Self { delegate: SimCa::new(Arc::new(SimEntropy::new(0))) }
    }
}

impl Ca for FailingBundleCa {
    fn root(&self) -> Result<RootCaHandle, CaError> {
        self.delegate.root()
    }

    fn issue_intermediate(&self, node: &NodeId) -> Result<IntermediateHandle, CaError> {
        self.delegate.issue_intermediate(node)
    }

    fn issue_svid(&self, req: &SvidRequest) -> Result<SvidMaterial, CaError> {
        self.delegate.issue_svid(req)
    }

    fn trust_bundle(&self) -> Result<TrustBundle, CaError> {
        Err(CaError::signing_failed(
            "test double: trust_bundle refresh fails to drive the Err path",
        ))
    }
}

fn nid(s: &str) -> NodeId {
    NodeId::new(s).expect("valid NodeId")
}
fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}
fn wid(s: &str) -> WorkloadId {
    WorkloadId::new(s).expect("valid WorkloadId")
}
fn svid_target(w: &WorkloadId) -> TargetResource {
    TargetResource::new(&format!("job/{w}")).expect("valid target")
}
fn svid_reconciler_name() -> ReconcilerName {
    ReconcilerName::new(<SvidLifecycle as Reconciler>::NAME).expect("valid reconciler name")
}

async fn build_state(
    tmp: &TempDir,
    clock: Arc<SimClock>,
    obs: Arc<dyn ObservationStore>,
    identity: Arc<IdentityMgr>,
) -> AppState {
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
    runtime.register(svid_lifecycle()).await.expect("register svid-lifecycle");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    // The bundle-failing CA — issuance/audit/hold succeed, only `trust_bundle`
    // fails, driving the D6 refresh onto its `Err` path.
    let ca: Arc<dyn Ca> = Arc::new(FailingBundleCa::new());
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        clock,
        Arc::new(SimDataplane::new()),
        ca,
        identity,
        nid(NODE_NAME),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

async fn write_running_alloc(state: &AppState, w: &WorkloadId, a: &AllocationId, counter: u64) {
    let row = AllocStatusRow {
        alloc_id: a.clone(),
        workload_id: w.clone(),
        node_id: nid(NODE_NAME),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter, writer: nid(NODE_NAME) },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Job,
        listeners: Vec::new(),
        started_at: None,
        // Host-netns fixture — no canonical workload address (AllocStatusRowV2 additive field, GH #241).
        workload_addr: None,
    };
    state.obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write alloc row");
}

/// Bugfix witness — a `D6` trust-bundle refresh failure during `IssueSvid`
/// is surfaced via the structured `issue_svid.trust_bundle_refresh_failed`
/// warning, stays non-fatal (`Ok`), and leaves the audited hold intact (K4).
#[tokio::test]
async fn issue_svid_surfaces_bundle_refresh_failure_without_unwinding_hold() {
    // `set_default` is thread-local — no `#[serial]` needed.
    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let _guard = tracing::subscriber::set_default(subscriber);

    let tmp = TempDir::new().expect("tmpdir");
    let clock = Arc::new(SimClock::new());
    let obs =
        Arc::new(SimObservationStore::single_peer(nid(NODE_NAME), 0)) as Arc<dyn ObservationStore>;
    let identity = Arc::new(IdentityMgr::new(None));
    let state =
        build_state(&tmp, Arc::clone(&clock), Arc::clone(&obs), Arc::clone(&identity)).await;

    let workload = wid(WORKLOAD_NAME);
    let alloc = aid("payments-0");
    let target = svid_target(&workload);

    // One Running, unheld, no-retry alloc — the first-issue path. The reconciler
    // emits `IssueSvid`; the shim's executor issues + audits + holds successfully,
    // then the D6 bundle refresh calls the failing CA and fails.
    write_running_alloc(&state, &workload, &alloc, 1).await;
    state
        .runtime
        .broker()
        .submit(Evaluation { reconciler: svid_reconciler_name(), target: target.clone() });

    let now = std::time::Instant::now();
    let deadline = now + Duration::from_millis(100);

    let pending = {
        let mut broker = state.runtime.broker();
        broker.drain_pending()
    };
    let mut tick_result = None;
    for eval in pending {
        if eval.reconciler.as_str() != SVID_LIFECYCLE {
            continue;
        }
        tick_result = Some(
            run_convergence_tick(&state, &eval.reconciler, &eval.target, now, 0, deadline).await,
        );
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
    }

    // (1) The bundle refresh is NON-FATAL — the tick returns Ok despite the
    //     `trust_bundle()` failure. This kills the "fail the dispatch" mutant.
    let tick_result = tick_result.expect("a svid-lifecycle eval must have been pending");
    assert!(
        tick_result.is_ok(),
        "a D6 trust-bundle refresh failure must be non-fatal — the tick must return Ok, got {tick_result:?}"
    );

    // (2) K4 — the audited hold survives the bundle failure. This is the API-free
    //     assertion that kills the "drop the hold on bundle error" mutant.
    assert!(
        identity.svid_for(&alloc).is_some(),
        "K4: the audited hold must survive the trust-bundle refresh failure (the SVID was \
         already minted + audited + held before the refresh ran)"
    );

    // (3) The failed refresh installed NO bundle — the stale-bundle symptom is
    //     real and observable in state.
    assert!(
        identity.current_bundle().is_none(),
        "a failed trust-bundle refresh must install no bundle (current_bundle stays None)"
    );

    // (4) RED witness — the structured `issue_svid.trust_bundle_refresh_failed`
    //     event WAS emitted. Pre-fix the warning is never emitted (silent swallow),
    //     so this assertion fails. The `name:` slot lands on metadata().name().
    let events = collector.snapshot();
    let surfaced = events.iter().any(|row| {
        row.name == "issue_svid.trust_bundle_refresh_failed"
            || row.fields.get("name").map(String::as_str)
                == Some("issue_svid.trust_bundle_refresh_failed")
    });
    assert!(
        surfaced,
        "a trust-bundle refresh failure must be SURFACED via the structured \
         issue_svid.trust_bundle_refresh_failed event (pre-fix the Err arm was silently \
         swallowed); got events: {events:?}"
    );
}
