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
use futures::StreamExt as _;
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
/// Pre-`assign`s every `<job>` the rows declare into the SHARED allocator —
/// modeling the 01-05 deploy-time assigner having bound `<job> → F` BEFORE the
/// backend appeared (REV-3: `frontend_for` is a PURE READER, so a
/// resolvable-but-unassigned `<job>` would be WITHHELD; the assigner-runs-first
/// is the production precondition these tests stand in for).
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
// The relist-on-`Lagged` arm must be the ONLY path that makes `present`
// resolvable — otherwise the test is vacuous (REV-2: a List-at-probe that
// already sees `present` lets `await_answer`'s first synchronous `answer_for`
// return `want` immediately, before any `yield_now`, so the spawned drain never
// runs and the `Lagged → relist` arm never executes).
//
// The `RelistRecoversStore` double makes the List-at-probe return EMPTY and the
// post-`Lagged` relist return the `present` healthy row, sequenced by a
// deterministic call counter on `all_service_backends_rows` (NOT timing):
//   - call #1 (the probe's List leg) → `Ok(vec![])` → `present` NOT resolvable;
//   - call #2+ (the relist-on-`Lagged`) → `Ok(vec![present healthy row])`.
// So `present` becomes resolvable ONLY via the Lagged-triggered relist: a mutant
// deleting the `Lagged` arm (or the `relist_into` inside it) leaves `present`
// unresolved forever and the test goes RED.
// ---------------------------------------------------------------------------

/// A store double whose List leg is sequenced by a deterministic call counter so
/// the relist-on-`Lagged` recovery is the ONLY path that makes a name resolvable:
/// `all_service_backends_rows` call #1 (the probe's List leg) returns EMPTY; call
/// #2+ (the relist driven by the single emitted `Lagged`) returns `rows`. The
/// watch hands one `SubscriptionEvent::Lagged` then ends.
struct RelistRecoversStore {
    inner: Arc<SimObservationStore>,
    /// Counts `all_service_backends_rows` invocations: #1 = the probe's List
    /// leg (empty), #2+ = the relist-on-`Lagged` (returns `rows`).
    list_calls: Arc<std::sync::atomic::AtomicUsize>,
    /// The authoritative snapshot the relist recovers (returned from call #2+).
    rows: Vec<ServiceBackendRow>,
}

#[async_trait]
impl ObservationStore for RelistRecoversStore {
    async fn write(&self, row: ObservationRow) -> Result<(), ObservationStoreError> {
        self.inner.write(row).await
    }

    async fn subscribe_all_events(&self) -> Result<LagAwareSubscription, ObservationStoreError> {
        // One Lagged, then STAY PENDING (never end) — the drain must relist to
        // recover. The trailing `pending()` matters: if the stream ENDED after the
        // single `Lagged`, the drain's next `next().await` would observe stream-end
        // and fault the watch (`name_index.rs:414`) right after the recovery,
        // withholding `present` again. A live-but-quiet watch is the post-recovery
        // steady state we are asserting.
        let stream = futures::stream::iter(vec![SubscriptionEvent::Lagged { missed: 7 }])
            .chain(futures::stream::pending());
        Ok(Box::new(Box::pin(stream)) as LagAwareSubscription)
    }

    async fn all_service_backends_rows(
        &self,
    ) -> Result<Vec<ServiceBackendRow>, ObservationStoreError> {
        // Deterministic sequencing (not timing): the FIRST call is the probe's
        // List leg → EMPTY (so `present` is NOT resolvable before the watch); the
        // SECOND+ call is the relist-on-`Lagged` → the authoritative `rows` (the
        // ONLY path that makes `present` resolvable).
        let call = self.list_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if call == 0 { Ok(vec![]) } else { Ok(self.rows.clone()) }
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
/// an async drain path. The List-at-probe returns EMPTY, so the ONLY path that
/// makes `present` resolvable is the `Lagged`-triggered relist (call #2 of
/// `all_service_backends_rows`). A mutant that drops the `Lagged → relist_into`
/// arm leaves `present` unresolved forever (the test goes RED).
#[tokio::test]
async fn idx_03_relist_on_lagged_reflects_store_state() {
    let sim = fresh_store();
    let allocator = FrontendAddrAllocator::new();
    let present_name = mesh_name("present");
    let absent_name = mesh_name("absent");

    // State S (recovered by the relist): `present` has a healthy backend;
    // `absent` has none. The 01-05 deploy-time assigner has bound `present → F`.
    let s_rows = vec![backends_row(1, vec![backend_for("present", 1, true)], 1)];
    records_of(&allocator, &present_name); // assign present → F (deploy-time).

    let store: Arc<dyn ObservationStore> = Arc::new(RelistRecoversStore {
        inner: Arc::clone(&sim),
        list_calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        rows: s_rows,
    });
    let index = NameIndex::new(store, allocator.clone());
    // Probe's List leg (call #1) returns EMPTY → `present` is NOT yet resolvable;
    // the watch then emits one Lagged → relist (call #2) recovers S.
    index.probe().await.expect("probe Lists EMPTY and opens the lagging watch");

    // `present` becomes resolvable ONLY via the Lagged-triggered relist: the
    // first synchronous `answer_for` is NxDomain (List was empty) so `await_answer`
    // yields, the drain runs, the `Lagged` arm relists S, and `present` resolves.
    let want = records_of(&allocator, &present_name);
    assert_eq!(
        await_answer(&index, &present_name, &want).await,
        want,
        "after relist-on-Lagged (the ONLY resolvable path), `present` resolves to its stable F",
    );
    assert_eq!(
        answer_for(&absent_name, RecordType::A, &index),
        NameAnswer::NxDomain,
        "after relist-on-Lagged, a <job> absent from S does not resolve",
    );
}

// ---------------------------------------------------------------------------
// S-DBN-IDX-03 (fail-closed) — a relist-on-`Lagged` whose store read FAILS
// faults the watch, so a previously-resolvable name WITHHOLDS (fail-closed).
// Covers the Lagged-relist-FAILURE branch (`name_index.rs:404-406`, the
// `watch_healthy.store(false)` set on `relist_into(...).is_err()`).
//
// The `RelistFailsStore` double makes the List-at-probe SUCCEED (`present`
// resolves) but the post-`Lagged` relist return `Err`, sequenced by the same
// deterministic call counter:
//   - call #1 (the probe's List leg) → `Ok(vec![present healthy row])`;
//   - call #2  (the relist-on-`Lagged`) → `Err(ObservationStoreError)`.
// So `present` is resolvable AFTER probe, then the failed relist faults the
// watch and `present` WITHHOLDS. A mutant deleting the `watch_healthy.store(false)`
// at line 405 keeps `present` resolving — the test goes RED.
// ---------------------------------------------------------------------------

/// A store double whose List leg is sequenced by a call counter so the
/// relist-on-`Lagged` FAILS: `all_service_backends_rows` call #1 (the probe's
/// List leg) returns `rows` (so `present` resolves); call #2 (the relist driven
/// by the single emitted `Lagged`) returns `Err`. The watch hands one
/// `SubscriptionEvent::Lagged` then ends.
struct RelistFailsStore {
    inner: Arc<SimObservationStore>,
    /// Counts `all_service_backends_rows` invocations: #1 = the probe's List
    /// leg (returns `rows`), #2 = the relist-on-`Lagged` (returns `Err`).
    list_calls: Arc<std::sync::atomic::AtomicUsize>,
    /// The authoritative snapshot the List leg returns (from call #1).
    rows: Vec<ServiceBackendRow>,
}

#[async_trait]
impl ObservationStore for RelistFailsStore {
    async fn write(&self, row: ObservationRow) -> Result<(), ObservationStoreError> {
        self.inner.write(row).await
    }

    async fn subscribe_all_events(&self) -> Result<LagAwareSubscription, ObservationStoreError> {
        // One Lagged, then end — the drain relists, and call #2 fails.
        let stream = futures::stream::iter(vec![SubscriptionEvent::Lagged { missed: 7 }]);
        Ok(Box::new(Box::pin(stream)) as LagAwareSubscription)
    }

    async fn all_service_backends_rows(
        &self,
    ) -> Result<Vec<ServiceBackendRow>, ObservationStoreError> {
        // Deterministic sequencing (not timing): the FIRST call is the probe's
        // List leg → succeeds with `rows` (so `present` resolves after probe);
        // the SECOND call is the relist-on-`Lagged` → `Err`, faulting the watch.
        let call = self.list_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if call == 0 {
            Ok(self.rows.clone())
        } else {
            Err(ObservationStoreError::Unreachable { peer: "relist-failed".to_owned() })
        }
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

/// `#[tokio::test]` (single shared runtime) — the Lagged-relist-FAILURE fault.
/// The List-at-probe SUCCEEDS so `present` is resolvable; the `Lagged`-triggered
/// relist (call #2) returns `Err`, faulting the watch, so `present` WITHHOLDS
/// fail-closed thereafter. A mutant deleting `watch_healthy.store(false)` at
/// `name_index.rs:405` keeps `present` resolving forever (the test goes RED).
#[tokio::test]
async fn idx_03_relist_on_lagged_failure_faults_the_watch_fail_closed() {
    let sim = fresh_store();
    let allocator = FrontendAddrAllocator::new();
    let present_name = mesh_name("present");

    // State S the List leg returns: `present` has a healthy backend. The 01-05
    // deploy-time assigner has bound `present → F`.
    let s_rows = vec![backends_row(1, vec![backend_for("present", 1, true)], 1)];
    let f = allocator.assign(&present_name).expect("allocator has free addresses");

    let store: Arc<dyn ObservationStore> = Arc::new(RelistFailsStore {
        inner: Arc::clone(&sim),
        list_calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        rows: s_rows,
    });
    let index = NameIndex::new(store, allocator.clone());
    // Probe's List leg (call #1) returns S → `present` IS resolvable; the watch
    // then emits one Lagged → relist (call #2) FAILS → the drain faults the watch.
    index.probe().await.expect("probe Lists S and opens the lagging watch");

    // After probe `present` was resolvable; once the failed relist faults the
    // watch, `frontend_for` WITHHOLDS (fail-closed) and the answer becomes
    // NxDomain — the first synchronous `answer_for` is Records([F]) (≠ NxDomain),
    // so `await_answer` yields, the drain runs the failing relist, faults, and the
    // next `answer_for` is NxDomain.
    assert_eq!(
        await_answer(&index, &present_name, &NameAnswer::NxDomain).await,
        NameAnswer::NxDomain,
        "a relist-on-Lagged failure faults the watch → previously-resolvable name WITHHOLDS",
    );
    // Guard the test's own premise: `present` WAS resolvable + assigned F before
    // the relist failed (so the NxDomain above is the FAULT withholding, not a
    // never-resolvable name).
    assert_ne!(
        NameAnswer::Records(vec![SocketAddrV4::new(f, 0)]),
        NameAnswer::NxDomain,
        "premise: present was resolvable and assigned F before the relist failed",
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

// ---------------------------------------------------------------------------
// S-DBN-IDX-04 (REV-3 root-cause fix) — frontend_for is a PURE READER: a DNS
// query for a resolvable <job> the allocator does NOT yet bind is WITHHELD
// (NxDomain) and leaves the allocator's snapshot BYTE-UNCHANGED (no
// assign-on-read). This is the falsifier the prior suite could not express
// (every prior pre-assert pre-`assign`ed the <job>, so assign-on-read and
// read-the-binding were indistinguishable). A mutant restoring
// `self.allocator.assign(name)` on the read path adds a binding AND flips the
// answer from NxDomain to Records — both halves go RED.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn idx_04_query_for_unassigned_job_is_withheld_and_does_not_mutate_the_allocator() {
    let store = fresh_store();
    let allocator = FrontendAddrAllocator::new();
    // The <job> is running-AND-healthy (so the resolvability gate is OPEN), but
    // the allocator does NOT bind it — the 01-05 deploy-time assigner has not
    // run for it. Built WITHOUT `index_listing` deliberately: that helper
    // pre-`assign`s every row's <job> (modeling the assigner), which is exactly
    // the precondition this test must NOT have. The index must READ the (absent)
    // binding, never WRITE one.
    store
        .write(ObservationRow::ServiceBackend(backends_row(
            1,
            vec![backend_for("unassigned", 1, true)],
            1,
        )))
        .await
        .expect("write resolvable row");
    let index = NameIndex::new(Arc::clone(&store) as Arc<dyn ObservationStore>, allocator.clone());
    index.probe().await.expect("probe Lists the resolvable (but unassigned) row");
    let name = mesh_name("unassigned");

    // Snapshot the allocator BEFORE the query.
    let before = allocator.snapshot();

    // A query for a resolvable-but-unassigned <job> is WITHHELD (NxDomain) —
    // NOT assigned-on-read.
    assert_eq!(
        answer_for(&name, RecordType::A, &index),
        NameAnswer::NxDomain,
        "a resolvable <job> the allocator does not yet bind is WITHHELD, never assigned-on-read",
    );

    // The query left the allocator BYTE-UNCHANGED — no binding appeared as a
    // side effect of the read (the single-source / pure-reader property).
    let after = allocator.snapshot();
    assert_eq!(
        after, before,
        "frontend_for is a PURE READER — a DNS query mutates no allocator binding",
    );
    assert!(
        !after.contains_key(&name),
        "no <job> → F binding was created on the read path (no assign-on-read)",
    );
}

// ---------------------------------------------------------------------------
// Fail-closed faulted-flag (Blocking #3) — once the drain dies (the watch
// terminates / a relist fails), a previously-resolvable name WITHHOLDS
// (NxDomain) rather than serving a stale liveness answer. Mirrors
// `ServiceBackendsResolve`'s `watch_healthy` faulted posture.
//
// The `EndingStore` hands the drain a subscription that immediately ends
// (stream-end) AFTER the List leg seeded a resolvable, assigned <job>. The
// drain sets `watch_healthy = false` on stream-end; `frontend_for` then
// withholds. A mutant that drops the stream-end fault keeps answering Records
// forever — the test goes RED.
// ---------------------------------------------------------------------------

/// A store double whose List leg delegates to a backing `SimObservationStore`
/// but whose watch ENDS IMMEDIATELY (an empty stream) — the drain observes a
/// terminal watch (stream end) and must fault, so a previously-resolvable name
/// is WITHHELD fail-closed thereafter.
struct EndingStore {
    inner: Arc<SimObservationStore>,
}

#[async_trait]
impl ObservationStore for EndingStore {
    async fn write(&self, row: ObservationRow) -> Result<(), ObservationStoreError> {
        self.inner.write(row).await
    }

    async fn subscribe_all_events(&self) -> Result<LagAwareSubscription, ObservationStoreError> {
        // An empty stream — the drain sees stream-end and faults the watch.
        let stream = futures::stream::iter(Vec::<SubscriptionEvent>::new());
        Ok(Box::new(Box::pin(stream)) as LagAwareSubscription)
    }

    async fn all_service_backends_rows(
        &self,
    ) -> Result<Vec<ServiceBackendRow>, ObservationStoreError> {
        self.inner.all_service_backends_rows().await
    }

    // The remaining surface is unused by NameIndex — delegate to the backing
    // SimObservationStore.
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

#[tokio::test]
async fn faulted_watch_withholds_a_previously_resolvable_name_fail_closed() {
    let sim = fresh_store();
    // State S: `svc` has a running-AND-healthy backend (so it is resolvable).
    sim.write(ObservationRow::ServiceBackend(backends_row(
        1,
        vec![backend_for("svc", 1, true)],
        1,
    )))
    .await
    .expect("seed resolvable row");
    let allocator = FrontendAddrAllocator::new();
    let name = mesh_name("svc");
    // The 01-05 assigner already bound <job> → F (the writer ran on deploy).
    let f = allocator.assign(&name).expect("allocator has free addresses");

    let store: Arc<dyn ObservationStore> = Arc::new(EndingStore { inner: Arc::clone(&sim) });
    let index = NameIndex::new(store, allocator.clone());
    // Probe Lists S (seeding `svc` resolvable) then opens the watch, whose
    // stream ends immediately → the drain faults the watch.
    index.probe().await.expect("probe Lists S and opens the (immediately-ending) watch");

    // Once the watch has faulted, `svc` is WITHHELD fail-closed — the index
    // serves no liveness answer from a signal it has stopped updating.
    let want_records = NameAnswer::Records(vec![SocketAddrV4::new(f, 0)]);
    assert_eq!(
        await_answer(&index, &name, &NameAnswer::NxDomain).await,
        NameAnswer::NxDomain,
        "after the watch faults (stream end), a previously-resolvable name WITHHOLDS (fail-closed)",
    );
    // Guard the test's own premise: `svc` WAS resolvable+assigned (so the
    // NxDomain above is the FAULT withholding, not a never-resolvable name).
    assert_ne!(
        want_records,
        NameAnswer::NxDomain,
        "premise: svc was resolvable and assigned F before the watch faulted",
    );
}

// ---------------------------------------------------------------------------
// One-service-per-job invariant (non-blocking, apply_row eviction safety) —
// asserted through the driving port. Two DISTINCT services each contribute a
// running-AND-healthy backend to the SAME <job>; when one service's row goes
// zero-healthy, the <job> STILL resolves (the other service's healthy backend
// is not stranded). Pins that the per-(<job>, service_id)-scoped eviction never
// drops a co-resident service's contribution — the property the eviction-site
// invariant comment documents.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn apply_row_one_service_per_job_eviction_does_not_strand_a_coresident_service() {
    let store = fresh_store();
    let allocator = FrontendAddrAllocator::new();
    let name = mesh_name("svc");
    // Two DISTINCT services (service_id 1 and 2) each contribute a healthy
    // backend to the SAME <job> `svc`, at DISTINCT addrs.
    let index = index_listing(
        &store,
        allocator.clone(),
        vec![
            backends_row(1, vec![backend_for("svc", 1, true)], 1),
            backends_row(2, vec![backend_for("svc", 2, true)], 1),
        ],
    )
    .await;
    // The 01-05 assigner bound <job> → F.
    let f = allocator.assign(&name).expect("allocator has free addresses");
    let want = NameAnswer::Records(vec![SocketAddrV4::new(f, 0)]);

    // Both contribute → resolvable.
    assert_eq!(answer_for(&name, RecordType::A, &index), want.clone());

    // Service 1's backend goes zero-healthy (a fresh full-row replace). The
    // per-(<job>, service_id) eviction drops ONLY service 1's contribution —
    // service 2's healthy backend keeps `svc` resolvable.
    store
        .write(ObservationRow::ServiceBackend(backends_row(
            1,
            vec![backend_for("svc", 1, false)],
            2,
        )))
        .await
        .expect("write service-1 zero-healthy row");
    // After the drain folds it, `svc` STILL resolves (service 2 is healthy) —
    // the co-resident service was not stranded by service 1's eviction.
    // Spin a bounded number of yields, asserting `svc` never drops to NxDomain.
    for _ in 0..1000 {
        tokio::task::yield_now().await;
        assert_eq!(
            answer_for(&name, RecordType::A, &index),
            want.clone(),
            "a co-resident healthy service keeps <job> resolvable when another service evicts",
        );
    }
}
