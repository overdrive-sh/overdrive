//! `ProbeResultRow` — per-probe observation row, one latest row per
//! `(alloc_id, role, probe_idx)`, merged by the producer's run sequence
//! (PROBE-RUN-COUNT, ADR-0191).
//!
//! Per ADR-0054 §5 as amended by ADR-0080 § D2: a probe's latest
//! observed outcome is durable observation, never intent. Row identity
//! is the composite PK `(alloc_id, role, probe_idx)`;
//! latest-writer-wins. `role` is load-bearing in the key — `probe_idx`
//! is 0-indexed **within its own role array**, so without `role` the
//! three roles' probe 0 would collide on one key and clobber each
//! other under LWW. NOT append-mode —
//! per-tick history is recomputed at read time from this latest row
//! plus the live spec policy, NEVER persisted (per
//! `.claude/rules/development.md` § "Persist inputs, not derived
//! state").
//!
//! Per ADR-0048 + ADR-0054 §5 QR1: rkyv envelope with `#[repr(u8)]`
//! discriminant pinning. V1 = 0; V2 = 1 (PROBE-RUN-COUNT, ADR-0191:
//! the run sequence and the producer's observed streak), appended at
//! the tail. The fixture test pins both archived bytes AND variant
//! discriminant values:
//!
//! ```text
//! const FIXTURE_V1_DISCRIMINANT: u8 = 0;
//! const FIXTURE_V2_DISCRIMINANT: u8 = 1;
//! ```
//!
//! Cross-reference: auto-memory `feedback_rkyv_envelope_forward_
//! traps.md` documents the prior-known gap that motivated the
//! discriminant-pinning callout. The schema-evolution test lives at
//! `crates/overdrive-core/tests/schema_evolution/probe_result_row.rs`.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::codec::{EnvelopeError, VersionedEnvelope};
use crate::id::AllocationId;

/// 0-indexed position of a probe within its role's TOML array (or
/// `0` for the inferred default probe per ADR-0058 / C4).
///
/// `ProbeIdx` is the load-bearing cross-step variable per
/// `discuss/shared-artifacts-registry.md`: it MUST match across
/// (parser → ProbeResultRow PK → CLI render → wire payloads).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    utoipa::ToSchema,
)]
#[serde(transparent)]
pub struct ProbeIdx(pub u32);

impl ProbeIdx {
    pub const fn new(idx: u32) -> Self {
        Self(idx)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for ProbeIdx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Run sequence number of a written probe row: the producer's count of the
/// rows it has written for one `(alloc_id, role, probe_idx)` key, starting
/// at 1. A run that writes no row consumes no number (PROBE-RUN-COUNT,
/// ADR-0191).
///
/// The observation merge keeps, per key, the row with the greater
/// `run_seq` (see [`crate::traits::observation_store::ObservationStore::write_probe_result`]).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[serde(try_from = "u64", into = "u64")]
pub struct ProbeRunSeq(u64);

/// Parse / validation failure for [`ProbeRunSeq`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProbeRunSeqParseError {
    /// The value 0; a run sequence starts at 1.
    #[error("probe run sequence must be at least 1, got 0")]
    Zero,
    /// The input is not a decimal `u64`.
    #[error("probe run sequence is not a decimal u64: {0}")]
    Malformed(String),
}

impl ProbeRunSeq {
    /// The run sequence of the first row a probe task writes.
    pub const FIRST: Self = Self(1);

    /// Validating constructor; `Err(ProbeRunSeqParseError::Zero)` for 0.
    ///
    /// # Errors
    ///
    /// [`ProbeRunSeqParseError::Zero`] when `seq == 0`.
    pub const fn new(seq: u64) -> Result<Self, ProbeRunSeqParseError> {
        if seq == 0 {
            return Err(ProbeRunSeqParseError::Zero);
        }
        Ok(Self(seq))
    }

    /// The run sequence as a `u64` (always ≥ 1).
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The next run sequence (saturating; one run per second for a `u64`
    /// never reaches the ceiling).
    // SCAFFOLD: true — RED scaffold (netns-density-295, PROBE-RUN-COUNT);
    // implemented in DELIVER step 9.
    #[must_use]
    pub const fn succ(self) -> Self {
        let _ = self;
        panic!("Not yet implemented -- RED scaffold")
    }
}

impl std::str::FromStr for ProbeRunSeq {
    type Err = ProbeRunSeqParseError;

    /// Decimal, ≥ 1.
    // SCAFFOLD: true — RED scaffold; implemented in DELIVER step 9.
    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        panic!("Not yet implemented -- RED scaffold")
    }
}

impl std::fmt::Display for ProbeRunSeq {
    /// Decimal.
    // SCAFFOLD: true — RED scaffold; implemented in DELIVER step 9.
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        panic!("Not yet implemented -- RED scaffold")
    }
}

impl TryFrom<u64> for ProbeRunSeq {
    type Error = ProbeRunSeqParseError;

    /// Same as [`ProbeRunSeq::new`].
    fn try_from(seq: u64) -> Result<Self, Self::Error> {
        Self::new(seq)
    }
}

impl From<ProbeRunSeq> for u64 {
    fn from(seq: ProbeRunSeq) -> Self {
        seq.get()
    }
}

/// The probe runner's observed streak: the number of consecutive written
/// runs of one `(alloc_id, role, probe_idx)` key, ending with this row, that
/// produced this row's status (PROBE-RUN-COUNT, ADR-0191). Always ≥ 1;
/// independent of any threshold.
///
/// A `u32` because the thresholds it is compared with are `u32`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[serde(try_from = "u32", into = "u32")]
pub struct SameStatusRuns(u32);

/// Parse / validation failure for [`SameStatusRuns`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SameStatusRunsParseError {
    /// The value 0; a streak counts at least the row that carries it.
    #[error("probe same-status run count must be at least 1, got 0")]
    Zero,
    /// The input is not a decimal `u32`.
    #[error("probe same-status run count is not a decimal u32: {0}")]
    Malformed(String),
}

impl SameStatusRuns {
    /// The streak of a row whose status differs from the previous written
    /// row's, or of the first row a probe task writes.
    pub const ONE: Self = Self(1);

    /// Validating constructor; `Err(SameStatusRunsParseError::Zero)` for 0.
    ///
    /// # Errors
    ///
    /// [`SameStatusRunsParseError::Zero`] when `runs == 0`.
    pub const fn new(runs: u32) -> Result<Self, SameStatusRunsParseError> {
        if runs == 0 {
            return Err(SameStatusRunsParseError::Zero);
        }
        Ok(Self(runs))
    }

    /// The streak as a `u32` (always ≥ 1).
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The streak of the next written row of the same status (saturating).
    // SCAFFOLD: true — RED scaffold; implemented in DELIVER step 9.
    #[must_use]
    pub const fn succ(self) -> Self {
        let _ = self;
        panic!("Not yet implemented -- RED scaffold")
    }
}

impl std::str::FromStr for SameStatusRuns {
    type Err = SameStatusRunsParseError;

    /// Decimal, ≥ 1.
    // SCAFFOLD: true — RED scaffold; implemented in DELIVER step 9.
    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        panic!("Not yet implemented -- RED scaffold")
    }
}

impl std::fmt::Display for SameStatusRuns {
    /// Decimal.
    // SCAFFOLD: true — RED scaffold; implemented in DELIVER step 9.
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        panic!("Not yet implemented -- RED scaffold")
    }
}

impl TryFrom<u32> for SameStatusRuns {
    type Error = SameStatusRunsParseError;

    /// Same as [`SameStatusRuns::new`].
    fn try_from(runs: u32) -> Result<Self, Self::Error> {
        Self::new(runs)
    }
}

impl From<SameStatusRuns> for u32 {
    fn from(runs: SameStatusRuns) -> Self {
        runs.get()
    }
}

/// Role of a probe within the per-Service lifecycle.
///
/// Per ADR-0057: three roles `startup`, `readiness`, `liveness`,
/// matching k8s vocabulary. Role determines:
/// - When the probe runs (startup: bounded by `startup_deadline`;
///   readiness/liveness: continuous post-Stable).
/// - What the probe gates (startup: `Stable` predicate; readiness:
///   `Backend.healthy`; liveness: `RestartAllocation`).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ProbeRole {
    Startup,
    Readiness,
    Liveness,
}

impl ProbeRole {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Readiness => "readiness",
            Self::Liveness => "liveness",
        }
    }

    /// Stable durable-key discriminant per ADR-0080 § D2. Values are
    /// PERSISTED as the `role_byte` segment of the probe-result
    /// composite key (`alloc_id || 0x00 || role_byte || probe_idx LE
    /// u32`) — **never renumber; append only**.
    ///
    /// The byte order MUST agree with this enum's derived [`Ord`]
    /// (declaration order `Startup < Readiness < Liveness`) so the
    /// byte-keyed `LocalObservationStore` and the tuple-keyed
    /// `SimObservationStore` iterate a given alloc's rows in the same
    /// order, per `.claude/rules/development.md` § "Trait definitions
    /// specify behavior, not just signature".
    #[must_use]
    pub const fn as_key_byte(self) -> u8 {
        match self {
            Self::Startup => 0,
            Self::Readiness => 1,
            Self::Liveness => 2,
        }
    }
}

/// Observed outcome of a probe attempt as durable observation.
///
/// `Pending` is the synthetic Sentinel for "probe declared but not
/// yet ticked" — the renderer surfaces this as `last=pending` per
/// US-06. Adapters DO NOT write `Pending`; absence of the row IS
/// pending. The render layer materialises `Pending` from row
/// absence.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[serde(rename_all = "snake_case", tag = "status", content = "data")]
pub enum ProbeStatus {
    Pass,
    Fail { last_fail_reason: String },
}

/// `ProbeResultRow` V1 payload — the latest-observed outcome of a
/// single probe for a single allocation.
///
/// Composite PK: `(alloc_id, role, probe_idx)` per ADR-0080 § D2.
/// Superseded as the latest payload by [`ProbeResultRowV2`]; retained as
/// envelope variant V1 for decoding rows written before PROBE-RUN-COUNT.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct ProbeResultRowV1 {
    /// Allocation this probe targets.
    pub alloc_id: AllocationId,
    /// 0-indexed position within the role array.
    pub probe_idx: ProbeIdx,
    /// Which role this probe is.
    pub role: ProbeRole,
    /// Latest observed outcome.
    pub status: ProbeStatus,
    /// Wall-clock at which the probe attempt completed. UNIX-epoch
    /// milliseconds; sourced from the ProbeRunner's injected
    /// `Clock::unix_now()` per ADR-0013.
    pub last_observed_at_unix_ms: u64,
    /// Whether this probe was inferred by the platform (per ADR-0058
    /// default-TCP-startup inference rule) vs explicitly declared
    /// by the operator. Renderer surfaces as `(inferred)` suffix.
    pub inferred: bool,
}

/// `ProbeResultRow` V2 payload (PROBE-RUN-COUNT, ADR-0191): every
/// [`ProbeResultRowV1`] field, unchanged in name, type and meaning, plus the
/// producer's run sequence and its observed streak.
///
/// Composite PK: `(alloc_id, role, probe_idx)` per ADR-0080 § D2. The merge
/// keeps the row with the greater `run_seq`; `last_observed_at_unix_ms` is
/// shown to operators and no longer decides the merge.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct ProbeResultRowV2 {
    /// Allocation this probe targets.
    pub alloc_id: AllocationId,
    /// 0-indexed position within the role array.
    pub probe_idx: ProbeIdx,
    /// Which role this probe is.
    pub role: ProbeRole,
    /// Latest observed outcome.
    pub status: ProbeStatus,
    /// Observation time shown to operators; no longer decides the merge.
    pub last_observed_at_unix_ms: u64,
    /// Whether this probe was inferred by the platform (ADR-0058).
    pub inferred: bool,
    /// The producer's run sequence for `(alloc_id, role, probe_idx)`.
    pub run_seq: ProbeRunSeq,
    /// The producer's observed streak of `status` ending with this run.
    pub same_status_runs: SameStatusRuns,
}

/// Up-conversion of a V1 row: every field copied, `run_seq =
/// ProbeRunSeq::FIRST`, `same_status_runs = SameStatusRuns::ONE`. A V1 row
/// can only belong to an allocation of an earlier `serve` process, which
/// boot reclamation reclaims before any probe runs, so it is never counted
/// toward a live allocation (ADR-0191).
#[allow(
    clippy::fallible_impl_from,
    reason = "RED scaffold body until DELIVER step 9; the implemented conversion copies every field"
)]
impl From<ProbeResultRowV1> for ProbeResultRowV2 {
    // SCAFFOLD: true — RED scaffold; implemented in DELIVER step 9.
    fn from(_v1: ProbeResultRowV1) -> Self {
        panic!("Not yet implemented -- RED scaffold")
    }
}

/// Public alias — `ProbeResultRow` is V2's payload.
///
/// Per ADR-0048 alias-to-payload convention: public callers
/// construct `ProbeResultRow { ... }` directly using struct-literal
/// syntax. The codec-internal envelope is consumed only at the
/// persistence boundary.
pub type ProbeResultRow = ProbeResultRowV2;

/// Documentation alias for "the latest payload variant of
/// [`ProbeResultRowEnvelope`]". Mirrors the alias-to-payload
/// convention from ADR-0048 UI-02.
pub type ProbeResultRowLatest = ProbeResultRowV2;

/// Codec-internal envelope enum per ADR-0048.
///
/// NOT re-exported from `crates/overdrive-core/src/lib.rs` — only
/// persistence-boundary code in the worker / store-local adapters
/// names this type.
///
/// Discriminant pinning per ADR-0054 §5 QR1: `V1 = 0`, `V2 = 1`.
/// Future variants append at the tail only. The schema-evolution fixture
/// at `crates/overdrive-core/tests/schema_evolution/probe_result_row.rs`
/// pins the archived bytes AND the discriminant values.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum ProbeResultRowEnvelope {
    /// Discriminant 0 — the pre-PROBE-RUN-COUNT payload.
    V1(ProbeResultRowV1),
    /// Discriminant 1 — PROBE-RUN-COUNT (ADR-0191).
    V2(ProbeResultRowV2),
}

impl VersionedEnvelope for ProbeResultRowEnvelope {
    type Latest = ProbeResultRowV2;

    fn latest(payload: Self::Latest) -> Self {
        Self::V2(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1.into()),
            Self::V2(v2) => Ok(v2),
        }
    }

    /// Discriminant offset for `ProbeResultRowEnvelope` archives,
    /// measured from the END of the archive bytes.
    ///
    /// rkyv 0.8 places the archived envelope root at the end of the
    /// buffer and sizes it to its largest variant, so every archive (V1
    /// or V2 variant) carries the tag at the root's first byte: 72 bytes
    /// from the end (`tag` + 7 padding + the 64-byte archived
    /// `ProbeResultRowV2`). Re-pinned at the V2 bump together with the
    /// schema-evolution fixture's `GOLDEN_DISCRIMINANT_OFFSET` per
    /// `.claude/rules/development.md` § "Version-bump procedure".
    fn discriminant_offset_from_end() -> Option<usize> {
        Some(72)
    }

    fn known_discriminants() -> &'static [u8] {
        // V1 = 0, V2 = 1 (declaration order).
        &[0, 1]
    }

    fn type_name() -> &'static str {
        "ProbeResultRowEnvelope"
    }
}
