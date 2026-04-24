//! Integration tests for `POST /v1/jobs` idempotency + conflict contract
//! — step 03-04.
//!
//! Tightens the contract pinned in `submit_round_trip.rs` beyond what
//! step 03-01's tests already cover:
//!
//! 1. **§4.9** "`IntentStore` contains only one entry at the intent key" —
//!    byte-identical re-submit must not produce a second stored copy or
//!    drift the stored bytes.
//! 2. **§4.10** "`IntentStore` still carries the original spec under that
//!    intent key" after a 409 — verified through the live HTTP surface
//!    via `GET /v1/jobs/{id}` rather than a back-door redb read, so the
//!    invariant is phrased in terms an operator can observe.
//! 3. Triple byte-identical re-submit is stable — N submissions of the
//!    same spec return the same `commit_index` every time, not just the
//!    first pair.
//! 4. The 409 `ErrorBody.message` names the intent key path (`jobs/...`)
//!    so an operator can identify *which* key conflicted from the wire
//!    response alone.
//!
//! These tests are additive — they do NOT replace the happy-path or
//! bad-spec coverage in `submit_round_trip.rs`.
//!
//! ADR references:
//! - ADR-0015 §4 — idempotent re-submit + 409 contract, Phase 1
//!   LWW / read-before-write note.
//! - ADR-0011 — rkyv-archived bytes deterministic per Job.
//! - ADR-0008 — `ErrorBody` shape `{error, message, field}`.
//!
//! Tier 3 — real redb file on `tempfile`, real axum server, real rustls
//! handshake, real reqwest. Gated by the `integration-tests` feature at
//! the `tests/integration.rs` entrypoint.

use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use overdrive_control_plane::api::{
    ErrorBody, JobDescription, SubmitJobRequest, SubmitJobResponse,
};
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server};
use overdrive_core::aggregate::{IntentKey, Job, JobSpecInput};
use overdrive_core::id::JobId;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

// -----------------------------------------------------------------------
// Helpers — mirror the harness used in `submit_round_trip.rs` and
// `describe_round_trip.rs`. Duplicated rather than extracted so each
// scenario file remains self-contained and readable; if a fourth
// scenario file appears that needs the same shape, promote to a shared
// `integration/common.rs` module at that point.
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

fn read_ca_from_trust_triple(data_dir: &std::path::Path) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let config_path = data_dir.join(".overdrive").join("config");
    let text = std::fs::read_to_string(&config_path)
        .expect(&format!("read trust triple at {}", config_path.display()));
    // ADR-0019 canonical TOML shape: `current-context = "local"` +
    // `[[contexts]]` array-of-tables, each entry carrying `name`,
    // `endpoint`, and the base64-PEM trust triple.
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
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind addr"),
        data_dir: tmp.path().to_path_buf(),
    };
    let handle = run_server(config).await.expect("run_server");
    let bound = handle.local_addr().await.expect("bound addr");
    let ca_pem = read_ca_from_trust_triple(tmp.path());
    (handle, bound, tmp, ca_pem)
}

/// Back-door read against the same redb file the server is committing
/// to. Used to verify the §4.9 "only one entry" invariant — the stored
/// bytes at `key` must be *exactly* the rkyv archive of the submitted
/// spec, with no drift across re-submissions.
async fn read_intent_key_from_store(data_dir: &std::path::Path, key: &[u8]) -> Option<Bytes> {
    let path = data_dir.join("intent.redb");
    assert!(path.exists(), "expected redb file at {}; found none", path.display());
    let store = LocalIntentStore::open(&path).expect("open LocalIntentStore for back-door read");
    store.get(key).await.expect("back-door get")
}

fn payments_spec() -> JobSpecInput {
    JobSpecInput {
        id: "payments".to_owned(),
        replicas: 3,
        cpu_milli: 500,
        memory_bytes: 536_870_912, // 512 MiB
    }
}

fn payments_spec_alt_replicas() -> JobSpecInput {
    // Same JobId, different replicas — semantically different spec at
    // the same intent key; rkyv bytes differ; must 409.
    JobSpecInput {
        id: "payments".to_owned(),
        replicas: 7,
        cpu_milli: 500,
        memory_bytes: 536_870_912,
    }
}

// -----------------------------------------------------------------------
// AC (a) — byte-identical re-submit returns the original commit_index
// and the store still contains exactly one entry at the intent key
// (test-scenarios §4.9, ADR-0015 §4 idempotent success).
// -----------------------------------------------------------------------

#[tokio::test]
async fn byte_identical_resubmit_returns_original_commit_index_unchanged() {
    let (handle, bound, tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);
    let url = format!("https://localhost:{}/v1/jobs", bound.port());

    let spec = payments_spec();

    // First submit — captures the ambient commit_index N.
    let first: SubmitJobResponse = client
        .post(&url)
        .json(&SubmitJobRequest { spec: spec.clone() })
        .send()
        .await
        .expect("first submit")
        .json()
        .await
        .expect("decode first response");

    // Second submit — byte-identical spec; must return the SAME
    // commit_index, not the next one.
    let second: SubmitJobResponse = client
        .post(&url)
        .json(&SubmitJobRequest { spec: spec.clone() })
        .send()
        .await
        .expect("second submit")
        .json()
        .await
        .expect("decode second response");

    assert_eq!(
        second.commit_index, first.commit_index,
        "byte-identical re-submit must return the ORIGINAL commit_index; \
         got first = {}, second = {}",
        first.commit_index, second.commit_index,
    );
    assert_eq!(second.job_id, first.job_id, "job_id must echo canonical JobId on both submits");

    // Shut the server down before the back-door read so the redb write
    // handle is released.
    handle.shutdown(Duration::from_secs(2)).await;

    // §4.9 tail: the IntentStore contains only one entry at the intent
    // key — i.e. the stored bytes are byte-equal to the rkyv archive of
    // the canonical spec, unchanged by the second submission.
    let job_id = JobId::new("payments").expect("parse payments JobId");
    let key = IntentKey::for_job(&job_id);
    let persisted = read_intent_key_from_store(tmp.path(), key.as_bytes())
        .await
        .expect("jobs/payments must be populated after successful submit");

    let expected_job = Job::from_spec(spec).expect("canonical spec constructs a Job");
    let expected_bytes =
        rkyv::to_bytes::<rkyv::rancor::Error>(&expected_job).expect("rkyv archive of expected Job");
    assert_eq!(
        persisted.as_ref(),
        expected_bytes.as_ref(),
        "stored bytes must equal the canonical rkyv archive of the original spec; \
         a re-submit must not rewrite / re-append / mutate the stored value",
    );
}

// -----------------------------------------------------------------------
// AC (b) — a different spec at the same JobId returns 409 Conflict with
// `ErrorBody.error = "conflict"` (test-scenarios §4.10).
// -----------------------------------------------------------------------

#[tokio::test]
async fn different_spec_at_existing_key_returns_409_conflict_with_error_body() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);
    let url = format!("https://localhost:{}/v1/jobs", bound.port());

    // Prime the store with the canonical payments spec.
    let primed = client
        .post(&url)
        .json(&SubmitJobRequest { spec: payments_spec() })
        .send()
        .await
        .expect("prime submit");
    assert_eq!(primed.status(), reqwest::StatusCode::OK, "priming submit must be 200 OK");

    // Second submit: same id, different replicas. Must be 409.
    let conflict = client
        .post(&url)
        .json(&SubmitJobRequest { spec: payments_spec_alt_replicas() })
        .send()
        .await
        .expect("conflicting submit");

    assert_eq!(
        conflict.status(),
        reqwest::StatusCode::CONFLICT,
        "different spec at occupied key must be HTTP 409 Conflict",
    );

    let body: ErrorBody = conflict.json().await.expect("decode ErrorBody");
    assert_eq!(body.error, "conflict", "error kind must be 'conflict' per ADR-0015 enumeration");
    assert!(
        !body.message.is_empty(),
        "409 ErrorBody must carry a non-empty message describing the conflict",
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

// -----------------------------------------------------------------------
// AC (c) — the IntentStore content is unchanged after a 409: Describe
// returns the ORIGINAL replicas, not the rejected new value.
// (test-scenarios §4.10 "IntentStore still carries the original spec")
// -----------------------------------------------------------------------

#[tokio::test]
async fn intent_store_unchanged_after_conflict_attempt() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);

    let submit_url = format!("https://localhost:{}/v1/jobs", bound.port());
    let describe_url = format!("https://localhost:{}/v1/jobs/payments", bound.port());

    // Prime with replicas = 3 (canonical).
    let primed = client
        .post(&submit_url)
        .json(&SubmitJobRequest { spec: payments_spec() })
        .send()
        .await
        .expect("prime submit");
    assert_eq!(primed.status(), reqwest::StatusCode::OK);

    // Capture the commit_index at the moment of the original submit so
    // we can assert the 409 did not advance it (read-before-write must
    // NOT call `put` on the conflict branch).
    let commit_at_prime: SubmitJobResponse = primed.json().await.expect("decode prime response");

    // Reject with replicas = 7 — must be 409.
    let conflict = client
        .post(&submit_url)
        .json(&SubmitJobRequest { spec: payments_spec_alt_replicas() })
        .send()
        .await
        .expect("conflicting submit");
    assert_eq!(conflict.status(), reqwest::StatusCode::CONFLICT);

    // Describe the key — must still carry the ORIGINAL spec, not the
    // rejected alternate. This is the operator-visible §4.10 invariant.
    let described = client.get(&describe_url).send().await.expect("GET /v1/jobs/payments");
    assert_eq!(
        described.status(),
        reqwest::StatusCode::OK,
        "describe must succeed after a conflict — the original entry is still there",
    );
    let desc_body: JobDescription = described.json().await.expect("decode JobDescription");
    assert_eq!(
        desc_body.spec.replicas, 3,
        "after a 409, the stored spec must remain the ORIGINAL (replicas = 3), \
         not the rejected replacement (replicas = 7); got {:?}",
        desc_body.spec,
    );
    assert_eq!(
        desc_body.commit_index, commit_at_prime.commit_index,
        "commit_index must not advance on a 409 — the conflict branch must NOT call put",
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

// -----------------------------------------------------------------------
// AC (d) — triple byte-identical re-submit is stable: all three return
// the same commit_index, not just the first pair.
// -----------------------------------------------------------------------

#[tokio::test]
async fn triple_resubmit_byte_identical_all_return_same_commit_index() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);
    let url = format!("https://localhost:{}/v1/jobs", bound.port());

    let spec = payments_spec();

    let mut indices = Vec::with_capacity(3);
    for attempt in 0..3 {
        let resp: SubmitJobResponse = client
            .post(&url)
            .json(&SubmitJobRequest { spec: spec.clone() })
            .send()
            .await
            .expect(&format!("submit attempt {attempt}"))
            .json()
            .await
            .expect(&format!("decode response attempt {attempt}"));
        indices.push(resp.commit_index);
    }

    // All three commit indices must be equal — a handler that writes on
    // every submission would drift the index on attempts 2 and 3.
    assert_eq!(
        indices[0], indices[1],
        "commit_index must match on submits 1 and 2; got {indices:?}",
    );
    assert_eq!(
        indices[1], indices[2],
        "commit_index must match on submits 2 and 3 — idempotency must \
         be stable across N re-submits, not just 2; got {indices:?}",
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

// -----------------------------------------------------------------------
// AC (e) — the 409 ErrorBody.message names the intent-key path
// (`jobs/payments`) so an operator can identify which key conflicted
// without reading server logs.
// -----------------------------------------------------------------------

#[tokio::test]
async fn conflict_message_names_intent_key_path() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);
    let url = format!("https://localhost:{}/v1/jobs", bound.port());

    // Prime, then conflict.
    let primed = client
        .post(&url)
        .json(&SubmitJobRequest { spec: payments_spec() })
        .send()
        .await
        .expect("prime submit");
    assert_eq!(primed.status(), reqwest::StatusCode::OK);

    let conflict = client
        .post(&url)
        .json(&SubmitJobRequest { spec: payments_spec_alt_replicas() })
        .send()
        .await
        .expect("conflicting submit");
    assert_eq!(conflict.status(), reqwest::StatusCode::CONFLICT);

    let body: ErrorBody = conflict.json().await.expect("decode ErrorBody");

    // The message must name the canonical intent-key path — the operator
    // needs to be able to tell WHICH key conflicted from the body alone.
    // `jobs/payments` is the canonical form per `IntentKey::for_job`.
    assert!(
        body.message.contains("jobs/payments"),
        "conflict ErrorBody.message must name the intent-key path \
         (substring 'jobs/payments'); got {:?}",
        body.message,
    );

    handle.shutdown(Duration::from_secs(2)).await;
}
