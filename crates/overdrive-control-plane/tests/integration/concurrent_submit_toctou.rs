//! Integration test for `POST /v1/jobs` under concurrent submission —
//! the TOCTOU race pinned shut by `IntentStore::put_if_absent`.
//!
//! Prior to step 03-01's hardening, `submit_job` used a naive `get`
//! (read txn) + `put` (write txn) pair. Two concurrent submitters for
//! the same `JobId` but *different* specs could both observe `None`
//! on the read, both fall through to the blind `put`, and the second
//! writer would silently clobber the first — no 409 returned to
//! either caller, one spec lost.
//!
//! The invariant this test defends:
//!
//! * **Exactly one `201 Inserted` (HTTP 200 with a new `commit_index`)
//!   per unique intent key, no matter how many concurrent submissions
//!   race on the same key.**
//! * Every other concurrent submission whose spec differs from the
//!   winner returns HTTP 409 with `ErrorBody.error == "conflict"`.
//! * Every other concurrent submission whose spec is byte-identical
//!   to the winner returns HTTP 200 with the winner's
//!   `commit_index` — this is the idempotency leg.
//! * The stored bytes at the intent key equal the rkyv archive of
//!   the winning spec; they are never mutated by the losers.
//!
//! Tier 3 — real redb file on `tempfile`, real axum server, real
//! rustls handshake, real reqwest, real `tokio::join_all` concurrency.
//! Gated by the `integration-tests` feature at the
//! `tests/integration.rs` entrypoint.

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
// Helpers — duplicated from `submit_round_trip.rs` / `idempotent_resubmit.rs`
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
/// Step 01-02 — see RCA §WHY 4C. Callers that read the redb file
/// back-door derive the data dir via [`data_dir_under`].
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
    };
    let handle = run_server(config).await.expect("run_server");
    let bound = handle.local_addr().await.expect("bound addr");
    let ca_pem = read_ca_from_trust_triple(&operator_config_dir);
    (handle, bound, tmp, ca_pem)
}

/// Resolve the redb storage root for a tempdir-rooted server fixture.
fn data_dir_under(tmp: &std::path::Path) -> std::path::PathBuf {
    tmp.join("data")
}

/// Resolve the operator-config base directory for a tempdir-rooted
/// server fixture.
fn operator_config_dir_under(tmp: &std::path::Path) -> std::path::PathBuf {
    tmp.join("conf")
}

async fn read_intent_key_from_store(data_dir: &std::path::Path, key: &[u8]) -> Option<Bytes> {
    let path = data_dir.join("intent.redb");
    assert!(path.exists(), "expected redb file at {}; found none", path.display());
    let store = LocalIntentStore::open(&path).expect("open LocalIntentStore for back-door read");
    // `IntentStore::get` returns `(Bytes, u64)` per
    // `fix-commit-index-per-entry`; this helper projects to bytes
    // only because every back-door reader in this file asserts on
    // the rkyv archive shape, not on the per-entry commit_index.
    store.get(key).await.expect("back-door get").map(|(bytes, _idx)| bytes)
}

fn spec_with_replicas(replicas: u32) -> JobSpecInput {
    JobSpecInput { id: "payments".to_owned(), replicas, cpu_milli: 500, memory_bytes: 536_870_912 }
}

// -----------------------------------------------------------------------
// The TOCTOU defence test.
//
// Fires N concurrent submits against the same JobId, each carrying a
// DISTINCT spec (distinct `replicas` — produces distinct rkyv archive
// bytes). Exactly one must win with HTTP 200 and produce the only
// commit_index advance; every other submit must return HTTP 409.
//
// The key property: it does not matter WHICH spec wins — what matters
// is that exactly one does, and the IntentStore ends up holding
// byte-exactly that winner's archive. A TOCTOU race under the naive
// `get + put` pattern would show up here as two submits receiving HTTP
// 200 with distinct archived bytes, and the IntentStore retaining
// whichever write committed last.
// -----------------------------------------------------------------------

#[tokio::test]
async fn concurrent_distinct_specs_same_key_commit_exactly_once() {
    // N distinct specs, same JobId = "payments". Each spec produces
    // distinct rkyv archive bytes because `replicas` differs, so any
    // pair of winners would be an observable byte-drift failure.
    const CONCURRENCY: u32 = 8;

    let (handle, bound, tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);
    let url = format!("https://localhost:{}/v1/jobs", bound.port());

    let specs: Vec<JobSpecInput> = (1..=CONCURRENCY).map(spec_with_replicas).collect();

    // Fire every request on the same reqwest client — reqwest pools
    // connections per client, so the server sees N concurrent
    // TLS-bearing streams on the HTTP/2 multiplexed connection.
    let mut set = tokio::task::JoinSet::new();
    for spec in specs.iter().cloned() {
        let client = client.clone();
        let url = url.clone();
        set.spawn(async move {
            let resp = client
                .post(&url)
                .json(&SubmitJobRequest { spec: spec.clone() })
                .send()
                .await
                .expect("concurrent submit send");
            let status = resp.status();
            if status.is_success() {
                let body: SubmitJobResponse =
                    resp.json().await.expect("decode SubmitJobResponse on 200");
                (spec, status.as_u16(), Some(body), None::<ErrorBody>)
            } else {
                let body: ErrorBody = resp.json().await.expect("decode ErrorBody on non-2xx");
                (spec, status.as_u16(), None, Some(body))
            }
        });
    }

    let mut ok_outcomes: Vec<(JobSpecInput, SubmitJobResponse)> = Vec::new();
    let mut conflict_outcomes: Vec<(JobSpecInput, ErrorBody)> = Vec::new();
    while let Some(res) = set.join_next().await {
        let (spec, status, ok, err) = res.expect("join concurrent submit task");
        match (status, ok, err) {
            (200, Some(body), None) => ok_outcomes.push((spec, body)),
            (409, None, Some(body)) => conflict_outcomes.push((spec, body)),
            other => panic!(
                "unexpected concurrent submit outcome {other:?} — must be HTTP 200 (winner) \
                 or HTTP 409 (loser with distinct spec) under the TOCTOU contract"
            ),
        }
    }

    // Core invariant: exactly ONE winner. A naive `get + put` handler
    // would allow multiple concurrent submits to see `None` on the read
    // and all fall through to `put`, producing two or more 200s with
    // silently-drifting IntentStore bytes.
    assert_eq!(
        ok_outcomes.len(),
        1,
        "expected exactly 1 concurrent submit to win with HTTP 200; \
         got {}. This is the TOCTOU failure shape — two callers both \
         observed the key as empty under a non-atomic get + put, both \
         wrote, and the last write silently clobbered the first. \
         ok_outcomes = {ok_outcomes:#?}, conflict_outcomes = {conflict_outcomes:#?}",
        ok_outcomes.len(),
    );
    assert_eq!(
        conflict_outcomes.len(),
        (CONCURRENCY as usize) - 1,
        "every loser must return HTTP 409; got {} conflicts against {} losers. \
         A loser silently succeeding is the TOCTOU race. conflict_outcomes = {conflict_outcomes:#?}",
        conflict_outcomes.len(),
        (CONCURRENCY as usize) - 1,
    );

    // Every 409 body must carry the `conflict` error kind and name the
    // intent-key path, matching the shape the CLI decodes (ADR-0015 §4).
    for (_spec, body) in &conflict_outcomes {
        assert_eq!(body.error, "conflict", "409 ErrorBody.error must be 'conflict'; got {body:?}");
        assert!(
            body.message.contains("jobs/payments"),
            "409 ErrorBody.message must name the intent-key path; got {body:?}",
        );
    }

    // The IntentStore must contain byte-exactly the rkyv archive of the
    // winning spec — no post-commit drift from a losing writer.
    let winning_spec = ok_outcomes[0].0.clone();
    let winning_commit_index = ok_outcomes[0].1.commit_index;

    handle.shutdown(Duration::from_secs(2)).await;

    let job_id = JobId::new("payments").expect("parse JobId");
    let key = IntentKey::for_job(&job_id);
    let persisted = read_intent_key_from_store(&data_dir_under(tmp.path()), key.as_bytes())
        .await
        .expect("jobs/payments must be populated after a successful concurrent submit");

    let expected_job = Job::from_spec(winning_spec.clone()).expect("winning spec constructs a Job");
    let expected_bytes =
        rkyv::to_bytes::<rkyv::rancor::Error>(&expected_job).expect("rkyv archive of winning Job");
    assert_eq!(
        persisted.as_ref(),
        expected_bytes.as_ref(),
        "stored bytes must equal the rkyv archive of the WINNING spec \
         (replicas = {}); a byte-drift here means a loser's write \
         clobbered the winner — the exact TOCTOU failure put_if_absent \
         exists to prevent",
        winning_spec.replicas,
    );

    // Guard against the winner's commit_index collapsing to zero — if
    // the atomic insert never bumped the counter the whole round-trip
    // is suspect even when the stored bytes look right.
    assert!(
        winning_commit_index >= 1,
        "winner's commit_index must advance on insert; got {winning_commit_index}",
    );
}

// -----------------------------------------------------------------------
// Concurrent byte-identical submits: one winner, N−1 idempotent 200s.
//
// Separate test so the invariant is pinned independently of the
// "distinct specs produce 409" test above. A naive `get + put` handler
// could pass the 409 test but silently double-write on the idempotent
// path — this test would catch that by asserting the IntentStore
// bytes remain exactly one rkyv archive AND every submit returns the
// same commit_index.
// -----------------------------------------------------------------------

#[tokio::test]
async fn concurrent_byte_identical_submits_return_single_commit_index() {
    const CONCURRENCY: usize = 8;

    let (handle, bound, tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);
    let url = format!("https://localhost:{}/v1/jobs", bound.port());

    let spec = spec_with_replicas(3);

    let mut set = tokio::task::JoinSet::new();
    for _ in 0..CONCURRENCY {
        let client = client.clone();
        let url = url.clone();
        let spec = spec.clone();
        set.spawn(async move {
            let resp = client
                .post(&url)
                .json(&SubmitJobRequest { spec })
                .send()
                .await
                .expect("concurrent submit send");
            assert_eq!(
                resp.status(),
                reqwest::StatusCode::OK,
                "every byte-identical concurrent submit must be HTTP 200 \
                 (either the winner or an idempotent re-submit); a 409 \
                 here would be the TOCTOU race misclassifying an identical \
                 write as a conflict",
            );
            resp.json::<SubmitJobResponse>().await.expect("decode SubmitJobResponse")
        });
    }

    let mut responses: Vec<SubmitJobResponse> = Vec::with_capacity(CONCURRENCY);
    while let Some(res) = set.join_next().await {
        responses.push(res.expect("join concurrent identical submit task"));
    }

    // All N responses must carry the same commit_index — the one the
    // first writer produced. A second commit_index value in the set
    // means a second write committed, which is the double-write
    // TOCTOU failure shape.
    let unique_indices: std::collections::HashSet<u64> =
        responses.iter().map(|r| r.commit_index).collect();
    assert_eq!(
        unique_indices.len(),
        1,
        "byte-identical concurrent submits must all return the SAME \
         commit_index; got {unique_indices:?}. More than one distinct \
         index means the handler double-wrote under concurrency.",
    );
    let the_index = *unique_indices.iter().next().expect("one element");
    assert!(the_index >= 1, "commit_index must be >= 1 after insert; got {the_index}");

    // Per-entry commit_index contract (fix-commit-index-per-entry RCA
    // §WHY 1A) — a follow-on `describe` of the same job_id MUST return
    // the same commit_index every concurrent submitter saw. A handler
    // that returns the live store counter at describe-time would drift
    // here if any other write had happened between submit and describe;
    // even with no intervening write, this assertion pins the
    // describe-returns-write-index property by construction. RED
    // scaffold (Step 01-01) — flips GREEN once Step 01-02 lands the
    // trait + handler change.
    let job_id_str = responses.first().expect("at least one response").job_id.clone();
    let describe_url = format!("https://localhost:{}/v1/jobs/{}", bound.port(), job_id_str);
    let describe_resp =
        client.get(&describe_url).send().await.expect("GET /v1/jobs/{id} after concurrent burst");
    assert_eq!(
        describe_resp.status(),
        reqwest::StatusCode::OK,
        "describe of the just-burst-submitted job must be HTTP 200",
    );
    let description: JobDescription =
        describe_resp.json().await.expect("decode JobDescription after burst");
    assert_eq!(
        description.commit_index, the_index,
        "RED scaffold (Step 01-01) — describe(job_id).commit_index must \
         equal the index every concurrent submitter saw ({the_index}); got \
         {} from describe — this is the per-entry contract violation \
         (RCA §WHY 1A) the fix-commit-index-per-entry feature pins shut.",
        description.commit_index,
    );

    // IntentStore must hold exactly one rkyv archive of the spec —
    // byte-equal to what any of the concurrent submitters would have
    // archived.
    handle.shutdown(Duration::from_secs(2)).await;

    let job_id = JobId::new("payments").expect("parse JobId");
    let key = IntentKey::for_job(&job_id);
    let persisted = read_intent_key_from_store(&data_dir_under(tmp.path()), key.as_bytes())
        .await
        .expect("jobs/payments must be populated after concurrent identical submits");

    let expected_job = Job::from_spec(spec).expect("spec constructs a Job");
    let expected_bytes =
        rkyv::to_bytes::<rkyv::rancor::Error>(&expected_job).expect("rkyv archive of Job");
    assert_eq!(
        persisted.as_ref(),
        expected_bytes.as_ref(),
        "stored bytes must equal the rkyv archive of the submitted spec; \
         a mismatch here means a concurrent writer drifted the bytes post-commit",
    );
}
