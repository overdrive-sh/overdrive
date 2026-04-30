//! RED scaffold — Step 01-01 of `fix-convergence-loop-not-spawned`.
//!
//! Pins the production-side spawn omission identified in
//! `docs/feature/fix-convergence-loop-not-spawned/bugfix-rca.md`:
//! `run_server_with_obs_and_driver` constructs `AppState` and the
//! axum task but never spawns a tokio task that drains the
//! `EvaluationBroker` and calls `run_convergence_tick`. Submitted
//! jobs never reach `Running`, and `cluster_status.broker.dispatched`
//! permanently reads `0`.
//!
//! This regression test boots the real server end-to-end (axum +
//! rustls + reqwest + `LocalIntentStore` on tempdir) with a `SimClock`
//! and `SimDriver` so the test runs uniformly on macOS and Linux in
//! the default `--features integration-tests` lane (no real kernel,
//! no `ExecDriver` cleanup).
//!
//! It references `ServerConfig.clock` and `ServerConfig.tick_cadence`
//! — fields that do NOT yet exist on `ServerConfig` against current
//! main. The compile failure IS the RED state: the test cannot
//! compile until the production wiring (Step 01-02, GREEN) lands the
//! fields and the broker-driven `tokio::spawn` of the tick loop
//! between `AppState::new` and the listener bind.
//!
//! See `.claude/rules/testing.md` § "RED scaffolds and
//! intentionally-failing commits" for the two-step pattern. Step
//! 01-02 is the GREEN counterpart that wires the production server
//! and makes this test pass.
//!
//! Two assertions, each closing a distinct root cause from the RCA:
//!   1. `broker.dispatched >= 1` after submit + N ticks — kills
//!      Root Cause C (no automated gate on `broker.dispatched`).
//!   2. Submitted job reaches `Running` — kills Roots A + B
//!      together (production omission of the spawn AND
//!      `submit_job` not enqueueing an evaluation).
//!
//! Tier 3 — real axum server, real rustls handshake, real reqwest
//! client. Gated by the `integration-tests` feature at the
//! `tests/integration.rs` entrypoint.

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::api::{
    AllocStatusResponse, ClusterStatus, IdempotencyOutcome, SubmitJobRequest, SubmitJobResponse,
};
use overdrive_control_plane::{ServerConfig, run_server_with_obs_and_driver};
use overdrive_core::aggregate::{DriverInput, ExecInput, JobSpecInput, ResourcesInput};
use overdrive_core::id::NodeId;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use tempfile::TempDir;

// -----------------------------------------------------------------------
// Helpers — duplicated from `observation_empty_rows.rs` /
// `submit_round_trip.rs` per the convention in
// `observation_empty_rows.rs:34-38`. There is no shared helpers module
// across integration scenarios; each file is self-contained so a
// reviewer reads one file end-to-end.
// -----------------------------------------------------------------------

fn client_trusting(ca_pem: &str) -> reqwest::Client {
    let cert = reqwest::Certificate::from_pem(ca_pem.as_bytes()).expect("parse CA PEM");
    reqwest::Client::builder()
        .add_root_certificate(cert)
        .https_only(true)
        .use_rustls_tls()
        .build()
        .expect("build reqwest client")
}

fn read_ca_from_trust_triple(operator_config_dir: &std::path::Path) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let config_path = operator_config_dir.join(".overdrive").join("config");
    let text = std::fs::read_to_string(&config_path)
        .expect(&format!("read trust triple at {}", config_path.display()));
    let doc: toml::Value = toml::from_str(&text).expect("parse trust triple TOML");
    let ca_b64 = doc
        .get("contexts")
        .and_then(toml::Value::as_array)
        .and_then(|arr| {
            arr.iter().find(|c| c.get("name").and_then(toml::Value::as_str) == Some("local"))
        })
        .and_then(|c| c.get("ca"))
        .and_then(toml::Value::as_str)
        .expect("[[contexts]] with name=\"local\" must carry a ca field");
    let ca_bytes = BASE64.decode(ca_b64).expect("base64 decode ca");
    String::from_utf8(ca_bytes).expect("ca PEM is UTF-8")
}

// -----------------------------------------------------------------------
// Regression test — production server boot must spawn the convergence
// loop and dispatch broker-enqueued evaluations.
// -----------------------------------------------------------------------

#[tokio::test]
async fn submitted_job_reaches_running_via_real_server_boot() {
    let temp = TempDir::new().expect("tempdir");
    let data_dir = temp.path().join("data");
    let operator_config_dir = temp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("create operator config dir");

    // SimClock is shared via Arc — `SimClock::clone` shares the
    // underlying counter, but constructing one Arc and cloning it via
    // `clock.clone()` keeps the test code symmetrical with the
    // production wiring (`Arc<dyn Clock>`).
    let clock = Arc::new(SimClock::new());
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));

    // The `clock` and `tick_cadence` field references below are
    // load-bearing for the RED scaffold — they do NOT exist on
    // `ServerConfig` against current main. The compile failure here
    // (`unknown field clock`, `unknown field tick_cadence`) IS the
    // RED state. Step 01-02 lands these fields plus the broker-driven
    // `tokio::spawn` that makes this test pass.
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind addr"),
        data_dir,
        operator_config_dir: operator_config_dir.clone(),
        // Control-plane integration tests don't start real workloads;
        // bypass the cgroup pre-flight so they run uniformly on macOS
        // and on Linux without delegation.
        allow_no_cgroups: true,
        tick_cadence: Duration::from_millis(100),
        clock: clock.clone(),
    };

    let handle = run_server_with_obs_and_driver(config, Arc::clone(&obs), Arc::clone(&driver))
        .await
        .expect("server boot");
    let bound = handle.local_addr().await.expect("bound addr");
    let ca_pem = read_ca_from_trust_triple(&operator_config_dir);
    let client = client_trusting(&ca_pem);

    // Submit `payments` job via the real HTTP surface — same shape as
    // `submit_round_trip.rs`. Asserts 200 + Inserted to confirm the
    // submit succeeded and we are now waiting on the convergence loop.
    let submit_url = format!("https://localhost:{}/v1/jobs", bound.port());
    let spec = JobSpecInput {
        id: "payments".to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    };
    let resp = client
        .post(&submit_url)
        .json(&SubmitJobRequest { spec })
        .send()
        .await
        .expect("POST /v1/jobs");
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "submit must return 200");
    let body: SubmitJobResponse = resp.json().await.expect("decode SubmitJobResponse");
    assert_eq!(
        body.outcome,
        IdempotencyOutcome::Inserted,
        "fresh submit must report `outcome = Inserted`; got {:?}",
        body.outcome,
    );

    // Drive the SimClock forward in 100ms steps, yielding to the tokio
    // runtime between ticks so the spawned convergence task gets
    // scheduling time. `SimClock::tick(&self, Duration)` is sync per
    // `crates/overdrive-sim/src/adapters/clock.rs:55`.
    for _ in 0..30 {
        clock.tick(Duration::from_millis(100));
        tokio::task::yield_now().await;
    }

    // Assertion 1 — kills Root Cause C (no gate on broker.dispatched).
    // Under the fix, every drained evaluation increments the dispatched
    // counter; under current main, the counter is permanently 0.
    let info_url = format!("https://localhost:{}/v1/cluster/info", bound.port());
    let info: ClusterStatus = client
        .get(&info_url)
        .send()
        .await
        .expect("GET /v1/cluster/info")
        .json()
        .await
        .expect("decode ClusterStatus");
    assert!(
        info.broker.dispatched >= 1,
        "broker.dispatched must advance under steady-state traffic; got {}",
        info.broker.dispatched,
    );

    // Assertion 2 — kills Roots A + B together. Under the fix the
    // submit enqueues an evaluation, the spawned loop drains it, the
    // job-lifecycle reconciler runs, and the SimDriver advances the
    // alloc to Running. Under current main the alloc is never created.
    let allocs_url = format!("https://localhost:{}/v1/allocs?job=payments", bound.port());
    let allocs: AllocStatusResponse = client
        .get(&allocs_url)
        .send()
        .await
        .expect("GET /v1/allocs?job=payments")
        .json()
        .await
        .expect("decode AllocStatusResponse");
    // `AllocState::Display` renders the canonical lowercase form
    // (`"running"`) — see `overdrive-core::traits::observation_store::AllocState::fmt`.
    // The Step 01-01 RED scaffold mistakenly asserted on the
    // capitalised form; the case fix is a test-bug correction, not a
    // weakening of the assertion (the assertion still pins
    // `state.to_string() == "running"` AND `job_id == "payments"`).
    assert!(
        allocs.rows.iter().any(|a| a.job_id == "payments"
            && matches!(a.state, overdrive_control_plane::api::AllocStateWire::Running)),
        "submitted job must reach Running via the production convergence loop; \
         got {:?}",
        allocs.rows,
    );

    handle.shutdown(Duration::from_secs(1)).await;
}
