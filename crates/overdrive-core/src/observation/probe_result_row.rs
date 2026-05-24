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
//!
//! RED scaffold — types and envelope land empty here; bodies and
//! schema-evolution fixture land in slice 01.
// SCAFFOLD: true

#![allow(dead_code)]
#![expect(clippy::todo, reason = "RED scaffold; lands GREEN in slice-01")]
#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::struct_field_names,
    reason = "DISTILL RED scaffold; per `.claude/rules/testing.md` § 'RED scaffolds' lints land when DELIVER replaces todo!() bodies + rewrites docs"
)]

use serde::{Deserialize, Serialize};

use crate::id::AllocationId;

/// 0-indexed position of a probe within its role's TOML array (or
/// `0` for the inferred default probe per ADR-0058 / C4).
///
/// `ProbeIdx` is the load-bearing cross-step variable per
/// `discuss/shared-artifacts-registry.md`: it MUST match across
/// (parser → ProbeResultRow PK → CLI render → wire payloads).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProbeIdx(pub u32);

impl ProbeIdx {
    pub const fn new(idx: u32) -> Self {
        Self(idx)
    }

    pub const fn get(self) -> u32 {
        self.0
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
///
/// RED scaffold — fields documented; rkyv derives + envelope wiring
/// land in slice 01.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Codec-internal envelope enum per ADR-0048.
///
/// NOT re-exported from `crates/overdrive-core/src/lib.rs` — only
/// persistence-boundary code in the worker / store-local adapters
/// names this type.
///
/// Discriminant pinning per ADR-0054 §5 QR1: `V1 = 0`. Future
/// variants append at the tail only. The schema-evolution fixture
/// at `crates/overdrive-core/tests/schema_evolution/
/// probe_result_row.rs` pins both the archived bytes AND
/// `FIXTURE_V1_DISCRIMINANT: u8 = 0`.
///
/// RED scaffold — rkyv derives land in slice 01.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ProbeResultRowEnvelope {
    V1(ProbeResultRowV1) = 0,
}

impl ProbeResultRowEnvelope {
    /// Wrap the current `Latest` payload into the envelope. The
    /// persistence-boundary code is the only call site.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "RED scaffold; production body in slice-01 will consume payload into Self::V1"
    )]
    pub fn latest(payload: ProbeResultRowV1) -> Self {
        let _ = payload;
        todo!("RED scaffold: ProbeResultRowEnvelope::latest — wire into Self::V1 in slice-01")
    }

    /// Project the envelope into the current `Latest` payload.
    /// Older variants chain through `From` impls; V1 returns
    /// self-payload unchanged.
    pub fn into_latest(self) -> Result<ProbeResultRowV1, EnvelopeError> {
        todo!("RED scaffold: ProbeResultRowEnvelope::into_latest — implement V1 arm in slice-01")
    }
}

/// Envelope projection error — surfaces when archived bytes decode
/// into an envelope variant that cannot be projected to the current
/// `Latest`. Slice 01 has only V1, so this is structurally
/// unreachable today; the variant exists for forward-compat per
/// ADR-0048.
#[derive(Debug, thiserror::Error)]
pub enum EnvelopeError {
    #[error("envelope variant {variant} cannot be projected to latest")]
    UnsupportedVariant { variant: u8 },
}
