//! Step 02-04 / Slice 3B scenario 3.10 —
//! `stop_on_already_stopped_job_returns_already_stopped_outcome`.
//!
//! After a successful `POST /v1/jobs/<id>/stop`, a second call with
//! the same `<id>` must return 200 OK with `outcome =
//! "already_stopped"` rather than failing or re-stopping.
//!
//! Default-lane (in-memory). Per ADR-0027 the `put_if_absent` on the
//! `IntentKey::for_job_stop` key gives idempotent semantics for free.

#![allow(clippy::expect_used, clippy::expect_fun_call, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::time::Duration;

use overdrive_control_plane::api::{IdempotencyOutcome, SubmitJobRequest, SubmitJobResponse};
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server};
use overdrive_core::aggregate::{DriverInput, ExecInput, JobSpecInput, ResourcesInput};
use serde::Deserialize;
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
        // on macOS (no cgroup v2) and Linux without delegation.
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
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    }
}

/// Local mirror of the wire shape `POST /v1/jobs/<id>/stop` returns.
/// Defined here in the test to pin the contract — the production type
/// lives in `overdrive_control_plane::api` (`StopJobResponse`).
#[derive(Debug, Deserialize)]
struct StopJobResponseBody {
    job_id: String,
    outcome: String,
}

#[tokio::test]
async fn stop_on_already_stopped_job_returns_already_stopped_outcome() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);
    let submit_url = format!("https://localhost:{}/v1/jobs", bound.port());
    let stop_url = format!("https://localhost:{}/v1/jobs/payments/stop", bound.port());

    // Submit the job first.
    let submit_resp = client
        .post(&submit_url)
        .json(&SubmitJobRequest { spec: payments_spec() })
        .send()
        .await
        .expect("POST /v1/jobs");
    assert_eq!(submit_resp.status(), reqwest::StatusCode::OK);
    let submit_body: SubmitJobResponse = submit_resp.json().await.expect("decode submit");
    assert_eq!(submit_body.outcome, IdempotencyOutcome::Inserted);

    // First stop — must succeed with outcome=stopped.
    let resp_first = client.post(&stop_url).send().await.expect("first stop");
    assert_eq!(resp_first.status(), reqwest::StatusCode::OK);
    let body_first: StopJobResponseBody = resp_first.json().await.expect("decode first stop body");
    assert_eq!(body_first.job_id, "payments");
    assert_eq!(body_first.outcome, "stopped", "first stop must report outcome=stopped");

    // Second stop — must succeed with outcome=already_stopped.
    let resp_second = client.post(&stop_url).send().await.expect("second stop");
    assert_eq!(resp_second.status(), reqwest::StatusCode::OK);
    let body_second: StopJobResponseBody =
        resp_second.json().await.expect("decode second stop body");
    assert_eq!(body_second.job_id, "payments");
    assert_eq!(
        body_second.outcome, "already_stopped",
        "second stop must report outcome=already_stopped"
    );

    handle.shutdown(Duration::from_secs(2)).await;
}
