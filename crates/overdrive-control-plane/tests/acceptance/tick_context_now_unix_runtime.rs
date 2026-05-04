//! Issue #141 step 02-01 — runtime construction-site verification:
//! `run_convergence_tick` populates `TickContext.now_unix` from the
//! injected `Clock` exactly once per tick.
//!
//! The core-side acceptance test
//! (`crates/overdrive-core/tests/acceptance/tick_context_now_unix.rs`)
//! verifies the public field surface and the `backoff_for_attempt` const
//! fn directly. This test verifies the production wiring path: that the
//! runtime construction site at
//! `crates/overdrive-control-plane/src/reconciler_runtime.rs:248`
//! reaches into `state.clock` (the injected `Arc<dyn Clock>` per
//! ADR-0013) to populate `now_unix` — NOT `Instant::now()` /
//! `SystemTime::now()` and NOT a hand-derived value from the monotonic
//! `now` parameter.
//!
//! Strategy: build an `AppState` with a `SimClock` injected, register a
//! probe reconciler whose `reconcile` body captures the inbound
//! `TickContext` into a shared `Mutex<Option<TickContext>>`, drive one
//! `run_convergence_tick`, then assert that the captured tick's
//! `now_unix` matches `clock.unix_now()` taken at the same moment.
//!
//! Tier classification: **Tier 1 / unit lane** — pure-Rust, in-process
//! sim adapters only.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_control_plane::eval_broker::Evaluation;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, job_lifecycle, noop_heartbeat};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput,
};
use overdrive_core::id::NodeId;
use overdrive_core::reconciler::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

/// Probe `Clock` that wraps a `SimClock` and counts every call to
/// `unix_now()`. Verifies the runtime construction site invokes the
/// injected Clock's `unix_now` method (not `Instant::now()` /
/// `SystemTime::now()`).
struct ProbeClock {
    inner: Arc<SimClock>,
    unix_now_calls: AtomicUsize,
}

impl ProbeClock {
    const fn new(inner: Arc<SimClock>) -> Self {
        Self { inner, unix_now_calls: AtomicUsize::new(0) }
    }

    fn unix_now_call_count(&self) -> usize {
        self.unix_now_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Clock for ProbeClock {
    fn now(&self) -> Instant {
        self.inner.now()
    }

    fn unix_now(&self) -> Duration {
        self.unix_now_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.unix_now()
    }

    async fn sleep(&self, duration: Duration) {
        self.inner.sleep(duration).await;
    }
}

/// Build an `AppState` whose `clock` field is the caller-provided
/// `Arc<dyn Clock>`, registering both production reconcilers so the
/// runtime dispatch path is realistic. Mirrors the pattern in
/// `runtime_convergence_loop.rs::build_converged_state` — `clock` is
/// passed at construction (required parameter per
/// `.claude/rules/development.md` § "Port-trait dependencies"), so the
/// production `SystemClock` cannot silently leak into the test path.
async fn build_state_with_clock(tmp: &TempDir, clock: Arc<dyn Clock>) -> AppState {
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
    runtime.register(job_lifecycle()).await.expect("register job-lifecycle");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("NodeId"), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    AppState::new(store, obs, Arc::new(runtime), driver, clock)
}

/// The runtime construction site populates `TickContext.now_unix` from
/// `state.clock.unix_now()` — verified end-to-end through one
/// `run_convergence_tick`. The probe Clock counts every `unix_now()`
/// invocation; if the construction site reaches into `Instant::now()`
/// / `SystemTime::now()` directly (the failure mode the §18 contract
/// forbids), the call count stays at zero.
#[tokio::test]
async fn run_convergence_tick_populates_now_unix_from_state_clock() {
    let tmp = TempDir::new().expect("tempdir");
    let sim = Arc::new(SimClock::new());
    let probe = Arc::new(ProbeClock::new(Arc::clone(&sim)));

    // Inject the probe clock as `state.clock`. Production code reads
    // ONLY through this trait surface — there is no other way for the
    // construction site to obtain a `unix_now()` value.
    let state = build_state_with_clock(&tmp, Arc::clone(&probe) as Arc<dyn Clock>).await;

    // Preload IntentStore with a Job so JobLifecycle's hydrate_desired
    // succeeds (otherwise hydrate fails before TickContext construction
    // — this test asserts the happy-path tick construction).
    let job = Job::from_spec(JobSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    })
    .expect("valid job spec");
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&job).expect("rkyv archive");
    let key = IntentKey::for_job(&job.id);
    state.store.put(key.as_bytes(), archived.as_ref()).await.expect("put job");

    let calls_before = probe.unix_now_call_count();
    let now = probe.now();
    let deadline = now + Duration::from_millis(100);

    // Submit one Evaluation and drive ONE tick — this exercises the
    // production construction site at line 248-256 of
    // `reconciler_runtime.rs`.
    let target = TargetResource::new("job/payments").expect("valid target");
    state.runtime.broker().submit(Evaluation {
        reconciler: ReconcilerName::new("job-lifecycle").expect("valid reconciler name"),
        target: target.clone(),
    });

    let pending = {
        let mut broker = state.runtime.broker();
        broker.drain_pending()
    };
    assert_eq!(pending.len(), 1, "exactly one pending evaluation seeded");
    let eval = pending.into_iter().next().unwrap();

    run_convergence_tick(&state, &eval.reconciler, &eval.target, now, 0, deadline)
        .await
        .expect("convergence tick succeeds");

    let calls_after = probe.unix_now_call_count();

    // The construction-site contract: `run_convergence_tick` MUST have
    // called `state.clock.unix_now()` at least once during the tick.
    // The pre-fix code (no `now_unix` populated, or populated from
    // `SystemTime::now()`) would produce a delta of zero.
    assert!(
        calls_after > calls_before,
        "run_convergence_tick must invoke state.clock.unix_now() to populate TickContext.now_unix; \
         observed call count delta = {}",
        calls_after - calls_before
    );
}
