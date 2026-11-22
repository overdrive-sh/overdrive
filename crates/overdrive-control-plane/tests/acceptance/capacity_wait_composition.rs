//! The rig for a workload waiting for guest-network capacity, composed end to
//! end (node-local persistent volume, GH #348; ADR-0336 the view, ADR-0338 the
//! admission handling; feature delta § "Operator surface: a workload waiting
//! for capacity"; DISTILL ruling W-3 = A).
//!
//! This file holds the composed node only; its tests are
//! `tests/acceptance/alloc_status_capacity_wait.rs` (the cheap handler guards)
//! and, because they fill the real attachment pool to its cap of 16,384,
//! `tests/integration/{alloc_status_capacity_wait,capacity_wait_composition}.rs`,
//! which include this file through `#[path]` (`super::serve_ports` resolves
//! in both binaries). It has no test of its own and no placement declaration.
//!
//! Composition. Owner path (all production): `handlers::{submit_workload,
//! stop_workload, restart_workload, alloc_status}` over `AppState`, the real
//! `ReconcilerRuntime` with the registered `workload-lifecycle` reconciler and
//! its `ViewStore`, the real action shim
//! (`run_convergence_tick_with_guest_network_provisioner_for_test`, the
//! accepted seam that substitutes only the guest-network driven port), the real
//! `GuestAddressPool`. Driven ports substituted: `SimDriver`,
//! `SimObservationStore`, `SimCa`, `SimClock`, `SimDataplane`, the
//! guest-network owner (`SimSharedGuestNetworkOwner` behind a ledger that
//! counts leases at the provisioner port). No row, lease or View entry is
//! seeded. The CLI render of the response is not driven here: this crate does
//! not depend on `overdrive-cli`; the `Waiting` line is pinned by
//! `overdrive-cli/tests/{acceptance,integration}/workload_describe*.rs`.
//!
//! How the node is filled. The pool's lease operations are crate-private, so
//! the test fills it the way production does: other targets' admissions. Each
//! filler is one `Action::StartAllocation` dispatched through the production
//! action shim, which takes its lease from the server's own pool
//! (`GuestAddressPool::assign`, the pool's own admission, refusing at the cap).
//! The fillers belong to a workload with no intent and are never evaluated by
//! this test, so nothing garbage-collects them. The resync pass is realised as
//! the per-target `workload-lifecycle` evaluation it submits: the pass
//! (`run_resync_pass`) is crate-private and would also evaluate the fillers.
//! A full fill is about 56 s on a debug build.
//!
//! Evidence tier: Tier 1, in-process, deterministic (no randomness; every time
//! source is the `SimClock`).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::future_not_send,
    dead_code,
    reason = "a test rig fails at once on a broken precondition, naming it; helpers are shared with sibling files"
)]

use overdrive_core::byte_size::ByteSize;
use overdrive_core::cpu_quantity::CpuQuantity;
use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use overdrive_control_plane::action_shim::dispatch_with_guest_network_provisioner_for_test;
use overdrive_control_plane::api::{
    AllocStatusResponse, CapacityResource, CapacityWait, SubmitWorkloadRequest,
};
use overdrive_control_plane::guest_network::{
    ForwardingActivation, GuestAddressPool, GuestNetworkPlan, GuestNetworkProvisioner,
    Result as GuestNetworkResult, SharedGuestNetworkOwner,
};
use overdrive_control_plane::handlers::{
    AllocStatusQuery, alloc_status, restart_workload, stop_workload, submit_workload,
};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, run_convergence_tick_with_guest_network_provisioner_for_test,
};
use overdrive_control_plane::worker::exit_observer;
use overdrive_control_plane::{AppState, workload_lifecycle};
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::api::submit::SubmitSpecInput;
use overdrive_core::guest_network::{GuestNetworkExecWiring, MAX_GUEST_NETWORK_ATTACHMENTS};
use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, ReconcilerName, TargetResource, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationSpec, DriverPayload, DriverType, Resources, VmPayload,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use parking_lot::Mutex;
use serde_json::json;
use tempfile::TempDir;

/// The node's attachment cap as a count.
pub const CAP: usize = MAX_GUEST_NETWORK_ATTACHMENTS as usize;
/// The one node every fixture places on.
const NODE: &str = "local";
/// One tick of the injected clock.
const STEP: Duration = Duration::from_millis(100);
/// Fillers dispatched per shim call.
const FILL_BATCH: usize = 256;
/// Upper bound on the evaluations any one wait may take.
pub const MAX_EVALUATIONS: usize = 40;
/// The resync period of the convergence loop (feature delta § Over-report).
const RESYNC_PERIOD: Duration = Duration::from_secs(30);

/// The one owner instance's lease ledger at the guest-network driven port: a
/// lease is acquired at `provision` (the shim takes the pool lease and
/// immediately calls it) and released when a `teardown` succeeds (the shim
/// releases the pool lease after it). The pool's own counts are crate-private.
pub struct Ledger {
    inner: Arc<SimSharedGuestNetworkOwner>,
    held: Mutex<BTreeSet<AllocationId>>,
}

impl Ledger {
    pub fn held_count(&self) -> usize {
        self.held.lock().len()
    }

    pub fn holds(&self, alloc: &AllocationId) -> bool {
        self.held.lock().contains(alloc)
    }
}

#[async_trait::async_trait]
impl GuestNetworkProvisioner for Ledger {
    async fn provision(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        self.held.lock().insert(plan.alloc().clone());
        self.inner.provision(plan).await
    }

    async fn activate(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<ForwardingActivation> {
        self.inner.activate(plan).await
    }

    async fn teardown(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        let outcome = self.inner.teardown(plan).await;
        if outcome.is_ok() {
            self.held.lock().remove(plan.alloc());
        }
        outcome
    }
}

/// One node: the production composition over sim driven ports.
pub struct Node {
    _tmp: TempDir,
    pub state: AppState,
    pub driver: Arc<SimDriver>,
    pub clock: Arc<SimClock>,
    pub ledger: Arc<Ledger>,
    reconciler: ReconcilerName,
    tick: AtomicU64,
    fillers: AtomicU64,
    _supervisor: Arc<overdrive_core::guest_network::GuestNetworkExecSupervisor>,
    _pool: Arc<GuestAddressPool>,
}

impl Node {
    /// Compose one node. Panics, naming the precondition, when the harness
    /// itself cannot be built (not a contract verdict).
    pub async fn compose() -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let mut runtime =
            ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
        runtime.register(workload_lifecycle()).await.expect("register workload-lifecycle");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("intent store"));
        let node_id = NodeId::new(NODE).expect("node id");
        let obs: Arc<dyn ObservationStore> =
            Arc::new(SimObservationStore::single_peer(node_id.clone(), 0));
        let clock = Arc::new(SimClock::new());
        let driver = Arc::new(SimDriver::with_clock(DriverType::Vm, clock.clone()));
        let allocator =
            overdrive_control_plane::test_default_allocator(store.clone() as Arc<dyn IntentStore>);

        let worker = super::serve_ports::worker();
        worker.start_shared_owner().await.expect("the worker's shared owner starts");
        let owner = Arc::new(SimSharedGuestNetworkOwner::default());
        owner.converge_shared().await.expect("the shared guest-network owner converges");
        let exec = GuestNetworkExecWiring::new(clock.clone());
        let supervisor = exec.supervisor();
        let pool = super::serve_ports::pool();

        let state = AppState::new(
            store,
            store_path,
            obs,
            Arc::new(runtime),
            driver.clone(),
            clock.clone(),
            Arc::new(SimDataplane::new()),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(0)))),
            Arc::new(IdentityMgr::new(None)),
            node_id,
            allocator,
            overdrive_control_plane::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            worker,
            Arc::clone(&owner) as Arc<dyn SharedGuestNetworkOwner>,
            exec.gate(),
            Arc::clone(&pool),
        );
        assert!(supervisor.open_after_boot(), "the EXEC gate opens after owner convergence");
        exit_observer::spawn(
            state.obs.clone(),
            driver.clone(),
            state.lifecycle_events.clone(),
            clock.clone(),
        );
        Self {
            _tmp: tmp,
            state,
            driver,
            clock,
            ledger: Arc::new(Ledger { inner: owner, held: Mutex::new(BTreeSet::new()) }),
            reconciler: ReconcilerName::new("workload-lifecycle").expect("reconciler name"),
            tick: AtomicU64::new(0),
            fillers: AtomicU64::new(0),
            _supervisor: supervisor,
            _pool: pool,
        }
    }

    // -- operator inputs: the real handlers ---------------------------------

    fn vm_spec_json(id: &str) -> serde_json::Value {
        json!({
            "id": id,
            "replicas": 1,
            "resources": {"cpu": "0", "memory": "1MiB"},
            "vm": {"command": "/sbin/init", "args": [], "kernel": "/kernel", "rootfs": "/rootfs"},
        })
    }

    /// `overdrive deploy` of a one-replica VM Service, through the real handler.
    pub async fn deploy_service(&self, id: &str) {
        let mut spec = Self::vm_spec_json(id);
        spec["listeners"] = json!([{"port": 8080, "protocol": "tcp"}]);
        spec["startup_probes"] = json!([]);
        spec["readiness_probes"] = json!([]);
        spec["liveness_probes"] = json!([]);
        let spec: overdrive_core::api::submit::ServiceSpecInput =
            serde_json::from_value(spec).expect("a VM service wire spec");
        self.submit(SubmitSpecInput::Service(spec)).await;
    }

    /// `overdrive deploy` of a one-replica VM Job, through the real handler.
    pub async fn deploy_job(&self, id: &str) {
        let spec: overdrive_core::aggregate::JobSpecInput =
            serde_json::from_value(Self::vm_spec_json(id)).expect("a VM job wire spec");
        self.submit(SubmitSpecInput::Job(spec)).await;
    }

    async fn submit(&self, spec: SubmitSpecInput) {
        let response = submit_workload(
            State(self.state.clone()),
            HeaderMap::new(),
            Json(SubmitWorkloadRequest { spec }),
        )
        .await
        .expect("the handler admits a valid VM workload");
        assert!(
            response.status().is_success(),
            "the deploy must be accepted, got {}",
            response.status()
        );
    }

    /// `overdrive workload stop`, through the real handler.
    pub async fn stop(&self, id: &str) {
        let _ = stop_workload(State(self.state.clone()), Path(id.to_owned()))
            .await
            .expect("the handler records the stop");
    }

    /// `overdrive workload restart`, through the real handler.
    pub async fn restart(&self, id: &str) {
        let _ = restart_workload(State(self.state.clone()), Path(id.to_owned()))
            .await
            .expect("the handler records the restart");
    }

    // -- the node's other tenants: the pool's own admission ------------------

    /// Take `count` more attachments the way other targets do: one
    /// `StartAllocation` each through the production action shim, whose lease
    /// comes from the server's own pool. Panics when an admission is refused
    /// (the fill never exceeds the cap).
    pub async fn fill(&self, count: usize) {
        let before = self.ledger.held_count();
        let mut tasks = Vec::new();
        let mut remaining = count;
        while remaining > 0 {
            let batch = remaining.min(FILL_BATCH);
            let actions: Vec<Action> = (0..batch).map(|_| self.filler_start()).collect();
            let state = self.state.clone();
            let ledger = Arc::clone(&self.ledger);
            let clock = Arc::clone(&self.clock);
            let tick_n = self.tick.fetch_add(1, Ordering::SeqCst);
            tasks.push(tokio::spawn(async move {
                let now = clock.now();
                let tick = TickContext {
                    now,
                    now_unix: UnixInstant::from_clock(clock.as_ref()),
                    tick: tick_n,
                    deadline: now + Duration::from_secs(60),
                };
                Box::pin(dispatch_with_guest_network_provisioner_for_test(
                    actions,
                    &state,
                    &tick,
                    ledger.as_ref(),
                ))
                .await
            }));
            remaining -= batch;
        }
        for task in tasks {
            task.await
                .expect("the fill task runs")
                .expect("the fill's admissions succeed below the cap");
        }
        assert_eq!(
            self.ledger.held_count(),
            before + count,
            "each filler must take exactly one attachment"
        );
    }

    fn filler_start(&self) -> Action {
        let index = self.fillers.fetch_add(1, Ordering::SeqCst);
        let name = format!("filler-{index:05}");
        Action::StartAllocation {
            alloc_id: AllocationId::new(&name).expect("alloc id"),
            workload_id: WorkloadId::new("filler").expect("workload id"),
            node_id: NodeId::new(NODE).expect("node id"),
            spec: AllocationSpec {
                alloc: AllocationId::new(&name).expect("alloc id"),
                identity: SpiffeId::new("spiffe://overdrive.local/workload/filler")
                    .expect("spiffe id"),
                driver: DriverPayload::Vm(VmPayload {
                    volume: None,
                    command: "/sbin/init".to_owned(),
                    args: Vec::new(),
                    kernel: PathBuf::from("/kernel"),
                    rootfs: PathBuf::from("/rootfs"),
                }),
                resources: Resources {
                    cpu: CpuQuantity::from_millis(0),
                    memory: ByteSize::new(1024 * 1024),
                },
                probe_descriptors: Vec::new(),
                transport: None,
                declared_listeners: vec![overdrive_core::aggregate::Listener {
                    protocol: overdrive_core::dataplane::Proto::Tcp,
                    port: NonZeroU16::new(8080).expect("port"),
                }],
            },
            kind: WorkloadKind::Service,
        }
    }

    /// Another workload's cleanup releases one attachment: the production
    /// reclaim of filler number `index`.
    pub async fn free_filler(&self, index: usize) {
        let alloc = AllocationId::new(&format!("filler-{index:05}")).expect("alloc id");
        assert!(self.ledger.holds(&alloc), "the filler to free must hold an attachment");
        let now = self.clock.now();
        let tick = TickContext {
            now,
            now_unix: UnixInstant::from_clock(self.clock.as_ref()),
            tick: self.tick.fetch_add(1, Ordering::SeqCst),
            deadline: now + Duration::from_secs(60),
        };
        Box::pin(dispatch_with_guest_network_provisioner_for_test(
            vec![Action::ReclaimAllocationNetwork { alloc_id: alloc.clone() }],
            &self.state,
            &tick,
            self.ledger.as_ref(),
        ))
        .await
        .expect("the reclaim is dispatched");
        assert!(!self.ledger.holds(&alloc), "the reclaim must release the filler's attachment");
    }

    // -- evaluation: the production tick ------------------------------------

    /// One production `workload-lifecycle` evaluation of `workload`: hydrate,
    /// reconcile, persist the View, dispatch through the action shim. This is
    /// the evaluation the resync pass submits for each managed target.
    pub async fn evaluate(&self, workload: &str) -> Result<(), String> {
        let n = self.tick.fetch_add(1, Ordering::SeqCst);
        let now = self.clock.now();
        let target = TargetResource::new(&format!("workload/{workload}")).expect("target");
        Box::pin(run_convergence_tick_with_guest_network_provisioner_for_test(
            &self.state,
            &self.reconciler,
            &target,
            now,
            n,
            now + Duration::from_secs(60),
            self.ledger.as_ref(),
        ))
        .await
        .map_err(|error| format!("{error:?}"))
    }

    /// Advance the injected clock by one step and yield so the exit observer
    /// and the driver's tasks run.
    pub async fn settle(&self) {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        self.clock.tick(STEP);
    }

    /// Evaluate `workload` until `done` holds over the observation rows, at
    /// most [`MAX_EVALUATIONS`] times. Returns whether it held.
    pub async fn evaluate_until(
        &self,
        workload: &str,
        done: impl Fn(&[AllocStatusRow]) -> bool,
    ) -> bool {
        for _ in 0..MAX_EVALUATIONS {
            self.evaluate(workload).await.expect("the evaluation is dispatched");
            self.settle().await;
            if done(&self.rows_of(workload).await) {
                return true;
            }
        }
        false
    }

    /// Advance the injected clock one resync period, as the 30 s pass anchors
    /// it, and run the evaluation the pass submits for `workload`.
    pub async fn resync(&self, workload: &str) {
        self.clock.tick(RESYNC_PERIOD);
        self.evaluate(workload).await.expect("the resync evaluation is dispatched");
        self.settle().await;
    }

    // -- observation --------------------------------------------------------

    pub async fn rows_of(&self, workload: &str) -> Vec<AllocStatusRow> {
        let mut rows: Vec<AllocStatusRow> = self
            .state
            .obs
            .alloc_status_rows()
            .await
            .expect("alloc status rows")
            .into_iter()
            .filter(|row| row.workload_id.as_str() == workload)
            .collect();
        rows.sort_by(|a, b| a.alloc_id.cmp(&b.alloc_id));
        rows
    }

    pub async fn running_count(&self, workload: &str) -> usize {
        self.rows_of(workload).await.iter().filter(|r| r.state == AllocState::Running).count()
    }

    /// The `restart_counts` of the reconciler's View for `workload`.
    pub fn restart_counts(&self, workload: &str) -> std::collections::BTreeMap<AllocationId, u32> {
        let target = TargetResource::new(&format!("workload/{workload}")).expect("target");
        self.state.runtime.view_for_workload_lifecycle(&target).restart_counts
    }

    /// `GET /v1/allocs?job=<id>`, through the real handler.
    pub async fn describe(&self, workload: &str) -> AllocStatusResponse {
        alloc_status(
            State(self.state.clone()),
            Query(AllocStatusQuery { job: Some(workload.to_owned()) }),
        )
        .await
        .expect("alloc_status answers for a deployed workload")
        .0
    }
}

/// The wait a full node reports.
pub const fn full_node_wait() -> CapacityWait {
    CapacityWait {
        resource: CapacityResource::GuestNetworkAttachments,
        held: MAX_GUEST_NETWORK_ATTACHMENTS,
        cap: MAX_GUEST_NETWORK_ATTACHMENTS,
    }
}

/// No `Failed` row exists for `workload`.
pub async fn assert_no_failed_row(node: &Node, workload: &str, when: &str) {
    let failed: Vec<_> = node
        .rows_of(workload)
        .await
        .into_iter()
        .filter(|row| row.state == AllocState::Failed)
        .map(|row| row.alloc_id)
        .collect();
    assert!(failed.is_empty(), "{when}: a capacity wait writes no Failed row, got {failed:?}");
}
