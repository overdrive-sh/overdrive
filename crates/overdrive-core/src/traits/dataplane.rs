//! [`Dataplane`] — kernel-side enforcement boundary.
//!
//! Control-plane logic never loads eBPF programs or touches BPF maps
//! directly. Every change it wants to apply crosses this trait. Production
//! wires this to `EbpfDataplane` (aya-rs); tests wire it to `SimDataplane`
//! (in-memory `HashMap`).
//!
//! See `docs/whitepaper.md` §7 for the dataplane's kernel surface.

use std::net::{Ipv4Addr, SocketAddrV4};

use async_trait::async_trait;
use thiserror::Error;

use crate::SpiffeId;
use crate::dataplane::ServiceFrontend;

#[derive(Debug, Error)]
pub enum DataplaneError {
    #[error("dataplane busy, retry later")]
    Busy,
    #[error("program failed to load: {0}")]
    LoadFailed(String),
    #[error("dataplane I/O: {0}")]
    Io(#[from] std::io::Error),
    /// Resolution of an interface name to a kernel ifindex failed —
    /// the named interface does not exist on the host. Surfaces
    /// `ENODEV` / `ENOENT` from `if_nametoindex(2)` per S-2.2-03.
    /// The loader uses this BEFORE attempting to load any BPF
    /// program; see `EbpfDataplane::new` in `overdrive-dataplane`.
    #[error("interface not found: {iface}")]
    IfaceNotFound { iface: String },
    /// Kernel rejected an inner-map allocation during the 5-step
    /// HASH_OF_MAPS atomic-swap primitive (ADR-0040 § 2 step 2 —
    /// `bpf(BPF_MAP_CREATE)`). On this error the existing outer-map
    /// pointer is **unchanged**: the swap aborts before step 3 (the
    /// load-bearing single-syscall pointer update). Surfaced as a
    /// distinct variant per `.claude/rules/development.md` § Errors
    /// — collapsing this into `LoadFailed(String)` would lose the
    /// preservation guarantee S-2.2-11 pins.
    #[error("inner-map allocation rejected by kernel: {source}")]
    MapAllocFailed {
        #[source]
        source: std::io::Error,
    },
    /// `LOCAL_BACKEND_MAP` insert rejected by the kernel (ADR-0053
    /// § 6). Surfaces from `register_local_backend` when the BPF
    /// HASH map update fails (EINVAL on malformed key, ENOMEM on
    /// kernel allocator exhaustion, EPERM if the map FD was
    /// invalidated). Distinct variant per `.claude/rules/
    /// development.md` § "Distinct failure modes get distinct
    /// error variants".
    #[error("LOCAL_BACKEND_MAP insert rejected by kernel: {source}")]
    LocalBackendInsert {
        #[source]
        source: std::io::Error,
    },
    /// `LOCAL_BACKEND_MAP` delete rejected by the kernel (ADR-0053
    /// § 6). Surfaces from `deregister_local_backend`. KeyNotFound
    /// is NOT surfaced here — it's idempotent per the trait contract.
    #[error("LOCAL_BACKEND_MAP delete rejected by kernel: {source}")]
    LocalBackendDelete {
        #[source]
        source: std::io::Error,
    },
    /// `LOCAL_BACKEND_MAP` probe sentinel round-trip failed (ADR-0053
    /// § 6 Earned-Trust probe extension). The composition root's
    /// "wire then probe then use" invariant per ADR-0052 § 3.
    #[error("LOCAL_BACKEND_MAP probe round-trip failed: {message}")]
    LocalBackendProbe { message: String },
    /// An XDP program attach returned `EBUSY` — the kernel permits
    /// exactly one program per netdev XDP hook, and that hook on
    /// `iface` is already occupied (ADR-0061 § 5 / D3). Surfaces when
    /// the loader's forward (`client_iface`) and reverse
    /// (`backend_iface`) attaches both target one netdev — the
    /// single-node default `DataplaneConfig::loopback()` does exactly
    /// this until 01-03 lands. Distinct variant per
    /// `.claude/rules/development.md` § Errors: collapsing `EBUSY`
    /// into `LoadFailed(String)` would mask the real cause behind a
    /// misleading "native attach failed" / `DRV_MODE` string and
    /// prescribe the wrong remediation.
    #[error(
        "XDP slot on interface '{iface}' is already occupied (EBUSY): the kernel \
         permits exactly one XDP program per netdev hook. The single-node default \
         expects a dedicated veth pair — verify client_iface != backend_iface, and \
         detach any stale Overdrive XDP program with `ip link show {iface}` (see \
         debugging.md § \"Leftover XDP attachments across runs\")"
    )]
    IfaceXdpSlotBusy { iface: String },
}

/// Policy decision compiled into the BPF `POLICY_MAP`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Deny,
}

/// A single service backend — IP/port and load-balancing weight.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Backend {
    pub alloc: SpiffeId,
    pub addr: std::net::SocketAddr,
    pub weight: u16,
    pub healthy: bool,
}

/// Policy lookup key — source and destination identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PolicyKey {
    pub src: SpiffeId,
    pub dst: SpiffeId,
}

#[async_trait]
pub trait Dataplane: Send + Sync + 'static {
    /// Install or update a single policy verdict.
    async fn update_policy(&self, key: PolicyKey, verdict: Verdict) -> Result<(), DataplaneError>;

    /// Atomically replace the backend set for the service frontend
    /// `(frontend.vip, frontend.port, frontend.proto)`.
    ///
    /// # Preconditions
    ///
    /// - `frontend` is V4-guaranteed by construction
    ///   ([`ServiceFrontend::new`] rejected IPv6 at the action-shim);
    ///   adapters narrow `frontend.vip → Ipv4Addr` infallibly via
    ///   [`ServiceFrontend::vip_v4`]. No adapter re-validates the VIP.
    /// - `backends` MAY be empty (see edge cases). Each `Backend.addr`
    ///   is a `SocketAddr`; non-IPv4 backend addresses are skipped from
    ///   the `REVERSE_NAT` key set (GH #155 deferral).
    ///
    /// # Postconditions on `Ok(())`
    ///
    /// After return, the adapter's `REVERSE_NAT` key set **for
    /// `frontend.proto`** equals exactly the keys derived from
    /// `backends`: `{ BackendKey { ip, port: backend.addr.port(),
    /// proto: frontend.proto } : backend ∈ backends, backend.addr is
    /// IPv4 }`, each mapping to `frontend.vip_v4()`. Keys for **other**
    /// protocols of the same VIP — installed by separate per-listener
    /// `update_service` calls — are untouched.
    ///
    /// # Edge cases
    ///
    /// - `backends.is_empty()` ⇒ **per-proto purge** (ADR-0060 D4). The
    ///   adapter removes the prior `frontend.proto` `REVERSE_NAT` keys
    ///   for this VIP that are not still live in another service's
    ///   backend set (the existing `live_keys` difference check).
    ///   `REVERSE_NAT` keys for *other* protocols of the same VIP are
    ///   **not** removed.
    /// - Idempotent re-apply: calling `update_service(frontend,
    ///   backends)` twice with identical arguments yields the same
    ///   post-state.
    /// - A backend with an IPv6 `addr` contributes no `REVERSE_NAT` key
    ///   (silently skipped).
    ///
    /// # Observable invariant (cross-adapter)
    ///
    /// For the same `(frontend, backends)`, `SimDataplane` and
    /// `EbpfDataplane` install the **identical** `(ip, port, proto) →
    /// vip` `REVERSE_NAT` set. The `ReverseNatLockstep` three-tier gate
    /// enforces this (per `.claude/rules/development.md` § "The DST
    /// equivalence test is the structural guard").
    async fn update_service(
        &self,
        frontend: ServiceFrontend,
        backends: Vec<Backend>,
    ) -> Result<(), DataplaneError>;

    /// Drain queued flow events (for telemetry consumers).
    async fn drain_flow_events(&self) -> Result<Vec<FlowEvent>, DataplaneError>;

    /// Register or replace the local backend for `(vip, vip_port)`
    /// per ADR-0053 § 2.
    ///
    /// # Preconditions
    /// - `vip` is an IPv4 service VIP issued by `ServiceVipAllocator`
    ///   (ADR-0049). The allocator produces values of the
    ///   `overdrive_core::id::ServiceVip` newtype (validated `Ipv4Addr`
    ///   range), and `ServiceMapHydrator` extracts the inner
    ///   `Ipv4Addr` via `ServiceVip::get()` immediately before
    ///   emitting `Action::RegisterLocalBackend` — so every VIP
    ///   reaching this method has transited the typed allocator
    ///   surface. The signature stays `Ipv4Addr` (rather than
    ///   `ServiceVip`) for parallel with `update_service.vip` and to
    ///   keep the cgroup-path call shape identical to the XDP path
    ///   per ADR-0053 § 1; the type-system enforcement lives one
    ///   call site up the stack.
    /// - `backend` is a `SocketAddrV4` reachable from the host
    ///   netns. Phase 1 single-node guarantees this when the
    ///   backend's allocation is Running on the same host.
    ///
    /// # Postconditions on `Ok(())`
    /// - For every subsequent `connect(vip:vip_port)` from a
    ///   process inside the dataplane's attach cgroup, the kernel
    ///   establishes a connection to `backend.ip():backend.port()`
    ///   instead. **This is the observable postcondition every
    ///   adapter MUST satisfy** — the
    ///   `backend-discovery-bridge/walking_skeleton` integration
    ///   test exercises exactly this: register a backend, perform a
    ///   real `connect(vip:vip_port)` from inside the attach cgroup,
    ///   assert the peer is `backend`. Per
    ///   `.claude/rules/development.md` § "Trait definitions specify
    ///   behavior, not just signature" — this is the contract clause
    ///   the DST/integration equivalence harnesses check.
    /// - The application's `getpeername(2)` returns `backend`, not
    ///   `(vip, vip_port)`. Per Cilium ClusterIP semantics; see
    ///   ADR-0053 § "Consequences".
    ///
    /// # Edge cases
    /// - Re-registration with the same `backend` is idempotent; the
    ///   map update is the same triple.
    /// - Re-registration with a different `backend` for the same
    ///   `(vip, vip_port)` atomically replaces the existing entry
    ///   (single-map point write; no in-flight readers between the
    ///   syscall returning and the next `connect`).
    ///
    /// # Observable invariants
    /// - After `deregister_local_backend(vip, vip_port)`,
    ///   subsequent `connect(vip:vip_port)` reaches the kernel
    ///   without rewrite — the connect either succeeds against
    ///   whatever the VIP was *originally* routed to (typically
    ///   nothing in Phase 1, producing `ECONNREFUSED`), or fails
    ///   with `EHOSTUNREACH`. The cgroup hook does not deny; it
    ///   only rewrites.
    /// - `update_service(vip, ...)` and `register_local_backend(vip,
    ///   port, ...)` for the same VIP are NOT mutually exclusive
    ///   at the adapter — the XDP path consumes the first, the
    ///   cgroup path consumes the second. The classifier in
    ///   `ServiceMapHydrator` (ADR-0053 § 4) is responsible for
    ///   choosing exactly one per backend.
    async fn register_local_backend(
        &self,
        vip: Ipv4Addr,
        vip_port: u16,
        backend: SocketAddrV4,
    ) -> Result<(), DataplaneError>;

    /// Remove the local backend registration for `(vip, vip_port)`
    /// per ADR-0053 § 2.
    ///
    /// # Preconditions
    /// - None. The method is idempotent: removing an entry that
    ///   does not exist is `Ok(())`, NOT an error.
    ///
    /// # Postconditions on `Ok(())`
    /// - The cgroup hook will no longer rewrite `connect(vip:vip_port)`
    ///   calls for this `(vip, vip_port)`. The kernel proceeds with
    ///   the operator-supplied destination, which in Phase 1 typically
    ///   produces `ECONNREFUSED` (no listener on the VIP itself).
    ///   This is the observable postcondition every adapter MUST
    ///   satisfy — verifiable via a real `connect(vip:vip_port)`
    ///   from inside the attach cgroup after deregistration (the
    ///   inverse of the `register_local_backend` walking-skeleton
    ///   check).
    ///
    /// # Edge cases
    /// - Removing a `(vip, vip_port)` that was never registered
    ///   succeeds with no side effect.
    /// - Concurrent `register_local_backend(vip, vip_port, b1)` +
    ///   `deregister_local_backend(vip, vip_port)` is not defined
    ///   to interleave at the adapter — callers must serialise.
    async fn deregister_local_backend(
        &self,
        vip: Ipv4Addr,
        vip_port: u16,
    ) -> Result<(), DataplaneError>;
}

/// A single kernel-emitted flow record. See `docs/whitepaper.md` §12.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowEvent {
    pub src: SpiffeId,
    pub dst: SpiffeId,
    pub verdict: Verdict,
    pub bytes_up: u64,
    pub bytes_down: u64,
}
