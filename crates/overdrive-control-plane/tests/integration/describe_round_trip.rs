//! Integration tests for `GET /v1/jobs/{id}` — step 03-02.
//!
//! Proves the Phase 1 `describe_workload` handler round-trip:
//!
//! 1. After `POST /v1/jobs`, `GET /v1/jobs/{id}` returns HTTP 200 with
//!    the canonical `WorkloadDescription` shape — `spec`, `spec_digest`
//!    (per ADR-0020 the `commit_index` field is dropped).
//! 2. The returned `spec` is byte-identical (via rkyv archive of the
//!    round-tripped `Job`) to the spec the operator submitted.
//! 3. `spec_digest` is `ContentHash::of(<rkyv-archived-bytes>).to_string()`.
//! 4. Unknown `{id}` returns HTTP 404 with an `ErrorBody` whose `error`
//!    field is `"not_found"`.
//! 5. Submit-then-describe proptest (`PROPTEST_CASES=256`) — mandatory
//!    rkyv-roundtrip call site per `.claude/rules/testing.md`.
//!
//! Tier 3 — real redb file on `tempfile`, real axum server, real rustls
//! handshake, real reqwest. Gated by the `integration-tests` feature at
//! the `tests/integration.rs` entrypoint.

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::body::to_bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use overdrive_control_plane::api::{
    ErrorBody, SubmitWorkloadRequest, SubmitWorkloadResponse, WorkloadDescription,
};
use overdrive_control_plane::handlers::{describe_workload, submit_workload};
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_control_plane::{AppState, ServerConfig, ServerHandle, run_server};
use overdrive_core::aggregate::{DriverInput, ExecInput, Job, JobSpecInput, ResourcesInput};
use overdrive_core::api::describe::DescribeSpecOutput;
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::id::NodeId;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_host::RealCgroupFs;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use proptest::prelude::*;
use tempfile::TempDir;

// -----------------------------------------------------------------------
// Helpers — spawn a server, mint a reqwest client trusting the ephemeral
// CA. Shared in shape with `submit_round_trip.rs` — any drift here should
// be refactored into a shared helper module under `tests/integration/`,
// but for now duplicating keeps each scenario self-contained and readable.
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
/// Step 01-02 — see RCA §WHY 4C for why the overload is unsound.
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
        // ADR-0061 § 1 (step 01-03): the default `ServerConfig.dataplane`
        // is now the veth-named single-node shape, whose `client_iface`
        // (`ovd-veth-cli`) does not exist in the test VM. This fixture
        // injects `SimDataplane` (no XDP attach) but still resolves
        // `host_ipv4` from `client_iface` at boot, so it names `lo` via
        // the shared SSOT helper.
        dataplane: Some(super::dataplane_lo::lo_dataplane_config()),
        // Step 02-02 (C1-AMEND) — hermetic in-process boot KEK so `boot_ca`'s
        // KEK-resolve probe succeeds with no kernel-keyring / env dependency.
        ..ServerConfig::new(std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
    };
    let handle =
        run_server(config, std::sync::Arc::new(RealCgroupFs::new())).await.expect("run_server");
    let bound = handle.local_addr().await.expect("bound addr");
    let ca_pem = read_ca_from_trust_triple(&operator_config_dir);
    (handle, bound, tmp, ca_pem)
}

fn payments_spec() -> JobSpecInput {
    JobSpecInput {
        id: "payments".to_owned(),
        replicas: 3,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 536_870_912 }, // 512 MiB
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    }
}

/// Compute the canonical `spec_digest` a correct handler must return:
/// `ContentHash::of(rkyv::to_bytes(Job::from_submit(spec))).to_string()`.
///
/// This mirrors the handler's expected behaviour exactly — if the handler
/// instead hashes `serde_json::to_string(&job)` or re-canonicalises via
/// JCS, the assertions in this module will fail, as they should.
fn expected_spec_digest(spec: &JobSpecInput) -> String {
    let job = Job::from_submit(spec.clone()).expect("canonical spec constructs a Job");
    overdrive_core::aggregate::WorkloadIntent::Job(job)
        .spec_digest()
        .expect("spec_digest")
        .to_string()
}

// -----------------------------------------------------------------------
// AC — §4.1: Submit then Describe round-trips byte-identical
// -----------------------------------------------------------------------

#[tokio::test]
async fn get_v1_jobs_id_returns_described_job_after_submit() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);

    // 1. Submit.
    let submit_url = format!("https://localhost:{}/v1/jobs", bound.port());
    let submit_resp = client
        .post(&submit_url)
        .json(&SubmitWorkloadRequest { spec: SubmitSpecInput::Job(payments_spec()) })
        .send()
        .await
        .expect("POST /v1/jobs");
    assert_eq!(submit_resp.status(), reqwest::StatusCode::OK);
    let submit_body: SubmitWorkloadResponse =
        submit_resp.json().await.expect("decode submit response");

    // 2. Describe.
    let describe_url =
        format!("https://localhost:{}/v1/jobs/{}", bound.port(), submit_body.workload_id);
    let describe_resp = client.get(&describe_url).send().await.expect("GET /v1/jobs/{id}");
    assert_eq!(
        describe_resp.status(),
        reqwest::StatusCode::OK,
        "describe of a just-submitted job must be HTTP 200 OK",
    );
    let description: WorkloadDescription =
        describe_resp.json().await.expect("decode WorkloadDescription");

    // 3. spec must round-trip byte-identical (via rkyv) to the submitted
    //    spec — `Job::from_submit(spec)` applied to both sides must produce
    //    byte-identical archives. Post-ADR-0064 the describe response is a
    //    `DescribeSpecOutput` oneOf; a Job submit describes as the `Job` arm
    //    carrying the verbatim `JobSpecInput` projection.
    assert_eq!(
        description.spec,
        DescribeSpecOutput::Job(payments_spec()),
        "described spec must be the Job arm carrying the byte-identical (via rkyv) submitted spec",
    );

    // 4. spec_digest must be present and non-empty — the per-write
    //    witness that submit and describe agree on the canonical bytes
    //    (per ADR-0020). The full digest-equality property is pinned
    //    in `describe_spec_digest_equals_content_hash_of_archived_bytes`.
    assert_eq!(
        description.spec_digest.len(),
        64,
        "described spec_digest must be 64 hex chars (SHA-256); got {} chars",
        description.spec_digest.len(),
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

// -----------------------------------------------------------------------
// Regression — malformed `{id}` path parameter returns 400 with
// `field: Some("id")` so client tooling can branch on the discriminator.
//
// Pre-fix shape: `WorkloadId::new(...).map_err(AggregateError::Id)?` routed
// through `to_response`'s `Aggregate(Id(_))` arm, which hardcodes
// `field = None`. The handler holds stronger context than the wrapped
// aggregate error — the path parameter name is statically known — and
// must attach it explicitly.
// -----------------------------------------------------------------------

#[tokio::test]
async fn get_v1_jobs_malformed_id_returns_400_with_field_id() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);

    // `WorkloadId::new` rejects labels that don't start with an alphanumeric
    // character (`validate_label` enforces `InvalidFormat`). A leading
    // hyphen is URL-safe (no percent-encoding required, distinguishable
    // from a captured path segment) and forces the validation lane the
    // bug report targets — distinct from the `no-such-job` 404 path
    // which uses a *valid* WorkloadId form that simply isn't stored.
    //
    // (Note: uppercase ASCII canonicalises to lowercase via
    // `to_ascii_lowercase` in the parser, so `INVALID` → `invalid`
    // and yields a 404, not a 400 — the wrong lane for this test.)
    let describe_url = format!("https://localhost:{}/v1/jobs/-bad", bound.port());
    let resp = client.get(&describe_url).send().await.expect("GET /v1/jobs/{malformed}");

    assert_eq!(
        resp.status(),
        reqwest::StatusCode::BAD_REQUEST,
        "describe of a malformed WorkloadId must be HTTP 400",
    );

    let body: ErrorBody = resp.json().await.expect("decode ErrorBody");
    assert_eq!(body.error, "validation", "error kind must be 'validation'; got {:?}", body.error);
    assert_eq!(
        body.field.as_deref(),
        Some("id"),
        "field discriminator must name the offending path parameter; \
         got {:?}. Without this, client tooling branching on `field` \
         loses the ability to distinguish path-parameter validation \
         from request-body validation.",
        body.field,
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

// -----------------------------------------------------------------------
// AC — §4.4: Describe unknown id returns 404 with `not_found` error body
// -----------------------------------------------------------------------

#[tokio::test]
async fn get_v1_jobs_unknown_id_returns_404_with_error_body() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);

    // No prior submit — any valid WorkloadId format we pass in must 404
    // because the underlying `IntentStore::get` returns `None` for the
    // canonical key.
    let describe_url = format!("https://localhost:{}/v1/jobs/no-such-job", bound.port());
    let resp = client.get(&describe_url).send().await.expect("GET /v1/jobs/{unknown}");

    assert_eq!(
        resp.status(),
        reqwest::StatusCode::NOT_FOUND,
        "describe of an unknown WorkloadId must be HTTP 404",
    );

    let body: ErrorBody = resp.json().await.expect("decode ErrorBody");
    assert_eq!(body.error, "not_found", "error kind must be 'not_found'; got {:?}", body.error);
    assert!(
        body.message.contains("no-such-job"),
        "message must identify the missing resource; got {:?}",
        body.message,
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

// -----------------------------------------------------------------------
// AC — spec_digest equals ContentHash::of(<archived bytes>) exactly
// -----------------------------------------------------------------------

#[tokio::test]
async fn describe_spec_digest_equals_content_hash_of_archived_bytes() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);

    let spec = payments_spec();
    let submit_url = format!("https://localhost:{}/v1/jobs", bound.port());
    let submit_resp = client
        .post(&submit_url)
        .json(&SubmitWorkloadRequest { spec: SubmitSpecInput::Job(spec.clone()) })
        .send()
        .await
        .expect("POST /v1/jobs");
    assert_eq!(submit_resp.status(), reqwest::StatusCode::OK);
    let submit_body: SubmitWorkloadResponse =
        submit_resp.json().await.expect("decode submit response");

    let describe_url =
        format!("https://localhost:{}/v1/jobs/{}", bound.port(), submit_body.workload_id);
    let resp = client.get(&describe_url).send().await.expect("GET /v1/jobs/{id}");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let description: WorkloadDescription = resp.json().await.expect("decode WorkloadDescription");

    let expected = expected_spec_digest(&spec);
    assert_eq!(
        description.spec_digest, expected,
        "spec_digest must equal ContentHash::of(rkyv::to_bytes(Job::from_submit(spec))) — \
         ADR-0002 canonical hashing. Got {:?}; expected {:?}",
        description.spec_digest, expected,
    );

    // Hash must be 64 lowercase hex chars (`ContentHash::Display` format).
    assert_eq!(
        description.spec_digest.len(),
        64,
        "spec_digest must be 64-char SHA-256 hex; got len {}",
        description.spec_digest.len(),
    );
    assert!(
        description.spec_digest.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "spec_digest must be lowercase hex; got {:?}",
        description.spec_digest,
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

// -----------------------------------------------------------------------
// AC — Describe returns spec_digest matching what submit returned —
// the round-trip witness that submit and describe agree on the same
// canonical bytes (per ADR-0020 the per-write witness is `spec_digest`,
// not `commit_index`).
// -----------------------------------------------------------------------

#[tokio::test]
async fn describe_returns_spec_digest_matching_submit_response() {
    let (handle, bound, _tmp, ca_pem) = spawn_server().await;
    let client = client_trusting(&ca_pem);

    let submit_url = format!("https://localhost:{}/v1/jobs", bound.port());
    let submit_resp = client
        .post(&submit_url)
        .json(&SubmitWorkloadRequest { spec: SubmitSpecInput::Job(payments_spec()) })
        .send()
        .await
        .expect("POST /v1/jobs");
    let submit_body: SubmitWorkloadResponse =
        submit_resp.json().await.expect("decode submit response");

    let describe_url =
        format!("https://localhost:{}/v1/jobs/{}", bound.port(), submit_body.workload_id);
    let resp = client.get(&describe_url).send().await.expect("GET /v1/jobs/{id}");
    let description: WorkloadDescription = resp.json().await.expect("decode WorkloadDescription");

    assert_eq!(
        description.spec_digest, submit_body.spec_digest,
        "described spec_digest must match the value returned by submit \
         — both come from hashing the same rkyv-archived bytes",
    );

    handle.shutdown(Duration::from_secs(2)).await;
}

// -----------------------------------------------------------------------
// AC (step 01-03) — describe a Service returns `DescribeSpecOutput::Service`
// carrying the platform-issued VIP.
//
// Port-to-port (in-process driving ports): submit a Service through the
// production `submit_workload` handler so the VIP is allocated under the
// SAME `spec_digest_hash.as_bytes()` key the describe handler reads, then
// drive the production `describe_workload` handler and assert the response
// is `WorkloadDescription { spec: DescribeSpecOutput::Service(ServiceSpecOutput
// { vip, .. }), .. }` carrying the issued VIP.
//
// Litmus: reverting the handler's exhaustive match (back to the HTTP-400
// Service rejection) turns this test RED — the Service arm would no longer
// describe. Sharing the AppState between the two handler calls is the
// load-bearing detail; the allocator memo is keyed by the content-addressed
// digest the submit handler derives, so describe's read-only `get` hits.
// -----------------------------------------------------------------------

/// Build an in-process `AppState` whose `allocator` carries the default
/// `VipRange`, sharing a single tempdir-backed `LocalIntentStore` so the
/// VIP `submit_workload` allocates survives into the `describe_workload`
/// read. Mirrors `vip_allocator_lifecycle.rs::build_state_with_range`.
fn build_in_process_state(tmp: &TempDir) -> AppState {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::from_str("local").expect("NodeId"), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        Arc::clone(&store) as Arc<dyn IntentStore>,
    )));
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        Arc::new(SimClock::new()),
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        NodeId::new("writer-1").expect("NodeId"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

/// Compose a `ServiceSpecInput` with a single `(port, "tcp")` listener
/// and an `exec` driver. Mirrors the shape used by
/// `vip_allocator_lifecycle.rs`.
fn service_spec(id: &str, port: u16) -> ServiceSpecInput {
    ServiceSpecInput {
        id: id.to_owned(),
        replicas: 2,
        resources: ResourcesInput { cpu_milli: 250, memory_bytes: 268_435_456 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
        listeners: vec![ListenerInput { port, protocol: "tcp".to_owned() }],
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

#[tokio::test]
async fn describe_service_returns_discriminated_shape_with_vip() {
    let tmp = TempDir::new().expect("tempdir");
    let state = build_in_process_state(&tmp);

    // ---- (1) Submit the Service through the production handler — this
    //          allocates the VIP under the content-addressed spec_digest.
    let spec = service_spec("frontend", 8080);
    let submit_response = submit_workload(
        State(state.clone()),
        HeaderMap::new(),
        Json(SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec.clone()) }),
    )
    .await
    .expect("Service submit must succeed against the default pool");
    let submit_bytes =
        to_bytes(submit_response.into_body(), usize::MAX).await.expect("submit body bytes");
    let submit_body: SubmitWorkloadResponse =
        serde_json::from_slice(&submit_bytes).expect("decode SubmitWorkloadResponse");
    let issued_vip = submit_body.vip.clone().expect("Service submit echoes the allocated VIP");

    // ---- (2) Describe the Service through the production handler.
    let description = describe_workload(State(state), Path(submit_body.workload_id.clone()))
        .await
        .expect("describe of a submitted Service must succeed (no HTTP-400 rejection)")
        .0;

    // ---- (3) The response carries the discriminated Service arm with the
    //          platform-issued VIP and the operator-declared fields.
    let service = match &description.spec {
        DescribeSpecOutput::Service(svc) => svc,
        other => {
            panic!("describe of a Service must return DescribeSpecOutput::Service; got {other:?}")
        }
    };
    assert_eq!(
        service.vip.to_string(),
        issued_vip,
        "described Service VIP must equal the VIP submit allocated under the same spec_digest",
    );
    assert_eq!(service.id, "frontend", "described Service id must round-trip");
    assert_eq!(service.replicas, 2, "described Service replicas must round-trip");
    assert_eq!(
        service.listeners.len(),
        1,
        "described Service must carry the single operator-declared listener; got {:?}",
        service.listeners,
    );
    assert_eq!(service.listeners[0].port, 8080, "described listener port must round-trip");

    // ---- (4) spec_digest is the canonical 64-char SHA-256 hex and matches
    //          the digest submit returned — the round-trip witness.
    assert_eq!(
        description.spec_digest, submit_body.spec_digest,
        "described spec_digest must match the digest submit returned",
    );
    assert_eq!(
        description.spec_digest.len(),
        64,
        "described spec_digest must be 64 hex chars (SHA-256); got {}",
        description.spec_digest.len(),
    );
}

// -----------------------------------------------------------------------
// AC — submit-then-describe proptest. Mandatory rkyv-roundtrip call site
// per `.claude/rules/testing.md` §Property-based testing.
// -----------------------------------------------------------------------

fn arb_valid_job_spec() -> impl Strategy<Value = JobSpecInput> {
    // `WorkloadId::new` accepts lowercase-ASCII labels (`[a-z0-9][a-z0-9-]*`
    // with trailing-hyphen / length constraints handled internally). We
    // bias toward short valid-by-construction labels so the proptest
    // exercises the handler path, not the newtype's parse failure modes
    // (those are covered in the `id.rs` proptest suite).
    //
    // Resource generators stay modest so the rkyv archive stays small
    // and the default PROPTEST_CASES=256 completes within the
    // integration-test wall-clock budget.
    let id = "[a-z][a-z0-9]{0,15}";
    (id, 1u32..100u32, 1u32..10_000u32, 1u64..(1u64 << 40)).prop_map(
        |(id, replicas, cpu_milli, memory_bytes)| JobSpecInput {
            id,
            replicas,
            resources: ResourcesInput { cpu_milli, memory_bytes },
            driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
        },
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Submit-then-describe round-trip: for any valid `JobSpecInput`,
    /// describing the returned workload_id yields a `WorkloadDescription` whose
    /// `spec` equals the submitted spec AND whose `spec_digest` equals
    /// the canonical `ContentHash::of(rkyv::to_bytes(Job::from_submit(spec)))`.
    ///
    /// Why this shape: the mandatory "rkyv roundtrip" property per
    /// testing.md requires asserting that a rkyv-archived value, when
    /// read back through `access` + `deserialize`, yields the original.
    /// Here the round trip is: operator's `JobSpecInput` → server
    /// `Job::from_submit` → `rkyv::to_bytes` → `IntentStore::put` →
    /// (new request) `IntentStore::get` → `rkyv::access` → `rkyv::deserialize`
    /// → `JobSpecInput::from(&Job)` → wire. Every step is exercised.
    #[test]
    fn submit_then_describe_round_trips_spec_and_digest(spec in arb_valid_job_spec()) {
        // `proptest!` test bodies are synchronous; we spin up a
        // single-threaded tokio runtime per case. This is more
        // expensive than a pooled runtime but keeps the test
        // self-contained and isolated.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        let outcome: Result<(), TestCaseError> = rt.block_on(async {
            let (handle, bound, _tmp, ca_pem) = spawn_server().await;
            let client = client_trusting(&ca_pem);

            let submit_url = format!("https://localhost:{}/v1/jobs", bound.port());
            let submit_resp = client
                .post(&submit_url)
                .json(&SubmitWorkloadRequest { spec: SubmitSpecInput::Job(spec.clone()) })
                .send()
                .await
                .expect("POST /v1/jobs");
            prop_assert_eq!(submit_resp.status(), reqwest::StatusCode::OK);
            let submit_body: SubmitWorkloadResponse =
                submit_resp.json().await.expect("decode submit body");

            let describe_url = format!(
                "https://localhost:{}/v1/jobs/{}",
                bound.port(),
                submit_body.workload_id,
            );
            let describe_resp = client.get(&describe_url).send().await.expect("GET describe");
            prop_assert_eq!(describe_resp.status(), reqwest::StatusCode::OK);
            let description: WorkloadDescription =
                describe_resp.json().await.expect("decode description");

            prop_assert_eq!(
                &description.spec,
                &DescribeSpecOutput::Job(spec.clone()),
                "described spec must be the Job arm carrying the byte-identical (via rkyv) spec",
            );
            prop_assert_eq!(
                &description.spec_digest,
                &expected_spec_digest(&spec),
                "spec_digest must be ContentHash::of(rkyv-archived Job)",
            );

            handle.shutdown(Duration::from_secs(2)).await;
            Ok(())
        });

        outcome?;
    }
}
