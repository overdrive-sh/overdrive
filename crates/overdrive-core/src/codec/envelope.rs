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
//!
//!     fn type_name() -> &'static str {
//!         "ExampleEnvelope"
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
    /// Reserved variant-translation slot. `into_latest` itself does
    /// not surface [`EnvelopeError::UnknownVersion`] today — every
    /// envelope's `match self` is exhaustive at compile time, so
    /// an unknown variant cannot reach this fn. The primary path
    /// that surfaces `UnknownVersion` is the pre-decode probe
    /// [`probe_known_variant`] (upstream of the rkyv decode call),
    /// which inspects the archived discriminant byte against the
    /// envelope's [`Self::known_discriminants`] and reports the
    /// observed byte to the caller. The variant remains reachable
    /// from `into_latest` for future explicit-translation steps
    /// that may reject during the `From` chain.
    ///
    /// Returns [`EnvelopeError::Malformed`] when an inner-payload
    /// conversion via `From` chain itself fails (none today; the
    /// variant is reserved for future explicit-translation steps that
    /// can reject malformed data — e.g. an enum-rename migration).
    fn into_latest(self) -> Result<Self::Latest, EnvelopeError>;

    /// Distance (in bytes) of the rkyv enum discriminant from the END
    /// of the archived envelope bytes.
    ///
    /// rkyv 0.8 archives place variable-length payload data
    /// (strings, vecs) in a leading slab and the fixed-size "root"
    /// structure — including the outer enum's discriminant byte — at
    /// the END of the buffer. The distance from the buffer's end to
    /// the discriminant is therefore **stable across all archives of
    /// the same envelope shape**, regardless of how long any inner
    /// string or vec is. The absolute offset from the start, in
    /// contrast, shifts with every payload size change.
    ///
    /// Pin this offset by archiving canonical payloads of varying
    /// inner-string sizes and probing for the byte position whose
    /// mutation causes rkyv's bytecheck to surface
    /// `"invalid discriminant 'N' for enum 'Archived<EnvelopeName>'"`
    /// at the top of the error chain (no preceding `trace:` frame).
    /// The probe converges to a single `n - offset` value across
    /// every payload size.
    ///
    /// Returning `None` (the default) means the offset has not yet
    /// been pinned for this envelope; [`probe_known_variant`] becomes
    /// a no-op and unknown-tag bytes fall back to surfacing as
    /// [`EnvelopeError::Malformed`] via rkyv's bytecheck validator.
    /// New `VersionedEnvelope` impls that intend to surface
    /// [`EnvelopeError::UnknownVersion`] on future-binary bytes MUST
    /// override this function with the empirically-pinned offset.
    ///
    /// **Version-bump invariant.** When appending a `V<N+1>` variant
    /// to the envelope enum, the developer regenerates the
    /// schema-evolution fixture AND re-inspects this offset. If the
    /// `V<N+1>` payload's archived footprint differs from `V<N>`'s,
    /// rkyv re-pads the inline region to `max(V_1..V<N+1>)` and the
    /// distance from the end shifts. The pinned offset MUST be
    /// updated in the same commit as the variant addition; the
    /// schema-evolution roundtrip test pins the bytes, and this
    /// helper's offset value pins where this binary expects to find
    /// the discriminant in newly-archived envelopes.
    // mutants: skip — default-impl mutations are semantically
    // unreachable. cargo-mutants synthesises `Some(0)` / `Some(1)`
    // replacement bodies for this default. Every production
    // `VersionedEnvelope` impl (`JobEnvelope`,
    // `AllocStatusRowEnvelope`, `NodeHealthRowEnvelope`,
    // `ServiceHydrationResultRowEnvelope`,
    // `ServiceBackendRowEnvelope`) overrides this method with the
    // empirically-pinned offset; the default's `None` return is
    // observable only from an envelope that does NOT override it. No
    // such envelope exists in production code. The per-envelope
    // overrides ARE mutation-tested by the
    // `*_unknown_version_probe_surfaces` tests in
    // `tests/schema_evolution/`.
    fn discriminant_offset_from_end() -> Option<usize> {
        None
    }

    /// Set of variant discriminants this binary recognises.
    ///
    /// For a `V1`-only envelope this is `&[0]` (rkyv assigns
    /// discriminants in declaration order starting at 0). When `V2`
    /// is appended, the slice becomes `&[0, 1]` — the prior tag
    /// continues to round-trip through `into_latest` via the
    /// `From<V1>` chain.
    ///
    /// The default returns `&[]`; [`probe_known_variant`] is a no-op
    /// in that case and falls back to rkyv's bytecheck for
    /// classification.
    // mutants: skip — default-impl mutations are semantically
    // unreachable. cargo-mutants synthesises
    // `Vec::leak(Vec::new())`, `Vec::leak(vec![0])`, and
    // `Vec::leak(vec![1])` replacement bodies for this default.
    // Every production `VersionedEnvelope` impl overrides this
    // method with `&[0]`; the default's `&[]` return is observable
    // only from an envelope that does NOT override it. No such
    // envelope exists in production code. Per-envelope overrides
    // ARE mutation-tested by the `*_unknown_version_probe_surfaces`
    // tests in `tests/schema_evolution/`.
    fn known_discriminants() -> &'static [u8] {
        &[]
    }

    /// Stable diagnostic name for the envelope (e.g. `"JobEnvelope"`,
    /// `"AllocStatusRowEnvelope"`).
    ///
    /// Used by [`EnvelopeError::UnknownVersion`] to identify which
    /// envelope's read path surfaced the unknown tag and by tracing
    /// events at the decode boundary. The default is `"<unknown>"`;
    /// every production envelope SHOULD override this with its
    /// canonical name.
    // mutants: skip — default-impl mutations are semantically
    // unreachable. cargo-mutants synthesises `""` and `"xyzzy"`
    // replacement bodies for this default. Every production
    // `VersionedEnvelope` impl overrides this method with its
    // canonical name; the default's `"<unknown>"` return is
    // observable only from an envelope that does NOT override it.
    // No such envelope exists in production code. Per-envelope
    // overrides ARE mutation-tested by the
    // `*_unknown_version_probe_surfaces` tests in
    // `tests/schema_evolution/`.
    fn type_name() -> &'static str {
        "<unknown>"
    }
}

/// Inspect the rkyv-archived discriminant byte BEFORE attempting full
/// decode and classify out-of-set values as
/// [`EnvelopeError::UnknownVersion`].
///
/// # Why this exists
///
/// Per ADR-0048 § 3 the envelope read path needs to distinguish two
/// failure shapes:
///
/// * **Unknown future variant** — the bytes were written by a newer
///   binary that bumped to `V<N+1>` while this reader knows only up
///   to `V<N>`. The operator remediation is "upgrade this binary or
///   delete the redb file." The intent path refuses to start;
///   observation rows are log+skipped per § 3.
/// * **Malformed bytes** — the bytes were not produced by this
///   envelope's writer (truncation, corruption, different envelope
///   shape). The operator remediation is "delete the redb file."
///
/// Without a pre-decode probe both shapes collapse into
/// [`EnvelopeError::Malformed`] (rkyv's bytecheck rejects every
/// out-of-set discriminant the same way). This helper inspects the
/// discriminant byte at the empirically-pinned position
/// (`bytes.len() - E::discriminant_offset_from_end()`) and returns
/// [`EnvelopeError::UnknownVersion`] for tags outside the known set
/// ([`VersionedEnvelope::known_discriminants`]) — the typed
/// classification feeds operator-facing diagnostics and lets the
/// caller branch.
///
/// # Behavior
///
/// * If the envelope's
///   [`discriminant_offset_from_end`](VersionedEnvelope::discriminant_offset_from_end)
///   is `None`, the probe is a no-op and returns `Ok(())` — the
///   caller falls through to rkyv decode, which classifies the
///   bytes via bytecheck.
/// * If the byte slice is shorter than the from-end offset, the
///   probe also returns `Ok(())` and lets rkyv surface the
///   truncation as [`EnvelopeError::Malformed`].
/// * If the discriminant byte is in
///   [`known_discriminants`](VersionedEnvelope::known_discriminants),
///   returns `Ok(())` and the caller proceeds with rkyv decode.
/// * Otherwise returns [`EnvelopeError::UnknownVersion`] with the
///   observed byte, the envelope's
///   [`type_name`](VersionedEnvelope::type_name), and the highest
///   known variant tag (max of `known_discriminants`).
///
/// # Composition
///
/// Callers compose this with `rkyv::from_bytes` at every persistence
/// boundary:
///
/// ```rust,ignore
/// let bytes: &[u8] = /* read from redb */;
/// probe_known_variant::<FooEnvelope>(bytes)?;
/// let envelope: FooEnvelope = rkyv::from_bytes(bytes)
///     .map_err(|source| EnvelopeError::Malformed { source })?;
/// let latest = envelope.into_latest()?;
/// ```
///
/// The probe MUST run BEFORE the `from_bytes` call — running it
/// after would still surface the bytecheck rejection as `Malformed`,
/// losing the `UnknownVersion` distinction.
pub(crate) fn probe_known_variant<E: VersionedEnvelope>(bytes: &[u8]) -> Result<(), EnvelopeError> {
    // No offset pinned → probe is a no-op; rkyv decode classifies.
    let Some(from_end) = E::discriminant_offset_from_end() else {
        return Ok(());
    };
    // Bytes too short to contain the trailing root region → let rkyv
    // surface the truncation as Malformed.
    let Some(offset) = bytes.len().checked_sub(from_end) else {
        return Ok(());
    };
    let Some(observed) = bytes.get(offset).copied() else {
        return Ok(());
    };
    let known = E::known_discriminants();
    if known.contains(&observed) {
        return Ok(());
    }
    Err(EnvelopeError::UnknownVersion {
        observed,
        type_name: E::type_name(),
        supported_max: known.iter().copied().max().unwrap_or(0),
    })
}

/// Decode `bytes` through a [`VersionedEnvelope`] and project to the
/// `Latest` payload shape — the canonical composition every
/// persistence-boundary read site performs.
///
/// # Why this helper exists
///
/// Every redb adapter that reads a versioned envelope writes the same
/// three-step shape:
///
/// 1. Copy the redb-returned slice (unknown alignment) into an
///    `AlignedVec::<8>` (rkyv 0.8 requires 8-byte alignment).
/// 2. Call [`probe_known_variant::<E>`] to surface known-but-unsupported
///    tags as [`EnvelopeError::UnknownVersion`] before rkyv decode would
///    collapse them into [`EnvelopeError::Malformed`].
/// 3. `rkyv::from_bytes::<E, rkyv::rancor::Error>` and project through
///    [`VersionedEnvelope::into_latest`].
///
/// Four observation tables plus the Job aggregate had hand-copied
/// versions of this shape. Per ADR-0048 § 4b ("typed codec on the
/// persisted value") the wrapping discipline belongs in one place; this
/// helper is that place for the read direction, mirroring how
/// [`VersionedEnvelope::latest`] is the one place for the write
/// direction.
///
/// # Behavior
///
/// * Returns `Ok(latest)` when the bytes decode to a known variant and
///   project cleanly to `Self::Latest`.
/// * Returns [`EnvelopeError::UnknownVersion`] when the empirically-
///   pinned discriminant byte names a tag outside
///   [`VersionedEnvelope::known_discriminants`] (future-binary surface).
/// * Returns [`EnvelopeError::Malformed`] when rkyv's bytecheck rejects
///   the slice (truncation, corruption, foreign envelope shape) or
///   when an `into_latest` chain itself fails.
///
/// # Composition
///
/// Callers wrap this in their layer-specific error surface — intent
/// fail-fast via `IntentStoreError::Envelope`, observation log+skip via
/// `ObservationStoreError::Envelope`. The asymmetric read policy
/// (ADR-0048 § 3) lives one layer up; this helper carries only the
/// decode primitive.
pub fn decode_envelope_bytes<E>(bytes: &[u8]) -> Result<E::Latest, EnvelopeError>
where
    E: VersionedEnvelope + rkyv::Archive,
    E::Archived: for<'a> rkyv::bytecheck::CheckBytes<rkyv::api::high::HighValidator<'a, rkyv::rancor::Error>>
        + rkyv::Deserialize<E, rkyv::rancor::Strategy<rkyv::de::Pool, rkyv::rancor::Error>>,
{
    let mut aligned = rkyv::util::AlignedVec::<8>::new();
    aligned.extend_from_slice(bytes);

    probe_known_variant::<E>(aligned.as_ref())?;

    let envelope: E = rkyv::from_bytes::<E, rkyv::rancor::Error>(&aligned)
        .map_err(|source| EnvelopeError::Malformed { source })?;
    envelope.into_latest()
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
    ///
    /// The structured surface is produced by [`probe_known_variant`]
    /// against the envelope's empirically-pinned discriminant offset.
    /// Without that probe, rkyv's bytecheck would collapse this case
    /// into [`Self::Malformed`] — the probe restores the distinction
    /// for operator-facing diagnostics.
    #[error(
        "{type_name} carries unknown version tag {observed} (this binary supports up to V{supported_max})"
    )]
    UnknownVersion {
        /// The discriminant byte the bytes carried.
        observed: u8,
        /// Stable diagnostic name of the envelope whose read path
        /// surfaced the unknown tag (e.g. `"JobEnvelope"`,
        /// `"AllocStatusRowEnvelope"`).
        type_name: &'static str,
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
