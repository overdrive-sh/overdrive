//! V-3 probe (feature `reconciler-convergence-guarantees`): cost of the designed 30 s resync pass.
//!
//! REAL production code driven here: `ReconcilerRuntime` (+ its `EvaluationBroker`, `submit_evaluation`),
//! the seven registered reconcilers (their real `hydrate_*` / `reconcile`), `run_convergence_tick`, the
//! real deploy handler `handlers::submit_workload`, `LocalIntentStore` (redb), `LocalObservationStore`
//! (redb), the real `RedbViewStore`, the real `SystemClock`.
//!
//! Sim adapters (driven ports only, as the crate's own fixtures use them): `SimDriver` (VM start), `SimCa`
//! (SVID issuance), `SimDataplane`, `SimSharedGuestNetworkOwner`, the mTLS worker over `SimMtlsIntercept`.
//!
//! The ONE re-implementation: `spawn_loop` mirrors `lib.rs::spawn_convergence_loop` (private fn, so it
//! cannot be called). It keeps the same admission shape (cap 8, one in flight per target, FuturesUnordered
//! polled by one task, `drain_pending(capacity, active_targets, ..)`). Differences, all stated:
//!  - `ReconcilerRuntime::submitted()` (the submit wake) is `pub(crate)`, so an idle loop polls every
//!    200 microseconds instead of waking on `notify_one` (adds at most ~0.2 ms to an admission).
//!  - no cadence table / shutdown / drain handling (not relevant to the measurement).
//!  - each admission records `queued_for` (broker-reported) and the tick's elapsed time.

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use overdrive_control_plane::api::SubmitWorkloadRequest;
use overdrive_control_plane::guest_network::{GuestAddressPool, SharedGuestNetworkOwner};
use overdrive_control_plane::handlers::submit_workload;
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, hydrate_actual_for_test, hydrate_desired_for_test, run_convergence_tick,
};
use overdrive_control_plane::{
    AppState, InterestRouterBroker, arm_next_wake, build_cadence_table, build_interest_table,
    due_resync_evaluations, noop_heartbeat, spawn_interest_router, service_lifecycle, service_map_hydrator, svid_lifecycle,
    test_default_allocator, test_empty_listener_facts, vm_reclamation, workflow_lifecycle,
    workload_lifecycle,
};
use overdrive_core::aggregate::{
    DriverInput, JobSpecInput, ResourcesInput, VmInput, WorkloadIntent, scan_workload_intents,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::eval_broker::{Evaluation, EvaluationEligibility};
use overdrive_core::guest_network::{GuestNetworkExecGate, GuestNetworkExecWiring};
use overdrive_core::id::NodeId;
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::wall_clock::UnixInstant;
use overdrive_host::SystemClock;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::{SimIdentityRead, SimMtlsIntercept, SimMtlsResolve};
use overdrive_store_local::{LocalIntentStore, LocalObservationStore};
use overdrive_worker::mtls_intercept_port::MtlsIntercept;
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

const MAX_IN_FLIGHT: usize = 8; // lib.rs CONVERGENCE_MAX_IN_FLIGHT
const PERIOD: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------------------------
// small utilities
// ---------------------------------------------------------------------------------------------

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn cpu_secs() -> f64 {
    // SAFETY: getrusage with a valid out pointer.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    tv(ru.ru_utime) + tv(ru.ru_stime)
}

#[derive(Clone, Debug)]
struct Stats {
    n: usize,
    mean: f64,
    sd: f64,
    min: f64,
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
}

fn stats(xs: &[f64]) -> Stats {
    if xs.is_empty() {
        return Stats { n: 0, mean: 0.0, sd: 0.0, min: 0.0, p50: 0.0, p95: 0.0, p99: 0.0, max: 0.0 };
    }
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    let mean = v.iter().sum::<f64>() / n as f64;
    let var = if n > 1 { v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64 } else { 0.0 };
    let q = |p: f64| v[(((n - 1) as f64) * p).round() as usize];
    Stats { n, mean, sd: var.sqrt(), min: v[0], p50: q(0.5), p95: q(0.95), p99: q(0.99), max: v[n - 1] }
}

fn stats_json(s: &Stats) -> Value {
    json!({"n": s.n, "mean": s.mean, "sd": s.sd, "min": s.min, "p50": s.p50, "p95": s.p95, "p99": s.p99, "max": s.max})
}

fn rn(s: &str) -> ReconcilerName {
    ReconcilerName::new(s).expect("reconciler name")
}

fn tr(s: &str) -> TargetResource {
    TargetResource::new(s).expect("target")
}

// ---------------------------------------------------------------------------------------------
// fixture: the crate's own AppState::new composition (what tests/integration fixtures use)
// ---------------------------------------------------------------------------------------------

struct Fixture {
    state: AppState,
    _supervisor: Arc<overdrive_core::guest_network::GuestNetworkExecSupervisor>,
}

fn worker() -> Arc<MtlsInterceptWorker> {
    use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
    use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
    use overdrive_sim::adapters::clock::SimClock;
    use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
    let enforcement: Arc<dyn MtlsEnforcement> = Arc::new(SimMtlsEnforcement::new(
        Arc::new(SimIdentityRead::new(BTreeMap::new(), None)),
        MtlsLimits::default(),
        Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
    ));
    let resolve: Arc<dyn MtlsResolve> =
        Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh));
    let intercept: Arc<dyn MtlsIntercept> = Arc::new(SimMtlsIntercept::new());
    Arc::new(MtlsInterceptWorker::new(enforcement, resolve, Arc::new(SimClock::new()), intercept))
}

fn pool() -> Arc<GuestAddressPool> {
    Arc::new(GuestAddressPool::new(
        ipnet::Ipv4Net::new_assert(Ipv4Addr::new(100, 95, 0, 0), 16),
        Ipv4Addr::new(100, 95, 0, 1),
        Arc::new(SimEntropy::new(0x295)),
    ))
}

async fn build_fixture(root: &Path) -> Fixture {
    std::fs::create_dir_all(root).expect("mkdir root");
    let mut runtime = ReconcilerRuntime::new_with_redb_view_store_for_test(root).expect("runtime");
    // the registration set of run_server with the gateway disabled (lib.rs:3963-4054)
    runtime.register(noop_heartbeat()).await.expect("reg");
    runtime.register(workload_lifecycle()).await.expect("reg");
    runtime.register(workflow_lifecycle()).await.expect("reg");
    runtime.register(svid_lifecycle()).await.expect("reg");
    runtime.register(vm_reclamation()).await.expect("reg");
    runtime.register(service_map_hydrator(Ipv4Addr::LOCALHOST)).await.expect("reg");
    runtime.register(service_lifecycle()).await.expect("reg");

    let store_path = root.join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("intent store"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(LocalObservationStore::open(root.join("observation.redb")).expect("obs store"));
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::with_clock(DriverType::Vm, Arc::clone(&clock)));

    let worker = worker();
    worker.start_shared_owner().await.expect("shared mtls owner");
    let owner = Arc::new(SimSharedGuestNetworkOwner::default());
    owner.converge_shared().await.expect("owner converges");
    let wiring = GuestNetworkExecWiring::new(Arc::new(overdrive_sim::adapters::clock::SimClock::new()));
    let gate: Arc<GuestNetworkExecGate> = wiring.gate();
    let supervisor = wiring.supervisor();
    assert!(supervisor.open_after_boot());

    let allocator = test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    let state = AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        clock,
        Arc::new(SimDataplane::new()),
        Arc::new(SimCa::new(Arc::new(SimEntropy::new(0)))),
        Arc::new(IdentityMgr::new(None)),
        NodeId::new("local").expect("node id"),
        allocator,
        test_empty_listener_facts(),
        Ipv4Addr::LOCALHOST,
        worker,
        Arc::clone(&owner) as Arc<dyn SharedGuestNetworkOwner>,
        gate,
        pool(),
    );
    Fixture { state, _supervisor: supervisor }
}

// ---------------------------------------------------------------------------------------------
// deploy through the REAL handler
// ---------------------------------------------------------------------------------------------

fn vm() -> DriverInput {
    DriverInput::Vm(VmInput {
        command: "/bin/serve".to_owned(),
        args: vec![],
        kernel: "/kernel".to_owned(),
        rootfs: "/rootfs".to_owned(),
    })
}

fn job_spec(id: &str) -> SubmitSpecInput {
    SubmitSpecInput::Job(JobSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 50, memory_bytes: 64 * 1024 * 1024 },
        driver: vm(),
    })
}

fn service_spec(id: &str) -> SubmitSpecInput {
    SubmitSpecInput::Service(ServiceSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 50, memory_bytes: 64 * 1024 * 1024 },
        driver: vm(),
        listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    })
}

/// Returns the handler wall time. Panics on a handler error (the probe must be running real deploys).
async fn deploy(state: &AppState, spec: SubmitSpecInput) -> Duration {
    let t = Instant::now();
    let r = submit_workload(
        State(state.clone()),
        HeaderMap::new(),
        Json(SubmitWorkloadRequest { spec }),
    )
    .await;
    let d = t.elapsed();
    if let Err(e) = r {
        panic!("deploy handler failed: {e:?}");
    }
    d
}

// ---------------------------------------------------------------------------------------------
// the mirrored convergence loop
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Rec {
    reconciler: String,
    target: String,
    queued_ms: f64,
    elapsed_ms: f64,
    admitted_at_ms: f64, // since epoch0
    done_at_ms: f64,
    ok: bool,
}

struct LoopCtl {
    stop: AtomicBool,
    active: AtomicUsize,
    recs: std::sync::Mutex<Vec<Rec>>,
    drain_calls_ns: std::sync::Mutex<Vec<(u64, u64)>>, // (pending after drain, ns)
    epoch0: Instant,
}

fn spawn_loop(state: AppState, ctl: Arc<LoopCtl>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick_n: u64 = 0;
        let mut active_targets: BTreeSet<TargetResource> = BTreeSet::new();
        let mut active = FuturesUnordered::new();
        let cadence = Duration::from_millis(100);
        while !ctl.stop.load(Ordering::Relaxed) {
            let now = state.clock.now();
            let now_unix = UnixInstant::from_clock(&*state.clock);
            let capacity = MAX_IN_FLIGHT.saturating_sub(active.len());
            if capacity > 0 {
                let t = Instant::now();
                let (pending, left) = {
                    let mut broker = state.runtime.broker();
                    let p = broker.drain_pending(capacity, &active_targets, now, now_unix);
                    (p, broker.counters().queued)
                };
                let ns = t.elapsed().as_nanos() as u64;
                if !pending.is_empty() {
                    ctl.drain_calls_ns.lock().unwrap().push((left, ns));
                }
                for (eval, queued_for) in pending {
                    let tick = tick_n;
                    tick_n += 1;
                    let deadline = now + cadence;
                    active_targets.insert(eval.target.clone());
                    let st = state.clone();
                    let epoch0 = ctl.epoch0;
                    active.push(async move {
                        let started = Instant::now();
                        let r = run_convergence_tick(&st, &eval.reconciler, &eval.target, now, tick, deadline).await;
                        (eval, queued_for, started, epoch0, r.is_ok())
                    });
                }
                ctl.active.store(active.len(), Ordering::Relaxed);
            }
            if active.is_empty() {
                tokio::time::sleep(Duration::from_micros(200)).await;
            } else {
                tokio::select! {
                    Some((eval, queued_for, started, epoch0, ok)) = active.next() => {
                        active_targets.remove(&eval.target);
                        let done = Instant::now();
                        ctl.recs.lock().unwrap().push(Rec {
                            reconciler: eval.reconciler.to_string(),
                            target: eval.target.as_str().to_owned(),
                            queued_ms: ms(queued_for),
                            elapsed_ms: ms(done.duration_since(started)),
                            admitted_at_ms: ms(started.duration_since(epoch0)),
                            done_at_ms: ms(done.duration_since(epoch0)),
                            ok,
                        });
                        ctl.active.store(active.len(), Ordering::Relaxed);
                    }
                    () = tokio::time::sleep(Duration::from_micros(200)) => {}
                }
            }
        }
    })
}

async fn wait_idle(state: &AppState, ctl: &LoopCtl) {
    let mut quiet = 0;
    loop {
        tokio::time::sleep(Duration::from_millis(2)).await;
        let q = state.runtime.broker().counters().queued;
        if q == 0 && ctl.active.load(Ordering::Relaxed) == 0 {
            quiet += 1;
            if quiet >= 3 {
                return;
            }
        } else {
            quiet = 0;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// target enumeration: stand-in for the designed `managed_targets`, per the delta's D-1a table.
// (Reconcilers do not have `managed_targets` yet; this uses the same production reads the table names.)
// ---------------------------------------------------------------------------------------------

async fn enumerate(state: &AppState) -> BTreeMap<ReconcilerName, BTreeSet<TargetResource>> {
    let intents = scan_workload_intents(state.store.as_ref(), &state.intent_redb_path).await.expect("scan intents");
    let allocs = state.obs.alloc_status_rows().await.expect("alloc rows");
    let backends = state.obs.all_service_backends_rows().await.expect("backend rows");

    let mut out: BTreeMap<ReconcilerName, BTreeSet<TargetResource>> = BTreeMap::new();
    let mut ids: BTreeSet<String> = BTreeSet::new();
    let mut service_ids: BTreeSet<String> = BTreeSet::new();
    for (_k, intent) in &intents {
        match intent {
            WorkloadIntent::Job(j) => {
                ids.insert(j.id.to_string());
            }
            WorkloadIntent::Service(s) => {
                ids.insert(s.id.to_string());
                service_ids.insert(s.id.to_string());
            }
            WorkloadIntent::Schedule(_) => {}
        }
    }
    let mut alloc_workloads: BTreeSet<String> = BTreeSet::new();
    for r in &allocs {
        alloc_workloads.insert(r.workload_id.to_string());
    }
    // workload-lifecycle: intents ∪ alloc_status workload ids
    let wl: BTreeSet<TargetResource> =
        ids.union(&alloc_workloads).map(|i| tr(&format!("workload/{i}"))).collect();
    out.insert(rn("workload-lifecycle"), wl);
    // service-lifecycle: Service-kind workload ids
    out.insert(
        rn("service-lifecycle"),
        service_ids.iter().map(|i| tr(&format!("workload/{i}"))).collect(),
    );
    // svid-lifecycle: alloc_status workload ids (held-SVID ids are a subset in steady state)
    out.insert(
        rn("svid-lifecycle"),
        alloc_workloads.iter().map(|i| tr(&format!("workload/{i}"))).collect(),
    );
    // service-map-hydrator: service ids in service_backends rows
    out.insert(
        rn("service-map-hydrator"),
        backends.iter().map(|b| tr(&format!("service/{}", b.service_id))).collect(),
    );
    out.insert(rn("vm-reclamation"), BTreeSet::from([tr(&format!("node/{}", state.node_id))]));
    out.insert(rn("workflow-lifecycle"), BTreeSet::new());
    out.insert(rn("noop-heartbeat"), BTreeSet::new());
    out
}

fn submit_pass(state: &AppState, set: &BTreeMap<ReconcilerName, BTreeSet<TargetResource>>) -> BTreeSet<(String, String)> {
    let mut keys = BTreeSet::new();
    for (name, targets) in set {
        for t in targets {
            state.runtime.submit_evaluation(
                Evaluation { reconciler: name.clone(), target: t.clone() },
                Instant::now(),
                EvaluationEligibility::Immediate,
            );
            keys.insert((name.to_string(), t.as_str().to_owned()));
        }
    }
    keys
}

/// Run passes until a pass produces no evaluation beyond the ones it submitted (no action requeued),
/// twice in a row. Returns (rounds, evals_in_last_round).
async fn settle(state: &AppState, ctl: &LoopCtl) -> (usize, usize) {
    let mut clean = 0;
    for round in 1..=30 {
        ctl.recs.lock().unwrap().clear();
        let set = enumerate(state).await;
        let keys = submit_pass(state, &set);
        wait_idle(state, ctl).await;
        let recs = ctl.recs.lock().unwrap().len();
        if recs == keys.len() {
            clean += 1;
            if clean >= 2 {
                return (round, recs);
            }
        } else {
            clean = 0;
        }
    }
    (30, 0)
}

// ---------------------------------------------------------------------------------------------
// measurements
// ---------------------------------------------------------------------------------------------

async fn measure_per_eval(
    state: &AppState,
    set: &BTreeMap<ReconcilerName, BTreeSet<TargetResource>>,
) -> Value {
    // Serial, one evaluation at a time, uncontended. Warm-up 10, then 40 samples spread over targets.
    let mut out = BTreeMap::new();
    for (name, targets) in set {
        if targets.is_empty() {
            continue;
        }
        let reconciler = state.runtime.get(name).expect("registered");
        let list: Vec<&TargetResource> = targets.iter().collect();
        let pick = |i: usize| list[(i * 7919) % list.len()];
        for i in 0..10 {
            let t = pick(i);
            let _ = hydrate_desired_for_test(reconciler, t, state).await;
            let _ = hydrate_actual_for_test(reconciler, t, state).await;
        }
        let mut hyd = vec![];
        let mut hyd_d = vec![];
        let mut hyd_a = vec![];
        for i in 0..40 {
            let t = pick(100 + i);
            let a = Instant::now();
            hydrate_desired_for_test(reconciler, t, state).await.expect("hydrate desired");
            let b = Instant::now();
            hydrate_actual_for_test(reconciler, t, state).await.expect("hydrate actual");
            let c = Instant::now();
            hyd_d.push(ms(b - a));
            hyd_a.push(ms(c - b));
            hyd.push(ms(c - a));
        }
        // full tick (hydrate + reconcile + persist_view + dispatch + requeue decision)
        let mut tick = vec![];
        let cpu0 = cpu_secs();
        let w0 = Instant::now();
        for i in 0..10 {
            let t = pick(i);
            run_convergence_tick(state, name, t, Instant::now(), 0, Instant::now() + PERIOD).await.expect("tick");
        }
        for i in 0..40 {
            let t = pick(200 + i);
            let a = Instant::now();
            run_convergence_tick(state, name, t, Instant::now(), i as u64, Instant::now() + PERIOD)
                .await
                .expect("tick");
            tick.push(ms(a.elapsed()));
        }
        let cpu_per_tick_ms = (cpu_secs() - cpu0) * 1000.0 / 50.0;
        let _ = w0;
        // drain anything the ticks requeued (steady state: nothing)
        out.insert(
            name.to_string(),
            json!({
                "targets": targets.len(),
                "hydrate_desired_plus_actual_ms": stats_json(&stats(&hyd)),
                "hydrate_desired_ms": stats_json(&stats(&hyd_d)),
                "hydrate_actual_ms": stats_json(&stats(&hyd_a)),
                "full_tick_ms": stats_json(&stats(&tick)),
                "full_tick_cpu_ms_per_tick_incl_warmup": cpu_per_tick_ms,
            }),
        );
    }
    json!(out)
}

struct ProbeSample {
    at_ms: f64,
    wait_us: f64,
}

fn spawn_lock_probe(
    state: AppState,
    stop: Arc<AtomicBool>,
    epoch0: Instant,
    out: Arc<std::sync::Mutex<Vec<ProbeSample>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while !stop.load(Ordering::Relaxed) {
            let t = Instant::now();
            let g = state.allocator.lock().await;
            let w = t.elapsed();
            drop(g);
            out.lock().unwrap().push(ProbeSample {
                at_ms: ms(t.duration_since(epoch0)),
                wait_us: w.as_secs_f64() * 1e6,
            });
            tokio::time::sleep(Duration::from_micros(500)).await;
        }
    })
}

async fn edge_sample(
    state: &AppState,
    ctl: &Arc<LoopCtl>,
    id: &str,
    service: bool,
) -> (Duration, Option<Rec>) {
    let spec = if service { service_spec(id) } else { job_spec(id) };
    let handler = deploy(state, spec).await;
    // the handler's own submit_evaluation happened inside; the admission record appears when the tick completes
    let target = format!("workload/{id}");
    for _ in 0..100_000 {
        if let Some(r) = ctl
            .recs
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.reconciler == "workload-lifecycle" && r.target == target)
            .cloned()
        {
            return (handler, Some(r));
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    (handler, None)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Baseline,
    Proposed,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Baseline => "baseline",
            Mode::Proposed => "proposed",
        }
    }
}

/// One burst + one deploy-shaped edge wake 3 ms after the burst starts.
async fn one_sample(
    state: &AppState,
    ctl: &Arc<LoopCtl>,
    epoch0: Instant,
    n: usize,
    s: usize,
    mode: Mode,
    service_edge: bool,
    verbose: bool,
) -> (Value, String) {
    ctl.recs.lock().unwrap().clear();
    ctl.drain_calls_ns.lock().unwrap().clear();
    let probe_out: Arc<std::sync::Mutex<Vec<ProbeSample>>> = Arc::new(std::sync::Mutex::new(vec![]));
    let probe_stop = Arc::new(AtomicBool::new(false));
    let edge_id = format!("edge-{}-{s}-{}", mode.name(), if service_edge { "svc" } else { "job" });
    let ph;
    let cpu0;
    let t0;
    let t0_ms;
    let keys: BTreeSet<(String, String)>;
    let handler;
    let edge_rec;
    let mut list_to_first_submit_ms = 0.0_f64;
    let mut router_handle = None;
    let router_stop = CancellationToken::new();
    match mode {
        Mode::Proposed => {
            let set = enumerate(state).await;
            ph = spawn_lock_probe(state.clone(), Arc::clone(&probe_stop), epoch0, Arc::clone(&probe_out));
            tokio::time::sleep(Duration::from_millis(20)).await;
            cpu0 = cpu_secs();
            t0 = Instant::now();
            t0_ms = ms(t0.duration_since(epoch0));
            keys = submit_pass(state, &set);
            tokio::time::sleep(Duration::from_millis(3)).await;
            let r = edge_sample(state, ctl, &edge_id, service_edge).await;
            handler = r.0;
            edge_rec = r.1;
        }
        Mode::Baseline => {
            // Independent reproduction of the key set (checked against what actually completes):
            // every alloc_status row x every reconciler whose interests() contains AllocStatus, plus the resync_schedule targets.
            let table = build_interest_table(state.runtime.reconcilers_iter());
            let interested = table
                .get(&overdrive_core::traits::observation_store::ObservationRowKind::AllocStatus)
                .cloned()
                .unwrap_or_default();
            let rows = state.obs.alloc_status_rows().await.expect("alloc rows");
            let mut k = BTreeSet::new();
            for r in &rows {
                for name in &interested {
                    k.insert((name.to_string(), format!("workload/{}", r.workload_id)));
                }
            }
            let schedules = build_cadence_table(state.runtime.reconcilers_iter());
            let now_unix = UnixInstant::from_clock(&*state.clock);
            // arm one period in the past so every scheduled reconciler is due now (worst-case phase alignment with the relist)
            let mut next_wake: BTreeMap<_, _> = arm_next_wake(&schedules, now_unix)
                .into_iter()
                .map(|(nm, _)| (nm, now_unix))
                .collect();
            let cadence_evals = due_resync_evaluations(&schedules, &mut next_wake, now_unix, &state.node_id);
            for e in &cadence_evals {
                k.insert((e.reconciler.to_string(), e.target.as_str().to_owned()));
            }
            keys = k;
            let subscription = state.obs.subscribe_all_events().await.expect("subscribe");
            ph = spawn_lock_probe(state.clone(), Arc::clone(&probe_stop), epoch0, Arc::clone(&probe_out));
            tokio::time::sleep(Duration::from_millis(20)).await;
            cpu0 = cpu_secs();
            t0 = Instant::now();
            t0_ms = ms(t0.duration_since(epoch0));
            // REAL production path: the router's boot LIST is `list_and_route`; relist period 1 h so only that one list runs.
            router_handle = Some(spawn_interest_router(
                Arc::clone(&state.obs),
                subscription,
                table,
                InterestRouterBroker::from_runtime(Arc::clone(&state.runtime), Arc::clone(&state.clock)),
                Arc::clone(&state.clock),
                Duration::from_secs(3600),
                router_stop.clone(),
            ));
            // wait for the first router submission to become visible, then submit the resync_schedule evals
            let first = loop {
                if state.runtime.broker().counters().queued > 0 || ctl.active.load(Ordering::Relaxed) > 0 {
                    break Instant::now();
                }
                if t0.elapsed() > Duration::from_secs(5) {
                    panic!("router never submitted");
                }
                tokio::task::yield_now().await;
            };
            list_to_first_submit_ms = ms(first.duration_since(t0));
            for e in cadence_evals {
                state.runtime.submit_evaluation(e, Instant::now(), EvaluationEligibility::Immediate);
            }
            // edge deploy 3 ms after the burst's first submission
            let wait = Duration::from_millis(3).saturating_sub(first.elapsed());
            tokio::time::sleep(wait).await;
            let r = edge_sample(state, ctl, &edge_id, service_edge).await;
            handler = r.0;
            edge_rec = r.1;
        }
    }
    // t0_ms for baseline is the spawn instant (includes the list read); the edge offset below is relative to the first submission
    let edge_offset_base_ms = match mode {
        Mode::Proposed => 0.0,
        Mode::Baseline => list_to_first_submit_ms,
    };
    wait_idle(state, ctl).await;
    let wall_incl_tail = t0.elapsed();
    let cpu1 = cpu_secs();
    probe_stop.store(true, Ordering::Relaxed);
    ph.await.unwrap();
    if let Some(h) = router_handle {
        router_stop.cancel();
        let _ = h.await;
    }

    let recs = ctl.recs.lock().unwrap().clone();
    let mut first_done: BTreeMap<(String, String), f64> = BTreeMap::new();
    for r in &recs {
        let k = (r.reconciler.clone(), r.target.clone());
        if keys.contains(&k) {
            first_done.entry(k).and_modify(|d| *d = d.min(r.done_at_ms)).or_insert(r.done_at_ms);
        }
    }
    let keys_missing = keys.len() - first_done.len();
    let pass_done_ms = first_done.values().cloned().fold(0.0_f64, f64::max);
    let pass_wall_ms = pass_done_ms - t0_ms;
    let pass_recs: Vec<&Rec> = recs.iter().filter(|r| keys.contains(&(r.reconciler.clone(), r.target.clone()))).collect();
    let extra_recs = recs.len() - pass_recs.len();
    let queued: Vec<f64> = pass_recs.iter().map(|r| r.queued_ms).collect();
    let elapsed: Vec<f64> = pass_recs.iter().map(|r| r.elapsed_ms).collect();
    let mut by_rec: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for r in &pass_recs {
        by_rec.entry(r.reconciler.clone()).or_default().push(r.elapsed_ms);
    }
    let by_rec_json: BTreeMap<String, Value> = by_rec.iter().map(|(k, v)| (k.clone(), stats_json(&stats(v)))).collect();
    let by_rec_count: BTreeMap<String, usize> = by_rec.iter().map(|(k, v)| (k.clone(), v.len())).collect();
    let sum_elapsed: f64 = elapsed.iter().sum();
    let drains = ctl.drain_calls_ns.lock().unwrap().clone();
    let drain_total_ms: f64 = drains.iter().map(|(_, ns)| *ns as f64 / 1e6).sum();
    let drain_max_ms: f64 = drains.iter().map(|(_, ns)| *ns as f64 / 1e6).fold(0.0, f64::max);
    let probes = probe_out.lock().unwrap();
    let in_pass: Vec<f64> = probes.iter().filter(|p| p.at_ms >= t0_ms && p.at_ms <= pass_done_ms).map(|p| p.wait_us).collect();
    let edge = edge_rec.as_ref();
    let edge_submitted_ms = edge.map(|r| r.done_at_ms - r.elapsed_ms - r.queued_ms - t0_ms); // approx: admitted - queued
    let sample = json!({
        "mode": mode.name(),
        "sample": s,
        "evals_submitted": keys.len(),
        "keys_never_completed": keys_missing,
        "evals_by_reconciler": by_rec_count,
        "evals_total_incl_requeue_and_edge": recs.len(),
        "extra_evals_beyond_pass": extra_recs,
        "pass_wall_ms": pass_wall_ms,
        "list_to_first_submit_ms": list_to_first_submit_ms,
        "wall_incl_edge_tail_ms": ms(wall_incl_tail),
        "cpu_s_during_window": cpu1 - cpu0,
        "duty_cycle_wall_pct_of_30s": pass_wall_ms / 1000.0 / 30.0 * 100.0,
        "duty_cycle_cpu_pct_of_30s_one_core": (cpu1 - cpu0) / 30.0 * 100.0,
        "sum_tick_elapsed_ms": sum_elapsed,
        "queued_ms_all_pass_evals": stats_json(&stats(&queued)),
        "tick_elapsed_ms_all_pass_evals": stats_json(&stats(&elapsed)),
        "tick_elapsed_ms_by_reconciler": by_rec_json,
        "drain_pending_calls": drains.len(),
        "drain_pending_total_ms": drain_total_ms,
        "drain_pending_max_ms": drain_max_ms,
        "edge": {
            "kind": if service_edge {"service"} else {"job"},
            "edge_offset_base_ms": edge_offset_base_ms,
            "edge_submitted_approx_ms_after_t0": edge_submitted_ms,
            "deploy_handler_wall_ms": ms(handler),
            "admission_delay_ms": edge.map(|r| r.queued_ms),
            "tick_elapsed_ms": edge.map(|r| r.elapsed_ms),
            "completed_after_burst_start_ms": edge.map(|r| r.done_at_ms - t0_ms),
        },
        "allocator_lock_wait_us_in_pass": stats_json(&stats(&in_pass)),
    });
    if verbose {
        eprintln!(
            "[N={n}] {} sample {s}: evals {} (missing {keys_missing}, extra {extra_recs}) wall {:.0} ms (list->first {:.1} ms) duty {:.2}% cpu {:.2}s edge({}) admission {:?} ms handler {:.1} ms; lock wait p50 {:.0} us max {:.0} us",
            mode.name(),
            keys.len(),
            pass_wall_ms,
            list_to_first_submit_ms,
            pass_wall_ms / 300.0,
            cpu1 - cpu0,
            if service_edge { "svc" } else { "job" },
            edge.map(|r| r.queued_ms),
            ms(handler),
            stats(&in_pass).p50,
            stats(&in_pass).max
        );
    }
    let mut raw = String::new();
    for r in &recs {
        raw.push_str(
            &json!({"n": n, "mode": mode.name(), "sample": s, "reconciler": r.reconciler, "target": r.target, "queued_ms": r.queued_ms,
                "elapsed_ms": r.elapsed_ms, "admitted_at_ms": r.admitted_at_ms - t0_ms, "done_at_ms": r.done_at_ms - t0_ms, "ok": r.ok})
            .to_string(),
        );
        raw.push('\n');
    }
    drop(probes);
    (sample, raw)
}

async fn run_n(n: usize, samples: usize, data_root: &Path, evidence: &Path) -> Value {
    let root = data_root.join(format!("n{n}"));
    let _ = std::fs::remove_dir_all(&root);
    let fx = build_fixture(&root).await;
    let state = fx.state.clone();
    let epoch0 = Instant::now();
    let ctl = Arc::new(LoopCtl {
        stop: AtomicBool::new(false),
        active: AtomicUsize::new(0),
        recs: std::sync::Mutex::new(vec![]),
        drain_calls_ns: std::sync::Mutex::new(vec![]),
        epoch0,
    });
    let loop_h = spawn_loop(state.clone(), Arc::clone(&ctl));

    // seed N workloads through the real deploy handler: 80% Job / 20% Service
    let t_seed = Instant::now();
    for i in 0..n {
        if i % 5 == 0 {
            deploy(&state, service_spec(&format!("svc-{i}"))).await;
        } else {
            deploy(&state, job_spec(&format!("job-{i}"))).await;
        }
    }
    let seed_s = t_seed.elapsed().as_secs_f64();
    let (rounds, last) = settle(&state, &ctl).await;
    let alloc_rows = state.obs.alloc_status_rows().await.expect("rows").len();
    let backend_rows = state.obs.all_service_backends_rows().await.expect("rows").len();
    let running = state
        .obs
        .alloc_status_rows()
        .await
        .expect("rows")
        .iter()
        .filter(|r| format!("{:?}", r.state) == "Running")
        .count();
    eprintln!(
        "[N={n}] seeded in {seed_s:.1}s; settled after {rounds} rounds ({last} evals in last); alloc rows {alloc_rows} (running {running}), service_backends rows {backend_rows}"
    );

    // enumeration cost (proxy for managed_targets)
    let mut enum_ms = vec![];
    for _ in 0..10 {
        let t = Instant::now();
        let _ = enumerate(&state).await;
        enum_ms.push(ms(t.elapsed()));
    }
    let set = enumerate(&state).await;
    let per_reconciler_targets: BTreeMap<String, usize> =
        set.iter().map(|(k, v)| (k.to_string(), v.len())).collect();
    let total_targets: usize = per_reconciler_targets.values().sum();

    // M1
    ctl.recs.lock().unwrap().clear();
    let per_eval = measure_per_eval(&state, &set).await;
    wait_idle(&state, &ctl).await;
    let (_, again) = settle(&state, &ctl).await;
    eprintln!("[N={n}] M1 done (settled, {again} evals)");

    // M3 baseline: edge wake with no pass (idle), 3 samples
    let mut idle_adm = vec![];
    let mut idle_handler_job = vec![];
    for i in 0..3 {
        ctl.recs.lock().unwrap().clear();
        let (h, rec) = edge_sample(&state, &ctl, &format!("idle-edge-{i}"), false).await;
        idle_handler_job.push(ms(h));
        idle_adm.push(rec.map_or(f64::NAN, |r| r.queued_ms));
        wait_idle(&state, &ctl).await;
    }
    let mut idle_adm_svc = vec![];
    let mut idle_handler_svc = vec![];
    for i in 0..3 {
        ctl.recs.lock().unwrap().clear();
        let (h, rec) = edge_sample(&state, &ctl, &format!("idle-edge-svc-{i}"), true).await;
        idle_handler_svc.push(ms(h));
        idle_adm_svc.push(rec.map_or(f64::NAN, |r| r.queued_ms));
        wait_idle(&state, &ctl).await;
    }
    // idle lock-wait baseline (300 ms window)
    let probe_out = Arc::new(std::sync::Mutex::new(vec![]));
    let probe_stop = Arc::new(AtomicBool::new(false));
    let ph = spawn_lock_probe(state.clone(), Arc::clone(&probe_stop), epoch0, Arc::clone(&probe_out));
    tokio::time::sleep(Duration::from_millis(300)).await;
    probe_stop.store(true, Ordering::Relaxed);
    ph.await.unwrap();
    let idle_lock: Vec<f64> = probe_out.lock().unwrap().iter().map(|s| s.wait_us).collect();
    settle(&state, &ctl).await;

    // M2 + M3: pass samples. Mode::Baseline = today's router relist + resync_schedule; Mode::Proposed = the ADR-0282 pass
    // (same code as the prior run), interleaved sample by sample as a same-run control.
    let mut pass_json = vec![];
    let mut raw = String::new();
    // one unmeasured baseline burst: first service-lifecycle ticks on Job targets create their Views (cold), steady state follows
    {
        let (v, r) = one_sample(&state, &ctl, epoch0, n, 0, Mode::Baseline, true, false).await;
        let mut v = v;
        v["warmup"] = json!(true);
        raw.push_str(&r);
        pass_json.push(v);
        settle(&state, &ctl).await;
    }
    for round in 0..samples {
        for mode in [Mode::Baseline, Mode::Proposed] {
            let (v, r) = one_sample(&state, &ctl, epoch0, n, round, mode, round % 2 == 1, true).await;
            raw.push_str(&r);
            pass_json.push(v);
            settle(&state, &ctl).await;
        }
    }
    std::fs::write(evidence.join(format!("raw-evals-n{n}.jsonl")), raw).expect("write raw");

    ctl.stop.store(true, Ordering::Relaxed);
    let _ = loop_h.await;
    drop(fx);
    json!({
        "n": n,
        "seed_s": seed_s,
        "alloc_rows": alloc_rows, "running_rows": running, "service_backends_rows": backend_rows,
        "settle_rounds": rounds,
        "targets_per_reconciler": per_reconciler_targets,
        "targets_total": total_targets,
        "enumeration_proxy_ms": stats_json(&stats(&enum_ms)),
        "per_evaluation_serial_uncontended": per_eval,
        "idle_edge_wake": {
            "job_admission_delay_ms": stats_json(&stats(&idle_adm)),
            "job_deploy_handler_ms": stats_json(&stats(&idle_handler_job)),
            "service_admission_delay_ms": stats_json(&stats(&idle_adm_svc)),
            "service_deploy_handler_ms": stats_json(&stats(&idle_handler_svc)),
            "allocator_lock_wait_us_idle": stats_json(&stats(&idle_lock)),
        },
        "pass_samples": pass_json,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let evidence = PathBuf::from(args.get(1).expect("usage: resync-cost <evidence-dir> <data-root> [ns] [samples]"));
    let data_root = PathBuf::from(args.get(2).expect("data root"));
    let ns: Vec<usize> = args
        .get(3)
        .map_or("10,100,500,1000".to_owned(), Clone::clone)
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect();
    let samples: usize = args.get(4).map_or(5, |s| s.parse().unwrap());
    std::fs::create_dir_all(&evidence).unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let mut all = vec![];
    for n in ns {
        let v = rt.block_on(run_n(n, samples, &data_root, &evidence));
        std::fs::write(evidence.join(format!("result-n{n}.json")), serde_json::to_string_pretty(&v).unwrap())
            .unwrap();
        all.push(v);
    }
    std::fs::write(evidence.join("results.json"), serde_json::to_string_pretty(&all).unwrap()).unwrap();
    let _ = AsyncMutex::new(());
}
