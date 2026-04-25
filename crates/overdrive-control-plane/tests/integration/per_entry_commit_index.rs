//! Integration test pinning the per-entry `commit_index` contract.
//!
//! This is the user-story regression test for the
//! `fix-commit-index-per-entry` bug — verbatim from
//! `docs/feature/phase-1-control-plane-core/discuss/user-stories.md:234`:
//!
//! > Ana submits `payments.toml`, gets commit_index 17. She submits a
//! > (different) `frontend.toml`, gets commit_index 18. She describes
//! > `payments` — the commit_index returned is still 17 (the submit's),
//! > not the latest store index. The commit counter is monotonic; reads
//! > do not invent a new one.
//!
//! `JobDescription::commit_index` is rustdoc-documented as "the commit
//! index at which it was written" (`api.rs:50-52`) — explicitly
//! per-entry. The current handler (HEAD as of this RED scaffold) returns
//! `state.store.commit_index()` which is the live store-wide counter at
//! describe-time, not the per-entry index — so after an intervening
//! write to a *different* key, describing the original entry returns the
//! wrong index.
//!
//! Per `.claude/rules/testing.md` §"RED scaffolds and intentionally-failing
//! commits", this test is intentionally RED at this commit. Step 01-02
//! lands the trait + store + handler change that makes it GREEN.
//!
//! Tier 3 — real redb file on `tempfile`, real axum server, real rustls
//! handshake, real reqwest. Gated by the `integration-tests` feature at
//! the `tests/integration.rs` entrypoint.

use std::net::SocketAddr;
use std::time::Duration;

use overdrive_control_plane::api::{JobDescription, SubmitJobRequest, SubmitJobResponse};
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server};
use overdrive_core::aggregate::JobSpecInput;
use tempfile::TempDir;

// -----------------------------------------------------------------------
// Helpers — duplicated from the other `tests/integration/*.rs` files
// per the local convention that each scenario file is self-contained.
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

/// Spawn a server. `data_dir` and `operator_config_dir` are SEPARATE
/// subdirectories of the tempdir per `fix-cli-cannot-reach-control-plane`
/// Step 01-02 — see the canonical `concurrent_submit_toctou.rs::spawn_server`
/// shape this is cloned from.
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
    };
    let handle = run_server(config).await.expect("run_server");
    let bound = handle.local_addr().await.expect("bound addr");
    let ca_pem = read_ca_from_trust_triple(&operator_config_dir);
    (handle, bound, tmp, ca_pem)
}

// -----------------------------------------------------------------------
// Two distinct specs — different `id` AND different `replicas`. Distinct
// `id` keeps them at different IntentStore keys (so the second submit is
// a fresh insert, not idempotent re-submit at the same key); distinct
// `replicas` makes their rkyv archive bytes distinct as a belt-and-braces
// guard against any future canonical-form change that might collapse
// equal-id specs to the same archive.
// -----------------------------------------------------------------------

fn payments_spec() -> JobSpecInput {
    JobSpecInput {
        id: "payments".to_owned(),
        replicas: 3,
        cpu_milli: 500,
        memory_bytes: 536_870_912,
    }
}

fn frontend_spec() -> JobSpecInput {
    JobSpecInput {
        id: "frontend".to_owned(),
        replicas: 7,
        cpu_milli: 750,
        memory_bytes: 1_073_741_824,
    }
}

// -----------------------------------------------------------------------
// User-story §234 verbatim:
//
//   Ana submits payments.toml, gets commit_index N (= idx_a).
//   She submits a (different) frontend.toml, gets commit_index N+1 (= idx_b).
//   She describes payments — the commit_index returned is still idx_a,
//   not idx_b.
//
// The current `describe_job` handler returns `state.store.commit_index()`
// at describe-time — i.e. the live counter, which has advanced past
// `idx_a` after frontend's write. So `idx_a_after != idx_a` in HEAD,
// failing the per-entry contract `JobDescription` rustdoc documents.
// -----------------------------------------------------------------------

#[tokio::test]
async fn describe_returns_index_at_which_entry_was_written() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);

    let submit_url = format!("https://localhost:{}/v1/jobs", bound.port());

    // 1. Ana submits A (payments). Capture the commit_index returned by
    //    submit — this is the index at which A was written.
    let submit_a = client
        .post(&submit_url)
        .json(&SubmitJobRequest { spec: payments_spec() })
        .send()
        .await
        .expect("POST /v1/jobs (A)");
    assert_eq!(submit_a.status(), reqwest::StatusCode::OK);
    let submit_a_body: SubmitJobResponse =
        submit_a.json().await.expect("decode SubmitJobResponse for A");
    let idx_a = submit_a_body.commit_index;

    // 2. Ana submits a DIFFERENT spec B (frontend). This advances the
    //    live store counter past idx_a. Distinct key + distinct rkyv
    //    archive so neither idempotent-re-submit nor conflict path
    //    fires.
    let submit_b = client
        .post(&submit_url)
        .json(&SubmitJobRequest { spec: frontend_spec() })
        .send()
        .await
        .expect("POST /v1/jobs (B)");
    assert_eq!(submit_b.status(), reqwest::StatusCode::OK);
    let submit_b_body: SubmitJobResponse =
        submit_b.json().await.expect("decode SubmitJobResponse for B");
    let idx_b = submit_b_body.commit_index;

    // The two writes must produce distinct, advancing indices — this
    // pins global monotonicity, the existing contract.
    assert!(
        idx_b > idx_a,
        "after writing two distinct specs, the second submit's commit_index \
         must strictly exceed the first; got idx_a={idx_a}, idx_b={idx_b}",
    );

    // 3. Ana describes A. The returned commit_index is the per-entry
    //    contract — the index at which A was committed, NOT the live
    //    store counter (which has advanced past idx_a thanks to B's
    //    write).
    let describe_url =
        format!("https://localhost:{}/v1/jobs/{}", bound.port(), submit_a_body.job_id);
    let describe_a = client.get(&describe_url).send().await.expect("GET /v1/jobs/{a_id}");
    assert_eq!(describe_a.status(), reqwest::StatusCode::OK);
    let description_a: JobDescription =
        describe_a.json().await.expect("decode JobDescription for A");
    let idx_a_after = description_a.commit_index;

    // The per-entry contract: describe(A) returns A's write index, not
    // the live counter. A handler that returns `state.store.commit_index()`
    // at describe-time fails this — it would return idx_b (or higher),
    // which is the bug RCA §WHY 1A documents.
    assert_eq!(
        idx_a_after, idx_a,
        "RED scaffold (Step 01-01) — describe(A).commit_index must equal \
         A's write index ({idx_a}) per the JobDescription rustdoc contract \
         \"the commit index at which it was written\"; got {idx_a_after}, \
         which is the live store counter (idx_b = {idx_b}) — this is the \
         per-entry contract violation documented in user-stories.md:234 \
         and RCA §WHY 1A. Step 01-02 lands the GREEN trait + handler change.",
    );

    handle.shutdown(Duration::from_secs(2)).await;
}
