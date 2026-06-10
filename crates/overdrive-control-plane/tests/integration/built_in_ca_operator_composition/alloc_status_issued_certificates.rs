//! Integration — the `alloc_status` consumer-side projection selects the
//! CURRENT issued certificate by monotonic `IssuanceOrdinal`, NOT by
//! `issued_at` (built-in-ca-operator-composition, feature-delta § D1-AMEND).
//!
//! This is the load-bearing regression for step-0302 adversarial review
//! findings 1 AND 2:
//!
//! * **Finding 1** — under a fixed/seeded `SimClock` two issuances for one
//!   SPIFFE ID tie on `issued_at`; the old projection
//!   (`max_by_key(|c| c.issued_at)`) resolves the tie by the audit store's
//!   serial-keyed iteration order (`issued_certificate_rows()` returns rows
//!   ascending by `CertSerial`), so `max_by_key` returns the row with the
//!   LARGEST serial — a CSPRNG draw with NO relation to recency. A stale cert
//!   surfaces as "current".
//! * **Finding 2** — no test seeded multiple rows for one alloc to prove
//!   "latest, not history" through the server projection (the previous
//!   render-layer scaffold could not reach the selection logic).
//!
//! The test seeds two `IssuedCertificateRow`s for ONE running alloc with
//! DISTINCT ordinals and EQUAL `issued_at`, arranged so the OLDER
//! (lower-ordinal) row carries the LARGER serial. Under the old
//! `issued_at`-keyed projection the tie resolves to the older row (largest
//! serial, last in ascending-serial iteration) — WRONG. Only the
//! ordinal-keyed projection selects the newer row. It drives the REAL
//! `alloc_status` handler (the server projection, `handlers.rs`), not the
//! render-only CLI layer.
//!
//! Default-lane in-process: `AppState::new` over `SimObservationStore` +
//! `LocalIntentStore`, no real network. Mirrors `alloc_status_snapshot.rs`.

#![allow(clippy::expect_used, clippy::expect_fun_call, clippy::unwrap_used)]

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use overdrive_control_plane::AppState;
use overdrive_control_plane::api::AllocStatusResponse;
use overdrive_control_plane::handlers::{AllocStatusQuery, alloc_status};
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput,
};
use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
use overdrive_core::id::{AllocationId, CertSerial, IssuanceOrdinal, NodeId, SpiffeId, WorkloadId};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

const JOB_ID: &str = "ws-current-cert";
const ALLOC_ID: &str = "alloc-current-cert-0";

/// The serial of the OLDER issuance (lower ordinal). Deliberately the LARGER
/// hex serial so that, under the OLD `max_by_key(issued_at)` tie-break, the
/// ascending-serial store iteration surfaces THIS (stale) row as "current".
const STALE_LARGER_SERIAL: &str = "ffffffffffff";
/// The serial of the NEWER issuance (higher ordinal). The SMALLER hex serial:
/// the new ordinal-keyed projection must select it despite it sorting first.
const CURRENT_SMALLER_SERIAL: &str = "0a0a0a0a0a0a";

/// Equal `issued_at` for BOTH issuances — the tie the fixed `SimClock`
/// produces and the ordinal exists to break.
const TIED_ISSUED_AT_SECS: u64 = 1_700_000_005;

fn writer_node() -> NodeId {
    NodeId::from_str("node-a").expect("valid node id")
}

fn build_app_state(tmp: &TempDir) -> AppState {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(writer_node(), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
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
        NodeId::new("writer-1").unwrap(),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

fn sample_spec() -> JobSpecInput {
    JobSpecInput {
        id: JOB_ID.to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 134_217_728 },
        driver: DriverInput::Exec(ExecInput {
            command: "/usr/local/bin/ws".to_string(),
            args: vec![],
        }),
    }
}

/// Persist `Job::from_submit(spec)` into the `IntentStore` — the precondition
/// for a 200 from `alloc_status` (absent ⇒ 404).
async fn install_job(state: &AppState, spec: JobSpecInput) -> Job {
    let job = Job::from_submit(spec).expect("Job::from_submit must succeed for fixture");
    let key = IntentKey::for_workload(&job.id);
    let archived = overdrive_core::aggregate::WorkloadIntent::Job(job.clone())
        .archive_for_store()
        .expect("rkyv archive");
    state.store.put(key.as_bytes(), archived.as_ref()).await.expect("IntentStore put");
    job
}

/// Seed a RUNNING `alloc_status` row so the alloc is projected (the projection
/// only renders certs for RUNNING allocs).
async fn write_running_row(state: &AppState, alloc: &AllocationId, workload_id: &WorkloadId) {
    let row = AllocStatusRow {
        alloc_id: alloc.clone(),
        workload_id: workload_id.clone(),
        node_id: writer_node(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 2, writer: writer_node() },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Job,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
    };
    state.obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("obs write");
}

/// Write an `issued_certificates` audit row for `spiffe` at the given serial /
/// ordinal, with the SHARED tied `issued_at`.
async fn write_issued_cert(state: &AppState, serial: &str, spiffe: &SpiffeId, ordinal: u64) {
    let at = UnixInstant::from_unix_duration(Duration::from_secs(TIED_ISSUED_AT_SECS));
    let row = IssuedCertificateRow {
        serial: CertSerial::new(serial).expect("valid serial"),
        spiffe_id: spiffe.clone(),
        issuer_serial: CertSerial::new("00").expect("valid issuer serial"),
        not_before: at,
        not_after: at + Duration::from_secs(3600),
        node_id: writer_node(),
        issued_at: at,
        issuance_ordinal: IssuanceOrdinal::new(ordinal),
    };
    state.obs.write(ObservationRow::IssuedCertificate(row)).await.expect("obs write");
}

/// `@integration @driving_port @slice-3` — the `alloc_status` projection
/// selects the CURRENT cert by `issuance_ordinal`, not `issued_at`.
///
/// Two issued-cert rows for ONE running alloc share an equal `issued_at`; the
/// OLDER (ordinal 0) carries the LARGER serial, the NEWER (ordinal 1) the
/// SMALLER serial. The old `max_by_key(issued_at)` projection resolves the tie
/// by ascending-serial store iteration → the larger (stale) serial. The new
/// ordinal-keyed projection selects the newer (smaller) serial. Asserting the
/// newer serial is present exactly once AND the older absent makes this test
/// FAIL against the old behaviour and PASS only with the ordinal.
#[tokio::test]
async fn alloc_status_projects_current_cert_by_ordinal_not_issued_at() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);
    let job = install_job(&state, sample_spec()).await;
    let alloc = AllocationId::from_str(ALLOC_ID).expect("valid alloc id");

    // The SPIFFE identity the projection matches audit rows on.
    let spiffe = SpiffeId::for_allocation(&job.id, &alloc);

    write_running_row(&state, &alloc, &job.id).await;

    // Row A (OLDER): ordinal 0, LARGER serial — the one the OLD projection
    // would (wrongly) pick on the `issued_at` tie.
    write_issued_cert(&state, STALE_LARGER_SERIAL, &spiffe, 0).await;
    // Row B (NEWER): ordinal 1, SMALLER serial — the genuinely current cert.
    write_issued_cert(&state, CURRENT_SMALLER_SERIAL, &spiffe, 1).await;

    let resp = alloc_status(
        State(state.clone()),
        Query(AllocStatusQuery { job: Some(JOB_ID.to_owned()) }),
    )
    .await
    .expect("alloc_status returned err");

    let body: AllocStatusResponse = resp.0;

    // Exactly ONE issued-cert summary — the projection renders the CURRENT
    // cert, not the history (one row per running alloc).
    assert_eq!(
        body.issued_certificates.len(),
        1,
        "exactly one current cert per running alloc (latest, not history); got {:?}",
        body.issued_certificates
    );
    let summary = &body.issued_certificates[0];

    // The CURRENT (newer, higher-ordinal) serial is present — this is the
    // assertion that FAILS under the old `issued_at`-keyed projection (which
    // would surface the stale larger serial) and PASSES only with the ordinal.
    assert_eq!(
        summary.serial,
        CertSerial::new(CURRENT_SMALLER_SERIAL).unwrap(),
        "the projection must select the newest issuance (max ordinal), not the stale row the \
         `issued_at` tie + serial-iteration order would surface"
    );
    // The STALE (older, lower-ordinal, larger) serial is ABSENT.
    assert_ne!(
        summary.serial,
        CertSerial::new(STALE_LARGER_SERIAL).unwrap(),
        "the stale (lower-ordinal) cert must NOT be surfaced as current"
    );
    // The summary is bound to the running alloc's SPIFFE identity.
    assert_eq!(summary.spiffe_id, spiffe, "the summary's SPIFFE id matches the running alloc");

    // No PEM / private-key material leaks onto the operator surface — the audit
    // row persists FACTS only (root CLAUDE.md two-CA / workload-identity
    // discipline; ADR-0067 #215-boundary).
    let json = serde_json::to_string(&body).expect("serialize AllocStatusResponse");
    for forbidden in ["BEGIN CERTIFICATE", "PRIVATE KEY", "BEGIN EC", "-----BEGIN"] {
        assert!(
            !json.contains(forbidden),
            "alloc_status response must carry NO cert bytes / private key; found {forbidden:?}"
        );
    }
}
