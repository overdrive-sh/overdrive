//! `SimDataplane` — in-memory implementation of the [`Dataplane`] port.
//!
//! The real dataplane loads eBPF programs via aya-rs and writes BPF
//! maps. The sim dataplane stores the *same* maps in memory so that
//! control-plane logic can assert "after this write, the BPF map
//! reflects the change" without loading a kernel. Flow events are
//! pre-seeded by the test author; `drain_flow_events` returns them in
//! FIFO order and empties the queue.
//!
//! # Iteration determinism
//!
//! `services` is a [`BTreeMap`], not a [`HashMap`], per
//! `.claude/rules/development.md` § Ordered-collection choice. DST
//! harnesses observe iteration order via invariant evaluators (and
//! later, via map-iteration callsites in the slice-08 hydrator) — a
//! `HashMap`'s `RandomState`-driven order would violate the K3
//! *seed → bit-identical trajectory* property that whitepaper § 21
//! pins.
//!
//! `policy` retains its [`HashMap`] storage because `PolicyKey` (a
//! `(SpiffeId, SpiffeId)` pair) is point-accessed only — DST
//! invariants read it via `policy_verdict(&key)`, never iterate
//! over it. Promoting it to `BTreeMap` would require adding `Ord`
//! to `PolicyKey` solely for storage convenience, which is wider
//! than the dst-lint clause asks for.
//!
//! # `REVERSE_NAT` lockstep (Slice 05 / S-2.2-20)
//!
//! `services` and `reverse_nat` live inside a single
//! [`Mutex<ServiceState>`] so that every `update_service` call writes
//! both maps under one mutex acquisition. Mirrors the production
//! `EbpfDataplane`'s `REVERSE_NAT_MAP` lockstep contract: the
//! userspace-side update writes / removes `REVERSE_NAT_MAP` entries in
//! the same critical section that swaps the `SERVICE_MAP` outer-map
//! slot. Observers cannot witness a partial update — the
//! `ReverseNatLockstep` invariant pins this at PR time.

use std::collections::{BTreeMap, HashMap};
use std::net::{Ipv4Addr, SocketAddr};

use async_trait::async_trait;
use parking_lot::Mutex;

use overdrive_core::dataplane::DropClass;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::backend_key::{BackendKey, Proto};
use overdrive_core::dataplane::fingerprint::{BackendSetFingerprint, fingerprint};
use overdrive_core::id::{BackendId, SpiffeId};
use overdrive_core::traits::ProgrammedServiceView;
use overdrive_core::traits::dataplane::{
    Backend, Dataplane, DataplaneError, FlowEvent, PolicyKey, Verdict,
};

/// Forward-path + reverse-NAT state guarded by a single mutex so the
/// two maps stay in lockstep. Per `.claude/rules/development.md`
/// § *Production code is not shaped by simulation*, this mirrors the
/// production atomicity property — observers see either the
/// pre-update or the post-update view of BOTH maps, never a mixed
/// state.
struct ServiceState {
    /// Forward-path: `(VIP, port, proto) → backend set`. Keyed on the
    /// full frontend identity `(vip, port, proto)` — matching the host
    /// `EbpfDataplane`'s `SERVICE_MAP` `ServiceKey { vip_host, port_host,
    /// proto }` and the `ServiceFrontend` the `Dataplane` trait takes —
    /// so two listeners on the same VIP differing by port AND/OR proto
    /// (installed by separate per-listener `update_service` calls) are
    /// distinct entries and purge independently (ADR-0060 D4). Keying on
    /// `(vip, proto)` and dropping the port was a sim-only divergence
    /// from the host frontend identity: same-proto-different-port
    /// co-location (tcp/80 + tcp/443) collapsed to one slot, so the
    /// second install evicted the first. Guarded by
    /// `tests/sim_dataplane_forward_frontend_scoped.rs`.
    services: BTreeMap<(Ipv4Addr, u16, Proto), Vec<Backend>>,
    /// Reverse-path: `(backend_ip, backend_port, proto) → original
    /// VIP`. The egress reverse-NAT path uses this to rewrite the
    /// source 5-tuple of a backend response packet back to the VIP
    /// the client connected to.
    reverse_nat: BTreeMap<BackendKey, Ipv4Addr>,
    /// Allocator memo: `(backend address, proto) -> BackendId` for every
    /// endpoint a live service still names. Mirrors the host
    /// `BackendIdAllocator` memo: released when no service names the endpoint
    /// (orphan GC), and the counter never recycles an id.
    backend_memo: BTreeMap<(SocketAddr, Proto), BackendId>,
    /// The next `BackendId` the monotonic counter hands out (starts at 1).
    next_backend_id: u32,
    /// Applied identity of every `BackendId` ever minted: re-associated when a
    /// different workload is programmed at the id's endpoint, and retained
    /// after the id leaves its service, so an old receipt still resolves
    /// (mirrors the host's process-lifetime `Applied` associations).
    identities: BTreeMap<BackendId, SpiffeId>,
}

impl ServiceState {
    const fn new() -> Self {
        Self {
            services: BTreeMap::new(),
            reverse_nat: BTreeMap::new(),
            backend_memo: BTreeMap::new(),
            next_backend_id: 1,
            identities: BTreeMap::new(),
        }
    }
}

/// Sim dataplane state.
///
/// The forward-path `services` map and the `reverse_nat` map share
/// a single [`Mutex`] so every `update_service` write touches both
/// under one acquisition (Slice 05 lockstep contract). `policy`
/// stays a point-accessed `HashMap` guarded by a separate mutex —
/// orthogonal to service routing.
pub struct SimDataplane {
    // dst-lint: hashmap-ok point-accessed only via `policy_verdict`; never iterated
    policy: Mutex<HashMap<PolicyKey, Verdict>>,
    state: Mutex<ServiceState>,
    flow_events: Mutex<Vec<FlowEvent>>,
    /// Per-class drop counter — mirrors the kernel-side `DROP_COUNTER`
    /// `BPF_MAP_TYPE_PERCPU_ARRAY` slot layout so DST tests can assert
    /// on per-class increments without loading a kernel. One slot per
    /// `DropClass` variant; index = `DropClass::as_index()`. Slot 0 is
    /// `MalformedHeader`, slot 5 is `OversizePacket`.
    ///
    /// In production the per-CPU shape spreads contention across CPUs;
    /// the sim collapses to a single counter array because the harness
    /// runs single-threaded per evaluation. The `aggregate_per_cpu`
    /// helper in `overdrive_core::dataplane` handles the production
    /// userspace sum.
    drop_counter: Mutex<[u64; DropClass::VARIANT_COUNT as usize]>,
    /// Test-only FIFO of failures for [`Dataplane::update_service`]: each call
    /// pops the front entry BEFORE touching the service map and returns it as
    /// `Err(..)`; the map is unchanged. Empty on the non-test path. See
    /// [`SimDataplane::script_update_service_failure`].
    pending_update_service_failures: Mutex<std::collections::VecDeque<DataplaneError>>,
}

impl SimDataplane {
    /// Construct an empty sim dataplane.
    #[must_use]
    pub fn new() -> Self {
        Self {
            policy: Mutex::new(HashMap::new()),
            state: Mutex::new(ServiceState::new()),
            flow_events: Mutex::new(Vec::new()),
            drop_counter: Mutex::new([0_u64; DropClass::VARIANT_COUNT as usize]),
            pending_update_service_failures: Mutex::new(std::collections::VecDeque::new()),
        }
    }

    /// Test-only: queue a failure for the next [`Dataplane::update_service`]
    /// call (the sim counterpart of a map-update failure on the real
    /// dataplane). FIFO: each call pops the front entry and returns it as
    /// `Err(..)` WITHOUT mutating the service map; once the queue is empty
    /// calls proceed normally. Stack several to model consecutive failures.
    pub fn script_update_service_failure(&self, error: DataplaneError) {
        self.pending_update_service_failures.lock().push_back(error);
    }

    /// Record a kernel-side drop event for `class`. Increments the
    /// matching slot in the in-memory counter mirror. Saturates at
    /// `u64::MAX` — counter rollover within a single observation
    /// window is not a real failure mode (the per-class slot is the
    /// kernel-side guard against this).
    ///
    /// Mirrors the production kernel-side
    /// `DROP_COUNTER.get_ptr_mut(class.as_index())` + atomic-add
    /// path; the sim collapses both writers to a single mutex-guarded
    /// array because DST runs single-threaded per evaluation.
    pub fn record_drop(&self, class: DropClass) {
        let mut counter = self.drop_counter.lock();
        let slot = class.as_index() as usize;
        counter[slot] = counter[slot].saturating_add(1);
    }

    /// Read the recorded drop count for `class`. Mirrors the
    /// production userspace path
    /// `aggregate_per_cpu(percpu_array.get(class.as_index()))` —
    /// the sim collapses the per-CPU sum because it stores a single
    /// scalar per slot, but the surface shape is identical.
    ///
    /// Not part of the `Dataplane` trait — this accessor is for tests
    /// and DST invariant evaluators (Slice 06).
    #[must_use]
    pub fn read_drop_counter(&self, class: DropClass) -> u64 {
        let counter = self.drop_counter.lock();
        counter[class.as_index() as usize]
    }

    /// Snapshot the entire drop counter array, indexed by
    /// `DropClass::as_index()`. Length is `DropClass::VARIANT_COUNT`.
    /// Useful for DST invariants that need to walk every slot in
    /// canonical order.
    #[must_use]
    pub fn snapshot_drop_counter(&self) -> [u64; DropClass::VARIANT_COUNT as usize] {
        *self.drop_counter.lock()
    }

    /// Queue a flow event for the next `drain_flow_events` call.
    /// Tests use this to stage the telemetry the dataplane would have
    /// emitted from the kernel in a real run.
    pub fn enqueue_flow_event(&self, event: FlowEvent) {
        self.flow_events.lock().push(event);
    }

    /// Read the verdict currently stored for `key`, if any. Not part
    /// of the `Dataplane` trait — callers that use the `Dataplane`
    /// surface read verdicts by replaying flow events; this accessor
    /// is for tests that want to assert on the stored map directly.
    #[must_use]
    pub fn policy_verdict(&self, key: &PolicyKey) -> Option<Verdict> {
        self.policy.lock().get(key).copied()
    }

    /// Read the backend set currently stored for a service VIP, across
    /// every protocol registered for it (the forward map is keyed
    /// per-`(vip, proto)` per ADR-0060 D4; this accessor unions the
    /// per-proto entries in deterministic proto order). Returns `None`
    /// when no protocol of the VIP is registered.
    #[must_use]
    pub fn service_backends(&self, vip: Ipv4Addr) -> Option<Vec<Backend>> {
        let state = self.state.lock();
        let mut merged: Vec<Backend> = Vec::new();
        for ((v, _port, _proto), backends) in &state.services {
            if *v == vip {
                merged.extend(backends.iter().cloned());
            }
        }
        drop(state);
        if merged.is_empty() { None } else { Some(merged) }
    }

    /// Read the backend set stored for one exact frontend
    /// `(vip, port, proto)`, or `None` when that frontend is not
    /// registered. Mirrors the host `EbpfDataplane::service_map_contains`
    /// observability surface (the host returns a `bool` from a fallible
    /// BPF-map read; the sim returns the backend set directly, infallibly)
    /// so cross-adapter equivalence tests can assert on the same
    /// per-frontend forward-map slot the host exposes.
    ///
    /// Not part of the `Dataplane` trait — a test/DST accessor. Distinct
    /// from [`Self::service_backends`], which unions every port/proto on a
    /// VIP; this keys on the full frontend identity so co-resident
    /// listeners differing only by port are independently addressable.
    #[must_use]
    pub fn service_backends_for(
        &self,
        vip: Ipv4Addr,
        port: u16,
        proto: Proto,
    ) -> Option<Vec<Backend>> {
        self.state.lock().services.get(&(vip, port, proto)).cloned()
    }

    /// The `BackendId` the Sim selects for `(vip, port, proto)`: the lowest id
    /// of the key's current generation, or `None` when the key has no outer
    /// entry (the Sim twin of the connect hook's selection; the real Maglev
    /// choice is proven only at Tier 3). Test-support surface for
    /// `SimGatewayConnectDataplane`; not part of the `Dataplane` trait.
    pub(crate) fn selected_backend_for(
        &self,
        vip: Ipv4Addr,
        port: u16,
        proto: Proto,
    ) -> Option<BackendId> {
        let state = self.state.lock();
        state
            .services
            .get(&(vip, port, proto))?
            .iter()
            .filter_map(|backend| state.backend_memo.get(&(backend.addr, proto)).copied())
            .min()
    }

    /// The applied identity of `backend_id`, if it was ever minted by this
    /// instance. Test-support surface for `SimGatewayConnectDataplane`.
    pub(crate) fn applied_identity(&self, backend_id: BackendId) -> Option<SpiffeId> {
        self.state.lock().identities.get(&backend_id).cloned()
    }

    /// Enumerate every VIP currently registered, in `Ord` order on
    /// [`Ipv4Addr`]. Iteration order is a function of the keys (the
    /// `BTreeMap` invariant), never of insertion history — this is
    /// the property DST seed reproducibility relies on.
    ///
    /// Not part of the `Dataplane` trait — this accessor is for
    /// tests and DST invariant evaluators that need to assert on
    /// the stored map's iteration order directly.
    #[must_use]
    pub fn service_vip_keys(&self) -> Vec<Ipv4Addr> {
        let state = self.state.lock();
        let mut vips: Vec<Ipv4Addr> = state.services.keys().map(|(v, _port, _proto)| *v).collect();
        drop(state);
        vips.dedup();
        vips
    }

    /// Read the original VIP recorded in the reverse-NAT map for the
    /// given `(backend_ip, backend_port, proto)` triple. Not part of
    /// the `Dataplane` trait — this accessor is for tests and DST
    /// invariant evaluators (Slice 05 / `ReverseNatLockstep`).
    #[must_use]
    pub fn reverse_nat_lookup(&self, key: BackendKey) -> Option<Ipv4Addr> {
        self.state.lock().reverse_nat.get(&key).copied()
    }

    /// Snapshot every reverse-NAT entry, in `Ord` order on
    /// `BackendKey`. Returned `Vec` is a clone of the live map at the
    /// moment of acquisition. Not part of the `Dataplane` trait —
    /// this accessor is for DST invariant evaluators (Slice 05 /
    /// `ReverseNatLockstep`) that need to walk the entire map.
    #[must_use]
    pub fn reverse_nat_entries(&self) -> Vec<(BackendKey, Ipv4Addr)> {
        self.state.lock().reverse_nat.iter().map(|(k, v)| (*k, *v)).collect()
    }
}

impl Default for SimDataplane {
    fn default() -> Self {
        Self::new()
    }
}

/// Derive the reverse-NAT key the lockstep contract installs for a
/// backend under a **single declared L4 protocol** (ADR-0060 D4 — the
/// `[Tcp, Udp]` hardcode is narrowed to `proto`, the protocol the
/// `ServiceFrontend` declares). The forward-path `Backend` does not
/// carry proto — it is a property of the listener, not the backend
/// address — so the proto is threaded from the `ServiceFrontend`.
///
/// Only IPv4 backends are routable through the Phase 2.2 LB — IPv6 /
/// ICMP / SCTP are GH #155 / future-phase deferrals. `update_service`
/// refuses a set containing an IPv6 backend before calling this, so the
/// IPv6 arm is unreachable from it.
fn reverse_nat_key_for(backend: &Backend, proto: Proto) -> BackendKey {
    match backend.addr.ip() {
        std::net::IpAddr::V4(v4) => BackendKey::new(v4, backend.addr.port(), proto),
        std::net::IpAddr::V6(_) => {
            unreachable!("update_service refuses an IPv6 backend before deriving reverse-NAT keys")
        }
    }
}

/// A `ServiceVip` from a stored IPv4 address.
fn frontend_vip(addr: Ipv4Addr) -> Result<overdrive_core::id::ServiceVip, DataplaneError> {
    overdrive_core::id::ServiceVip::new(std::net::IpAddr::V4(addr))
        .map_err(|e| DataplaneError::LoadFailed(format!("service vip {addr}: {e}")))
}

#[async_trait]
impl ProgrammedServiceView for SimDataplane {
    async fn programmed_fingerprint(
        &self,
        frontend: &ServiceFrontend,
    ) -> Result<Option<BackendSetFingerprint>, DataplaneError> {
        let key = (frontend.vip_v4(), frontend.port().get(), frontend.proto());
        Ok(self
            .state
            .lock()
            .services
            .get(&key)
            .map(|backends| fingerprint(&frontend.vip(), backends)))
    }
}

#[async_trait]
impl Dataplane for SimDataplane {
    async fn update_policy(&self, key: PolicyKey, verdict: Verdict) -> Result<(), DataplaneError> {
        self.policy.lock().insert(key, verdict);
        Ok(())
    }

    async fn update_service(
        &self,
        frontend: ServiceFrontend,
        backends: Vec<Backend>,
    ) -> Result<(), DataplaneError> {
        // First validation, before any state is read or written: the Phase 2.2
        // LB routes IPv4 only (GH #155), so a set naming an IPv6 backend is
        // refused whole and leaves every prior state, fingerprint included,
        // unchanged. Matches the host adapter.
        if let Some(v6) = backends.iter().find(|backend| backend.addr.is_ipv6()) {
            return Err(DataplaneError::LoadFailed(format!(
                "backend {} is IPv6; the dataplane routes IPv4 backends only",
                v6.addr
            )));
        }
        // Test-only injection (see `script_update_service_failure`): the
        // failure precedes every mutation, so a failed update leaves the
        // service map exactly as it was.
        let scripted = self.pending_update_service_failures.lock().pop_front();
        if let Some(error) = scripted {
            return Err(error);
        }
        // `frontend.vip` is IPv4-by-construction (ADR-0060 D1a) —
        // narrow infallibly. `proto` is the single declared protocol;
        // the reverse-NAT fan-out is per-proto (D4), never the legacy
        // `[Tcp, Udp]` over-install.
        let vip = frontend.vip_v4();
        let port = frontend.port().get();
        let proto = frontend.proto();

        // Single mutex acquisition guards both maps — observers
        // cannot witness a partial update. Mirrors the production
        // `EbpfDataplane`'s `REVERSE_NAT_MAP` lockstep contract:
        // `SERVICE_MAP` and `REVERSE_NAT_MAP` updates land in the same
        // critical section.
        let mut state = self.state.lock();

        // Preflight, before any mutation: one endpoint may not appear twice in
        // one update, whichever workloads name it.
        let mut named: std::collections::BTreeSet<SocketAddr> = std::collections::BTreeSet::new();
        for backend in &backends {
            if !named.insert(backend.addr) {
                return Err(DataplaneError::LoadFailed(format!(
                    "backend endpoint {} is named twice in one update",
                    backend.addr
                )));
            }
        }
        // R-59: a reverse-NAT key another Service's VIP already owns is
        // rejected before anything is minted or written.
        for backend in &backends {
            let key = reverse_nat_key_for(backend, proto);
            if let Some(owner) = state.reverse_nat.get(&key).filter(|owner| **owner != vip) {
                return Err(DataplaneError::ReverseNatConflict {
                    key,
                    current_vip: frontend_vip(*owner)?,
                    attempted_vip: frontend.vip(),
                });
            }
        }
        // Resolve every endpoint's id into locals first: the fallible id
        // allocation may fail part-way, and a failed update keeps the prior
        // association (the host re-associates at commit only).
        let mut reassociations: Vec<(BackendId, SpiffeId)> = Vec::new();
        let mut fresh: Vec<((SocketAddr, Proto), BackendId, SpiffeId)> = Vec::new();
        let mut next_backend_id = state.next_backend_id;
        for backend in &backends {
            // A memoised endpoint keeps its id; the id follows the workload now
            // programmed there.
            if let Some(held_id) = state.backend_memo.get(&(backend.addr, proto)).copied() {
                reassociations.push((held_id, backend.workload_identity.clone()));
                continue;
            }
            let backend_id = BackendId::new(next_backend_id).map_err(|error| {
                DataplaneError::LoadFailed(format!("backend id counter: {error}"))
            })?;
            next_backend_id = next_backend_id.checked_add(1).ok_or_else(|| {
                DataplaneError::LoadFailed("backend id counter exhausted".to_owned())
            })?;
            fresh.push(((backend.addr, proto), backend_id, backend.workload_identity.clone()));
        }
        // Last fallible step passed: commit the associations and allocations.
        state.next_backend_id = next_backend_id;
        for (held_id, identity) in reassociations {
            state.identities.insert(held_id, identity);
        }
        for (endpoint, backend_id, identity) in fresh {
            state.backend_memo.insert(endpoint, backend_id);
            state.identities.insert(backend_id, identity);
        }

        // Snapshot prior reverse-NAT keys for this `(vip, proto)`
        // before any mutation — the diff drives the purge below. Keyed
        // per-proto so a co-resident other-proto frontend on the same
        // VIP is untouched (D4).
        let prior_keys: std::collections::BTreeSet<BackendKey> = state
            .services
            .get(&(vip, port, proto))
            .map(|prior| prior.iter().map(|b| reverse_nat_key_for(b, proto)).collect())
            .unwrap_or_default();

        // Compute new reverse-NAT keys for the incoming backend set,
        // under the declared proto only.
        let new_keys: std::collections::BTreeSet<BackendKey> =
            backends.iter().map(|b| reverse_nat_key_for(b, proto)).collect();

        // Install the new reverse-NAT entries for the incoming
        // backend set. Each `(backend_ip, backend_port, proto)` →
        // `vip` mapping lets the egress reverse-NAT path rewrite
        // the source 5-tuple of a response packet back to the VIP
        // the client connected to.
        for &key in &new_keys {
            state.reverse_nat.insert(key, vip);
        }

        // Atomic forward-path replacement. Empty backend set removes
        // this `(vip, proto)` entry entirely (per-proto purge, D4) —
        // matches `EbpfDataplane` which deletes the SERVICE_MAP outer
        // key for this frontend on empty-backend updates.
        if backends.is_empty() {
            state.services.remove(&(vip, port, proto));
        } else {
            state.services.insert((vip, port, proto), backends);
        }

        // A reverse-NAT key has exactly one owning Service (R-59), so the keys
        // that left THIS `(vip, port, proto)` are exactly the ones to purge.
        for key in prior_keys.difference(&new_keys) {
            state.reverse_nat.remove(key);
        }

        // Orphan release (mirrors the host's post-commit sweep): an endpoint no
        // service names any longer leaves the allocator memo, so a later update
        // may bind it to another workload under a fresh id. Applied identities
        // are retained.
        let live_endpoints: std::collections::BTreeSet<(SocketAddr, Proto)> = state
            .services
            .iter()
            .flat_map(|((_v, _port, p), bs)| bs.iter().map(move |b| (b.addr, *p)))
            .collect();
        state.backend_memo.retain(|endpoint, _| live_endpoints.contains(endpoint));

        // Drop the guard before returning so the mutex is released
        // before any caller `.await` resumes — minimises contention
        // for concurrent observers and silences
        // `clippy::significant_drop_tightening`.
        drop(state);
        Ok(())
    }

    async fn drain_flow_events(&self) -> Result<Vec<FlowEvent>, DataplaneError> {
        Ok(std::mem::take(&mut *self.flow_events.lock()))
    }
}
