//! Step 02-04 / Slice 3B scenario 3.11 —
//! `stop_on_unknown_job_returns_404`.
//!
//! `POST /v1/jobs/<id>/stop` for an `<id>` that was never submitted
//! must return HTTP 404 with the canonical `ErrorBody { error,
//! message, field }` shape. Per ADR-0027 + ADR-0015.
//!
//! Default-lane (in-memory). Error-path scenario.

#![allow(clippy::expect_used, clippy::expect_fun_call, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::time::Duration;

use overdrive_control_plane::api::ErrorBody;
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server};
use overdrive_host::RealCgroupFs;
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
        // `tick_cadence` + `clock` default per
        // `fix-convergence-loop-not-spawned` Step 01-02.
        dataplane_override: Some(std::sync::Arc::new(
            overdrive_sim::adapters::dataplane::SimDataplane::new(),
        )),
        // ADR-0061 § 1 (step 01-03): default `dataplane` is now veth-named
        // (`ovd-veth-cli` absent in the test VM); name `lo` so the boot
        // `host_ipv4` resolution succeeds. SimDataplane skips XDP attach.
        dataplane: Some(super::dataplane_lo::lo_dataplane_config()),
        ..Default::default()
    };
    let handle =
        run_server(config, std::sync::Arc::new(RealCgroupFs::new())).await.expect("run_server");
    let bound = handle.local_addr().await.expect("bound addr");
    let ca_pem = read_ca_from_trust_triple(&operator_config_dir);
    (handle, bound, tmp, ca_pem)
}

#[tokio::test]
async fn stop_on_unknown_job_returns_404() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);
    let stop_url = format!("https://localhost:{}/v1/jobs/never-submitted/stop", bound.port());

    let resp = client.post(&stop_url).send().await.expect("POST stop");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::NOT_FOUND,
        "stop on unknown job must return 404; got {}",
        resp.status()
    );

    let body: ErrorBody = resp.json().await.expect("decode ErrorBody");
    assert_eq!(body.error, "not_found", "404 ErrorBody.error must be 'not_found'");

    handle.shutdown(Duration::from_secs(2)).await;
}
