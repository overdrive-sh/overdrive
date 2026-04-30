//! Integration tests for `POST /v1/jobs` — step 03-01.
//!
//! Proves the Phase 1 `submit_job` handler round-trip:
//!
//! 1. Validates the spec via `Job::from_spec` (errors map to HTTP 400).
//! 2. Archives via `rkyv::to_bytes::<rancor::Error>`.
//! 3. Commits through `IntentStore::put_if_absent` at `jobs/<JobId>`.
//! 4. Returns `{job_id, spec_digest, outcome}` with
//!    `outcome == Inserted` on a fresh insert (per ADR-0020 the
//!    `commit_index` field is dropped).
//! 5. Idempotency: byte-identical re-submission returns the same
//!    `spec_digest` and `outcome == Unchanged` (ADR-0015 §4 amended
//!    by ADR-0020).
//! 6. Conflict: a different spec at the same intent key returns HTTP
//!    409 with an `ErrorBody` (ADR-0015 §4).
//!
//! Tier 3 — real redb file on `tempfile`, real axum server, real rustls
//! handshake, real reqwest. Gated by the `integration-tests` feature at
//! the `tests/integration.rs` entrypoint.

use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use overdrive_control_plane::api::{
    ErrorBody, IdempotencyOutcome, SubmitJobRequest, SubmitJobResponse,
};
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput,
};
use overdrive_core::id::JobId;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

// -----------------------------------------------------------------------
// Helpers — spawn a server, mint a reqwest client trusting the ephemeral
// CA, and read the intent key straight out of the redb file the server
// writes to. The back-door read opens the SAME redb path the server is
// committing to; since redb supports concurrent read transactions even
// with a live writer, this is a legitimate out-of-process observation.
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

/// Spawn a server. `data_dir` and `operator_config_dir` are SEPARATE
/// subdirectories of the tempdir per `fix-cli-cannot-reach-control-plane`
/// Step 01-02: `data_dir` is the redb storage root; the trust triple
/// goes under `operator_config_dir`. Callers that read the redb file
/// back-door derive the `data_dir` as `tmp.path().join("data")` (see
/// the `data_dir_under` helper).
async fn spawn_server() -> (ServerHandle, SocketAddr, TempDir, String) {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = data_dir_under(tmp.path());
    let operator_config_dir = operator_config_dir_under(tmp.path());
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("create operator config dir");
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind addr"),
        data_dir,
        operator_config_dir: operator_config_dir.clone(),
        // Control-plane integration tests don't start real workloads;
        // bypass the cgroup pre-flight so they run uniformly on macOS
        // and on Linux without delegation.
        allow_no_cgroups: true,
        // `tick_cadence` and `clock` default to
        // `DEFAULT_TICK_CADENCE` (100ms) and `Arc::new(SystemClock)`.
        // Per `fix-convergence-loop-not-spawned` Step 01-02: the
        // production server now spawns a convergence-tick loop. This
        // test does not assert on convergence outcomes — its
        // assertions ride on the IntentStore round-trip through the
        // submit_job handler — and shutdown ordering in
        // `ServerHandle::shutdown` cancels the convergence task
        // before axum graceful so any in-flight ticks land cleanly.
        ..Default::default()
    };
    let handle = run_server(config).await.expect("run_server");
    let bound = handle.local_addr().await.expect("bound addr");
    let ca_pem = read_ca_from_trust_triple(&operator_config_dir);
    (handle, bound, tmp, ca_pem)
}

/// Resolve the redb storage root for a tempdir-rooted server fixture.
/// Mirrors the layout `spawn_server` writes — kept in one place so the
/// back-door redb readers in this file cannot drift from `spawn_server`.
fn data_dir_under(tmp: &std::path::Path) -> std::path::PathBuf {
    tmp.join("data")
}

/// Resolve the operator config base directory for a tempdir-rooted
/// server fixture. Pair of [`data_dir_under`].
fn operator_config_dir_under(tmp: &std::path::Path) -> std::path::PathBuf {
    tmp.join("conf")
}

/// Back-door read: open a SECOND `LocalIntentStore` at the same redb file the
/// server is committing to, and return the raw bytes at `key` (if any).
///
/// `LocalIntentStore::open` is safe under multi-handle read access — redb
/// permits concurrent readers alongside a live writer, which is what we
/// need here. We close this handle on drop by letting it fall out of
/// scope.
async fn read_intent_key_from_store(data_dir: &std::path::Path, key: &[u8]) -> Option<Bytes> {
    // The server writes its redb file at `<data_dir>/intent.redb` — see
    // step 01-04 / `overdrive-store-local` usage in `run_server`. If the
    // name drifts, fail fast with a clear message so the test surface is
    // the authoritative record of the path.
    let path = data_dir.join("intent.redb");
    assert!(path.exists(), "expected redb file at {}; found none", path.display());
    let store = LocalIntentStore::open(&path).expect("open LocalIntentStore for back-door read");
    // Per ADR-0020 `IntentStore::get` returns `Option<Bytes>`.
    store.get(key).await.expect("back-door get")
}

fn payments_spec() -> JobSpecInput {
    JobSpecInput {
        id: "payments".to_owned(),
        replicas: 3,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 536_870_912 }, // 512 MiB
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    }
}

fn payments_spec_alt() -> JobSpecInput {
    // Different replicas — same id/key, different spec -> should 409.
    JobSpecInput {
        id: "payments".to_owned(),
        replicas: 7,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 536_870_912 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    }
}

// -----------------------------------------------------------------------
// AC — happy-path round trip: POST /v1/jobs returns 200 + Inserted +
// non-empty spec_digest. Per ADR-0020 the per-write witness is
// `outcome` + `spec_digest`, not `commit_index`.
// -----------------------------------------------------------------------

#[tokio::test]
async fn post_v1_jobs_with_valid_spec_returns_200_inserted_with_canonical_digest() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);
    let url = format!("https://localhost:{}/v1/jobs", bound.port());

    let spec = payments_spec();

    let resp = client
        .post(&url)
        .json(&SubmitJobRequest { spec: spec.clone() })
        .send()
        .await
        .expect("POST /v1/jobs");

    assert_eq!(resp.status(), reqwest::StatusCode::OK, "expected 200 OK");
    let body: SubmitJobResponse = resp.json().await.expect("decode SubmitJobResponse");

    assert_eq!(body.job_id, "payments", "job_id must echo the canonicalised JobId");
    assert_eq!(
        body.outcome,
        IdempotencyOutcome::Inserted,
        "fresh insert must report `outcome = Inserted`; got {:?}",
        body.outcome,
    );

    // spec_digest must equal the locally-computable SHA-256 of the
    // rkyv-archived Job bytes (ADR-0002 + ADR-0020). Mismatch means
    // a server-side re-archival or a serde-driven recomputation
    // somewhere in the pipeline.
    let local_job = Job::from_spec(spec).expect("Job::from_spec for digest reference");
    let local_archived =
        rkyv::to_bytes::<rkyv::rancor::Error>(&local_job).expect("local rkyv archive");
    let local_digest = overdrive_core::id::ContentHash::of(local_archived.as_ref()).to_string();
    assert_eq!(
        body.spec_digest, local_digest,
        "spec_digest must equal the locally-computable canonical \
         hash; got server={}, local={}",
        body.spec_digest, local_digest,
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

// -----------------------------------------------------------------------
// AC — the archived Job is persisted at the canonical intent key
// -----------------------------------------------------------------------

#[tokio::test]
async fn post_v1_jobs_persists_archived_job_under_jobs_prefix_in_local_store() {
    let (handle, bound, tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);
    let url = format!("https://localhost:{}/v1/jobs", bound.port());

    let spec = payments_spec();
    let resp = client
        .post(&url)
        .json(&SubmitJobRequest { spec: spec.clone() })
        .send()
        .await
        .expect("POST /v1/jobs");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // Shut the server down first so the redb file's write handle is
    // released — our back-door read can then open the file cleanly in
    // environments where redb's file lock is exclusive.
    handle.shutdown(Duration::from_secs(2)).await;

    let job_id = JobId::new("payments").expect("parse payments JobId");
    let key = IntentKey::for_job(&job_id);
    let persisted = read_intent_key_from_store(&data_dir_under(tmp.path()), key.as_bytes())
        .await
        .expect("jobs/payments must be populated after successful submit");

    assert!(!persisted.is_empty(), "archived Job bytes must be non-empty");

    // Rebuild the expected archive from the same spec and compare bytes.
    let expected_job = Job::from_spec(spec).expect("canonical spec constructs a Job");
    let expected_bytes =
        rkyv::to_bytes::<rkyv::rancor::Error>(&expected_job).expect("rkyv archive of expected Job");

    assert_eq!(
        persisted.as_ref(),
        expected_bytes.as_ref(),
        "persisted bytes must equal rkyv archive of Job::from_spec(...) \
         — handler must archive via rkyv, not via serde_json or another format"
    );
}

// -----------------------------------------------------------------------
// AC — invalid spec (zero replicas) -> HTTP 400 with field-pointing body
// -----------------------------------------------------------------------

#[tokio::test]
async fn post_v1_jobs_with_invalid_spec_returns_400_with_error_body_naming_field() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);
    let url = format!("https://localhost:{}/v1/jobs", bound.port());

    // `replicas = 0` fails `Job::from_spec` at the `NonZeroU32` gate with
    // `AggregateError::Validation { field: "replicas", .. }`.
    let bad = JobSpecInput {
        id: "payments".to_owned(),
        replicas: 0,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 536_870_912 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    };

    let resp = client
        .post(&url)
        .json(&SubmitJobRequest { spec: bad })
        .send()
        .await
        .expect("POST /v1/jobs with bad spec");

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST, "bad spec must be HTTP 400");
    let body: ErrorBody = resp.json().await.expect("decode ErrorBody");
    assert_eq!(body.error, "validation", "error kind must be 'validation'");
    assert!(
        body.message.contains("replica"),
        "message must name the offending field (substring `replica`); got {:?}",
        body.message
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

// -----------------------------------------------------------------------
// AC — byte-identical re-submission is idempotent: same spec_digest
// and `outcome = Unchanged` (ADR-0015 §4 amended by ADR-0020).
// -----------------------------------------------------------------------

#[tokio::test]
async fn post_v1_jobs_idempotent_byte_identical_spec_returns_unchanged_with_same_digest() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);
    let url = format!("https://localhost:{}/v1/jobs", bound.port());

    let spec = payments_spec();

    let first: SubmitJobResponse = client
        .post(&url)
        .json(&SubmitJobRequest { spec: spec.clone() })
        .send()
        .await
        .expect("first submit")
        .json()
        .await
        .expect("decode first response");

    let second_resp =
        client.post(&url).json(&SubmitJobRequest { spec }).send().await.expect("second submit");

    assert_eq!(
        second_resp.status(),
        reqwest::StatusCode::OK,
        "byte-identical re-submission must be 200 OK (idempotent)",
    );
    let second: SubmitJobResponse = second_resp.json().await.expect("decode second response");

    assert_eq!(first.job_id, second.job_id);
    assert_eq!(
        first.outcome,
        IdempotencyOutcome::Inserted,
        "first submit must report `outcome = Inserted`; got {:?}",
        first.outcome,
    );
    assert_eq!(
        second.outcome,
        IdempotencyOutcome::Unchanged,
        "byte-identical re-submission must report `outcome = Unchanged`; \
         got {:?}",
        second.outcome,
    );
    assert_eq!(
        first.spec_digest, second.spec_digest,
        "byte-identical re-submission must return the same spec_digest \
         (ADR-0015 §4 amended by ADR-0020: idempotent success)",
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

// -----------------------------------------------------------------------
// AC — different spec at same JobId -> HTTP 409 Conflict
// -----------------------------------------------------------------------

#[tokio::test]
async fn post_v1_jobs_with_different_spec_at_existing_key_returns_409_conflict() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);
    let url = format!("https://localhost:{}/v1/jobs", bound.port());

    // First submit: canonical spec.
    let first = client
        .post(&url)
        .json(&SubmitJobRequest { spec: payments_spec() })
        .send()
        .await
        .expect("first submit");
    assert_eq!(first.status(), reqwest::StatusCode::OK);

    // Second submit: same JobId, different replicas. Must be rejected
    // with 409 per ADR-0015 §4 "Duplicate intent-key with *different*
    // spec".
    let conflict = client
        .post(&url)
        .json(&SubmitJobRequest { spec: payments_spec_alt() })
        .send()
        .await
        .expect("second submit");

    assert_eq!(
        conflict.status(),
        reqwest::StatusCode::CONFLICT,
        "different spec at same JobId must be HTTP 409 Conflict",
    );
    let body: ErrorBody = conflict.json().await.expect("decode ErrorBody");
    assert_eq!(body.error, "conflict", "error kind must be 'conflict'");

    handle.shutdown(Duration::from_secs(2)).await;
}

// Per ADR-0020 the `LocalIntentStore::commit_index()` accessor was
// removed; the previous `local_store_commit_index_monotonically_increases`
// test has no counter to assert against and is deleted in
// `redesign-drop-commit-index` step 01-04.
