//! `VersionedEnvelope` trait + `EnvelopeError` enum.
//!
//! Per ADR-0048 (§ 1 per-type rkyv enum; § 3 read policy) every
//! rkyv-persisted type at a durable-storage boundary is wrapped in a
//! per-type envelope enum (e.g. `AllocStatusRowEnvelope`,
//! `JobEnvelope`). The trait defined here is the shared contract:
//!
//! * `latest(payload)` — construct the envelope wrapping the latest
//!   variant. Writers go through this path exclusively (Layer 1 /
//!   Layer 2 enforcement in ADR-0048 § 2 prevents direct variant
//!   construction cross-crate and in-crate).
//! * `into_latest()` — read path. Older variants chain through
//!   `From<V_N>` impls to the `Latest` shape; unknown / malformed
//!   bytes surface as [`EnvelopeError`].
//!
//! The asymmetric read policy (intent fail-fast; observation degrade)
//! lives one layer up at each driving port — this trait carries the
//! decode primitive; the caller decides how to react.
//!
//! # SCAFFOLD: true
//!
//! Phase 1 RED scaffold; lands GREEN in DELIVER steps 01-02..03-02.

// SCAFFOLD: true
/// Per-type versioned envelope contract.
///
/// Every rkyv-persisted type at a durable-storage boundary implements
/// this trait on its per-type envelope enum. See ADR-0048 § 1.
pub trait VersionedEnvelope {
    /// The latest (current) payload variant the envelope wraps.
    type Latest;

    /// Construct the envelope wrapping the latest payload.
    ///
    /// GREEN: every implementer wraps `payload` into the highest
    /// variant (e.g. `Self::V1(payload)` today, `Self::V2(payload)`
    /// after the next bump).
    fn latest(payload: Self::Latest) -> Self;

    /// Up-convert any historical variant through `From` impls to the
    /// `Latest` shape.
    ///
    /// GREEN: each implementer matches on its variants and converts
    /// via `From<V_N>` for `V_{N+1}`. Unknown variant tags and
    /// malformed payloads surface as [`EnvelopeError`].
    ///
    /// # Errors
    ///
    /// Returns [`EnvelopeError::UnknownVersion`] when the decoded
    /// envelope carries a variant tag this binary does not know,
    /// and [`EnvelopeError::Malformed`] when the underlying bytes
    /// did not decode as the envelope at all.
    fn into_latest(self) -> Result<Self::Latest, EnvelopeError>;
}

// SCAFFOLD: true
/// Errors produced when decoding bytes through a `VersionedEnvelope`.
///
/// Per ADR-0048 § 3 the read policy is asymmetric — intent refuses to
/// start on either variant; observation rows log + skip the offending
/// row. The error type carries the structured cause so the caller can
/// branch.
#[derive(Debug, thiserror::Error)]
pub enum EnvelopeError {
    /// Bytes decoded to a variant tag this binary does not know.
    #[error(
        "envelope carries unknown version tag {observed} (this binary supports up to V{supported_max})"
    )]
    UnknownVersion {
        /// The discriminant byte the bytes carried.
        observed: u8,
        /// The highest variant this binary recognises.
        supported_max: u8,
    },

    /// Bytes did not decode as the envelope at all (truncated,
    /// corrupt, or a different type).
    #[error("envelope bytes are malformed: {source}")]
    Malformed {
        /// The underlying rkyv validator error.
        #[source]
        source: rkyv::rancor::Error,
    },
}
