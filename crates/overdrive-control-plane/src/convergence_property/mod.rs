//! Slice 04 of `reconciler-convergence-guarantees` (GH #351; feature delta D-6,
//! D-7, D-12): the shared multi-tick convergence property over every registered
//! reconciler, and the per-reconciler `managed_targets` containment obligation.
//!
//! A source-local module (not an integration test) because the property drives
//! re-evaluation through `resync_reconciler`, the crate-private per-reconciler
//! step of the resync pass. Default lane (V-5): the production
//! [`run_convergence_tick`] over a sim-composed [`AppState`] with a per-case
//! temporary redb view store, the fixture
//! `tick_context_now_unix_runtime.rs` runs in the default lane; no kernel, no
//! network.
//!
//! One round is: advance the injected clock, run the reconciler's pass step
//! (`resync_reconciler`), admit what the broker holds through the production
//! `drain_pending`, and run each admitted evaluation through
//! `run_convergence_tick`. Removing the pass therefore stalls every world that
//! needs a re-evaluation the edge never delivered, which is how the property
//! catches the "remove the pass" mutant.
//!
//! Modules: `hydrator`, `workload`, `svid`, `service`, `vm_reclaim` (one
//! property each), `executor_idempotency` (the companion), and
//! `managed_targets_containment` (WK-4). `noop-heartbeat` and
//! `workflow-lifecycle` carry reasoned no-obligation rulings, and
//! `gateway-svid-lifecycle` has its own module (`gateway`) over a node whose
//! gateway composition is enabled: see
//! `docs/feature/reconciler-convergence-guarantees/distill/test-scenarios.md`
//! § "Slice 04".

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "the harness asserts its own preconditions; the repository-mandated \
              CONTRACT_SHAPE declaration is an exact machine-read line"
)]

mod executor_idempotency;
mod gateway;
mod hydrator;
mod managed_targets_containment;
mod service;
mod svid;
mod vm_reclaim;
mod workload;

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{
    DriverInput, IntentKey, Job, JobSpecInput, ResourcesInput, Service, VmInput, WorkloadIntent,
    WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip, WorkloadId};
use overdrive_core::reconcilers::{HydrateError, ReconcilerName, TargetResource};
use overdrive_core::traits::ca::Ca;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore, TransitionSource,
};
use overdrive_dataplane::mtls::gateway_identity::GatewayIdentitySlot;
use overdrive_reconcilers::AnyReconciler;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::vm_host_state::SimVmHostState;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

use crate::AppState;
use crate::reconciler_runtime::{ReconcilerRuntime, resync_reconciler, run_convergence_tick};

/// Rounds a world may take to converge.
pub const ROUNDS: usize = 8;
/// Extra rounds run after convergence to observe stability.
pub const STABLE_ROUNDS: usize = 3;
/// Clock advance per round: longer than every reconciler's one-second backoff.
pub const ROUND_ADVANCE: Duration = Duration::from_secs(2);
/// The convergence loop's in-flight cap (`CONVERGENCE_MAX_IN_FLIGHT`).
const ADMISSION_CAP: usize = 8;

pub fn node(name: &str) -> NodeId {
    NodeId::from_str(name).expect("valid NodeId")
}

pub fn workload_id(name: &str) -> WorkloadId {
    WorkloadId::new(name).expect("valid WorkloadId")
}

pub fn target_of(prefix: &str, id: &str) -> TargetResource {
    TargetResource::new(&format!("{prefix}/{id}")).expect("valid TargetResource")
}

pub fn name_of(reconciler: &str) -> ReconcilerName {
    ReconcilerName::new(reconciler).expect("valid ReconcilerName")
}

/// Whether the node is composed with the public-ingress gateway.
enum GatewayComposition {
    Disabled,
    /// Enabled, with the retry memory a previous process persisted (if any).
    Enabled(Option<gateway::PersistedRetry>),
}

/// The sim-composed node the properties run over: the production reconciler
/// runtime with the named reconcilers registered, inside the [`AppState`]
/// `run_convergence_tick` takes, with `Sim*` adapters at every driven port.
pub struct Composed {
    _tmp: TempDir,
    pub state: AppState,
    pub obs: Arc<SimObservationStore>,
    pub dataplane: Arc<SimDataplane>,
    pub clock: Arc<SimClock>,
    pub driver: Arc<SimDriver>,
    pub vm_host: SimVmHostState,
    /// The one gateway identity slot, present when the node was composed with
    /// the gateway enabled ([`Composed::new_with_gateway`]).
    pub gateway_slot: Option<Arc<GatewayIdentitySlot>>,
    tick_n: AtomicU64,
    row_counter: AtomicU64,
}

impl Composed {
    pub async fn new(reconcilers: Vec<AnyReconciler>) -> Self {
        Self::build(reconcilers, GatewayComposition::Disabled).await
    }

    /// The same node with the public-ingress gateway enabled, as `run_server`
    /// composes it when `config.gateway` is `Some` (see `gateway::compose`).
    /// `persisted_retry` is the retry memory a previous process left in the
    /// `gateway-svid-lifecycle` View (`None`: the forgotten View).
    pub async fn new_with_gateway(
        reconcilers: Vec<AnyReconciler>,
        persisted_retry: Option<gateway::PersistedRetry>,
    ) -> Self {
        Self::build(reconcilers, GatewayComposition::Enabled(persisted_retry)).await
    }

    async fn build(reconcilers: Vec<AnyReconciler>, gateway: GatewayComposition) -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let clock = Arc::new(SimClock::new());
        if let GatewayComposition::Enabled(Some(retry)) = gateway {
            gateway::persist_retry(tmp.path(), retry, clock.as_ref()).await;
        }
        let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path())
            .expect("reconciler runtime");
        for reconciler in reconcilers {
            runtime.register(reconciler).await.expect("register reconciler");
        }
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
        let obs = Arc::new(SimObservationStore::single_peer(node("local"), 42));
        let dataplane = Arc::new(SimDataplane::new());
        let ca = Arc::new(SimCa::new(Arc::new(SimEntropy::new(0))));
        let driver =
            Arc::new(SimDriver::with_clock(DriverType::Vm, Arc::clone(&clock) as Arc<dyn Clock>));
        let vm_host = SimVmHostState::new();
        let allocator = crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
        let mtls_worker = crate::shared_network_test_ports::mtls_worker();
        mtls_worker.start_shared_owner().await.expect("shared mTLS owner starts over sim adapters");
        let guest_owner = crate::shared_network_test_ports::shared_guest_network_owner();
        crate::guest_network::SharedGuestNetworkOwner::converge_shared(guest_owner.as_ref())
            .await
            .expect("shared owner reports its bound flow ports");
        let wiring = overdrive_core::guest_network::GuestNetworkExecWiring::new(
            Arc::clone(&clock) as Arc<dyn Clock>
        );
        assert!(wiring.supervisor().open_after_boot(), "EXEC opens after both owners converge");
        let mut state = AppState::new(
            store,
            store_path,
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::new(runtime),
            Arc::clone(&driver) as Arc<dyn Driver>,
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&dataplane),
            Arc::clone(&ca) as Arc<dyn Ca>,
            Arc::new(crate::identity_mgr::IdentityMgr::new(None)),
            node("writer-1"),
            allocator,
            crate::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            mtls_worker,
            guest_owner,
            wiring.gate(),
            crate::shared_network_test_ports::guest_pool(),
        );
        state.vm_host_state = Arc::new(vm_host.clone());
        let gateway_slot = if matches!(gateway, GatewayComposition::Enabled(_)) {
            let (composition, slot) = gateway::compose(
                tmp.path(),
                &state.store,
                &state.obs,
                &state.clock,
                ca.as_ref(),
                &state.node_id,
            )
            .await;
            state.gateway = composition;
            Some(slot)
        } else {
            None
        };
        Self {
            _tmp: tmp,
            state,
            obs,
            dataplane,
            clock,
            driver,
            vm_host,
            gateway_slot,
            tick_n: AtomicU64::new(0),
            row_counter: AtomicU64::new(0),
        }
    }

    /// One round for `reconciler` (see the module doc).
    ///
    /// An error is never discarded. An enumeration error from the pass always
    /// fails the round (no property arms a read fault, so a compliant pass
    /// enumerates cleanly). A tick error fails the round unless `faults_armed`
    /// says the world carries a dropped effect that legitimately surfaces as
    /// one (a rejected start, a failed update, a failed write): that error is
    /// the fault, and the next round retries it. The stability phase always
    /// passes `false`: once converged, no tick may error.
    pub async fn round(&self, reconciler: &ReconcilerName, faults_armed: bool) {
        self.clock.tick(ROUND_ADVANCE);
        let now: Instant = Clock::now(&*self.clock);
        let now_unix = UnixInstant::from_clock(&*self.clock);
        if let Err(error) = resync_reconciler(&self.state, reconciler).await {
            panic!("{reconciler}: the pass failed to enumerate its targets: {error}");
        }
        let admitted = {
            let mut broker = self.state.runtime.broker();
            broker.drain_pending(ADMISSION_CAP, &BTreeSet::new(), now, now_unix)
        };
        for (evaluation, _queued_for) in admitted {
            let tick = self.tick_n.fetch_add(1, Ordering::SeqCst);
            let outcome = run_convergence_tick(
                &self.state,
                &evaluation.reconciler,
                &evaluation.target,
                now,
                tick,
                now + Duration::from_millis(100),
            )
            .await;
            if let Err(error) = outcome {
                assert!(
                    faults_armed,
                    "{reconciler}: tick for {} errored with no fault armed: {error}",
                    evaluation.target
                );
            }
        }
    }

    /// `managed_targets` of the registered `reconciler`, through the same
    /// hydration context the tick builds.
    #[expect(
        clippy::significant_drop_tightening,
        reason = "guards lent into HydrationContext must outlive the managed_targets .await"
    )]
    pub async fn managed_targets(
        &self,
        reconciler: &ReconcilerName,
    ) -> Result<BTreeSet<TargetResource>, HydrateError> {
        let registered = self.state.runtime.get(reconciler).expect("reconciler is registered");
        let allocator = self.state.allocator.lock().await;
        let listener_facts = self.state.listener_facts.lock().await;
        let ctx = crate::reconciler_runtime::build_hydration_context(
            &self.state,
            &allocator,
            &listener_facts,
        );
        registered.managed_targets(&ctx).await
    }

    /// The next monotone logical-timestamp counter for a precondition row.
    pub fn next_counter(&self) -> u64 {
        self.row_counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Persist a Job intent (and its kind marker) as the submit edge does.
    pub async fn put_job(&self, id: &str) {
        let job = Job::from_submit(JobSpecInput {
            id: id.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
            driver: DriverInput::Vm(VmInput {
                command: "/bin/sleep".to_owned(),
                args: vec!["3600".to_owned()],
                kernel: "/kernel".to_owned(),
                rootfs: "/rootfs".to_owned(),
            }),
        })
        .expect("valid job spec");
        self.persist(&workload_id(id), &WorkloadIntent::Job(job), WorkloadKind::Job).await;
    }

    /// Persist a Service intent with one TCP listener, allocate its VIP through
    /// the production allocator and record its listener fact, as the submit
    /// edge does. Returns the service's id and VIP.
    pub async fn put_service(&self, id: &str) -> (ServiceId, ServiceVip) {
        let svc = Service::from_submit(ServiceSpecInput {
            id: id.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
            driver: DriverInput::Vm(VmInput {
                command: "/bin/serve".to_owned(),
                args: vec![],
                kernel: "/kernel".to_owned(),
                rootfs: "/rootfs".to_owned(),
            }),
            listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
            startup_probes: vec![],
            readiness_probes: vec![],
            liveness_probes: vec![],
        })
        .expect("valid service spec");
        let intent = WorkloadIntent::Service(svc.clone());
        self.persist(&svc.id, &intent, WorkloadKind::Service).await;
        let digest = *intent.spec_digest().expect("spec_digest").as_bytes();
        let vip = self.state.allocator.lock().await.allocate(digest).await.expect("allocate vip");
        self.state.listener_facts.lock().await.upsert(svc.id.clone(), &vip, &svc.listeners);
        let proto = svc.listeners[0].protocol;
        let sid = ServiceId::derive(
            &vip,
            std::num::NonZeroU16::new(8080).expect("non-zero port"),
            proto,
            "service-map",
        );
        (sid, vip)
    }

    async fn persist(&self, id: &WorkloadId, intent: &WorkloadIntent, kind: WorkloadKind) {
        let archived = intent.archive_for_store().expect("rkyv archive");
        self.state
            .store
            .put(IntentKey::for_workload(id).as_bytes(), archived.as_ref())
            .await
            .expect("put intent");
        self.state
            .store
            .put(IntentKey::for_workload_kind(id).as_bytes(), &[kind.discriminator_byte()])
            .await
            .expect("put kind");
    }

    /// Write an allocation row as the owning node writes it (a scenario
    /// precondition, never a consequence).
    #[cfg(test)]
    pub async fn write_alloc_row(
        &self,
        workload: &str,
        alloc: &str,
        state: AllocState,
        kind: WorkloadKind,
        workload_addr: Option<Ipv4Addr>,
    ) {
        let started_at = matches!(state, AllocState::Running | AllocState::Terminated)
            .then(|| UnixInstant::from_unix_duration(self.clock.unix_now()));
        let row = AllocStatusRow {
            alloc_id: AllocationId::new(alloc).expect("alloc id"),
            workload_id: workload_id(workload),
            node_id: node("local"),
            state,
            updated_at: LogicalTimestamp { counter: self.next_counter(), writer: node("local") },
            reason: None,
            detail: None,
            terminal: None,
            stderr_tail: None,
            kind,
            listeners: vec![],
            started_at,
            workload_addr,
            last_terminated: None,
            restart_count: 0,
        };
        self.obs
            .write_alloc_lifecycle(row, TransitionSource::Reconciler)
            .await
            .expect("write allocation row");
    }

    /// Every allocation row, as `(alloc, state)` in id order: the observable a
    /// further action could change.
    pub async fn alloc_rows(&self) -> Vec<(String, AllocState)> {
        let mut rows: Vec<(String, AllocState)> = self
            .obs
            .alloc_status_rows()
            .await
            .expect("alloc rows")
            .into_iter()
            .map(|row| (row.alloc_id.to_string(), row.state))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }
}

/// A tokio runtime for one property run.
pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime")
}

/// The property configuration: a fixed seed (printed with the failing case), so
/// a failing run reproduces exactly (`testing.md` § "Property-based testing").
pub fn property_config(cases: u32, seed: u64) -> proptest::test_runner::Config {
    let mut config = proptest::test_runner::Config::with_cases(cases);
    config.failure_persistence = None;
    config.rng_seed = proptest::test_runner::RngSeed::Fixed(seed);
    config
}
