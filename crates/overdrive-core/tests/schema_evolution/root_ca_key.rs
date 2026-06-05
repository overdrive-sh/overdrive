//! Schema-evolution golden-bytes test — `RootCaKeyEnvelope` (built-in-ca / GH #28).
//!
//! S-EV-CA-01. Pins the V1 archived layout of the `RootCaKeyRecord`
//! envelope (ADR-0063 D2 + D4) so any future commit that appends a field
//! to the V1 payload — rather than minting a `V2` — breaks this test and
//! signals the schema-evolution violation per ADR-0048 § 1 and
//! `.claude/rules/testing.md` § "Archive schema-evolution roundtrip".
//!
//! The V1 payload (`RootCaKeyRecordV1`, ADR-0063 D4) persists the AEAD
//! *inputs* (never derived state): `kek_id`, HKDF `salt` + `info`,
//! AES-GCM `nonce`, sealed `ciphertext` (root key DER), and `aead_tag`.
//!
//! **`FIXTURE_V1` is never touched** once minted. Bumping to `V2` adds a
//! new `FIXTURE_V2` constant + a new assertion in the same commit;
//! existing constants stay verbatim (`development.md` § "Version-bump
//! procedure").
//!
//! Default lane (no `integration-tests` feature) — pure in-memory rkyv,
//! no I/O.
//!
//! RED scaffold (DISTILL): the `RootCaKeyEnvelope` type + its
//! empirically-pinned `discriminant_offset_from_end` do not exist yet.
//! DELIVER mints `RootCaKeyRecordV1`, generates `FIXTURE_V1` from a
//! hand-pinned canonical payload, pins the discriminant offset via byte
//! flip (mirror `alloc_status_row.rs`), and replaces this scaffold with
//! the real `assert_envelope_v_roundtrip` / triangulation assertions.

/// `@property` `@S-02` — golden-bytes roundtrip: hex-decode `FIXTURE_V1`,
/// rkyv-deserialise into `RootCaKeyEnvelope`, `into_latest()`, assert
/// equality against the canonical `Latest` projection; the archived layout
/// is byte-stable.
///
/// DELIVER mirror: `super::harness::assert_envelope_v_roundtrip::<RootCaKeyEnvelope>(FIXTURE_V1, &canonical)`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn root_ca_key_envelope_v1_golden_bytes_roundtrip() {
    panic!(
        "Not yet implemented -- RED scaffold (S-02 / RootCaKeyEnvelope FIXTURE_V1 golden-bytes \
         roundtrip + canonical Latest projection equality, mirror schema_evolution/alloc_status_row.rs)"
    );
}

/// `@property` `@S-02` — discriminant-offset triangulation: an independent
/// pin of the V1 discriminant offset must agree with
/// `RootCaKeyEnvelope::discriminant_offset_from_end()` (two-source guard
/// against unilateral drift of either pin, per ADR-0048).
#[test]
#[should_panic(expected = "RED scaffold")]
fn root_ca_key_envelope_discriminant_offset_triangulates() {
    panic!(
        "Not yet implemented -- RED scaffold (S-02 / RootCaKeyEnvelope discriminant offset \
         triangulation against discriminant_offset_from_end())"
    );
}

/// `@property` `@S-02` `@error` — an unknown/forward envelope version
/// surfaces `EnvelopeError` via `probe_known_variant` rather than decoding
/// into garbage (intent fail-fast precursor: the `IntentStore` decode path
/// emits `health.startup.refused` on this error).
#[test]
#[should_panic(expected = "RED scaffold")]
fn root_ca_key_envelope_unknown_version_probe_surfaces_error() {
    panic!(
        "Not yet implemented -- RED scaffold (S-02 / RootCaKeyEnvelope unknown-version probe \
         surfaces EnvelopeError, not a silent garbage decode)"
    );
}
