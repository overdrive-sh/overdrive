//! S-DBN-ANSWER-01..05 — the pure `answer_for(name, qtype, &index)` proptests
//! (Tier 1, default unit lane, in-process; ADR-0072 REV-2 "stable-frontend",
//! GH #243; roadmap 01-03 / DDN-4). `answer_for` is THE primary mutation-gate
//! target of the slice.
//!
//! Port-to-port discipline (Mandate M2 / M3): every property asserts THROUGH
//! the pinned `answer_for` + `NameIndex` public surface — never the index's
//! internal `by_name` map. The litmus: an `answer_for` that returns the wrong
//! arm (empty / extra / wrong-block addr, or a fabricated v6 addr) flips the
//! single-stable-F equality RED.
//!
//! The stable frontend `F` answered for a resolvable `<job>` is the
//! `FrontendAddrAllocator`'s binding (the SINGLE source of frontend truth) —
//! the tests assert `Records == vec![SocketAddrV4::new(F, 0)]` where `F` is read
//! back from `allocator.assign(<job>)` (idempotent), NEVER a per-instance
//! backend addr in `10.99.0.0/16`.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;

use hickory_proto::rr::RecordType;
use overdrive_control_plane::dns_responder::answer::answer_for;
use overdrive_control_plane::dns_responder::frontend_addr_allocator::{
    FrontendAddrAllocator, WORKLOAD_FRONTEND_BASE,
};
use overdrive_control_plane::dns_responder::name_index::NameIndex;
use overdrive_core::id::{MeshServiceName, NameAnswer, NodeId, ServiceId, SpiffeId};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationRow, ObservationStore, ServiceBackendRow,
};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategies + fixtures.
// ---------------------------------------------------------------------------

/// A valid `<job>` label: DNS-1123, starts + ends alphanumeric, single label,
/// short to keep generation cheap (the boundary is covered by the
/// `MeshServiceName` validation suite, not here).
fn arb_job_label() -> impl Strategy<Value = String> {
    "[a-z0-9]([a-z0-9-]{0,12}[a-z0-9])?"
        .prop_filter("no trailing/leading hyphen", |s| !s.starts_with('-') && !s.ends_with('-'))
}

/// A DISTINCT set of `<job>` labels of size `1..=n_max` (canonical-string
/// distinctness keeps the set genuinely n-element).
fn arb_distinct_jobs(n_max: usize) -> impl Strategy<Value = Vec<String>> {
    proptest::collection::hash_set(arb_job_label(), 1..=n_max)
        .prop_map(|labels| labels.into_iter().collect())
}

fn mesh_name(label: &str) -> MeshServiceName {
    MeshServiceName::new(&format!("{label}.{}", MeshServiceName::SUFFIX))
        .expect("generated label is a valid mesh service name")
}

fn fresh_store() -> Arc<SimObservationStore> {
    Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("valid node id"), 0))
}

/// A `Backend` whose `alloc` SVID carries `/job/<job>/alloc/...`, at a
/// per-instance backend addr in `10.99.0.0/16` (deliberately a DIFFERENT block
/// from the frontend `F` the answer must return).
fn backend_for(job: &str, instance: u8, healthy: bool) -> Backend {
    let spiffe = SpiffeId::new(&format!("spiffe://overdrive.local/job/{job}/alloc/a{instance}"))
        .expect("valid spiffe id");
    Backend {
        alloc: spiffe,
        addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 99, 0, instance), 8080)),
        weight: 1,
        healthy,
    }
}

fn backends_row(service_id: u64, backends: Vec<Backend>) -> ServiceBackendRow {
    ServiceBackendRow {
        service_id: ServiceId::new(service_id).expect("valid service id"),
        vip: Ipv4Addr::new(10, 96, 0, 1),
        backends,
        updated_at: LogicalTimestamp {
            counter: 1,
            writer: NodeId::new("local").expect("valid node id"),
        },
    }
}

/// The `<job>` mesh name a `Backend`'s `alloc` SVID
/// (`spiffe://overdrive.local/job/<job>/alloc/...`) dials as — mirrors the
/// production `name_index::job_of` extraction so the fixture can model the
/// 01-05 deploy-time assigner binding `<job> → F` on declaration.
fn job_of_backend(backend: &Backend) -> MeshServiceName {
    let mut segments = backend.alloc.path().split('/').filter(|s| !s.is_empty());
    let label = loop {
        match segments.next() {
            Some("job") => break segments.next().expect("job segment present"),
            Some(_) => {}
            None => panic!("backend SVID carries no /job/<job>/ segment"),
        }
    };
    mesh_name(label)
}

/// Build a `NameIndex` over a store seeded with `rows`, sharing `allocator`.
/// Writes the rows FIRST then probes, so List-at-probe seeds them. Pre-`assign`s
/// every `<job>` the rows declare into the SHARED allocator — modeling the 01-05
/// deploy-time assigner having bound `<job> → F` BEFORE the backend appeared
/// (REV-3: `frontend_for` is a PURE READER, so a resolvable-but-unassigned
/// `<job>` would be WITHHELD; the assigner-runs-first is the production
/// precondition these tests stand in for).
async fn index_listing(
    store: &Arc<SimObservationStore>,
    allocator: FrontendAddrAllocator,
    rows: Vec<ServiceBackendRow>,
) -> NameIndex {
    for row in &rows {
        for backend in &row.backends {
            // Idempotent per <job>; mirrors the deploy-time assign-on-declare.
            allocator.assign(&job_of_backend(backend)).expect("allocator has free addresses");
        }
    }
    for row in rows {
        store.write(ObservationRow::ServiceBackend(row)).await.expect("write service_backends row");
    }
    let index = NameIndex::new(Arc::clone(store) as Arc<dyn ObservationStore>, allocator);
    index.probe().await.expect("probe Lists the pre-existing rows");
    index
}

/// The expected `Records` answer for a resolvable `<job>`: exactly the single
/// stable frontend `F` the allocator binds, wrapped as `SocketAddrV4::new(F, 0)`.
fn expected_records(allocator: &FrontendAddrAllocator, name: &MeshServiceName) -> NameAnswer {
    let f = allocator.assign(name).expect("allocator has free addresses");
    NameAnswer::Records(vec![SocketAddrV4::new(f, 0)])
}

// ---------------------------------------------------------------------------
// S-DBN-ANSWER-01 — resolvable A → Records(vec![F]), F ∈ 10.98.0.0/16, single.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn answer_01_resolvable_a_returns_single_stable_frontend(
        jobs in arb_distinct_jobs(5),
    ) {
        tokio::runtime::Runtime::new().expect("rt").block_on(async {
            let store = fresh_store();
            let allocator = FrontendAddrAllocator::new();
            // Every job is resolvable: one running-AND-healthy backend each.
            let rows: Vec<ServiceBackendRow> = jobs
                .iter()
                .enumerate()
                .map(|(i, job)| {
                    let sid = u64::try_from(i + 1).expect("small");
                    backends_row(sid, vec![backend_for(job, u8::try_from(i + 1).expect("small"), true)])
                })
                .collect();
            let index = index_listing(&store, allocator.clone(), rows).await;

            for job in &jobs {
                let name = mesh_name(job);
                let answer = answer_for(&name, RecordType::A, &index);
                // Exactly the single stable frontend F — never a backend addr.
                prop_assert_eq!(
                    answer.clone(),
                    expected_records(&allocator, &name),
                    "resolvable {} must answer its single stable frontend F", name,
                );
                // The answered addr is in the frontend block, NOT 10.99.0.0/16.
                if let NameAnswer::Records(addrs) = answer {
                    prop_assert_eq!(addrs.len(), 1, "exactly one stable frontend addr");
                    prop_assert!(
                        WORKLOAD_FRONTEND_BASE.contains(addrs[0].ip()),
                        "answered F {} must be in the frontend block {}", addrs[0].ip(), WORKLOAD_FRONTEND_BASE,
                    );
                } else {
                    prop_assert!(false, "resolvable name must be Records, got {:?}", answer);
                }
            }
            Ok(())
        })?;
    }
}

// ---------------------------------------------------------------------------
// S-DBN-ANSWER-02 — absent / zero-healthy A → NxDomain, NEVER Records.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn answer_02_withheld_or_absent_a_returns_nxdomain(
        present in arb_job_label(),
        unhealthy in arb_job_label(),
        absent in arb_job_label(),
    ) {
        prop_assume!(present != unhealthy && present != absent && unhealthy != absent);
        tokio::runtime::Runtime::new().expect("rt").block_on(async {
            let store = fresh_store();
            let allocator = FrontendAddrAllocator::new();
            let rows = vec![
                backends_row(1, vec![backend_for(&present, 1, true)]),
                // `unhealthy` declared-but-not-running (0 healthy backends).
                backends_row(2, vec![backend_for(&unhealthy, 2, false)]),
            ];
            let index = index_listing(&store, allocator, rows).await;

            // The all-unhealthy name is WITHHELD.
            prop_assert_eq!(
                answer_for(&mesh_name(&unhealthy), RecordType::A, &index),
                NameAnswer::NxDomain,
                "all-unhealthy name must be NxDomain, never Records",
            );
            // The never-declared name is absent → NxDomain.
            prop_assert_eq!(
                answer_for(&mesh_name(&absent), RecordType::A, &index),
                NameAnswer::NxDomain,
                "absent name must be NxDomain",
            );
            Ok(())
        })?;
    }
}

// ---------------------------------------------------------------------------
// S-DBN-ANSWER-03 — AAAA → NoData on resolvable, NxDomain on withheld/absent.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn answer_03_aaaa_is_nodata_on_live_nxdomain_on_absent(
        live in arb_job_label(),
        absent in arb_job_label(),
    ) {
        prop_assume!(live != absent);
        tokio::runtime::Runtime::new().expect("rt").block_on(async {
            let store = fresh_store();
            let allocator = FrontendAddrAllocator::new();
            let rows = vec![backends_row(1, vec![backend_for(&live, 1, true)])];
            let index = index_listing(&store, allocator, rows).await;

            // Resolvable name, AAAA → NoData (never a fabricated v6 addr).
            prop_assert_eq!(
                answer_for(&mesh_name(&live), RecordType::AAAA, &index),
                NameAnswer::NoData,
                "AAAA on a resolvable name is NoData",
            );
            // Absent name, AAAA → NxDomain.
            prop_assert_eq!(
                answer_for(&mesh_name(&absent), RecordType::AAAA, &index),
                NameAnswer::NxDomain,
                "AAAA on an absent name is NxDomain",
            );
            Ok(())
        })?;
    }
}

// ---------------------------------------------------------------------------
// S-DBN-ANSWER-05 — a miss does not corrupt a co-resident hit (single example).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn answer_05_miss_does_not_corrupt_the_hit_path() {
    let store = fresh_store();
    let allocator = FrontendAddrAllocator::new();
    let rows = vec![backends_row(1, vec![backend_for("server", 1, true)])];
    let index = index_listing(&store, allocator.clone(), rows).await;

    let server = mesh_name("server");
    let nonexistent = mesh_name("nonexistent");

    // The miss is NxDomain.
    assert_eq!(
        answer_for(&nonexistent, RecordType::A, &index),
        NameAnswer::NxDomain,
        "a never-declared name is NxDomain",
    );
    // The hit still resolves to its single stable F.
    assert_eq!(
        answer_for(&server, RecordType::A, &index),
        expected_records(&allocator, &server),
        "the co-resident hit still answers its stable frontend F after a miss",
    );
}
