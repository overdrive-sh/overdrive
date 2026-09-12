//! Seeded DISTILL scaffold for the accepted terminal-authority outcome.
//!
//! DELIVER activates this through the existing `overdrive-sim` lifecycle and
//! backend-observation boundaries. The fixed seed must reproduce a schedule in
//! which a VM Service becomes terminal while health work is in flight, then
//! prove only the externally meaningful invariant: terminal wins and the dead
//! backend never returns to eligibility. This scenario does not authorize a
//! ProbeRunner drain/join contract, tombstone, write suppression, or new seam.

// Contract-shape declarations intentionally use the repository-mandated token.
#![allow(
    clippy::doc_markdown,
    clippy::missing_panics_doc,
    clippy::print_stderr,
    clippy::too_many_lines
)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::{FutureExt, StreamExt};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{
    AppState, InterestRouterBroker, build_interest_table, service_lifecycle, spawn_interest_router,
    workload_lifecycle,
};
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, ServiceV2, VmInput, WorkloadIntent,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{NodeId, ServiceId, ServiceVip};
use overdrive_core::observation::{ProbeIdx, ProbeRole, ProbeStatus};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, Resources,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, LagAwareSubscription, ObservationRow, ObservationStore, ObservationWrite,
    ServiceBackendRow, SubscriptionEvent,
};
use overdrive_core::traits::prober::ProbeOutcome;
use overdrive_core::transition_reason::{ServiceFailureReason, TerminalCondition};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{AllocationId, SpiffeId};
use overdrive_reconcilers::service_lifecycle::{
    ServiceAllocFact, ServiceDataplaneIdentity, ServiceLifecycleReconciler, ServiceLifecycleState,
    ServiceLifecycleView,
};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimExecProber, SimHttpProber, SimTcpProber};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::probe_runner::ProbeRunner;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

const SEED: u64 = 25_717;

fn alloc_id() -> AllocationId {
    AllocationId::new("alloc-service-vm-terminal-25717").expect("static allocation ID is valid")
}

fn tick(number: u64, seconds: u64) -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(seconds)),
        tick: number,
        deadline: Instant::now() + Duration::from_secs(1),
    }
}

fn service_dataplane() -> ServiceDataplaneIdentity {
    ServiceDataplaneIdentity {
        port: std::num::NonZeroU16::new(18_081).expect("listener port"),
        protocol: Proto::Tcp,
        vip: ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 96, 0, 42))).expect("valid vip"),
        writer: NodeId::new("svm-025717").expect("valid node id"),
    }
}

fn fact(state: AllocState, startup: ProbeStatus, readiness: ProbeStatus) -> ServiceAllocFact {
    ServiceAllocFact {
        alloc_id: alloc_id(),
        state,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
        exit_code: Some(1).filter(|_| state == AllocState::Failed),
        latest_startup_probe: Some(startup),
        latest_startup_probe_observed_at: Some(UnixInstant::from_unix_duration(
            Duration::from_secs(2),
        )),
        max_attempts: 1,
        startup_deadline: Duration::from_secs(1),
        mechanic_summary: "tcp 0.0.0.0:18081".to_string(),
        inferred: false,
        startup_probes_empty: false,
        latest_readiness_probe: Some(readiness),
        has_readiness_probe: true,
        readiness_success_threshold: 1,
        backend_spiffe: SpiffeId::new(
            "spiffe://overdrive.local/workload/service-vm-terminal-25717/alloc/0",
        )
        .expect("static backend SPIFFE ID is valid"),
        backend_ip: Ipv4Addr::new(192, 0, 2, 17),
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
    }
}

fn state_for(fact: ServiceAllocFact) -> ServiceLifecycleState {
    ServiceLifecycleState {
        allocs: BTreeMap::from([(fact.alloc_id.clone(), fact)]),
        service_dataplane: BTreeMap::from([(
            ServiceId::new(42).expect("service id"),
            service_dataplane(),
        )]),
        ..Default::default()
    }
}

fn backend_row(actions: &[Action]) -> &ServiceBackendRow {
    actions
        .iter()
        .find_map(|action| match action {
            Action::WriteServiceBackendRow { row, .. } => Some(row),
            _ => None,
        })
        .expect("terminal invariant tick writes a backend observation")
}

async fn apply_backend_writes(store: &SimObservationStore, actions: &[Action]) {
    for action in actions {
        if let Action::WriteServiceBackendRow { row, .. } = action {
            store
                .write(ObservationWrite::ServiceBackend(row.clone()))
                .await
                .expect("sim backend observation write");
        }
    }
}

async fn hydrate_backend(state: &mut ServiceLifecycleState, store: &SimObservationStore) {
    let service_id = ServiceId::new(42).expect("service id");
    let row = store
        .service_backends_rows(&service_id)
        .await
        .expect("sim backend observation read")
        .into_iter()
        .next()
        .expect("service backend row exists");
    state.observed_backend_rows.insert(service_id, row);
}

/// S-SVM-17 — fixed seed `25717` interleaves terminal state with outstanding
/// health work; the terminal allocation remains terminal and its backend never
/// becomes eligible again.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread")]
async fn terminal_state_wins_and_dead_vm_backend_never_returns_to_eligibility() {
    eprintln!("service-kind-vm terminal invariant seed={SEED}");
    let node = NodeId::new("svm-025717").expect("static node ID is valid");
    let store = SimObservationStore::single_peer(node, SEED);
    let reconciler = ServiceLifecycleReconciler::new();
    let alloc_id = alloc_id();

    // The first tick observes a live allocation and a readiness Pass while a
    // startup failure is still inside its deadline. Health work may make the
    // backend eligible, but it cannot publish a terminal yet.
    let mut in_flight = state_for(fact(
        AllocState::Running,
        ProbeStatus::Fail { last_fail_reason: "guest TCP port 18081 refused".to_string() },
        ProbeStatus::Pass,
    ));
    let (in_flight_actions, in_flight_view) =
        reconciler.reconcile(&in_flight, &in_flight, &ServiceLifecycleView::default(), &tick(1, 1));
    assert!(backend_row(&in_flight_actions).backends[0].healthy);
    assert!(
        !in_flight_actions.iter().any(|action| matches!(action, Action::FinalizeFailed { .. }))
    );
    apply_backend_writes(&store, &in_flight_actions).await;
    hydrate_backend(&mut in_flight, &store).await;

    // The owner now observes the same allocation as terminal after the
    // startup deadline. The deciding tick withdraws its backend before the
    // existing terminal publication reaches the observation boundary.
    in_flight.allocs.get_mut(&alloc_id).expect("allocation fact").state = AllocState::Failed;
    let (terminal_actions, terminal_view) =
        reconciler.reconcile(&in_flight, &in_flight, &in_flight_view, &tick(2, 3));
    assert!(matches!(
        terminal_actions.iter().find(|action| matches!(action, Action::FinalizeFailed { .. })),
        Some(Action::FinalizeFailed {
            alloc_id: action_alloc_id,
            terminal: Some(TerminalCondition::ServiceFailed {
                reason: ServiceFailureReason::StartupProbeFailed { attempts: 1, .. },
            }),
        }) if action_alloc_id == &alloc_id
    ));
    assert!(
        !backend_row(&terminal_actions).backends.iter().any(|backend| backend.healthy),
        "terminal publication must not leave an eligible backend"
    );
    assert!(terminal_view.terminal_announced.contains(&alloc_id));
    assert!(
        !terminal_actions.iter().any(|action| matches!(action, Action::RestartAllocation { .. }))
    );
    apply_backend_writes(&store, &terminal_actions).await;
    hydrate_backend(&mut in_flight, &store).await;

    // A same-ID replacement may be observed Running and healthy, but the
    // retained terminal veto keeps it out of eligibility. A subsequent late
    // Pass is a no-op against the persisted unhealthy projection.
    in_flight.allocs.get_mut(&alloc_id).expect("allocation fact").state = AllocState::Running;
    in_flight.allocs.get_mut(&alloc_id).expect("allocation fact").latest_startup_probe =
        Some(ProbeStatus::Pass);
    let (replacement_actions, replacement_view) =
        reconciler.reconcile(&in_flight, &in_flight, &terminal_view, &tick(3, 4));
    let replacement_row = backend_row(&replacement_actions);
    assert!(!replacement_row.backends[0].healthy);
    assert!(replacement_view.terminal_announced.contains(&alloc_id));
    assert!(
        !replacement_actions
            .iter()
            .any(|action| matches!(action, Action::RestartAllocation { .. }))
    );
    apply_backend_writes(&store, &replacement_actions).await;
    hydrate_backend(&mut in_flight, &store).await;

    let (late_actions, late_view) =
        reconciler.reconcile(&in_flight, &in_flight, &replacement_view, &tick(4, 5));
    assert!(late_actions.is_empty(), "a late readiness Pass cannot revive terminal eligibility");
    assert!(late_view.terminal_announced.contains(&alloc_id));
    let persisted = store
        .service_backends_rows(&ServiceId::new(42).expect("service id"))
        .await
        .expect("sim backend observation read");
    assert_eq!(persisted.len(), 1);
    assert!(!persisted[0].backends[0].healthy);
}

/// Driver wrapper used only to keep the production allocation hook and the
/// production `ProbeRunner` in the same composition. The simulated driver is
/// VM-typed so the scenario retains the same lifecycle-family discriminator as
/// the native E11 journey; no VM-specific policy is reimplemented here.
struct ProbedVmDriver {
    inner: overdrive_sim::adapters::driver::SimDriver,
    probes: Arc<ProbeRunner>,
}

#[async_trait]
impl Driver for ProbedVmDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Vm
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

fn vm_probe_descriptors() -> (ProbeDescriptor, ProbeDescriptor) {
    let startup = ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_owned(), port: 18_081 },
        timeout_seconds: 1,
        // Keep the supervised task parked while this invariant drives the
        // single-tick production `ProbeRunner` calls explicitly.
        interval_seconds: 60,
        max_attempts: 1,
        failure_threshold: None,
        success_threshold: None,
        inferred: false,
    };
    let readiness = ProbeDescriptor {
        role: ProbeRole::Readiness,
        success_threshold: Some(1),
        ..startup.clone()
    };
    (startup, readiness)
}

async fn wait_for_probe_event(
    events: &mut LagAwareSubscription,
    expected: ProbeStatus,
    seed: u64,
) -> overdrive_core::observation::ProbeResultRow {
    let row = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match events.next().await {
                Some(SubscriptionEvent::Row(ObservationRow::ProbeResult(row))) => break row,
                Some(SubscriptionEvent::Lagged { missed }) => {
                    panic!("seed={seed}: audit subscription lagged by {missed} rows")
                }
                Some(_) => {}
                None => panic!("seed={seed}: audit subscription closed unexpectedly"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("seed={seed}: accepted probe event did not arrive"));
    assert_eq!(row.status, expected, "seed={seed}: event must carry the durable probe outcome");
    row
}

fn assert_no_probe_event(events: &mut LagAwareSubscription, seed: u64) {
    // Drain rows that were already queued for this audit subscriber, while
    // allowing ordinary alloc/backend rows to pass. A losing probe write must
    // not add a `ProbeResult` projection to that stream.
    for _ in 0..32 {
        let Some(Some(event)) = events.next().now_or_never() else { return };
        match event {
            SubscriptionEvent::Row(ObservationRow::ProbeResult(row)) => {
                panic!("seed={seed}: losing probe row emitted an event: {row:?}")
            }
            SubscriptionEvent::Lagged { missed } => {
                panic!("seed={seed}: audit subscription lagged by {missed} rows")
            }
            SubscriptionEvent::Row(_) => {}
        }
    }
}

async fn run_owner_tick(
    state: &AppState,
    clock: &SimClock,
    owner: &str,
    target: &overdrive_core::reconcilers::TargetResource,
    tick: u64,
) {
    let reconciler = overdrive_core::reconcilers::ReconcilerName::new(owner)
        .expect("static reconciler name is valid");
    run_convergence_tick(
        state,
        &reconciler,
        target,
        clock.now(),
        tick,
        clock.now() + Duration::from_secs(30),
    )
    .await
    .unwrap_or_else(|error| panic!("seed={SEED}: {owner} convergence failed: {error:?}"));
}

async fn converge_service_wakes(
    state: &AppState,
    clock: &SimClock,
    target: &overdrive_core::reconcilers::TargetResource,
    next_tick: &mut u64,
) {
    let mut ran = false;
    let mut quiet_rounds = 0;
    for _ in 0..128 {
        let pending = state.runtime.broker().drain_pending(
            usize::MAX,
            &std::collections::BTreeSet::new(),
            clock.now(),
            overdrive_core::UnixInstant::from_unix_duration(std::time::Duration::ZERO),
        );
        let mut ran_this_round = false;
        for (evaluation, _) in pending {
            if evaluation.reconciler.as_str() != "service-lifecycle" || evaluation.target != *target
            {
                continue;
            }
            *next_tick = next_tick.saturating_add(1);
            run_owner_tick(state, clock, "service-lifecycle", target, *next_tick).await;
            ran = true;
            ran_this_round = true;
        }
        if ran_this_round {
            quiet_rounds = 0;
        } else {
            quiet_rounds += 1;
            if ran && quiet_rounds >= 8 {
                return;
            }
        }
        tokio::task::yield_now().await;
    }
    assert!(ran, "seed={SEED}: the production interest router never woke ServiceLifecycle");
    panic!("seed={SEED}: ServiceLifecycle broker did not reach a quiet point");
}

/// E11 amendment — seed `25717` drives the production ProbeRunner → Sim store
/// → interest-router → broker → convergence path. The readiness transition
/// changes only the existing backend-health projection and does not restart
/// the VM allocation. The final operator stop supplies a real terminal row so
/// a late accepted readiness Pass is checked against terminal eligibility.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread")]
async fn seeded_probe_result_wake_converges_vm_readiness_without_restart() {
    let seed = SEED;
    eprintln!("service-kind-vm readiness-wake seed={seed}");
    let temp = TempDir::new().expect("temporary production composition directory");
    let kernel = temp.path().join("kernel");
    let rootfs = temp.path().join("rootfs");
    let mut header = vec![0; overdrive_core::vm::config::KERNEL_MAGIC_WINDOW];
    header[..4].copy_from_slice(b"\x7fELF");
    std::fs::write(&kernel, header).expect("sim kernel fixture");
    std::fs::write(&rootfs, b"sim-rootfs").expect("sim rootfs fixture");

    let node = NodeId::new("svm-025717").expect("static node ID is valid");
    let clock = Arc::new(SimClock::new());
    let sim_obs = Arc::new(SimObservationStore::single_peer(node.clone(), seed));
    let obs: Arc<dyn ObservationStore> = sim_obs.clone();
    let tcp = Arc::new(SimTcpProber::new());
    let probes = Arc::new(ProbeRunner::new(
        tcp.clone(),
        Arc::new(SimHttpProber::new()),
        Arc::new(SimExecProber::new()),
        clock.clone(),
        obs.clone(),
    ));
    let driver: Arc<dyn Driver> = Arc::new(ProbedVmDriver {
        inner: SimDriver::with_clock(DriverType::Vm, clock.clone()),
        probes: probes.clone(),
    });

    let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(temp.path())
        .expect("production reconciler runtime");
    runtime.register(workload_lifecycle()).await.expect("register workload lifecycle");
    runtime.register(service_lifecycle()).await.expect("register service lifecycle");
    let runtime = Arc::new(runtime);
    let store =
        Arc::new(LocalIntentStore::open(temp.path().join("intent.redb")).expect("intent store"));
    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    let state = AppState::new(
        Arc::clone(&store),
        temp.path().join("intent.redb"),
        obs.clone(),
        runtime.clone(),
        driver.clone(),
        clock.clone(),
        Arc::new(SimDataplane::new()),
        Arc::new(SimCa::new(Arc::new(SimEntropy::new(seed)))),
        Arc::new(IdentityMgr::new(None)),
        node.clone(),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        Ipv4Addr::LOCALHOST,
    );

    let (startup, readiness) = vm_probe_descriptors();
    let workload_id =
        overdrive_core::WorkloadId::new("service-vm-readiness-25717").expect("workload ID");
    let spec = ServiceV2::from_submit(ServiceSpecInput {
        id: workload_id.to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Vm(VmInput {
            command: "/long-lived-http-server".to_owned(),
            args: vec![],
            kernel: kernel.display().to_string(),
            rootfs: rootfs.display().to_string(),
        }),
        listeners: vec![ListenerInput { port: 18_081, protocol: "tcp".to_owned() }],
        startup_probes: vec![startup.clone()],
        readiness_probes: vec![readiness.clone()],
        liveness_probes: vec![],
    })
    .expect("validated VM Service spec");
    let intent = WorkloadIntent::Service(spec.clone());
    let digest = intent.spec_digest().expect("Service spec digest");
    let vip = state
        .allocator
        .lock()
        .await
        .allocate(*digest.as_bytes())
        .await
        .expect("allocator-issued Service VIP");
    let intent_key = IntentKey::for_workload(&workload_id);
    let archived = intent.archive_for_store().expect("archive intent");
    state.store.put(intent_key.as_bytes(), archived.as_ref()).await.expect("persist intent");
    *state.listener_facts.lock().await =
        overdrive_control_plane::listener_facts::ListenerFactStore::rebuild_from_intent(
            &state.store,
            &state.intent_redb_path,
            &state.allocator,
        )
        .await
        .expect("rebuild listener facts");
    let target =
        overdrive_core::reconcilers::TargetResource::new(&format!("workload/{workload_id}"))
            .expect("workload target");
    let service_id = ServiceId::derive(
        &vip,
        std::num::NonZeroU16::new(18_081).expect("listener port"),
        Proto::Tcp,
        "service-map",
    );

    // Subscribe both the router and the independent event oracle before any
    // allocation or probe write. The router uses the exact production table
    // inversion and the same restricted broker consumed by convergence.
    let router_subscription = obs.subscribe_all_events().await.expect("router subscription");
    let mut audit = obs.subscribe_all_events().await.expect("audit subscription");
    let shutdown = CancellationToken::new();
    let table = build_interest_table(runtime.reconcilers_iter());
    let router = spawn_interest_router(
        obs.clone(),
        router_subscription,
        table,
        InterestRouterBroker::from_runtime(runtime.clone(), clock.clone()),
        clock.clone(),
        Duration::from_secs(30),
        shutdown.clone(),
    );

    // The allocation and Stable state are produced by the registered
    // WorkloadLifecycle/ServiceLifecycle composition, not seeded as rows.
    run_owner_tick(&state, &clock, "workload-lifecycle", &target, 1).await;
    let alloc_rows = obs.alloc_status_rows().await.expect("allocation snapshot");
    assert_eq!(alloc_rows.len(), 1, "seed={seed}: one VM allocation is placed");
    let alloc_id = alloc_rows[0].alloc_id.clone();
    assert_eq!(alloc_rows[0].state, AllocState::Running);
    assert_eq!(alloc_rows[0].restart_count, 0);
    assert_eq!(alloc_rows[0].kind, overdrive_core::aggregate::WorkloadKind::Service);

    let mut next_tick = 1;
    tcp.enqueue_outcome(ProbeOutcome::Pass);
    probes
        .probe_once_and_record(&alloc_id, ProbeIdx::new(0), &startup, clock.as_ref(), obs.as_ref())
        .await
        .expect("startup Pass write");
    let startup_event = wait_for_probe_event(&mut audit, ProbeStatus::Pass, seed).await;
    assert_eq!(startup_event.alloc_id, alloc_id);
    assert_eq!(startup_event.role, ProbeRole::Startup);

    tcp.enqueue_outcome(ProbeOutcome::Pass);
    probes
        .probe_once_and_record(
            &alloc_id,
            ProbeIdx::new(0),
            &readiness,
            clock.as_ref(),
            obs.as_ref(),
        )
        .await
        .expect("baseline readiness Pass write");
    let baseline_event = wait_for_probe_event(&mut audit, ProbeStatus::Pass, seed).await;
    assert_eq!(baseline_event.alloc_id, alloc_id);
    converge_service_wakes(&state, &clock, &target, &mut next_tick).await;
    let baseline_row = obs
        .service_backends_rows(&service_id)
        .await
        .expect("baseline backend read")
        .into_iter()
        .next()
        .expect("baseline backend row");
    assert_eq!(baseline_row.backends.len(), 1);
    assert!(baseline_row.backends[0].healthy, "seed={seed}: baseline is healthy");
    let baseline_alloc = obs.alloc_status_row(&alloc_id).await.expect("alloc read").expect("alloc");
    assert_eq!(baseline_alloc.state, AllocState::Running);
    assert!(matches!(baseline_alloc.terminal.as_ref(), Some(TerminalCondition::Stable { .. })));
    assert_eq!(baseline_alloc.restart_count, 0);
    let baseline_occurrences =
        obs.alloc_lifecycle_occurrences(&alloc_id).await.expect("baseline terminal history");
    assert!(baseline_occurrences.iter().any(|row| row.terminal.is_some()));

    // A readiness Fail is accepted by the LWW store, produces exactly one
    // event after commit, and wakes the existing ServiceLifecycle owner.
    clock.tick(Duration::from_secs(1));
    tcp.enqueue_outcome(ProbeOutcome::Fail { reason: "guest readiness refused".to_owned() });
    probes
        .probe_once_and_record(
            &alloc_id,
            ProbeIdx::new(0),
            &readiness,
            clock.as_ref(),
            obs.as_ref(),
        )
        .await
        .expect("readiness Fail write");
    let fail_event = wait_for_probe_event(
        &mut audit,
        ProbeStatus::Fail { last_fail_reason: "guest readiness refused".to_owned() },
        seed,
    )
    .await;
    assert_eq!(fail_event.role, ProbeRole::Readiness);
    let durable_fail = obs
        .list_probe_results_for_alloc(&alloc_id)
        .await
        .expect("durable readiness snapshot")
        .into_iter()
        .find(|row| row.role == ProbeRole::Readiness)
        .expect("durable readiness Fail row");
    assert_eq!(durable_fail, fail_event, "event is the committed durable winner");
    let occurrences_before_fail =
        obs.alloc_lifecycle_occurrences(&alloc_id).await.expect("history");
    converge_service_wakes(&state, &clock, &target, &mut next_tick).await;
    let failed_backend = obs
        .service_backends_rows(&service_id)
        .await
        .expect("failed backend read")
        .into_iter()
        .next()
        .expect("failed backend row");
    assert!(!failed_backend.backends[0].healthy, "seed={seed}: readiness Fail withdraws traffic");
    let failed_alloc = obs.alloc_status_row(&alloc_id).await.expect("alloc read").expect("alloc");
    assert_eq!(failed_alloc.state, AllocState::Running);
    assert_eq!(failed_alloc.alloc_id, alloc_id);
    assert_eq!(failed_alloc.restart_count, 0);
    assert_eq!(failed_alloc.terminal, baseline_alloc.terminal);
    assert_eq!(
        obs.alloc_lifecycle_occurrences(&alloc_id).await.expect("history"),
        occurrences_before_fail,
        "readiness does not append lifecycle terminal history",
    );
    assert_eq!(driver.as_ref().r#type(), DriverType::Vm);
    assert_eq!(
        probes.active_alloc_count(),
        1,
        "seed={seed}: readiness remains supervised after Stable",
    );

    // Equal and older LWW probe rows are silent: no event, no durable change.
    obs.write_probe_result(durable_fail.clone()).await.expect("equal probe write");
    let mut older = durable_fail.clone();
    older.last_observed_at_unix_ms = 0;
    obs.write_probe_result(older).await.expect("stale probe write");
    assert_no_probe_event(&mut audit, seed);

    // An accepted readiness Pass restores the complete backend row and keeps
    // the same Running/Stable allocation with no restart.
    clock.tick(Duration::from_secs(1));
    tcp.enqueue_outcome(ProbeOutcome::Pass);
    probes
        .probe_once_and_record(
            &alloc_id,
            ProbeIdx::new(0),
            &readiness,
            clock.as_ref(),
            obs.as_ref(),
        )
        .await
        .expect("readiness recovery Pass write");
    let pass_event = wait_for_probe_event(&mut audit, ProbeStatus::Pass, seed).await;
    assert_eq!(pass_event.role, ProbeRole::Readiness);
    converge_service_wakes(&state, &clock, &target, &mut next_tick).await;
    let recovered_backend = obs
        .service_backends_rows(&service_id)
        .await
        .expect("recovered backend read")
        .into_iter()
        .next()
        .expect("recovered backend row");
    assert!(recovered_backend.backends[0].healthy, "seed={seed}: readiness Pass restores traffic");
    let recovered_alloc =
        obs.alloc_status_row(&alloc_id).await.expect("alloc read").expect("alloc");
    assert_eq!(recovered_alloc.state, AllocState::Running);
    assert_eq!(recovered_alloc.alloc_id, alloc_id);
    assert_eq!(recovered_alloc.restart_count, 0);
    assert_eq!(recovered_alloc.terminal, baseline_alloc.terminal);
    assert_eq!(
        obs.alloc_lifecycle_occurrences(&alloc_id).await.expect("history"),
        occurrences_before_fail,
    );
    assert_eq!(probes.active_alloc_count(), 1);

    // End the allocation through the existing WorkloadLifecycle stop owner,
    // then send a late accepted Pass. The terminal row and empty backend are
    // produced by the existing owners; the late event cannot revive it.
    let stop_key = IntentKey::for_workload_stop(&workload_id);
    state.store.put(stop_key.as_bytes(), &[]).await.expect("persist stop intent");
    run_owner_tick(&state, &clock, "workload-lifecycle", &target, next_tick + 1).await;
    converge_service_wakes(&state, &clock, &target, &mut next_tick).await;
    let terminal_alloc =
        obs.alloc_status_row(&alloc_id).await.expect("terminal alloc read").expect("alloc");
    assert_eq!(terminal_alloc.alloc_id, alloc_id);
    assert_eq!(terminal_alloc.state, AllocState::Terminated);
    assert_eq!(terminal_alloc.restart_count, 0);
    assert!(
        obs.service_backends_rows(&service_id)
            .await
            .expect("terminal backend read")
            .into_iter()
            .next()
            .expect("terminal backend row")
            .backends
            .is_empty()
    );

    clock.tick(Duration::from_secs(1));
    tcp.enqueue_outcome(ProbeOutcome::Pass);
    probes
        .probe_once_and_record(
            &alloc_id,
            ProbeIdx::new(0),
            &readiness,
            clock.as_ref(),
            obs.as_ref(),
        )
        .await
        .expect("late readiness Pass write");
    let late_event = wait_for_probe_event(&mut audit, ProbeStatus::Pass, seed).await;
    assert_eq!(late_event.role, ProbeRole::Readiness);
    converge_service_wakes(&state, &clock, &target, &mut next_tick).await;
    let late_backend = obs
        .service_backends_rows(&service_id)
        .await
        .expect("late backend read")
        .into_iter()
        .next()
        .expect("late backend row");
    assert!(late_backend.backends.is_empty(), "seed={seed}: terminal late Pass stays ineligible");
    let terminal_history =
        obs.alloc_lifecycle_occurrences(&alloc_id).await.expect("terminal history");
    assert!(terminal_history.iter().any(|row| row.to == AllocState::Terminated));
    assert!(terminal_history.iter().all(|row| row.to != AllocState::Failed));
    assert_eq!(
        obs.alloc_status_rows().await.expect("final alloc snapshot").len(),
        1,
        "seed={seed}: no replacement allocation was created",
    );
    assert_eq!(
        probes.active_alloc_count(),
        0,
        "seed={seed}: terminal owner cancelled the full probe supervisor",
    );

    shutdown.cancel();
    router.await.expect("interest router shutdown");
}
