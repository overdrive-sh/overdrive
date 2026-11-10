//! Quint conformance driver — `restart_authority` (feature
//! service-allocation-readiness-and-ending: D5, D6, D7, D8, B2, B3 and the D14
//! producer rule as the workload flag `gl`).
//!
//! Replays `specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt`,
//! conformance main `ra_conf` (module `restart_authority`, recording `conf_step`),
//! against the production owners the protocol spans, composed by the reconciler
//! runtime: `ServiceLifecycle` and `WorkloadLifecycle` driven through
//! their own driving ports (`hydrate_*`, pure `reconcile`, view write-through,
//! shim `dispatch` of the returned actions, awaited), the production exit observer for the guest's own exit,
//! `SimDriver`, `SimObservationStore` and `SimClock` at the driven ports.
//!
//! Placement (testing.md "Placement and lane"): the owner path spans both
//! reconcilers' `reconcile`, the shim `StopAllocation` dispatch with a scripted
//! `SimDriver::stop` failure and an injected row-write failure, and the exit
//! observer. Only `overdrive-control-plane` composes those (`overdrive-reconcilers`
//! has no sim dev-dependency), so the driver sits beside the probe-count driver.
//!
//! Action -> production call (every handler calls a driving port):
//!
//! | Action | Driver |
//! |---|---|
//! | `init` | no world yet (the model starts undeployed) |
//! | `Deploy(decl, gl, ok)` | persist the Service intent (declared startup probe or the opt-out), script the start outcome (`ok = false`: `SimDriver::script_next_start_rejection`, a one-shot `DriverError::StartRejected`), then one `workload-lifecycle` tick (`StartAllocation`, awaited). `gl = true` declares the sole startup descriptor `ProbeMechanic::GuestListeners` (D14 item 1; the producer rule and gate threshold are DELIVER behaviour) |
//! | `SLAnnounceTerminal(i)` | a `service-lifecycle` tick (the owner's `reconcile`: DG-12 admits every `Failed` / `Terminated` fact to `terminal_announced`) |
//! | `Crash(i)` / `CleanExit(i)` | `SimDriver::inject_exit_after(Crashed / CleanExit)`; the production exit observer writes the row |
//! | `OperatorStop(i)` | the operator stop intent key (`put_if_absent`, as `stop_workload`), then a `workload-lifecycle` tick |
//! | `StartupRow(i, res)` | a startup `ProbeResultRow` (probe 0, next `run_seq`, the producer-counted streak) written through `SimObservationStore`: an input |
//! | `Deadline(i)` | `SimClock::tick` past the startup deadline |
//! | `SLStable(i)` / `SLStartupStop(i, stage)` / `SLLivenessStop(i, stage)` | a whole-workload `service-lifecycle` tick: DG-12 also acknowledges every terminal predecessor; stage `pre_hook` scripts `SimDriver::stop` to fail, `row_write` injects one terminal-row write failure |
//! | `WLRestart(ok)` / `WLGiveUp` | the start outcome scripted as for `Deploy` (`WLRestart` only), then a `workload-lifecycle` tick after the restart backoff |
//! | `SLFinalizeCrash` / `SLFinalizeStartup` | hazard-only actions: the driver bails with the action name |
//! | `Idle` | the runtime is driven to quiescence; the comparison checks nothing changed |
//!
//! Projection (`State::from_driver`), per the pinned table, per attempt `i`
//! (attempt = the allocation ids of the workload in creation order):
//! `row` <- `ObservationStore::alloc_status_row` (`state`, `reason`, `terminal`),
//! the model has no `pending` row (DG-2: the start outcome is applied atomically); `acted` <-
//! `ServiceLifecycleView.startup_last_acted_run[(alloc, 0)]` and `ann` <-
//! `ServiceLifecycleView.terminal_announced` through
//! `ReconcilerRuntime::loaded_service_lifecycle_views_for_test`; `stable` <- the
//! committed `Stable` terminal on the row, compared only while the row runs;
//! `claim` <- the terminal of the `FinalizeFailed` a `reconcile` returned;
//! `author` <- which reconciler's `reconcile` returned it (the driver calls the
//! two separately: hydrate -> reconcile -> view write-through -> shim dispatch of
//! the returned actions); `cur` <- `overdrive_reconcilers::current_alloc`; `used` <-
//! `WorkloadLifecycleView.restart_counts[cur]` through
//! `loaded_workload_lifecycle_views_for_test`; `done` <- a give-up `FinalizeFailed` returned by `WorkloadLifecycle::reconcile`.
//! Inputs (`sup`, `srow`, `sseq`, `sstreak`, `dl`, `stopped`, `vm`, `crashed`,
//! `deployed`, `g`, `conf_step`) are model-only.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
    non_snake_case,
    dead_code,
    clippy::significant_drop_tightening,
    clippy::needless_pass_by_value,
    clippy::unused_self,
    clippy::unnecessary_wraps,
    reason = "a conformance driver asserts its own preconditions; protocol names are prose"
)]

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use itf::Value;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, Service, VmInput, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::id::{AllocationId, NodeId, ServiceVip, WorkloadId};
use overdrive_core::observation::{
    ProbeIdx, ProbeResultRow, ProbeRole, ProbeRunSeq, ProbeStatus, SameStatusRuns,
};
use overdrive_core::reconcilers::{Action, ReconcilerName, TargetResource, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver as DriverPort, DriverType, ExitKind};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, ObservationStore, ObservationStoreError,
};
use overdrive_core::transition_reason::{
    ServiceFailureReason, StoppedBy, TerminalCondition, TransitionReason,
};
use overdrive_reconcilers::AnyReconcilerView;
use overdrive_reconcilers::current_alloc;
use overdrive_reconcilers::workload_lifecycle::RESTART_BACKOFF_CEILING;
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
use tokio_util::sync::CancellationToken;

use super::{ReconcilerRuntime, build_hydration_context};
use crate::AppState;
use crate::action_shim::dispatch_with_workflow_intent;
use crate::guest_network::quint_itf::{self, ItfError, Qualified};
use crate::view_store::ViewStore;

const MAIN: &str = "ra_conf";
const MODULE: &str = "restart_authority";
const SPEC: &str =
    "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt";

/// The model's startup `max_attempts`.
const MAXA: u32 = 2;
/// The startup probe's interval: `startup_deadline = interval x MAXA` = 120 s.
const STARTUP_INTERVAL_SECS: u32 = 60;
const STARTUP_DEADLINE: Duration = Duration::from_secs(120);
/// Past the startup deadline.
const PAST_DEADLINE: Duration = Duration::from_secs(121);
/// Past `RESTART_BACKOFF_DURATION` (1 s).
const PAST_BACKOFF: Duration = Duration::from_secs(2);
const WORKLOAD: &str = "svc";
const LISTENER_PORT: u16 = 8080;
/// Attempts 0..=CEIL of the conformance instance (ids 0..5).
const MODEL_ATTEMPTS: i64 = 6;

/// The `CEIL` the spec's `ra_conf` instance is imported with, read from the spec text.
fn spec_ceiling() -> u32 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(SPEC);
    let text = std::fs::read_to_string(&path).expect("read the restart_authority spec");
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with("module ra_conf"))
        .expect("the spec declares module ra_conf");
    let tail = line.split("CEIL = ").nth(1).expect("ra_conf sets CEIL");
    tail.split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|n| n.parse().ok())
        .expect("CEIL is an integer")
}

#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("`{0}` is a hazard-only action: it never occurs in the conformance instance")]
    HazardOnly(&'static str),
    #[error("model attempt {0} has no allocation yet")]
    NoAttempt(i64),
    #[error("a convergence tick failed: {0}")]
    Tick(String),
    #[error("the exit observer did not write the row of {0} within the settle bound")]
    Unsettled(String),
}

// ---------------------------------------------------------------- the world

struct RaWorld {
    state: AppState,
    /// The observation port the owners write to; the projection and the inputs go through it.
    obs: Arc<dyn ObservationStore>,
    /// The same store's fault-injection handle (a failing terminal-row write). Never projected.
    write_faults: Arc<SimObservationStore>,
    clock: Arc<SimClock>,
    driver: Arc<SimDriver>,
    workload: WorkloadId,
    target: TargetResource,
    service_lifecycle: ReconcilerName,
    workload_lifecycle: ReconcilerName,
    tick_n: u64,
    /// Per attempt allocation: startup rows written (run_seq), the consecutive
    /// failure count and the consecutive pass count.
    startup: BTreeMap<AllocationId, (u64, u32, u32)>,
    live_seq: BTreeMap<AllocationId, u64>,
    /// Every `FinalizeFailed` a reconciler's `reconcile` RETURNED, in order:
    /// the returning reconciler (the driver calls the two separately), the
    /// allocation and the action's terminal. The pinned `claim`, `author` and
    /// `done` project from these returned actions, not from a row diff.
    returned_finalize: Vec<(&'static str, AllocationId, Option<TerminalCondition>)>,
    observer_stop: CancellationToken,
    /// The redb-backed view store the runtime persists through; a control-plane
    /// restart rebuilds the runtime over it (the redb file stays open, as one
    /// process holds one handle to it).
    view_store: Arc<dyn ViewStore>,
    tmp: TempDir,
}

fn tcp(role: ProbeRole, port: u16, interval: u32, max_attempts: u32) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role,
        mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_owned(), port },
        timeout_seconds: 1,
        interval_seconds: interval,
        max_attempts,
        failure_threshold: (role == ProbeRole::Liveness).then_some(1),
        success_threshold: None,
        inferred: false,
    }
}

fn service_intent(decl: bool, gl: bool) -> WorkloadIntent {
    let startup = if gl {
        // D14 item 1: the platform's default startup gate is the sole startup descriptor.
        vec![ProbeDescriptor {
            mechanic: ProbeMechanic::GuestListeners,
            ..tcp(ProbeRole::Startup, 8081, STARTUP_INTERVAL_SECS, MAXA)
        }]
    } else if decl {
        vec![tcp(ProbeRole::Startup, 8081, STARTUP_INTERVAL_SECS, MAXA)]
    } else {
        vec![]
    };
    let svc = Service::from_submit(ServiceSpecInput {
        id: WORKLOAD.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Vm(VmInput {
            command: "/bin/serve".to_owned(),
            args: vec![],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
        listeners: vec![ListenerInput { port: LISTENER_PORT, protocol: "tcp".to_owned() }],
        startup_probes: startup,
        readiness_probes: vec![],
        liveness_probes: vec![tcp(ProbeRole::Liveness, 8083, 1, 30)],
    })
    .expect("valid service spec");
    WorkloadIntent::Service(svc)
}

async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

impl RaWorld {
    async fn compose() -> Self {
        assert_eq!(
            spec_ceiling(),
            RESTART_BACKOFF_CEILING,
            "the spec's ra_conf CEIL must equal the production restart ceiling"
        );
        let tmp = TempDir::new().expect("tempdir");
        let view_store: Arc<dyn ViewStore> = Arc::new(
            crate::view_store::redb::RedbViewStore::open(tmp.path()).expect("open the view store"),
        );
        let mut runtime = ReconcilerRuntime::new(tmp.path(), Arc::clone(&view_store))
            .expect("reconciler runtime");
        runtime.register(crate::service_lifecycle()).await.expect("register service-lifecycle");
        runtime.register(crate::workload_lifecycle()).await.expect("register workload-lifecycle");

        let node = NodeId::new("local").expect("node id");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
        let workload = WorkloadId::new(WORKLOAD).expect("workload id");
        let obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));
        let clock = Arc::new(SimClock::new());
        let driver = Arc::new(SimDriver::with_clock(DriverType::Vm, clock.clone()));
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
            Arc::new(SimDataplane::new()),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(0)))),
            Arc::new(crate::identity_mgr::IdentityMgr::new(None)),
            node,
            allocator,
            crate::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            mtls_worker,
            guest_owner,
            wiring.gate(),
            crate::shared_network_test_ports::guest_pool(),
        );
        // The production exit observer for the driver (one per registry driver).
        let observer_stop = CancellationToken::new();
        let _observer = crate::worker::exit_observer::spawn_with_runtime(
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::clone(&driver) as Arc<dyn DriverPort>,
            state.lifecycle_events.clone(),
            Arc::clone(&clock) as Arc<dyn Clock>,
            Some(Arc::clone(&state.runtime)),
            observer_stop.clone(),
        );

        Self {
            state,
            obs: Arc::clone(&obs) as Arc<dyn ObservationStore>,
            write_faults: obs,
            clock,
            driver,
            workload,
            target: TargetResource::new(&format!("workload/{WORKLOAD}")).expect("target"),
            service_lifecycle: ReconcilerName::new("service-lifecycle").expect("name"),
            workload_lifecycle: ReconcilerName::new("workload-lifecycle").expect("name"),
            tick_n: 0,
            startup: BTreeMap::new(),
            live_seq: BTreeMap::new(),
            returned_finalize: Vec::new(),
            observer_stop,
            view_store,
            tmp,
        }
    }

    /// `Deploy`: persists the Service intent (declared startup probe, the opt-out, or the
    /// guest-listener gate) and its VIP, as `overdrive deploy` leaves them for the
    /// reconcilers.
    async fn deploy(&self, decl: bool, gl: bool) {
        let intent = service_intent(decl, gl);
        let archived = intent.archive_for_store().expect("rkyv archive");
        self.state
            .store
            .put(IntentKey::for_workload(&self.workload).as_bytes(), archived.as_ref())
            .await
            .expect("put intent");
        self.state
            .store
            .put(
                IntentKey::for_workload_kind(&self.workload).as_bytes(),
                &[WorkloadKind::Service.discriminator_byte()],
            )
            .await
            .expect("put kind");
        let digest = intent.spec_digest().expect("spec digest");
        let mut guard = self.state.allocator.lock().await;
        let _vip: ServiceVip = guard.allocate(*digest.as_bytes()).await.expect("allocate vip");
    }

    /// A control-plane restart: a fresh `ReconcilerRuntime` is built and registers both
    /// reconcilers, which is the production boot path (`register` probes the store and
    /// `bulk_load`s every persisted view). The in-memory views are rebuilt from the redb
    /// ViewStore alone, then replace the runtime the projection reads. The exit observer
    /// does not use the runtime, so it keeps running.
    async fn restart_control_plane(&mut self) -> std::result::Result<(), DriverError> {
        settle().await;
        let mut rebuilt = ReconcilerRuntime::new(self.tmp.path(), Arc::clone(&self.view_store))
            .map_err(|error| DriverError::Tick(error.to_string()))?;
        rebuilt
            .register(crate::service_lifecycle())
            .await
            .map_err(|error| DriverError::Tick(error.to_string()))?;
        rebuilt
            .register(crate::workload_lifecycle())
            .await
            .map_err(|error| DriverError::Tick(error.to_string()))?;
        self.state.runtime = Arc::new(rebuilt);
        Ok(())
    }

    /// The workload's allocations in creation order (the trailing `-<n>` of the id).
    async fn attempts(&self) -> Vec<AllocStatusRow> {
        let mut rows: Vec<(u64, AllocStatusRow)> = self
            .obs
            .alloc_status_rows()
            .await
            .expect("alloc rows")
            .into_iter()
            .filter(|row| row.workload_id == self.workload)
            .filter_map(|row| {
                let n = row.alloc_id.as_str().rsplit_once('-')?.1.parse::<u64>().ok()?;
                Some((n, row))
            })
            .collect();
        rows.sort_by_key(|(n, _)| *n);
        rows.into_iter().map(|(_, row)| row).collect()
    }

    async fn alloc_of(&self, i: i64) -> std::result::Result<AllocationId, DriverError> {
        let index = usize::try_from(i).map_err(|_| DriverError::NoAttempt(i))?;
        self.attempts()
            .await
            .get(index)
            .map(|row| row.alloc_id.clone())
            .ok_or(DriverError::NoAttempt(i))
    }

    /// One convergence tick of `name` after `advance`, driven through the
    /// reconciler's own driving ports: `hydrate_desired` / `hydrate_actual`,
    /// then the pure `reconcile`, the view write-through, and the shim
    /// `dispatch` of the RETURNED actions. A shim driver error is tolerated
    /// when `tolerate_stop_failure` (the scripted `pre_hook` stage).
    async fn tick(
        &mut self,
        name: &ReconcilerName,
        author: &'static str,
        advance: Duration,
        tolerate_stop_failure: bool,
    ) -> std::result::Result<(), DriverError> {
        if !advance.is_zero() {
            self.clock.tick(advance);
        }
        self.tick_n += 1;
        let now: Instant = self.clock.now();
        let tick = TickContext {
            now,
            now_unix: UnixInstant::from_clock(&*self.state.clock),
            tick: self.tick_n,
            deadline: now + Duration::from_secs(1),
        };
        let actions = {
            let reconciler = self
                .state
                .runtime
                .get(name)
                .ok_or_else(|| DriverError::Tick(format!("{name} is not registered")))?;
            let (desired, actual) = {
                let allocator = self.state.allocator.lock().await;
                let listener_facts = self.state.listener_facts.lock().await;
                let ctx = build_hydration_context(&self.state, &allocator, &listener_facts);
                let desired = reconciler
                    .hydrate_desired(&ctx, &self.target)
                    .await
                    .map_err(|error| DriverError::Tick(error.to_string()))?;
                let actual = reconciler
                    .hydrate_actual(&ctx, &self.target)
                    .await
                    .map_err(|error| DriverError::Tick(error.to_string()))?;
                (desired, actual)
            };
            let view =
                self.state.runtime.get_view(name, &self.target).unwrap_or(AnyReconcilerView::Unit);
            let (actions, next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);
            self.state
                .runtime
                .persist_view(name, &self.target, next_view)
                .await
                .map_err(|error| DriverError::Tick(error.to_string()))?;
            actions
        };
        for action in &actions {
            if let Action::FinalizeFailed { alloc_id, terminal } = action {
                self.returned_finalize.push((author, alloc_id.clone(), terminal.clone()));
            }
        }
        let result = dispatch_with_workflow_intent(actions, &self.state, &tick).await;
        settle().await;
        match result {
            // a scripted stop failure or an injected row-write failure surfaces as the dispatch error
            Err(_) if tolerate_stop_failure => Ok(()),
            Err(error) => Err(DriverError::Tick(error.to_string())),
            Ok(()) => Ok(()),
        }
    }

    async fn wl_tick(&mut self, advance: Duration) -> std::result::Result<(), DriverError> {
        let name = self.workload_lifecycle.clone();
        self.tick(&name, "workload_lifecycle", advance, false).await
    }

    /// A `service-lifecycle` tick whose stop dispatch ends at `stage`.
    async fn sl_tick(&mut self, stage: Option<&str>) -> std::result::Result<(), DriverError> {
        match stage {
            Some("pre_hook") => {
                self.driver.script_stop_failure(Some("scripted Driver::stop failure"));
            }
            Some("row_write") => {
                self.write_faults.inject_write_failure(ObservationStoreError::Io(
                    std::io::Error::other("injected terminal row write failure"),
                ));
            }
            _ => {}
        }
        let name = self.service_lifecycle.clone();
        let result = self.tick(&name, "service_lifecycle", Duration::ZERO, stage.is_some()).await;
        self.driver.script_stop_failure(None);
        // an armed one-shot write fault must not leak into a later write
        self.write_faults.clear_write_failures();
        result
    }

    /// The model's start outcome: `ok = false` scripts the driver to reject the next start.
    fn script_start_outcome(&self, ok: bool) {
        self.driver.script_next_start_rejection((!ok).then_some("scripted start rejection"));
    }

    async fn write_startup_row(
        &mut self,
        alloc: &AllocationId,
        pass: bool,
    ) -> std::result::Result<(), DriverError> {
        let entry = self.startup.entry(alloc.clone()).or_insert((0, 0, 0));
        entry.0 += 1;
        if pass {
            entry.1 = 0;
            entry.2 += 1;
        } else {
            entry.2 = 0;
            entry.1 += 1;
        }
        let (seq, fails, passes) = *entry;
        let status = if pass {
            ProbeStatus::Pass
        } else {
            ProbeStatus::Fail { last_fail_reason: "connection refused".to_owned() }
        };
        let row = ProbeResultRow {
            alloc_id: alloc.clone(),
            probe_idx: ProbeIdx::new(0),
            role: ProbeRole::Startup,
            status,
            last_observed_at_unix_ms: u64::try_from(self.clock.unix_now().as_millis())
                .unwrap_or(u64::MAX),
            inferred: false,
            run_seq: ProbeRunSeq::new(seq).expect("non-zero run_seq"),
            same_status_runs: SameStatusRuns::new(if pass { passes } else { fails }.max(1))
                .expect("non-zero streak"),
        };
        self.obs.write_probe_result(row).await.map_err(|error| DriverError::Tick(error.to_string()))
    }

    async fn write_liveness_fail(
        &mut self,
        alloc: &AllocationId,
    ) -> std::result::Result<(), DriverError> {
        let seq = self.live_seq.entry(alloc.clone()).or_insert(0);
        *seq += 1;
        let row = ProbeResultRow {
            alloc_id: alloc.clone(),
            probe_idx: ProbeIdx::new(0),
            role: ProbeRole::Liveness,
            status: ProbeStatus::Fail { last_fail_reason: "connection refused".to_owned() },
            last_observed_at_unix_ms: u64::try_from(self.clock.unix_now().as_millis())
                .unwrap_or(u64::MAX),
            inferred: false,
            run_seq: ProbeRunSeq::new(*seq).expect("non-zero run_seq"),
            same_status_runs: SameStatusRuns::new(1).expect("non-zero streak"),
        };
        self.obs.write_probe_result(row).await.map_err(|error| DriverError::Tick(error.to_string()))
    }

    /// The guest ends on its own; the production exit observer writes the row.
    async fn guest_exits(
        &self,
        alloc: &AllocationId,
        kind: ExitKind,
    ) -> std::result::Result<(), DriverError> {
        self.driver.inject_exit_after(alloc, Duration::ZERO, kind);
        for _ in 0..2000 {
            settle().await;
            let row = self.obs.alloc_status_row(alloc).await.expect("alloc row");
            if row.is_some_and(|r| matches!(r.state, AllocState::Failed | AllocState::Terminated)) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        Err(DriverError::Unsettled(alloc.to_string()))
    }
}

impl Drop for RaWorld {
    fn drop(&mut self) {
        self.observer_stop.cancel();
    }
}

// ---------------------------------------------------------------- the driver

/// Runs `$body` (an async block over `$w`) on the driver's runtime against its world.
macro_rules! on_world {
    ($self:ident, $w:ident => $body:expr) => {{
        let rt = &$self.runtime;
        let $w = &mut $self.world;
        rt.block_on(async { $body })?
    }};
}

struct RestartAuthorityDriver {
    runtime: Runtime,
    /// The composition under test, held directly so the projection reads the production
    /// owners through it. A trace starts from `init`, which composes a fresh one.
    world: RaWorld,
}

impl RestartAuthorityDriver {
    fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let world = runtime.block_on(RaWorld::compose());
        Self { runtime, world }
    }

    fn init(&mut self) {
        // The previous trace's world (and its exit observer) is dropped by the assignment.
        self.world = self.runtime.block_on(RaWorld::compose());
        self.runtime.block_on(settle());
    }

    fn deploy(&mut self, decl: bool, gl: bool, ok: bool) -> Result {
        self.runtime.block_on(self.world.deploy(decl, gl));
        self.world.script_start_outcome(ok);
        self.runtime.block_on(self.world.wl_tick(Duration::ZERO))?;
        Ok(())
    }

    fn crash(&mut self, i: i64) -> Result {
        on_world!(self, w => {
            let alloc = w.alloc_of(i).await?;
            w.guest_exits(&alloc, ExitKind::Crashed { exit_code: Some(1), signal: None }).await
        });
        Ok(())
    }

    fn clean_exit(&mut self, i: i64) -> Result {
        on_world!(self, w => {
            let alloc = w.alloc_of(i).await?;
            w.guest_exits(&alloc, ExitKind::CleanExit).await
        });
        Ok(())
    }

    fn operator_stop(&mut self, _i: i64) -> Result {
        on_world!(self, w => {
            w.state
                .store
                .put_if_absent(IntentKey::for_workload_stop(&w.workload).as_bytes(), b"")
                .await
                .map_err(|error| DriverError::Tick(error.to_string()))?;
            w.wl_tick(Duration::ZERO).await
        });
        Ok(())
    }

    fn startup_row(&mut self, i: i64, res: &str) -> Result {
        let pass = res == "pass";
        on_world!(self, w => {
            let alloc = w.alloc_of(i).await?;
            w.write_startup_row(&alloc, pass).await
        });
        Ok(())
    }

    fn deadline(&mut self, _i: i64) -> Result {
        on_world!(self, w => {
            w.clock.tick(PAST_DEADLINE);
            settle().await;
            Ok::<(), DriverError>(())
        });
        Ok(())
    }

    fn sl_tick(&mut self, _i: i64, stage: Option<String>) -> Result {
        on_world!(self, w => w.sl_tick(stage.as_deref()).await);
        Ok(())
    }

    fn sl_liveness_stop(&mut self, i: i64, stage: String) -> Result {
        on_world!(self, w => {
            let alloc = w.alloc_of(i).await?;
            w.write_liveness_fail(&alloc).await?;
            w.sl_tick(Some(stage.as_str()).filter(|s| *s != "landed")).await
        });
        Ok(())
    }

    fn wl_after_backoff(&mut self, ok: Option<bool>) -> Result {
        if let Some(ok) = ok {
            self.world.script_start_outcome(ok);
        }
        on_world!(self, w => w.wl_tick(PAST_BACKOFF).await);
        Ok(())
    }

    fn hazard(&self, action: &'static str, _i: i64) -> Result {
        Err(DriverError::HazardOnly(action).into())
    }

    /// `Idle`: the control plane restarts (the model's reload is a no-op on its state),
    /// so the views the next comparison reads are the ones `bulk_load` rebuilt.
    fn idle(&mut self) -> Result {
        self.runtime.block_on(self.world.restart_control_plane())?;
        Ok(())
    }
}

impl Driver for RestartAuthorityDriver {
    type State = RaState;

    fn config() -> Config {
        Config { state: &[], nondet: &["ra_conf::restart_authority::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            init => self.init(),
            Deploy(decl: bool, gl: bool, ok: bool) => self.deploy(decl, gl, ok)?,
            Crash(i: i64) => self.crash(i)?,
            CleanExit(i: i64) => self.clean_exit(i)?,
            OperatorStop(i: i64) => self.operator_stop(i)?,
            StartupRow(i: i64, res: String) => self.startup_row(i, &res)?,
            Deadline(i: i64) => self.deadline(i)?,
            SLStable(i: i64) => self.sl_tick(i, None)?,
            SLStartupStop(i: i64, stage: String) => self.sl_tick(i, Some(stage).filter(|s| s != "landed"))?,
            SLLivenessStop(i: i64, stage: String) => self.sl_liveness_stop(i, stage)?,
            SLFinalizeCrash(i: i64) => self.hazard("SLFinalizeCrash", i)?,
            SLFinalizeStartup(i: i64) => self.hazard("SLFinalizeStartup", i)?,
            SLAnnounceTerminal(i: i64) => self.sl_tick(i, None)?,
            WLRestart(ok: bool) => self.wl_after_backoff(Some(ok))?,
            WLGiveUp => self.wl_after_backoff(None)?,
            Idle => self.idle()?,
        })
    }
}

// ---------------------------------------------------------------- the compared state

/// The compared fields of one model attempt (`sup`, `srow`, `sseq`, `sstreak`,
/// `dl`, `stopped`, `vm` and `crashed` are inputs or ghosts, never projected).
#[derive(Debug, Clone, PartialEq, Eq)]
struct AttView {
    row: String,
    acted: i64,
    ann: bool,
    stable: bool,
    claim: String,
    author: String,
}

#[derive(Debug, PartialEq, Eq)]
struct RaState {
    at: BTreeMap<i64, AttView>,
    cur: i64,
    used: i64,
    done: bool,
}

fn claim_of(terminal: Option<&TerminalCondition>) -> &'static str {
    match terminal {
        Some(TerminalCondition::ServiceFailed { reason }) => match reason {
            ServiceFailureReason::EarlyExit { .. } => "early_exit",
            ServiceFailureReason::StartupProbeFailed { .. } => "startup_probe_failed",
            ServiceFailureReason::LivenessProbeFailed { .. } => "liveness_probe_failed",
            _ => "",
        },
        Some(TerminalCondition::BackoffExhausted { .. }) => "backoff_exhausted",
        _ => "",
    }
}

/// The model's row class of a production row. After `WLGiveUp`, the
/// `FinalizeFailed` shim writes `Failed`; a prior startup stop therefore
/// projects as `failed_other`, while `claim` retains `StartupProbeFailed`.
fn row_class(row: Option<&AllocStatusRow>) -> &'static str {
    let Some(row) = row else { return "none" };
    let claim = claim_of(row.terminal.as_ref());
    match row.state {
        AllocState::Pending | AllocState::Running | AllocState::Draining => "running",
        AllocState::Failed => match &row.reason {
            Some(
                TransitionReason::WorkloadCrashedImmediately { .. }
                | TransitionReason::VmOutOfMemory { .. },
            ) => "failed_crash",
            _ => "failed_other",
        },
        AllocState::Terminated => match (&row.terminal, &row.reason) {
            (Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }), _) => "t_startup",
            (Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }), _) => "t_liveness",
            (
                Some(TerminalCondition::Stopped { by: StoppedBy::Operator | StoppedBy::SystemGc }),
                _,
            ) => "t_operator",
            (Some(TerminalCondition::Stopped { by: StoppedBy::Process }), _) => "t_clean",
            _ if claim == "startup_probe_failed" => "t_startup",
            _ if claim == "liveness_probe_failed" => "t_liveness",
            (_, Some(TransitionReason::Stopped { by: StoppedBy::Process })) => "t_clean",
            _ => "t_operator",
        },
        AllocState::Suspended => "failed_other",
    }
}

impl RaState {
    fn from_value(state: &Value) -> std::result::Result<Self, ItfError> {
        let vars = Qualified::of(state, MAIN, MODULE)?;
        let at = quint_itf::int_map(vars.var("at")?, |entry| {
            let row = quint_itf::string(quint_itf::field(entry, "row")?)?;
            let live = row == "running";
            Ok(AttView {
                row,
                acted: quint_itf::int(quint_itf::field(entry, "acted")?)?,
                ann: quint_itf::boolean(quint_itf::field(entry, "ann")?)?,
                stable: live && quint_itf::boolean(quint_itf::field(entry, "stable")?)?,
                claim: quint_itf::string(quint_itf::field(entry, "claim")?)?,
                author: quint_itf::string(quint_itf::field(entry, "author")?)?,
            })
        })?;
        Ok(Self {
            at,
            cur: quint_itf::int(vars.var("cur")?)?,
            used: quint_itf::int(vars.var("used")?)?,
            done: quint_itf::boolean(vars.var("done")?)?,
        })
    }
}

impl<'de> Deserialize<'de> for RaState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

impl State<RestartAuthorityDriver> for RaState {
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &RestartAuthorityDriver) -> Result<Self> {
        let world = &driver.world;
        let rows = driver.runtime.block_on(driver.world.attempts());
        let sl_views = driver
            .world
            .state
            .runtime
            .loaded_service_lifecycle_views_for_test(&driver.world.service_lifecycle)
            .and_then(|views| views.get(&driver.world.target).cloned())
            .unwrap_or_default();
        let wl_views = driver
            .world
            .state
            .runtime
            .loaded_workload_lifecycle_views_for_test(&driver.world.workload_lifecycle)
            .and_then(|views| views.get(&driver.world.target).cloned())
            .unwrap_or_default();

        let refs: Vec<&AllocStatusRow> = rows.iter().collect();
        let current = current_alloc(&refs);
        let cur = current
            .and_then(|row| rows.iter().position(|r| r.alloc_id == row.alloc_id))
            .map_or(0, |p| i64::try_from(p).expect("attempt index fits i64"));
        let used = current
            .and_then(|row| wl_views.restart_counts.get(&row.alloc_id).copied())
            .map_or(0, i64::from);

        let mut at = BTreeMap::new();
        // `done`: a give-up `FinalizeFailed` returned by `WorkloadLifecycle::reconcile`.
        let done = world.returned_finalize.iter().any(|(by, _, terminal)| {
            *by == "workload_lifecycle" && !claim_of(terminal.as_ref()).is_empty()
        });
        for i in 0..MODEL_ATTEMPTS {
            let row = usize::try_from(i).ok().and_then(|p| rows.get(p));
            // `claim` / `author`: the first give-up `FinalizeFailed` a reconciler's
            // `reconcile` returned for this attempt (pinned projection).
            let returned = row.and_then(|r| {
                world.returned_finalize.iter().find(|(_, alloc, terminal)| {
                    *alloc == r.alloc_id && !claim_of(terminal.as_ref()).is_empty()
                })
            });
            let claim = returned.map_or("", |(_, _, terminal)| claim_of(terminal.as_ref()));
            let author = returned.map_or("", |(by, _, _)| *by);
            let class = row_class(row);
            let alloc = row.map(|r| r.alloc_id.clone());
            let acted = alloc
                .as_ref()
                .and_then(|a| sl_views.startup_last_acted_run.get(&(a.clone(), ProbeIdx::new(0))))
                .map_or(0, |seq| i64::try_from(seq.get()).expect("run_seq fits i64"));
            let ann = alloc.as_ref().is_some_and(|a| sl_views.terminal_announced.contains(a));
            let stable = class == "running"
                && row
                    .is_some_and(|r| matches!(r.terminal, Some(TerminalCondition::Stable { .. })));
            at.insert(
                i,
                AttView {
                    row: class.to_owned(),
                    acted,
                    ann,
                    stable,
                    claim: claim.to_owned(),
                    author: author.to_owned(),
                },
            );
        }
        Ok(Self { at, cur, used, done })
    }
}

// ---------------------------------------------------------------- named runs
// One `#[quint_test]` per named run of `restart_authority` on `ra_conf` (README
// "Named runs"; checks.toml), then randomized traces. CONTRACT_SHAPE: bounded-change.

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "crashed_and_rejected_allocs_join_terminal_announced",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn restart_authority_crashed_and_rejected_allocs_join_terminal_announced() -> impl Driver {
    RestartAuthorityDriver::new()
}

/// Regression witness at HEAD, green because no owner changed behaviour: the run never
/// takes a `ServiceLifecycle` tick after the crash and the deadline, so D5's guard (no
/// startup verdict for a non-running allocation) is NOT exercised here. The runs that
/// tick `ServiceLifecycle` past the deadline
/// (`running_alloc_with_exhausted_probes_past_deadline_is_stopped`,
/// `startup_never_passes_loop_gives_up_startup_probe_failed`) prove D5.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "crash_after_attempts_exhausted_then_deadline_passes",
    max_samples = 1
)]
fn restart_authority_crash_after_attempts_exhausted_then_deadline_passes() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "running_alloc_with_exhausted_probes_past_deadline_is_stopped",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn restart_authority_running_alloc_with_exhausted_probes_past_deadline_is_stopped() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "pre_startup_crash_loop_gives_up_early_exit",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn restart_authority_pre_startup_crash_loop_gives_up_early_exit() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "pre_startup_crash_then_successor_passes",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn restart_authority_pre_startup_crash_then_successor_passes() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "crash_after_startup_pass_gives_up_backoff_exhausted",
    max_samples = 1
)]
fn restart_authority_crash_after_startup_pass_gives_up_backoff_exhausted() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "opt_out_service_crash_loop_gives_up_backoff_exhausted",
    max_samples = 1
)]
fn restart_authority_opt_out_service_crash_loop_gives_up_backoff_exhausted() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "non_crash_failed_row_gives_up_backoff_exhausted",
    max_samples = 1
)]
fn restart_authority_non_crash_failed_row_gives_up_backoff_exhausted() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "liveness_kill_loop_gives_up_liveness_probe_failed",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D6/D7 give-up classification (WorkloadLifecycle alone authors the ending)"]
fn restart_authority_liveness_kill_loop_gives_up_liveness_probe_failed() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "startup_never_passes_loop_gives_up_startup_probe_failed",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn restart_authority_startup_never_passes_loop_gives_up_startup_probe_failed() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "pass_then_fails_before_stable_is_stopped",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn restart_authority_pass_then_fails_before_stable_is_stopped() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "startup_stop_then_successor_passes_is_stable",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn restart_authority_startup_stop_then_successor_passes_is_stable() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "startup_stop_then_crash_names_last_cause",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn restart_authority_startup_stop_then_crash_names_last_cause() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "crash_then_startup_stop_names_last_cause",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn restart_authority_crash_then_startup_stop_names_last_cause() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "startup_stop_is_not_intentional_stop",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn restart_authority_startup_stop_is_not_intentional_stop() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "first_fail_at_deadline_with_no_earlier_row_stops",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D14 item 3 startup gate threshold 1 for a GuestListeners descriptor; D7 startup stop"]
fn restart_authority_first_fail_at_deadline_with_no_earlier_row_stops() -> impl Driver {
    RestartAuthorityDriver::new()
}

/// Regression witness at HEAD: `streak`, `startupStopEnabled` and the gate state are
/// model-only inputs here (startup rows are written through the observation port) and no
/// owner observes the gate, so this proves only that the driver replays the run. It does
/// NOT prove that an explicit probe keeps the counted streak; the D14 item 3 run
/// `first_fail_at_deadline_with_no_earlier_row_stops` and the S-53 rows tests
/// (`guest_listener_probe_rows.rs`) do.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "explicit_probe_fail_after_deadline_keeps_counted_streak",
    max_samples = 1
)]
fn restart_authority_explicit_probe_fail_after_deadline_keeps_counted_streak() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "failed_startup_stop_is_retried_on_next_row",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D7 startup stop (StopAllocation { Stopped { StartupProbe } }), D5 guard, D8 startup_last_acted_run"]
fn restart_authority_failed_startup_stop_is_retried_on_next_row() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "late_pass_after_failed_stop_reaches_stable",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D7 startup stop (StopAllocation { Stopped { StartupProbe } }), D5 guard, D8 startup_last_acted_run"]
fn restart_authority_late_pass_after_failed_stop_reaches_stable() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "stop_row_write_failure_is_retried",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D7 startup stop (StopAllocation { Stopped { StartupProbe } }), D5 guard, D8 startup_last_acted_run"]
fn restart_authority_stop_row_write_failure_is_retried() -> impl Driver {
    RestartAuthorityDriver::new()
}

#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf",
    test = "control_plane_restart_keeps_acted_run",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D7 startup stop (StopAllocation { Stopped { StartupProbe } }), D5 guard, D8 startup_last_acted_run"]
fn restart_authority_control_plane_restart_keeps_acted_run() -> impl Driver {
    RestartAuthorityDriver::new()
}

/// Randomized `ra_conf` traces against the production owners (seed printed on failure).
#[quint_run(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt",
    main = "ra_conf"
)]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D5-D8, D14, DG-12 restart authority (the random traces reach the unimplemented D6/D7 endings and the DG-12 announcement)"]
fn restart_authority_conformance_ra_conf() -> impl Driver {
    RestartAuthorityDriver::new()
}
