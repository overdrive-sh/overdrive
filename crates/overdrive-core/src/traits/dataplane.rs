//! [`Dataplane`] — kernel-side enforcement boundary.
//!
//! Control-plane logic never loads eBPF programs or touches BPF maps
//! directly. Every change it wants to apply crosses this trait. Production
//! wires this to `EbpfDataplane` (aya-rs); tests wire it to `SimDataplane`
//! (in-memory `HashMap`).
//!
//! See `docs/whitepaper.md` §7 for the dataplane's kernel surface.

use std::net::Ipv4Addr;

use async_trait::async_trait;
use thiserror::Error;

use crate::SpiffeId;

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
}

/// Policy decision compiled into the BPF `POLICY_MAP`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Deny,
}

/// A single service backend — IP/port and load-balancing weight.
#[derive(Debug, Clone, PartialEq, Eq)]
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

    /// Atomically replace the backend set for a service VIP.
    async fn update_service(
        &self,
        vip: Ipv4Addr,
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
