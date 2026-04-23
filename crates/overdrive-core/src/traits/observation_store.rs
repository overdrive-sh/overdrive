//! [`ObservationStore`] — live eventually-consistent cluster map.
//!
//! Allocation status, service backends, node health, compiled policy
//! verdicts. Every node writes its own rows; every node reads locally.
//! Production uses Corrosion (cr-sqlite + SWIM/QUIC); simulation uses
//! `SimObservationStore` with an injectable gossip-delay and partition
//! matrix.
//!
//! # Why typed rows, not `&[u8]`
//!
//! `ObservationStore` is the observation half of the §4 Intent /
//! Observation split. Intent carries `JobSpec`, `Policy`, `Certificate`,
//! and other declaration-of-what-should-be types through [`IntentStore`].
//! Observation carries rows describing *what is happening right now*.
//!
//! A shared `write(&[u8])` surface on both stores would let a reconciler
//! accidentally route a job spec into observation (or a node heartbeat
//! into intent). The [`ObservationRow`] enum closes that door at the
//! type level: a `JobSpec` (intent class) cannot be passed to
//! [`ObservationStore::write`] — the compiler rejects it with a type
//! mismatch that names both sides.
//!
//! See `docs/whitepaper.md` §4 (Intent / Observation split) and §17
//! (storage rationale).

use async_trait::async_trait;
use futures::Stream;
use thiserror::Error;

use crate::id::{AllocationId, JobId, NodeId, Region};

#[derive(Debug, Error)]
pub enum ObservationStoreError {
    #[error("observation peer {peer} unreachable")]
    Unreachable { peer: String },
    #[error("observation store I/O: {0}")]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// Row types — observation class
// ---------------------------------------------------------------------------

/// Lifecycle state for an allocation as observed by the owning node.
///
/// Matches the lifecycle documented in whitepaper §4 and §14 —
/// `pending → running ⇄ suspended → terminated`, plus `draining` as the
/// transient state a node reports while migrating an allocation away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocState {
    Pending,
    Running,
    Draining,
    Suspended,
    Terminated,
}

impl std::fmt::Display for AllocState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Canonical lowercase form matches whitepaper §4 lifecycle
        // rendering. Used on the REST wire for `alloc_status.state`.
        let s = match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Draining => "draining",
            Self::Suspended => "suspended",
            Self::Terminated => "terminated",
        };
        f.write_str(s)
    }
}

/// Logical timestamp used for last-write-wins ordering across
/// [`ObservationStore`] peers.
///
/// `(counter, writer)` is lexicographically ordered: the lamport counter
/// dominates, and the writer's [`NodeId`] breaks ties deterministically.
/// Clock skew across peers cannot invert ordering.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LogicalTimestamp {
    pub counter: u64,
    pub writer: NodeId,
}

/// `alloc_status` row — Phase 1 minimal shape per brief §6.
///
/// Written by the node that owns the allocation; gossiped to every peer.
/// Full-row writes only (no field-diff merges) per the §4 guardrail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocStatusRow {
    pub alloc_id: AllocationId,
    pub job_id: JobId,
    pub node_id: NodeId,
    pub state: AllocState,
    pub updated_at: LogicalTimestamp,
}

/// `node_health` row — Phase 1 minimal shape per brief §6.
///
/// Written by the node itself on each heartbeat tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeHealthRow {
    pub node_id: NodeId,
    pub region: Region,
    pub last_heartbeat: LogicalTimestamp,
}

/// The closed set of row shapes [`ObservationStore`] accepts.
///
/// This enum *is* the compile-time boundary between intent and
/// observation: any type that is not a variant of [`ObservationRow`]
/// cannot be written into an [`ObservationStore`]. Phase 2+ extensions
/// add variants here as new row shapes are introduced (service
/// backends, compiled policy verdicts, revoked operator certs, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservationRow {
    AllocStatus(AllocStatusRow),
    NodeHealth(NodeHealthRow),
}

// ---------------------------------------------------------------------------
// Intent-class type used by the compile-fail fixture
// ---------------------------------------------------------------------------

/// Minimal `JobSpec` placeholder used exclusively by the §5.3
/// compile-fail fixture to prove [`ObservationStore::write`] rejects
/// intent-class payloads at compile time.
///
/// Phase 2+ will replace this stub with the full job-spec type declared
/// in a dedicated `intent` module. Until then, this type is carried here
/// solely so the compile-fail fixture has a concrete intent-class value
/// to attempt to write into the wrong store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSpec {
    pub owner: NodeId,
}

impl JobSpec {
    /// Construct a minimal job spec for compile-fail fixtures.
    #[must_use]
    pub const fn new(owner: NodeId) -> Self {
        Self { owner }
    }
}

// ---------------------------------------------------------------------------
// Subscription stream alias
// ---------------------------------------------------------------------------

/// A subscription stream over all observation rows written to or
/// gossiped into this peer.
///
/// Phase 2+ introduces a filter parameter (`prefix` / predicate) once
/// there are enough row variants to justify it; the Phase 1 sim surface
/// is intentionally "subscribe to everything."
pub type ObservationSubscription = Box<dyn Stream<Item = ObservationRow> + Send + Unpin>;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait ObservationStore: Send + Sync + 'static {
    /// Persist a full observation row on this peer and fan it out to
    /// any active subscriptions. Full-row writes only (§4 guardrail).
    async fn write(&self, row: ObservationRow) -> Result<(), ObservationStoreError>;

    /// Subscribe to every observation row written to this peer.
    async fn subscribe_all(&self) -> Result<ObservationSubscription, ObservationStoreError>;

    /// Read a deterministic snapshot of every `alloc_status` row this
    /// peer currently holds as LWW winner. Intended for point-in-time
    /// reads from the REST API (`GET /v1/allocs`) — reads locally, no
    /// cross-peer RPC. Iteration order is deterministic, keyed by
    /// `AllocationId`.
    ///
    /// Phase 1 motivation: the REST observation-read handlers land in
    /// step 03-03; the existing `subscribe_all` surface is suited to
    /// long-lived reactive consumers (reconcilers, dataplane hydration),
    /// not one-shot HTTP handlers. A typed snapshot is the honest read
    /// primitive for request/response handlers.
    async fn alloc_status_rows(&self)
        -> Result<Vec<AllocStatusRow>, ObservationStoreError>;

    /// Read a deterministic snapshot of every `node_health` row this
    /// peer has observed. Phase 1 has no LWW current-row index for
    /// `node_health` (see `SimObservationStore::apply`) — callers see
    /// the full ordered history; Phase 2 will add LWW parallel
    /// tracking and this method will return winners only.
    async fn node_health_rows(&self)
        -> Result<Vec<NodeHealthRow>, ObservationStoreError>;
}
