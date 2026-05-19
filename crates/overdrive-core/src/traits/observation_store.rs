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

use std::net::Ipv4Addr;

use crate::aggregate::{Listener, ServiceVip, WorkloadKind};
use crate::codec::{EnvelopeError, VersionedEnvelope};
use crate::dataplane::backend_key::Proto;
use crate::dataplane::fingerprint::BackendSetFingerprint;
use crate::id::{AllocationId, NodeId, Region, ServiceId, WorkloadId};
use crate::traits::dataplane::Backend;
use crate::transition_reason::{TerminalCondition, TransitionReason};
use crate::wall_clock::UnixInstant;

#[derive(Debug, Error)]
pub enum ObservationStoreError {
    #[error("observation peer {peer} unreachable")]
    Unreachable { peer: String },
    #[error("observation store I/O: {0}")]
    Io(#[from] std::io::Error),
    // SCAFFOLD: true — RED scaffold per ADR-0048 § 3 (asymmetric read
    // policy; observation log + skip on envelope decode failure).
    // Lands GREEN in DELIVER step 02-01..02-03 when each
    // `LocalObservationStore::*_rows` adapter wires the envelope
    // decode path.
    #[error("observation envelope decode failed: {source}")]
    Envelope {
        #[from]
        #[source]
        source: EnvelopeError,
    },
}

impl ObservationStoreError {
    /// Classify whether this error is a transient condition the caller
    /// should retry, or a terminal failure that must be surfaced via a
    /// louder failure mode.
    ///
    /// Used by `worker::exit_observer` to gate a bounded retry loop on
    /// the obs-write path: transient errors (e.g. a transiently
    /// unreachable peer, or genuinely retryable I/O kinds) re-attempt
    /// the write; terminal errors short-circuit to a degraded
    /// `LifecycleEvent` so subscribers see the failure surface rather
    /// than an alloc silently stuck `Running`.
    ///
    /// # Classification policy
    ///
    /// - [`Self::Unreachable`] — always retryable. The peer may be
    ///   transiently down (gossip in flight, network blip); a bounded
    ///   retry window is the right shape.
    /// - [`Self::Io`] — retryable only for genuinely transient
    ///   `io::ErrorKind` values: `Interrupted` (syscall interrupted by
    ///   signal), `WouldBlock` (non-blocking I/O hit back-pressure),
    ///   `TimedOut` (operation deadline elapsed), `ResourceBusy`
    ///   (kernel/backend held a lock). Every other `io::ErrorKind`
    ///   (`PermissionDenied`, `AlreadyExists`, `NotFound` on a write
    ///   path, `OutOfMemory`, `Other`, `Unsupported`, …) is a terminal
    ///   condition where retrying cannot succeed — return `false` so
    ///   the caller escalates immediately.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Unreachable { .. } => true,
            Self::Io(err) => matches!(
                err.kind(),
                std::io::ErrorKind::Interrupted
                    | std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::ResourceBusy
            ),
            // Envelope decode failures are terminal — the bytes do
            // not get any more decodable on retry. Per ADR-0048 § 3
            // the observation-layer caller logs + skips the row
            // rather than retrying.
            Self::Envelope { .. } => false,
        }
    }
}

#[cfg(test)]
mod is_retryable_tests {
    use super::ObservationStoreError;
    use std::io;

    #[test]
    fn unreachable_is_retryable() {
        let err = ObservationStoreError::Unreachable { peer: "node-2".to_owned() };
        assert!(err.is_retryable(), "Unreachable variant must be classified retryable");
    }

    #[test]
    fn io_interrupted_is_retryable() {
        let err = ObservationStoreError::Io(io::Error::from(io::ErrorKind::Interrupted));
        assert!(err.is_retryable(), "Io(Interrupted) must be classified retryable");
    }

    #[test]
    fn io_would_block_is_retryable() {
        let err = ObservationStoreError::Io(io::Error::from(io::ErrorKind::WouldBlock));
        assert!(err.is_retryable(), "Io(WouldBlock) must be classified retryable");
    }

    #[test]
    fn io_timed_out_is_retryable() {
        let err = ObservationStoreError::Io(io::Error::from(io::ErrorKind::TimedOut));
        assert!(err.is_retryable(), "Io(TimedOut) must be classified retryable");
    }

    #[test]
    fn io_resource_busy_is_retryable() {
        let err = ObservationStoreError::Io(io::Error::from(io::ErrorKind::ResourceBusy));
        assert!(err.is_retryable(), "Io(ResourceBusy) must be classified retryable");
    }

    #[test]
    fn io_permission_denied_is_terminal() {
        let err = ObservationStoreError::Io(io::Error::from(io::ErrorKind::PermissionDenied));
        assert!(!err.is_retryable(), "Io(PermissionDenied) must be terminal");
    }

    #[test]
    fn io_already_exists_is_terminal() {
        let err = ObservationStoreError::Io(io::Error::from(io::ErrorKind::AlreadyExists));
        assert!(!err.is_retryable(), "Io(AlreadyExists) must be terminal");
    }

    #[test]
    fn io_not_found_is_terminal() {
        let err = ObservationStoreError::Io(io::Error::from(io::ErrorKind::NotFound));
        assert!(!err.is_retryable(), "Io(NotFound) must be terminal on a write path");
    }

    #[test]
    fn io_out_of_memory_is_terminal() {
        let err = ObservationStoreError::Io(io::Error::from(io::ErrorKind::OutOfMemory));
        assert!(!err.is_retryable(), "Io(OutOfMemory) must be terminal");
    }

    #[test]
    fn io_other_is_terminal() {
        let err = ObservationStoreError::Io(io::Error::from(io::ErrorKind::Other));
        assert!(!err.is_retryable(), "Io(Other) must be terminal — unknown kinds are not retried");
    }

    #[test]
    fn io_unsupported_is_terminal() {
        let err = ObservationStoreError::Io(io::Error::from(io::ErrorKind::Unsupported));
        assert!(!err.is_retryable(), "Io(Unsupported) must be terminal");
    }
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
/// # Schema evolution
///
/// Per ADR-0048 (`docs/product/architecture/adr-0048-rkyv-versioned-envelope.md`)
/// this type is the **inner payload** of [`AllocStatusRowEnvelope`].
/// rkyv archives are **fixed positional layouts** — appending a field
/// to this struct shifts every subsequent offset and renders
/// pre-existing bytes unreadable. The previous docstring claim that
/// `Option<T>` fields are "additive on the rkyv archive shape" was
/// incorrect (RCA: `docs/feature/rkyv-envelope-evolution/distill/`)
/// — schema evolution at this boundary goes through a new envelope
/// variant (`V2`, `V3`, …) added per the version-bump procedure in
/// `.claude/rules/development.md` § "rkyv schema evolution"; existing
/// `FIXTURE_V<N>` golden bytes are NEVER touched.
///
/// Writers go through [`AllocStatusRow::latest`]
/// (= [`AllocStatusRowEnvelope::latest`]); readers project through
/// [`AllocStatusRowEnvelope::into_latest`].
pub type AllocStatusRow = AllocStatusRowV1;

/// Observation-side twin of the intent-side [`Listener`] per ADR-0011.
///
/// Carries `(port, protocol, vip)` — the same triple shape as the
/// intent-side [`Listener`], but distinct as a type so the bounded
/// context boundary stays load-bearing. The action shim's
/// `build_alloc_status_row` copies from intent-side listeners onto this
/// shape at write time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ListenerRow {
    pub port: std::num::NonZeroU16,
    pub protocol: Proto,
    pub vip: Option<ServiceVip>,
}

impl From<&Listener> for ListenerRow {
    fn from(l: &Listener) -> Self {
        // Per ADR-0049 § 5 / service-vip-allocator step 02-01 the
        // intent-side [`Listener`] no longer carries a `vip` field;
        // VIPs are platform-issued at the service level via
        // `ServiceVipAllocator`. Observation-side `ListenerRow.vip`
        // is populated downstream by the allocator / action-shim
        // path, not by mirroring the intent-side spec. Today's
        // observation-row writers all construct
        // `listeners: Vec::new()`, so this `From` impl is a forward-
        // compat shim — the `vip: None` projection is the right
        // default when the call site has no allocator context.
        Self { port: l.port, protocol: l.protocol, vip: None }
    }
}

/// `node_health` row — Phase 1 minimal shape per brief §6.
///
/// Written by the node itself on each heartbeat tick.
///
/// # Schema evolution
///
/// Per ADR-0048 (`docs/product/architecture/adr-0048-rkyv-versioned-envelope.md`)
/// this type is the **inner payload** of [`NodeHealthRowEnvelope`]
/// under the UI-02 amendment alias-to-payload public API. rkyv
/// archives are **fixed positional layouts** — appending a field
/// to this struct shifts every subsequent offset and renders
/// pre-existing bytes unreadable. Schema evolution at this boundary
/// goes through a new envelope variant (`V2`, `V3`, …) added per
/// the version-bump procedure in `.claude/rules/development.md`
/// § "rkyv schema evolution"; existing `FIXTURE_V<N>` golden bytes
/// are NEVER touched.
///
/// Writers go through [`NodeHealthRow::latest`]
/// (= [`NodeHealthRowEnvelope::latest`]); readers project through
/// [`NodeHealthRowEnvelope::into_latest`].
pub type NodeHealthRow = NodeHealthRowV1;

/// Status of a service-hydration dispatch attempt — one source of
/// truth per `service_hydration_results` row per
/// `docs/feature/phase-2-xdp-service-map/design/architecture.md`
/// §§ 7, 12.
///
/// `Pending` is the row shape the hydrator (Slice 08-02) writes
/// before invoking dispatch; `Completed` and `Failed` are the
/// post-dispatch terminal-of-attempt rows the action shim writes
/// from `Action::DataplaneUpdateService`. Per architecture.md § 7,
/// the failure surface is observation, NOT a `TerminalCondition`
/// claim — service hydration cannot terminate an allocation; this
/// enum carries every dispatch outcome.
///
/// Variant ordering and discriminants are STABLE — additions are
/// minor-version (per ADR-0037 K8s-Condition convention); reordering
/// or removal is a major-version break that requires a new ADR.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum ServiceHydrationStatus {
    /// Hydrator emitted the action; dispatch has not yet returned.
    Pending,
    /// Dispatch returned `Ok(())` from `Dataplane::update_service`.
    Completed {
        /// Fingerprint of the `(vip, backends)` pair the dispatch
        /// successfully applied.
        fingerprint: BackendSetFingerprint,
        /// Wall-clock snapshot at evaluation start (`tick.now_unix`)
        /// — observation, not derived state, so the hydrator can
        /// compare against `actual.fingerprint` at the next tick
        /// without recomputing.
        applied_at: UnixInstant,
    },
    /// Dispatch returned `Err(DataplaneError::*)`. The hydrator
    /// reads this row at the next tick to decide whether to retry
    /// (per its retry-budget policy in the typed View — Slice
    /// 08-02).
    Failed {
        /// Fingerprint of the `(vip, backends)` pair the dispatch
        /// attempted to apply.
        fingerprint: BackendSetFingerprint,
        /// Wall-clock snapshot at evaluation start (`tick.now_unix`).
        failed_at: UnixInstant,
        /// `Display::to_string(&err)` of the underlying
        /// `DataplaneError`. Diagnostic-only; the hydrator does
        /// not branch on this string (typed retry-budget policy
        /// lives in the View per `.claude/rules/development.md`
        /// § "Persist inputs, not derived state").
        reason: String,
    },
}

/// `service_hydration_results` row — observation surface for the
/// `Action::DataplaneUpdateService` action shim per
/// architecture.md § 7 *Failure surface* and § 12 *Schema*.
///
/// Written by the action shim on dispatch completion (`Completed`
/// or `Failed`). The hydrator reconciler (Slice 08-02) reads this
/// row via [`ObservationStore::service_hydration_results_rows`]
/// projected into `actual` and either advances on
/// `Completed { fingerprint == desired.fingerprint }` or, on
/// `Failed`, applies its retry-budget policy from the typed View.
///
/// LWW key is `(service_id, fingerprint)` — content-hashed, so two
/// writes for the same `(service_id, fingerprint)` are strictly
/// idempotent under `LogicalTimestamp::dominates`.
///
/// # Schema evolution
///
/// Per ADR-0048 (`docs/product/architecture/adr-0048-rkyv-versioned-envelope.md`)
/// this type is the **inner payload** of
/// [`ServiceHydrationResultRowEnvelope`] under the UI-02 amendment
/// alias-to-payload public API. rkyv archives are **fixed positional
/// layouts** — appending a field to this struct shifts every
/// subsequent offset and renders pre-existing bytes unreadable.
/// Schema evolution at this boundary goes through a new envelope
/// variant (`V2`, `V3`, …) added per the version-bump procedure in
/// `.claude/rules/development.md` § "rkyv schema evolution"; existing
/// `FIXTURE_V<N>` golden bytes are NEVER touched.
///
/// The embedded [`ServiceHydrationStatus`] enum stays **unwrapped**
/// per ADR-0048 § 4 (additive variant additions on inner rkyv enums
/// are the documented exception — `ServiceHydrationStatus`'s STABLE
/// variant-ordering docstring is the structural commitment that
/// keeps the inner-enum exception load-bearing).
///
/// Writers go through [`ServiceHydrationResultRow::latest`]
/// (= [`ServiceHydrationResultRowEnvelope::latest`]); readers project
/// through [`ServiceHydrationResultRowEnvelope::into_latest`].
pub type ServiceHydrationResultRow = ServiceHydrationResultRowV1;

/// `service_backends` row — the desired backend set for a service,
/// written by the control plane when allocation status changes and
/// read by the `ServiceMapHydrator` reconciler to hydrate `desired`
/// state per architecture.md § 8 *Hydration shape*.
///
/// Keyed by [`ServiceId`] alone — one row per service carrying the
/// full current backend set. LWW resolution uses
/// [`LogicalTimestamp::dominates`] on `updated_at`.
///
/// Per §4 guardrail: full-row writes only, no field-diff merges.
///
/// # Schema evolution
///
/// Per ADR-0048 (`docs/product/architecture/adr-0048-rkyv-versioned-envelope.md`)
/// this type is the **inner payload** of [`ServiceBackendRowEnvelope`]
/// under the UI-02 amendment alias-to-payload public API. rkyv
/// archives are **fixed positional layouts** — appending a field to
/// this struct shifts every subsequent offset and renders pre-existing
/// bytes unreadable. Schema evolution at this boundary goes through a
/// new envelope variant (`V2`, `V3`, …) added per the version-bump
/// procedure in `.claude/rules/development.md` § "rkyv schema
/// evolution"; existing `FIXTURE_V<N>` golden bytes are NEVER touched.
///
/// Writers go through [`ServiceBackendRow::latest`]
/// (= [`ServiceBackendRowEnvelope::latest`]); readers project through
/// [`ServiceBackendRowEnvelope::into_latest`].
pub type ServiceBackendRow = ServiceBackendRowV1;

/// The closed set of row shapes [`ObservationStore`] accepts.
///
/// This enum *is* the compile-time boundary between intent and
/// observation: any type that is not a variant of [`ObservationRow`]
/// cannot be written into an [`ObservationStore`]. Phase 2+ extensions
/// add variants here as new row shapes are introduced (compiled policy
/// verdicts, revoked operator certs, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservationRow {
    AllocStatus(AllocStatusRow),
    NodeHealth(NodeHealthRow),
    /// `service_hydration_results` row — written by the action shim
    /// on `Action::DataplaneUpdateService` dispatch per
    /// `docs/feature/phase-2-xdp-service-map/design/architecture.md`
    /// §§ 7, 12. Read by the `ServiceMapHydrator` reconciler
    /// (Slice 08-02).
    ServiceHydration(ServiceHydrationResultRow),
    /// `service_backends` row — the desired backend set for a
    /// service. Read by the `ServiceMapHydrator` reconciler to
    /// hydrate `desired` state (GH #160).
    ServiceBackend(ServiceBackendRow),
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
// Versioned envelopes — RED scaffolds per ADR-0048
// ---------------------------------------------------------------------------
//
// Each per-type envelope wraps the existing row type as `V1`. Per the
// 01-01 scaffolding-step caveat (CLAUDE.md / step description), the
// legacy row types above remain in place — call-site migration lands
// in subsequent steps (01-03 for AllocStatusRow; 02-01..02-03 for the
// rest). Inner payload types are `pub(crate)` per ADR-0048 § 2
// Layer 1 (cross-crate writers cannot name the payload to construct
// it).

// SCAFFOLD: true
//
// ADR-0048 § 2 Layer 1 specifies `pub(crate)` on inner payload
// types. In practice, rustc E0446 rejects `pub(crate)` types
// referenced from a `pub` trait's `type Latest` impl — see
// `feat(rkyv-envelope)/01-01 surfacing note` in the step return
// message. Layer 1 is therefore enforced by **non-re-export from
// `overdrive_core::lib.rs`** (cross-crate writers must reach the
// type via the verbose `overdrive_core::traits::observation_store::*`
// path, which is discouraged by code review) PLUS Layer 2 (the
// `xtask::dst_lint` envelope-variant-construction scanner in
// Group 5). The structural defense is Layer 2.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum AllocStatusRowEnvelope {
    V1(AllocStatusRowV1),
}

pub type AllocStatusRowLatest = AllocStatusRowV1;

// SCAFFOLD: true — `pub` due to rustc E0446 in trait impl; Layer 1
// enforced by non-re-export from `lib.rs` + Layer 2 dst_lint scanner
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct AllocStatusRowV1 {
    pub alloc_id: AllocationId,
    pub workload_id: WorkloadId,
    pub node_id: NodeId,
    pub state: AllocState,
    pub updated_at: LogicalTimestamp,
    pub reason: Option<TransitionReason>,
    pub detail: Option<String>,
    pub terminal: Option<TerminalCondition>,
    pub stderr_tail: Option<String>,
    pub kind: WorkloadKind,
    pub listeners: Vec<ListenerRow>,
}

impl VersionedEnvelope for AllocStatusRowEnvelope {
    type Latest = AllocStatusRowV1;

    fn latest(payload: Self::Latest) -> Self {
        Self::V1(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1),
        }
    }

    /// Discriminant offset for `AllocStatusRowEnvelope` archives,
    /// measured from the END of the archive bytes.
    ///
    /// Empirically determined against canonical V1 payloads of
    /// varying `listeners: Vec<ListenerRow>` sizes: rkyv 0.8 places
    /// the outer enum's discriminant byte 168 bytes from the END of
    /// the archive, stable across all payload sizes (the trailing
    /// "root" structure has a fixed footprint; only the leading slab
    /// grows with variable-length data).
    ///
    /// Re-pin alongside the schema-evolution fixture at every
    /// version-bump per
    /// [`VersionedEnvelope::discriminant_offset_from_end`]'s
    /// docstring.
    fn discriminant_offset_from_end() -> Option<usize> {
        Some(168)
    }

    fn known_discriminants() -> &'static [u8] {
        // V1 carries rkyv discriminant 0 (declaration order — first
        // variant). Empirically verified by archiving a canonical
        // `AllocStatusRowEnvelope::latest(...)` and inspecting the
        // byte at `bytes.len() - 168`.
        &[0]
    }

    fn type_name() -> &'static str {
        "AllocStatusRowEnvelope"
    }
}

// SCAFFOLD: true
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum NodeHealthRowEnvelope {
    V1(NodeHealthRowV1),
}

pub type NodeHealthRowLatest = NodeHealthRowV1;

// SCAFFOLD: true — `pub` due to rustc E0446 in trait impl; Layer 1
// enforced by non-re-export from `lib.rs` + Layer 2 dst_lint scanner
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct NodeHealthRowV1 {
    pub node_id: NodeId,
    pub region: Region,
    pub last_heartbeat: LogicalTimestamp,
}

impl VersionedEnvelope for NodeHealthRowEnvelope {
    type Latest = NodeHealthRowV1;

    fn latest(payload: Self::Latest) -> Self {
        Self::V1(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1),
        }
    }

    /// Discriminant offset for `NodeHealthRowEnvelope` archives,
    /// measured from the END of the archive bytes.
    ///
    /// Empirically determined against canonical V1 payloads of
    /// varying NodeId / Region / writer-id lengths (including the
    /// inline-vs-out-of-line `ArchivedString` boundary at 8 bytes)
    /// and counter values from 1 to u64::MAX: rkyv 0.8 places the
    /// outer enum's discriminant byte 40 bytes from the END of the
    /// archive, stable across all payload sizes.
    ///
    /// The trailing 40 bytes encompass the root structure footprint:
    /// 1 byte discriminant + 7 padding + 8-byte counter + 16-byte
    /// `ArchivedString` (relptr+len OR inline) for the writer
    /// NodeId + 8-byte enum padding. (rkyv rounds the root region
    /// up to align to 8 bytes.)
    ///
    /// Re-pin alongside the schema-evolution fixture at every
    /// version-bump per
    /// [`VersionedEnvelope::discriminant_offset_from_end`]'s
    /// docstring.
    fn discriminant_offset_from_end() -> Option<usize> {
        Some(40)
    }

    fn known_discriminants() -> &'static [u8] {
        // V1 carries rkyv discriminant 0 (declaration order — first
        // variant). Empirically verified by archiving a canonical
        // `NodeHealthRowEnvelope::latest(...)` and inspecting the
        // byte at `bytes.len() - 40`.
        &[0]
    }

    fn type_name() -> &'static str {
        "NodeHealthRowEnvelope"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum ServiceHydrationResultRowEnvelope {
    V1(ServiceHydrationResultRowV1),
}

pub type ServiceHydrationResultRowLatest = ServiceHydrationResultRowV1;

// `pub` due to rustc E0446 in trait impl; Layer 1 enforced by
// non-re-export from `lib.rs` + Layer 2 dst_lint scanner.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ServiceHydrationResultRowV1 {
    /// Identity of the service whose backend set was being
    /// rewritten. Maps 1:1 to a `MAGLEV_MAP` outer-map key.
    pub service_id: ServiceId,
    /// Fingerprint of the `(vip, backends)` pair the dispatch
    /// applied (or attempted to apply on `Failed`). Forms the
    /// secondary key under LWW so distinct backend sets land in
    /// distinct rows.
    pub fingerprint: BackendSetFingerprint,
    /// Outcome of the dispatch attempt — see
    /// [`ServiceHydrationStatus`]. The embedded enum stays
    /// unwrapped per ADR-0048 § 4 (inner rkyv enum additive variant
    /// additions are the documented exception).
    pub status: ServiceHydrationStatus,
    /// Lamport timestamp of this row. Same shape as
    /// [`AllocStatusRow::updated_at`] — the action shim writes
    /// `(counter = tick.tick + 1, writer = node_id)` so two writes
    /// for the same `(service_id, fingerprint)` on different ticks
    /// are correctly ordered under LWW.
    pub updated_at: LogicalTimestamp,
}

impl VersionedEnvelope for ServiceHydrationResultRowEnvelope {
    type Latest = ServiceHydrationResultRowV1;

    fn latest(payload: Self::Latest) -> Self {
        Self::V1(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1),
        }
    }

    /// Discriminant offset for `ServiceHydrationResultRowEnvelope`
    /// archives, measured from the END of the archive bytes.
    ///
    /// Empirically determined against canonical V1 payloads of varying
    /// `ServiceHydrationStatus` variants (`Pending` / `Completed` /
    /// `Failed`), failure-reason string lengths (inline-vs-out-of-line
    /// `ArchivedString` boundary at 8 bytes), and `writer: NodeId`
    /// lengths: rkyv 0.8 places the outer enum's discriminant byte 80
    /// bytes from the END of the archive, stable across all payload
    /// sizes (the trailing "root" structure has a fixed 80-byte
    /// footprint — 1B discriminant + 7B pad + 8B service_id + 8B
    /// fingerprint + 24B status enum (1B inner-disc + 7B pad + 16B
    /// payload max) + 8B counter + 16B writer ArchivedString + 8B
    /// trailing alignment; only the leading slab — failure reason
    /// strings and long writer NodeId payloads — grows with
    /// variable-length data).
    ///
    /// Re-pin alongside the schema-evolution fixture at every
    /// version-bump per
    /// [`VersionedEnvelope::discriminant_offset_from_end`]'s
    /// docstring.
    fn discriminant_offset_from_end() -> Option<usize> {
        Some(80)
    }

    fn known_discriminants() -> &'static [u8] {
        // V1 carries rkyv discriminant 0 (declaration order — first
        // variant). Empirically verified by archiving a canonical
        // `ServiceHydrationResultRowEnvelope::latest(...)` and
        // inspecting the byte at `bytes.len() - 80`.
        &[0]
    }

    fn type_name() -> &'static str {
        "ServiceHydrationResultRowEnvelope"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum ServiceBackendRowEnvelope {
    V1(ServiceBackendRowV1),
}

pub type ServiceBackendRowLatest = ServiceBackendRowV1;

// `pub` due to rustc E0446 in trait impl; Layer 1 enforced by
// non-re-export from `lib.rs` + Layer 2 dst_lint scanner.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ServiceBackendRowV1 {
    /// Identity of the service. Primary key for LWW.
    pub service_id: ServiceId,
    /// Virtual IP for the service — wire-shape `Ipv4Addr`, not
    /// `ServiceVip`. The hydrator wraps into `ServiceVip` at the
    /// read boundary (architecture.md § 8 lines 616-629).
    pub vip: Ipv4Addr,
    /// Current backend set for the service.
    pub backends: Vec<Backend>,
    /// Lamport timestamp for LWW ordering.
    pub updated_at: LogicalTimestamp,
}

impl VersionedEnvelope for ServiceBackendRowEnvelope {
    type Latest = ServiceBackendRowV1;

    fn latest(payload: Self::Latest) -> Self {
        Self::V1(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1),
        }
    }

    /// Discriminant offset for `ServiceBackendRowEnvelope` archives,
    /// measured from the END of the archive bytes.
    ///
    /// Empirically determined via the byte-flip locator
    /// `schema_evolution::service_backend_row::locate_service_backend_discriminant_offset_via_byte_flip`
    /// (run with `--run-ignored only --no-capture`): flipping the
    /// byte at `bytes.len() - 48` to a non-zero discriminant fires
    /// `bytecheck` with `invalid discriminant '153' for enum
    /// 'ArchivedServiceBackendRowEnvelope'`. The trailing 48 bytes
    /// encompass the V1 payload root: 8B service_id + 4B vip + 4B
    /// alignment + 8B backends RelVec + 8B counter + 16B writer
    /// ArchivedString (the alloc/addr strings of every backend live
    /// in the leading slab, not the trailing root).
    ///
    /// Re-pin alongside the schema-evolution fixture at every
    /// version-bump per
    /// [`VersionedEnvelope::discriminant_offset_from_end`]'s
    /// docstring.
    fn discriminant_offset_from_end() -> Option<usize> {
        Some(48)
    }

    fn known_discriminants() -> &'static [u8] {
        // V1 carries rkyv discriminant 0 (declaration order — first
        // variant). Empirically verified by archiving a canonical
        // `ServiceBackendRowEnvelope::latest(...)` and inspecting the
        // byte at `bytes.len() - 48`.
        &[0]
    }

    fn type_name() -> &'static str {
        "ServiceBackendRowEnvelope"
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
    /// to recover the prior `(workload_id, node_id, updated_at)` tuple
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

    /// Read every LWW-winner `service_hydration_results` row for the
    /// given [`ServiceId`]. Used by the `ServiceMapHydrator` reconciler
    /// (Slice 08-02) to project the observation surface into `actual`
    /// and detect convergence on `Completed { fingerprint ==
    /// desired.fingerprint }`.
    ///
    /// Iteration order is deterministic — keyed by `(service_id,
    /// fingerprint)` under the adapter's storage shape (e.g.
    /// [`std::collections::BTreeMap`]). One row may exist per
    /// `(service_id, fingerprint)`; the same `service_id` with a
    /// different `fingerprint` lives in a distinct row (the secondary
    /// key is the content-hashed fingerprint per architecture.md § 12).
    ///
    /// Per architecture.md § 12 the table is single-writer (the
    /// action shim) and additive-only — a Phase 2 Corrosion-backed
    /// implementation gossips rows under the same LWW semantics as
    /// `alloc_status` and `node_health`.
    async fn service_hydration_results_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ServiceHydrationResultRow>, ObservationStoreError>;

    /// Read the LWW-winner `service_backends` rows for the given
    /// [`ServiceId`]. Used by the `ServiceMapHydrator` reconciler
    /// (GH #160) to hydrate `desired` state from observation.
    ///
    /// Returns at most one row per `ServiceId` — the table is keyed
    /// by `ServiceId` alone (not a composite key). LWW resolution
    /// uses [`LogicalTimestamp::dominates`] on `updated_at`.
    async fn service_backends_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ServiceBackendRow>, ObservationStoreError>;
}
