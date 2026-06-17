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
use crate::ca::issued_certificate_row::IssuedCertificateRow;
use crate::codec::{EnvelopeError, VersionedEnvelope};
use crate::dataplane::backend_key::Proto;
use crate::dataplane::fingerprint::BackendSetFingerprint;
use crate::id::{
    AllocationId, CorrelationKey, IssuanceOrdinal, NodeId, Region, ServiceId, WorkloadId,
};
use crate::observation::ProbeResultRow;
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
///
/// # Why `AllocStatus` carries `Box<AllocStatusRow>`
///
/// [`AllocStatusRow`] grew past the `large_enum_variant` clippy
/// threshold (~304 bytes) when `TerminalCondition::{Stable,
/// ServiceFailed}` were appended on 2026-05-24 per ADR-0055 /
/// ADR-0056 — the inline `Option<TerminalCondition>` footprint is
/// dominated by `Stable { settled_in_ms: u64, witness: ProbeWitness }`.
/// Leaving the variant unboxed would 6× the memory cost of every
/// other variant in the enum (`NodeHealth`, `ServiceHydration`,
/// `ServiceBackend`) because the enum's discriminant + slot is sized
/// to its largest variant.
///
/// The `Box` is a private implementation detail — public callers
/// continue to construct `ObservationRow::AllocStatus(Box::new(row))`
/// and destructuring readers see `&AllocStatusRow` via auto-deref.
/// `ObservationStore::write` still takes `ObservationRow` by value;
/// the boxing happens at construction sites only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservationRow {
    AllocStatus(Box<AllocStatusRow>),
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
    /// `reconcile_conflict` row — written by `run_convergence_tick` on
    /// a genuine same-slot reconcile-output invariant violation (Fix C,
    /// RCA `fix-mixed-backend-dispatch-spin`). The machine-queryable
    /// control signal that pairs with the supplemental
    /// `reconciler.output.invariant_violation` tracing event (the
    /// Kubernetes Events model: best-effort human signal distinct from
    /// the durable observation row). Read via
    /// [`ObservationStore::reconcile_conflict_rows`].
    ReconcileConflict(ReconcileConflictRow),
    /// `issued_certificates` audit row (table `issued_certificates`,
    /// ADR-0063 D6) — the record of *what was issued*. Written through
    /// [`ObservationStore::write`] by the CA issuance seam on every
    /// workload-SVID mint, bound so an audit-write failure refuses the
    /// issuance (no silent issuance, US-CA-05). Read back via
    /// [`ObservationStore::issued_certificate_rows`].
    ///
    /// The CA *material* (root/intermediate keys) is intent (ADR-0063 D2);
    /// the *record of an issuance* is OBSERVATION (D6) — this variant
    /// mirrors the `alloc_status` / `node_health` observation-row plumbing
    /// exactly. Single-node-local today; gossiped (like every observation
    /// row) when GH #36 lands. The payload's rkyv versioned envelope and
    /// co-located typed codec ([`IssuedCertificateRow::archive_for_store`] /
    /// [`IssuedCertificateRow::from_store_bytes`]) are reused as-is per
    /// ADR-0048.
    IssuedCertificate(IssuedCertificateRow),
    /// `workflow_terminal` row — the durable terminal surface for a
    /// workflow instance per ADR-0064 §2. Written by the
    /// [`WorkflowEngine`](../../../overdrive_control_plane/workflow_runtime/struct.WorkflowEngine.html)
    /// via the sanctioned [`ObservationStore::write`] path when the
    /// author's `async fn run` returns its terminal — the engine projects
    /// the body's `Result<Output, TerminalError>` to a
    /// [`WorkflowStatus`](crate::workflow::WorkflowStatus) (ADR-0065 §3) —
    /// NOT a direct engine write bypassing the channels (slice-01 AC5).
    ///
    /// Keyed by `correlation` (the instance [`CorrelationKey`]) so the
    /// emitting workflow-lifecycle reconciler finds the status
    /// deterministically on the next tick and converges the instance to
    /// terminated. The `Action::StartWorkflow { start, correlation }` the
    /// reconciler emits carries the SAME key the terminal row is filed
    /// under (`development.md` Reconciler I/O rule 2 — correlation, not
    /// request ID, links cause to response).
    WorkflowTerminal {
        /// Cause-to-response linkage — the instance correlation key.
        correlation: CorrelationKey,
        /// The workflow instance's terminal status (the engine-owned
        /// projection of the body's `Result<Output, TerminalError>`).
        status: crate::workflow::WorkflowStatus,
    },
    /// `workflow_signal` row — the typed cross-workflow signal surface a
    /// `ctx.wait_for_signal(key)` blocks on per ADR-0064 §4. A producer
    /// satisfies a blocked wait by writing this row keyed by the same
    /// [`SignalKey`](crate::workflow::SignalKey); the engine's live
    /// `ctx.wait_for_signal` path reads it via
    /// [`ObservationStore::workflow_signal`].
    ///
    /// In-process single-node delivery (#207 cross-node-under-partition
    /// is OUT). Keyed by `key` alone — one current value per key; a
    /// re-write replaces it (the value is opaque to the primitive). This
    /// is the absent-vs-present surface that makes the blocking honest:
    /// before this row exists, `ctx.wait_for_signal(key)` stays pending.
    Signal {
        /// The signal key the row satisfies — the same key a
        /// `ctx.wait_for_signal(key)` blocks on.
        key: crate::workflow::SignalKey,
        /// The opaque payload the blocked wait receives verbatim.
        value: crate::workflow::SignalValue,
    },
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
    /// Wall-clock instant at which this allocation first transitioned
    /// Pending → Running, as observed by the owning node via the
    /// injected [`crate::traits::clock::Clock`] port.
    ///
    /// Typed [`UnixInstant`] — the unit (since-UNIX-epoch) and origin
    /// are encoded in the type itself, per the canonical wall-clock
    /// newtype defined in [`crate::wall_clock`].
    ///
    /// # Semantics
    ///
    /// - `None`: the allocation has not yet been observed Running
    ///   (Pending only, or driver-rejected start). This is the honest
    ///   shape — no Running observation exists yet, so there is no
    ///   wall-clock to record.
    /// - `Some(ts)`: the allocation reached Running at wall-clock
    ///   `ts`. The value is captured ONCE at the Pending → Running
    ///   transition from `tick.now_unix` — the same `Clock` port DST
    ///   already controls (`SystemClock` in production, `SimClock`
    ///   under simulation).
    ///
    /// Once `Some(ts)`, the field is preserved verbatim on every
    /// subsequent state transition (Failed, Stopped, Terminated) by
    /// reading the LWW-winner prior row and copying the field forward.
    /// This is the load-bearing invariant that lets the
    /// `ServiceLifecycleReconciler`'s deadline gates (Stable
    /// `settled_in_ms`, EarlyExit `elapsed < startup_deadline`,
    /// StartupProbeFailed `elapsed >= deadline_ms`) read a stable
    /// "started at" wall-clock from any post-Running row.
    ///
    /// # Discipline
    ///
    /// Per `.claude/rules/development.md` § "Persist inputs, not
    /// derived state": this field is the INPUT (the observed
    /// wall-clock of the Pending → Running transition). Derived
    /// values — `elapsed` (computed as `tick.now_unix` minus
    /// `started_at`), `settled_in_ms`, and deadline gates —
    /// are recomputed on every reconcile tick from this input plus
    /// the live policy (deadline thresholds in the spec). The field
    /// is never a cache of today's policy.
    ///
    /// Per `.claude/rules/development.md` § "Distinct failure modes
    /// get distinct error variants": consumers MUST match on
    /// `Some(ts)` explicitly. Do NOT collapse to an unwrap-to-zero —
    /// that re-creates the silent-zero bug this field exists to
    /// avoid (elapsed becomes huge → EarlyExit never fires,
    /// StartupProbeFailed always fires). `None` is the explicit
    /// "no Running observation yet" signal; reconcilers branch on
    /// it rather than treating it as zero.
    pub started_at: Option<UnixInstant>,
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
    /// varying `listeners: Vec<ListenerRow>` / `detail` / `stderr_tail`
    /// / `terminal` shapes: rkyv 0.8 places the outer enum's
    /// discriminant byte at a fixed offset from the END of the
    /// archive, stable across all payload sizes (the trailing "root"
    /// structure has a fixed footprint; only the leading slab grows
    /// with variable-length data).
    ///
    /// **Repinned 2026-05-24** (greenfield, no shipped consumers; per
    /// the `feedback_single_cut_greenfield_migrations` rule): the
    /// previously-pinned offset of 168 reflected the canonical V1
    /// layout before `TerminalCondition::Stable` and `::ServiceFailed`
    /// were appended. Appending those two variants grew the inline
    /// footprint of `Option<TerminalCondition>` (which `AllocStatusRowV1`
    /// embeds inline), which extended the trailing root structure by
    /// 24 bytes. The new offset of 192 was empirically located by
    /// flipping each stable-zero byte in canonical archives to 0xFE
    /// and observing which one caused rkyv to reject the archive with
    /// `invalid discriminant '254' for enum
    /// 'ArchivedAllocStatusRowEnvelope'`.
    ///
    /// **Repinned 2026-05-29** (greenfield extension per the
    /// subsidiary GAP-1 fix): added an `Option<UnixInstant>`
    /// `started_at` field inline to `AllocStatusRowV1`. `UnixInstant`
    /// wraps `Duration` (12 bytes — 8 for seconds + 4 for nanos),
    /// inlined behind the `Option` discriminant; this extends the
    /// trailing root structure beyond the prior 192-byte pin. The
    /// new offset is determined empirically by the schema-evolution
    /// fixture's triangulation test (`alloc_status_row_discriminant
    /// _offset_triangulation`); update this constant and
    /// `GOLDEN_DISCRIMINANT_OFFSET_V1` in lockstep at every variant
    /// or layout change.
    ///
    /// Re-pin alongside the schema-evolution fixture at every
    /// version-bump per
    /// [`VersionedEnvelope::discriminant_offset_from_end`]'s
    /// docstring.
    fn discriminant_offset_from_end() -> Option<usize> {
        Some(212)
    }

    fn known_discriminants() -> &'static [u8] {
        // V1 carries rkyv discriminant 0 (declaration order — first
        // variant). Empirically verified by archiving a canonical
        // `AllocStatusRowEnvelope::latest(...)` and inspecting the
        // byte at `bytes.len() - 208`.
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

/// Route a conflicting reconcile-output write would take through the
/// dataplane port boundary — the core-side mirror of
/// `overdrive_control_plane::action_shim::validate::WriteRoute`.
///
/// `WriteRoute` lives in `overdrive-control-plane` (the dispatch
/// boundary) and is NOT reachable from `overdrive-core` — and an
/// `overdrive-core → overdrive-control-plane` dependency would invert
/// the crate-class layering (core depends only on the trait surface).
/// This enum is the minimal core-side representation the observation
/// row needs to record *which two routes* a genuine same-slot conflict
/// spanned. It is pure data: two unit variants, no behaviour.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub enum ConflictRoute {
    /// XDP `SERVICE_MAP` / Maglev path, keyed on `(vip, port, proto)`.
    /// Mirrors `WriteRoute::Xdp`.
    Xdp,
    /// cgroup `connect4` rewrite path (`LOCAL_BACKEND_MAP`), keyed on
    /// `(vip, vip_port, proto)`. Mirrors `WriteRoute::Cgroup`.
    Cgroup,
}

#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum ReconcileConflictRowEnvelope {
    V1(ReconcileConflictRowV1),
}

/// Public payload alias for the `reconcile_conflict` observation row,
/// per the UI-02 alias-to-payload amendment (mirrors
/// [`ServiceHydrationResultRow`]). Callers construct
/// `ReconcileConflictRow { .. }` directly; the persistence boundary
/// wraps via [`ReconcileConflictRowEnvelope::latest`] and projects via
/// [`ReconcileConflictRowEnvelope::into_latest`].
pub type ReconcileConflictRow = ReconcileConflictRowV1;

/// Documentation alias for "the latest payload shape" of the
/// `reconcile_conflict` row. Re-aliased alongside
/// [`ReconcileConflictRow`] at every schema version bump.
pub type ReconcileConflictRowLatest = ReconcileConflictRowV1;

// `pub` due to rustc E0446 in trait impl; Layer 1 enforced by
// non-re-export from `lib.rs` + Layer 2 dst_lint scanner.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ReconcileConflictRowV1 {
    /// Identity of the service whose `reconcile()` output carried the
    /// genuine same-slot conflict. The reconciler identity operators
    /// grep for. Primary key for LWW alongside the conflicting slot.
    pub service_id: ServiceId,
    /// Virtual IP of the conflicting `(vip, port, proto)` map slot.
    /// Wire-shape `Ipv4Addr` (same precedent as
    /// [`ServiceBackendRowV1::vip`]); the conflict is observed in
    /// wire-shape terms at the dispatch boundary.
    pub vip: Ipv4Addr,
    /// Port of the conflicting map slot.
    pub port: u16,
    /// L4 protocol of the conflicting map slot. `(vip, port, proto)`
    /// together name the single slot the two writes collided on.
    pub proto: Proto,
    /// Route the FIRST emitted conflicting write took. After ADR-0053
    /// revision 2026-06-03 the only surviving conflict class is
    /// same-route same-slot, so `first_route == second_route` today;
    /// both are recorded as OBSERVED INPUTS (not a derived
    /// classification) so a future cross-class conflict can populate
    /// them distinctly without a schema change.
    pub first_route: ConflictRoute,
    /// Route the SECOND (conflicting) emitted write took.
    pub second_route: ConflictRoute,
    /// Lamport timestamp of this row. Same shape as
    /// [`ServiceHydrationResultRowV1::updated_at`] — the convergence
    /// tick writes `(counter = tick.tick + 1, writer = node_id)` so two
    /// conflict observations for the same `(service_id, vip, port,
    /// proto)` on different ticks are correctly ordered under LWW.
    pub updated_at: LogicalTimestamp,
}

impl VersionedEnvelope for ReconcileConflictRowEnvelope {
    type Latest = ReconcileConflictRowV1;

    fn latest(payload: Self::Latest) -> Self {
        Self::V1(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1),
        }
    }

    /// Discriminant offset for `ReconcileConflictRowEnvelope` archives,
    /// measured from the END of the archive bytes.
    ///
    /// Empirically determined via the byte-flip locator in
    /// `tests/schema_evolution/reconcile_conflict_row.rs` (the
    /// `#[ignore]`d `print_fixture_v1_bytes` regeneration aid + the
    /// triangulation test confirm the value). The trailing root region
    /// of the V1 payload is fixed-size — 8B service_id + 4B vip + 2B
    /// port + 1B proto + 1B first_route + 1B second_route + padding +
    /// 8B counter + 16B writer `ArchivedString` (the writer NodeId's
    /// variable-length bytes live in the leading slab, not the trailing
    /// root) + trailing 8-byte alignment.
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
        // `ReconcileConflictRowEnvelope::latest(...)` and inspecting the
        // byte at `bytes.len() - discriminant_offset_from_end()`.
        &[0]
    }

    fn type_name() -> &'static str {
        "ReconcileConflictRowEnvelope"
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
    /// # Append-only contract (`issued_certificates`)
    ///
    /// [`ObservationRow::IssuedCertificate`] is the one variant NOT
    /// governed by the LWW contract above — it has no `updated_at`. Its
    /// table is **append-only**, keyed by serial: the first row written
    /// at a given serial is immutable. A second write at an
    /// already-present serial MUST NOT overwrite the stored row and MUST
    /// NOT be emitted on subscriptions (a serial already broadcast is
    /// never re-broadcast). A duplicate is an issuance replay / retry, or
    /// — once `issued_certificates` is gossiped (GH #36) — the idempotent
    /// re-delivery every other row tolerates; in all cases the original
    /// row is kept. This mirrors the LWW-reject path's no-mutate /
    /// no-emit shape with the serial key, rather than `updated_at`, as
    /// the immutability boundary.
    ///
    /// This contract is exercised by the trait-conformance harness at
    /// `overdrive_core::testing::observation_store::run_lww_conformance`
    /// (LWW cases, plus the append-only case for `issued_certificates`).
    /// The two adapter implementations in this workspace —
    /// `SimObservationStore` and `LocalObservationStore` — honour the
    /// contract. Future adapters (Phase 2 Corrosion replacement, any
    /// future test fakes) MUST honour it identically.
    ///
    /// See `docs/whitepaper.md` §4 (Intent / Observation split,
    /// "tombstones, full rows over field diffs"),
    /// `docs/feature/fix-observation-lww-merge/deliver/rca.md` for the
    /// bug RCA that codified the LWW invariant, and
    /// `docs/feature/fix-issued-cert-append-only/deliver/rca.md` for the
    /// append-only one.
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

    /// Read every `issued_certificates` audit row this peer holds
    /// (table `issued_certificates`, ADR-0063 D6) — the
    /// operator-observable issuance audit surface (the internal-CT
    /// equivalent), readable the same way the REST observation-read
    /// path reads `alloc_status`.
    ///
    /// The table is **append-only** — keyed by the issued certificate's
    /// serial bytes, so each distinct issuance is its own row (serials
    /// are CSPRNG-drawn; a key collision is the issuance bug, not an
    /// LWW case). Unlike the LWW siblings there is no "winner" to
    /// resolve; every recorded issuance is returned.
    ///
    /// # Postconditions
    ///
    /// Iteration order is deterministic — ascending by serial-key bytes
    /// under the adapter's storage shape. A row whose envelope bytes
    /// fail to decode (corrupt / unknown future version) is skipped with
    /// a `tracing::warn!` per ADR-0048 § 3 (observation log-and-skip;
    /// asymmetric vs intent fail-fast) and the surviving rows are
    /// returned — convergence proceeds.
    ///
    /// # Errors
    ///
    /// [`ObservationStoreError::Io`] on a backing-store read failure. A
    /// per-row envelope-decode failure does NOT error — it is logged and
    /// the row skipped.
    async fn issued_certificate_rows(
        &self,
    ) -> Result<Vec<IssuedCertificateRow>, ObservationStoreError>;

    /// Atomically allocate the next global issuance ordinal from a durable,
    /// strictly-monotonic counter, and return it.
    ///
    /// This is the SSOT for the `IssuanceOrdinal` stamped on every
    /// `issued_certificates` audit row (ADR-0063 D6, rev 8). It REPLACES the
    /// former `issued_certificate_rows().await?.len()` derivation
    /// (`ca_issuance.rs`, pre-rev-8), which was a check-then-act TOCTOU
    /// (`.claude/rules/development.md` § "Check-and-act must be atomic"): two
    /// concurrent issuances read the same `len()` and stamped DUPLICATE
    /// ordinals. Drawing from an atomically-incremented durable counter makes
    /// that collision UNREPRESENTABLE — two concurrent callers receive two
    /// distinct, strictly-increasing values.
    ///
    /// # Preconditions
    ///
    /// None. The method is safe to call concurrently for the same store from
    /// any number of callers — that is the whole point. It requires no
    /// serialization, no single-writer discipline, and no append-only
    /// invariant on any other table.
    ///
    /// # Postconditions
    ///
    /// On `Ok(n)`:
    /// * `n` is strictly greater than every ordinal this method has previously
    ///   returned for this store, INCLUDING across process restart (the counter
    ///   is durable). The first call on a fresh store returns the initial
    ///   value (see "Edge cases").
    /// * The counter has been durably advanced past `n` BEFORE this method
    ///   returns `Ok(n)` — a crash immediately after return cannot re-issue
    ///   `n` to a later caller.
    /// * No `issued_certificates` row is read or written by this call. The
    ///   ordinal is allocated independently of the audit table's contents; it
    ///   is NOT a function of `issued_certificate_rows().len()`.
    ///
    /// # Edge cases
    ///
    /// * **Fresh store.** The first call on a never-before-allocated store
    ///   returns the initial ordinal value. The initial value is pinned at
    ///   ordinal `0` (via [`IssuanceOrdinal::new`]) to match the pre-fix
    ///   `len()`-of-empty-table semantics (an empty audit log derived ordinal
    ///   0 for its first row); this keeps existing consumers' "ordinals start
    ///   at 0" expectation and the golden-bytes V1 fixtures valid. Greenfield
    ///   single-cut migration (`feedback_single_cut_greenfield_migrations.md`):
    ///   a fresh store starts the counter at 0; "delete the redb file" is the
    ///   upgrade path. No migration code reconstructs a counter from a pre-fix
    ///   store.
    /// * **Allocated-but-unused ordinal (gap semantics).** This method commits
    ///   the counter advance unconditionally on `Ok`. If the caller's
    ///   subsequent mint or audit-row write fails, the allocated ordinal is
    ///   BURNED — the sequence is left with a hole. This is by design and
    ///   CORRECT: the consumer projection
    ///   (`handlers::issued_certificates_for_rows`) maxes over the ordinal for
    ///   strict ORDERING, not density. A burned ordinal never collides and
    ///   never re-issues. Callers MUST NOT attempt to "return" or "reclaim" an
    ///   allocated-but-unused ordinal.
    /// * **Counter saturation.** `u64` ordinals do not realistically saturate
    ///   (2^64 issuances). Adapters do NOT special-case overflow; a wrapping or
    ///   saturating add is unnecessary, and `u64::MAX` issuances is out of any
    ///   operational envelope.
    ///
    /// # Observable invariants
    ///
    /// * **Monotonic-and-unique across concurrency.** For any interleaving of N
    ///   concurrent calls, the N returned values are N distinct, totally-ordered
    ///   `IssuanceOrdinal`s. No two calls — ever, on the same store — return the
    ///   same value.
    /// * **Durable across restart.** The value returned after a restart is
    ///   strictly greater than every value returned before the restart. The
    ///   counter survives process death (host: persisted; sim: in-memory for
    ///   the sim process lifetime — sufficient, as DST does not restart the
    ///   sim process mid-run).
    /// * **Independent of the audit table.** Deleting, compacting, or GCing
    ///   `issued_certificates` rows does NOT rewind the counter (this is the
    ///   #226 delete-survival property). The counter is a separate durable
    ///   datum.
    ///
    /// # Errors
    ///
    /// [`ObservationStoreError::Io`] on a backing-store read/write/commit
    /// failure while allocating. On error, NO ordinal is allocated and the
    /// counter is unchanged (the advance is committed atomically or not at all).
    async fn next_issuance_ordinal(&self) -> Result<IssuanceOrdinal, ObservationStoreError>;

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

    /// Read a deterministic snapshot of EVERY LWW-winner `service_backends`
    /// row this peer currently holds — across ALL services, with NO
    /// `ServiceId` key. This is the keyless enumerate sibling of the
    /// `ServiceId`-keyed [`Self::service_backends_rows`]; it is symmetric
    /// with the existing unkeyed enumerators [`Self::alloc_status_rows`] and
    /// [`Self::node_health_rows`] (no-arg, returns the full LWW-winner set),
    /// and follows the same `*_rows()`-no-arg naming convention with an
    /// `all_`-prefix to avoid colliding with the keyed method.
    ///
    /// # Motivation (the List leg of List-then-Watch)
    ///
    /// The transparent-mTLS `ServiceBackendsResolve` adapter (ADR-0071,
    /// C4 / D-TME-11) resolves an arbitrary `orig_dst: SocketAddrV4` against
    /// an in-RAM `addr → Backend` reverse index. It holds NO `ServiceId`, so
    /// the keyed [`Self::service_backends_rows`] is the wrong surface for a
    /// boot-time snapshot. This method is the **List** leg of its
    /// List-then-Watch: at `probe()` it bulk-loads the current snapshot into
    /// the index BEFORE the Earned-Trust gate opens (so the index is never
    /// empty-but-trusted), and on a watch-loss (`broadcast::RecvError::Lagged`)
    /// it re-Lists the authoritative snapshot. This is the same List-before-
    /// Watch shape the reconciler runtime's `bulk_load` already uses.
    ///
    /// # Preconditions
    /// None.
    ///
    /// # Postconditions on `Ok(rows)`
    /// `rows` is every `service_backends` row this peer currently holds as
    /// LWW winner — at most one row per `ServiceId` (the table is keyed by
    /// `ServiceId` alone). For each returned row, calling
    /// [`Self::service_backends_rows`] with that row's `service_id` returns
    /// the same row. The two surfaces are coherent: this method is the union
    /// of the keyed reads over every present service.
    ///
    /// # Edge cases
    /// - **Empty store** → `Ok(vec![])` (no services have written backends).
    /// - A row whose envelope bytes fail to decode (corrupt / unknown future
    ///   version) is skipped with a structured `tracing::warn!` per ADR-0048
    ///   § 3 (observation log-and-skip; asymmetric vs intent fail-fast) and
    ///   the surviving rows are returned — convergence proceeds.
    ///
    /// # Observable invariants
    /// Iteration order is deterministic — ascending by `ServiceId` under the
    /// adapter's storage shape (a [`std::collections::BTreeMap`] for the sim
    /// adapter; ascending key bytes for the redb-backed local adapter), so
    /// repeated calls against unchanged state return byte-identical ordered
    /// snapshots (K3 reproducibility). The method is read-only: it mutates no
    /// store state.
    ///
    /// # Errors
    /// [`ObservationStoreError::Io`] on a backing-store read failure. A
    /// per-row envelope-decode failure does NOT error — it is logged and the
    /// row skipped.
    async fn all_service_backends_rows(
        &self,
    ) -> Result<Vec<ServiceBackendRow>, ObservationStoreError>;

    /// Read every LWW-winner `reconcile_conflict` row for the given
    /// [`ServiceId`]. The machine-queryable surface for genuine
    /// same-slot reconcile-output conflicts written by
    /// `run_convergence_tick` (Fix C, RCA
    /// `fix-mixed-backend-dispatch-spin`). Operators query this instead
    /// of grepping the supplemental
    /// `reconciler.output.invariant_violation` tracing event.
    ///
    /// Iteration order is deterministic — keyed by `(service_id, vip,
    /// port, proto)` under the adapter's storage shape (e.g.
    /// [`std::collections::BTreeMap`]). One row may exist per
    /// `(service_id, vip, port, proto)` slot; the same `service_id` with
    /// a different slot lives in a distinct row. LWW resolution uses
    /// [`LogicalTimestamp::dominates`] on `updated_at`, mirroring the
    /// `service_hydration_results` reader.
    async fn reconcile_conflict_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ReconcileConflictRow>, ObservationStoreError>;

    /// Write a single [`ProbeResultRow`] — the latest observed
    /// outcome of one probe for one allocation.
    ///
    /// # Preconditions
    /// - `row.alloc_id` identifies a live or terminated allocation
    ///   this peer observes. The store does not validate referential
    ///   integrity against the intent surface; an alloc that never
    ///   existed produces an unreachable orphan row (acceptable —
    ///   readers filter by alloc_id at read time).
    /// - `row.probe_idx` matches the spec's 0-indexed probe position
    ///   at the time of observation. The probe-runner is the only
    ///   sanctioned writer; it never invents indices.
    ///
    /// # Postconditions
    /// - On `Ok(())`, the row is durable: a subsequent
    ///   [`Self::list_probe_results_for_alloc`] call MUST include the
    ///   written row IFF its `last_observed_at_unix_ms` dominates the
    ///   prior row at the composite primary key
    ///   `(alloc_id, probe_idx)`.
    /// - LWW resolution on `last_observed_at_unix_ms`: a write whose
    ///   timestamp does NOT strictly exceed the existing row's
    ///   timestamp at the same `(alloc_id, probe_idx)` MUST NOT
    ///   mutate state.
    /// - The probe-result surface is per-alloc-per-probe LATEST only;
    ///   per-tick history is NOT persisted. Per
    ///   `.claude/rules/development.md` § "Persist inputs, not derived
    ///   state" the renderer recomputes history from the row + the
    ///   live spec policy.
    ///
    /// # Edge cases
    /// - Concurrent writes for the same `(alloc_id, probe_idx)` with
    ///   identical `last_observed_at_unix_ms`: the second write loses
    ///   (strict-dominate rule). Idempotent.
    /// - `row.status == ProbeStatus::Fail { reason }` where `reason`
    ///   is empty: accepted as-is. The trait does not validate the
    ///   reason string shape — the renderer handles empty strings.
    ///
    /// # Observable invariants
    /// - For any `(alloc_id, probe_idx)`,
    ///   `list_probe_results_for_alloc(alloc_id)` returns exactly one
    ///   row per distinct `probe_idx` — the LWW winner.
    /// - `write_probe_result` is the ONLY mutator of the probe-result
    ///   table; no other trait method writes probe rows.
    async fn write_probe_result(&self, row: ProbeResultRow) -> Result<(), ObservationStoreError>;

    /// List every LWW-winner [`ProbeResultRow`] for the given
    /// allocation, one row per distinct `probe_idx`.
    ///
    /// # Preconditions
    /// - `alloc_id` is well-formed. The store does not validate
    ///   existence; an allocation that has never been observed
    ///   returns `Ok(vec![])`.
    ///
    /// # Postconditions
    /// - Iteration order is deterministic — sorted ascending by
    ///   `probe_idx` (per
    ///   `.claude/rules/development.md` § "Ordered-collection choice"
    ///   — `BTreeMap`-shape semantics on the underlying store).
    /// - At most one row per distinct `probe_idx` is returned (the
    ///   LWW winner; see [`Self::write_probe_result`]).
    /// - The returned rows have `row.alloc_id == *alloc_id` for every
    ///   element. The store filters at read time; the caller does
    ///   not need to re-filter.
    ///
    /// # Edge cases
    /// - No rows for `alloc_id`: returns `Ok(vec![])`, not an error.
    /// - Malformed envelope bytes for one row: the row is skipped
    ///   with a `tracing::warn!` event per ADR-0048 § 3
    ///   observation-layer policy (log + skip). Convergence proceeds
    ///   on surviving rows.
    ///
    /// # Observable invariants
    /// - Calling this method has no side effects beyond an optional
    ///   `tracing::warn!` on envelope-decode failure.
    /// - The result reflects every write that happens-before this
    ///   call — single-node read-after-write is strict.
    async fn list_probe_results_for_alloc(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Vec<ProbeResultRow>, ObservationStoreError>;

    /// List every observed `workflow_terminal` row — the durable
    /// terminal surface for workflow instances per ADR-0064 §2. Each row
    /// is keyed by the instance [`CorrelationKey`] the
    /// [`WorkflowEngine`](../../../overdrive_control_plane/workflow_runtime/struct.WorkflowEngine.html)
    /// wrote it under on `run` terminal; the workflow-lifecycle
    /// reconciler's `hydrate_actual` reads this to mark an instance
    /// converged (terminated).
    ///
    /// # Preconditions
    /// - None. An instance that has not yet reached terminal contributes
    ///   no row; the result simply omits it.
    ///
    /// # Postconditions
    /// - One `(correlation, result)` pair per distinct instance
    ///   correlation that has reached terminal. The terminal write is
    ///   idempotent on the stable correlation key (a crash-resume
    ///   re-write of the same instance re-files under the same key), so
    ///   at most one row per correlation is observable.
    /// - Iteration order is deterministic across runs — the underlying
    ///   store iterates by a stable key ordering
    ///   (`.claude/rules/development.md` § "Ordered-collection choice").
    ///
    /// # Edge cases
    /// - No terminal rows observed: returns `Ok(vec![])`, not an error.
    ///
    /// # Observable invariants
    /// - Calling this method has no side effects.
    /// - The result reflects every `WorkflowTerminal` write that
    ///   happens-before this call (single-node read-after-write is
    ///   strict).
    async fn workflow_terminal_rows(
        &self,
    ) -> Result<Vec<(CorrelationKey, crate::workflow::WorkflowStatus)>, ObservationStoreError>;

    /// Read the current typed signal value for `key`, if present — the
    /// surface a `ctx.wait_for_signal(key)` blocks on per ADR-0064 §4.
    /// In-process single-node delivery (#207 cross-node-under-partition
    /// is OUT).
    ///
    /// # Preconditions
    /// - None. A key that has never been signalled simply returns
    ///   `Ok(None)` — that ABSENCE is the load-bearing signal the live
    ///   `ctx.wait_for_signal` path parks on (it re-reads via the
    ///   injected [`Clock`](crate::traits::clock::Clock) until the row
    ///   appears).
    ///
    /// # Postconditions
    /// - Returns `Ok(Some(value))` IFF an `ObservationRow::Signal { key,
    ///   value }` for this exact `key` has been written (single-node
    ///   read-after-write is strict). Returns `Ok(None)` otherwise.
    /// - One current value per key — a re-write of the same key replaces
    ///   the prior value (the value is opaque; last write wins on log
    ///   order).
    ///
    /// # Edge cases
    /// - No signal written for `key`: returns `Ok(None)`, not an error —
    ///   absence is the normal "still blocked" condition, not a failure.
    ///
    /// # Observable invariants
    /// - Calling this method has no side effects (it is a pure read).
    /// - `workflow_signal(key)` transitions monotonically from
    ///   `Ok(None)` to `Ok(Some(_))` for a given `key` (a written signal
    ///   is not un-written within an instance's lifetime).
    async fn workflow_signal(
        &self,
        key: &crate::workflow::SignalKey,
    ) -> Result<Option<crate::workflow::SignalValue>, ObservationStoreError>;
}
