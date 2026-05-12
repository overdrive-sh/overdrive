//! `VersionedEnvelope` trait + `EnvelopeError` enum.
//!
//! Per ADR-0048 (§ 1 per-type rkyv enum; § 3 read policy) every
//! rkyv-persisted type at a durable-storage boundary is wrapped in a
//! per-type envelope enum (e.g. `AllocStatusRowEnvelope`,
//! `JobEnvelope`). The trait defined here is the shared contract:
//!
//! * [`VersionedEnvelope::latest`] — construct the envelope wrapping
//!   the latest variant. Writers go through this path exclusively
//!   (Layer 1 / Layer 2 enforcement in ADR-0048 § 2 prevents direct
//!   variant construction cross-crate and in-crate).
//! * [`VersionedEnvelope::into_latest`] — read path. Older variants
//!   chain through `From<V_N>` impls to the `Latest` shape; unknown /
//!   malformed bytes surface as [`EnvelopeError`].
//!
//! The asymmetric read policy (intent fail-fast; observation degrade)
//! lives one layer up at each driving port — this trait carries the
//! decode primitive; the caller decides how to react.
//!
//! # Example
//!
//! A minimal envelope implementation, exercised through its round-trip
//! invariant `latest(p).into_latest() == Ok(p)`:
//!
//! ```
//! use overdrive_core::codec::{EnvelopeError, VersionedEnvelope};
//!
//! #[derive(Debug, Clone, PartialEq, Eq)]
//! pub enum ExampleEnvelope {
//!     V1(ExampleV1),
//! }
//!
//! #[derive(Debug, Clone, PartialEq, Eq)]
//! pub struct ExampleV1 {
//!     pub value: u32,
//! }
//!
//! impl VersionedEnvelope for ExampleEnvelope {
//!     type Latest = ExampleV1;
//!
//!     fn latest(payload: Self::Latest) -> Self {
//!         Self::V1(payload)
//!     }
//!
//!     fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
//!         match self {
//!             Self::V1(v1) => Ok(v1),
//!         }
//!     }
//! }
//!
//! let payload = ExampleV1 { value: 42 };
//! let envelope = ExampleEnvelope::latest(payload.clone());
//! assert_eq!(envelope.into_latest().unwrap(), payload);
//! ```

/// Per-type versioned envelope contract.
///
/// Every rkyv-persisted type at a durable-storage boundary implements
/// this trait on its per-type envelope enum. See ADR-0048 § 1.
///
/// # Contract
///
/// The trait carries a strict round-trip invariant: for every payload
/// `p` of type `Self::Latest`,
///
/// ```text
/// Self::latest(p).into_latest() == Ok(p)
/// ```
///
/// must hold. This is the property the per-envelope schema-evolution
/// fixtures pin (see `tests/schema_evolution/harness.rs`).
///
/// Historical variants converge to the latest shape through chained
/// `From<V_N> for V_{N+1}` impls — see § "rkyv schema evolution" in
/// `.claude/rules/development.md` for the version-bump procedure.
pub trait VersionedEnvelope {
    /// The latest (current) payload variant the envelope wraps.
    ///
    /// Bumping `Latest` to a new payload type is the load-bearing
    /// step in the version-bump procedure — every writer in the
    /// codebase begins constructing `Self::latest(NewPayload { ... })`
    /// from that commit forward.
    type Latest;

    /// Construct the envelope wrapping the latest payload.
    ///
    /// # Preconditions
    ///
    /// `payload` is a valid `Self::Latest`. There are no further
    /// preconditions — the constructor cannot fail.
    ///
    /// # Postconditions
    ///
    /// The returned envelope wraps `payload` in the highest variant
    /// the implementer recognises (`Self::V<N>(payload)` where `V<N>`
    /// is the variant whose payload type is `Self::Latest`).
    /// `self.into_latest()` on the returned envelope yields
    /// `Ok(payload)` bit-equivalent to the input.
    ///
    /// # Observable invariants
    ///
    /// `latest(p).into_latest() == Ok(p)` for every `p: Self::Latest`.
    /// Writers MUST construct envelopes exclusively through this
    /// method — direct variant construction
    /// (`Self::V<N>(payload)`) is forbidden cross-crate by the
    /// non-re-export of inner payload types from
    /// `overdrive_core::lib.rs`, and in-crate by the
    /// `scan_for_envelope_variant_construction` dst-lint scanner
    /// (ADR-0048 § 2 Layer 2).
    fn latest(payload: Self::Latest) -> Self;

    /// Up-convert any historical variant through `From` impls to the
    /// `Latest` shape.
    ///
    /// # Preconditions
    ///
    /// `self` was deserialised from rkyv bytes that decoded
    /// successfully into one of the envelope's known variants. The
    /// caller is responsible for surfacing rkyv-level decode failures
    /// as [`EnvelopeError::Malformed`] before invoking this method
    /// (the rkyv `from_bytes` call site is the natural attachment
    /// point — see the shared `assert_envelope_v_roundtrip` harness
    /// for the canonical decode-then-project pattern).
    ///
    /// # Postconditions
    ///
    /// On `Ok(latest)`, `latest` is the canonical projection of
    /// `self`'s payload into `Self::Latest`. The conversion chain
    /// `V_1 -> V_2 -> ... -> V<N> = Latest` traverses `From<V_K> for
    /// V_{K+1}` impls, each additive-only per ADR-0048 § 1.
    ///
    /// # Edge cases
    ///
    /// * Latest-variant input: returns `Ok(payload)` with the inner
    ///   payload unwrapped directly (no `From` chain).
    /// * Historical-variant input: applies the chain of `From` impls
    ///   from the observed variant up to `Latest`, then returns
    ///   `Ok(...)`.
    ///
    /// # Errors
    ///
    /// Returns [`EnvelopeError::UnknownVersion`] when the envelope
    /// somehow carries a variant tag this binary does not know — in
    /// practice this is unreachable for envelopes whose enum
    /// definition is exhaustive at compile time, but the variant
    /// remains in the error type so future dynamic-variant designs
    /// have a typed slot.
    ///
    /// Returns [`EnvelopeError::Malformed`] when an inner-payload
    /// conversion via `From` chain itself fails (none today; the
    /// variant is reserved for future explicit-translation steps that
    /// can reject malformed data — e.g. an enum-rename migration).
    fn into_latest(self) -> Result<Self::Latest, EnvelopeError>;
}

/// Errors produced when decoding bytes through a [`VersionedEnvelope`].
///
/// Per ADR-0048 § 3 the read policy is asymmetric — intent refuses to
/// start on either variant; observation rows log + skip the offending
/// row. The error type carries the structured cause so the caller can
/// branch.
#[derive(Debug, thiserror::Error)]
pub enum EnvelopeError {
    /// Bytes decoded to a variant tag this binary does not know.
    ///
    /// Surfaces when an older binary attempts to read bytes written
    /// by a newer binary that bumped to `V<N+1>` while the reader
    /// still knows only up to `V<N>`. Per ADR-0048 § 3 the intent
    /// path refuses to start (operator remediates by upgrading or
    /// deleting the redb file); the observation path logs and skips
    /// the offending row.
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
    ///
    /// Surfaces when the underlying rkyv validator rejects the byte
    /// slice — either the bytes were not produced by `rkyv::to_bytes`
    /// against this envelope shape, or they were truncated /
    /// corrupted in transit.
    #[error("envelope bytes are malformed: {source}")]
    Malformed {
        /// The underlying rkyv validator error.
        #[source]
        source: rkyv::rancor::Error,
    },
}
