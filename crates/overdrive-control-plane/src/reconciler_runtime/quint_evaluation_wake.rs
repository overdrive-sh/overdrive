//! Quint conformance driver — `evaluation_wake` (feature
//! reconciler-convergence-guarantees, GH #351: D-1 the resync pass, D-3
//! `ServiceMapHydrator::next_evaluation_at`, D-12 the conformance pins).
//!
//! Replays `specs/quint/reconciler-evaluation-wake/evaluation_wake.qnt`
//! (referenced in place), conformance main `wake_conf` (module `evaluation_wake`,
//! recording `conf_step`, `dispatched` and `cancelled`), against the production
//! owner: the reconciler runtime (`ReconcilerRuntime` with its evaluation
//! broker) holding the four production reconcilers the model's keys name,
//! inside the sim-composed `AppState` that `run_convergence_tick` takes, with
//! `SimObservationStore`, `SimDataplane`, `SimDriver` and
//! `SimClock` at the driven ports. The production `Action::EnqueueEvaluation` handoffs
//! stay on: the model's `Finish` carries them (`HAND`), so the driver suppresses nothing.
//!
//! Key -> production pair (fixed; the spec keys are production-reachable pairs), all on one
//! Service workload `orders` that declares a listener:
//!
//! | Model key | Reconciler (model name) | Production reconciler | Target |
//! |---|---|---|---|
//! | `k1` | `r4W` | `workload-lifecycle` | `workload/orders` |
//! | `k3` | `r1S` | `service-lifecycle` | `workload/orders` (the same target as `k1`: per-target exclusion) |
//! | `k4` | `r3V` | `svid-lifecycle` | `workload/orders` (the same target) |
//! | `k2` | `r2M` | `service-map-hydrator` | `service/<id>` of the `orders` listener (alone on its target) |
//!
//! The abstract names sort like the production names, so the pass walks `r1S, r2M, r3V, r4W` =
//! `service-lifecycle, service-map-hydrator, svid-lifecycle, workload-lifecycle`, the production
//! `BTreeMap` order. The driver calls the per-reconciler driving port `resync_reconciler` in that
//! order (4 calls per pass).
//!
//! Action -> production call (every handler calls the owner or mutates the world
//! through a production driven port; nothing is bookkeeping-only):
//!
//! | Action | Driver |
//! |---|---|
//! | `init` | compose the world (runtime registering the four reconcilers over the redb View store, `AppState`, `Sim*` ports) |
//! | `Tick` | the period timer is not a driving port (the driver calls `resync_reconciler` directly): the runtime is driven to quiescence and the comparison checks the timer firing changed no broker state |
//! | `PassStep({enumOk})` | `resync_reconciler(state, <the next reconciler in pass order>)`; `enumOk = false` arms a sticky read fault at the `SimObservationStore` for that reconciler's enumeration read (allocation rows for `workload-lifecycle` / `service-lifecycle` / `svid-lifecycle`, service rows for `service-map-hydrator`), disarmed after the call |
//! | `Drain` | `BrokerDrain::drain_pending(cap - in_flight, in_flight_targets, now, now_unix)`; the admitted evaluations are the driver's in-flight set (its stimulus) |
//! | `Finish({k, o})` | `run_convergence_tick` for the admitted evaluation of `k`, with `o` injected at its driven port first: `ok` none; `dpFail` `SimDataplane::script_update_service_failure`; `hydrateErr` a sticky read fault at `SimObservationStore`; `persistErr` a failing durable write at the View store; `dispatchErr` a one-shot write failure at `SimObservationStore` (`inject_alloc_lifecycle_write_failure` for `workload-lifecycle`, and for `service-lifecycle` while the listener has no VIP, because it then writes no service row (its write is the Stable commit); `inject_write_failure` for `service-map-hydrator`, for `service-lifecycle` with a VIP (the service-row writers) and for `svid-lifecycle` (the `IssueSvid` audit write goes through `ObservationStore::write`, so a failed issue leaves the SVID unheld and puts `k4` in its own backoff window)). The handoff wakes (`k1` -> `k3`, `k4`; `k3` -> `k2`, only with a VIP) are submitted by the production action shim during the tick, not by the driver |
//! | `DivergeWake({k})` | the world change of `DivergeDrop`, then `ReconcilerRuntime::submit_evaluation(.., Immediate)` |
//! | `DivergeDrop({k})` | the world change alone, through the intent store and `SimObservationStore` (see below) |
//! | `EdgeWake({k})` | `ReconcilerRuntime::submit_evaluation(.., Immediate)` |
//! | `WindowEnds({k})` | `SimClock::tick(RESTART_BACKOFF_DURATION)`: one global advance. `backoff_for_attempt` ignores the attempt (one constant) and the hydrator's and svid-lifecycle's windows are both `last_failure_seen_at + backoff_for_attempt(..)` with an in-window test `now < deadline`, so the advance ends every open window (including `k4`'s) and makes every retained `NotBefore` eligible, whichever `k` the step names |
//! | `Crash` | drop the runtime and its broker and rebuild `ReconcilerRuntime` over the same View store (`register` runs `bulk_load`; the rebuilt broker has zero counters); intent, observation and the held SVIDs (`AppState::identity`, the same `IdentityMgr`) survive, as the model's `Crash` keeps `held` |
//!
//! World construction. The `orders` Service declares a listener; `has` / `need` are the
//! driver's stimulus, never projected. The managed-set rule (user ruling) is that a Service is
//! managed by `service-lifecycle` only once it has an allocation or an assigned VIP, never on intent alone:
//!
//! * `k1` need: no Running allocation. The `orders` intent is persisted (the managed set; no VIP
//!   is assigned and no allocation exists, so `service-lifecycle` and `svid-lifecycle` still manage
//!   nothing); a Running allocation is replaced by a `Failed` row with the restart memory seeded as
//!   a long-ago failure, so the restart is immediate. (`workload-lifecycle` ignores
//!   `replicas`, so a second replica is not used.) The model's consequences: `k3` becomes needy when
//!   a VIP exists, `k4` when an SVID is held (a `DropSvid` for an allocation no longer Running).
//! * `k3` need: the `orders` listener has a VIP assigned and its listener fact recorded (never
//!   intent alone), with no observed `ServiceBackendRow` matching the projection (before the first
//!   publish, or after the backends changed; an already published row is replaced by a
//!   dominating over-broad one). The intent is persisted too, which brings `k1` into `has`.
//! * `k4` need: a Running allocation row with no SVID held (the intent is persisted too). A fresh
//!   allocation is added every time, so a converged `k4` can need work again. An allocation row of
//!   the Service makes `service-lifecycle` manage it, so `k4` implies `k3` (managed-set rule), and
//!   the Running allocation clears `need(k1)`.
//! * `k2` need: the workload intent, the `orders` listener's VIP and listener fact, and a
//!   `ServiceBackendRow` with a new backend set, so desired differs from the programmed map; the
//!   observed row differs from `service-lifecycle`'s projection, so `k3` is needy too.
//!   The `orders` listener's `ServiceId` is fixed at composition by a scratch allocator (no world
//!   effect) so the key table can name `k2`'s target before the VIP is assigned; the world
//!   change asserts the production allocator yields the same id.
//!
//! Projection (`State::from_driver`): `pending` <- `BrokerDrain::pending_keys()`
//! (admission order), mapped through the fixed table above; `dispatched`,
//! `cancelled` <- `BrokerDrain::counters()`. Nothing else is projected. `has`,
//! `need`, `inWin`, `elig`, `active`, `due`, `passPos`, `faults`, `drifts` and
//! `conf_step` are model-only or stimulus. See
//! `docs/feature/reconciler-convergence-guarantees/distill/conformance-driver.md`.
//!
//! Every named run of `wake_conf` is pinned with `#[quint_test]`; `#[quint_run]`
//! replays randomized traces. Lane: `integration-tests`, `cargo xtask metal run --`.
//!

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::significant_drop_tightening,
    clippy::needless_pass_by_value,
    clippy::needless_pass_by_ref_mut,
    clippy::unused_self,
    clippy::unnecessary_wraps,
    non_snake_case,
    reason = "a conformance driver asserts its own preconditions; protocol names are prose"
)]

use overdrive_core::byte_size::ByteSize;
use overdrive_core::cpu_quantity::CpuQuantity;
use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use itf::Value;
use overdrive_core::SpiffeId;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, Service, VmInput, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::eval_broker::{Evaluation, EvaluationEligibility};
use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip, WorkloadId};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::{Backend, DataplaneError};
use overdrive_core::traits::driver::{Driver as DriverPort, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore, ObservationStoreError,
    ObservationWrite, ServiceBackendRow, TransitionSource,
};
use overdrive_reconcilers::workload_lifecycle::{RESTART_BACKOFF_DURATION, WorkloadLifecycleView};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use tempfile::TempDir;
use tokio::runtime::Runtime;

use super::{ReconcilerRuntime, resync_reconciler, run_convergence_tick};
use crate::AppState;
use crate::guest_network::quint_itf::{self, ItfError, Qualified};
use crate::view_store::redb::RedbViewStore;
use crate::view_store::{ProbeError, ViewStore, ViewStoreError};

use overdrive_testing::placement;

placement!(host_pure);

const MAIN: &str = "wake_conf";
const MODULE: &str = "evaluation_wake";
/// The model's `CAP`, standing for the convergence loop's in-flight cap.
const CAP: usize = 2;
/// The workload `k1` and `k3` share (one target, `workload/orders`).
const ORDERS: &str = "orders";
const LISTENER_PORT: u16 = 8080;
/// A Running allocation's workload address (the hydrate invariant requires one).
const WORKLOAD_ADDR: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 2);

/// The model's pass order (`r1S`, `r2M`, `r3V`, `r4W`), which is the production `BTreeMap` order of the names.
const PASS_ORDER: [&str; 4] =
    ["service-lifecycle", "service-map-hydrator", "svid-lifecycle", "workload-lifecycle"];

#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("unknown key `{0}`")]
    UnknownKey(String),
    #[error("unknown outcome `{0}`")]
    UnknownOutcome(String),
    #[error("`{action}`: the model enables this but the driver's stimulus does not hold: {why}")]
    NotEnabled { action: &'static str, why: String },
    #[error("a convergence tick that must succeed failed: {0}")]
    Tick(String),
    #[error("the reconciler step of the pass failed although no fault was armed: {0}")]
    Pass(String),
    #[error("the runtime did not rebuild over the same stores: {0}")]
    Rebuild(String),
}

/// A model evaluation key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    K1,
    K2,
    K3,
    K4,
}

impl Key {
    fn parse(key: &str) -> std::result::Result<Self, DriverError> {
        match key {
            "k1" => Ok(Self::K1),
            "k2" => Ok(Self::K2),
            "k3" => Ok(Self::K3),
            "k4" => Ok(Self::K4),
            other => Err(DriverError::UnknownKey(other.to_owned())),
        }
    }

    const ALL: [Self; 4] = [Self::K1, Self::K2, Self::K3, Self::K4];

    fn model_name(self) -> &'static str {
        match self {
            Self::K1 => "k1",
            Self::K2 => "k2",
            Self::K3 => "k3",
            Self::K4 => "k4",
        }
    }

    fn reconciler_name(self) -> &'static str {
        match self {
            Self::K1 => "workload-lifecycle",
            Self::K2 => "service-map-hydrator",
            Self::K3 => "service-lifecycle",
            Self::K4 => "svid-lifecycle",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Ok,
    DpFail,
    HydrateErr,
    PersistErr,
    DispatchErr,
}

impl Outcome {
    fn parse(outcome: &str) -> std::result::Result<Self, DriverError> {
        match outcome {
            "ok" => Ok(Self::Ok),
            "dpFail" => Ok(Self::DpFail),
            "hydrateErr" => Ok(Self::HydrateErr),
            "persistErr" => Ok(Self::PersistErr),
            "dispatchErr" => Ok(Self::DispatchErr),
            other => Err(DriverError::UnknownOutcome(other.to_owned())),
        }
    }
}

fn node(name: &str) -> NodeId {
    NodeId::new(name).expect("valid NodeId")
}

fn reconciler(name: &str) -> ReconcilerName {
    ReconcilerName::new(name).expect("valid ReconcilerName")
}

fn target(text: &str) -> TargetResource {
    TargetResource::new(text).expect("valid TargetResource")
}

async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

fn service_spec(id: &str) -> ServiceSpecInput {
    ServiceSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput {
            cpu: CpuQuantity::from_millis(100),
            memory: ByteSize::new(128 * 1024 * 1024),
        },
        driver: DriverInput::Vm(VmInput {
            volume: None,
            command: "/bin/serve".to_owned(),
            args: vec![],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
        listeners: vec![ListenerInput { port: LISTENER_PORT, protocol: "tcp".to_owned() }],
        // The explicit startup opt-out: the first Running allocation is its Stable.
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

async fn register_reconcilers(runtime: &mut ReconcilerRuntime) {
    runtime.register(crate::workload_lifecycle()).await.expect("register workload-lifecycle");
    runtime.register(crate::service_lifecycle()).await.expect("register service-lifecycle");
    runtime.register(crate::service_map_hydrator()).await.expect("register service-map-hydrator");
    runtime.register(crate::svid_lifecycle()).await.expect("register svid-lifecycle");
}

// ---------------------------------------------------------------- the view store

/// The production `RedbViewStore` (the View store a restarted runtime reloads from)
/// with one fault: the durable write failing, the driven-port fault of the model's
/// `persistErr`. Test-local; it adds no production surface.
struct FaultableViewStore {
    inner: RedbViewStore,
    fsync_fails: AtomicBool,
}

impl FaultableViewStore {
    fn set_fsync_fails(&self, fails: bool) {
        self.fsync_fails.store(fails, Ordering::SeqCst);
    }
}

#[async_trait]
impl ViewStore for FaultableViewStore {
    async fn bulk_load_bytes(
        &self,
        reconciler: &'static str,
    ) -> std::result::Result<BTreeMap<TargetResource, Vec<u8>>, ViewStoreError> {
        self.inner.bulk_load_bytes(reconciler).await
    }

    async fn write_through_bytes(
        &self,
        reconciler: &'static str,
        target: &TargetResource,
        cbor: &[u8],
    ) -> std::result::Result<(), ViewStoreError> {
        if self.fsync_fails.load(Ordering::SeqCst) {
            return Err(ViewStoreError::FsyncFailed {
                message: "the view store refused the durable write".to_owned(),
            });
        }
        self.inner.write_through_bytes(reconciler, target, cbor).await
    }

    async fn delete(
        &self,
        reconciler: &'static str,
        target: &TargetResource,
    ) -> std::result::Result<(), ViewStoreError> {
        self.inner.delete(reconciler, target).await
    }

    async fn probe(&self) -> std::result::Result<(), ProbeError> {
        self.inner.probe().await
    }
}

// ---------------------------------------------------------------- the world

struct WakeWorld {
    state: AppState,
    obs: Arc<SimObservationStore>,
    clock: Arc<SimClock>,
    dataplane: Arc<SimDataplane>,
    view_store: Arc<FaultableViewStore>,
    /// The hydrator key's target, `service/<id>` of the `orders` listener.
    service_target: TargetResource,
    /// The `orders` listener's service id, fixed at composition.
    service_id: ServiceId,
    tmp: TempDir,
    // ----- stimulus (never projected)
    /// The convergence loop's in-flight set: the evaluations `Drain` admitted
    /// and `Finish` has not yet run (the cap and per-target exclusion live in
    /// the loop, which is not a driving port).
    active: BTreeSet<(ReconcilerName, TargetResource)>,
    /// The reconciler the next `PassStep` walks (an index into [`PASS_ORDER`]).
    pass_pos: usize,
    tick_n: u64,
    counter: u64,
    next_aux_alloc: u32,
    backend_generation: u8,
    /// The `orders` listener has an assigned VIP (the model's `vip`; stimulus, never projected).
    vip_assigned: bool,
}

impl WakeWorld {
    async fn compose() -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let view_store = Arc::new(FaultableViewStore {
            inner: RedbViewStore::open(tmp.path()).expect("open the view store"),
            fsync_fails: AtomicBool::new(false),
        });
        let mut runtime =
            ReconcilerRuntime::new(tmp.path(), Arc::clone(&view_store) as Arc<dyn ViewStore>)
                .expect("reconciler runtime");
        register_reconcilers(&mut runtime).await;

        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
        let obs = Arc::new(SimObservationStore::single_peer(node("local"), 0));
        let clock = Arc::new(SimClock::new());
        let driver = Arc::new(SimDriver::with_clock(DriverType::Vm, clock.clone()));
        let dataplane = Arc::new(SimDataplane::new());
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
        let state = AppState::new(
            store,
            store_path,
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::new(runtime),
            Arc::clone(&driver) as Arc<dyn DriverPort>,
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&dataplane),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(0)))),
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

        // The `orders` listener's service id, fixed here so the key table can name the hydrator
        // key's target before any VIP is assigned in the world. A scratch allocator over a scratch
        // store yields it without touching `state.allocator`; `ensure_orders_listener` asserts the
        // production allocator assigns the same id.
        let scratch_store = Arc::new(
            LocalIntentStore::open(tmp.path().join("scratch-vip.redb")).expect("scratch store"),
        );
        let scratch_allocator =
            crate::test_default_allocator(scratch_store as Arc<dyn IntentStore>);
        let orders = Self::orders_service();
        let vip = scratch_allocator
            .lock()
            .await
            .allocate(Self::orders_digest(&orders))
            .await
            .expect("allocate vip");
        let service_id = ServiceId::derive(
            &vip,
            NonZeroU16::new(LISTENER_PORT).expect("non-zero port"),
            orders.listeners[0].protocol,
            "service-map",
        );

        Self {
            state,
            obs,
            clock,
            dataplane,
            view_store,
            service_target: target(&format!("service/{service_id}")),
            service_id,
            tmp,
            active: BTreeSet::new(),
            pass_pos: 0,
            tick_n: 0,
            counter: 0,
            next_aux_alloc: 90,
            backend_generation: 0,
            vip_assigned: false,
        }
    }

    fn now(&self) -> Instant {
        Clock::now(&*self.clock)
    }

    fn orders() -> WorkloadId {
        WorkloadId::new(ORDERS).expect("valid WorkloadId")
    }

    fn orders_service() -> Service {
        Service::from_submit(service_spec(ORDERS)).expect("valid service spec")
    }

    fn orders_digest(service: &Service) -> [u8; 32] {
        *WorkloadIntent::Service(service.clone()).spec_digest().expect("spec_digest").as_bytes()
    }

    fn target_of(&self, key: Key) -> TargetResource {
        match key {
            Key::K1 | Key::K3 | Key::K4 => target(&format!("workload/{ORDERS}")),
            Key::K2 => self.service_target.clone(),
        }
    }

    /// The fixed table, production pair -> model key name.
    fn key_name(&self, name: &ReconcilerName, target: &TargetResource) -> String {
        Key::ALL
            .into_iter()
            .find(|key| key.reconciler_name() == name.as_str() && self.target_of(*key) == *target)
            .map_or_else(
                || format!("unmapped:{name}@{}", target.as_str()),
                |key| key.model_name().to_owned(),
            )
    }

    fn evaluation(&self, key: Key) -> Evaluation {
        Evaluation { reconciler: reconciler(key.reconciler_name()), target: self.target_of(key) }
    }

    fn submit(&self, key: Key) {
        self.state.runtime.submit_evaluation(
            self.evaluation(key),
            self.now(),
            EvaluationEligibility::Immediate,
        );
    }

    fn next_counter(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    // ------------------------------------------------------------ read faults

    /// The sticky read fault of the reconciler's enumeration or hydration read.
    fn arm_read_fault(&self, reconciler_name: &str, armed: bool) {
        if reconciler_name == "service-map-hydrator" {
            self.obs.script_service_rows_read_failure(armed);
        } else {
            self.obs.script_alloc_status_rows_read_failure(armed);
        }
    }

    // ------------------------------------------------------------ world changes

    async fn persist_orders_intent(&self) {
        let id = Self::orders();
        let svc = Self::orders_service();
        let archived =
            WorkloadIntent::Service(svc).archive_for_store().expect("rkyv archive of the intent");
        self.state
            .store
            .put(IntentKey::for_workload(&id).as_bytes(), archived.as_ref())
            .await
            .expect("put intent");
        self.state
            .store
            .put(
                IntentKey::for_workload_kind(&id).as_bytes(),
                &[WorkloadKind::Service.discriminator_byte()],
            )
            .await
            .expect("put kind");
    }

    async fn orders_rows(&self) -> Vec<AllocStatusRow> {
        let id = Self::orders();
        self.obs
            .alloc_status_rows()
            .await
            .expect("alloc rows")
            .into_iter()
            .filter(|row| row.workload_id == id)
            .collect()
    }

    /// The `orders` listener: its VIP is allocated and its listener fact recorded, as the
    /// submit edge leaves them. Until then `service-lifecycle` manages nothing for the
    /// workload (no allocation, no VIP), so it is not in that reconciler's managed set.
    /// Idempotent: the allocator returns the VIP it already holds.
    async fn ensure_orders_listener(&mut self) -> ServiceVip {
        let orders = Self::orders_service();
        let vip = self
            .state
            .allocator
            .lock()
            .await
            .allocate(Self::orders_digest(&orders))
            .await
            .expect("allocate vip");
        self.state.listener_facts.lock().await.upsert(orders.id.clone(), &vip, &orders.listeners);
        let id = ServiceId::derive(
            &vip,
            NonZeroU16::new(LISTENER_PORT).expect("non-zero port"),
            orders.listeners[0].protocol,
            "service-map",
        );
        assert_eq!(id, self.service_id, "the production allocator assigns the composed service id");
        self.vip_assigned = true;
        vip
    }

    /// An allocation row as the owning node writes it (a precondition). The row's
    /// timestamp dominates the row it replaces, as the owning node's write does.
    // `cfg(test)`: dst-lint scans this file as production source and bans the row literals;
    // the module itself is test-only.
    #[cfg(test)]
    async fn write_orders_row(&mut self, alloc: &AllocationId, state: AllocState) {
        let running = state == AllocState::Running;
        let prior = self
            .orders_rows()
            .await
            .into_iter()
            .find(|row| row.alloc_id == *alloc)
            .map(|row| row.updated_at);
        let tick_floor = self.next_counter();
        let row = AllocStatusRow {
            alloc_id: alloc.clone(),
            workload_id: Self::orders(),
            node_id: node("local"),
            state,
            updated_at: LogicalTimestamp::dominating(tick_floor, node("local"), prior.as_ref()),
            reason: None,
            detail: None,
            terminal: None,
            stderr_tail: None,
            kind: WorkloadKind::Service,
            listeners: vec![],
            started_at: running.then(|| UnixInstant::from_clock(&*self.clock)),
            workload_addr: running.then_some(WORKLOAD_ADDR),
            last_terminated: None,
            restart_count: 0,
        };
        self.obs
            .write_alloc_lifecycle(row, TransitionSource::Reconciler)
            .await
            .expect("write allocation row");
    }

    /// `k1`: the workload is managed and has no Running allocation.
    #[cfg(test)]
    async fn make_workload_needy(&mut self) {
        self.persist_orders_intent().await;
        let running: Vec<AllocationId> = self
            .orders_rows()
            .await
            .into_iter()
            .filter(|row| row.state == AllocState::Running)
            .map(|row| row.alloc_id)
            .collect();
        for alloc in running {
            self.write_orders_row(&alloc, AllocState::Failed).await;
            // The failure was seen long ago: the restart is not inside its backoff.
            self.state.runtime.seed_workload_lifecycle_view_for_test(
                &self.target_of(Key::K1),
                WorkloadLifecycleView {
                    restart_counts: BTreeMap::from([(alloc.clone(), 0)]),
                    last_failure_seen_at: BTreeMap::from([(
                        alloc,
                        UnixInstant::from_unix_duration(Duration::ZERO),
                    )]),
                    ..Default::default()
                },
            );
        }
    }

    /// `k3`: the `orders` listener has a VIP and a listener fact (never the intent alone), and
    /// no observed backend row matches its projection. The intent is persisted too (it brings
    /// `k1` into the managed set). An already published row would satisfy the projection:
    /// it is replaced by a dominating over-broad one (an extra backend).
    #[cfg(test)]
    async fn make_service_needy(&mut self) {
        self.persist_orders_intent().await;
        self.ensure_orders_listener().await;
        let published = self
            .obs
            .service_backends_rows(&self.service_id)
            .await
            .expect("service rows")
            .into_iter()
            .next();
        if let Some(prior) = published {
            let mut backends = prior.backends.clone();
            backends.push(Backend {
                alloc: SpiffeId::from_str("spiffe://overdrive.local/workload/orders/alloc/extra")
                    .expect("spiffe"),
                addr: SocketAddr::new(Ipv4Addr::new(10, 9, 9, 9).into(), LISTENER_PORT),
                weight: 1,
                healthy: true,
            });
            let tick_floor = self.next_counter();
            self.obs
                .write(ObservationWrite::ServiceBackend(ServiceBackendRow {
                    service_id: prior.service_id,
                    vip: prior.vip,
                    backends,
                    updated_at: LogicalTimestamp::dominating(
                        tick_floor,
                        node("writer-1"),
                        Some(&prior.updated_at),
                    ),
                }))
                .await
                .expect("write service_backends");
        }
    }

    /// `k4`: an allocation row for the workload that holds no SVID. A fresh Running allocation is
    /// added every time, so a converged `k4` can need work again. The intent is persisted too.
    #[cfg(test)]
    async fn make_svid_needy(&mut self) {
        self.persist_orders_intent().await;
        let alloc = AllocationId::new(&format!("alloc-{ORDERS}-{}", self.next_aux_alloc))
            .expect("alloc id");
        self.next_aux_alloc += 1;
        self.write_orders_row(&alloc, AllocState::Running).await;
    }

    /// `k2`: the workload, the `orders` listener's VIP and listener fact, and a backend row whose
    /// set the programmed map does not carry.
    #[cfg(test)]
    async fn make_service_backends_needy(&mut self) {
        self.persist_orders_intent().await;
        self.backend_generation += 1;
        let vip = self.ensure_orders_listener().await;
        let backends = vec![Backend {
            alloc: SpiffeId::from_str(&format!(
                "spiffe://overdrive.local/workload/{ORDERS}/alloc/a{}",
                self.backend_generation
            ))
            .expect("spiffe"),
            addr: SocketAddr::new(Ipv4Addr::new(10, 9, self.backend_generation, 1).into(), 8080),
            weight: 100,
            healthy: true,
        }];
        let prior = self
            .obs
            .service_backends_rows(&self.service_id)
            .await
            .expect("service rows")
            .into_iter()
            .next();
        let tick_floor = self.next_counter();
        self.obs
            .write(ObservationWrite::ServiceBackend(ServiceBackendRow {
                service_id: self.service_id,
                vip: vip.try_as_ipv4().expect("allocator issues IPv4"),
                backends,
                updated_at: LogicalTimestamp::dominating(
                    tick_floor,
                    node("writer-1"),
                    prior.as_ref().map(|row| &row.updated_at),
                ),
            }))
            .await
            .expect("write service_backends");
    }

    #[cfg(test)]
    async fn diverge(&mut self, key: Key) {
        match key {
            Key::K1 => self.make_workload_needy().await,
            Key::K3 => self.make_service_needy().await,
            Key::K4 => self.make_svid_needy().await,
            Key::K2 => self.make_service_backends_needy().await,
        }
    }

    // ------------------------------------------------------------ actions

    async fn pass_step(&mut self, enum_ok: bool) -> std::result::Result<(), DriverError> {
        let name = PASS_ORDER[self.pass_pos];
        self.pass_pos = (self.pass_pos + 1) % PASS_ORDER.len();
        if !enum_ok {
            self.arm_read_fault(name, true);
        }
        let result = resync_reconciler(&self.state, &reconciler(name)).await;
        self.arm_read_fault(name, false);
        match (enum_ok, result) {
            (true, Err(error)) => Err(DriverError::Pass(error.to_string())),
            // An enumeration error is logged by the pass and submits nothing; the
            // comparison checks that nothing was submitted.
            (true, Ok(_)) | (false, _) => Ok(()),
        }
    }

    fn drain(&mut self) {
        let blocked: BTreeSet<TargetResource> =
            self.active.iter().map(|(_, target)| target.clone()).collect();
        let limit = CAP.saturating_sub(self.active.len());
        let now = self.now();
        let now_unix = UnixInstant::from_clock(&*self.clock);
        let admitted = self.state.runtime.broker().drain_pending(limit, &blocked, now, now_unix);
        for (evaluation, _queued_for) in admitted {
            self.active.insert((evaluation.reconciler, evaluation.target));
        }
    }

    async fn finish(&mut self, key: Key, outcome: Outcome) -> std::result::Result<(), DriverError> {
        let name = reconciler(key.reconciler_name());
        let target_resource = self.target_of(key);
        if !self.active.remove(&(name.clone(), target_resource.clone())) {
            return Err(DriverError::NotEnabled {
                action: "Finish",
                why: format!("{} was not admitted by a Drain", key.model_name()),
            });
        }
        match outcome {
            Outcome::Ok => {}
            Outcome::DpFail => self.dataplane.script_update_service_failure(
                DataplaneError::LoadFailed("the dataplane refused the update".to_owned()),
            ),
            Outcome::HydrateErr => self.arm_read_fault(key.reconciler_name(), true),
            Outcome::PersistErr => self.view_store.set_fsync_fails(true),
            Outcome::DispatchErr => {
                let error =
                    ObservationStoreError::Unreachable { peer: "sim-dispatch-fault".to_owned() };
                // `service-lifecycle` writes a service row only with a VIP (the model's `vip`); without
                // one its write is the allocation-lifecycle row (the Stable commit), so the fault is
                // armed at that seam. `service-map-hydrator`'s dispatch writes no row, so its only
                // failure is the scripted dataplane failure (`DpFail`); the model no longer offers
                // `dispatchErr` for `k2`.
                // `svid-lifecycle`'s `IssueSvid` dispatch audits through the generic
                // `ObservationStore::write` (`issue_and_audit`, `action_shim/issue_svid.rs`), which
                // `inject_write_failure` consumes: the issue fails and the SVID stays unheld.
                let writes_generic_row = key == Key::K4 || (key == Key::K3 && self.vip_assigned);
                if writes_generic_row {
                    self.obs.inject_write_failure(error);
                } else {
                    self.obs.inject_alloc_lifecycle_write_failure(error);
                }
            }
        }
        self.tick_n += 1;
        let now = self.now();
        let result = run_convergence_tick(
            &self.state,
            &name,
            &target_resource,
            now,
            self.tick_n,
            now + Duration::from_millis(100),
        )
        .await;
        // Disarm whatever the tick did not consume, so a fault never leaks into a later step.
        self.arm_read_fault(key.reconciler_name(), false);
        self.view_store.set_fsync_fails(false);
        self.obs.clear_write_failures();
        match (outcome, result) {
            (Outcome::Ok | Outcome::DpFail, Err(error)) => {
                Err(DriverError::Tick(error.to_string()))
            }
            // The error outcomes surface as the tick's error: the model records the
            // outcome by what the broker holds afterwards.
            _ => Ok(()),
        }
    }

    async fn crash(&mut self) -> std::result::Result<(), DriverError> {
        settle().await;
        let mut rebuilt = ReconcilerRuntime::new(
            self.tmp.path(),
            Arc::clone(&self.view_store) as Arc<dyn ViewStore>,
        )
        .map_err(|error| DriverError::Rebuild(error.to_string()))?;
        register_reconcilers(&mut rebuilt).await;
        // Only the runtime (its broker) is rebuilt: `state.identity` (the held SVIDs), the stores
        // and the world survive, as the model's `Crash` keeps `held`.
        self.state.runtime = Arc::new(rebuilt);
        self.active.clear();
        self.pass_pos = 0;
        Ok(())
    }
}

// ---------------------------------------------------------------- the driver

struct EvaluationWakeDriver {
    runtime: Runtime,
    world: WakeWorld,
}

impl EvaluationWakeDriver {
    fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let world = runtime.block_on(WakeWorld::compose());
        Self { runtime, world }
    }

    fn init(&mut self) -> Result {
        self.world = self.runtime.block_on(WakeWorld::compose());
        Ok(())
    }

    fn tick(&mut self) -> Result {
        self.runtime.block_on(settle());
        Ok(())
    }

    fn pass_step(&mut self, enum_ok: bool) -> Result {
        self.runtime.block_on(self.world.pass_step(enum_ok))?;
        Ok(())
    }

    fn drain(&mut self) -> Result {
        self.world.drain();
        Ok(())
    }

    fn finish(&mut self, key: &str, outcome: &str) -> Result {
        let (key, outcome) = (Key::parse(key)?, Outcome::parse(outcome)?);
        self.runtime.block_on(self.world.finish(key, outcome))?;
        Ok(())
    }

    fn diverge_wake(&mut self, key: &str) -> Result {
        let key = Key::parse(key)?;
        self.runtime.block_on(self.world.diverge(key));
        self.world.submit(key);
        Ok(())
    }

    fn diverge_drop(&mut self, key: &str) -> Result {
        let key = Key::parse(key)?;
        self.runtime.block_on(self.world.diverge(key));
        Ok(())
    }

    fn edge_wake(&mut self, key: &str) -> Result {
        self.world.submit(Key::parse(key)?);
        Ok(())
    }

    fn window_ends(&mut self, key: &str) -> Result {
        Key::parse(key)?;
        self.world.clock.tick(RESTART_BACKOFF_DURATION);
        Ok(())
    }

    fn crash(&mut self) -> Result {
        self.runtime.block_on(self.world.crash())?;
        Ok(())
    }
}

impl Driver for EvaluationWakeDriver {
    type State = WakeState;

    fn config() -> Config {
        Config { state: &[], nondet: &["wake_conf::evaluation_wake::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            init => self.init()?,
            Tick => self.tick()?,
            PassStep(enumOk: bool) => self.pass_step(enumOk)?,
            Drain => self.drain()?,
            Finish(k: String, o: String) => self.finish(&k, &o)?,
            DivergeWake(k: String) => self.diverge_wake(&k)?,
            DivergeDrop(k: String) => self.diverge_drop(&k)?,
            EdgeWake(k: String) => self.edge_wake(&k)?,
            WindowEnds(k: String) => self.window_ends(&k)?,
            Crash => self.crash()?,
        })
    }
}

// ---------------------------------------------------------------- the compared state

/// The compared variables: `pending` (keys in admission order) and the broker's
/// cumulative `dispatched` / `cancelled` counters.
#[derive(Debug, PartialEq, Eq)]
struct WakeState {
    pending: Vec<String>,
    dispatched: u64,
    cancelled: u64,
}

impl State<EvaluationWakeDriver> for WakeState {
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &EvaluationWakeDriver) -> Result<Self> {
        let broker = driver.world.state.runtime.broker();
        let pending = broker
            .pending_keys()
            .iter()
            .map(|(name, target)| driver.world.key_name(name, target))
            .collect();
        let counters = broker.counters();
        Ok(Self { pending, dispatched: counters.dispatched, cancelled: counters.cancelled })
    }
}

impl WakeState {
    fn from_value(state: &Value) -> std::result::Result<Self, ItfError> {
        let vars = Qualified::of(state, MAIN, MODULE)?;
        let counter = |name: &str| -> std::result::Result<u64, ItfError> {
            let value = quint_itf::int(vars.var(name)?)?;
            u64::try_from(value).map_err(|_| ItfError::Shape {
                expected: "a non-negative counter",
                found: vars.var(name).cloned().unwrap_or(Value::Number(value)),
            })
        };
        let pending = quint_itf::list_elems(vars.var("pending")?)?
            .into_iter()
            .map(quint_itf::string)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Self { pending, dispatched: counter("dispatched")?, cancelled: counter("cancelled")? })
    }
}

impl<'de> Deserialize<'de> for WakeState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------- named runs
// One `#[quint_test]` per named run of `wake_conf` (README "Named runs";
// checks.toml), then randomized traces. CONTRACT_SHAPE: bounded-change.

/// Happy path: an edge wake alone converges the key; the confirming requeue is a Noop.
#[quint_test(
    spec = "../../specs/quint/reconciler-evaluation-wake/evaluation_wake.qnt",
    main = "wake_conf",
    test = "healthy_edge_converges",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_evaluation_wake_healthy_edge_converges() -> impl Driver {
    EvaluationWakeDriver::new()
}

/// A hydrate error ends the evaluation with nothing scheduled; the pass is the retry.
#[quint_test(
    spec = "../../specs/quint/reconciler-evaluation-wake/evaluation_wake.qnt",
    main = "wake_conf",
    test = "hydrate_error_recovers_by_pass",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_evaluation_wake_hydrate_error_recovers_by_pass() -> impl Driver {
    EvaluationWakeDriver::new()
}

/// A View persist error, same recovery.
#[quint_test(
    spec = "../../specs/quint/reconciler-evaluation-wake/evaluation_wake.qnt",
    main = "wake_conf",
    test = "persist_error_recovers_by_pass",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_evaluation_wake_persist_error_recovers_by_pass() -> impl Driver {
    EvaluationWakeDriver::new()
}

/// Crash between View fsync and dispatch: the broker is lost, the boot pass retries.
#[quint_test(
    spec = "../../specs/quint/reconciler-evaluation-wake/evaluation_wake.qnt",
    main = "wake_conf",
    test = "crash_between_fsync_and_dispatch_recovers_by_boot_pass",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_evaluation_wake_crash_between_fsync_and_dispatch_recovers_by_boot_pass() -> impl Driver {
    EvaluationWakeDriver::new()
}

/// D-3: a failed dataplane update inside the backoff window retries without a row write or a pass.
#[quint_test(
    spec = "../../specs/quint/reconciler-evaluation-wake/evaluation_wake.qnt",
    main = "wake_conf",
    test = "failed_dataplane_inside_backoff_recovers_without_row_write",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_evaluation_wake_failed_dataplane_inside_backoff_recovers_without_row_write() -> impl Driver
{
    EvaluationWakeDriver::new()
}

/// Fresh deploy: intent exists, no row, no edge wake; the pass evaluates it.
#[quint_test(
    spec = "../../specs/quint/reconciler-evaluation-wake/evaluation_wake.qnt",
    main = "wake_conf",
    test = "fresh_deploy_without_row_is_evaluated",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_evaluation_wake_fresh_deploy_without_row_is_evaluated() -> impl Driver {
    EvaluationWakeDriver::new()
}

/// An enumeration failure of one reconciler submits nothing of its set and starves no other.
#[quint_test(
    spec = "../../specs/quint/reconciler-evaluation-wake/evaluation_wake.qnt",
    main = "wake_conf",
    test = "enumeration_failure_of_one_reconciler_does_not_starve_others",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_evaluation_wake_enumeration_failure_of_one_reconciler_does_not_starve_others()
-> impl Driver {
    EvaluationWakeDriver::new()
}

/// An edge wake submitted during a pass burst is admitted after it; per-target exclusion holds.
#[quint_test(
    spec = "../../specs/quint/reconciler-evaluation-wake/evaluation_wake.qnt",
    main = "wake_conf",
    test = "edge_wake_during_burst_is_admitted_after_burst",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_evaluation_wake_edge_wake_during_burst_is_admitted_after_burst() -> impl Driver {
    EvaluationWakeDriver::new()
}

/// A pass submit at a key holding a retained NotBefore merges it to Immediate.
#[quint_test(
    spec = "../../specs/quint/reconciler-evaluation-wake/evaluation_wake.qnt",
    main = "wake_conf",
    test = "pass_submit_merges_retained_not_before_to_immediate",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_evaluation_wake_pass_submit_merges_retained_not_before_to_immediate() -> impl Driver {
    EvaluationWakeDriver::new()
}

/// The world changes between a key's Drain and its Finish; Finish acts on the world read then.
#[quint_test(
    spec = "../../specs/quint/reconciler-evaluation-wake/evaluation_wake.qnt",
    main = "wake_conf",
    test = "world_changes_between_drain_and_finish",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_evaluation_wake_world_changes_between_drain_and_finish() -> impl Driver {
    EvaluationWakeDriver::new()
}

/// Randomized `wake_conf` traces against the production owner (seed printed on failure).
#[quint_run(
    spec = "../../specs/quint/reconciler-evaluation-wake/evaluation_wake.qnt",
    main = "wake_conf"
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_evaluation_wake_conformance_wake_conf() -> impl Driver {
    EvaluationWakeDriver::new()
}
