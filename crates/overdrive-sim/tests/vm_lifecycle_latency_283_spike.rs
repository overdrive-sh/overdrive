//! Issue-283 regression: the actual broker/convergence owner under slow Driver effects.
//! No terminal rows, private lifecycle state, or broker schedule are fabricated.
//! The Driver port substitutes external process latency; native probes own VMM timing.
#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::print_stderr, clippy::too_many_lines, clippy::large_futures)]
#![allow(
    clippy::doc_markdown,
    reason = "the required per-test CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use async_trait::async_trait;
use base64::Engine;
use overdrive_control_plane::api::{SubmitWorkloadRequest, SubmitWorkloadResponse};
use overdrive_control_plane::dataplane_config::DataplaneConfig;
use overdrive_control_plane::{ServerConfig, run_server_with_obs_and_driver};
use overdrive_core::aggregate::{DriverInput, ExecInput, JobSpecInput, ResourcesInput};
use overdrive_core::api::submit::SubmitSpecInput;
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, Resources,
};
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_sim::adapters::{
    SimKek, clock::SimClock, dataplane::SimDataplane, driver::SimDriver,
    observation_store::SimObservationStore,
};
use parking_lot::Mutex;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Semaphore, mpsc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Effect {
    Healthy,
    Start,
    Stop,
}

struct DelayedDriver {
    inner: SimDriver,
    effect: Effect,
    entered: mpsc::UnboundedSender<Effect>,
    release: Semaphore,
    events: Mutex<Vec<DriverEvent>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DriverEvent {
    Entered(Effect, AllocationId),
    Returned(Effect, AllocationId),
}

#[async_trait]
impl Driver for DelayedDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Exec
    }
    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        self.events.lock().push(DriverEvent::Entered(Effect::Start, spec.alloc.clone()));
        if self.effect == Effect::Start && spec.alloc.as_str().starts_with("alloc-slow") {
            self.entered.send(Effect::Start).unwrap();
            self.release.acquire().await.unwrap().forget();
        }
        let result = self.inner.start(spec).await?;
        self.events.lock().push(DriverEvent::Returned(Effect::Start, spec.alloc.clone()));
        Ok(result)
    }
    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        self.events.lock().push(DriverEvent::Entered(Effect::Stop, handle.alloc.clone()));
        self.inner.stop(handle).await?;
        if self.effect == Effect::Stop && handle.alloc.as_str().starts_with("alloc-slow") {
            self.entered.send(Effect::Stop).unwrap();
            self.release.acquire().await.unwrap().forget();
        }
        self.events.lock().push(DriverEvent::Returned(Effect::Stop, handle.alloc.clone()));
        Ok(())
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
    fn release_supervision(&self, alloc: &overdrive_core::id::AllocationId) {
        self.inner.release_supervision(alloc);
    }
}

fn client(config_dir: &std::path::Path) -> reqwest::Client {
    let contents = std::fs::read_to_string(config_dir.join(".overdrive/config")).unwrap();
    let config: toml::Value = toml::from_str(&contents).unwrap();
    let encoded = config["contexts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"].as_str() == Some("local"))
        .unwrap()["ca"]
        .as_str()
        .unwrap();
    let pem = base64::engine::general_purpose::STANDARD.decode(encoded).unwrap();
    reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(&pem).unwrap())
        .https_only(true)
        .use_rustls_tls()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

async fn submit(client: &reqwest::Client, base: &str, id: &str) {
    let body = SubmitWorkloadRequest {
        spec: SubmitSpecInput::Job(JobSpecInput {
            id: id.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: DriverInput::Exec(ExecInput {
                command: "/bin/sleep".to_owned(),
                args: vec!["3600".to_owned()],
            }),
        }),
    };
    let response = client.post(format!("{base}/v1/workloads")).json(&body).send().await.unwrap();
    assert!(response.status().is_success(), "submit {id}: {response:?}");
    let _: SubmitWorkloadResponse = response.json().await.unwrap();
}

async fn running(obs: &SimObservationStore, id: &str) -> bool {
    obs.alloc_status_rows()
        .await
        .unwrap()
        .iter()
        .any(|row| row.workload_id.as_str() == id && row.state == AllocState::Running)
}

fn effect_targets(
    driver: &DelayedDriver,
    effect: Effect,
    returned: bool,
) -> BTreeSet<AllocationId> {
    driver
        .events
        .lock()
        .iter()
        .filter_map(|event| match event {
            DriverEvent::Returned(kind, alloc) if returned && *kind == effect => {
                Some(alloc.clone())
            }
            DriverEvent::Entered(kind, alloc) if !returned && *kind == effect => {
                Some(alloc.clone())
            }
            _ => None,
        })
        .collect()
}

async fn result_state_targets(obs: &SimObservationStore, effect: Effect) -> BTreeSet<AllocationId> {
    let state = if effect == Effect::Start { AllocState::Running } else { AllocState::Terminated };
    obs.alloc_status_rows()
        .await
        .unwrap()
        .into_iter()
        .filter(|row| row.workload_id.as_str().starts_with("slow") && row.state == state)
        .map(|row| row.alloc_id)
        .collect()
}

async fn advance(clock: &SimClock, seed: u64) {
    clock.tick(Duration::from_millis(100 + seed % 7));
    // Real HTTPS/redb are only integration-host scheduling. Their wall-clock
    // settle is not counted as a simulated latency result or product KPI.
    tokio::time::sleep(Duration::from_millis(5)).await;
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

async fn drive(effect: Effect) {
    let seed = 283_001;
    eprintln!("issue283 seed={seed} effect={effect:?}");
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(SimClock::new());
    let node = NodeId::new("local").unwrap();
    let obs = Arc::new(SimObservationStore::single_peer(node, seed));
    let (entered, mut entries) = mpsc::unbounded_channel();
    let driver = Arc::new(DelayedDriver {
        inner: SimDriver::with_clock(DriverType::Exec, clock.clone()),
        effect,
        entered,
        release: Semaphore::new(0),
        events: Mutex::new(Vec::new()),
    });
    let config_dir = directory.path().join("operator");
    let config = ServerConfig {
        data_dir: directory.path().join("data"),
        operator_config_dir: config_dir.clone(),
        clock: clock.clone(),
        dataplane: Some(DataplaneConfig { client_iface: "lo".into(), backend_iface: "lo".into() }),
        dataplane_override: Some(Arc::new(SimDataplane::new())),
        ..ServerConfig::new(Arc::new(SimKek::for_boot()))
    };
    let server = run_server_with_obs_and_driver(config, obs.clone(), driver.clone()).await.unwrap();
    let bound = server.local_addr().await.unwrap();
    let client = client(&config_dir);
    let base = format!("https://localhost:{}", bound.port());
    submit(&client, &base, "slow").await;
    if effect != Effect::Start {
        for _ in 0..50 {
            advance(&clock, seed).await;
            if running(&obs, "slow").await {
                break;
            }
        }
        assert!(running(&obs, "slow").await, "seed={seed}: initial healthy start failed");
    }
    if effect == Effect::Stop {
        let response = client.post(format!("{base}/v1/workloads/slow/stop")).send().await.unwrap();
        assert!(response.status().is_success(), "stop: {response:?}");
    }
    if effect != Effect::Healthy {
        let mut observed = None;
        for _ in 0..50 {
            advance(&clock, seed).await;
            if let Ok(event) = entries.try_recv() {
                observed = Some(event);
                break;
            }
        }
        assert_eq!(observed, Some(effect), "seed={seed}: real owner never entered slow effect");
    }
    submit(&client, &base, "independent").await;
    let mut progressed_while_held = false;
    for _ in 0..10 {
        advance(&clock, seed).await;
        // This public query additionally proves the HTTP owner remains live.
        let response = client.get(format!("{base}/v1/cluster/info")).send().await.unwrap();
        assert!(response.status().is_success());
        progressed_while_held |= running(&obs, "independent").await;
    }
    driver.release.add_permits(1);
    let mut progressed_after_release = progressed_while_held;
    for _ in 0..100 {
        advance(&clock, seed).await;
        progressed_after_release |= running(&obs, "independent").await;
        if progressed_after_release {
            break;
        }
    }
    // Cooperative production shutdown, never abort the convergence owner.
    server.shutdown(Duration::from_secs(1)).await.unwrap();
    eprintln!(
        "issue283 seed={seed} effect={effect:?} independent_running_while_held={progressed_while_held} independent_running_after_release={progressed_after_release} shutdown=joined"
    );
    assert!(progressed_after_release, "seed={seed}: recovery/control must progress");
    assert!(
        progressed_while_held,
        "RED scaffold (S-VLL-01): seed={seed}: independent workload cannot progress while {effect:?} holds the production convergence owner"
    );
}

/// CONTRACT_SHAPE: bounded-change.
/// Independent convergence while a start effect is pending.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn slow_start_does_not_block_independent_convergence() {
    drive(Effect::Start).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// Independent convergence while a stop effect is pending.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn slow_stop_does_not_block_independent_convergence() {
    drive(Effect::Stop).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// Healthy complete-owner convergence control.
#[tokio::test]
async fn healthy_driver_control_progresses() {
    drive(Effect::Healthy).await;
}

/// Drive the existing production owner with distinct public workload targets.
/// The semaphore substitutes only external Driver latency; admission, pending
/// coalescing, storage, HTTP owners, and shutdown are the real composition.
async fn capacity_case(effect: Effect, held: usize, close_admission: bool) {
    let seed = 283_001;
    eprintln!("vm-lifecycle seed={seed} effect={effect:?} held={held} close={close_admission}");
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(SimClock::new());
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").unwrap(), seed));
    let (entered, mut entries) = mpsc::unbounded_channel();
    let driver = Arc::new(DelayedDriver {
        inner: SimDriver::with_clock(DriverType::Exec, clock.clone()),
        effect,
        entered,
        release: Semaphore::new(0),
        events: Mutex::new(Vec::new()),
    });
    let config_dir = directory.path().join("operator");
    let config = ServerConfig {
        data_dir: directory.path().join("data"),
        operator_config_dir: config_dir.clone(),
        clock: clock.clone(),
        dataplane: Some(DataplaneConfig { client_iface: "lo".into(), backend_iface: "lo".into() }),
        dataplane_override: Some(Arc::new(SimDataplane::new())),
        ..ServerConfig::new(Arc::new(SimKek::for_boot()))
    };
    let server = run_server_with_obs_and_driver(config, obs.clone(), driver.clone()).await.unwrap();
    let base = format!("https://localhost:{}", server.local_addr().await.unwrap().port());
    let client = client(&config_dir);
    for index in 0..held {
        submit(&client, &base, &format!("slow{index}")).await;
    }
    if effect == Effect::Stop {
        for _ in 0..100 {
            advance(&clock, seed).await;
        }
        for index in 0..held {
            let id = format!("slow{index}");
            assert!(running(&obs, &id).await, "seed={seed}: healthy setup failed for {id}");
            let response =
                client.post(format!("{base}/v1/workloads/{id}/stop")).send().await.unwrap();
            assert!(response.status().is_success(), "seed={seed}: stop rejected for {id}");
        }
    }
    let mut entered_while_held = 0;
    for _ in 0..100 {
        advance(&clock, seed).await;
        while let Ok(observed) = entries.try_recv() {
            assert_eq!(observed, effect, "seed={seed}: wrong held effect");
            entered_while_held += 1;
        }
        if entered_while_held == held {
            break;
        }
    }
    submit(&client, &base, "independent").await;
    let mut while_held = false;
    for _ in 0..50 {
        advance(&clock, seed).await;
        while_held |= running(&obs, "independent").await;
    }
    let after_release;
    if close_admission {
        let entered_targets_at_close = effect_targets(&driver, effect, false);
        assert_eq!(
            entered_targets_at_close.len(),
            entered_while_held,
            "seed={seed}: entry ledger disagrees"
        );
        // Release one real Driver call at a time. Each returned allocation must
        // reach the shim-authored post-result row while shutdown still waits for
        // every remaining hold. A port return alone is not result consumption.
        let shutdown = tokio::spawn(server.shutdown(Duration::from_secs(1)));
        for _ in 0..10 {
            advance(&clock, seed).await;
        }
        let mut held_waits = Vec::new();
        let mut stages = Vec::new();
        for released in 1..=entered_while_held {
            held_waits.push(!shutdown.is_finished());
            driver.release.add_permits(1);
            for _ in 0..100 {
                advance(&clock, seed).await;
                let returned = effect_targets(&driver, effect, true);
                if returned.len() == released
                    && result_state_targets(&obs, effect).await == returned
                {
                    break;
                }
            }
            stages.push((
                effect_targets(&driver, effect, true),
                result_state_targets(&obs, effect).await,
            ));
        }
        // The current serial owner may have pre-drained additional evaluations
        // before the first held Driver call. Release those during cleanup too;
        // their later returns do not establish eight concurrent active effects.
        driver.release.add_permits(held);
        tokio::time::timeout(Duration::from_secs(10), shutdown).await.unwrap().unwrap().unwrap();
        after_release = running(&obs, "independent").await;
        assert!(
            held_waits.iter().all(|waiting| *waiting),
            "seed={seed}: a held result was discarded"
        );
        for (index, (returned, states)) in stages.iter().enumerate() {
            assert_eq!(returned.len(), index + 1, "seed={seed}: missing Driver result at {index}");
            assert!(
                returned.is_subset(&entered_targets_at_close),
                "seed={seed}: result without pre-close Driver entry"
            );
            assert_eq!(states, returned, "seed={seed}: shim did not publish every returned result");
        }
        let returned_at_join = effect_targets(&driver, effect, true);
        let entered_at_join = effect_targets(&driver, effect, false);
        assert_eq!(
            returned_at_join, entered_at_join,
            "seed={seed}: every entered Driver call returned"
        );
        assert_eq!(
            result_state_targets(&obs, effect).await,
            returned_at_join,
            "seed={seed}: final shim rows"
        );
        assert!(
            driver.events.lock().iter().all(|event| !matches!(
                event,
                DriverEvent::Entered(_, alloc) if alloc.as_str().starts_with("alloc-independent")
            )),
            "seed={seed}: ninth workload reached the Driver boundary"
        );
        assert!(
            obs.alloc_status_rows()
                .await
                .unwrap()
                .iter()
                .all(|row| row.workload_id.as_str() != "independent"),
            "seed={seed}: ninth workload acquired an allocation observation"
        );
        eprintln!(
            "vm-lifecycle seed={seed} effect={effect:?} entered_before_close={} returned_and_published_at_join={} sequential_release_stages={} ninth_driver_entries=0 ninth_allocation_rows=0; eight-way/private owner oracle remains pending",
            entered_targets_at_close.len(),
            returned_at_join.len(),
            stages.len()
        );
        // Current production reaches only one sequential-release stage, then
        // fails THIS concurrent-entry precondition. Later serial batch returns
        // cannot establish the eight-active or private owner-consumption oracle.
        assert_eq!(
            entered_while_held, held,
            "RED scaffold (S-VLL-06a admission): seed={seed} all eight slots must admit"
        );
        assert_eq!(
            entered_at_join, entered_targets_at_close,
            "seed={seed}: Driver entry set grew during drain"
        );
    } else {
        // One completion frees exactly one slot; it is enough for an eligible
        // independent target even when the other seven effects remain held.
        driver.release.add_permits(1);
        let mut progressed = while_held;
        for _ in 0..100 {
            advance(&clock, seed).await;
            progressed |= running(&obs, "independent").await;
            if progressed {
                break;
            }
        }
        after_release = progressed;
        driver.release.add_permits(held);
        for _ in 0..100 {
            advance(&clock, seed).await;
        }
        server.shutdown(Duration::from_secs(1)).await.unwrap();
    }
    eprintln!(
        "vm-lifecycle seed={seed} held={held} entered_while_held={entered_while_held} while_held={while_held} after_release={after_release} close={close_admission} shutdown=joined"
    );
    // The marker identifies a live, reproduced missing admission behavior. A
    // setup, transport, or cleanup panic cannot satisfy this expected RED.
    assert_eq!(
        entered_while_held, held,
        "RED scaffold (S-VLL-02/03): seed={seed} all requested slots must admit"
    );
    assert_eq!(while_held, held < 8, "RED scaffold (S-VLL-02/03): seed={seed} eight-slot boundary");
    assert_eq!(
        after_release, !close_admission,
        "RED scaffold (S-VLL-03/06): seed={seed} freed slot versus closed admission"
    );
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-02: seven pending starts leave capacity for independent convergence.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn seven_held_starts_leave_one_progress_slot() {
    capacity_case(Effect::Start, 7, false).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-02: seven pending stops leave capacity for independent convergence.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn seven_held_stops_leave_one_progress_slot() {
    capacity_case(Effect::Stop, 7, false).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-03: at eight starts the ninth waits, then one completion refills it.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn eight_held_starts_bound_and_refill_admission() {
    capacity_case(Effect::Start, 8, false).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-03: at eight stops the ninth waits, then one completion refills it.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn eight_held_stops_bound_and_refill_admission() {
    capacity_case(Effect::Stop, 8, false).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-06: shutdown drains eight starts and leaves the ninth unexecuted.
/// Per-target Driver return and shim row checks are live; the eight-way schedule
/// and private owner result-consumption/non-admission oracle remain pending.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn admission_close_drains_owned_starts_without_admitting_ninth() {
    capacity_case(Effect::Start, 8, true).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-06: shutdown drains eight stops and leaves the ninth unexecuted.
/// Per-target Driver return and shim row checks are live; the eight-way schedule
/// and private owner result-consumption/non-admission oracle remain pending.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn admission_close_drains_owned_stops_without_admitting_ninth() {
    capacity_case(Effect::Stop, 8, true).await;
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-13: the composed runtime must publish no effect when its View write
/// fails. Drive valid desired-generation inputs, never fabricate a next View;
/// prove the same inputs dispatch after recovery. Seed 283001 is a preservation
/// witness, not a newly alleged production defect.
#[tokio::test]
async fn view_fsync_failure_prevents_dispatch_and_recovers() {
    use overdrive_control_plane::AppState;
    use overdrive_control_plane::identity_mgr::IdentityMgr;
    use overdrive_control_plane::reconciler_runtime::{
        ConvergenceError, ReconcilerRuntime, run_convergence_tick,
    };
    use overdrive_control_plane::view_store::ViewStoreExt;
    use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
    use overdrive_core::traits::clock::Clock;
    use overdrive_core::traits::intent_store::IntentStore;
    use overdrive_reconcilers::WorkloadLifecycleView;
    use overdrive_sim::adapters::{ca::SimCa, entropy::SimEntropy, view_store::SimViewStore};
    use overdrive_store_local::LocalIntentStore;

    let seed = 283_001;
    eprintln!("vm-lifecycle S-VLL-13 seed={seed} healthy/fsync-failure/recovery");
    let directory = tempfile::tempdir().unwrap();
    let clock = Arc::new(SimClock::new());
    let views = Arc::new(SimViewStore::new());
    let mut runtime = ReconcilerRuntime::new(directory.path(), views.clone()).unwrap();
    runtime.register(overdrive_control_plane::workload_lifecycle()).await.unwrap();
    let path = directory.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&path).unwrap());
    let node = NodeId::new("local").unwrap();
    let obs = Arc::new(SimObservationStore::single_peer(node.clone(), seed));
    let driver = Arc::new(SimDriver::with_clock(DriverType::Exec, clock.clone()));
    let allocator =
        overdrive_control_plane::test_default_allocator(store.clone() as Arc<dyn IntentStore>);
    let state = AppState::new(
        store,
        path,
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
        std::net::Ipv4Addr::LOCALHOST,
    );
    let name = ReconcilerName::new("workload-lifecycle").unwrap();
    let control = TargetResource::new("workload/control").unwrap();
    let subject = TargetResource::new("workload/fsync-subject").unwrap();
    runtime_job_input(&state, "control").await;
    runtime_job_input(&state, "fsync-subject").await;
    let now = clock.now();
    let deadline = now + Duration::from_secs(1);
    run_convergence_tick(&state, &name, &control, now, 0, deadline).await.unwrap();
    assert!(running(&obs, "control").await, "seed={seed}: healthy composition must dispatch");
    assert_eq!(driver.live_count(), 1);

    let rows_before = obs.alloc_status_rows().await.unwrap();
    let specs_before: Vec<_> = driver.started_specs().into_iter().map(|spec| spec.alloc).collect();
    let hot_before = state.runtime.loaded_workload_lifecycle_views_for_test(&name).unwrap();
    let stored_before: std::collections::BTreeMap<TargetResource, WorkloadLifecycleView> =
        views.bulk_load("workload-lifecycle").await.unwrap();
    let mut lifecycle = state.lifecycle_events.subscribe();
    views.inject_fsync_failure();
    let failure = run_convergence_tick(&state, &name, &subject, now, 1, deadline).await;
    assert!(matches!(failure, Err(ConvergenceError::ViewPersist(_))), "seed={seed}: {failure:?}");
    assert_eq!(
        obs.alloc_status_rows().await.unwrap(),
        rows_before,
        "seed={seed}: no allocation writes"
    );
    assert_eq!(
        driver.started_specs().into_iter().map(|spec| spec.alloc).collect::<Vec<_>>(),
        specs_before
    );
    assert_eq!(driver.live_count(), 1, "seed={seed}: no subject Driver effect");
    assert!(
        matches!(lifecycle.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Empty)),
        "seed={seed}: no lifecycle publication"
    );
    assert_eq!(state.runtime.loaded_workload_lifecycle_views_for_test(&name).unwrap(), hot_before);
    let stored_after: std::collections::BTreeMap<TargetResource, WorkloadLifecycleView> =
        views.bulk_load("workload-lifecycle").await.unwrap();
    assert_eq!(stored_after, stored_before);

    views.clear_fsync_failure();
    run_convergence_tick(&state, &name, &subject, now, 2, deadline).await.unwrap();
    assert!(running(&obs, "fsync-subject").await, "seed={seed}: same-input recovery must dispatch");
    assert_eq!(driver.live_count(), 2);
    let recovered = state.runtime.loaded_workload_lifecycle_views_for_test(&name).unwrap();
    assert_eq!(recovered.get(&subject).unwrap().observed_generation, 1);
    let durable: std::collections::BTreeMap<TargetResource, WorkloadLifecycleView> =
        views.bulk_load("workload-lifecycle").await.unwrap();
    assert_eq!(durable, recovered, "seed={seed}: recovered View is durable");
    run_convergence_tick(&state, &name, &subject, now, 3, deadline).await.unwrap();
    assert_eq!(
        driver.started_specs().len(),
        2,
        "seed={seed}: no repeated placement after recovery"
    );
}

/// Valid driving-port inputs equivalent to submit followed by a generation
/// advance before placement. The production reconciler alone authors the View
/// and resulting allocation; no stored/hot View or allocation row is seeded.
async fn runtime_job_input(state: &overdrive_control_plane::AppState, id: &str) {
    use overdrive_core::aggregate::{IntentKey, Job, WorkloadIntent};
    use overdrive_core::traits::intent_store::{IntentStore, TxnOp, TxnOutcome};
    let job = Job::from_submit(JobSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/sleep".to_owned(),
            args: vec!["3600".to_owned()],
        }),
    })
    .unwrap();
    let key = IntentKey::for_workload(&job.id);
    let generation = IntentKey::for_workload_generation(&job.id);
    let body = WorkloadIntent::Job(job).archive_for_store().unwrap();
    assert!(matches!(
        state
            .store
            .txn(vec![
                TxnOp::Put {
                    key: key.as_bytes().to_vec().into(),
                    value: body.as_ref().to_vec().into()
                },
                TxnOp::IncrementU64 { key: generation.as_bytes().to_vec().into() },
            ])
            .await
            .unwrap(),
        TxnOutcome::Committed
    ));
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-04. Seed 283001; submit an actual Service, hold its Driver start, and
/// drive accepted AllocStatus/ProbeResult plus public stop. Distinct reconciler
/// names for that workload share one lease until the complete runtime result
/// is consumed. A duplicate remains one pending later turn; hydration then
/// observes latest stop intent. Independent targets remain able to progress.
/// Compare complete target Views, allocation rows and final Driver membership.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn same_workload_reconcilers_share_the_complete_evaluation_lease() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VLL-04 / real Service owner lease and latest intent)"
    );
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-06b. Source-local convergence-owner integration complements the
/// production-server drain tests above: after every active result is consumed,
/// the one exit report equals the locked coalesced pending snapshot. A producer
/// submitting after that snapshot cannot alter the event or execute work.
/// Exercise consumed exit-event write failure through its existing four
/// attempts (50/100/200ms) before join; unread-queue draining is not promised.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn convergence_exit_report_is_the_owner_snapshot_not_final_server_backlog() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VLL-06b / convergence exit snapshot and consumed retry)"
    );
}
