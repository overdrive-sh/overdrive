//! S-DBN-ANSWER-04 + S-DBN-IDX-01..04 — the List-then-Watch `NameIndex`
//! proptests (Tier 1, default unit lane, in-process; ADR-0072 REV-2
//! "stable-frontend", GH #243; roadmap 01-03 / DDN-2 / Finding-2 / OQ-1).
//!
//! Port-to-port discipline (Mandate M2 / M3): every property asserts THROUGH
//! `answer_for(name, qtype, &index)` (and the SHARED `FrontendAddrAllocator`'s
//! public `assign`/`snapshot`) — NEVER the index's internal `by_name` map. The
//! healthy gate is exercised as a WITHHOLD seam (resolvability), the
//! single-source-of-frontend-truth as the allocator's binding, and the
//! withhold-not-release as `answer_for → NxDomain` WHILE `snapshot()` retains
//! `<job> → F`.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;

use async_trait::async_trait;
use hickory_proto::rr::RecordType;
use overdrive_control_plane::dns_responder::answer::answer_for;
use overdrive_control_plane::dns_responder::frontend_addr_allocator::FrontendAddrAllocator;
use overdrive_control_plane::dns_responder::name_index::NameIndex;
use overdrive_core::id::{MeshServiceName, NameAnswer, NodeId, ServiceId, SpiffeId};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    LagAwareSubscription, LogicalTimestamp, ObservationRow, ObservationStore,
    ObservationStoreError, ServiceBackendRow, SubscriptionEvent,
};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategies + fixtures.
// ---------------------------------------------------------------------------

fn arb_job_label() -> impl Strategy<Value = String> {
    "[a-z0-9]([a-z0-9-]{0,12}[a-z0-9])?"
        .prop_filter("no trailing/leading hyphen", |s| !s.starts_with('-') && !s.ends_with('-'))
}

fn mesh_name(label: &str) -> MeshServiceName {
    MeshServiceName::new(&format!("{label}.{}", MeshServiceName::SUFFIX))
        .expect("generated label is a valid mesh service name")
}

fn fresh_store() -> Arc<SimObservationStore> {
    Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("valid node id"), 0))
}

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

fn backends_row(service_id: u64, backends: Vec<Backend>, counter: u64) -> ServiceBackendRow {
    ServiceBackendRow {
        service_id: ServiceId::new(service_id).expect("valid service id"),
        vip: Ipv4Addr::new(10, 96, 0, 1),
        backends,
        updated_at: LogicalTimestamp {
            counter,
            writer: NodeId::new("local").expect("valid node id"),
        },
    }
}

async fn index_listing(
    store: &Arc<SimObservationStore>,
    allocator: FrontendAddrAllocator,
    rows: Vec<ServiceBackendRow>,
) -> NameIndex {
    for row in rows {
        store.write(ObservationRow::ServiceBackend(row)).await.expect("write service_backends row");
    }
    let index = NameIndex::new(Arc::clone(store) as Arc<dyn ObservationStore>, allocator);
    index.probe().await.expect("probe Lists the pre-existing rows");
    index
}

/// Bounded spin until `answer_for(name, A)` reaches `want` (the background drain
/// folds rows asynchronously — no unbounded wait).
async fn await_answer(index: &NameIndex, name: &MeshServiceName, want: &NameAnswer) -> NameAnswer {
    let mut last = answer_for(name, RecordType::A, index);
    for _ in 0..1000 {
        if &last == want {
            return last;
        }
        tokio::task::yield_now().await;
        last = answer_for(name, RecordType::A, index);
    }
    last
}

fn records_of(allocator: &FrontendAddrAllocator, name: &MeshServiceName) -> NameAnswer {
    let f = allocator.assign(name).expect("allocator has free addresses");
    NameAnswer::Records(vec![SocketAddrV4::new(f, 0)])
}

// ---------------------------------------------------------------------------
// S-DBN-ANSWER-04 — healthy gate is the WITHHOLD seam, co-resident M resolves.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    #[test]
    fn answer_04_unhealthy_withheld_while_co_resident_resolves(
        unhealthy in arb_job_label(),
        healthy in arb_job_label(),
    ) {
        prop_assume!(unhealthy != healthy);
        tokio::runtime::Runtime::new().expect("rt").block_on(async {
            let store = fresh_store();
            let allocator = FrontendAddrAllocator::new();
            // N: unhealthy-only. M: running-and-healthy.
            let rows = vec![
                backends_row(1, vec![backend_for(&unhealthy, 1, false)], 1),
                backends_row(2, vec![backend_for(&healthy, 2, true)], 1),
            ];
            let index = index_listing(&store, allocator.clone(), rows).await;

            // The healthy gate WITHHOLDS the unhealthy-only name.
            prop_assert_eq!(
                answer_for(&mesh_name(&unhealthy), RecordType::A, &index),
                NameAnswer::NxDomain,
                "unhealthy-only <job> must be withheld (NxDomain)",
            );
            // The co-resident healthy name resolves to its stable F.
            prop_assert_eq!(
                answer_for(&mesh_name(&healthy), RecordType::A, &index),
                records_of(&allocator, &mesh_name(&healthy)),
                "co-resident healthy <job> resolves to its stable F",
            );
            Ok(())
        })?;
    }
}

// ---------------------------------------------------------------------------
// S-DBN-IDX-01 — List seeds at probe + Watch seeds a post-probe row.
//
// The watch half exercises the asynchronous single-owner drain (a spawned
// background task), so this is a `#[tokio::test]` single-example on ONE shared
// runtime — the methodology's wiring/timing layer, where a per-case `Runtime`
// in a proptest loop would starve the drain. The pure List-path + healthy-gate
// PROPERTIES live in `answer_04` / `answer_for`'s proptests.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn idx_01_list_at_probe_and_watch_make_name_resolvable() {
    // List half: row exists BEFORE probe → seeded by List-at-probe.
    let store = fresh_store();
    let allocator = FrontendAddrAllocator::new();
    let index = index_listing(
        &store,
        allocator.clone(),
        vec![backends_row(1, vec![backend_for("listed", 1, true)], 1)],
    )
    .await;
    let listed_name = mesh_name("listed");
    assert_eq!(
        answer_for(&listed_name, RecordType::A, &index),
        records_of(&allocator, &listed_name),
        "List-at-probe makes a pre-existing healthy row resolvable to its stable F",
    );

    // Watch half: a row written AFTER probe is folded by the drain.
    let watched_name = mesh_name("watched");
    assert_eq!(
        answer_for(&watched_name, RecordType::A, &index),
        NameAnswer::NxDomain,
        "watched name is absent before its row is written",
    );
    store
        .write(ObservationRow::ServiceBackend(backends_row(
            2,
            vec![backend_for("watched", 2, true)],
            1,
        )))
        .await
        .expect("write post-probe row");
    let want = records_of(&allocator, &watched_name);
    assert_eq!(
        await_answer(&index, &watched_name, &want).await,
        want,
        "the watch drain makes a post-probe healthy row resolvable to its stable F",
    );
}

// ---------------------------------------------------------------------------
// S-DBN-IDX-02 — withhold-not-release on the watch path (the F is retained).
//
// `#[tokio::test]` (single shared runtime) — exercises the async drain across a
// zero-healthy window then a healthy-again window. A mutant that RELEASES F on
// zero-healthy passes the NXDOMAIN check but fails the retained-F check.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn idx_02_zero_healthy_withholds_but_retains_the_same_frontend() {
    let store = fresh_store();
    let allocator = FrontendAddrAllocator::new();
    let index = index_listing(
        &store,
        allocator.clone(),
        vec![backends_row(1, vec![backend_for("svc", 1, true)], 1)],
    )
    .await;
    let name = mesh_name("svc");

    // Initially resolvable to F.
    let f_answer = records_of(&allocator, &name);
    let NameAnswer::Records(ref f_addrs) = f_answer else {
        unreachable!("setup: records_of always returns Records")
    };
    let f = *f_addrs[0].ip();
    assert_eq!(answer_for(&name, RecordType::A, &index), f_answer.clone());

    // A fresh row with the SAME backend now unhealthy → WITHHELD.
    store
        .write(ObservationRow::ServiceBackend(backends_row(
            1,
            vec![backend_for("svc", 1, false)],
            2,
        )))
        .await
        .expect("write zero-healthy row");
    assert_eq!(
        await_answer(&index, &name, &NameAnswer::NxDomain).await,
        NameAnswer::NxDomain,
        "a fresh zero-healthy row WITHHOLDS the name",
    );
    // ... but the allocator STILL binds <job> → the SAME F (withhold-not-release).
    assert_eq!(
        allocator.snapshot().get(&name).copied(),
        Some(f),
        "the allocator retains <job> → the SAME F across the zero-healthy window",
    );

    // A running-AND-healthy row returns → resolves to the SAME F (no churn).
    store
        .write(ObservationRow::ServiceBackend(backends_row(
            1,
            vec![backend_for("svc", 1, true)],
            3,
        )))
        .await
        .expect("write healthy-again row");
    assert_eq!(
        await_answer(&index, &name, &f_answer).await,
        f_answer,
        "re-resolution after healthy returns yields the SAME F (no churn)",
    );
}

// ---------------------------------------------------------------------------
// S-DBN-IDX-03 — relist-on-Lagged reflects store state S exactly.
//
// A store double whose subscription emits one `Lagged` (then nothing) forces
// the drain down the relist-on-Lagged path; after the relist the index must
// reflect the backing store's authoritative snapshot S.
// ---------------------------------------------------------------------------

/// A store double that delegates the List leg to a backing `SimObservationStore`
/// but hands the watch a subscription that emits a single `Lagged` (then ends).
/// This forces the drain's relist-on-`Lagged` recovery — the only path that can
/// reconcile the index with a snapshot it never saw a `Row` for.
struct LaggingStore {
    inner: Arc<SimObservationStore>,
}

#[async_trait]
impl ObservationStore for LaggingStore {
    async fn write(&self, row: ObservationRow) -> Result<(), ObservationStoreError> {
        self.inner.write(row).await
    }

    async fn subscribe_all_events(&self) -> Result<LagAwareSubscription, ObservationStoreError> {
        // One Lagged, then end — the drain must relist to recover.
        let stream = futures::stream::iter(vec![SubscriptionEvent::Lagged { missed: 7 }]);
        Ok(Box::new(Box::pin(stream)) as LagAwareSubscription)
    }

    async fn all_service_backends_rows(
        &self,
    ) -> Result<Vec<ServiceBackendRow>, ObservationStoreError> {
        self.inner.all_service_backends_rows().await
    }

    // The remaining surface is unused by NameIndex — delegate everything to
    // the backing SimObservationStore.
    async fn alloc_status_rows(
        &self,
    ) -> Result<Vec<overdrive_core::traits::observation_store::AllocStatusRow>, ObservationStoreError>
    {
        self.inner.alloc_status_rows().await
    }
    async fn alloc_status_row(
        &self,
        alloc_id: &overdrive_core::id::AllocationId,
    ) -> Result<
        Option<overdrive_core::traits::observation_store::AllocStatusRow>,
        ObservationStoreError,
    > {
        self.inner.alloc_status_row(alloc_id).await
    }
    async fn node_health_rows(
        &self,
    ) -> Result<Vec<overdrive_core::traits::observation_store::NodeHealthRow>, ObservationStoreError>
    {
        self.inner.node_health_rows().await
    }
    async fn issued_certificate_rows(
        &self,
    ) -> Result<
        Vec<overdrive_core::ca::issued_certificate_row::IssuedCertificateRow>,
        ObservationStoreError,
    > {
        self.inner.issued_certificate_rows().await
    }
    async fn next_issuance_ordinal(
        &self,
    ) -> Result<overdrive_core::id::IssuanceOrdinal, ObservationStoreError> {
        self.inner.next_issuance_ordinal().await
    }
    async fn write_probe_result(
        &self,
        row: overdrive_core::observation::ProbeResultRow,
    ) -> Result<(), ObservationStoreError> {
        self.inner.write_probe_result(row).await
    }
    async fn list_probe_results_for_alloc(
        &self,
        alloc_id: &overdrive_core::id::AllocationId,
    ) -> Result<Vec<overdrive_core::observation::ProbeResultRow>, ObservationStoreError> {
        self.inner.list_probe_results_for_alloc(alloc_id).await
    }
    async fn workflow_terminal_rows(
        &self,
    ) -> Result<
        Vec<(overdrive_core::id::CorrelationKey, overdrive_core::workflow::WorkflowStatus)>,
        ObservationStoreError,
    > {
        self.inner.workflow_terminal_rows().await
    }
    async fn workflow_signal(
        &self,
        key: &overdrive_core::workflow::SignalKey,
    ) -> Result<Option<overdrive_core::workflow::SignalValue>, ObservationStoreError> {
        self.inner.workflow_signal(key).await
    }
    async fn service_hydration_results_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<
        Vec<overdrive_core::traits::observation_store::ServiceHydrationResultRow>,
        ObservationStoreError,
    > {
        self.inner.service_hydration_results_rows(service_id).await
    }
    async fn service_backends_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ServiceBackendRow>, ObservationStoreError> {
        self.inner.service_backends_rows(service_id).await
    }
    async fn reconcile_conflict_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<
        Vec<overdrive_core::traits::observation_store::ReconcileConflictRow>,
        ObservationStoreError,
    > {
        self.inner.reconcile_conflict_rows(service_id).await
    }
}

/// `#[tokio::test]` (single shared runtime) — the relist-on-`Lagged` recovery is
/// an async drain path. A mutant that drops the `Lagged → relist_into` arm leaves
/// the `present` name unresolved forever (the test goes RED).
#[tokio::test]
async fn idx_03_relist_on_lagged_reflects_store_state() {
    let sim = fresh_store();
    // State S: `present` has a healthy backend; `absent` has none.
    sim.write(ObservationRow::ServiceBackend(backends_row(
        1,
        vec![backend_for("present", 1, true)],
        1,
    )))
    .await
    .expect("seed present row");
    let allocator = FrontendAddrAllocator::new();
    let store: Arc<dyn ObservationStore> = Arc::new(LaggingStore { inner: Arc::clone(&sim) });
    let index = NameIndex::new(store, allocator.clone());
    // Probe Lists S then opens the watch (which will emit Lagged → relist).
    index.probe().await.expect("probe Lists S and opens the lagging watch");

    let present_name = mesh_name("present");
    let absent_name = mesh_name("absent");
    // After the Lagged-triggered relist, the index reflects S exactly: `present`
    // resolves to its stable F; `absent` does not.
    let want = records_of(&allocator, &present_name);
    assert_eq!(
        await_answer(&index, &present_name, &want).await,
        want,
        "after relist-on-Lagged, a <job> healthy in S resolves to its stable F",
    );
    assert_eq!(
        answer_for(&absent_name, RecordType::A, &index),
        NameAnswer::NxDomain,
        "after relist-on-Lagged, a <job> absent from S does not resolve",
    );
}

// ---------------------------------------------------------------------------
// S-DBN-IDX-04 — single source of frontend truth (the allocator's binding).
//
// `#[tokio::test]` (single shared runtime) — the row-removal half drains a
// fresh empty-backend-set row asynchronously.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn idx_04_answered_f_is_the_allocators_binding_no_second_source() {
    let store = fresh_store();
    let allocator = FrontendAddrAllocator::new();
    let index = index_listing(
        &store,
        allocator.clone(),
        vec![backends_row(1, vec![backend_for("svc", 1, true)], 1)],
    )
    .await;
    let name = mesh_name("svc");

    // The answered F IS the allocator's binding for the <job> (idempotent).
    let f = allocator.assign(&name).expect("allocator has free addresses");
    assert_eq!(
        answer_for(&name, RecordType::A, &index),
        NameAnswer::Records(vec![SocketAddrV4::new(f, 0)]),
        "the answered F is byte-identical to the allocator's binding",
    );

    // Remove all rows for the <job> (empty backend set) and re-derive: the index
    // WITHHOLDS (no stale retention) WHILE the allocator still binds <job> → F
    // (the binding is the allocator's, not the index's).
    store
        .write(ObservationRow::ServiceBackend(backends_row(1, vec![], 2)))
        .await
        .expect("write empty-backend-set row");
    assert_eq!(
        await_answer(&index, &name, &NameAnswer::NxDomain).await,
        NameAnswer::NxDomain,
        "removing all rows withholds the name (no stale retention at the index)",
    );
    assert_eq!(
        allocator.snapshot().get(&name).copied(),
        Some(f),
        "the allocator still binds <job> → F (the single source of frontend truth)",
    );
}
