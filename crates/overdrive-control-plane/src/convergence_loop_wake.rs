//! D17 (ADR-0273): a broker submit wakes the convergence loop at once.
//!
//! Traceability: obligation S-48 (`NoLostWake`, `CadenceNeverStarved`,
//! `DrainOrderDeterministicAcrossSeeds`). No Quint model by design decision S6;
//! this scenario is the evidence. The production `spawn_convergence_loop` runs
//! over a `SimClock` that never auto-advances, so any admission observed
//! without a clock advance was caused by the submit's wake. Observations:
//! `EvaluationBroker::counters()` (`queued`, `dispatched`, `cancelled`) and the
//! loop's `convergence.evaluation.admitted` events (admission order).
//!
//! Not covered here: S-48 (b), a submit landing in the drain-to-park window, and
//! (e), a submit for a target whose evaluation is in flight. A current-thread
//! simulator has no await point in the first window, and the second needs a
//! reconciler whose tick blocks, which no production reconciler offers.
//! (b) rests on tokio's documented stored permit and the mutation targets.
//!
//! CONTRACT_SHAPE: bounded-change.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::print_stderr,
    reason = "scenarios assert their own preconditions; the seed is printed for reproduction"
)]

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::eval_broker::{Evaluation, EvaluationEligibility};
use overdrive_core::id::NodeId;
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::reconciler_runtime::ReconcilerRuntime;
use crate::{AppState, spawn_convergence_loop};

const CADENCE: Duration = Duration::from_millis(100);

struct Loop {
    state: AppState,
    clock: Arc<SimClock>,
    shutdown: CancellationToken,
    _tmp: tempfile::TempDir,
}

async fn settle() {
    for _ in 0..128 {
        tokio::task::yield_now().await;
    }
}

/// The production runtime with `noop-heartbeat` registered, and the
/// production convergence loop spawned over a `SimClock`, parked.
async fn parked_loop() -> Loop {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    runtime.register(crate::noop_heartbeat()).await.expect("register noop-heartbeat");
    let node = NodeId::new("wake-node").expect("node id");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
    let obs = Arc::new(SimObservationStore::single_peer(node.clone(), 0));
    let clock = Arc::new(SimClock::new());
    let allocator = crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    let mtls_worker = crate::shared_network_test_ports::mtls_worker();
    mtls_worker.start_shared_owner().await.expect("shared mTLS owner starts over sim adapters");
    let guest_owner = crate::shared_network_test_ports::shared_guest_network_owner();
    crate::guest_network::SharedGuestNetworkOwner::converge_shared(guest_owner.as_ref())
        .await
        .expect("shared owner reports its bound flow ports");
    let wiring = overdrive_core::guest_network::GuestNetworkExecWiring::new(
        Arc::clone(&clock) as Arc<dyn Clock>,
    );
    assert!(wiring.supervisor().open_after_boot());
    let state = AppState::new(
        store,
        store_path,
        obs as Arc<dyn ObservationStore>,
        Arc::new(runtime),
        Arc::new(SimDriver::with_clock(DriverType::Vm, clock.clone())) as Arc<dyn Driver>,
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
    let shutdown = CancellationToken::new();
    let _task =
        spawn_convergence_loop(state.clone(), Arc::clone(&clock) as Arc<dyn Clock>, CADENCE, shutdown.clone());
    settle().await;
    Loop { state, clock, shutdown, _tmp: tmp }
}

fn eval(target: &str) -> Evaluation {
    Evaluation {
        reconciler: ReconcilerName::new("noop-heartbeat").expect("name"),
        target: TargetResource::new(target).expect("target"),
    }
}

impl Loop {
    fn submit(&self, target: &str, eligibility: EvaluationEligibility) {
        self.state.runtime.submit_evaluation(eval(target), self.clock.now(), eligibility);
    }

    fn counters(&self) -> overdrive_core::eval_broker::BrokerCounters {
        self.state.runtime.broker().counters()
    }

    fn stop(&self) {
        self.shutdown.cancel();
    }
}

/// Records the order of `convergence.evaluation.admitted` events.
#[derive(Clone, Default)]
struct AdmissionOrder(Arc<std::sync::Mutex<Vec<String>>>);

struct TargetField<'a>(&'a mut Option<String>);

impl tracing::field::Visit for TargetField<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "target" {
            *self.0 = Some(format!("{value:?}"));
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "target" {
            *self.0 = Some(value.to_owned());
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for AdmissionOrder {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if event.metadata().name() == "convergence.evaluation.admitted" {
            let mut target = None;
            event.record(&mut TargetField(&mut target));
            self.0.lock().expect("lock").push(target.unwrap_or_default());
        }
    }
}

/// S-48 (a): a submit while the loop is parked is admitted with no clock
/// advance, exactly once.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER: D17 a broker submit wakes the convergence loop at once"]
async fn a_submit_while_the_loop_is_parked_is_admitted_without_waiting_for_the_tick() {
    let l = parked_loop().await;
    assert_eq!(l.counters().dispatched, 0);

    l.submit("node/a", EvaluationEligibility::Immediate);
    settle().await;

    let c = l.counters();
    assert_eq!(c.dispatched, 1, "admitted at once, once, with the clock not advanced");
    assert_eq!(c.queued, 0);
    l.stop();
}

/// S-48 (c): a burst for distinct targets drains in one wake up to the
/// capacity then on completions, in full; a same-key burst collapses to one
/// dispatch.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER: D17 a broker submit wakes the convergence loop at once"]
async fn a_burst_of_submits_coalesces_per_target_and_drains_in_full() {
    let l = parked_loop().await;
    for n in 0..12 {
        l.submit(&format!("node/burst-{n}"), EvaluationEligibility::Immediate);
    }
    settle().await;
    assert_eq!(l.counters().dispatched, 12, "twelve distinct targets, all admitted");
    assert_eq!(l.counters().queued, 0);

    let before = l.counters();
    for _ in 0..5 {
        l.submit("node/same", EvaluationEligibility::Immediate);
    }
    settle().await;
    let after = l.counters();
    assert_eq!(after.dispatched - before.dispatched, 1, "a same-key burst is one dispatch");
    assert_eq!(after.cancelled - before.cancelled, 4, "the four superseded submits are cancelled");
    l.stop();
}

/// S-48 (d): with no submits, advancing the clock one cadence period keeps
/// the loop alive and drains nothing. Passes on HEAD.
#[tokio::test(flavor = "current_thread")]
async fn the_cadence_tick_with_no_submits_drains_nothing_and_keeps_the_loop_alive() {
    let l = parked_loop().await;
    for _ in 0..3 {
        l.clock.tick(CADENCE);
        settle().await;
    }
    let c = l.counters();
    assert_eq!((c.dispatched, c.queued), (0, 0));
    l.stop();
    settle().await;
}

/// S-48 (f): a deferred evaluation is admitted at its eligibility, not earlier
/// because of an unrelated wake.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER: D17 a broker submit wakes the convergence loop at once"]
async fn a_deferred_evaluation_is_not_admitted_early_by_an_unrelated_wake() {
    let l = parked_loop().await;
    let not_before =
        UnixInstant::from_clock(&*l.clock) + Duration::from_secs(5);
    l.submit("node/deferred", EvaluationEligibility::NotBefore(not_before));
    l.submit("node/unrelated", EvaluationEligibility::Immediate);
    settle().await;
    assert_eq!(l.counters().dispatched, 1, "only the unrelated target is admitted");
    assert_eq!(l.counters().queued, 1, "the deferred evaluation still waits");

    l.clock.tick(Duration::from_secs(5));
    settle().await;
    assert_eq!(l.counters().dispatched, 2, "admitted once its eligibility passes");
    assert_eq!(l.counters().queued, 0);
    l.stop();
}

/// S-48: the admission order of a burst is identical across two runs of the
/// same seed.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER: D17 a broker submit wakes the convergence loop at once"]
async fn the_admission_order_of_a_burst_is_identical_across_runs_of_one_seed() {
    async fn run(seed: u64) -> Vec<String> {
        let order = AdmissionOrder::default();
        let _guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(order.clone()));
        let l = parked_loop().await;
        let mut targets: Vec<u64> = (0..6).collect();
        // a seeded permutation of the submit order
        let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        for i in (1..targets.len()).rev() {
            state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let j = usize::try_from(state >> 33).expect("fits") % (i + 1);
            targets.swap(i, j);
        }
        for t in targets {
            l.submit(&format!("node/order-{t}"), EvaluationEligibility::Immediate);
        }
        settle().await;
        l.stop();
        order.0.lock().expect("lock").clone()
    }
    for seed in 0_u64..4 {
        eprintln!("convergence loop wake: seed {seed}");
        let first = run(seed).await;
        let second = run(seed).await;
        assert_eq!(first.len(), 6, "seed {seed}: every submit admitted");
        assert_eq!(first, second, "seed {seed}: admission order differs between runs");
    }
}
