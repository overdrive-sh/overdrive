//! `ServiceBackendsResolve` — the v1 host [`MtlsResolve`] adapter
//! (transparent-mtls-enrollment, ADR-0071; GH #242 anti-corruption boundary).
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
//! # What it is (the #242 v1 SHELL)
//!
//! `ServiceBackendsResolve` implements the [`MtlsResolve`] driven port by
//! resolving each captured connection's `orig_dst` against the mesh's `running`
//! backend set, read from `service_backends` via [`ObservationStore`]. It is the
//! v1 SHELL: it returns `expected_svid: None` for EVERY backend and does NOT
//! thread `IdentityRead` (the expected-SVID join is GH #242 — threading it here
//! is a boundary-divergence rejection per CLAUDE.md § "Implement to the design",
//! consistent with the C2 sub-decision and the shipped 01-01 port rustdoc).
//!
//! # Read mechanism (C4 / D-TME-11 — List-then-Watch over the `ObservationStore`)
//!
//! [`MtlsResolve::resolve`] is handed an arbitrary `orig_dst: SocketAddrV4` and
//! holds NO `ServiceId`; the only `ServiceId`-keyed backend-read surface
//! (`service_backends_rows(service_id)`) is the WRONG surface. Per C4 (REVISED
//! 2026-06-17, resolve-index-coherence research) the adapter resolves against an
//! in-RAM, ownership-aware address-keyed reverse index
//! (`addr → {service → Backend}`, F-A) of the `running`
//! `service_backends` set, maintained by **List-then-Watch** — the
//! industry-canonical shape for a coherent local cache over a forward-only,
//! lossy watch (k8s informer/reflector, etcd watch, Envoy xDS, Cilium kvstore /
//! `ipcache`):
//!
//! - **List-at-probe.** [`probe`](MtlsResolve::probe) bulk-loads the current
//!   `service_backends` snapshot via the keyless
//!   [`all_service_backends_rows`](ObservationStore::all_service_backends_rows)
//!   enumerate into the in-RAM index AND opens the
//!   [`subscribe_all_events`](ObservationStore::subscribe_all_events) watch
//!   BEFORE it returns
//!   `Ok` — so the index is seeded before the Earned-Trust gate opens and is
//!   never empty-but-trusted (closes #237 cold-start; mirrors Cilium
//!   `ListDone`-gates-`synced`). A failed List OR a failed subscribe →
//!   `Err(MtlsResolveError::Probe)` and the node refuses to start
//!   (`health.startup.refused`).
//! - **Watch (single-owner drain).** A SINGLE background drain task — the only
//!   owner of the subscription — continuously drains
//!   [`subscribe_all_events`](ObservationStore::subscribe_all_events) into the
//!   index under the index write-lock. There is NO shared `take()`/restore of
//!   the subscription (the F2 TOCTOU is dissolved structurally — the
//!   subscription is never shared, `.claude/rules/development.md` §
//!   "Check-and-act must be atomic"). The task's abort handle is held; the task
//!   is aborted on `Drop`.
//! - **relist-on-`Lagged` → completeness (the F4 fix — wired this step).** The
//!   drain consumes the LAG-SURFACING
//!   [`subscribe_all_events`](ObservationStore::subscribe_all_events)
//!   subscription, which carries a
//!   [`SubscriptionEvent::Lagged { missed }`](overdrive_core::traits::observation_store::SubscriptionEvent::Lagged)
//!   in-band when the broadcast drops rows (the now-removed lossy `subscribe_all`
//!   surface stripped it silently). On `Lagged`
//!   the drain re-Lists the authoritative snapshot via `relist_into` and
//!   rebuilds the index, so a dropped `service_backends` update is RECOVERED
//!   (mirrors Cilium `ErrCompacted → goto reList`; the etcd-`ErrCompacted` /
//!   k8s-reflector-`Gone` recovery contract applied to tokio `broadcast`). A
//!   dropped row is thus always either delivered (`Row`) or signalled-then-relisted
//!   (`Lagged`), never silently lost (the C4 / D-TME-11 completeness guarantee).
//!   A `Lagged`-triggered relist whose store read FAILS leaves the index
//!   uncertifiable → the watch is faulted (`resolve` → `StoreUnreadable`).
//! - **Watch-failure → fault.** When the watch terminates (the broadcast sender
//!   is dropped — the stream yields `None`), the drain marks the watch FAULTED.
//!   While the watch is faulted [`resolve`](MtlsResolve::resolve) returns
//!   `Err(MtlsResolveError::StoreUnreadable)` — the index can no longer be
//!   certified current, which is exactly the 01-01 "an underlying subscription
//!   errored" `StoreUnreadable` contract (`mtls_resolve.rs` rustdoc). On a
//!   HEALTHY watch, `resolve` always classifies (never faults).
//! - **`resolve` reads the index only.** It takes the index read-lock,
//!   classifies, returns — it does NOT read the store per call (the F2 race is
//!   gone because `resolve` no longer touches the subscription).
//!
//! Headless v1 (D-TME-10): the addr DNS returns IS the backend addr, so the
//! index is keyed by the backend addr DIRECTLY — there is NO VIP→backend
//! translation in the resolve path (that is #167/#61, out of scope).
//!
//! ## relist-on-`Lagged` — WIRED via `subscribe_all_events` (C4 / D-TME-11
//! ## refinement, option 2; this step closes F4)
//!
//! C4 / D-TME-11 pin a **relist-on-loss** leg: on a watch-loss signal the drain
//! re-Lists the authoritative snapshot. The earlier observe-only revision left
//! this leg blocked because the then-extant lossy `subscribe_all` surface
//! (`Box<dyn Stream<Item = ObservationRow>>`) stripped
//! `broadcast::RecvError::Lagged` inside both store adapters before any consumer
//! saw it. The ratified refinement (option 2) added a dedicated LAG-SURFACING
//! subscription — [`subscribe_all_events`](ObservationStore::subscribe_all_events)
//! returning a `LagAwareSubscription` of
//! [`SubscriptionEvent`](overdrive_core::traits::observation_store::SubscriptionEvent)
//! — that maps the broadcast `Lagged(n)` to a domain
//! [`SubscriptionEvent::Lagged { missed: n }`](overdrive_core::traits::observation_store::SubscriptionEvent::Lagged)
//! at the adapter boundary (the core trait never names the tokio error). The
//! single-owner drain now consumes that subscription and, on `Lagged`, re-Lists
//! via the already-shipped
//! [`all_service_backends_rows`](ObservationStore::all_service_backends_rows)
//! and rebuilds the index — closing F4 with the *completeness* guarantee (a
//! dropped `service_backends` update is always either delivered or
//! signalled-then-relisted, never silently lost). `subscribe_all_events` is now
//! the single subscription surface on `ObservationStore`: this adapter relists
//! on `Lagged`; every other consumer (the DST workflow invariants, the store
//! conformance harness) handles `Lagged` by failing loudly, since lag is a
//! structural impossibility there. The
//! [`relist`](ServiceBackendsResolve::relist) machinery (shared with the drain
//! via `relist_into`) is exercised at List-at-probe, on watch-close, AND now on
//! `Lagged`.
//!
//! # Classification — the THREE-way re-key (02-00; ADR-0072 REV-2 Finding-1/3)
//!
//! [`BackendIndex::classify`] takes `(orig_dst, proto)` and is a THREE-way
//! branch. The contract (pinned per `.claude/rules/development.md` § "Trait
//! definitions specify behavior"; the `mtls_resolve_rekey` equivalence test is
//! the enforcement):
//!
//! 1. **`by_frontend` HIT** — `(orig_dst, proto)` keyed by [`FrontendKey`] is a
//!    mesh frontend endpoint `(F, listener.port, listener.protocol)`. Translate
//!    to its `ServiceId`, select that service's **FIRST-by-`Ord`**
//!    running-AND-healthy backend →
//!    [`Mesh(ResolvedBackend { addr, expected_svid: None })`](MtlsResolution::Mesh)
//!    (a frontend HIT is ALWAYS mesh — NEVER `NonMesh`, NEVER an unhealthy
//!    backend). With NO healthy backend right now →
//!    [`MeshUnreachable`](MtlsResolution::MeshUnreachable) (the service is KNOWN
//!    but has no live backend: fail-closed, NO cleartext). `F` is the SAME
//!    stable frontend the
//!    [`FrontendAddrAllocator`](crate::dns_responder::frontend_addr_allocator::FrontendAddrAllocator)
//!    binds and the DNS `name_index` answers — there is NO second `<job> → F`
//!    source (DDN-2).
//! 2. **`by_frontend` MISS, `orig_dst.ip() ∈ 10.98.0.0/16`** —
//!    fail-closed-on-frontend-subnet-miss (Finding-3): a mesh dial that is early
//!    (race) OR to a withdrawn `<job>` → [`MeshUnreachable`] (refuse, NO
//!    cleartext). The membership test is a `contains` against the ONE pinned
//!    [`WORKLOAD_FRONTEND_BASE`] const — never a broader "any reserved subnet"
//!    helper.
//! 3. **`by_frontend` MISS, outside the subnet** — fall through to the
//!    pre-REV-2 `by_addr` classification verbatim (the additive-EXTEND
//!    backward-compat path, [`BackendIndex::classify_by_addr`]):
//!    - `orig_dst` HITS a `running`-and-healthy mesh backend →
//!      [`Mesh`](MtlsResolution::Mesh);
//!    - a matched backend is **present-but-unreachable** (`Backend.healthy ==
//!      false`) → [`MeshUnreachable`](MtlsResolution::MeshUnreachable);
//!    - `orig_dst` MISSES (no mesh backend, outside the frontend subnet) →
//!      [`NonMesh`](MtlsResolution::NonMesh) (cleartext pass-through, by design).
//!      **A general miss is `NonMesh`, NOT `MeshUnreachable`** — making EVERY
//!      miss fail-closed would break legitimate external / non-mesh egress (C4
//!      scoping note); the residual convergence window is covered by (a)
//!      fail-toward-handshake (#236).
//!
//! The v1 resolve call site keys [`Proto::Tcp`] because the worker-layer
//! outbound capture is TCP-only today (`mtls_intercept_worker.rs:792-794`); the
//! future-UDP capture surfaces the captured proto into `resolve` — the index key
//! already carries the axis, so that is a capture/plumbing change, NOT an
//! index-key change.
//!
//! - A store-layer READ FAULT (a failed List/subscribe at probe time, or a
//!   FAULTED watch at resolve time) surfaces per the 01-01 error split as an
//!   `Err` of [`MtlsResolveError::StoreUnreadable`] (resolve) /
//!   [`MtlsResolveError::Probe`] (probe) — NOT `MeshUnreachable` (the contract's
//!   asymmetry, preserved verbatim).
//!
//! # Earned-Trust probe (criterion 4)
//!
//! [`probe`](MtlsResolve::probe) demonstrates the adapter can read the
//! `service_backends` surface (it Lists the snapshot and opens the watch). On an
//! unreadable store it returns a structured [`MtlsResolveError::Probe`]
//! (`health.startup.refused`-shaped) and the node MUST refuse to start — it NEVER
//! silently returns empty / `NonMesh` (silent-empty degrading to silent
//! pass-through IS the silent-cleartext footgun the enrollment model exists to
//! remove).
//!
//! # Dependency discipline
//!
//! [`ServiceBackendsResolve::new`] takes its [`ObservationStore`] as a
//! **mandatory constructor parameter** (`Arc<dyn ObservationStore>`) — REQUIRED,
//! not defaulted, no builder (`.claude/rules/development.md` § "Port-trait
//! dependencies"). `Send + Sync + 'static` (held as `Arc<dyn MtlsResolve>`).
//!
//! [`probe`]: MtlsResolve::probe
//! [`subscribe_all_events`]: ObservationStore::subscribe_all_events

use std::collections::BTreeMap;
use std::net::{SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use futures::StreamExt;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::ServiceId;
use overdrive_core::traits::dataplane::Backend;

use crate::dns_responder::frontend_addr_allocator::{
    FrontendAddrAllocator, WORKLOAD_FRONTEND_BASE,
};
use crate::dns_responder::name_index::job_of;
use overdrive_core::traits::mtls_resolve::{
    MtlsResolution, MtlsResolve, MtlsResolveError, ResolvedBackend, Result,
};
use overdrive_core::traits::observation_store::{
    ObservationRow, ObservationStore, ServiceBackendRow, SubscriptionEvent,
};
use parking_lot::{Mutex, RwLock};
use tokio::task::JoinHandle;

/// The re-keyed `by_frontend` lookup key (ADR-0072 REV-2 Finding-1) — a
/// **mesh frontend endpoint** `(F, listener.port, listener.protocol)`.
///
/// A named newtype rather than a bare `(SocketAddrV4, Proto)` tuple so the
/// proto-discrimination contract is explicit at the type level: one frontend
/// IP `F` fronting N distinct `(port, proto)` listeners derives N DISTINCT
/// keys. A bare `SocketAddrV4` is ip+port ONLY and **cannot distinguish
/// `tcp/53` from `udp/53`** — the collision Finding-1 names; the `Proto` axis
/// is what the key carries to prevent two same-`(F, port)` listeners on
/// different L4 protos from colliding onto one entry before their distinct
/// `ServiceId` values are ever read. `Ord` (deriving from the field order
/// `addr` then `proto`) keeps the `BTreeMap<FrontendKey, ServiceId>` iteration
/// deterministic across seeds (§ "Ordered-collection choice").
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FrontendKey {
    /// The mesh frontend endpoint `(F, listener.port)` — `F` drawn from
    /// [`WORKLOAD_FRONTEND_BASE`] (`10.98.0.0/16`); the port is the listener's.
    pub addr: SocketAddrV4,
    /// The listener's L4 protocol — the axis that discriminates two
    /// same-`(F, port)` listeners on different protos.
    pub proto: Proto,
}

impl FrontendKey {
    /// Construct a frontend key from its endpoint and protocol.
    #[must_use]
    pub const fn new(addr: SocketAddrV4, proto: Proto) -> Self {
        Self { addr, proto }
    }
}

/// The in-RAM, OWNERSHIP-AWARE address-keyed reverse index of the `running`
/// `service_backends` set — the C4 read-mechanism detail, EXTENDED by 02-00
/// with the `by_frontend` re-key (ADR-0072 REV-2).
///
/// # Why ownership-aware (F-A — the security fix)
///
/// An earlier shape keyed `by_addr: BTreeMap<SocketAddrV4, Backend>` (one
/// `Backend` per addr) and evicted a service's prior addrs from that GLOBAL
/// map unconditionally with last-writer-wins. That relied on an UNSTATED
/// "one `(IP:port)` ↔ one service" invariant the writers do not enforce; if
/// two services ever contributed the same addr, one service's backend-set
/// SHRINK would evict the OTHER service's still-healthy backend → `NonMesh`
/// → silent cleartext until a relist (the F-A blocking defect). The index is
/// now keyed `addr → { service → that service's Backend at this addr }`, so
/// an addr's resolvability is **per-contributing-service**, not
/// global-last-writer-wins: a service can only ever evict ITS OWN
/// contribution, and an addr stays resolvable while ANY service still claims
/// it.
///
/// Keyed by [`SocketAddrV4`] (a [`BTreeMap`], not `HashMap` — the index is
/// observable under DST and its iteration order must be deterministic across
/// seeds, § "Ordered-collection choice"); the inner per-service map is a
/// [`BTreeMap`] for the same reason. A per-`service_id` secondary map records
/// which addrs a given service currently contributes, so an updated row for
/// that service REPLACES exactly its prior addrs (the index never strands a
/// stale backend after a service's backend set shrinks, and never evicts a
/// DIFFERENT service's backend).
#[derive(Default)]
pub struct BackendIndex {
    /// `addr → { service → that service's `Backend` at this addr }`. The
    /// point-lookup surface `resolve` consults. Only V4 backends are indexed
    /// (a V6 `Backend.addr` never matches a V4 `orig_dst`, so it is simply
    /// not inserted). An addr key is present iff at least one service claims
    /// it; the inner map is dropped when its last contributing service is
    /// evicted.
    by_addr: BTreeMap<SocketAddrV4, BTreeMap<ServiceId, Backend>>,
    /// `service_id → the V4 addrs that service currently contributes`. On a
    /// new row for a service, exactly that service's entries are removed from
    /// [`by_addr`](Self::by_addr) before the new set is inserted, so a shrunk
    /// or replaced backend set leaves no stale entries AND never touches
    /// another service's contribution.
    addrs_by_service: BTreeMap<ServiceId, Vec<SocketAddrV4>>,
    /// `(F, listener.port, proto) → ServiceId` — the 02-00 re-key
    /// (ADR-0072 REV-2). A mesh frontend endpoint translates to the `ServiceId`
    /// it fronts; a [`classify`](Self::classify) hit then selects that
    /// service's FIRST-by-`Ord` running-AND-healthy backend (the Cilium
    /// ClusterIP → backend translation). The `ServiceId` VALUE is the row's
    /// existing content-addressed `service_id` ([`ServiceBackendRow::service_id`]),
    /// NOT a re-derivation. The `F` keyed here is the SAME stable frontend the
    /// `FrontendAddrAllocator` binds and the DNS `name_index` answers — there is
    /// NO second `<job> → F` source (DDN-2 single-owner invariant). A `BTreeMap`
    /// (not `HashMap`) — observed under proptest/DST, deterministic iteration
    /// (§ "Ordered-collection choice").
    by_frontend: BTreeMap<FrontendKey, ServiceId>,
}

impl BackendIndex {
    /// Apply one full `service_backends` row to the index: drop ONLY this
    /// service's prior contribution, then insert its current V4 backends.
    /// Full-row replacement mirrors the `service_backends` §4 full-row-write
    /// contract — the row carries the service's entire current backend set.
    ///
    /// The eviction is SCOPED to `service_id` (F-A): for each addr the service
    /// previously contributed, the `service_id` entry is removed from
    /// `by_addr[addr]`, and the addr key is dropped iff its inner per-service
    /// map becomes empty (i.e. no OTHER service still claims it). A different
    /// service's backend at a shared addr is never evicted.
    pub fn apply_row(&mut self, service_id: ServiceId, backends: &[Backend]) {
        if let Some(stale) = self.addrs_by_service.remove(&service_id) {
            for addr in stale {
                if let Some(by_service) = self.by_addr.get_mut(&addr) {
                    by_service.remove(&service_id);
                    if by_service.is_empty() {
                        self.by_addr.remove(&addr);
                    }
                }
            }
        }
        let mut contributed = Vec::new();
        for backend in backends {
            if let SocketAddr::V4(v4) = backend.addr {
                self.by_addr.entry(v4).or_default().insert(service_id, backend.clone());
                contributed.push(v4);
            }
        }
        self.addrs_by_service.insert(service_id, contributed);
    }

    /// Rebuild the `by_addr` projection from an authoritative `service_backends`
    /// liveness snapshot (the List leg of List-then-Watch + the relist recovery).
    /// Every prior `by_addr` / `addrs_by_service` entry is dropped and the
    /// snapshot's rows are re-applied — a snapshot IS the complete current
    /// liveness state (the keyless enumerate returns every LWW winner), so a full
    /// replace cannot strand a service the snapshot omitted (a service whose
    /// backends were removed is simply absent from the snapshot and from the
    /// rebuilt index).
    ///
    /// `by_frontend` is INTENTIONALLY left untouched: the `service_backends`
    /// liveness snapshot carries NO frontend-binding information — the
    /// `<job> → F` bindings are owned by the
    /// [`FrontendAddrAllocator`](crate::dns_responder::frontend_addr_allocator::FrontendAddrAllocator)
    /// (DDN-2) and projected into `by_frontend` by [`Self::bind_frontend`], not by
    /// a liveness relist. Wiping `by_frontend` on a liveness `Lagged` relist would
    /// drop every frontend mapping and break withhold-not-release.
    ///
    /// Visibility: private (the `name_index.rs` precedent), consumed only by
    /// [`ServiceBackendsResolve::relist_into`] within this module — never a
    /// cross-crate caller (the pinned 02-00 surface is `by_frontend` /
    /// `FrontendKey` / `classify` only).
    fn replace_from_snapshot(&mut self, rows: &[ServiceBackendRow]) {
        self.by_addr.clear();
        self.addrs_by_service.clear();
        for row in rows {
            self.apply_row(row.service_id, &row.backends);
        }
    }

    /// Bind a mesh frontend endpoint `key = (F, listener.port, proto)` to the
    /// `ServiceId` it fronts — the 02-00 `by_frontend` re-key write half
    /// (ADR-0072 REV-2).
    ///
    /// # The enforced invariant (DDN-2 — byte-identity via the ONE allocator)
    ///
    /// `F` is the SAME stable frontend the
    /// [`FrontendAddrAllocator`](crate::dns_responder::frontend_addr_allocator::FrontendAddrAllocator)
    /// binds — the caller derives `F` for `<job>` from the ONE allocator
    /// instance, the SAME instance the DNS `name_index` answers `<job>` from.
    /// There is NO second `<job> → F` source: the `F` keyed here is
    /// byte-identical to the `F` DNS answers (the DDN-2 single-owner invariant,
    /// enforced by the COHERENCE-01 byte-identity property in
    /// `dns_name_index.rs`). A second `<job> → F` source (a divergent allocator,
    /// a re-derivation, a stale cache) would make the two projections key/answer
    /// DIFFERENT `F`s — exactly what byte-identity forbids.
    ///
    /// # No write-time ordering barrier between the two projections
    ///
    /// `by_frontend` (this re-key projection) and the DNS `name_index` are fed
    /// by TWO INDEPENDENT single-owner drains (`mtls_resolve_adapter` and
    /// `name_index`), each reading the ONE shared allocator — there is NO single
    /// ordered drain and NO temporal guarantee that `by_frontend` is bound before
    /// `name_index` exposes `F` (or vice versa). The two drains have no inter-drain
    /// ordering. Security does NOT rest on that ordering: even if `name_index`
    /// exposes `F` for a `<job>` whose `(F, listener.port, proto)` key is not yet
    /// bound here, a dial to that `F` MISSES `by_frontend`, falls into
    /// [`classify`](Self::classify) arm 2 (`F ∈ 10.98.0.0/16` →
    /// [`MeshUnreachable`](MtlsResolution::MeshUnreachable)) and fails closed — NEVER
    /// cleartext (the FAILCLOSED-01 security half). Convergence (the bind landing)
    /// is an availability nicety, not the security guarantee.
    pub fn bind_frontend(&mut self, key: FrontendKey, service_id: ServiceId) {
        self.by_frontend.insert(key, service_id);
    }

    /// PURE-READER `by_frontend` projection (02-01; ADR-0072 REV-3) — rebuild
    /// the `by_frontend` map from the authoritative `service_backends` rows AND
    /// the SHARED [`FrontendAddrAllocator`] snapshot, WITHOUT mutating the
    /// allocator.
    ///
    /// This mirrors the `name_index` drain's row→`<job>`→snapshot pattern
    /// ([`crate::dns_responder::name_index`] `replace_from_snapshot`): for each
    /// row, each running backend's `alloc` SpiffeId yields a `<job>`
    /// ([`job_of`]); the SHARED allocator's snapshot supplies its stable
    /// frontend `F` (a read-only `snapshot.get(<job>)` — NEVER `assign`, the
    /// REV-3 pure-reader invariant), and the backend's own listener port +
    /// `Proto::Tcp` (the v1 capture proto the [`Self::classify`] lookup keys —
    /// `mtls_intercept_worker.rs:792-794`) complete the [`FrontendKey`]. A
    /// `<job>` the allocator does NOT yet bind is WITHHELD (no entry) — NEVER
    /// fabricated; a dial to its `F` then fails closed via [`Self::classify`]
    /// arm 2 (the FAILCLOSED-01 security half; convergence is the availability
    /// nicety). The `<job> → F` binding has exactly ONE source — the shared
    /// allocator the DNS `name_index` answers from — so the `F` keyed here is
    /// byte-identical to the `F` DNS answers (DDN-2 single-owner invariant).
    ///
    /// The full `by_frontend` is REPLACED from the rows + snapshot (not merged):
    /// a `<job>` whose Service was removed (absent from `rows`) or whose `F` was
    /// released (absent from the snapshot) drops its `by_frontend` entry,
    /// matching the full-row-replace contract of [`Self::replace_from_snapshot`].
    fn project_by_frontend(
        &mut self,
        rows: &[ServiceBackendRow],
        frontend_snapshot: &BTreeMap<overdrive_core::id::MeshServiceName, std::net::Ipv4Addr>,
    ) {
        self.by_frontend.clear();
        for row in rows {
            self.project_row_by_frontend(row, frontend_snapshot);
        }
    }

    /// Project ONE `service_backends` row's frontend keys (the incremental
    /// per-row companion to [`Self::project_by_frontend`], used by the watch
    /// drain). First evicts the row's service's prior `by_frontend` entries
    /// (full-row replace — a shrunk backend set or a released `F` drops stale
    /// keys), then inserts the current `<job> → F` keys read from the shared
    /// allocator snapshot (the REV-3 pure read — NEVER `assign`).
    fn project_row_by_frontend(
        &mut self,
        row: &ServiceBackendRow,
        frontend_snapshot: &BTreeMap<overdrive_core::id::MeshServiceName, std::net::Ipv4Addr>,
    ) {
        // Evict this service's prior frontend keys (full-row replace, scoped to
        // `service_id`): a different service's frontend key is never touched.
        self.by_frontend.retain(|_, &mut sid| sid != row.service_id);
        for backend in &row.backends {
            // A backend whose SpiffeId is not the `/job/<job>/alloc/<alloc>`
            // shape (or whose `<job>` is not a v1 mesh name) contributes no
            // frontend key — it is not mesh-dialable by name.
            let Some(job) = job_of(&backend.alloc) else { continue };
            // READ the shared allocator's EXISTING binding (REV-3 pure reader).
            // WITHHOLD a `<job>` the allocator does not yet bind.
            let Some(&frontend_ip) = frontend_snapshot.get(&job) else { continue };
            // The frontend endpoint re-uses the backend's listener port
            // verbatim; only the IP changes (workload_addr → F). The v1 capture
            // is TCP, so the key carries `Proto::Tcp` — the same proto
            // `classify` looks up.
            let key =
                FrontendKey::new(SocketAddrV4::new(frontend_ip, backend.addr.port()), Proto::Tcp);
            self.by_frontend.insert(key, row.service_id);
        }
    }

    /// The FIRST-by-`Ord` running-AND-healthy backend `addr` for `service_id`,
    /// or `None` when the service has no healthy backend right now (BLOCKER-2:
    /// the deterministic tie-break that keeps DST replay-equivalence — v1
    /// single-replica is degenerate, but the rule is mutation-gate-able). The
    /// service's healthy backend addrs are scanned in `Ord` order (a `BTreeMap`
    /// range over `by_addr` would also serve; the per-service addr set is small)
    /// and the smallest is returned.
    fn first_healthy_backend_for(&self, service_id: ServiceId) -> Option<SocketAddrV4> {
        let addrs = self.addrs_by_service.get(&service_id)?;
        addrs
            .iter()
            .filter(|addr| {
                self.by_addr
                    .get(addr)
                    .and_then(|by_service| by_service.get(&service_id))
                    .is_some_and(|backend| backend.healthy)
            })
            .copied()
            .min()
    }

    /// Point-lookup `orig_dst`/`proto` and CLASSIFY it into an
    /// [`MtlsResolution`] arm — the THREE-way re-keyed classification the
    /// mutation gate targets (C1/C4 + ADR-0072 REV-2 Finding-1/Finding-3):
    ///
    /// 1. **`by_frontend` HIT** — `(orig_dst, proto)` is a mesh frontend
    ///    endpoint. Translate to its `ServiceId`, select that service's
    ///    FIRST-by-`Ord` running-AND-healthy backend → `Mesh { addr, None }`
    ///    (a frontend HIT is ALWAYS mesh — NEVER `NonMesh`, NEVER an unhealthy
    ///    backend). With NO healthy backend right now → `MeshUnreachable`
    ///    (fail-closed: the service is KNOWN but has no live backend).
    /// 2. **`by_frontend` MISS, but `orig_dst.ip() ∈ 10.98.0.0/16`** —
    ///    fail-closed-on-frontend-subnet-miss (Finding-3): a mesh dial that is
    ///    early (race) OR to a withdrawn `<job>`. → `MeshUnreachable` (refuse,
    ///    NO cleartext). The membership test is `WORKLOAD_FRONTEND_BASE.contains`
    ///    against the ONE pinned const — NEVER a broader "any reserved subnet"
    ///    helper.
    /// 3. **`by_frontend` MISS, outside the subnet** — fall through to today's
    ///    `by_addr` lookup verbatim (the additive-EXTEND backward-compat path):
    ///    - any contributing service has a `running`-and-`healthy` backend at
    ///      the addr → `Mesh { addr, None }` (the F-A any-healthy-at-addr rule);
    ///    - the addr is claimed but no healthy backend there → `MeshUnreachable`;
    ///    - the addr is unclaimed (a true non-mesh dst outside the frontend
    ///      subnet) → `NonMesh` (cleartext pass-through, by design — the live
    ///      rustdoc requires a GENERAL miss stay `NonMesh`).
    pub fn classify(&self, orig_dst: SocketAddrV4, proto: Proto) -> MtlsResolution {
        // Arm 1 — `by_frontend` HIT: translate the frontend endpoint to its
        // ServiceId, then select that service's FIRST-by-`Ord` running-AND-healthy
        // backend → Mesh (else MeshUnreachable on zero-healthy). A frontend HIT is
        // ALWAYS mesh — NEVER `NonMesh`, NEVER an unhealthy backend.
        if let Some(&service_id) = self.by_frontend.get(&FrontendKey::new(orig_dst, proto)) {
            // `Some(addr)` → the service's first-by-Ord healthy backend → Mesh;
            // `None` → the service is KNOWN (the key matched) but has no healthy
            // backend right now → MeshUnreachable (fail-closed, NO cleartext).
            return self
                .first_healthy_backend_for(service_id)
                .map_or(MtlsResolution::MeshUnreachable, |addr| {
                    MtlsResolution::Mesh(ResolvedBackend { addr, expected_svid: None })
                });
        }
        // Arm 2 — `by_frontend` MISS but `orig_dst.ip() ∈ 10.98.0.0/16` →
        // MeshUnreachable (fail-closed-on-frontend-subnet-miss, NO cleartext — a
        // mesh dial that is early (race) OR to a withdrawn <job>). The membership
        // test is `WORKLOAD_FRONTEND_BASE.contains` against the ONE pinned const,
        // NEVER a broader "any reserved subnet" helper.
        if WORKLOAD_FRONTEND_BASE.contains(orig_dst.ip()) {
            return MtlsResolution::MeshUnreachable;
        }
        // Arm 3 — outside the subnet: today's `by_addr` classification verbatim
        // (the additive-EXTEND backward-compat fall-through). A true non-mesh dst
        // outside the frontend subnet stays `NonMesh` (cleartext, by design).
        self.classify_by_addr(orig_dst)
    }

    /// The pre-REV-2 `by_addr` classification, preserved verbatim as the
    /// additive-EXTEND fall-through (arm 3 of [`classify`](Self::classify)):
    ///
    /// - ANY contributing service has a `running`-and-`healthy` backend at the
    ///   addr → `Mesh { addr, expected_svid: None }` (the **any-healthy-at-addr**
    ///   rule, F-A: a deterministic disjunction over contributing services, NOT
    ///   last-writer-wins);
    /// - the addr is claimed but NO contributing service has a healthy backend
    ///   there → `MeshUnreachable` (the readiness-gate "present but unreachable"
    ///   arm);
    /// - the addr is unclaimed (no entry) → `NonMesh` (cleartext pass-through,
    ///   by design — a general miss is NEVER `MeshUnreachable`).
    ///
    /// An addr key is present in `by_addr` IFF at least one service claims it:
    /// [`Self::apply_row`] drops an addr key the moment its inner per-service
    /// map empties and never inserts an empty inner map, so a `Some(by_service)`
    /// here is always non-empty.
    fn classify_by_addr(&self, orig_dst: SocketAddrV4) -> MtlsResolution {
        match self.by_addr.get(&orig_dst) {
            Some(by_service) if by_service.values().any(|backend| backend.healthy) => {
                MtlsResolution::Mesh(ResolvedBackend { addr: orig_dst, expected_svid: None })
            }
            Some(_) => MtlsResolution::MeshUnreachable,
            None => MtlsResolution::NonMesh,
        }
    }
}

/// The v1 host [`MtlsResolve`] adapter — resolves `orig_dst` against an in-RAM
/// reverse index of the `running` `service_backends` set, maintained by
/// List-then-Watch over [`ObservationStore`]. See the module rustdoc for the
/// full contract.
pub struct ServiceBackendsResolve {
    /// The backing observation surface, injected as a **mandatory** constructor
    /// parameter (no default, no builder). The List leg reads
    /// [`all_service_backends_rows`](ObservationStore::all_service_backends_rows);
    /// the Watch leg reads
    /// [`subscribe_all_events`](ObservationStore::subscribe_all_events).
    store: Arc<dyn ObservationStore>,
    /// The SHARED [`FrontendAddrAllocator`] — the single `<job> → F` owner
    /// (DDN-2; 02-01). The `by_frontend` projection READS this allocator's
    /// snapshot (a pure read — NEVER `assign`, the REV-3 pure-reader invariant)
    /// to key `(F, listener.port, proto) → ServiceId`. It is the SAME
    /// `Arc`-shared instance the DNS `name_index` answers `F` from, so the `F`
    /// keyed in `by_frontend` is byte-identical to the `F` DNS answers. The
    /// `<job> → F` binding is WRITTEN only by the 01-05 deploy-time assigner —
    /// NEVER by this drain.
    frontend: FrontendAddrAllocator,
    /// The C4 in-RAM ownership-aware `addr → {service → Backend}` reverse index
    /// (F-A), behind a synchronous
    /// [`parking_lot::RwLock`] and `Arc`-shared with the single-owner drain
    /// task. `resolve` takes the read lock; the drain task (and List-at-probe)
    /// take the write lock. The lock is never held across an `.await` — the
    /// List/relist awaits the store, then applies to the index in a sync
    /// critical section (`.claude/rules/development.md` § "Never hold a lock
    /// across `.await`").
    index: Arc<RwLock<BackendIndex>>,
    /// Watch-health flag, `Arc`-shared with the drain task. `true` while the
    /// single-owner drain is observing a live subscription; set `false` by the
    /// drain when the watch terminates unrecoverably (the broadcast sender was
    /// dropped). While `false`, [`resolve`](MtlsResolve::resolve) returns
    /// `Err(StoreUnreadable)` — the index can no longer be certified current.
    watch_healthy: Arc<AtomicBool>,
    /// The single-owner drain task's abort handle. Spawned once by the first
    /// successful [`probe`](MtlsResolve::probe); held so it can be aborted on
    /// `Drop`. `None` until the first probe opens the watch; a second probe
    /// does NOT re-spawn (the watch is single-owner).
    drain_task: Mutex<Option<JoinHandle<()>>>,
}

impl ServiceBackendsResolve {
    /// Construct the adapter from its REQUIRED [`ObservationStore`] and the
    /// SHARED [`FrontendAddrAllocator`] (02-01). Both mandatory, not defaulted,
    /// no builder — a caller that forgets either fails to construct
    /// (`.claude/rules/development.md` § "Port-trait dependencies"). The
    /// `frontend` allocator is the SAME `Arc`-shared instance the DNS
    /// `name_index` answers `F` from (DDN-2 single-owner); the `by_frontend`
    /// projection READS its snapshot (pure read — never `assign`). The index
    /// starts empty and no watch is open yet; [`probe`](MtlsResolve::probe)
    /// Lists the snapshot into the index, projects `by_frontend` from the
    /// allocator, and opens the single-owner watch (the Earned-Trust
    /// "wire → probe → use" gate).
    #[must_use]
    pub fn new(store: Arc<dyn ObservationStore>, frontend: FrontendAddrAllocator) -> Self {
        Self {
            store,
            frontend,
            index: Arc::new(RwLock::new(BackendIndex::default())),
            // Healthy until proven otherwise: a resolve before any probe (which
            // the composition root forbids — wire → probe → use) reads an empty
            // index and classifies every addr `NonMesh`, never faulting. The
            // drain sets this `false` only on a real watch termination.
            watch_healthy: Arc::new(AtomicBool::new(true)),
            drain_task: Mutex::new(None),
        }
    }

    /// List the authoritative `service_backends` snapshot and REBUILD the index
    /// from it (the List leg of List-then-Watch, and the relist recovery). The
    /// store read is awaited, then applied to the index in a sync critical
    /// section — the write-lock is NOT held across the `.await`
    /// (`.claude/rules/development.md` § "Never hold a lock across `.await`").
    /// A store-read fault surfaces as the `String` the probe/resolve callers
    /// map to `Probe` / `StoreUnreadable`.
    async fn relist(&self) -> std::result::Result<(), String> {
        Self::relist_into(&self.store, &self.index, &self.frontend).await
    }

    /// The relist primitive used by both [`Self::relist`] (the probe-time List
    /// leg) and the single-owner drain's `Lagged`-triggered relist. Takes the
    /// store + index by `Arc`-ref so the drain task — which holds `Arc`-clones,
    /// not `&self` — can re-List on a watch-loss signal. The store read is
    /// awaited, then `replace_from_snapshot` is applied in a sync critical
    /// section; the write-lock is NEVER held across the `.await`
    /// (`.claude/rules/development.md` § "Never hold a lock across `.await`").
    async fn relist_into(
        store: &Arc<dyn ObservationStore>,
        index: &Arc<RwLock<BackendIndex>>,
        frontend: &FrontendAddrAllocator,
    ) -> std::result::Result<(), String> {
        let rows = store.all_service_backends_rows().await.map_err(|err| err.to_string())?;
        // Read the shared allocator snapshot OUTSIDE the index lock (a `Mutex`
        // lock that does not cross the index `RwLock` — both are sync, neither
        // crosses the `.await`). The `by_frontend` projection is a PURE READ of
        // this snapshot (REV-3: never `assign`).
        let frontend_snapshot = frontend.snapshot();
        // Apply in a sync critical section — the await already returned. Scope
        // the write guard so it drops before the function returns
        // (clippy::significant_drop_tightening).
        {
            let mut index = index.write();
            index.replace_from_snapshot(&rows);
            index.project_by_frontend(&rows, &frontend_snapshot);
        }
        Ok(())
    }

    /// Spawn the SINGLE-OWNER drain task that exclusively owns `subscription`
    /// (a lag-surfacing [`LagAwareSubscription`]) and continuously folds every
    /// `service_backends` row into the index under the write lock. The task is
    /// the only owner of the subscription — there is no shared `take()`/restore
    /// (the F2 TOCTOU is structurally dissolved).
    ///
    /// The two non-`Row` events:
    ///
    /// - [`SubscriptionEvent::Lagged`] — the broadcast dropped rows because the
    ///   drain fell behind (the F4 lag-drop). The drain re-Lists the
    ///   authoritative `service_backends` snapshot via
    ///   [`Self::relist_into`] and rebuilds the index, so a dropped
    ///   `service_backends` update is recovered (the C4 / D-TME-11 completeness
    ///   guarantee — a dropped row is never silently lost; mirrors Cilium
    ///   `ErrCompacted → goto reList`). The `store` `Arc` is held by the task
    ///   for exactly this re-List. A relist whose store read FAILS means the
    ///   index can no longer be certified current — the watch is marked faulted
    ///   (`resolve` → `StoreUnreadable`), the same terminal posture as a closed
    ///   watch.
    /// - stream end (`None`) — the broadcast sender was dropped (a terminal
    ///   watch failure). The drain sets `watch_healthy = false` and exits, so
    ///   `resolve` faults thereafter.
    fn spawn_drain(
        store: Arc<dyn ObservationStore>,
        index: Arc<RwLock<BackendIndex>>,
        frontend: FrontendAddrAllocator,
        watch_healthy: Arc<AtomicBool>,
        mut subscription: overdrive_core::traits::observation_store::LagAwareSubscription,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            // `next().await` yields `None` only when the broadcast sender is
            // dropped (the watch is `Closed`); a `Lagged` loss signal now
            // arrives in-band as `SubscriptionEvent::Lagged` (it is no longer
            // stripped by the store — the C4 / D-TME-11 lag-surfacing
            // `subscribe_all_events` carries it here).
            while let Some(event) = subscription.next().await {
                match event {
                    SubscriptionEvent::Row(ObservationRow::ServiceBackend(row)) => {
                        // Read the shared allocator snapshot, then fold the row +
                        // project its frontend keys in one sync critical section
                        // (no lock across the `.await`). The `by_frontend`
                        // projection is a PURE READ of the snapshot (REV-3).
                        let frontend_snapshot = frontend.snapshot();
                        let mut index = index.write();
                        index.apply_row(row.service_id, &row.backends);
                        index.project_row_by_frontend(&row, &frontend_snapshot);
                    }
                    // Non-`service_backends` rows are not part of the resolve
                    // index — ignore them (the watch is the whole observation
                    // firehose; only `service_backends` rows are folded).
                    SubscriptionEvent::Row(_) => {}
                    SubscriptionEvent::Lagged { .. } => {
                        // The watch dropped rows: re-acquire the authoritative
                        // snapshot and rebuild the index (relist-on-`Lagged`,
                        // the F4 fix). A relist whose store read fails leaves
                        // the index uncertifiable — fault the watch so
                        // `resolve` returns `StoreUnreadable`, and stop draining
                        // (the index can no longer be kept current).
                        if Self::relist_into(&store, &index, &frontend).await.is_err() {
                            watch_healthy.store(false, Ordering::SeqCst);
                            return;
                        }
                    }
                }
            }
            // The watch terminated: the index can no longer be certified
            // current. Mark it faulted so `resolve` returns `StoreUnreadable`.
            watch_healthy.store(false, Ordering::SeqCst);
        })
    }
}

impl Drop for ServiceBackendsResolve {
    // mutants: skip — the only observable effect is aborting the background
    // drain task on adapter drop (best-effort cleanup, fire-and-forget). Its
    // sole symptom is the "still-running task at teardown" nextest reports as
    // leaky; there is no synchronous, in-process observable to assert on
    // through the public surface (Drop cannot await the abort), so a mutant
    // that empties this body is behaviourally indistinguishable in a test.
    fn drop(&mut self) {
        // Abort the single-owner drain task so it does not outlive the adapter.
        // Bind the `take` into a local so the `parking_lot` guard temporary
        // drops BEFORE `abort()` (clippy::significant_drop_in_scrutinee).
        let handle = self.drain_task.lock().take();
        if let Some(handle) = handle {
            handle.abort();
        }
    }
}

#[async_trait]
impl MtlsResolve for ServiceBackendsResolve {
    async fn probe(&self) -> Result<()> {
        // Earned Trust + List-at-probe: demonstrate the `service_backends`
        // surface is readable by (1) Listing the authoritative snapshot into
        // the index BEFORE the gate opens (so the index is never
        // empty-but-trusted), and (2) opening the single-owner watch for
        // incremental updates. An unreadable store at EITHER leg returns
        // `Probe` (the `health.startup.refused`-shaped refusal) — NEVER a
        // silent empty/`NonMesh`.

        // (1) List leg — seed the index from the authoritative snapshot.
        self.relist().await.map_err(|reason| MtlsResolveError::Probe { reason })?;

        // (2) Watch leg — open the subscription and spawn the single-owner
        // drain. Idempotent + single-owner: a probe that finds the watch already
        // open does NOT re-open or re-spawn (the first probe's drain is already
        // observing). The cheap `is_none` pre-check (no lock held across the
        // `.await`) avoids opening a subscription we'd immediately discard on the
        // common second-probe path; the claim itself is re-checked under the lock
        // so a concurrent first-probe race resolves to a single owner
        // (§ "Check-and-act must be atomic"). The `parking_lot::Mutex` guard is
        // never held across the `subscribe_all_events().await`.
        if self.drain_task.lock().is_some() {
            return Ok(());
        }
        let subscription = self
            .store
            .subscribe_all_events()
            .await
            .map_err(|err| MtlsResolveError::Probe { reason: err.to_string() })?;
        {
            let mut slot = self.drain_task.lock();
            if slot.is_some() {
                // A concurrent probe won the claim while we were awaiting
                // `subscribe_all_events`; this `subscription` is dropped at the
                // end of this scope (releasing the broadcast receiver) and the
                // single owner is kept.
                return Ok(());
            }
            self.watch_healthy.store(true, Ordering::SeqCst);
            let handle = Self::spawn_drain(
                Arc::clone(&self.store),
                Arc::clone(&self.index),
                self.frontend.clone(),
                Arc::clone(&self.watch_healthy),
                subscription,
            );
            *slot = Some(handle);
        }
        Ok(())
    }

    async fn resolve(&self, orig_dst: SocketAddrV4) -> Result<MtlsResolution> {
        // The watch is the freshness guarantee for the index. If it has
        // terminated unrecoverably, the index can no longer be certified
        // current — surface the 01-01 `StoreUnreadable` fault (NOT a
        // per-connection `MeshUnreachable` classification: the contract
        // asymmetry — a store-layer fault is not a classification).
        if !self.watch_healthy.load(Ordering::SeqCst) {
            return Err(MtlsResolveError::StoreUnreadable {
                reason: "service_backends watch terminated (subscription closed); \
                         index can no longer be certified current"
                    .to_owned(),
            });
        }

        // Read-only point lookup + pure classification. The read guard is taken
        // and dropped within this expression — no lock is held across an
        // `.await` (there is no `.await` in the classify path; the drain task
        // owns all index writes). The v1 resolve call site keys `Proto::Tcp`
        // because the worker-layer outbound capture is TCP-only today
        // (`mtls_intercept_worker.rs:792-794`); the future-UDP capture surfaces
        // the captured proto here — the index key already carries the axis, so
        // it is a capture/plumbing change, NOT an index-key change.
        Ok(self.index.read().classify(orig_dst, Proto::Tcp))
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

    // `Proto` flows in via `super::*` (imported at the module top for the
    // re-keyed `classify` signature). The existing `by_addr` unit tests key
    // `Proto::Tcp` — their `10.0.0.x` addrs are outside `10.98.0.0/16` and never
    // in `by_frontend`, so they reach the unchanged arm-3 `by_addr` path.
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

    /// Write `rows` into `store` FIRST, THEN construct + `probe` the adapter.
    /// Ordering is load-bearing for the List-then-Watch contract: the rows
    /// exist in the store BEFORE the adapter starts, so the List-at-probe leg
    /// (NOT the forward-only watch) is what seeds them into the index. The old
    /// observe-only adapter — which only subscribed and drained, never Listed —
    /// would MISS every pre-probe row; List-at-probe captures them.
    async fn adapter_listing_rows(
        store: &Arc<SimObservationStore>,
        rows: Vec<ServiceBackendRow>,
    ) -> ServiceBackendsResolve {
        for row in rows {
            store
                .write(ObservationRow::ServiceBackend(row))
                .await
                .expect("write service_backends row");
        }
        let adapter = ServiceBackendsResolve::new(
            Arc::clone(store) as Arc<dyn ObservationStore>,
            FrontendAddrAllocator::new(),
        );
        // List-at-probe seeds the pre-existing rows into the index.
        adapter.probe().await.expect("probe Lists the pre-existing rows");
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

        let adapter = adapter_listing_rows(
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

    /// Scenario (NEW-mechanism guarantee) —
    /// `list_at_probe_seeds_rows_written_before_subscribe`.
    ///
    /// The List-at-probe leg of List-then-Watch (C4 / D-TME-11): a
    /// `service_backends` row written to the store BEFORE the adapter is even
    /// constructed (let alone before its watch is opened) MUST resolve to
    /// `Mesh` — the boot-time List captures it. This is the test that FAILS on
    /// the old observe-only mechanism (which only subscribed forward and never
    /// Listed, so a pre-subscribe row was invisible) and PASSES on
    /// List-then-Watch. It is the #237 cold-start closure in test form.
    #[tokio::test]
    async fn list_at_probe_seeds_rows_written_before_subscribe() {
        let store = fresh_store();
        let addr = v4(10, 0, 0, 9, 8443);

        // Row exists in the store BEFORE the adapter / its watch exists.
        store
            .write(ObservationRow::ServiceBackend(backends_row(3, vec![backend(addr, true)], 1)))
            .await
            .expect("write a pre-existing service_backends row");

        // Construct + probe AFTER the write. A forward-only observe-only adapter
        // would never see this row; List-at-probe seeds it.
        let adapter = ServiceBackendsResolve::new(
            Arc::clone(&store) as Arc<dyn ObservationStore>,
            FrontendAddrAllocator::new(),
        );
        adapter.probe().await.expect("probe Lists the pre-existing snapshot");

        assert_eq!(
            adapter.resolve(addr).await.expect("resolve is Ok"),
            MtlsResolution::Mesh(ResolvedBackend { addr, expected_svid: None }),
            "List-at-probe must seed a row written before the watch opened",
        );
    }

    // ---- RED_UNIT: per-arm classification + new-mechanism units ------------

    /// Criterion 5 — a healthy `running` mesh backend resolves to
    /// `Mesh { addr, expected_svid: None }`, and `expected_svid` is `None` for
    /// EVERY backend (C2 — the adapter does NOT thread `IdentityRead`).
    #[tokio::test]
    async fn running_healthy_backend_resolves_to_mesh_with_no_expected_svid() {
        let store = fresh_store();
        let addr = v4(10, 0, 0, 5, 9000);
        let adapter =
            adapter_listing_rows(&store, vec![backends_row(7, vec![backend(addr, true)], 1)]).await;

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
        let adapter = adapter_listing_rows(
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
            adapter_listing_rows(&store, vec![backends_row(2, vec![backend(addr, false)], 1)])
                .await;

        let got = adapter.resolve(addr).await.expect("resolve is Ok (fail-closed is an Ok arm)");
        assert_eq!(got, MtlsResolution::MeshUnreachable);
    }

    /// NEW-mechanism unit — `relist_recovers_a_row_dropped_by_watch_lag`.
    ///
    /// The relist machinery (the F4 fix in spirit): a `service_backends` row
    /// that the WATCH never delivered (modelling a `Lagged` drop — the row was
    /// written to the store but never folded into the index via the
    /// subscription) MUST become visible after a relist re-acquires the
    /// authoritative snapshot. This drives the `relist` → `replace_from_snapshot`
    /// recovery path deterministically WITHOUT faking a stream value: the row is
    /// genuinely absent from the index (the watch never carried it), and the
    /// List leg recovers it from the store's authoritative `all_service_backends_rows`.
    ///
    /// (This unit pins the relist RECOVERY logic in isolation — that a relist
    /// re-acquires a row the watch never carried. The `Lagged`-TRIGGERED
    /// invocation of that recovery — the leg that was blocked in `25e7acf3` and
    /// is wired this step over the new `subscribe_all_events` surface — is
    /// covered by [`relist_on_lagged_recovers_a_dropped_update`] below.)
    #[tokio::test]
    async fn relist_recovers_a_row_dropped_by_watch_lag() {
        // `DeafWatchStore`'s watch stays OPEN but delivers NOTHING (a pending
        // stream) — so the drain never folds any row into the index, exactly
        // modelling a `Lagged` drop where the update was permanently missed by
        // the subscription. The List leg (`all_service_backends_rows`) and
        // `write` delegate to a real inner store, so relist can recover the row
        // the watch never carried. This makes the recovery DETERMINISTIC: only
        // relist can move the row into the index (the live watch cannot
        // race-deliver it).
        let store = Arc::new(ScriptableStore::with_watch(WatchMode::Deaf));
        let lagged_addr = v4(10, 0, 0, 42, 6000);

        let adapter = ServiceBackendsResolve::new(
            Arc::clone(&store) as Arc<dyn ObservationStore>,
            FrontendAddrAllocator::new(),
        );
        adapter.probe().await.expect("probe on an empty store is Ok");
        // The address is a miss right now (the row does not exist yet).
        assert_eq!(
            adapter.resolve(lagged_addr).await.expect("resolve is Ok"),
            MtlsResolution::NonMesh,
        );

        // Write the row to the store. The deaf watch will NEVER carry it into
        // the index — this is the would-be-lagged row, permanently missed by
        // the subscription.
        store
            .write(ObservationRow::ServiceBackend(backends_row(
                5,
                vec![backend(lagged_addr, true)],
                1,
            )))
            .await
            .expect("write the would-be-lagged row");

        // Confirm the watch did NOT deliver it: still a miss before relist.
        // (Yield first so any [incorrectly] live drain would have had its turn.)
        tokio::task::yield_now().await;
        assert_eq!(
            adapter.resolve(lagged_addr).await.expect("resolve is Ok"),
            MtlsResolution::NonMesh,
            "the deaf watch must not deliver the row — only relist can recover it",
        );

        // The relist re-acquires the authoritative snapshot and rebuilds the
        // index — the dropped row is recovered. (This is the recovery a
        // `Lagged` signal WOULD trigger if the surface delivered it.)
        adapter.relist().await.expect("relist re-acquires the authoritative snapshot");

        assert_eq!(
            adapter.resolve(lagged_addr).await.expect("resolve is Ok after relist"),
            MtlsResolution::Mesh(ResolvedBackend { addr: lagged_addr, expected_svid: None }),
            "relist must recover a row the watch dropped",
        );
    }

    /// NEW-mechanism unit (the leg blocked in `25e7acf3`, now wired) —
    /// `relist_on_lagged_recovers_a_dropped_update`.
    ///
    /// The relist TRIGGER: a real [`SubscriptionEvent::Lagged`] delivered on the
    /// lag-surfacing `subscribe_all_events` watch MUST drive the single-owner
    /// drain to re-List the authoritative snapshot and recover a
    /// `service_backends` update the watch dropped. This is the F4-closure
    /// guarantee in test form (a dropped row is never silently lost: it is
    /// signalled-then-relisted).
    ///
    /// DETERMINISM: the `LaggedChannel` double hands the drain an `mpsc`-backed
    /// watch the test controls — NO racing a 1024-deep broadcast. The sequence
    /// pins cause→effect tightly:
    /// 1. probe (empty store) → index empty, drain subscribed to the channel;
    /// 2. the addr is a miss (`NonMesh`);
    /// 3. write the row to the store — the channel watch does NOT carry it (the
    ///    test never pushes a `Row`), so it stays a miss;
    /// 4. `emit_lagged` pushes ONE `Lagged` → the drain relists → the row,
    ///    recovered from `all_service_backends_rows`, becomes `Mesh`.
    /// Step 3's persisted miss + step 4's recovery is the falsifiable core:
    /// delete the `Lagged → relist_into` arm in `spawn_drain` and the addr stays
    /// `NonMesh` forever (the test goes RED).
    #[tokio::test]
    async fn relist_on_lagged_recovers_a_dropped_update() {
        let store = Arc::new(ScriptableStore::with_watch(WatchMode::LaggedChannel));
        let dropped_addr = v4(10, 0, 0, 77, 7443);

        let adapter = ServiceBackendsResolve::new(
            Arc::clone(&store) as Arc<dyn ObservationStore>,
            FrontendAddrAllocator::new(),
        );
        adapter.probe().await.expect("probe on an empty store opens the channel watch");

        // (2) miss before anything is written.
        assert_eq!(
            adapter.resolve(dropped_addr).await.expect("resolve is Ok"),
            MtlsResolution::NonMesh,
        );

        // (3) write the row; the channel watch carries no `Row`, so the drain
        // never folds it — still a miss. (Yield so any drain progress lands.)
        store
            .write(ObservationRow::ServiceBackend(backends_row(
                9,
                vec![backend(dropped_addr, true)],
                1,
            )))
            .await
            .expect("write the would-be-lagged row");
        tokio::task::yield_now().await;
        assert_eq!(
            adapter.resolve(dropped_addr).await.expect("resolve is Ok"),
            MtlsResolution::NonMesh,
            "the channel watch carries no Row — the update is still missed pre-Lagged",
        );

        // (4) inject the loss signal: the drain MUST relist and recover the row.
        store.emit_lagged(3);

        // The drain relists asynchronously; spin briefly until the index
        // reflects the recovery (bounded — no unbounded wait).
        let mut recovered = MtlsResolution::NonMesh;
        for _ in 0..1000 {
            recovered = adapter.resolve(dropped_addr).await.expect("resolve is Ok");
            if recovered != MtlsResolution::NonMesh {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            recovered,
            MtlsResolution::Mesh(ResolvedBackend { addr: dropped_addr, expected_svid: None }),
            "a Lagged event must trigger a relist that recovers the dropped update",
        );
    }

    /// NEW-mechanism unit — `watch_failure_makes_resolve_return_store_unreadable`.
    ///
    /// When the watch terminates unrecoverably (the broadcast sender is dropped
    /// → the subscription closes → the drain task marks the watch faulted),
    /// `resolve` MUST return `Err(StoreUnreadable)` — the index can no longer be
    /// certified current (the 01-01 "an underlying subscription errored"
    /// `StoreUnreadable` contract). Driven by a store double whose subscription
    /// closes immediately, so the drain observes `Closed` and faults the watch.
    #[tokio::test]
    async fn watch_failure_makes_resolve_return_store_unreadable() {
        let store = Arc::new(ScriptableStore::with_watch(WatchMode::Closed));
        let adapter = ServiceBackendsResolve::new(
            Arc::clone(&store) as Arc<dyn ObservationStore>,
            FrontendAddrAllocator::new(),
        );

        // Probe succeeds (List Ok, subscribe_all_events Ok) — but the
        // subscription the double hands back is already closed, so the drain
        // immediately sees `Closed` and faults the watch.
        adapter.probe().await.expect("probe Lists + opens the (already-closed) watch");

        // Give the spawned drain a chance to observe `Closed` and set the flag.
        for _ in 0..100 {
            if !adapter.watch_healthy.load(Ordering::SeqCst) {
                break;
            }
            tokio::task::yield_now().await;
        }

        let err = adapter
            .resolve(v4(10, 0, 0, 1, 8080))
            .await
            .expect_err("a faulted watch surfaces as Err at resolve time");
        assert!(
            matches!(&err, MtlsResolveError::StoreUnreadable { reason } if reason.contains("watch")),
            "expected StoreUnreadable naming the watch fault, got {err:?}",
        );
    }

    /// Criterion 5 — a store-layer read fault at PROBE time (the List leg's
    /// `all_service_backends_rows` errors) returns `Err(Probe)` — distinct from
    /// the per-connection `Ok(MeshUnreachable)` classification (the 01-01
    /// contract asymmetry). Modelled by [`FaultyListStore`], a delegating
    /// `SimObservationStore` wrapper whose `all_service_backends_rows` is armed
    /// to error.
    #[tokio::test]
    async fn store_read_fault_at_probe_returns_probe() {
        let store = Arc::new(ScriptableStore::with_watch(WatchMode::Live));
        store.arm_list_fault();
        let adapter = ServiceBackendsResolve::new(
            Arc::clone(&store) as Arc<dyn ObservationStore>,
            FrontendAddrAllocator::new(),
        );

        let err = adapter.probe().await.expect_err("an errored List surfaces as Err at probe time");
        assert!(
            matches!(&err, MtlsResolveError::Probe { reason } if reason.contains("List")),
            "expected Probe naming the List fault, got {err:?}",
        );
    }

    /// Criterion 4 / 5 — `probe` succeeds (`Ok(())`) on a readable store (the
    /// Earned-Trust gate the composition root requires before serving).
    #[tokio::test]
    async fn probe_ok_on_readable_store() {
        let store = fresh_store();
        let adapter = adapter_listing_rows(&store, vec![]).await;
        adapter.probe().await.expect("probe on a readable (empty) store is Ok");
    }

    // ---- 02-01: by_frontend pure-reader drain projection -------------------

    /// 02-01 — the `by_frontend` projection is a PURE READER of the SHARED
    /// `FrontendAddrAllocator`: a dial to a `<job>`'s stable frontend `F`
    /// translates to that `<job>`'s service's healthy backend, where `F` comes
    /// from the allocator (NOT a re-derivation), and the projection NEVER
    /// `assign`s (a `<job>` the allocator does not yet bind is WITHHELD).
    ///
    /// Port-to-port through `MtlsResolve` (`probe` projects `by_frontend` from
    /// the allocator's snapshot at the List leg; `resolve` translates `F:port`).
    /// Falsifiable: delete the `project_by_frontend` call in `relist_into` and
    /// the dial to `F` MISSES `by_frontend`, falls into the `∈ 10.98.0.0/16`
    /// fail-closed arm → `MeshUnreachable` (the test's `Mesh` assertion goes
    /// RED). Two cases in one walkthrough: (A) allocator binds the `<job>` → `F`
    /// translates to the backend (`Mesh`); (B) the allocator does NOT bind a
    /// second `<job>` → its `F` is WITHHELD (no `by_frontend` entry → the
    /// frontend-subnet-miss fail-closed arm → `MeshUnreachable`, NEVER `Mesh`).
    #[tokio::test]
    async fn by_frontend_projection_reads_the_shared_allocator_and_never_assigns() {
        use overdrive_core::id::{AllocationId, MeshServiceName, WorkloadId};

        let store = fresh_store();
        let frontend = FrontendAddrAllocator::new();

        // The allocator is the SINGLE source of `F`: pre-bind `<job> = bound`
        // (the 01-05 deploy-time assigner's job) and LEAVE `<job> = withheld`
        // unbound (the race / not-yet-assigned case — its row exists but the
        // allocator does not bind its `F`, so the projection WITHHOLDS it).
        let bound_job = MeshServiceName::new("bound.svc.overdrive.local").expect("valid mesh name");
        let f_bound = frontend.assign(&bound_job).expect("assign F for the bound job");

        // A backend whose alloc SpiffeId is the `/job/<job>/alloc/<alloc>` shape
        // the projection parses (`job_of`). The backend's listener port is the
        // port the frontend key re-uses verbatim.
        let backend_port = 8080;
        let backend_addr = v4(10, 99, 0, 6, backend_port);
        let mk_backend = |job_label: &str, addr: SocketAddrV4, healthy: bool| Backend {
            alloc: SpiffeId::for_allocation(
                &WorkloadId::new(job_label).expect("valid workload id"),
                &AllocationId::new("alloc-1").expect("valid alloc id"),
            ),
            addr: SocketAddr::V4(addr),
            weight: 1,
            healthy,
        };

        // Both services have a healthy backend row; only `bound` has an
        // allocator F. Write the rows, THEN probe the SHARED allocator-bearing
        // adapter so the List-at-probe leg projects `by_frontend` from it.
        for row in [
            backends_row(101, vec![mk_backend("bound", backend_addr, true)], 1),
            backends_row(202, vec![mk_backend("withheld", v4(10, 99, 0, 10, 9000), true)], 1),
        ] {
            store.write(ObservationRow::ServiceBackend(row)).await.expect("write row");
        }
        let adapter = ServiceBackendsResolve::new(
            Arc::clone(&store) as Arc<dyn ObservationStore>,
            frontend.clone(),
        );
        adapter.probe().await.expect("probe projects by_frontend from the shared allocator");

        // (A) a dial to the bound job's frontend `F:port` translates to its
        // backend (Mesh) — `F` is the allocator's binding, not a re-derivation.
        let f_endpoint = SocketAddrV4::new(f_bound, backend_port);
        assert_eq!(
            adapter.resolve(f_endpoint).await.expect("resolve F is Ok"),
            MtlsResolution::Mesh(ResolvedBackend { addr: backend_addr, expected_svid: None }),
            "by_frontend must translate the allocator's F to the job's healthy backend",
        );

        // (B) the withheld job has NO allocator binding → its frontend key is
        // never projected. The allocator still binds ONLY `bound` (the
        // projection never `assign`ed `withheld`): a dial to ANY unbound
        // frontend-subnet addr MISSES by_frontend and fails closed.
        assert_eq!(
            frontend.snapshot().len(),
            1,
            "the pure-reader projection must NOT assign — only the pre-bound job is held",
        );
        let unbound_frontend = SocketAddrV4::new(std::net::Ipv4Addr::new(10, 98, 0, 250), 9000);
        assert_eq!(
            adapter.resolve(unbound_frontend).await.expect("resolve is Ok"),
            MtlsResolution::MeshUnreachable,
            "a frontend-subnet dial the allocator does not bind fails closed (never Mesh)",
        );
    }

    // ---- F-A: ownership-aware index (the security proof) -------------------

    /// Construct a [`ServiceId`] for the index unit tests.
    fn svc(id: u64) -> ServiceId {
        ServiceId::new(id).expect("valid service id")
    }

    /// F-A (blocking, security) —
    /// `shared_addr_eviction_is_scoped_to_the_shrinking_service`.
    ///
    /// The exact defect the post-arc review demands a test for: two services
    /// (A, B) both contribute the SAME healthy addr `X`; then A shrinks to an
    /// empty backend set. Under the OLD global-`addr → Backend` index with
    /// unconditional last-writer-wins eviction, A's shrink evicted `X`
    /// wholesale → `classify(X) == NonMesh` → silent cleartext for a backend B
    /// still serves. Under the ownership-aware index A can only evict its OWN
    /// contribution, so B's claim survives and `classify(X)` is STILL `Mesh`.
    ///
    /// Tested at the index boundary directly: the v1 writers cannot produce a
    /// shared `(IP:port)` (one service per addr in practice), so the index's
    /// DEFENSIVE behavior against the violated invariant is proven at its own
    /// boundary — exactly where the review located the latent footgun. RED on
    /// the old structure, GREEN on the fix.
    #[test]
    fn shared_addr_eviction_is_scoped_to_the_shrinking_service() {
        let shared = v4(10, 0, 0, 1, 8080);
        let mut index = BackendIndex::default();

        // Both A and B claim the same healthy addr.
        index.apply_row(svc(1), &[backend(shared, true)]);
        index.apply_row(svc(2), &[backend(shared, true)]);
        assert_eq!(
            index.classify(shared, Proto::Tcp),
            MtlsResolution::Mesh(ResolvedBackend { addr: shared, expected_svid: None }),
            "an addr claimed by two healthy services is Mesh",
        );

        // A shrinks to nothing. B still claims `shared` and is healthy.
        index.apply_row(svc(1), &[]);

        assert_eq!(
            index.classify(shared, Proto::Tcp),
            MtlsResolution::Mesh(ResolvedBackend { addr: shared, expected_svid: None }),
            "B's still-healthy claim must survive A's shrink — no global eviction",
        );
    }

    /// F-A (determinism) — `classify_is_any_healthy_independent_of_apply_order`.
    ///
    /// When two services claim the same addr with DIFFERENT readiness (A
    /// unhealthy, B healthy), the addr must classify `Mesh` regardless of
    /// which row was applied last — the any-healthy-at-addr rule, NOT
    /// last-writer-wins. The OLD global index would overwrite the addr's lone
    /// `Backend` with whichever row applied last, so the result depended on
    /// apply order (unhealthy-last ⇒ `MeshUnreachable`). The fix makes
    /// classification a deterministic disjunction over contributors. Both
    /// apply orders are asserted in one test.
    #[test]
    fn classify_is_any_healthy_independent_of_apply_order() {
        let shared = v4(10, 0, 0, 2, 9000);
        let expected = MtlsResolution::Mesh(ResolvedBackend { addr: shared, expected_svid: None });

        // Order 1: unhealthy A first, healthy B last.
        let mut index = BackendIndex::default();
        index.apply_row(svc(1), &[backend(shared, false)]);
        index.apply_row(svc(2), &[backend(shared, true)]);
        assert_eq!(
            index.classify(shared, Proto::Tcp),
            expected,
            "any-healthy: unhealthy-then-healthy"
        );

        // Order 2: healthy B first, unhealthy A last — same verdict.
        let mut index = BackendIndex::default();
        index.apply_row(svc(2), &[backend(shared, true)]);
        index.apply_row(svc(1), &[backend(shared, false)]);
        assert_eq!(
            index.classify(shared, Proto::Tcp),
            expected,
            "any-healthy: healthy-then-unhealthy"
        );

        // And when EVERY contributor at the addr is unhealthy → MeshUnreachable
        // (the addr is claimed but unreachable), never NonMesh.
        let mut index = BackendIndex::default();
        index.apply_row(svc(1), &[backend(shared, false)]);
        index.apply_row(svc(2), &[backend(shared, false)]);
        assert_eq!(
            index.classify(shared, Proto::Tcp),
            MtlsResolution::MeshUnreachable,
            "an addr claimed only by unhealthy backends is MeshUnreachable",
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// PBT (criterion 5/6) over the pure `by_addr` classification (arm 3,
        /// the mutation-gate target): a single backend at an arbitrary V4 addr
        /// OUTSIDE the `10.98.0.0/16` frontend subnet classifies by its
        /// `healthy` bit — `healthy` ⇒ `Mesh { addr, None }`; `!healthy` ⇒
        /// `MeshUnreachable` — and a DIFFERENT addr (also outside the subnet)
        /// always ⇒ `NonMesh`. The property holds over the `by_addr` address
        /// space, pinning the healthy-filter branch + the hit/miss boundary the
        /// match-arm mutations target.
        ///
        /// Constrained OUTSIDE `10.98.0.0/16` (02-00): an addr inside the
        /// frontend subnet now routes to the fail-closed arm-2 (a MISS there is
        /// `MeshUnreachable`, NOT `NonMesh`), which `mtls_resolve_rekey.rs`
        /// FAILCLOSED-01 covers directly — this property remains the arm-3
        /// `by_addr` totality it was authored for.
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
            // Exclude `10.98.0.0/16` (the frontend subnet) from BOTH the hit and
            // the flipped-high-octet miss, so this stays an arm-3 `by_addr`
            // totality test (the subnet arm is FAILCLOSED-01's surface).
            prop_assume!(!(a == 10 && b == 98));
            prop_assume!(!((a ^ 0xFF) == 10 && b == 98));
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
                    adapter_listing_rows(&store, vec![backends_row(1, vec![backend(hit, healthy)], 1)])
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

    // ---- fault-injecting ObservationStore doubles --------------------------

    use std::sync::atomic::AtomicBool as StdAtomicBool;

    use futures::channel::mpsc;
    use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
    use overdrive_core::id::{AllocationId, CorrelationKey, IssuanceOrdinal};
    use overdrive_core::observation::ProbeResultRow;
    use overdrive_core::traits::observation_store::{
        AllocStatusRow, LagAwareSubscription, NodeHealthRow, ObservationStoreError,
        ReconcileConflictRow, ServiceHydrationResultRow, SubscriptionEvent,
    };
    use overdrive_core::workflow::{SignalKey, SignalValue, WorkflowStatus};

    /// How the scriptable double's lag-surfacing `subscribe_all_events` watch
    /// behaves. The List leg (`all_service_backends_rows`) is governed
    /// separately by `list_fault_armed`.
    #[derive(Clone, Copy)]
    enum WatchMode {
        /// A real, live subscription delegated to the inner store (the watch
        /// carries writes into the index as normal).
        Live,
        /// An already-CLOSED watch: an empty stream that yields `None`
        /// immediately, modelling the broadcast sender dropping the instant the
        /// watch opens — the single-owner drain observes `Closed` and faults.
        Closed,
        /// A DEAF watch: a pending stream that NEVER yields — the watch stays
        /// open (never faults) but carries nothing, modelling a `Lagged` drop
        /// where the update is permanently missed by the subscription. Only a
        /// relist can recover a row written under this mode.
        Deaf,
        /// A CHANNEL-driven watch: `subscribe_all_events` hands back the
        /// receiving end of an `mpsc` the test controls via
        /// [`ScriptableStore::emit_lagged`]. The watch stays open (the sender is
        /// held by the double) and yields ONLY what the test pushes — so a
        /// [`SubscriptionEvent::Lagged`] can be injected DETERMINISTICALLY after
        /// a row is written, driving the relist-on-`Lagged` recovery without
        /// racing a real 1024-deep broadcast.
        LaggedChannel,
    }

    /// One delegating `ObservationStore` double for every fault scenario the
    /// resolve adapter must survive: a List-leg fault (`list_fault_armed`) and
    /// the watch behaviours ([`WatchMode`]). Every method delegates to a
    /// real inner [`SimObservationStore`] EXCEPT the two surfaces under test
    /// (`all_service_backends_rows` and `subscribe_all_events`) — delegation
    /// keeps every signature anchored to the real trait types so the double
    /// cannot drift, per the port-boundary double discipline.
    struct ScriptableStore {
        inner: SimObservationStore,
        watch_mode: WatchMode,
        list_fault_armed: StdAtomicBool,
        /// Sender half of the [`WatchMode::LaggedChannel`] watch, populated when
        /// `subscribe_all_events` is first called under that mode. Held by the
        /// double so the channel stays open (the drain never sees `Closed`)
        /// until the test drops the store; [`Self::emit_lagged`] pushes events.
        /// `futures::channel::mpsc` (not `tokio`) — its `UnboundedReceiver` IS a
        /// `Stream`, so no extra `tokio-stream` dep / wrapper is needed.
        lagged_tx: Mutex<Option<mpsc::UnboundedSender<SubscriptionEvent>>>,
    }

    impl ScriptableStore {
        fn with_watch(watch_mode: WatchMode) -> Self {
            Self {
                inner: SimObservationStore::single_peer(
                    NodeId::new("local").expect("valid node id"),
                    0,
                ),
                watch_mode,
                list_fault_armed: StdAtomicBool::new(false),
                lagged_tx: Mutex::new(None),
            }
        }

        /// Arm the next (and every subsequent) `all_service_backends_rows` List
        /// to error — the store-layer read fault the List leg surfaces as `Probe`.
        fn arm_list_fault(&self) {
            self.list_fault_armed.store(true, Ordering::SeqCst);
        }

        /// Push a [`SubscriptionEvent::Lagged { missed }`] onto the
        /// [`WatchMode::LaggedChannel`] watch — the deterministic loss-signal
        /// injection the relist-on-`Lagged` test drives the drain with. Returns
        /// the number of receivers the send reached (`true` once the drain has
        /// subscribed). The subscription must already be open (probe ran).
        fn emit_lagged(&self, missed: u64) {
            // Clone the sender out and drop the guard before sending — keeps the
            // `parking_lot` critical section to the map read (clippy::
            // significant_drop_tightening). `UnboundedSender` is `Clone`.
            let tx = self
                .lagged_tx
                .lock()
                .clone()
                .expect("subscribe_all_events opened the lagged channel");
            tx.unbounded_send(SubscriptionEvent::Lagged { missed })
                .expect("drain receiver is alive");
        }
    }

    #[async_trait]
    impl ObservationStore for ScriptableStore {
        async fn all_service_backends_rows(
            &self,
        ) -> std::result::Result<Vec<ServiceBackendRow>, ObservationStoreError> {
            if self.list_fault_armed.load(Ordering::SeqCst) {
                return Err(ObservationStoreError::Io(std::io::Error::other(
                    "injected List (all_service_backends_rows) fault",
                )));
            }
            self.inner.all_service_backends_rows().await
        }

        async fn subscribe_all_events(
            &self,
        ) -> std::result::Result<LagAwareSubscription, ObservationStoreError> {
            match self.watch_mode {
                // Delegate to the inner store's REAL lag-surfacing subscription
                // (exercises the same `subscribe_all_events` impl production
                // uses).
                WatchMode::Live => self.inner.subscribe_all_events().await,
                // Empty stream → yields `None` immediately (watch closed).
                WatchMode::Closed => Ok(Box::new(futures::stream::empty()) as LagAwareSubscription),
                // Pending stream → never yields (watch open but deaf).
                WatchMode::Deaf => Ok(Box::new(futures::stream::pending()) as LagAwareSubscription),
                // Channel the test drives: hand back the receiver (itself a
                // `Stream`); hold the sender so the stream stays open and
                // `emit_lagged` can push.
                WatchMode::LaggedChannel => {
                    let (tx, rx) = mpsc::unbounded::<SubscriptionEvent>();
                    *self.lagged_tx.lock() = Some(tx);
                    Ok(Box::new(rx) as LagAwareSubscription)
                }
            }
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
