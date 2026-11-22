//! Integration — `alloc status` lists, for each running allocation, the
//! certificate issued FOR THAT ALLOCATION, even when sibling allocations of the
//! workload share one SPIFFE identity (workload-spiffe-identity D2, ADR-0345).
//!
//! Two allocations of one workload present the same identity
//! `spiffe://overdrive.local/workload/<w>`, so the identity cannot attribute a
//! certificate to an allocation; the audit row's `issued_for` does. The
//! projection joins on `issued_for = Allocation(row.alloc_id)`, then selects the
//! max `IssuanceOrdinal` among that allocation's rows.
//!
//! Default-lane in-process (`AppState::new` over sim stores), like its sibling.
//! CONTRACT_SHAPE: bounded-change.

#![allow(clippy::expect_used, clippy::expect_fun_call, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use overdrive_control_plane::AppState;
use overdrive_control_plane::api::AllocStatusResponse;
use overdrive_control_plane::handlers::{AllocStatusQuery, alloc_status};
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{DriverInput, IntentKey, Job, JobSpecInput, ResourcesInput};
use overdrive_core::byte_size::ByteSize;
use overdrive_core::ca::issued_certificate_row::{IssuedCertificateRow, IssuedFor};
use overdrive_core::cpu_quantity::CpuQuantity;
use overdrive_core::id::{AllocationId, CertSerial, IssuanceOrdinal, NodeId, SpiffeId, WorkloadId};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore, ObservationWrite,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use overdrive_testing::overdrive_test;
use tempfile::TempDir;

const JOB_ID: &str = "ws-shared-identity";
const ISSUED_AT_SECS: u64 = 1_700_000_005;

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
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
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
        crate::integration::serve_ports::worker(),
        crate::integration::serve_ports::owner(),
        crate::integration::serve_ports::exec_gate(),
        crate::integration::serve_ports::pool(),
    )
}

fn sample_spec() -> JobSpecInput {
    JobSpecInput {
        id: JOB_ID.to_string(),
        replicas: 2,
        resources: ResourcesInput {
            cpu: CpuQuantity::from_millis(500),
            memory: ByteSize::new(134_217_728),
        },
        driver: DriverInput::Vm(overdrive_core::aggregate::VmInput {
            volume: None,
            command: "/usr/local/bin/ws".to_string(),
            args: vec![],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
    }
}

async fn install_job(state: &AppState, spec: JobSpecInput) -> Job {
    let job = Job::from_submit(spec).expect("Job::from_submit must succeed for fixture");
    let key = IntentKey::for_workload(&job.id);
    let archived = overdrive_core::aggregate::WorkloadIntent::Job(job.clone())
        .archive_for_store()
        .expect("rkyv archive");
    state.store.put(key.as_bytes(), archived.as_ref()).await.expect("IntentStore put");
    job
}

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
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    };
    state
        .obs
        .write_alloc_lifecycle(
            row,
            overdrive_core::traits::observation_store::TransitionSource::Reconciler,
        )
        .await
        .expect("obs write");
}

async fn write_issued_cert(
    state: &AppState,
    serial: &str,
    spiffe: &SpiffeId,
    ordinal: u64,
    issued_for: IssuedFor,
) {
    let at = UnixInstant::from_unix_duration(Duration::from_secs(ISSUED_AT_SECS));
    let row = IssuedCertificateRow {
        serial: CertSerial::new(serial).expect("valid serial"),
        spiffe_id: spiffe.clone(),
        issuer_serial: CertSerial::new("00").expect("valid issuer serial"),
        not_before: at,
        not_after: at + Duration::from_secs(3600),
        node_id: writer_node(),
        issued_at: at,
        issuance_ordinal: IssuanceOrdinal::new(ordinal),
        issued_for,
    };
    state.obs.write(ObservationWrite::IssuedCertificate(row)).await.expect("obs write");
}

/// Two running allocations of one workload present ONE identity; each holds its
/// own certificate (own serial, own ordinal). `alloc status` lists exactly each
/// allocation's own certificate: two summaries, two distinct serials, both
/// naming the workload identity. A gateway SVID row sharing the table and the
/// issuance ordinal space is never attributed to either allocation.
#[overdrive_test(guest = "cgroup-default")]
#[tokio::test]
async fn alloc_status_lists_each_allocations_own_certificate_when_they_share_an_identity() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);
    let job = install_job(&state, sample_spec()).await;
    let first = AllocationId::from_str("alloc-shared-0").expect("valid alloc id");
    let second = AllocationId::from_str("alloc-shared-1").expect("valid alloc id");
    let identity = SpiffeId::for_workload(&job.id);

    write_running_row(&state, &first, &job.id).await;
    write_running_row(&state, &second, &job.id).await;
    // The first allocation's certificate has the LOWER ordinal; an
    // identity-keyed join would give both allocations the higher-ordinal one.
    write_issued_cert(&state, "0a0a0a0a0a0a", &identity, 0, IssuedFor::Allocation(first)).await;
    write_issued_cert(&state, "0b0b0b0b0b0b", &identity, 1, IssuedFor::Allocation(second)).await;
    // A gateway certificate with the highest ordinal and a different identity.
    let gateway = SpiffeId::for_gateway(&writer_node());
    write_issued_cert(&state, "0c0c0c0c0c0c", &gateway, 2, IssuedFor::Gateway).await;

    let body: AllocStatusResponse = alloc_status(
        State(state.clone()),
        Query(AllocStatusQuery { job: Some(JOB_ID.to_owned()) }),
    )
    .await
    .expect("alloc_status returned err")
    .0;

    let serials: BTreeSet<CertSerial> =
        body.issued_certificates.iter().map(|summary| summary.serial.clone()).collect();
    assert_eq!(
        serials,
        BTreeSet::from([
            CertSerial::new("0a0a0a0a0a0a").unwrap(),
            CertSerial::new("0b0b0b0b0b0b").unwrap(),
        ]),
        "each running allocation lists its own certificate: {:?}",
        body.issued_certificates
    );
    assert_eq!(body.issued_certificates.len(), 2, "one summary per running allocation");
    assert!(
        body.issued_certificates.iter().all(|summary| summary.spiffe_id == identity),
        "both certificates name the workload identity"
    );
}
