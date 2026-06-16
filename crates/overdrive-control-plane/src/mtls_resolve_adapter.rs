//! `ServiceBackendsResolve` — the v1 host [`MtlsResolve`] adapter
//! (transparent-mtls-enrollment, ADR-0071; GH #178 anti-corruption boundary).
//!
//! # Adapter home (criterion 7)
//!
//! This adapter lives in `overdrive-control-plane` (`crate_class =
//! "adapter-host"`), NOT `overdrive-dataplane` and NOT a new crate. The
//! rationale, per the roadmap Reuse Analysis (EXTEND an existing adapter-host
//! crate):
//!
//! - The [`ObservationStore`] read surface it consumes is an adapter-host-side
//!   concern, and this crate already wires `ObservationStore`-backed adapters
//!   (`veth_provisioner`, `action_shim`).
//! - This crate is the boot composition site (`run_server`) where the resolve
//!   probe must refuse on an unreadable store (the Earned-Trust gate, wired in a
//!   LATER step — 04-02).
//! - `overdrive-dataplane` is the kTLS / map executor (the wrong concern for a
//!   per-connection resolve that reads an observation surface).
//! - A NEW crate is rejected (Reuse Analysis: EXTEND, do not CREATE-NEW).
//!
//! # What it is (the #178 v1 SHELL)
//!
//! `ServiceBackendsResolve` implements the [`MtlsResolve`] driven port by
//! resolving each captured connection's `orig_dst` against the mesh's `running`
//! backend set, read from `service_backends` via [`ObservationStore`]. It is the
//! v1 SHELL: it returns `expected_svid: None` for EVERY backend and does NOT
//! thread `IdentityRead` (the expected-SVID join is GH #178 — threading it here
//! is a boundary-divergence rejection per CLAUDE.md § "Implement to the design",
//! consistent with the C2 sub-decision and the shipped 01-01 port rustdoc).
//!
//! # Read mechanism (C4 — the in-RAM address-keyed reverse index)
//!
//! [`MtlsResolve::resolve`] is handed an arbitrary `orig_dst: SocketAddrV4` and
//! holds NO `ServiceId`; the only `ObservationStore` backend-read surface
//! (`service_backends_rows(service_id)`) is keyed by `ServiceId`, so a
//! per-`ServiceId` point query is the WRONG surface. Per C4 (feature-delta
//! § "C4 — resolve READ MECHANISM" / D-TME-11) the adapter instead resolves
//! against an in-RAM, address-keyed reverse index (`addr → Backend`) of the
//! `running` `service_backends` set, built and refreshed from the EXISTING
//! [`ObservationStore::subscribe_all`] observation surface (the same forward
//! `Stream<Item = ObservationRow>` the reconciler runtime already uses) — NOT a
//! per-`ServiceId` point query, and WITHOUT adding any new trait method. The
//! in-RAM index is an adapter-internal private detail; the PUBLIC [`MtlsResolve`]
//! contract is unchanged. This is the industry-canonical shape (Cilium's
//! `ipcache`: an in-RAM addr→identity reverse index populated by event
//! subscription, consulted per connection).
//!
//! Headless v1 (D-TME-10): the addr DNS returns IS the backend addr, so the
//! index is keyed by the backend addr DIRECTLY — there is NO VIP→backend
//! translation in the resolve path (that is #167/#61, out of scope).
//!
//! # Classification (C1 + C4, verbatim with the shipped 01-01 port rustdoc)
//!
//! - `orig_dst` HITS a `running`-and-healthy mesh backend in the index →
//!   [`Mesh(ResolvedBackend { addr, expected_svid: None })`](MtlsResolution::Mesh).
//! - `orig_dst` MISSES (no mesh backend), index readable →
//!   [`NonMesh`](MtlsResolution::NonMesh) (cleartext pass-through, by design).
//!   **A miss is `NonMesh`, NOT `MeshUnreachable`** — making a miss fail-closed
//!   would break legitimate external / non-mesh egress (C4 scoping note); the
//!   bounded cleartext edge is closed by the headless single-source invariant.
//! - A matched backend is **present-but-unreachable** (`Backend.healthy ==
//!   false` — the readiness gate, recomputed from probe results by
//!   `service_lifecycle`) →
//!   [`MeshUnreachable`](MtlsResolution::MeshUnreachable) (fail-closed, NO
//!   cleartext).
//! - A store-layer READ FAULT (an errored [`subscribe_all`](ObservationStore::subscribe_all)
//!   at probe/refresh time) surfaces per the 01-01 error split as an `Err` of
//!   [`MtlsResolveError::StoreUnreadable`] — NOT `MeshUnreachable` (the
//!   contract's asymmetry, preserved verbatim).
//!
//! # Earned-Trust probe (criterion 4)
//!
//! [`probe`](MtlsResolve::probe) demonstrates the adapter can read the
//! `service_backends` surface (it opens a [`subscribe_all`](ObservationStore::subscribe_all)
//! subscription and refreshes the index from the store). On an unreadable store
//! it returns a structured [`MtlsResolveError::Probe`] (`health.startup.refused`-
//! shaped) and the node MUST refuse to start — it NEVER silently returns
//! empty / `NonMesh` (silent-empty degrading to silent pass-through IS the
//! silent-cleartext footgun the enrollment model exists to remove).
//!
//! # Dependency discipline
//!
//! [`ServiceBackendsResolve::new`] takes its [`ObservationStore`] as a
//! **mandatory constructor parameter** (`Arc<dyn ObservationStore>`) — REQUIRED,
//! not defaulted, no builder (`.claude/rules/development.md` § "Port-trait
//! dependencies"). `Send + Sync + 'static` (held as `Arc<dyn MtlsResolve>`).
//!
//! [`probe`]: MtlsResolve::probe
//! [`subscribe_all`]: ObservationStore::subscribe_all

use std::collections::BTreeMap;
use std::net::{SocketAddr, SocketAddrV4};
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use overdrive_core::id::ServiceId;
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::mtls_resolve::{
    MtlsResolution, MtlsResolve, MtlsResolveError, ResolvedBackend, Result,
};
use overdrive_core::traits::observation_store::{
    ObservationRow, ObservationStore, ObservationSubscription,
};
use parking_lot::RwLock;

/// The in-RAM, address-keyed reverse index (`addr → Backend`) of the
/// `running` `service_backends` set — the C4 read-mechanism private detail.
///
/// Keyed by [`SocketAddrV4`] (a [`BTreeMap`], not `HashMap` — the index is
/// observable under DST and its iteration order must be deterministic across
/// seeds, § "Ordered-collection choice"). A per-`service_id` secondary map
/// records which addrs a given service currently contributes, so an updated
/// row for that service REPLACES its prior addrs (the index never strands a
/// stale backend after a service's backend set shrinks).
#[derive(Default)]
struct BackendIndex {
    /// `addr → Backend` — the point-lookup surface `resolve` consults. Only
    /// V4 backends are indexed (a V6 `Backend.addr` never matches a V4
    /// `orig_dst`, so it is simply not inserted).
    by_addr: BTreeMap<SocketAddrV4, Backend>,
    /// `service_id → the V4 addrs that service currently contributes`. On a
    /// new row for a service, its prior addrs are removed from
    /// [`by_addr`](Self::by_addr) before the new set is inserted, so a shrunk
    /// or replaced backend set leaves no stale entries.
    addrs_by_service: BTreeMap<ServiceId, Vec<SocketAddrV4>>,
}

impl BackendIndex {
    /// Apply one full `service_backends` row to the index: drop the service's
    /// prior addrs, then insert its current V4 backends. Full-row replacement
    /// mirrors the `service_backends` §4 full-row-write contract — the row
    /// carries the service's entire current backend set.
    fn apply_row(&mut self, service_id: ServiceId, backends: &[Backend]) {
        if let Some(stale) = self.addrs_by_service.remove(&service_id) {
            for addr in stale {
                self.by_addr.remove(&addr);
            }
        }
        let mut contributed = Vec::new();
        for backend in backends {
            if let SocketAddr::V4(v4) = backend.addr {
                self.by_addr.insert(v4, backend.clone());
                contributed.push(v4);
            }
        }
        self.addrs_by_service.insert(service_id, contributed);
    }

    /// Point-lookup `orig_dst` and CLASSIFY it into an [`MtlsResolution`] arm
    /// (the pure classification the mutation gate targets — C1/C4):
    ///
    /// - a `running`-and-`healthy` match → `Mesh { addr, expected_svid: None }`
    ///   (`expected_svid` is `None` for every backend in v1 — the identity
    ///   join is #178);
    /// - a present-but-`healthy == false` match → `MeshUnreachable`
    ///   (fail-closed, the readiness-gate "present but unreachable" arm);
    /// - a miss → `NonMesh` (cleartext pass-through, by design — a miss is
    ///   NEVER `MeshUnreachable` in v1).
    fn classify(&self, orig_dst: SocketAddrV4) -> MtlsResolution {
        match self.by_addr.get(&orig_dst) {
            Some(backend) if backend.healthy => {
                MtlsResolution::Mesh(ResolvedBackend { addr: orig_dst, expected_svid: None })
            }
            Some(_) => MtlsResolution::MeshUnreachable,
            None => MtlsResolution::NonMesh,
        }
    }
}

/// The v1 host [`MtlsResolve`] adapter — resolves `orig_dst` against an in-RAM
/// reverse index of the `running` `service_backends` set, read from
/// [`ObservationStore`]. See the module rustdoc for the full contract.
pub struct ServiceBackendsResolve {
    /// The backing observation surface, injected as a **mandatory** constructor
    /// parameter (no default, no builder). The adapter reads `service_backends`
    /// rows from it via [`ObservationStore::subscribe_all`].
    store: Arc<dyn ObservationStore>,
    /// The C4 in-RAM `addr → Backend` reverse index. Behind a
    /// [`parking_lot::RwLock`] so `resolve` (read) and the index refresh (write)
    /// can share it across the `&self` trait methods without holding a lock
    /// across `.await`.
    index: RwLock<BackendIndex>,
    /// The PERSISTENT [`subscribe_all`](ObservationStore::subscribe_all)
    /// subscription, established lazily on the first probe/resolve and HELD for
    /// the adapter's lifetime. This is the C4 read mechanism: the broadcast
    /// observation surface is forward-only (a subscription does NOT replay rows
    /// written before it — verified by `LocalObservationStore`'s
    /// "subscription opened after write must not replay historical rows"
    /// acceptance test), so a fresh subscription per call would observe nothing.
    /// One held subscription continuously receives every `service_backends`
    /// write from the moment it is opened; each probe/resolve drains the
    /// currently-ready rows into the index. Behind a [`tokio::sync::Mutex`]
    /// because it is mutated (drained) across the `.await` on the stream's
    /// `next()`. `None` until the first successful subscribe.
    subscription: tokio::sync::Mutex<Option<ObservationSubscription>>,
}

impl ServiceBackendsResolve {
    /// Construct the adapter from its REQUIRED [`ObservationStore`]. Mandatory,
    /// not defaulted, no builder — a caller that forgets the store fails to
    /// construct (`.claude/rules/development.md` § "Port-trait dependencies").
    /// The index starts empty and no subscription is open yet;
    /// [`probe`](MtlsResolve::probe) establishes the held subscription (the
    /// Earned-Trust "wire → probe → use" gate) and every probe/resolve refreshes
    /// the index from it.
    #[must_use]
    pub fn new(store: Arc<dyn ObservationStore>) -> Self {
        Self {
            store,
            index: RwLock::new(BackendIndex::default()),
            subscription: tokio::sync::Mutex::new(None),
        }
    }

    /// Ensure the persistent `subscribe_all` subscription is open, then drain
    /// every CURRENTLY-ready `service_backends` row into the in-RAM index. This
    /// is the C4 "build/refresh from the existing observation surface"
    /// mechanism — a SINGLE long-lived subscription, opened once and held, so
    /// the forward-only broadcast surface delivers every write from the moment
    /// the subscription is established (a fresh subscription per call would
    /// observe nothing, since the surface does not replay history). A store
    /// subscription fault is the `StoreUnreadable`/`Probe` read-fault the
    /// resolve-time and probe-time callers surface.
    ///
    /// The drain is non-blocking past the currently-ready items: it pulls every
    /// row already buffered on the subscription (`now_or_never`) and stops at
    /// the first pending poll, so it never blocks waiting for a future write.
    /// The index write-lock is taken and released INSIDE the loop per row —
    /// never held across the `.await` on the stream
    /// (`.claude/rules/development.md` § "Never hold a lock across `.await`").
    async fn refresh_index(&self) -> std::result::Result<usize, String> {
        use futures::FutureExt;

        // Take the held subscription OUT of the mutex (establishing it on the
        // first call), drain it WITHOUT holding the guard across the loop, then
        // restore it. Taking-then-restoring keeps the mutex guard's scope tight
        // (it is dropped before the drain) — the subscription is owned locally
        // for the duration of the drain, so no guard spans the `now_or_never`
        // polls. A store-layer subscription fault surfaces from
        // `subscribe_all` here.
        // Bind the take into a local so the mutex guard temporary drops
        // immediately (before the match) — not held across the
        // `subscribe_all().await` arm (`clippy::significant_drop_in_scrutinee`).
        let taken = self.subscription.lock().await.take();
        let mut subscription = match taken {
            Some(existing) => existing,
            None => self.store.subscribe_all().await.map_err(|err| err.to_string())?,
        };

        let mut ingested = 0usize;
        // Drain only the rows already ready on the subscription; stop at the
        // first not-yet-ready poll so the refresh is bounded and never awaits a
        // future write. The index write-lock is taken and released per row —
        // never held across the `.await`-bearing stream poll.
        while let Some(Some(row)) = subscription.next().now_or_never() {
            if let ObservationRow::ServiceBackend(row) = row {
                self.index.write().apply_row(row.service_id, &row.backends);
                ingested += 1;
            }
        }

        // Restore the (now-drained) subscription so the NEXT refresh continues
        // observing from where this one stopped — the held subscription is
        // forward-only and must persist across calls.
        *self.subscription.lock().await = Some(subscription);
        Ok(ingested)
    }
}

#[async_trait]
impl MtlsResolve for ServiceBackendsResolve {
    async fn probe(&self) -> Result<()> {
        // Earned Trust: demonstrate the `service_backends` surface is readable
        // by opening a subscription and refreshing the index. An unreadable
        // store returns `Probe` (the `health.startup.refused`-shaped refusal) —
        // NEVER a silent empty/`NonMesh` (silent-empty degrading to silent
        // pass-through IS the silent-cleartext footgun the enrollment model
        // exists to remove).
        self.refresh_index()
            .await
            .map(|_ingested| ())
            .map_err(|reason| MtlsResolveError::Probe { reason })
    }

    async fn resolve(&self, orig_dst: SocketAddrV4) -> Result<MtlsResolution> {
        // Refresh the in-RAM index from the observation surface, then
        // point-lookup + classify. A store-layer read fault surfaces as
        // `StoreUnreadable` (NOT classified into `MeshUnreachable` — the 01-01
        // contract asymmetry: a store-layer fault is not a per-connection
        // classification).
        self.refresh_index()
            .await
            .map_err(|reason| MtlsResolveError::StoreUnreadable { reason })?;

        // Read-only point lookup + pure classification. The read guard is taken
        // AFTER the refresh `.await` returned and dropped at the end of this
        // expression — no lock is held across an `.await`.
        Ok(self.index.read().classify(orig_dst))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::net::Ipv4Addr;

    use overdrive_core::id::{NodeId, SpiffeId};
    use overdrive_core::traits::observation_store::{LogicalTimestamp, ServiceBackendRow};
    use overdrive_sim::adapters::observation_store::SimObservationStore;
    use proptest::prelude::*;

    use super::*;

    // ---- test fixtures -----------------------------------------------------

    /// A fresh single-peer `SimObservationStore` (the in-process DST double —
    /// tests NEVER reach for a host/production `ObservationStore`).
    fn fresh_store() -> Arc<SimObservationStore> {
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("valid node id"), 0))
    }

    fn v4(a: u8, b: u8, c: u8, d: u8, port: u16) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), port)
    }

    /// A `Backend` at the given V4 addr with the given readiness.
    fn backend(addr: SocketAddrV4, healthy: bool) -> Backend {
        Backend {
            alloc: SpiffeId::new(&format!("spiffe://test/b/{}-{}", addr.ip(), addr.port()))
                .expect("valid spiffe id"),
            addr: SocketAddr::V4(addr),
            weight: 1,
            healthy,
        }
    }

    fn lts(counter: u64) -> LogicalTimestamp {
        LogicalTimestamp { counter, writer: NodeId::new("local").expect("valid node id") }
    }

    /// A full `service_backends` row for `service_id` carrying `backends`.
    fn backends_row(service_id: u64, backends: Vec<Backend>, counter: u64) -> ServiceBackendRow {
        ServiceBackendRow {
            service_id: ServiceId::new(service_id).expect("valid service id"),
            vip: Ipv4Addr::new(10, 1, 0, 1),
            backends,
            updated_at: lts(counter),
        }
    }

    /// Construct the adapter over `store`, OPEN its held subscription (via the
    /// Earned-Trust `probe`), THEN write `rows`. Ordering is load-bearing: the
    /// `subscribe_all` surface is forward-only (no replay of pre-subscription
    /// writes — verified by `LocalObservationStore`'s acceptance test), so the
    /// adapter must subscribe BEFORE the rows are written for its index to
    /// observe them. `probe` establishes the persistent subscription; the rows
    /// written after it flow into the held subscription, drained by the next
    /// `resolve`/`probe`.
    async fn adapter_with_rows(
        store: &Arc<SimObservationStore>,
        rows: Vec<ServiceBackendRow>,
    ) -> ServiceBackendsResolve {
        let adapter = ServiceBackendsResolve::new(Arc::clone(store) as Arc<dyn ObservationStore>);
        // Open the held subscription first (the production "wire → probe → use"
        // order); now subsequent writes are delivered to it.
        adapter.probe().await.expect("probe opens the held subscription");
        for row in rows {
            store
                .write(ObservationRow::ServiceBackend(row))
                .await
                .expect("write service_backends row");
        }
        adapter
    }

    // ---- RED_ACCEPTANCE: scenario through the MtlsResolve port -------------

    /// Scenario — `service_backends_resolve_classifies_orig_dst_into_three_arms`.
    ///
    /// Drives the real `ServiceBackendsResolve` THROUGH the [`MtlsResolve`] port
    /// (`probe` + `resolve`) against a `SimObservationStore` dataset, asserting
    /// all three arms in one walkthrough: a healthy mesh backend → `Mesh`; an
    /// unmeshed addr → `NonMesh`; an unhealthy mesh backend → `MeshUnreachable`.
    /// Port-to-port: it exercises only the trait surface — deleting the
    /// production classification keeps it RED.
    #[tokio::test]
    async fn service_backends_resolve_classifies_orig_dst_into_three_arms() {
        let store = fresh_store();
        let healthy = v4(10, 0, 0, 1, 8080);
        let unhealthy = v4(10, 0, 0, 2, 8080);
        let unmeshed = v4(203, 0, 113, 7, 443);

        let adapter = adapter_with_rows(
            &store,
            vec![backends_row(1, vec![backend(healthy, true), backend(unhealthy, false)], 1)],
        )
        .await;

        // Earned-Trust probe succeeds against a readable store.
        adapter.probe().await.expect("probe on a readable store is Ok");

        // ENFORCE arm — a healthy `running` mesh backend.
        assert_eq!(
            adapter.resolve(healthy).await.expect("resolve healthy is Ok"),
            MtlsResolution::Mesh(ResolvedBackend { addr: healthy, expected_svid: None }),
        );

        // PASS-THROUGH arm — an addr with no mesh backend (cleartext, by design).
        assert_eq!(
            adapter.resolve(unmeshed).await.expect("resolve unmeshed is Ok"),
            MtlsResolution::NonMesh,
        );

        // FAIL-CLOSED arm — a present-but-unhealthy mesh backend.
        assert_eq!(
            adapter.resolve(unhealthy).await.expect("resolve unhealthy is Ok"),
            MtlsResolution::MeshUnreachable,
        );
    }

    // ---- RED_UNIT: per-arm classification + filter-to-running --------------

    /// Criterion 5 — a healthy `running` mesh backend resolves to
    /// `Mesh { addr, expected_svid: None }`, and `expected_svid` is `None` for
    /// EVERY backend (C2 — the adapter does NOT thread `IdentityRead`).
    #[tokio::test]
    async fn running_healthy_backend_resolves_to_mesh_with_no_expected_svid() {
        let store = fresh_store();
        let addr = v4(10, 0, 0, 5, 9000);
        let adapter =
            adapter_with_rows(&store, vec![backends_row(7, vec![backend(addr, true)], 1)]).await;

        let resolution = adapter.resolve(addr).await.expect("resolve is Ok");
        assert_eq!(resolution, MtlsResolution::Mesh(ResolvedBackend { addr, expected_svid: None }),);
        // Pin the C2 invariant explicitly: whatever arm, the SVID is never joined.
        if let MtlsResolution::Mesh(b) = resolution {
            assert!(b.expected_svid.is_none(), "v1 is authn-only: expected_svid MUST be None");
        }
    }

    /// Criterion 5 — an `orig_dst` with no mesh backend resolves to `NonMesh`
    /// (cleartext pass-through). A MISS is `NonMesh`, NOT `MeshUnreachable`
    /// (C4 scoping note).
    #[tokio::test]
    async fn unmeshed_addr_resolves_to_nonmesh() {
        let store = fresh_store();
        // Index holds one service; query a different, unmeshed addr.
        let adapter = adapter_with_rows(
            &store,
            vec![backends_row(1, vec![backend(v4(10, 0, 0, 1, 8080), true)], 1)],
        )
        .await;

        let got = adapter.resolve(v4(198, 51, 100, 9, 443)).await.expect("resolve is Ok");
        assert_eq!(got, MtlsResolution::NonMesh);
    }

    /// Criterion 5 — a present-but-unhealthy mesh backend resolves to
    /// `MeshUnreachable` (fail-closed), an `Ok` arm, NOT an `Err`.
    #[tokio::test]
    async fn present_but_unhealthy_backend_resolves_to_mesh_unreachable() {
        let store = fresh_store();
        let addr = v4(10, 0, 0, 3, 7000);
        let adapter =
            adapter_with_rows(&store, vec![backends_row(2, vec![backend(addr, false)], 1)]).await;

        let got = adapter.resolve(addr).await.expect("resolve is Ok (fail-closed is an Ok arm)");
        assert_eq!(got, MtlsResolution::MeshUnreachable);
    }

    /// Criterion 5 — a store-layer read fault at resolve time returns
    /// `Err(StoreUnreadable)`, distinct from the per-connection
    /// `Ok(MeshUnreachable)` classification (the 01-01 contract asymmetry). The
    /// honest store-read fault the v1 adapter surfaces is an errored
    /// `subscribe_all`; modelled by [`FaultySubscribeStore`], a delegating
    /// `SimObservationStore` wrapper whose `subscribe_all` is armed to error.
    #[tokio::test]
    async fn store_read_fault_at_resolve_returns_store_unreadable() {
        let store = Arc::new(FaultySubscribeStore::new());
        store.arm_subscribe_fault();
        let adapter = ServiceBackendsResolve::new(Arc::clone(&store) as Arc<dyn ObservationStore>);

        let err = adapter
            .resolve(v4(10, 0, 0, 1, 8080))
            .await
            .expect_err("an errored subscribe_all surfaces as Err at resolve time");
        assert!(
            matches!(&err, MtlsResolveError::StoreUnreadable { reason } if reason.contains("subscribe")),
            "expected StoreUnreadable naming the subscription fault, got {err:?}",
        );
    }

    /// Criterion 4 / 5 — `probe` REFUSES on an unreadable store with
    /// `Err(Probe)` (the `health.startup.refused`-shaped refusal); it NEVER
    /// silently returns empty. The fault-injection scenario the probe survives
    /// by refusing.
    #[tokio::test]
    async fn probe_refuses_on_unreadable_store() {
        let store = Arc::new(FaultySubscribeStore::new());
        store.arm_subscribe_fault();
        let adapter = ServiceBackendsResolve::new(Arc::clone(&store) as Arc<dyn ObservationStore>);

        let err = adapter.probe().await.expect_err("probe on an unreadable store refuses");
        assert!(
            matches!(&err, MtlsResolveError::Probe { reason } if reason.contains("subscribe")),
            "expected Probe naming the store fault, got {err:?}",
        );
    }

    /// Criterion 4 / 5 — `probe` succeeds (`Ok(())`) on a readable store (the
    /// Earned-Trust gate the composition root requires before serving).
    #[tokio::test]
    async fn probe_ok_on_readable_store() {
        let store = fresh_store();
        let adapter = adapter_with_rows(&store, vec![]).await;
        adapter.probe().await.expect("probe on a readable (empty) store is Ok");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// PBT (criterion 5/6) over the pure classification (the mutation-gate
        /// target): a single backend at an arbitrary V4 addr classifies by its
        /// `healthy` bit — `healthy` ⇒ `Mesh { addr, None }`; `!healthy` ⇒
        /// `MeshUnreachable` — and a DIFFERENT addr always ⇒ `NonMesh`. The
        /// property holds over the whole address space, pinning the
        /// healthy-filter branch + the hit/miss boundary the
        /// `>`/`>=`/match-arm mutations target.
        ///
        /// Universe (observable): the [`MtlsResolution`] arm returned by
        /// `resolve` for the queried addr.
        #[test]
        fn classification_is_total_over_arbitrary_backends(
            a in any::<u8>(), b in any::<u8>(), c in any::<u8>(), d in any::<u8>(),
            port in 1u16..=u16::MAX,
            healthy in any::<bool>(),
            // A distinct miss addr (port offset guarantees it differs from the hit).
            miss_port in 1u16..=u16::MAX,
        ) {
            let hit = v4(a, b, c, d, port);
            // Force the miss addr to differ from the hit (flip the high octet).
            let miss = v4(a ^ 0xFF, b, c, d, miss_port);
            prop_assume!(miss != hit);

            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("current-thread runtime builds");
            rt.block_on(async {
                let store = fresh_store();
                let adapter =
                    adapter_with_rows(&store, vec![backends_row(1, vec![backend(hit, healthy)], 1)])
                        .await;

                let hit_arm = adapter.resolve(hit).await.expect("resolve hit is Ok");
                let expected_hit = if healthy {
                    MtlsResolution::Mesh(ResolvedBackend { addr: hit, expected_svid: None })
                } else {
                    MtlsResolution::MeshUnreachable
                };
                prop_assert_eq!(hit_arm, expected_hit);

                let miss_arm = adapter.resolve(miss).await.expect("resolve miss is Ok");
                prop_assert_eq!(miss_arm, MtlsResolution::NonMesh);
                Ok(())
            })?;
        }
    }

    // ---- fault-injecting ObservationStore double (subscribe_all errors) ----

    use std::sync::atomic::{AtomicBool, Ordering};

    use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
    use overdrive_core::id::{AllocationId, CorrelationKey, IssuanceOrdinal};
    use overdrive_core::observation::ProbeResultRow;
    use overdrive_core::traits::observation_store::{
        AllocStatusRow, NodeHealthRow, ObservationStoreError, ReconcileConflictRow,
        ServiceHydrationResultRow,
    };
    use overdrive_core::workflow::{SignalKey, SignalValue, WorkflowStatus};

    /// A delegating `ObservationStore` double that forwards every method to an
    /// inner [`SimObservationStore`] EXCEPT `subscribe_all`, which errors when
    /// armed — the store-layer read fault the v1 adapter surfaces as
    /// `StoreUnreadable` (at resolve) / `Probe` (at probe). Delegation keeps
    /// every signature anchored to the real types so the double cannot drift
    /// from the trait; only the one fault path is overridden.
    struct FaultySubscribeStore {
        inner: SimObservationStore,
        fault_armed: AtomicBool,
    }

    impl FaultySubscribeStore {
        fn new() -> Self {
            Self {
                inner: SimObservationStore::single_peer(
                    NodeId::new("local").expect("valid node id"),
                    0,
                ),
                fault_armed: AtomicBool::new(false),
            }
        }

        /// Arm the next (and every subsequent) `subscribe_all` to error.
        fn arm_subscribe_fault(&self) {
            self.fault_armed.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl ObservationStore for FaultySubscribeStore {
        async fn subscribe_all(
            &self,
        ) -> std::result::Result<ObservationSubscription, ObservationStoreError> {
            if self.fault_armed.load(Ordering::SeqCst) {
                return Err(ObservationStoreError::Io(std::io::Error::other(
                    "injected subscribe_all fault",
                )));
            }
            self.inner.subscribe_all().await
        }

        async fn write(
            &self,
            row: ObservationRow,
        ) -> std::result::Result<(), ObservationStoreError> {
            self.inner.write(row).await
        }

        async fn alloc_status_rows(
            &self,
        ) -> std::result::Result<Vec<AllocStatusRow>, ObservationStoreError> {
            self.inner.alloc_status_rows().await
        }

        async fn alloc_status_row(
            &self,
            alloc_id: &AllocationId,
        ) -> std::result::Result<Option<AllocStatusRow>, ObservationStoreError> {
            self.inner.alloc_status_row(alloc_id).await
        }

        async fn node_health_rows(
            &self,
        ) -> std::result::Result<Vec<NodeHealthRow>, ObservationStoreError> {
            self.inner.node_health_rows().await
        }

        async fn issued_certificate_rows(
            &self,
        ) -> std::result::Result<Vec<IssuedCertificateRow>, ObservationStoreError> {
            self.inner.issued_certificate_rows().await
        }

        async fn next_issuance_ordinal(
            &self,
        ) -> std::result::Result<IssuanceOrdinal, ObservationStoreError> {
            self.inner.next_issuance_ordinal().await
        }

        async fn service_hydration_results_rows(
            &self,
            service_id: &ServiceId,
        ) -> std::result::Result<Vec<ServiceHydrationResultRow>, ObservationStoreError> {
            self.inner.service_hydration_results_rows(service_id).await
        }

        async fn service_backends_rows(
            &self,
            service_id: &ServiceId,
        ) -> std::result::Result<Vec<ServiceBackendRow>, ObservationStoreError> {
            self.inner.service_backends_rows(service_id).await
        }

        async fn reconcile_conflict_rows(
            &self,
            service_id: &ServiceId,
        ) -> std::result::Result<Vec<ReconcileConflictRow>, ObservationStoreError> {
            self.inner.reconcile_conflict_rows(service_id).await
        }

        async fn write_probe_result(
            &self,
            row: ProbeResultRow,
        ) -> std::result::Result<(), ObservationStoreError> {
            self.inner.write_probe_result(row).await
        }

        async fn list_probe_results_for_alloc(
            &self,
            alloc_id: &AllocationId,
        ) -> std::result::Result<Vec<ProbeResultRow>, ObservationStoreError> {
            self.inner.list_probe_results_for_alloc(alloc_id).await
        }

        async fn workflow_terminal_rows(
            &self,
        ) -> std::result::Result<Vec<(CorrelationKey, WorkflowStatus)>, ObservationStoreError>
        {
            self.inner.workflow_terminal_rows().await
        }

        async fn workflow_signal(
            &self,
            key: &SignalKey,
        ) -> std::result::Result<Option<SignalValue>, ObservationStoreError> {
            self.inner.workflow_signal(key).await
        }
    }
}
