//! Shared schema-evolution roundtrip primitive.
//!
//! Per `.claude/rules/testing.md` § "Archive schema-evolution
//! roundtrip" + ADR-0048 § 6, every rkyv-versioned envelope ships a
//! per-version golden-bytes fixture pinning the bytes of a historical
//! payload. The fixture asserts that the hex-encoded bytes still
//! deserialise through today's envelope shape into today's `Latest`
//! projection — without that defense, a "merely additive" change to
//! an inner payload silently drifts the archived layout and corrupts
//! previously-persisted rows.
//!
//! This module exposes the one primitive every fixture in
//! `tests/schema_evolution/<envelope>.rs` consumes:
//! [`assert_envelope_v_roundtrip`]. The primitive takes a hex string
//! and a canonical `Latest` projection and asserts equality — it does
//! NOT bake per-envelope knowledge.

use overdrive_core::codec::{EnvelopeError, VersionedEnvelope, decode_envelope_bytes};

/// Decode `fixture_hex` (the pinned archived bytes of a historical
/// payload variant), deserialise into the envelope `E`, project to
/// `E::Latest`, and assert equality against `expected`.
///
/// The bound shape mirrors the canonical rkyv 0.8.x deserialise call
/// site used elsewhere in this workspace
/// (`crates/overdrive-store-local/src/observation_backend.rs`).
///
/// # Panics
///
/// Panics with a fixture-identifying message if any of:
/// * `fixture_hex` does not hex-decode.
/// * The bytes do not deserialise as `E`.
/// * `E::into_latest()` reports [`EnvelopeError`].
/// * The projected `Latest` value does not equal `expected`.
///
/// # Example call site
///
/// Per-envelope fixtures consume the primitive in one line:
///
/// ```rust,ignore
/// const FIXTURE_V1: &str = "<hex-encoded archived bytes of V1 payload>";
///
/// #[test]
/// fn alloc_status_row_v1_decodes_through_current_envelope() {
///     let expected = AllocStatusRowLatest { /* canonical V1 projection */ };
///     assert_envelope_v_roundtrip::<AllocStatusRowEnvelope>(
///         FIXTURE_V1, &expected,
///     );
/// }
/// ```
///
/// See `tests/schema_evolution/harness.rs` self-test for a working
/// example against an inline mock envelope.
pub fn assert_envelope_v_roundtrip<E>(fixture_hex: &str, expected: &E::Latest)
where
    E: VersionedEnvelope + rkyv::Archive,
    for<'a> <E as rkyv::Archive>::Archived: rkyv::bytecheck::CheckBytes<rkyv::api::high::HighValidator<'a, rkyv::rancor::Error>>
        + rkyv::Deserialize<E, rkyv::api::high::HighDeserializer<rkyv::rancor::Error>>,
    E::Latest: PartialEq + std::fmt::Debug,
{
    let bytes = hex::decode(fixture_hex.trim())
        .expect("schema_evolution fixture hex string must decode cleanly");

    // redb / on-disk reads land at unknown alignment; rkyv requires
    // 8-byte alignment. Copy into AlignedVec before deserialising.
    // Mirrors the production call site in
    // `overdrive-store-local::observation_backend`.
    let mut aligned = rkyv::util::AlignedVec::<8>::new();
    aligned.extend_from_slice(&bytes);

    let envelope: E = rkyv::from_bytes::<E, rkyv::rancor::Error>(&aligned)
        .expect("schema_evolution fixture bytes must deserialise as the envelope");

    let latest = envelope
        .into_latest()
        .expect("schema_evolution fixture must project to Latest without error");

    assert_eq!(
        &latest, expected,
        "schema_evolution Latest projection must equal the canonical expected payload",
    );
}

/// Triangulate the `discriminant_offset_from_end()` value against an
/// independent hard-coded golden constant.
///
/// The bare [`assert_discriminant_byte_at_pinned_offset`] helper reads
/// the offset from `E::discriminant_offset_from_end()` itself — the
/// same trait method whose correctness is the structural defense the
/// helper is supposed to enforce. That is **tautological**: if a
/// future commit silently edits the trait method's value (say to a
/// nearby position that also happens to land on a zero byte —
/// archived enums have many zero bytes from padding, zero
/// discriminants, and empty string-length headers), the helper still
/// passes and the `UnknownVersion` probe silently targets the wrong
/// position.
///
/// This helper closes the loop by accepting `golden_offset` as a
/// second, **independent** source of truth — pinned as a `const` in
/// each envelope's per-fixture test file. The helper asserts:
///
/// 1. `E::discriminant_offset_from_end() == Some(golden_offset)` —
///    the trait method has not drifted from the golden constant
///    without the fixture being updated.
/// 2. `bytes.len() >= golden_offset` — the canonical archive is at
///    least the golden offset long.
/// 3. `bytes[bytes.len() - golden_offset] == expected_tag` — the
///    canonical archive places the V<N> discriminant at the golden
///    position.
///
/// A future `V<N+1>` bump that genuinely shifts the offset (because
/// the payload grew the trailing root region) must update BOTH the
/// trait method's return value AND the per-envelope golden constant
/// in the same commit. A unilateral edit to either one trips this
/// assertion. This is the "two-source triangulation" structural
/// defense.
///
/// # Panics
///
/// Panics with a fixture-identifying message if any of the three
/// assertions above fails.
pub fn assert_discriminant_offset_triangulation<E>(
    canonical: E::Latest,
    golden_offset: usize,
    expected_tag: u8,
) where
    E: VersionedEnvelope
        + rkyv::Archive
        + for<'a> rkyv::Serialize<
            rkyv::api::high::HighSerializer<
                rkyv::util::AlignedVec,
                rkyv::ser::allocator::ArenaHandle<'a>,
                rkyv::rancor::Error,
            >,
        >,
{
    // Triangulation pin 1: the trait method must agree with the
    // golden constant. Catches a unilateral edit to the trait method
    // that did not update the per-envelope fixture (or vice versa).
    let trait_offset = E::discriminant_offset_from_end();
    assert_eq!(
        trait_offset,
        Some(golden_offset),
        "envelope's discriminant_offset_from_end() drifted from the per-envelope golden \
         constant — trait method returned {trait_offset:?}, golden is {golden_offset}. On a \
         V<N+1> bump both must be updated in the same commit per development.md § \
         Version-bump procedure.",
    );

    let envelope = E::latest(canonical);
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope)
        .expect("rkyv archive of canonical payload must succeed");

    // Triangulation pin 2: the canonical archived layout must place
    // the discriminant byte at the golden offset. Catches a rkyv
    // layout drift (e.g. an additive change to the V<N> payload that
    // shifted the trailing root region) that was not reflected in
    // either pin.
    let buffer_len = bytes.len();
    assert!(
        buffer_len >= golden_offset,
        "archived bytes ({buffer_len}) must be at least the golden offset ({golden_offset}) \
         long — golden offset outside the buffer means rkyv's archived layout shrank, the \
         trait method/golden agreement is meaningless, and the probe is targeting padding \
         or the empty before-buffer region",
    );

    let absolute_index = buffer_len - golden_offset;
    let observed = bytes[absolute_index];
    assert_eq!(
        observed, expected_tag,
        "discriminant byte at golden offset {golden_offset} (from end of {buffer_len}-byte \
         buffer, absolute index {absolute_index}) must equal V<N> tag {expected_tag} — got \
         {observed}. Either the golden offset is stale (re-pin per development.md § \
         Version-bump procedure, update BOTH the trait method and the golden constant in \
         the same commit) or the envelope's archived layout drifted without a version bump.",
    );
}

/// Pin the envelope's introspection surface (`known_discriminants`,
/// `type_name`, `discriminant_offset_from_end`) end-to-end through
/// [`decode_envelope_bytes`].
///
/// Per ADR-0048 § 3, the read path must distinguish "unknown future
/// variant" (returns [`EnvelopeError::UnknownVersion`] with structured
/// fields the operator can act on) from "malformed bytes" (returns
/// [`EnvelopeError::Malformed`]). The distinction depends on three
/// per-envelope trait methods working in concert:
///
/// 1. `discriminant_offset_from_end()` — where in the archive the
///    pre-decode probe inspects for the tag byte.
/// 2. `known_discriminants()` — the set of tags this binary recognises.
/// 3. `type_name()` — the diagnostic name surfaced in the typed error.
///
/// Without an end-to-end test pinning all three for each envelope, a
/// future edit could silently flip the probe's classification for
/// individual envelopes — mutation testing would catch the unkilled
/// blind spot, but only after the regression landed.
///
/// This helper closes that gap. It performs two assertions per
/// envelope:
///
/// * **Valid bytes round-trip.** Archives `canonical`, decodes via
///   [`decode_envelope_bytes`], asserts equality. Kills any mutation
///   that mis-shapes `known_discriminants()` to exclude the V1 tag
///   (e.g. `Vec::leak(Vec::new())` or `Vec::leak(vec![1])`) — those
///   mutations cause the probe to flag VALID bytes as
///   [`EnvelopeError::UnknownVersion`], so the round-trip fails.
/// * **Unknown-tag bytes surface `UnknownVersion`.** Synthesises bytes
///   with tag `UNKNOWN_TAG` at the empirically-pinned offset, asserts
///   [`decode_envelope_bytes`] returns
///   [`EnvelopeError::UnknownVersion`] with `observed == UNKNOWN_TAG`,
///   `type_name == expected_type_name`, and `supported_max ==
///   expected_supported_max`. Kills any mutation that erases
///   `type_name()` (e.g. `""` or `"xyzzy"`), nulls
///   `discriminant_offset_from_end()` (makes the probe a no-op, so the
///   tag-99 bytes fall through to bytecheck and surface as
///   `Malformed`), or erases the `probe_known_variant` body to
///   `Ok(())`.
///
/// # Choice of `UNKNOWN_TAG`
///
/// `99` matches the value used by the existing intent-side
/// integration test
/// (`crates/overdrive-store-local/tests/integration/envelope_intent_refuse.rs`).
/// Any value outside `known_discriminants()` works mechanically; `99`
/// keeps cross-test diagnostics consistent.
///
/// # Panics
///
/// Panics with a fixture-identifying message if any assertion fails.
///
/// # Example call site
///
/// ```rust,ignore
/// #[test]
/// fn alloc_status_row_unknown_version_probe_surfaces() {
///     assert_unknown_version_probe_surfaces::<AllocStatusRowEnvelope>(
///         canonical_v1_payload(),
///         "AllocStatusRowEnvelope",
///         0, // V1 only — highest known tag is 0
///     );
/// }
/// ```
pub fn assert_unknown_version_probe_surfaces<E>(
    canonical: E::Latest,
    expected_type_name: &'static str,
    expected_supported_max: u8,
) where
    E: VersionedEnvelope
        + rkyv::Archive
        + for<'a> rkyv::Serialize<
            rkyv::api::high::HighSerializer<
                rkyv::util::AlignedVec,
                rkyv::ser::allocator::ArenaHandle<'a>,
                rkyv::rancor::Error,
            >,
        >,
    E::Latest: Clone + PartialEq + std::fmt::Debug,
    <E as rkyv::Archive>::Archived: for<'a> rkyv::bytecheck::CheckBytes<rkyv::api::high::HighValidator<'a, rkyv::rancor::Error>>
        + rkyv::Deserialize<E, rkyv::rancor::Strategy<rkyv::de::Pool, rkyv::rancor::Error>>,
{
    /// Discriminant byte synthesised into the archive at the
    /// empirically-pinned offset. Must be outside every envelope's
    /// `known_discriminants()` slice — `99` is high enough to outlive
    /// any plausible variant-count growth without growing the test's
    /// vocabulary.
    const UNKNOWN_TAG: u8 = 99;

    // -----------------------------------------------------------------
    // Pin 1 — valid bytes round-trip through decode_envelope_bytes.
    // -----------------------------------------------------------------
    // If `known_discriminants()` is mutated to omit the V1 tag (e.g.
    // returns `&[]` or `&[1]`), `probe_known_variant` rejects VALID
    // bytes (tag 0) as `UnknownVersion`. This round-trip assertion
    // catches that class of mutation. The `canonical` argument is
    // moved here; the equality assertion below uses the freshly
    // decoded value against `canonical_clone` to keep the pin-2
    // synthesis path independent of move semantics.
    let canonical_clone = canonical.clone();
    let envelope = E::latest(canonical);
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope)
        .expect("rkyv archive of canonical payload must succeed");
    let decoded = decode_envelope_bytes::<E>(bytes.as_ref()).unwrap_or_else(|err| {
        panic!(
            "decode_envelope_bytes::<{expected_type_name}> must succeed on canonical bytes; got {err:?}",
        )
    });
    assert_eq!(
        decoded, canonical_clone,
        "canonical payload must round-trip bit-equivalent through decode_envelope_bytes",
    );

    // -----------------------------------------------------------------
    // Pin 2 — synthesised unknown-tag bytes surface UnknownVersion.
    // -----------------------------------------------------------------
    // Flip the discriminant byte at the pinned offset from V1 (0) to
    // UNKNOWN_TAG (99). The structural sanity of the slice is
    // preserved (length, all other bytes); only the tag is unknown.
    let offset_from_end = E::discriminant_offset_from_end().unwrap_or_else(|| {
        panic!(
            "{expected_type_name} must override discriminant_offset_from_end() with the \
             empirically-pinned offset — None means the probe is a no-op for this envelope, \
             and the UnknownVersion classification is unreachable",
        )
    });
    let mut synthesised = bytes.as_ref().to_vec();
    let n = synthesised.len();
    assert!(
        n >= offset_from_end,
        "archived bytes ({n}) must be at least the discriminant offset ({offset_from_end}) long \
         — without that, the probe's structural sanity check returns Ok(()) and falls through \
         to bytecheck, surfacing as Malformed instead of UnknownVersion",
    );
    let target = n - offset_from_end;
    synthesised[target] = UNKNOWN_TAG;

    let err = decode_envelope_bytes::<E>(&synthesised).err().unwrap_or_else(|| {
        panic!(
            "decode_envelope_bytes::<{expected_type_name}> must surface an error on \
             unknown-tag bytes (tag {UNKNOWN_TAG} at offset {offset_from_end} from end); got Ok",
        )
    });
    match err {
        EnvelopeError::UnknownVersion { observed, type_name, supported_max } => {
            assert_eq!(
                observed, UNKNOWN_TAG,
                "probe must surface the observed discriminant byte verbatim — \
                 either the probe is reading the wrong offset or the synthesis is off",
            );
            assert_eq!(
                type_name, expected_type_name,
                "probe must name the envelope whose decode path surfaced the unknown tag \
                 (asserts {expected_type_name}::type_name() has not drifted)",
            );
            assert_eq!(
                supported_max, expected_supported_max,
                "probe must surface the highest known tag (asserts \
                 {expected_type_name}::known_discriminants() has not drifted)",
            );
        }
        other @ EnvelopeError::Malformed { .. } => panic!(
            "decode_envelope_bytes::<{expected_type_name}> must surface UnknownVersion on \
             unknown-tag bytes (tag {UNKNOWN_TAG} at offset {offset_from_end} from end); got \
             Malformed instead — either probe_known_variant short-circuited to Ok(()) or the \
             discriminant_offset_from_end is wrong, causing the probe to inspect padding \
             bytes that happen to be in known_discriminants. Got: {other:?}",
        ),
    }
}

// ---------------------------------------------------------------------
// Self-test — exercises the primitive against a private mock envelope.
// Per `.claude/rules/testing.md` § "Property-based testing" → "Archive
// schema-evolution roundtrip", the harness primitive must itself be
// covered by a roundtrip test. The mock envelope lives inline so the
// self-test doesn't depend on any per-domain envelope being implemented.
// ---------------------------------------------------------------------

#[cfg(test)]
mod harness_self_test {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
    enum MockEnvelope {
        V1(MockV1),
    }

    #[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
    struct MockV1 {
        value: u32,
        label: String,
    }

    impl VersionedEnvelope for MockEnvelope {
        type Latest = MockV1;

        fn latest(payload: Self::Latest) -> Self {
            Self::V1(payload)
        }

        fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
            match self {
                Self::V1(v1) => Ok(v1),
            }
        }

        fn type_name() -> &'static str {
            "MockEnvelope"
        }
    }

    #[test]
    fn harness_self_test_roundtrips_mock_envelope() {
        let expected = MockV1 { value: 42, label: "fixture".to_string() };
        let envelope = MockEnvelope::latest(expected.clone());
        let bytes =
            rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive should succeed");
        let fixture_hex = hex::encode(bytes.as_ref());

        assert_envelope_v_roundtrip::<MockEnvelope>(&fixture_hex, &expected);
    }

    // The new `assert_unknown_version_probe_surfaces` helper does not
    // have a `MockEnvelope` self-test by design — the probe behavior
    // depends on `discriminant_offset_from_end` returning `Some(N)`
    // with `N` empirically pinned per envelope shape, which the inline
    // `MockEnvelope` here does not provide. The helper's correctness
    // is cross-validated by the 5 per-envelope call sites
    // (`tests/schema_evolution/{alloc_status_row,job,node_health_row,
    // service_backend_row,service_hydration_result_row}.rs`) — every
    // edit to the helper that regresses any one envelope fails its
    // dedicated `*_unknown_version_probe_surfaces` test. Adding a
    // tag-dedicated mock would either duplicate the per-envelope
    // pinning rationale or test a non-production code path.
}
