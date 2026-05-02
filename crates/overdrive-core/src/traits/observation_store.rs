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
use crate::transition_reason::TransitionReason;

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
///
/// `Failed` is the explicit terminal state a driver-rejected start
/// (or any cause-class failure transition) lands in. Per ADR-0032 §3
/// (Amendment 2026-04-30) the failure cause is structurally captured
/// on the `AllocStatusRow` via `reason: Option<TransitionReason>`; the
/// `Failed` state is the lifecycle bucket those rows live in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum AllocState {
    Pending,
    Running,
    Draining,
    Suspended,
    Terminated,
    /// Driver-rejected start, restart-budget exhaustion, cancellation,
    /// no-capacity, or any other cause-class failure. Mirrors
    /// `TransitionReason::is_failure() == true`. Per ADR-0032 §3.
    Failed,
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
            Self::Failed => "failed",
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct LogicalTimestamp {
    pub counter: u64,
    pub writer: NodeId,
}

impl LogicalTimestamp {
    /// Total order on [`LogicalTimestamp`]: `(counter, writer)`
    /// lexicographic. Returns `true` when `self` strictly dominates
    /// `other` and therefore wins under last-write-wins.
    ///
    /// Equal timestamps (same counter AND same writer) are treated as
    /// "not dominating" — the existing value is retained. This is the
    /// LWW idempotency case: re-delivering the same row via gossip is a
    /// no-op.
    ///
    /// The counter dominates first; on a tie, the writer's
    /// [`NodeId::Display`] form is the canonical ordering key, matching
    /// the §4 whitepaper rule for deterministic tiebreak. Clock skew
    /// across peers cannot invert ordering — the counter is a Lamport
    /// stamp, not a wall-clock time.
    ///
    /// This is the single comparator both [`ObservationStore`] adapters
    /// (`SimObservationStore` in `overdrive-sim`, `LocalObservationStore`
    /// in `overdrive-store-local`) MUST consult when applying a write.
    /// See `docs/feature/fix-observation-lww-merge/deliver/rca.md` for
    /// the bug RCA that motivated promoting this comparator out of the
    /// sim leaf crate.
    #[must_use]
    pub fn dominates(&self, other: &Self) -> bool {
        match self.counter.cmp(&other.counter) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            // Tiebreak on writer: lexicographically greater writer wins.
            // `NodeId`'s `Display` form is the canonical ordering key
            // and is what the §4 "deterministic tiebreak" rule consumes.
            std::cmp::Ordering::Equal => self.writer.to_string() > other.writer.to_string(),
        }
    }
}

/// `alloc_status` row — Phase 1 minimal shape per brief §6, extended
/// per ADR-0032 §3 (Amendment 2026-04-30) and §4 with cause-class
/// attribution.
///
/// Written by the node that owns the allocation; gossiped to every peer.
/// Full-row writes only (no field-diff merges) per the §4 guardrail.
///
/// # Cause-class attribution
///
/// `reason` carries the structured `TransitionReason` for the most
/// recent transition that produced this row. Progress markers
/// (`Scheduling`, `Starting`, `Started`, `BackoffPending`, `Stopped`)
/// describe healthy lifecycle progress; cause-class variants
/// (`ExecBinaryNotFound`, `CgroupSetupFailed`, `RestartBudgetExhausted`,
/// `Cancelled`, `NoCapacity`, …) describe failure transitions and
/// pair with `state == AllocState::Failed`.
///
/// `detail` carries verbatim driver text the typed `reason` payload
/// does not capture — most commonly the raw `errno`-decorated message
/// from `std::io::Error::Display` for cgroup / spawn failures. The
/// typed payload is the load-bearing artifact; `detail` is the human-
/// readable fallback for cases the cause-class taxonomy has not yet
/// grown a variant for.
///
/// # Forward compatibility
///
/// Both `reason` and `detail` are `Option<…>` and additive on the rkyv
/// archive shape — pre-feature redb files (where neither field exists)
/// continue to deserialise (rkyv treats `Option<T>` such that omitted
/// data deserialises to `None`). New writers populate both fields per
/// the action-shim contract (ADR-0023); old readers tolerate them.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct AllocStatusRow {
    pub alloc_id: AllocationId,
    pub job_id: JobId,
    pub node_id: NodeId,
    pub state: AllocState,
    pub updated_at: LogicalTimestamp,
    /// Structured cause for this transition. `None` when the writer
    /// (Phase 1: action shim) has not yet been wired to populate it,
    /// or when the row predates the schema extension.
    pub reason: Option<TransitionReason>,
    /// Verbatim driver / OS text the `reason` payload does not capture.
    /// Used for diagnostic fidelity on cause variants whose typed
    /// payload is incomplete (e.g. `DriverInternalError { detail }`
    /// duplicates this for self-containment, but other cause variants
    /// may carry only structured fields and rely on `detail` for the
    /// raw `errno` text).
    pub detail: Option<String>,
}

/// `node_health` row — Phase 1 minimal shape per brief §6.
///
/// Written by the node itself on each heartbeat tick.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
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
    ///
    /// # LWW contract
    ///
    /// An incoming row whose `updated_at` does not dominate the existing
    /// row at the same primary key MUST NOT mutate state and MUST NOT be
    /// emitted on subscriptions. Adapters MUST consult
    /// [`LogicalTimestamp::dominates`] for this comparison; the equal
    /// timestamp case (re-delivery of the same row) is treated as a
    /// no-op for the same reason.
    ///
    /// This contract is exercised by the trait-conformance harness at
    /// `overdrive_core::testing::observation_store::run_lww_conformance`.
    /// The two adapter implementations in this workspace —
    /// `SimObservationStore` and `LocalObservationStore` — honour the
    /// contract. Future adapters (Phase 2 Corrosion replacement, any
    /// future test fakes) MUST honour it identically.
    ///
    /// See `docs/whitepaper.md` §4 (Intent / Observation split,
    /// "tombstones, full rows over field diffs") and
    /// `docs/feature/fix-observation-lww-merge/deliver/rca.md` for the
    /// bug RCA that codified this trait-level invariant.
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
    async fn alloc_status_rows(&self) -> Result<Vec<AllocStatusRow>, ObservationStoreError>;

    /// Read the LWW-winner `alloc_status` row for a single
    /// [`AllocationId`], if any. Adapters MUST implement this as a
    /// direct point lookup against the per-alloc index — never as a
    /// scan-and-filter over [`Self::alloc_status_rows`]. The §4 LWW
    /// invariant guarantees at most one winner per key; this method
    /// makes that invariant load-bearing at the type level.
    ///
    /// Used by the worker subsystems (`exit_observer`, `action_shim`)
    /// to recover the prior `(job_id, node_id, updated_at)` tuple
    /// when writing a successor row. The previous shape — calling
    /// `alloc_status_rows()` and then `find`/`max_by_key` over the
    /// result — encoded a false suggestion that the contract permits
    /// duplicates and added an unjustified `O(n)` scan to a hot path.
    async fn alloc_status_row(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Option<AllocStatusRow>, ObservationStoreError>;

    /// Read a deterministic snapshot of every `node_health` row this
    /// peer has observed. Phase 1 has no LWW current-row index for
    /// `node_health` (see `SimObservationStore::apply`) — callers see
    /// the full ordered history; Phase 2 will add LWW parallel
    /// tracking and this method will return winners only.
    async fn node_health_rows(&self) -> Result<Vec<NodeHealthRow>, ObservationStoreError>;
}
