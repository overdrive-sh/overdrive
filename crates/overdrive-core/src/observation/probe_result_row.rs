//! `ProbeResultRow` — per-probe observation row, LWW per
//! `(alloc_id, probe_idx)`.
//!
//! Per ADR-0054 §5: a probe's latest observed outcome is durable
//! observation, never intent. Row identity is the composite PK
//! `(alloc_id, probe_idx)`; latest-writer-wins. NOT append-mode —
//! per-tick history is recomputed at read time from this latest row
//! plus the live spec policy, NEVER persisted (per
//! `.claude/rules/development.md` § "Persist inputs, not derived
//! state").
//!
//! Per ADR-0048 + ADR-0054 §5 QR1: rkyv envelope V1 with
//! `#[repr(u8)]` discriminant pinning. Discriminant for V1 = 0;
//! future V2/V3 append at the tail only. Fixture test pins both
//! archived bytes AND variant discriminant value:
//!
//! ```text
//! const FIXTURE_V1_DISCRIMINANT: u8 = 0;
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
/// Composite PK: `(alloc_id, probe_idx)`. LWW resolution per
/// `last_observed_at` (logical timestamp inherited from the
/// `ObservationRow` envelope; not duplicated here).
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

/// Public alias — `ProbeResultRow` is V1's payload.
///
/// Per ADR-0048 alias-to-payload convention: public callers
/// construct `ProbeResultRow { ... }` directly using struct-literal
/// syntax. The codec-internal envelope is consumed only at the
/// persistence boundary.
pub type ProbeResultRow = ProbeResultRowV1;

/// Documentation alias for "the latest payload variant of
/// [`ProbeResultRowEnvelope`]". Mirrors the alias-to-payload
/// convention from ADR-0048 UI-02.
pub type ProbeResultRowLatest = ProbeResultRowV1;

/// Codec-internal envelope enum per ADR-0048.
///
/// NOT re-exported from `crates/overdrive-core/src/lib.rs` — only
/// persistence-boundary code in the worker / store-local adapters
/// names this type.
///
/// Discriminant pinning per ADR-0054 §5 QR1: `V1 = 0`. Future
/// variants append at the tail only. The schema-evolution fixture
/// at `crates/overdrive-core/tests/schema_evolution/probe_result_row.rs`
/// pins both the archived bytes AND `FIXTURE_V1_DISCRIMINANT: u8 = 0`.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum ProbeResultRowEnvelope {
    V1(ProbeResultRowV1),
}

impl VersionedEnvelope for ProbeResultRowEnvelope {
    type Latest = ProbeResultRowV1;

    fn latest(payload: Self::Latest) -> Self {
        Self::V1(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1),
        }
    }

    /// Discriminant offset for `ProbeResultRowEnvelope` archives,
    /// measured from the END of the archive bytes.
    ///
    /// Empirically determined for canonical V1 payloads — rkyv 0.8
    /// places the outer enum's discriminant byte at a stable offset
    /// from the END of the archive, independent of variable-length
    /// payload growth. Pinned at the GREEN landing of slice 01-01
    /// alongside the schema-evolution fixture's
    /// `GOLDEN_DISCRIMINANT_OFFSET_V1` constant. Re-pin alongside
    /// the fixture at every version bump per
    /// `.claude/rules/development.md` § "Version-bump procedure".
    fn discriminant_offset_from_end() -> Option<usize> {
        Some(56)
    }

    fn known_discriminants() -> &'static [u8] {
        // V1 carries rkyv discriminant 0 (declaration order — first
        // variant). Empirically verified by archiving a canonical
        // `ProbeResultRowEnvelope::latest(...)` (alloc_id 14 chars,
        // ProbeStatus::Pass — no string payload) and inspecting the
        // byte at `bytes.len() - 56`; the perturbation surfaces
        // `"invalid discriminant 'N' for enum
        // 'ArchivedProbeResultRowEnvelope'"` at the top of the rkyv
        // bytecheck error chain (no preceding `trace:` frame),
        // confirming it is the outer-envelope tag and not a nested
        // enum's tag.
        &[0]
    }

    fn type_name() -> &'static str {
        "ProbeResultRowEnvelope"
    }
}
