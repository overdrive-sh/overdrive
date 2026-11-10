//! Shared composition of the Service deploy stream acceptance tests of feature
//! `service-allocation-readiness-and-ending` (D10, D11; S-35, S-39 to S-42).
//!
//! One node composed the way the control plane composes it, with only the
//! driven ports substituted: the real `submit_workload` / `stop_workload` /
//! `restart_workload` handlers, the real `WorkloadLifecycle` and
//! `ServiceLifecycle` reconcilers driven through the runtime's convergence tick
//! (hydrate, `reconcile`, view write-through, shim `dispatch`), the real action
//! shim, a real `ProbeRunner`, and `Sim*` adapters at the clock, driver,
//! observation store, guest dial and guest network owner. No lifecycle state is
//! seeded: every row is authored by the production writers.
//!
//! The harness is a test-local composition, not a production seam. It owns no
//! assertion and adds no production API.

#![allow(dead_code)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Method, Request, header};
use axum::routing::post;
use overdrive_control_plane::AppState;
use overdrive_control_plane::guest_network::SharedGuestNetworkOwner as _;
use overdrive_control_plane::handlers::{restart_workload, stop_workload, submit_workload};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, run_convergence_tick_with_guest_network_provisioner_for_test,
};
use overdrive_control_plane::{service_lifecycle, workload_lifecycle};
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{DriverInput, ResourcesInput};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use overdrive_core::reconcilers::{Action, ReconcilerName, TargetResource, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::DriverType;
use overdrive_core::traits::observation_store::{AllocStatusRow, ObservationStore};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_stream_dial::{SimDialScript, SimGuestStreamDial};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::probe_runner::ProbeRunner;
use serde_json::Value;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tower::ServiceExt as _;

/// The guest network owner the node's `AppState` holds and every convergence tick
/// dispatches through.
type Owner = overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;

pub use crate::acceptance::hooked_driver::HookedDriver as ProbedDriver;

/// One composed node. Cheap to clone: every field is shared.
#[derive(Clone)]
pub struct Node {
    pub state: AppState,
    pub clock: Arc<SimClock>,
    pub obs: Arc<SimObservationStore>,
    pub guest_dial: Arc<SimGuestStreamDial>,
    pub driver: Arc<ProbedDriver>,
    pub owner: Arc<Owner>,
    _tmp: Arc<TempDir>,
}

/// A deploy stream opened through the NDJSON lane of `submit_workload`: the raw
/// lines it yields as they arrive, and the join handle that returns every line once
/// the stream ends.
pub struct Tap {
    pub lines: mpsc::UnboundedReceiver<String>,
    pub done: tokio::task::JoinHandle<Vec<String>>,
    seen: Vec<String>,
}

impl Tap {
    /// Drain the raw lines the stream has yielded so far.
    pub fn drain(&mut self) -> &[String] {
        while let Ok(line) = self.lines.try_recv() {
            self.seen.push(line);
        }
        &self.seen
    }

    /// The raw terminal line (the last line after `Accepted`), if the stream yielded one.
    pub fn terminal_raw(&mut self) -> Option<String> {
        self.drain().iter().skip(1).last().cloned()
    }

    /// The terminal line decoded as a `ServiceSubmitEvent`.
    pub fn terminal(&mut self) -> Option<overdrive_control_plane::streaming::ServiceSubmitEvent> {
        self.terminal_raw().map(|line| serde_json::from_str(&line).expect("a service event line"))
    }

    /// The first line (`Accepted`) as JSON.
    pub fn accepted(&mut self) -> Value {
        let first = self.drain().first().cloned().expect("an Accepted line");
        serde_json::from_str(&first).expect("json")
    }

    /// Whether the stream has ended (its response body is exhausted).
    pub fn ended(&self) -> bool {
        self.done.is_finished()
    }
}

/// Let every task that is ready run: no wall-clock sleep.
pub async fn settle() {
    for _ in 0..256 {
        tokio::task::yield_now().await;
    }
}

/// A Service spec with one TCP startup probe: `interval_seconds` apart,
/// `max_attempts` attempts (so the startup deadline is their product).
pub fn service_spec(id: &str, interval_seconds: u32, max_attempts: u32) -> ServiceSpecInput {
    ServiceSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        driver: DriverInput::Vm(overdrive_core::aggregate::VmInput {
            command: "/bin/sleep".into(),
            args: vec!["3600".into()],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
        listeners: vec![ListenerInput { port: 18081, protocol: "tcp".into() }],
        startup_probes: vec![ProbeDescriptor {
            idx: ProbeIdx::new(0),
            role: ProbeRole::Startup,
            mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".into(), port: 18999 },
            timeout_seconds: 1,
            interval_seconds,
            max_attempts,
            failure_threshold: None,
            success_threshold: None,
        }],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

fn ndjson_request(spec: SubmitSpecInput) -> Request<Body> {
    let body = serde_json::to_vec(&overdrive_control_plane::api::SubmitWorkloadRequest { spec })
        .expect("serialize the submit request");
    Request::builder()
        .method(Method::POST)
        .uri("/v1/workloads")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/x-ndjson")
        .body(Body::from(body))
        .expect("build the submit request")
}

impl Node {
    /// Boot one node. `seed` names the run; it is printed by the caller.
    pub async fn boot(seed: u64) -> Self {
        let tmp = Arc::new(tempfile::tempdir().expect("tempdir"));
        let node = NodeId::new("local").expect("node id");
        let clock = Arc::new(SimClock::new());
        let obs = Arc::new(SimObservationStore::single_peer(node.clone(), seed));
        let guest_dial = Arc::new(SimGuestStreamDial::new());
        // The guest refuses every probe until a test lets it accept.
        guest_dial.script_default(SimDialScript::Refuse(
            overdrive_core::guest_transport::dial::GuestDialError::Refused {
                errno: overdrive_core::guest_transport::wire::KernelErrno::ECONNREFUSED,
            },
        ));
        let driver = Arc::new(ProbedDriver::new(
            Arc::new(SimDriver::with_clock(DriverType::Vm, clock.clone())),
            Arc::new(ProbeRunner::new(
                Arc::new(SimTcpProber::new()),
                Arc::new(SimHttpProber::new()),
                clock.clone(),
                obs.clone(),
                guest_dial.clone(),
                Arc::new(overdrive_worker::GuestListenerBoard::new()),
            )),
            clock.clone(),
        ));
        let mut runtime =
            ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
        runtime.register(workload_lifecycle()).await.expect("register workload-lifecycle");
        runtime.register(service_lifecycle()).await.expect("register service-lifecycle");
        let store =
            Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("intent store"));
        let allocator = overdrive_control_plane::test_default_allocator(
            store.clone() as Arc<dyn overdrive_core::traits::intent_store::IntentStore>
        );
        let exec_wiring =
            overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(SimClock::new()));
        let mtls_worker = crate::acceptance::serve_ports::worker();
        mtls_worker
            .start_shared_owner()
            .await
            .expect("the fixture starts the shared intercept owner before allocation dispatch");
        let owner = Arc::new(Owner::default());
        owner.converge_shared().await.expect("the fixture owner reports its bound ports");
        let state = AppState::new(
            store,
            tmp.path().join("intent.redb"),
            obs.clone(),
            Arc::new(runtime),
            driver.clone(),
            clock.clone(),
            Arc::new(SimDataplane::new()),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(seed)))),
            Arc::new(IdentityMgr::new(None)),
            node,
            allocator,
            overdrive_control_plane::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            mtls_worker,
            owner.clone(),
            exec_wiring.gate(),
            Arc::new(overdrive_control_plane::guest_network::GuestAddressPool::new(
                "100.95.0.0/16".parse().expect("static guest prefix"),
                Ipv4Addr::new(100, 95, 0, 1),
                Arc::new(SimEntropy::new(0x295)),
            )),
        );
        assert!(exec_wiring.supervisor().open_after_boot(), "the EXEC gate opens after the owner");
        Self { state, clock, obs, guest_dial, driver, owner, _tmp: tmp }
    }

    /// Submit through the NDJSON lane and run one `workload-lifecycle` tick, which starts
    /// the first attempt: returns the stream tap and the attempt's allocation id.
    pub async fn deploy_and_start(
        &self,
        spec: &ServiceSpecInput,
        tick: u64,
    ) -> (Tap, AllocationId) {
        let tap = self.submit_streaming(spec).await;
        self.tick("workload-lifecycle", &spec.id, tick).await;
        (tap, self.current_alloc_id(&spec.id).await)
    }

    /// Make the guest accept probe connections from now on.
    pub fn let_the_guest_accept(&self) {
        self.guest_dial.script_default(SimDialScript::Accept { reply: Vec::new() });
    }

    /// A guest that takes `boot` to start on the injected clock.
    pub fn set_boot_delay(&self, boot: Duration) {
        self.driver.set_boot_delay(boot);
    }

    /// Submit through the NDJSON lane of the production handler and tap the stream.
    pub async fn submit_streaming(&self, spec: &ServiceSpecInput) -> Tap {
        self.submit_streaming_input(SubmitSpecInput::Service(spec.clone())).await
    }

    /// Submit any spec through the NDJSON lane of the production handler.
    pub async fn submit_streaming_input(&self, spec: SubmitSpecInput) -> Tap {
        let router = Router::new()
            .route("/v1/workloads", post(submit_workload))
            .with_state(self.state.clone());
        let response = router.oneshot(ndjson_request(spec)).await.expect("router oneshot");
        assert!(response.status().is_success(), "the submit is accepted");
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        let done = tokio::spawn(async move {
            use http_body_util::BodyExt as _;
            let mut body = response.into_body();
            let mut lines = Vec::new();
            let mut buf = String::new();
            while let Some(frame) = body.frame().await {
                let Ok(frame) = frame else { break };
                if let Ok(data) = frame.into_data() {
                    buf.push_str(std::str::from_utf8(&data).expect("utf8"));
                    while let Some(pos) = buf.find('\n') {
                        let line = buf[..pos].trim().to_owned();
                        if !line.is_empty() {
                            let _ = tx.send(line.clone());
                            lines.push(line);
                        }
                        buf = buf[pos + 1..].to_owned();
                    }
                }
            }
            lines
        });
        let mut tap = Tap { lines: rx, done, seen: Vec::new() };
        settle().await;
        let _ = tap.drain();
        tap
    }

    /// One convergence tick of `owner` (`workload-lifecycle` or `service-lifecycle`)
    /// over `workload`, at the injected clock's current instant.
    pub async fn tick(&self, owner: &str, workload: &str, count: u64) {
        let target = TargetResource::new(&format!("workload/{workload}")).expect("target");
        run_convergence_tick_with_guest_network_provisioner_for_test(
            &self.state,
            &ReconcilerName::new(owner).expect("reconciler name"),
            &target,
            self.clock.now(),
            count,
            self.clock.now() + Duration::from_secs(30),
            self.owner.as_ref(),
        )
        .await
        .expect("convergence tick");
    }

    /// Advance the injected clock `seconds`, one second at a time, running every task
    /// that becomes ready after each step.
    pub async fn advance_secs(&self, seconds: u64) {
        for _ in 0..seconds {
            settle().await;
            self.clock.tick(Duration::from_secs(1));
            settle().await;
        }
    }

    /// Dispatch actions through the production action shim, as the runtime does.
    pub async fn dispatch(&self, actions: Vec<Action>, tick: u64) {
        let now = self.clock.now();
        let ctx = TickContext {
            now,
            now_unix: overdrive_core::UnixInstant::from_unix_duration(self.clock.unix_now()),
            tick,
            deadline: now + Duration::from_secs(30),
        };
        overdrive_control_plane::action_shim::dispatch_with_workflow_intent(
            actions,
            &self.state,
            &ctx,
        )
        .await
        .expect("shim dispatch");
    }

    /// Every allocation row of `workload`, ordered by allocation id.
    pub async fn rows_of(&self, workload: &str) -> Vec<AllocStatusRow> {
        let id = WorkloadId::new(workload).expect("workload id");
        let mut rows: Vec<AllocStatusRow> = self
            .obs
            .alloc_status_rows()
            .await
            .expect("alloc_status_rows")
            .into_iter()
            .filter(|row| row.workload_id == id)
            .collect();
        rows.sort_by(|a, b| a.alloc_id.as_str().cmp(b.alloc_id.as_str()));
        rows
    }

    /// The `alloc_id` of the workload's current attempt (the reconciler's own rule).
    pub async fn current_alloc_id(&self, workload: &str) -> AllocationId {
        let rows = self.rows_of(workload).await;
        let refs: Vec<&AllocStatusRow> = rows.iter().collect();
        overdrive_reconcilers::current_alloc(&refs).expect("a current attempt").alloc_id.clone()
    }

    /// Record the operator's stop intent through the production handler.
    pub async fn operator_stops(&self, workload: &str) {
        let _ack = stop_workload(State(self.state.clone()), Path(workload.to_owned()))
            .await
            .expect("stop_workload");
    }

    /// `overdrive workload restart` through the production handler.
    pub async fn operator_restarts(&self, workload: &str) {
        let _ack = restart_workload(State(self.state.clone()), Path(workload.to_owned()))
            .await
            .expect("restart_workload");
    }
}
