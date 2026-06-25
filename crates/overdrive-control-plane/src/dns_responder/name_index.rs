//! `name_index` — the List-then-Watch `NameIndex` that maps each RESOLVABLE
//! `<job>` to its **stable frontend address `F`** (dial-by-name-responder,
//! ADR-0072 REV-2 "stable-frontend"; GH #243; roadmap 01-03 / DDN-2 / OQ-1).
//!
//! # What it is
//!
//! `NameIndex` is the name-keyed sibling reader over the `service_backends`
//! observation surface — the FOURTH reader after outbound resolve, inbound
//! install, and the `ServiceBackendsResolve` mTLS index. It decides, per
//! logical `<job>`, whether the name is RESOLVABLE (has ≥1 running-AND-healthy
//! backend) and, when it is, exposes the stable frontend `F` the
//! [`FrontendAddrAllocator`] binds for that `<job>`. [`answer_for`] projects
//! that into a [`NameAnswer`].
//!
//! # Single source of frontend truth (DDN-2 single-owner invariant)
//!
//! The answered `F` is ALWAYS the [`FrontendAddrAllocator`]'s binding — the
//! SAME `Arc`-shared instance the 02-00 `by_frontend` re-key reads. `NameIndex`
//! introduces NO second `<job> → F` source: it does not fabricate an addr and
//! does not cache an `F` that could outlive the allocator's state. This is what
//! makes "an answered `F` always HITs `by_frontend`."
//!
//! # Healthy gate = WITHHOLD seam (Finding-2)
//!
//! The `Backend.healthy == true` filter governs *resolvability* (whether to
//! answer at all), NOT *which addr*. A `<job>` with zero running-AND-healthy
//! backends is WITHHELD ([`frontend_for`](NameIndex::frontend_for) returns
//! `None` → `answer_for → NxDomain`) — but the allocator RETAINS `<job> → F`
//! (`release` is logical-workload-DELETION only, NEVER a transient zero-healthy
//! window). When a running-AND-healthy backend returns, the name resolves to
//! the SAME `F` (no addr churn — withhold-not-release).
//!
//! # List-then-Watch (MIRRORS `ServiceBackendsResolve`)
//!
//! - **List-at-probe.** [`probe`](NameIndex::probe) bulk-loads the current
//!   `service_backends` snapshot via
//!   [`all_service_backends_rows`](ObservationStore::all_service_backends_rows)
//!   into the in-RAM index AND opens the
//!   [`subscribe_all_events`](ObservationStore::subscribe_all_events) watch
//!   BEFORE returning `Ok`.
//! - **Watch (single-owner drain).** A SINGLE background task — the only owner
//!   of the subscription — folds every `service_backends` row into the index
//!   under the index write-lock.
//! - **relist-on-`Lagged`.** On
//!   [`SubscriptionEvent::Lagged`](overdrive_core::traits::observation_store::SubscriptionEvent::Lagged)
//!   the drain re-Lists the authoritative snapshot and rebuilds the index, so a
//!   dropped `service_backends` update is RECOVERED (never silently lost).
//!
//! # OQ-1 — `<job>` extraction (DECISION: local parse helper, ADR-0072)
//!
//! Each `Backend.alloc` is a [`SpiffeId`] of the shape
//! `spiffe://overdrive.local/job/<job>/alloc/<alloc>` (the
//! [`SpiffeId::for_allocation`] derivation). The OQ-1 primitive — extract the
//! `<job>` label from a `SpiffeId`'s path so the rows can be grouped by `<job>`
//! and looked up against the [`FrontendAddrAllocator`] (keyed by
//! [`MeshServiceName`]) — is implemented as the LOCAL [`job_of`] parse helper
//! here, NOT a new `SpiffeId::job_segment()` accessor on the
//! `overdrive-core::id` newtype. Rationale: the extraction is a `dns_responder`
//! consumer concern (one call site), it stays inside this slice's module
//! boundary (no `overdrive-core/src/id.rs` edit), and the inverse direction
//! (`<job>` string → [`MeshServiceName`]) already exists via
//! [`MeshServiceName::new`]. A SpiffeId whose `<job>` segment is not a valid v1
//! single-label mesh name (dotted, `_`-bearing, over 63 octets) is simply not
//! mesh-dialable by name — its backend does not contribute a resolvable `<job>`,
//! which is the design's intended scope, not a regression.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use futures::StreamExt;
use overdrive_core::id::{MeshServiceName, SpiffeId};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    ObservationRow, ObservationStore, ServiceBackendRow, SubscriptionEvent,
};
use parking_lot::RwLock;
use tokio::task::JoinHandle;

use super::frontend_addr_allocator::FrontendAddrAllocator;

/// Extract the `<job>` label from a workload [`SpiffeId`]'s path and reconstruct
/// the [`MeshServiceName`] it dials as.
///
/// The path is `/job/<job>/alloc/<alloc>` (the [`SpiffeId::for_allocation`]
/// shape). This pulls the segment immediately after `/job/` and validates it
/// as a v1 single-label mesh name via [`MeshServiceName::new`]. Returns `None`
/// when the path is not the `job/.../alloc/...` shape OR the `<job>` segment is
/// not a valid v1 single-label mesh name (dotted, out-of-class, over 63
/// octets) — such a backend is not mesh-dialable by name in v1.
///
/// This is the OQ-1 primitive — see the module rustdoc for the DECISION
/// rationale (local helper, NOT a new `SpiffeId` accessor).
fn job_of(alloc: &SpiffeId) -> Option<MeshServiceName> {
    // The path is `/job/<job>/alloc/<alloc>` — split on `/`, find the segment
    // immediately after the `job` marker. A path that does not carry a `job`
    // segment (or has nothing after it) yields no `<job>`.
    let mut segments = alloc.path().split('/').filter(|segment| !segment.is_empty());
    let job_label = loop {
        match segments.next() {
            Some("job") => break segments.next()?,
            Some(_) => {}
            None => return None,
        }
    };
    // Reconstruct the full mesh name and validate it as a v1 single-label name.
    // An out-of-class / dotted / over-63-octet `<job>` is not mesh-dialable in
    // v1 and contributes no resolvable name.
    MeshServiceName::new(&format!("{job_label}.{}", MeshServiceName::SUFFIX)).ok()
}

/// The in-RAM resolvability set: the `<job>`s with ≥1 running-AND-healthy
/// backend. A `BTreeMap` (not `HashMap`) per `.claude/rules/development.md`
/// § "Ordered-collection choice" — the index is observed by proptests, so its
/// iteration order must be deterministic across seeds. The value is the count
/// of distinct healthy-backend addrs contributing to the `<job>`, kept so a
/// row that drops a `<job>` to zero healthy backends WITHHOLDS it.
#[derive(Default)]
struct ResolvableIndex {
    /// `<job> → the addrs of its currently running-AND-healthy backends`. A
    /// `<job>` key is present iff at least one healthy backend contributes; the
    /// `frontend_for` query treats a present (non-empty) `<job>` as resolvable.
    /// `service_id`-granularity is NOT tracked because a `<job>`'s healthy set is
    /// derived from a FULL-ROW-replace per writing row (§4 full-row contract):
    /// each row carries one service's entire current backend set, and a row
    /// folds via `apply_row`, which keys the per-`<job>` addr set off the
    /// service_id that produced it.
    by_name: BTreeMap<MeshServiceName, BTreeSet<SocketAddr>>,
    /// `<job> → the addr set the LAST row for a given service contributed`,
    /// keyed per (`<job>`, `service_id`) so a row that drops a `<job>`'s healthy
    /// backends to zero WITHHOLDS the name without stranding another service's
    /// contribution to the same `<job>`. Mirrors the
    /// `ServiceBackendsResolve::addrs_by_service` per-service scoping (F-A).
    addrs_by_job_service:
        BTreeMap<(MeshServiceName, overdrive_core::id::ServiceId), BTreeSet<SocketAddr>>,
}

impl ResolvableIndex {
    /// Apply one full `service_backends` row: drop ONLY this service's prior
    /// healthy contribution to each `<job>` it touched, then insert its current
    /// running-AND-healthy backends grouped by `<job>`. Full-row replacement
    /// mirrors the `service_backends` §4 full-row contract (the row carries the
    /// service's entire current backend set), so a row that drops a `<job>` to
    /// zero healthy backends WITHHOLDS it (the healthy-gate seam).
    fn apply_row(&mut self, service_id: overdrive_core::id::ServiceId, backends: &[Backend]) {
        // Evict this service's prior contribution from every `<job>` it touched,
        // scoped to `service_id` so a different service's healthy backend at the
        // same `<job>` is never evicted.
        let prior: Vec<MeshServiceName> = self
            .addrs_by_job_service
            .keys()
            .filter(|(_, sid)| *sid == service_id)
            .map(|(job, _)| job.clone())
            .collect();
        for job in prior {
            if let Some(stale) = self.addrs_by_job_service.remove(&(job.clone(), service_id))
                && let Some(addrs) = self.by_name.get_mut(&job)
            {
                for addr in &stale {
                    addrs.remove(addr);
                }
                if addrs.is_empty() {
                    self.by_name.remove(&job);
                }
            }
        }
        // Insert the current running-AND-healthy backends, grouped by `<job>`.
        // The `healthy == true` filter IS the WITHHOLD seam (resolvability).
        let mut contributed: BTreeMap<MeshServiceName, BTreeSet<SocketAddr>> = BTreeMap::new();
        for backend in backends.iter().filter(|backend| backend.healthy) {
            if let Some(job) = job_of(&backend.alloc) {
                contributed.entry(job).or_default().insert(backend.addr);
            }
        }
        for (job, addrs) in contributed {
            self.by_name.entry(job.clone()).or_default().extend(addrs.iter().copied());
            self.addrs_by_job_service.insert((job, service_id), addrs);
        }
    }

    /// Rebuild the WHOLE resolvability set from an authoritative snapshot (the
    /// List leg + the relist recovery). Every `<job>` with a running-AND-healthy
    /// backend in `rows` is present; every other `<job>` is absent.
    fn replace_from_snapshot(&mut self, rows: &[ServiceBackendRow]) {
        self.by_name.clear();
        self.addrs_by_job_service.clear();
        for row in rows {
            self.apply_row(row.service_id, &row.backends);
        }
    }

    /// Whether `name`'s `<job>` is currently resolvable (≥1 running-AND-healthy
    /// backend). The healthy-gate WITHHOLD seam in query form: a present key is
    /// always non-empty (`apply_row` drops a `<job>` key the moment its addr set
    /// empties), so presence IS resolvability.
    fn is_resolvable(&self, name: &MeshServiceName) -> bool {
        self.by_name.contains_key(name)
    }
}

/// The name-keyed List-then-Watch index over `service_backends` (ADR-0072
/// REV-2). Maps each resolvable `<job>` to its stable frontend address `F`,
/// where `F` is the [`FrontendAddrAllocator`]'s binding — NOT a second source
/// of frontend truth. See the module rustdoc for the full contract.
pub struct NameIndex {
    /// The backing observation surface, injected as a **mandatory** constructor
    /// parameter (no default, no builder — `.claude/rules/development.md`
    /// § "Port-trait dependencies"). The List leg reads
    /// [`all_service_backends_rows`](ObservationStore::all_service_backends_rows);
    /// the Watch leg reads
    /// [`subscribe_all_events`](ObservationStore::subscribe_all_events).
    store: Arc<dyn ObservationStore>,
    /// The SINGLE source of frontend truth (DDN-2): the SAME `Arc`-shared
    /// allocator instance the 02-00 `by_frontend` re-key reads. `frontend_for`
    /// answers `F` from `assign` (idempotent per `<job>`); the index never
    /// fabricates or caches an `F`.
    allocator: FrontendAddrAllocator,
    /// The in-RAM `<job>` resolvability set, behind a synchronous
    /// [`parking_lot::RwLock`] and `Arc`-shared with the single-owner drain
    /// task. The lock is never held across an `.await`.
    resolvable: Arc<RwLock<ResolvableIndex>>,
    /// The single-owner drain task's abort handle, held so the task is aborted
    /// on `Drop`. `None` until the first [`probe`](NameIndex::probe) opens the
    /// watch.
    drain_task: parking_lot::Mutex<Option<JoinHandle<()>>>,
}

impl NameIndex {
    /// Construct the index from its REQUIRED [`ObservationStore`] and the SHARED
    /// [`FrontendAddrAllocator`]. Both mandatory, no builder — a caller that
    /// forgets either fails to construct. The resolvability set starts empty and
    /// no watch is open; [`probe`](NameIndex::probe) Lists the snapshot and
    /// opens the single-owner watch.
    #[must_use]
    pub fn new(store: Arc<dyn ObservationStore>, allocator: FrontendAddrAllocator) -> Self {
        Self {
            store,
            allocator,
            resolvable: Arc::new(RwLock::new(ResolvableIndex::default())),
            drain_task: parking_lot::Mutex::new(None),
        }
    }

    /// The stable frontend address `F` for `name`, IFF the name is currently
    /// resolvable (≥1 running-AND-healthy backend). `None` when WITHHELD/absent
    /// — the healthy-gate WITHHOLD seam projected to the query the
    /// [`answer_for`](super::answer::answer_for) consumes.
    ///
    /// When resolvable, `F` is the [`FrontendAddrAllocator`]'s binding for the
    /// `<job>` (idempotent `assign`) — the SINGLE source of frontend truth, NOT
    /// a cached index value.
    #[must_use]
    pub fn frontend_for(&self, name: &MeshServiceName) -> Option<Ipv4Addr> {
        // The healthy-gate WITHHOLD seam: not resolvable ⇒ no answer (None).
        if !self.resolvable.read().is_resolvable(name) {
            return None;
        }
        // Resolvable ⇒ answer the allocator's binding (idempotent `assign` — the
        // SINGLE source of frontend truth). An allocator at full capacity for a
        // NEW `<job>` refuses; a refusal collapses to "no answer" (NxDomain),
        // never a fabricated addr.
        self.allocator.assign(name).ok()
    }

    /// List the authoritative `service_backends` snapshot into the index (the
    /// List leg of List-then-Watch + the relist recovery). The store read is
    /// awaited, then applied to the index in a sync critical section — the
    /// write-lock is NEVER held across the `.await`.
    async fn relist(&self) -> std::result::Result<(), String> {
        Self::relist_into(&self.store, &self.resolvable).await
    }

    /// The relist primitive shared by [`Self::relist`] (the probe-time List leg)
    /// and the single-owner drain's `Lagged`-triggered relist. Takes the store +
    /// index by `Arc`-ref so the drain task — which holds `Arc`-clones, not
    /// `&self` — can re-List on a watch-loss signal. The write-lock is NEVER held
    /// across the `.await`.
    async fn relist_into(
        store: &Arc<dyn ObservationStore>,
        resolvable: &Arc<RwLock<ResolvableIndex>>,
    ) -> std::result::Result<(), String> {
        let rows = store.all_service_backends_rows().await.map_err(|err| err.to_string())?;
        resolvable.write().replace_from_snapshot(&rows);
        Ok(())
    }

    /// Spawn the SINGLE-OWNER drain task that exclusively owns `subscription` and
    /// folds every `service_backends` row into the resolvability set, relisting
    /// on `Lagged` (MIRRORS `ServiceBackendsResolve::spawn_drain`). On
    /// [`SubscriptionEvent::Lagged`] the drain re-Lists the authoritative
    /// snapshot — a dropped update is RECOVERED, never silently lost. The task
    /// exits when the subscription closes (stream end).
    fn spawn_drain(
        store: Arc<dyn ObservationStore>,
        resolvable: Arc<RwLock<ResolvableIndex>>,
        mut subscription: overdrive_core::traits::observation_store::LagAwareSubscription,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(event) = subscription.next().await {
                match event {
                    SubscriptionEvent::Row(ObservationRow::ServiceBackend(row)) => {
                        // Sync critical section — no lock across the `.await`.
                        resolvable.write().apply_row(row.service_id, &row.backends);
                    }
                    // Non-`service_backends` rows are not part of the name index.
                    SubscriptionEvent::Row(_) => {}
                    SubscriptionEvent::Lagged { .. } => {
                        // The watch dropped rows: re-acquire the authoritative
                        // snapshot and rebuild (relist-on-`Lagged`). A relist
                        // whose store read fails stops the drain — the index can
                        // no longer be kept current.
                        if Self::relist_into(&store, &resolvable).await.is_err() {
                            return;
                        }
                    }
                }
            }
        })
    }

    /// List the authoritative `service_backends` snapshot into the index AND
    /// open the single-owner watch (the Earned-Trust "wire → probe → use" gate,
    /// MIRRORING `ServiceBackendsResolve::probe`). On an unreadable store either
    /// leg returns `Err` and the node refuses to start.
    ///
    /// # Errors
    ///
    /// Returns the [`ObservationStore`] error string when the List leg's
    /// `all_service_backends_rows` or the Watch leg's `subscribe_all_events`
    /// fails.
    pub async fn probe(&self) -> std::result::Result<(), String> {
        // (1) List leg — seed the index from the authoritative snapshot BEFORE
        // the watch opens, so the index is never empty-but-trusted.
        self.relist().await?;

        // (2) Watch leg — open the subscription and spawn the single-owner drain.
        // Idempotent + single-owner: a second probe that finds the watch already
        // open does NOT re-open or re-spawn. The cheap pre-check avoids opening a
        // subscription we'd immediately discard; the claim is re-checked under the
        // lock so a concurrent first-probe race resolves to one owner.
        if self.drain_task.lock().is_some() {
            return Ok(());
        }
        let subscription =
            self.store.subscribe_all_events().await.map_err(|err| err.to_string())?;
        {
            let mut slot = self.drain_task.lock();
            if slot.is_some() {
                return Ok(());
            }
            let handle = Self::spawn_drain(
                Arc::clone(&self.store),
                Arc::clone(&self.resolvable),
                subscription,
            );
            *slot = Some(handle);
        }
        Ok(())
    }
}

impl Drop for NameIndex {
    fn drop(&mut self) {
        // Abort the single-owner drain task so it does not outlive the index.
        // Bind the `take` into a local so the `parking_lot` guard temporary drops
        // BEFORE `abort()` (clippy::significant_drop_in_scrutinee).
        let handle = self.drain_task.lock().take();
        if let Some(handle) = handle {
            handle.abort();
        }
    }
}
