//! [`Dataplane`] — kernel-side enforcement boundary.
//!
//! Control-plane logic never loads eBPF programs or touches BPF maps
//! directly. Every change it wants to apply crosses this trait. Production
//! wires this to `EbpfDataplane` (aya-rs); tests wire it to `SimDataplane`
//! (in-memory `HashMap`).
//!
//! See `docs/whitepaper.md` §7 for the dataplane's kernel surface.

use std::net::SocketAddrV4;

use async_trait::async_trait;
use thiserror::Error;

use crate::SpiffeId;
use crate::dataplane::ServiceFrontend;
use crate::dataplane::backend_key::BackendKey;
use crate::dataplane::backend_key::Proto;
use crate::id::{BackendId, ServiceVip};

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
    /// `update_service` found one `BackendId` already bound to a different
    /// backend identity (the memo hit names another `Backend.alloc`). Returned
    /// before any map is mutated; no identity is reserved and no id is minted.
    #[error("backend id {backend_id} is bound to {current}, refusing {attempted}")]
    BackendIdentityConflict { backend_id: BackendId, current: SpiffeId, attempted: SpiffeId },
    /// `update_service` found a reverse-NAT key that already maps to a different
    /// VIP (R-59). Returned from planning, before any mutation: nothing is
    /// overwritten and rollback never restores another Service's entry.
    #[error("reverse-NAT key {key} already maps to {current_vip}, refusing {attempted_vip}")]
    ReverseNatConflict { key: BackendKey, current_vip: ServiceVip, attempted_vip: ServiceVip },
    /// A pre-commit `update_service` failure at `stage` whose restore also
    /// failed for one or more keys (`rollback`); when the restore fully
    /// succeeds the original stage error is returned instead.
    #[error("service apply failed at {stage:?} and {} restore delete(s) failed", rollback.len())]
    ServiceApplyRollback { stage: ServiceApplyStage, rollback: Vec<ServiceApplyRollbackFailure> },
}

/// The pre-commit stage of `update_service` that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceApplyStage {
    BackendMap,
    InnerMap,
    ReverseNat,
    OuterCommit,
}

/// One restore delete that failed during the rollback of a pre-commit failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceApplyRollbackFailure {
    BackendDelete(BackendId),
    /// The in-place `BACKEND_MAP` restore of this id's prior value failed (R-60).
    BackendRestore(BackendId),
    /// The reverse-NAT key that could not be deleted, as the backend endpoint it
    /// keys; the dataplane-private `BackendKeyPod` is converted at the adapter.
    ReverseNatDelete {
        backend: SocketAddrV4,
        proto: Proto,
    },
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
