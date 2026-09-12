//! ADR-0101: production policy owner -> action dispatch -> observed rows -> consumers.
//! Intent and prober ports supply inputs; no allocation, probe, terminal,
//! backend, fingerprint, or consumer-cache row is fabricated. The original
//! seed-257_209 diagnostic remains unchanged in its separate test binary.
//!
//! State model: submitted/no allocation -> Running/no startup decision ->
//! Stable OR terminal startup failure. Readiness independently follows
//! unobserved -> Pass threshold -> Fail -> Pass recovery. WorkloadLifecycle may
//! restart the same allocation identity after Failed; ServiceLifecycle's existing
//! terminal veto remains. A rejected row write leaves observation unchanged,
//! although dispatch drains terminal publication and persists the policy View.
//! Subsequent reconciliation repairs observation; consumer convergence follows
//! asynchronously. Repeated ticks are self-loops except readiness Pass counting.

#![expect(
    clippy::doc_markdown,
    reason = "repository-mandated CONTRACT_SHAPE tokens and narrative scenario names"
)]
#![expect(
    clippy::future_not_send,
    reason = "bounded current-thread scenarios own a Send-but-not-Sync lag-aware subscription; these futures never cross threads"
)]

use async_trait::async_trait;
use futures::{FutureExt, StreamExt};
use overdrive_control_plane::action_shim::ShimError;
use overdrive_control_plane::dns_responder::name_index::NameIndex;
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::listener_facts::ListenerFactStore;
use overdrive_control_plane::mtls_resolve_adapter::ServiceBackendsResolve;
use overdrive_control_plane::reconciler_runtime::{
    ConvergenceError, ReconcilerRuntime, run_convergence_tick,
};
use overdrive_control_plane::view_store::redb::RedbViewStore;
use overdrive_control_plane::{
    AppState, service_lifecycle, service_map_hydrator, workload_lifecycle,
};
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, ResourcesInput, ServiceV2, WorkloadIntent,
};
use overdrive_core::api::{ListenerInput, ServiceSpecInput};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{AllocationId, MeshServiceName, NodeId, ServiceId, ServiceVip};
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use overdrive_core::reconcilers::{Action, ReconcilerName, TargetResource, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, Resources,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_core::traits::observation_store::{
    AllocState, LagAwareSubscription, LogicalTimestamp, ObservationRow, ObservationStore,
    ObservationStoreError, ServiceBackendRow, SubscriptionEvent,
};
use overdrive_core::traits::prober::ProbeOutcome;
use overdrive_reconcilers::AnyReconcilerView;
use overdrive_sim::adapters::{
    ca::SimCa,
    clock::SimClock,
    dataplane::SimDataplane,
    driver::SimDriver,
    entropy::SimEntropy,
    observation_store::SimObservationStore,
    probers::{SimExecProber, SimHttpProber, SimTcpProber},
};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::probe_runner::ProbeRunner;
use std::{
    collections::BTreeSet,
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
    time::Duration,
};

const HOST: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 10);
const WORKLOAD: &str = "backend-projection";

/// Mirrors all production driver-to-ProbeRunner lifecycle hooks. SimDriver
/// substitutes only the external process; lifecycle rows come from dispatch.
struct ProbedDriver {
    inner: SimDriver,
    probes: ProbeRunner,
}

#[async_trait]
impl Driver for ProbedDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Exec
    }
    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        self.inner.start(spec).await
    }
    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        self.inner.stop(handle).await
    }
    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        self.inner.status(handle).await
    }
    async fn resize(
        &self,
        handle: &AllocationHandle,
        resources: Resources,
    ) -> Result<(), DriverError> {
        self.inner.resize(handle, resources).await
    }
    async fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        self.inner.release_for_exit_emission(handle).await;
    }
    fn release_supervision(&self, alloc: &AllocationId) {
        self.inner.release_supervision(alloc);
    }
    fn on_alloc_running(&self, spec: &AllocationSpec) {
        self.probes.start_alloc(spec);
    }
    fn on_alloc_terminal(&self, alloc: &AllocationId) {
        self.probes.stop_alloc(alloc);
    }
    fn on_alloc_stable(&self, alloc: &AllocationId) {
        self.probes.stop_role(alloc, ProbeRole::Startup);
    }
}

struct World {
    state: AppState,
    obs: Arc<SimObservationStore>,
    clock: Arc<SimClock>,
    dataplane: Arc<SimDataplane>,
    driver: Arc<ProbedDriver>,
    tcp: Arc<SimTcpProber>,
    http: Arc<SimHttpProber>,
    target: TargetResource,
    name: MeshServiceName,
    frontend: Ipv4Addr,
    vip: Option<ServiceVip>,
    listeners: Vec<(ServiceId, u16, Proto)>,
    views: Arc<RedbViewStore>,
    events: LagAwareSubscription,
    history: Vec<ObservationRow>,
    seed: u64,
    tick: u64,
    // Declared last: stores/tasks release their handles before TempDir cleanup.
    directory: tempfile::TempDir,
}

fn input(listeners: &[(u16, &str)], replicas: u32) -> ServiceSpecInput {
    ServiceSpecInput {
        id: WORKLOAD.into(),
        replicas,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/sleep".into(),
            args: vec!["3600".into()],
        }),
        listeners: listeners
            .iter()
            .map(|(port, protocol)| ListenerInput { port: *port, protocol: (*protocol).into() })
            .collect(),
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

fn startup() -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".into(), port: 18999 },
        timeout_seconds: 1,
        interval_seconds: 1,
        max_attempts: 3,
        failure_threshold: None,
        success_threshold: None,
        inferred: false,
    }
}

fn readiness(threshold: u32) -> ProbeDescriptor {
    ProbeDescriptor {
        role: ProbeRole::Readiness,
        mechanic: ProbeMechanic::Http { path: "/ready".into(), port: 18998, host: None },
        success_threshold: Some(threshold),
        ..startup()
    }
}

impl World {
    async fn new(seed: u64, input: ServiceSpecInput, allocate: bool) -> Self {
        print_seed(seed);
        let directory = tempfile::tempdir().unwrap();
        let node = NodeId::new("local").unwrap();
        let clock = Arc::new(SimClock::new());
        let obs = Arc::new(SimObservationStore::single_peer(node.clone(), seed));
        let tcp = Arc::new(SimTcpProber::new());
        let http = Arc::new(SimHttpProber::new());
        let driver = Arc::new(ProbedDriver {
            inner: SimDriver::with_clock(DriverType::Exec, clock.clone()),
            probes: ProbeRunner::new(
                tcp.clone(),
                http.clone(),
                Arc::new(SimExecProber::new()),
                clock.clone(),
                obs.clone(),
            ),
        });
        let views = Arc::new(RedbViewStore::open(directory.path()).unwrap());
        let runtime = registered_runtime(directory.path(), views.clone()).await;
        let store = Arc::new(LocalIntentStore::open(directory.path().join("intent.redb")).unwrap());
        let allocator =
            overdrive_control_plane::test_default_allocator(store.clone() as Arc<dyn IntentStore>);
        let dataplane = Arc::new(SimDataplane::new());
        let state = AppState::new(
            store,
            directory.path().join("intent.redb"),
            obs.clone(),
            Arc::new(runtime),
            driver.clone(),
            clock.clone(),
            dataplane.clone(),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(seed)))),
            Arc::new(IdentityMgr::new(None)),
            node,
            allocator,
            overdrive_control_plane::test_empty_listener_facts(),
            HOST,
        );
        // Same validated driving ports as submit: allocate frontend/VIP, archive
        // canonical intent, and rebuild listener facts from that real intent.
        let svc = ServiceV2::from_submit(input).unwrap();
        let declared = svc.listeners.clone();
        let name = MeshServiceName::new(&format!("{WORKLOAD}.svc.overdrive.local")).unwrap();
        let frontend = state.frontend_addr_allocator.assign(&name).unwrap();
        let key = IntentKey::for_workload(&svc.id);
        let intent = WorkloadIntent::Service(svc);
        let mut listeners = Vec::new();
        let assigned_vip = if allocate {
            let vip = state
                .allocator
                .lock()
                .await
                .allocate(*intent.spec_digest().unwrap().as_bytes())
                .await
                .unwrap();
            listeners = declared
                .iter()
                .map(|listener| {
                    (
                        ServiceId::derive(&vip, listener.port, listener.protocol, "service-map"),
                        listener.port.get(),
                        listener.protocol,
                    )
                })
                .collect();
            listeners.sort_by_key(|(id, _, _)| *id);
            Some(vip)
        } else {
            None
        };
        let bytes = intent.archive_for_store().unwrap();
        state.store.put(key.as_bytes(), bytes.as_ref()).await.unwrap();
        *state.listener_facts.lock().await = ListenerFactStore::rebuild_from_intent(
            &state.store,
            &state.intent_redb_path,
            &state.allocator,
        )
        .await
        .unwrap();
        let events = obs.subscribe_all_events().await.unwrap();
        Self {
            state,
            obs,
            clock,
            dataplane,
            driver,
            tcp,
            http,
            target: TargetResource::new(&format!("workload/{WORKLOAD}")).unwrap(),
            name,
            frontend,
            vip: assigned_vip,
            listeners,
            views,
            events,
            history: Vec::new(),
            seed,
            tick: 10,
            directory,
        }
    }

    async fn run(&mut self, owner: &str) {
        self.run_target(ReconcilerName::new(owner).unwrap(), self.target.clone()).await;
    }

    async fn run_target(&mut self, owner: ReconcilerName, target: TargetResource) {
        self.tick += 1;
        run_convergence_tick(
            &self.state,
            &owner,
            &target,
            self.clock.now(),
            self.tick,
            self.clock.now() + Duration::from_secs(30),
        )
        .await
        .unwrap();
        self.drain_events();
    }

    fn drain_events(&mut self) {
        while let Some(Some(event)) = self.events.next().now_or_never() {
            match event {
                SubscriptionEvent::Row(row) => {
                    if let ObservationRow::ServiceBackend(backend_row) = &row {
                        self.assert_vip(backend_row);
                    }
                    self.history.push(row);
                }
                SubscriptionEvent::Lagged { missed } => {
                    panic!("seed={}: publication oracle lost {missed} rows", self.seed)
                }
            }
        }
    }

    async fn reload_runtime(&mut self) {
        // The normal constructor/register load path reads this program's real
        // redb-written View. No old version, migration, or fabricated blob.
        let runtime = registered_runtime(self.directory.path(), self.views.clone()).await;
        self.state.runtime = Arc::new(runtime);
    }

    async fn planned_service_actions(&self) -> Vec<Action> {
        use overdrive_control_plane::reconciler_runtime::{
            hydrate_actual_for_test, hydrate_desired_for_test,
        };
        let reconciler = service_lifecycle();
        let actual = hydrate_actual_for_test(&reconciler, &self.target, &self.state).await.unwrap();
        let desired =
            hydrate_desired_for_test(&reconciler, &self.target, &self.state).await.unwrap();
        let mut views = self
            .state
            .runtime
            .loaded_service_lifecycle_views_for_test(
                &ReconcilerName::new("service-lifecycle").unwrap(),
            )
            .unwrap();
        let view =
            AnyReconcilerView::ServiceLifecycle(views.remove(&self.target).unwrap_or_default());
        let tick = TickContext {
            now: self.clock.now(),
            now_unix: overdrive_core::UnixInstant::from_clock(self.clock.as_ref()),
            tick: self.tick + 1,
            deadline: self.clock.now() + Duration::from_secs(30),
        };
        reconciler.reconcile(&desired, &actual, &view, &tick).0
    }

    async fn rows(&self) -> Vec<ServiceBackendRow> {
        let mut rows = self.obs.all_service_backends_rows().await.unwrap();
        for row in &rows {
            self.assert_vip(row);
        }
        rows.sort_by_key(|row| row.service_id);
        rows
    }

    fn assert_vip(&self, row: &ServiceBackendRow) {
        assert_eq!(
            Some(row.vip),
            self.vip.as_ref().and_then(ServiceVip::try_as_ipv4),
            "seed={}: every published row, including empty/withdrawal, uses the allocator VIP",
            self.seed
        );
    }

    fn assert_planned_stamps(&self, actions: &[Action], prior: &[ServiceBackendRow]) {
        let mut writes = 0;
        for action in actions {
            if let Action::WriteServiceBackendRow { row, .. } = action {
                writes += 1;
                self.assert_vip(row);
                let previous = prior.iter().find(|prior| prior.service_id == row.service_id);
                assert_eq!(
                    row.updated_at,
                    LogicalTimestamp::dominating(
                        self.tick + 1,
                        self.state.node_id.clone(),
                        previous.map(|row| &row.updated_at),
                    ),
                    "seed={}: exact owner writer, evaluation tick floor and observed same-key prior",
                    self.seed
                );
            }
        }
        assert!(writes > 0, "seed={}: stamp oracle must observe a real planned write", self.seed);
    }

    async fn advance_probes(&self) {
        self.settle().await;
        self.clock.tick(Duration::from_millis(1000 + self.seed % 11));
        self.settle().await;
    }

    async fn settle(&self) {
        for _ in 0..(64 + self.seed % 17) {
            tokio::task::yield_now().await;
        }
    }

    async fn hydrate_published(&mut self) {
        // Drive only the consumer's real queued handoff, never manufacture its
        // evaluation. Other owners remain under this scenario's explicit schedule.
        let pending = self.state.runtime.broker().drain_pending(
            usize::MAX,
            &BTreeSet::new(),
            self.clock.now(),
            overdrive_core::UnixInstant::from_clock(&*self.clock),
        );
        let consumer: Vec<_> = pending
            .into_iter()
            .filter(|(eval, _)| eval.reconciler.as_str() == "service-map-hydrator")
            .map(|(eval, _)| eval)
            .collect();
        assert_eq!(
            consumer.len(),
            self.listeners.len(),
            "seed={}: one hydrator handoff per changed listener",
            self.seed
        );
        for eval in consumer {
            self.run_target(eval.reconciler, eval.target).await;
        }
    }

    fn assert_shape(&self, rows: &[ServiceBackendRow], replicas: usize, healthy: bool) {
        assert_eq!(
            rows.iter().map(|row| row.service_id).collect::<Vec<_>>(),
            self.listeners.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
            "seed={}: exact current listener universe",
            self.seed
        );
        for (row, (_, port, _)) in rows.iter().zip(&self.listeners) {
            assert_eq!(
                row.backends.len(),
                replicas,
                "seed={}: exact Running allocation universe",
                self.seed
            );
            assert!(
                row.backends.iter().all(|backend| backend.healthy == healthy
                    && backend.weight == 1
                    && backend.addr == std::net::SocketAddr::V4(SocketAddrV4::new(HOST, *port))),
                "seed={}: listener-specific address, health and weight: {row:?}",
                self.seed
            );
            assert_eq!(
                row.backends.iter().map(|backend| &backend.alloc).collect::<BTreeSet<_>>().len(),
                replicas
            );
        }
    }
}

#[expect(
    clippy::print_stderr,
    reason = "seeded invariant failures must print their reproduction seed"
)]
fn print_seed(seed: u64) {
    eprintln!("service-backend-projection seed={seed}");
}

async fn registered_runtime(
    directory: &std::path::Path,
    views: Arc<RedbViewStore>,
) -> ReconcilerRuntime {
    let mut runtime = ReconcilerRuntime::new(directory, views).unwrap();
    runtime.register(workload_lifecycle()).await.unwrap();
    runtime.register(service_lifecycle()).await.unwrap();
    runtime.register(service_map_hydrator(HOST)).await.unwrap();
    runtime.register(overdrive_control_plane::vm_reclamation()).await.unwrap();
    runtime
}

/// CONTRACT_SHAPE: bounded-change.
/// Existing green control, separately reported from the multi-listener RED:
/// no readiness means eligible before startup passes; equal rows do not churn.
#[tokio::test(flavor = "current_thread")]
async fn single_listener_preterminal_control_is_healthy_and_stable() {
    let mut spec = input(&[(18081, "tcp")], 1);
    spec.startup_probes = vec![startup()];
    let mut world = World::new(257_223, spec, true).await;
    world.run("workload-lifecycle").await;
    world.run("service-lifecycle").await;
    let rows = world.rows().await;
    world.assert_shape(&rows, 1, true);
    for _ in 0..3 {
        world.run("service-lifecycle").await;
    }
    assert_eq!(world.rows().await, rows);
}

/// CONTRACT_SHAPE: bounded-change.
/// Start and operator Stop keep SVID wakes and also wake the sole backend
/// projection owner. Repeated operator-stopped reconciliation does not restart.
#[tokio::test(flavor = "current_thread")]
async fn workload_start_and_stop_wake_the_service_projection_owner() {
    let mut world = World::new(257_226, input(&[(18081, "tcp")], 1), true).await;
    world.run("workload-lifecycle").await;
    assert_service_and_svid_wakes(&world);
    world.run("service-lifecycle").await;
    world.assert_shape(&world.rows().await, 1, true);
    world.state.runtime.broker().drain_pending(
        usize::MAX,
        &BTreeSet::new(),
        world.clock.now(),
        overdrive_core::UnixInstant::from_clock(&*world.clock),
    );
    let workload = overdrive_core::WorkloadId::new(WORKLOAD).unwrap();
    let stop_key = IntentKey::for_workload_stop(&workload);
    world.state.store.put(stop_key.as_bytes(), &[]).await.unwrap();
    world.run("workload-lifecycle").await;
    assert_eq!(world.obs.alloc_status_rows().await.unwrap()[0].state, AllocState::Terminated);
    assert_service_and_svid_wakes(&world);
    world.run("service-lifecycle").await;
    world.assert_shape(&world.rows().await, 0, false);
    let stopped = world.obs.alloc_status_rows().await.unwrap();
    for _ in 0..3 {
        world.run("workload-lifecycle").await;
        world.run("service-lifecycle").await;
    }
    assert_eq!(world.obs.alloc_status_rows().await.unwrap(), stopped);
}

fn assert_service_and_svid_wakes(world: &World) {
    let mut pending = Vec::new();
    loop {
        let round = world.state.runtime.broker().drain_pending(
            usize::MAX,
            &BTreeSet::new(),
            world.clock.now(),
            overdrive_core::UnixInstant::from_clock(&*world.clock),
        );
        if round.is_empty() {
            break;
        }
        pending.extend(round);
    }
    for owner in ["service-lifecycle", "svid-lifecycle"] {
        assert!(
            pending
                .iter()
                .any(|(eval, _)| eval.reconciler.as_str() == owner && eval.target == world.target),
            "seed={}: required {owner} handoff for the same workload: {pending:?}",
            world.seed
        );
    }
}

/// CONTRACT_SHAPE: bounded-change.
/// Real liveness failure retains the normal Stop -> WorkloadLifecycle Restart
/// budget, then that owner's FinalizeFailed also wakes ServiceLifecycle. No
/// retry count, terminal row, or restart view is seeded to reach exhaustion.
#[tokio::test(flavor = "current_thread")]
async fn liveness_restart_budget_and_finalization_keep_projection_handoffs() {
    use overdrive_core::transition_reason::{ServiceFailureReason, TerminalCondition};
    let mut spec = input(&[(18081, "tcp")], 1);
    spec.liveness_probes = vec![ProbeDescriptor {
        role: ProbeRole::Liveness,
        failure_threshold: Some(1),
        ..startup()
    }];
    let mut world = World::new(257_227, spec, true).await;
    for _ in 0..32 {
        world.tcp.enqueue_outcome(ProbeOutcome::Fail { reason: "seeded liveness refusal".into() });
    }
    world.run("workload-lifecycle").await;
    world.run("service-lifecycle").await;
    world.assert_shape(&world.rows().await, 1, true);
    let original = world.obs.alloc_status_rows().await.unwrap().remove(0).alloc_id;
    let mut restarts = 0;
    for _ in 0..12 {
        world.advance_probes().await;
        world.run("service-lifecycle").await;
        world.state.runtime.broker().drain_pending(
            usize::MAX,
            &BTreeSet::new(),
            world.clock.now(),
            overdrive_core::UnixInstant::from_clock(&*world.clock),
        );
        world.clock.tick(Duration::from_secs(60));
        world.run("workload-lifecycle").await;
        let row = world.obs.alloc_status_rows().await.unwrap().remove(0);
        assert_eq!(row.alloc_id, original, "normal same-ID restart contract");
        if row.state == AllocState::Running {
            restarts += 1;
            assert_service_and_svid_wakes(&world);
        } else if matches!(
            row.terminal,
            Some(TerminalCondition::ServiceFailed {
                reason: ServiceFailureReason::LivenessProbeFailed { attempts: 5, .. }
            })
        ) {
            assert_eq!(restarts, 5, "the existing WorkloadLifecycle restart budget remains five");
            assert_service_and_svid_wakes(&world);
            world.run("service-lifecycle").await;
            world.assert_shape(&world.rows().await, 0, false);
            return;
        }
    }
    panic!("seed=257_227: liveness budget did not converge within 12 owner cycles");
}

/// CONTRACT_SHAPE: bounded-change.
/// Structural complement over facts hydrated from the composed owner path:
/// full changed-listener write/handoff pairs are ordered by ServiceId and precede
/// the first deciding terminal action. Exact stamps cover first publication,
/// tick-ahead readback, and prior-ahead readback after normal runtime/tick reload.
/// Correlation identity is content-derived from the allocator-issued VIP.
#[tokio::test(flavor = "current_thread")]
async fn deciding_tick_orders_complete_row_handoffs_before_failure() {
    use overdrive_core::dataplane::fingerprint::fingerprint;
    use overdrive_core::id::{ContentHash, CorrelationKey};
    let mut spec = input(&[(18082, "udp"), (18081, "tcp")], 1);
    spec.startup_probes = vec![startup()];
    let mut world = World::new(257_225, spec, true).await;
    for _ in 0..32 {
        world.tcp.enqueue_outcome(ProbeOutcome::Fail { reason: "seeded refusal".into() });
    }
    world.run("workload-lifecycle").await;
    let initial = world.rows().await;
    assert!(initial.is_empty());
    world.assert_planned_stamps(&world.planned_service_actions().await, &initial);
    world.run("service-lifecycle").await;
    for _ in 0..2 {
        world.advance_probes().await;
        world.run("service-lifecycle").await;
    }
    world.advance_probes().await;
    let prior = world.rows().await;
    assert!(!prior.is_empty(), "baseline must contain production-authored rows");
    assert!(prior.iter().all(|row| row.updated_at.counter < world.tick + 1));
    world.assert_planned_stamps(&world.planned_service_actions().await, &prior);

    // The production convergence loop starts its process-local tick at zero
    // (control-plane/lib.rs), while observation rows survive. Re-register the
    // real persisted View, then model only that existing tick reset through
    // the same tick argument used by run_convergence_tick. No row is seeded.
    world.reload_runtime().await;
    world.tick = 0;
    assert_eq!(world.rows().await, prior, "reload must not manufacture a prior stamp");
    assert!(prior.iter().all(|row| row.updated_at.counter > world.tick + 1));
    let actions = world.planned_service_actions().await;
    world.assert_planned_stamps(&actions, &prior);
    let terminal = actions
        .iter()
        .position(|action| matches!(action, Action::FinalizeFailed { .. }))
        .expect("real deciding startup action");
    let pairs = &actions[..terminal];
    assert_eq!(
        pairs.len(),
        world.listeners.len() * 2,
        "seed=257_225: complete withdrawal/handoff group before failure: {actions:?}"
    );
    for (pair, (id, port, _)) in pairs.chunks_exact(2).zip(&world.listeners) {
        let Action::WriteServiceBackendRow { row, correlation } = &pair[0] else {
            panic!("write starts each pair")
        };
        assert_eq!(row.service_id, *id);
        assert_eq!(row.backends.len(), 1);
        assert!(!row.backends[0].healthy);
        assert_eq!(row.backends[0].addr.port(), *port);
        world.assert_vip(row);
        let vip = world.vip.as_ref().unwrap();
        let target = format!("service-lifecycle/backends/{id}");
        let hash = ContentHash::of(fingerprint(vip, &row.backends).to_le_bytes());
        assert_eq!(
            *correlation,
            CorrelationKey::derive(&target, &hash, "write-service-backend-row")
        );
        assert!(
            matches!(&pair[1], Action::EnqueueEvaluation { reconciler, target } if reconciler.as_str() == "service-map-hydrator" && target.as_str() == format!("service/{id}"))
        );
    }
    // Commit through the real serial dispatcher as well; the action-level
    // oracle above never substitutes for resulting observed behavior.
    let start = world.history.len();
    world.run("service-lifecycle").await;
    let events = &world.history[start..];
    let failed = events.iter().position(|row| matches!(row, ObservationRow::AllocStatus(row) if row.state == AllocState::Failed)).unwrap();
    let withdrawals: Vec<_> = events[..failed]
        .iter()
        .filter_map(|row| match row {
            ObservationRow::ServiceBackend(row) => Some(row),
            _ => None,
        })
        .collect();
    assert_eq!(
        withdrawals.iter().map(|row| row.service_id).collect::<Vec<_>>(),
        world.listeners.iter().map(|(id, _, _)| *id).collect::<Vec<_>>()
    );
    assert!(withdrawals.iter().all(|row| row.backends.iter().all(|backend| !backend.healthy)));
}

/// CONTRACT_SHAPE: bounded-change.
/// A failed deciding withdrawal does not prevent terminal publication, and the
/// normally reloaded policy View still vetoes a same-ID restart. A subsequent
/// reconciliation repairs the rejected row from observation, not emit memory.
#[tokio::test(flavor = "current_thread")]
async fn failed_withdrawal_drains_terminal_and_repairs_after_view_reload() {
    let mut spec = input(&[(18081, "tcp")], 1);
    spec.startup_probes = vec![startup()];
    let mut world = World::new(257_224, spec, true).await;
    for _ in 0..32 {
        world.tcp.enqueue_outcome(ProbeOutcome::Fail { reason: "seeded refusal".into() });
    }
    world.run("workload-lifecycle").await;
    world.run("service-lifecycle").await;
    world.assert_shape(&world.rows().await, 1, true);
    for _ in 0..2 {
        world.advance_probes().await;
        world.run("service-lifecycle").await;
    }
    assert_eq!(world.obs.alloc_status_rows().await.unwrap()[0].state, AllocState::Running);
    world.advance_probes().await;
    world.obs.inject_write_failure(ObservationStoreError::Unreachable {
        peer: "deciding-withdrawal-fault".into(),
    });
    world.tick += 1;
    let outcome = run_convergence_tick(
        &world.state,
        &ReconcilerName::new("service-lifecycle").unwrap(),
        &world.target,
        world.clock.now(),
        world.tick,
        world.clock.now() + Duration::from_secs(30),
    )
    .await;
    assert!(
        matches!(outcome, Err(ConvergenceError::Shim(ShimError::Observation(ObservationStoreError::Unreachable { ref peer }))) if peer == "deciding-withdrawal-fault"),
        "exact injected error: {outcome:?}"
    );
    world.drain_events();
    let ended = world.obs.alloc_status_rows().await.unwrap().remove(0);
    assert_eq!(
        ended.state,
        AllocState::Failed,
        "per-action isolation must drain FinalizeFailed despite rejected withdrawal"
    );
    world.assert_shape(&world.rows().await, 1, true);
    let before = world
        .state
        .runtime
        .loaded_service_lifecycle_views_for_test(&ReconcilerName::new("service-lifecycle").unwrap())
        .unwrap();
    assert!(before[&world.target].terminal_announced.contains(&ended.alloc_id));
    world.reload_runtime().await;
    assert_eq!(
        world
            .state
            .runtime
            .loaded_service_lifecycle_views_for_test(
                &ReconcilerName::new("service-lifecycle").unwrap()
            )
            .unwrap(),
        before
    );
    world.run("workload-lifecycle").await;
    let restarted = world.obs.alloc_status_rows().await.unwrap().remove(0);
    assert_eq!((restarted.state, restarted.alloc_id), (AllocState::Running, ended.alloc_id));
    let publication_start = world.history.len();
    for _ in 0..3 {
        world.run("service-lifecycle").await;
    }
    world.assert_shape(&world.rows().await, 1, false);
    assert!(
        world.history[publication_start..]
            .iter()
            .filter_map(|row| match row {
                ObservationRow::ServiceBackend(row) => Some(row),
                _ => None,
            })
            .all(|row| row.backends.iter().all(|backend| !backend.healthy)),
        "no newly eligible publication while retained terminal veto applies"
    );
}

impl Drop for World {
    fn drop(&mut self) {
        // ProbeRunner's stop hook cancels all scenario-owned probe tasks. There
        // is no spawned production binary and no externally owned resource.
        for view in self
            .state
            .runtime
            .loaded_service_lifecycle_views_for_test(
                &ReconcilerName::new("service-lifecycle").unwrap(),
            )
            .into_iter()
            .flat_map(std::collections::BTreeMap::into_values)
        {
            for alloc in view.observed {
                self.driver.probes.stop_alloc(&alloc);
            }
        }
    }
}

/// CONTRACT_SHAPE: bounded-change.
/// Given 1/N current TCP/UDP listeners and one real Running allocation, when
/// ServiceLifecycle runs repeatedly, every listener has exactly its complete
/// row; no-readiness is eligible and an equal row is not rewritten.
/// Multi-allocation membership/order coverage is deferred to GitHub issue #282.
#[tokio::test(flavor = "current_thread")]
async fn complete_listener_projection_is_idempotent() {
    for (seed, listeners, replicas) in [
        (257_210, vec![(18081, "tcp")], 1),
        (257_211, vec![(18082, "udp"), (18081, "tcp"), (18081, "udp")], 1),
    ] {
        let mut world = World::new(seed, input(&listeners, replicas), true).await;
        world.run("workload-lifecycle").await;
        world.run("service-lifecycle").await;
        let baseline = world.rows().await;
        world.assert_shape(&baseline, replicas as usize, true);
        let mut allocs = world.obs.alloc_status_rows().await.unwrap();
        allocs.sort_by(|left, right| left.alloc_id.cmp(&right.alloc_id));
        let identities: Vec<_> = allocs
            .iter()
            .map(|alloc| {
                overdrive_core::SpiffeId::for_allocation(&alloc.workload_id, &alloc.alloc_id)
            })
            .collect();
        for row in &baseline {
            assert_eq!(
                row.backends.iter().map(|backend| backend.alloc.clone()).collect::<Vec<_>>(),
                identities,
                "seed={seed}: exact allocation identities in allocation-ID order"
            );
        }
        for _ in 0..3 {
            world.run("service-lifecycle").await;
        }
        assert_eq!(
            world.rows().await,
            baseline,
            "seed={seed}: equality excludes advancing tick; no write or stamp churn"
        );
    }
}

/// CONTRACT_SHAPE: bounded-change.
/// A current listener has a real desired empty row before any allocation exists;
/// the no-allocator case produces no row, not historical-row GC. Empty listener
/// declarations are admission-invalid and are not fabricated inside this fixture.
#[tokio::test(flavor = "current_thread")]
async fn empty_membership_and_absent_dataplane_are_distinct() {
    for (seed, listeners, allocate) in
        [(257_213, vec![(18081, "tcp")], false), (257_214, vec![(18081, "tcp")], true)]
    {
        let mut world = World::new(seed, input(&listeners, 1), allocate).await;
        world.run("service-lifecycle").await;
        let rows = world.rows().await;
        world.assert_shape(&rows, 0, false);
        world.run("service-lifecycle").await;
        assert_eq!(world.rows().await, rows, "seed={seed}: equal empty projection is stable");
    }
}

/// CONTRACT_SHAPE: bounded-change.
/// Given a healthy no-readiness baseline, when the first changed-row write
/// fails, the next observed-state reconciliation repairs it without inventing
/// retry scheduling or depending on a readiness transition.
#[tokio::test(flavor = "current_thread")]
async fn rejected_first_publication_is_repaired_from_observed_state() {
    let mut spec = input(&[(18081, "tcp")], 1);
    spec.startup_probes = vec![startup()];
    let mut world = World::new(257_215, spec, true).await;
    world.run("workload-lifecycle").await;
    world.obs.inject_write_failure(ObservationStoreError::Unreachable {
        peer: "seeded-write-fault".into(),
    });
    world.tick += 1;
    let outcome = run_convergence_tick(
        &world.state,
        &ReconcilerName::new("service-lifecycle").unwrap(),
        &world.target,
        world.clock.now(),
        world.tick,
        world.clock.now() + Duration::from_secs(30),
    )
    .await;
    assert!(
        matches!(outcome, Err(ConvergenceError::Shim(ShimError::Observation(ObservationStoreError::Unreachable { ref peer }))) if peer == "seeded-write-fault"),
        "exact injected write error: {outcome:?}"
    );
    assert!(world.rows().await.is_empty(), "fault must reject the publication, not setup");
    for _ in 0..3 {
        world.run("service-lifecycle").await;
    }
    world.assert_shape(&world.rows().await, 1, true);
}

/// CONTRACT_SHAPE: bounded-change.
/// Startup exhaustion is caused by ProbeRunner. Failure publication is not a
/// consumer acknowledgement; empty membership and same-ID restart must both
/// preserve the policy owner's retained veto through bounded reconciliation.
#[tokio::test(flavor = "current_thread")]
async fn startup_failure_withdraws_then_same_id_restart_remains_ineligible() {
    for seed in [257_209, 257_216] {
        let mut spec = input(&[(18081, "tcp")], 1);
        spec.startup_probes = vec![startup()];
        let mut world = World::new(seed, spec, true).await;
        for _ in 0..32 {
            world
                .tcp
                .enqueue_outcome(ProbeOutcome::Fail { reason: "seeded connection refused".into() });
        }
        world.run("workload-lifecycle").await;
        world.run("service-lifecycle").await;
        world.assert_shape(&world.rows().await, 1, true);
        let original = world.obs.alloc_status_rows().await.unwrap().remove(0);
        for _ in 0..4 {
            world.advance_probes().await;
            world.run("service-lifecycle").await;
        }
        let ended = world.obs.alloc_status_rows().await.unwrap().remove(0);
        assert_eq!(
            ended.state,
            AllocState::Failed,
            "seed={seed}: real startup failure precondition"
        );
        assert_eq!(ended.alloc_id, original.alloc_id);
        world.run("service-lifecycle").await;
        world.assert_shape(&world.rows().await, 0, false);
        world.run("workload-lifecycle").await;
        let restarted = world.obs.alloc_status_rows().await.unwrap().remove(0);
        assert_eq!(
            (restarted.state, restarted.alloc_id),
            (AllocState::Running, original.alloc_id.clone())
        );
        for _ in 0..6 {
            world.run("service-lifecycle").await;
            world.assert_shape(&world.rows().await, 1, false);
        }
        let views = world
            .state
            .runtime
            .loaded_service_lifecycle_views_for_test(
                &ReconcilerName::new("service-lifecycle").unwrap(),
            )
            .unwrap();
        assert!(views[&world.target].terminal_announced.contains(&original.alloc_id));
    }
}

/// CONTRACT_SHAPE: bounded-change.
/// Readiness Pass increments once per allocation/tick, not once per listener;
/// a real failing probe withdraws and later Pass observations recover without
/// marking startup terminal or altering the normal allocation identity.
#[tokio::test(flavor = "current_thread")]
async fn readiness_threshold_is_listener_independent_and_recovers() {
    for listener_count in [1, 3] {
        let listeners = [(18081, "tcp"), (18082, "tcp"), (18081, "udp")];
        let mut spec = input(&listeners[..listener_count], 1);
        spec.readiness_probes = vec![readiness(3)];
        let mut world = World::new(257_217 + listener_count as u64, spec, true).await;
        world.run("workload-lifecycle").await;
        world.advance_probes().await;
        for pass in 1..=3 {
            world.run("service-lifecycle").await;
            world.assert_shape(&world.rows().await, 1, pass == 3);
        }
        world.http.enqueue_outcome(ProbeOutcome::Fail { reason: "seeded readiness outage".into() });
        world.advance_probes().await;
        world.run("service-lifecycle").await;
        world.assert_shape(&world.rows().await, 1, false);
        world.advance_probes().await;
        for pass in 1..=3 {
            world.run("service-lifecycle").await;
            world.assert_shape(&world.rows().await, 1, pass == 3);
        }
        assert_eq!(world.obs.alloc_status_rows().await.unwrap()[0].state, AllocState::Running);
        let views = world
            .state
            .runtime
            .loaded_service_lifecycle_views_for_test(
                &ReconcilerName::new("service-lifecycle").unwrap(),
            )
            .unwrap();
        assert!(views[&world.target].terminal_announced.is_empty());
    }
}

/// CONTRACT_SHAPE: bounded-change.
/// Healthy control -> prober fault -> authoritative withdrawal -> independent
/// List/Watch and queued hydrator convergence -> prober recovery. DNS and new
/// mesh resolves are observed; established connections and VM map paths are not.
#[tokio::test(flavor = "current_thread")]
async fn existing_consumers_follow_withdrawal_and_recovery_asynchronously() {
    Box::pin(consumer_trajectory(true)).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// Existing List/Watch control proves that the actual production resolver and
/// DNS projection follow healthy -> failing -> recovering ProbeRunner outcomes,
/// independently of the new direct hydrator handoff being implemented.
#[tokio::test(flavor = "current_thread")]
async fn mesh_and_dns_watch_control_follows_real_readiness_outcomes() {
    Box::pin(consumer_trajectory(false)).await;
}

async fn consumer_trajectory(include_hydrator: bool) {
    let mut spec = input(&[(18081, "tcp")], 1);
    spec.readiness_probes = vec![readiness(1)];
    let mut world = World::new(257_221, spec, true).await;
    world.run("workload-lifecycle").await;
    world.advance_probes().await;
    world.run("service-lifecycle").await;
    world.assert_shape(&world.rows().await, 1, true);
    let resolve =
        ServiceBackendsResolve::new(world.obs.clone(), world.state.frontend_addr_allocator.clone());
    let dns = NameIndex::new(world.obs.clone(), world.state.frontend_addr_allocator.clone());
    resolve.probe().await.unwrap();
    dns.probe().await.unwrap();
    let destination = SocketAddrV4::new(world.frontend, 18081);
    assert!(matches!(resolve.resolve(destination).await.unwrap(), MtlsResolution::Mesh(_)));
    assert_eq!(dns.frontend_for(&world.name), Some(world.frontend));
    let vip = world.vip.as_ref().unwrap().try_as_ipv4().unwrap();
    if include_hydrator {
        world.hydrate_published().await;
        assert_eq!(
            world.dataplane.local_backend_for(vip, 18081, Proto::Tcp),
            Some(SocketAddrV4::new(HOST, 18081))
        );
    }
    world.http.enqueue_outcome(ProbeOutcome::Fail { reason: "seeded readiness outage".into() });
    world.advance_probes().await;
    world.run("service-lifecycle").await;
    world.assert_shape(&world.rows().await, 1, false);
    // Do not require synchronous observer state at publication's return.
    world.settle().await;
    assert_eq!(resolve.resolve(destination).await.unwrap(), MtlsResolution::MeshUnreachable);
    assert_eq!(dns.frontend_for(&world.name), None);
    if include_hydrator {
        world.hydrate_published().await;
        assert_eq!(world.dataplane.local_backend_for(vip, 18081, Proto::Tcp), None);
    }
    world.advance_probes().await;
    world.run("service-lifecycle").await;
    world.settle().await;
    assert!(matches!(resolve.resolve(destination).await.unwrap(), MtlsResolution::Mesh(_)));
    assert_eq!(dns.frontend_for(&world.name), Some(world.frontend));
    if include_hydrator {
        world.hydrate_published().await;
        assert_eq!(
            world.dataplane.local_backend_for(vip, 18081, Proto::Tcp),
            Some(SocketAddrV4::new(HOST, 18081))
        );
    }
    drop(dns);
    drop(resolve);
    world.settle().await;
}
