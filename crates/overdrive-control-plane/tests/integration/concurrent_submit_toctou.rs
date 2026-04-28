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
//! * **Exactly one HTTP 200 with `outcome == Inserted` per unique
//!   intent key, no matter how many concurrent submissions race on
//!   the same key.** (Per ADR-0020 the wire-level per-write witness
//!   is `outcome` + `spec_digest`, not `commit_index`.)
//! * Every other concurrent submission whose spec differs from the
//!   winner returns HTTP 409 with `ErrorBody.error == "conflict"`.
//! * Every other concurrent submission whose spec is byte-identical
//!   to the winner returns HTTP 200 with `outcome == Unchanged` and
//!   the same `spec_digest` as the winner — this is the idempotency
//!   leg.
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
    ErrorBody, IdempotencyOutcome, JobDescription, SubmitJobRequest, SubmitJobResponse,
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
        // Control-plane integration tests don't start real workloads;
        // bypass the cgroup pre-flight so they run uniformly on macOS
        // and on Linux without delegation.
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
    // Per ADR-0020 `IntentStore::get` returns `Option<Bytes>`.
    store.get(key).await.expect("back-door get")
}

fn spec_with_replicas(replicas: u32) -> JobSpecInput {
    JobSpecInput { id: "payments".to_owned(), replicas, cpu_milli: 500, memory_bytes: 536_870_912 }
}

// -----------------------------------------------------------------------
// The TOCTOU defence test.
//
// Fires N concurrent submits against the same JobId, each carrying a
// DISTINCT spec (distinct `replicas` — produces distinct rkyv archive
// bytes). Exactly one must win with HTTP 200 + `outcome == Inserted`;
// every other submit must return HTTP 409.
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
    let winning_outcome = ok_outcomes[0].1.outcome;
    let winning_digest = ok_outcomes[0].1.spec_digest.clone();

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

    // Per ADR-0020 the wire-level winner witness is `outcome ==
    // Inserted` and a non-empty `spec_digest`. A drift to `Unchanged`
    // here would mean the handler never took the insert branch at
    // all under contention — the exact bug class the TOCTOU defence
    // exists to prevent.
    assert_eq!(
        winning_outcome,
        IdempotencyOutcome::Inserted,
        "winner must report `outcome = Inserted`; got {winning_outcome:?} \
         — the handler took the idempotency branch instead of inserting",
    );
    assert_eq!(
        winning_digest.len(),
        64,
        "winner's spec_digest must be 64 hex chars (SHA-256); got {} chars",
        winning_digest.len(),
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
// same `spec_digest` (per ADR-0020 the per-write witness).
// -----------------------------------------------------------------------

#[tokio::test]
async fn concurrent_byte_identical_submits_return_single_spec_digest() {
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

    // All N responses must carry the same `spec_digest` — the one the
    // first writer produced (per ADR-0020 the spec digest is the
    // per-write witness; byte-identical specs hash to the same digest
    // by SHA-256 construction). A second digest value in the set
    // means a second write committed, which is the double-write
    // TOCTOU failure shape.
    let unique_digests: std::collections::BTreeSet<String> =
        responses.iter().map(|r| r.spec_digest.clone()).collect();
    assert_eq!(
        unique_digests.len(),
        1,
        "byte-identical concurrent submits must all return the SAME \
         spec_digest; got {unique_digests:?}. More than one distinct \
         digest means the handler double-wrote under concurrency.",
    );
    let the_digest = unique_digests.iter().next().expect("one element").clone();
    assert_eq!(
        the_digest.len(),
        64,
        "spec_digest must be 64 hex chars (SHA-256); got {} chars",
        the_digest.len(),
    );

    // Exactly one response must report `outcome = Inserted` (the
    // winner); every other must report `outcome = Unchanged`. The
    // idempotency leg for byte-identical resubmits (ADR-0015 §4
    // amended by ADR-0020).
    let inserted_count =
        responses.iter().filter(|r| r.outcome == IdempotencyOutcome::Inserted).count();
    let unchanged_count =
        responses.iter().filter(|r| r.outcome == IdempotencyOutcome::Unchanged).count();
    assert_eq!(
        inserted_count, 1,
        "exactly one byte-identical concurrent submit must report \
         `outcome = Inserted` (the winner); got {inserted_count}",
    );
    assert_eq!(
        unchanged_count,
        CONCURRENCY - 1,
        "every loser of a byte-identical concurrent submit must report \
         `outcome = Unchanged`; got {unchanged_count} unchanged out of {} losers",
        CONCURRENCY - 1,
    );

    // Describe(job_id) must return the same `spec_digest` every
    // concurrent submitter saw. Per ADR-0020 the digest is the
    // round-trip witness that submit and describe agree on the same
    // canonical bytes.
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
        description.spec_digest, the_digest,
        "describe(job_id).spec_digest must equal the digest every \
         concurrent submitter saw ({the_digest}); got {} from describe \
         — this is the round-trip property the per-write digest \
         witness (ADR-0020) defends.",
        description.spec_digest,
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
