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

use overdrive_core::codec::VersionedEnvelope;

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
    use overdrive_core::codec::EnvelopeError;

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
}
