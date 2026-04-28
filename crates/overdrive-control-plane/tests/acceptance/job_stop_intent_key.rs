//! Step 02-04 / Slice 3B scenario 3.12 —
//! `stop_writes_separate_intent_key_preserving_spec`.
//!
//! After `POST /v1/jobs/<id>/stop`, the `IntentStore` must hold BOTH
//! the original `IntentKey::for_job(<id>)` (unchanged byte-for-byte)
//! AND `IntentKey::for_job_stop(<id>)`. The job spec is preserved for
//! audit / rollback / debugging; the stop signal is recorded as a
//! separate key. Per ADR-0027.
//!
//! Default-lane (in-memory). Enters via the in-process server
//! fixture's HTTP client; asserts at the `IntentStore` back-door read
//! boundary.

#![allow(clippy::expect_used, clippy::expect_fun_call, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::time::Duration;

use overdrive_control_plane::api::{IdempotencyOutcome, SubmitJobRequest, SubmitJobResponse};
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server};
use overdrive_core::aggregate::{IntentKey, JobSpecInput};
use overdrive_core::id::JobId;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

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

async fn spawn_server() -> (ServerHandle, SocketAddr, TempDir, String) {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("create operator config dir");
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind addr"),
        data_dir,
        operator_config_dir: operator_config_dir.clone(),
        // Default-lane acceptance tests don't start real workloads;
        // bypass the cgroup pre-flight so this test runs uniformly
        // on macOS and Linux without delegation.
        allow_no_cgroups: true,
        // `tick_cadence` + `clock` default per
        // `fix-convergence-loop-not-spawned` Step 01-02.
        ..Default::default()
    };
    let handle = run_server(config).await.expect("run_server");
    let bound = handle.local_addr().await.expect("bound addr");
    let ca_pem = read_ca_from_trust_triple(&operator_config_dir);
    (handle, bound, tmp, ca_pem)
}

fn payments_spec() -> JobSpecInput {
    JobSpecInput {
        id: "payments".to_owned(),
        replicas: 1,
        cpu_milli: 100,
        memory_bytes: 256 * 1024 * 1024,
    }
}

#[tokio::test]
async fn stop_writes_separate_intent_key_preserving_spec() {
    let (handle, bound, tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);
    let submit_url = format!("https://localhost:{}/v1/jobs", bound.port());
    let stop_url = format!("https://localhost:{}/v1/jobs/payments/stop", bound.port());

    // Submit a job first.
    let resp = client
        .post(&submit_url)
        .json(&SubmitJobRequest { spec: payments_spec() })
        .send()
        .await
        .expect("POST /v1/jobs");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let submit_body: SubmitJobResponse = resp.json().await.expect("decode submit body");
    assert_eq!(submit_body.outcome, IdempotencyOutcome::Inserted);

    // Capture the expected spec bytes by re-archiving from the same
    // input — the original handler stores rkyv archive of `Job::from_spec`,
    // which is byte-deterministic.
    let job_id = JobId::new("payments").expect("parse job id");
    let job_key = IntentKey::for_job(&job_id);
    let job = overdrive_core::aggregate::Job::from_spec(payments_spec()).expect("Job::from_spec");
    let expected_spec_bytes =
        rkyv::to_bytes::<rkyv::rancor::Error>(&job).expect("rkyv archive expected Job");

    // Stop the job.
    let stop_resp = client.post(&stop_url).send().await.expect("POST stop");
    assert_eq!(
        stop_resp.status(),
        reqwest::StatusCode::OK,
        "stop on existing job must return 200; got {}",
        stop_resp.status()
    );

    // Shut server down so back-door read sees a clean store
    // (`DatabaseAlreadyOpen` is the redb error when a writer still
    // holds an exclusive lock on the file).
    handle.shutdown(Duration::from_secs(2)).await;

    let data_dir = tmp.path().join("data");
    let store_path = data_dir.join("intent.redb");
    let store = LocalIntentStore::open(&store_path).expect("open store post-stop");

    // Original job spec must be preserved byte-for-byte.
    let post_stop_spec_bytes = store
        .get(job_key.as_bytes())
        .await
        .expect("read job key post-stop")
        .expect("job key still populated after stop");
    assert_eq!(
        post_stop_spec_bytes.as_ref(),
        expected_spec_bytes.as_ref(),
        "POST .../stop must preserve the original job spec byte-for-byte"
    );

    // Stop intent key must now be present.
    let stop_key = IntentKey::for_job_stop(&job_id);
    let stop_bytes = store
        .get(stop_key.as_bytes())
        .await
        .expect("read stop key")
        .expect("stop intent key must be populated after POST .../stop");
    // Stop record carries no payload (the key's existence IS the signal).
    // We just assert that the stop key was written; the value can be
    // empty bytes.
    let _ = stop_bytes;
}
