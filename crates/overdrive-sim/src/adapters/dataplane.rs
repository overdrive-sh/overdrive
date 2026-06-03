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
use std::net::{Ipv4Addr, SocketAddrV4};

use async_trait::async_trait;
use parking_lot::Mutex;

use overdrive_core::dataplane::DropClass;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::backend_key::{BackendKey, Proto};
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
    /// Forward-path: `(VIP, proto) → backend set`. Keyed per-proto
    /// (ADR-0060 D4) so two protocols on the same VIP — installed by
    /// separate per-listener `update_service` calls — are distinct
    /// entries and purge independently.
    services: BTreeMap<(Ipv4Addr, Proto), Vec<Backend>>,
    /// Reverse-path: `(backend_ip, backend_port, proto) → original
    /// VIP`. The egress reverse-NAT path uses this to rewrite the
    /// source 5-tuple of a backend response packet back to the VIP
    /// the client connected to.
    reverse_nat: BTreeMap<BackendKey, Ipv4Addr>,
}

impl ServiceState {
    const fn new() -> Self {
        Self { services: BTreeMap::new(), reverse_nat: BTreeMap::new() }
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
    /// `LOCAL_BACKEND_MAP` mirror per ADR-0053 § 1. Single
    /// `(vip, vip_port) → backend` map populated by
    /// `register_local_backend` / `deregister_local_backend`.
    /// `BTreeMap` per `.claude/rules/development.md` §
    /// "Ordered-collection choice" — DST observers walk the
    /// map and require deterministic iteration order.
    local_backends: Mutex<BTreeMap<(Ipv4Addr, u16, Proto), SocketAddrV4>>,
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
            local_backends: Mutex::new(BTreeMap::new()),
        }
    }

    /// Read the locally-registered backend for `(vip, vip_port)`, if
    /// any. Returns `Option<SocketAddrV4>` directly — a DST-convenient
    /// shape that differs from production `EbpfDataplane`'s
    /// `local_backend_for` accessor (which returns
    /// `Result<Option<LocalBackendEntry>>`, since real BPF map I/O is
    /// fallible). The contract clause both adapters satisfy is the
    /// observable post-state described in the `Dataplane` trait's
    /// `register_local_backend` postcondition: a `connect(vip:vip_port)`
    /// from inside the attach cgroup reaches the resolved backend.
    /// Production verifies that via the walking-skeleton integration
    /// test; the sim adapter exposes this accessor so DST invariant
    /// evaluators can assert on the same post-state without loading a
    /// real kernel.
    ///
    /// This is a test-only accessor for DST invariant evaluators —
    /// not part of the `Dataplane` trait. Existence here is a testing
    /// convenience, not a trait-contract violation.
    #[must_use]
    pub fn local_backend_for(
        &self,
        vip: Ipv4Addr,
        vip_port: u16,
        proto: Proto,
    ) -> Option<SocketAddrV4> {
        self.local_backends.lock().get(&(vip, vip_port, proto)).copied()
    }

    /// Snapshot every `(vip, port, backend)` triple currently in the
    /// local-backend mirror, in `Ord` order on `(Ipv4Addr, u16)`.
    /// Iteration order is a function of the keys (`BTreeMap`
    /// invariant), never of insertion history — the property DST
    /// seed reproducibility relies on.
    ///
    /// This is a test-only accessor for DST invariant evaluators —
    /// not part of the `Dataplane` trait. The production
    /// `EbpfDataplane` does not expose an exactly-equivalent surface
    /// (it has `local_backend_map_entries()` returning
    /// `Vec<(LocalServiceKey, LocalBackendEntry)>` — same logical
    /// content, different value-shape for the real BPF map row type).
    /// Existence here is a testing convenience, not a trait-contract
    /// violation.
    #[must_use]
    pub fn local_backends(&self) -> Vec<(Ipv4Addr, u16, Proto, SocketAddrV4)> {
        self.local_backends.lock().iter().map(|(&(v, p, pr), &b)| (v, p, pr, b)).collect()
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
        for ((v, _proto), backends) in &state.services {
            if *v == vip {
                merged.extend(backends.iter().cloned());
            }
        }
        drop(state);
        if merged.is_empty() { None } else { Some(merged) }
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
        let mut vips: Vec<Ipv4Addr> = state.services.keys().map(|(v, _proto)| *v).collect();
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
/// ICMP / SCTP are GH #155 / future-phase deferrals. Non-IPv4 backends
/// contribute no key (silently skipped, parity with the production
/// `EbpfDataplane`).
const fn reverse_nat_key_for(backend: &Backend, proto: Proto) -> Option<BackendKey> {
    match backend.addr.ip() {
        std::net::IpAddr::V4(v4) => Some(BackendKey::new(v4, backend.addr.port(), proto)),
        std::net::IpAddr::V6(_) => None,
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
        // `frontend.vip` is IPv4-by-construction (ADR-0060 D1a) —
        // narrow infallibly. `proto` is the single declared protocol;
        // the reverse-NAT fan-out is per-proto (D4), never the legacy
        // `[Tcp, Udp]` over-install.
        let vip = frontend.vip_v4();
        let proto = frontend.proto();

        // Single mutex acquisition guards both maps — observers
        // cannot witness a partial update. Mirrors the production
        // `EbpfDataplane`'s `REVERSE_NAT_MAP` lockstep contract:
        // `SERVICE_MAP` and `REVERSE_NAT_MAP` updates land in the same
        // critical section.
        let mut state = self.state.lock();

        // Snapshot prior reverse-NAT keys for this `(vip, proto)`
        // before any mutation — the diff drives the purge below. Keyed
        // per-proto so a co-resident other-proto frontend on the same
        // VIP is untouched (D4).
        let prior_keys: std::collections::BTreeSet<BackendKey> = state
            .services
            .get(&(vip, proto))
            .map(|prior| prior.iter().filter_map(|b| reverse_nat_key_for(b, proto)).collect())
            .unwrap_or_default();

        // Compute new reverse-NAT keys for the incoming backend set,
        // under the declared proto only.
        let new_keys: std::collections::BTreeSet<BackendKey> =
            backends.iter().filter_map(|b| reverse_nat_key_for(b, proto)).collect();

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
            state.services.remove(&(vip, proto));
        } else {
            state.services.insert((vip, proto), backends);
        }

        // Compute the union of ALL active services' reverse-NAT keys
        // (after the forward-path update above), each under its own
        // stored proto. Only purge entries that left THIS service AND
        // are absent from the global set. Without this cross-service
        // check, removing a backend from one service would delete the
        // reverse-NAT entry even when another service still routes
        // through the same backend.
        let live_keys: std::collections::BTreeSet<BackendKey> = state
            .services
            .iter()
            .flat_map(|((_v, p), bs)| bs.iter().filter_map(move |b| reverse_nat_key_for(b, *p)))
            .collect();

        for key in prior_keys.difference(&new_keys) {
            if !live_keys.contains(key) {
                state.reverse_nat.remove(key);
            }
        }

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

    async fn register_local_backend(
        &self,
        vip: Ipv4Addr,
        vip_port: u16,
        backend: SocketAddrV4,
        proto: Proto,
    ) -> Result<(), DataplaneError> {
        // Single-map point write per ADR-0053 § 2 (rev 2026-06-03) —
        // keyed on `(vip, vip_port, proto)` so a co-located tcp/53 +
        // udp/53 install two distinct entries, observably-equivalent
        // to `EbpfDataplane`'s `LOCAL_BACKEND_MAP` (vip, port, proto)
        // key. Overwrites any pre-existing entry for the SAME quadruple
        // atomically (the mutex acquisition IS the atomicity boundary).
        self.local_backends.lock().insert((vip, vip_port, proto), backend);
        Ok(())
    }

    async fn deregister_local_backend(
        &self,
        vip: Ipv4Addr,
        vip_port: u16,
        proto: Proto,
    ) -> Result<(), DataplaneError> {
        // Idempotent per ADR-0053 § 2 — removing an entry that does
        // not exist is `Ok(())`, never `KeyNotFound`. Removes only the
        // `(vip, vip_port, proto)` entry; a co-located other-proto
        // entry on the same `(vip, vip_port)` is left intact.
        self.local_backends.lock().remove(&(vip, vip_port, proto));
        Ok(())
    }
}
