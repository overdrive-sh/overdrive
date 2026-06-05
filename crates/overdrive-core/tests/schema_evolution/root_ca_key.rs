//! Schema-evolution golden-bytes test — `RootCaKeyEnvelope` (built-in-ca / GH #28).
//!
//! S-EV-CA-01 (scenarios S-02-08 / S-02-09 / S-02-10). Pins the V1
//! archived layout of the `RootCaKeyRecord` envelope (ADR-0063 D2 + D4)
//! so any future commit that appends a field to the V1 payload — rather
//! than minting a `V2` — breaks this test and signals the
//! schema-evolution violation per ADR-0048 § 1 and
//! `.claude/rules/testing.md` § "Archive schema-evolution roundtrip".
//!
//! The V1 payload (`RootCaKeyRecordV1`, ADR-0063 D4) persists the AEAD
//! *inputs* (never derived state): `kek_id`, HKDF `salt` + `info`,
//! AES-GCM `nonce`, sealed `ciphertext` (root key DER), and `aead_tag`.
//! There is no decoded/plaintext key anywhere in the record — that is
//! the K3 zero-plaintext-at-rest guardrail this shape enables.
//!
//! **`FIXTURE_V1` is never touched** once minted. Bumping to `V2` adds a
//! new `FIXTURE_V2` constant + a new assertion in the same commit;
//! existing constants stay verbatim (`development.md` § "Version-bump
//! procedure").
//!
//! Default lane (no `integration-tests` feature) — pure in-memory rkyv,
//! no I/O.

use overdrive_core::ca::root_key_envelope::{
    KekId, RootCaKeyEnvelope, RootCaKeyRecordLatest, RootCaKeyRecordV1,
};
use overdrive_core::codec::VersionedEnvelope;

use super::harness::{
    assert_discriminant_offset_triangulation, assert_envelope_v_roundtrip,
    assert_unknown_version_probe_surfaces,
};

/// Independent pin of the V1 discriminant offset for triangulation
/// against `RootCaKeyEnvelope::discriminant_offset_from_end()`. The
/// two-source triangulation (this constant vs the trait method) guards
/// against unilateral drift of either pin per ADR-0048; on a `V<N+1>`
/// bump BOTH must update in the same commit. Empirically pinned by
/// regenerating `FIXTURE_V1` and locating the trailing-root discriminant
/// byte (mirror `alloc_status_row.rs::GOLDEN_DISCRIMINANT_OFFSET_V1`).
const GOLDEN_DISCRIMINANT_OFFSET_V1: usize = 52;

/// Canonical V1 payload pinned by `FIXTURE_V1` below. The expected
/// projection is built from these values verbatim — change any one of
/// them and the test fails until `FIXTURE_V1` is regenerated.
fn canonical_v1_payload() -> RootCaKeyRecordLatest {
    RootCaKeyRecordV1 {
        kek_id: KekId::new("kek-root-01").expect("valid kek id"),
        salt: vec![0x01, 0x02, 0x03, 0x04],
        info: b"overdrive-root-ca-v1".to_vec(),
        nonce: vec![0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b],
        ciphertext: vec![0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
        aead_tag: vec![
            0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d,
            0x2e, 0x2f,
        ],
    }
}

/// Hex-encoded rkyv-archived bytes of
/// `RootCaKeyEnvelope::V1(canonical_v1_payload())`.
///
/// Minted by `print_fixture_v1_bytes` (below) once the payload type
/// compiled. Pre-shipment regeneration is allowed under
/// `feedback_single_cut_greenfield_migrations.md`; once V1 ships to a
/// deployed consumer, this constant becomes immutable per
/// `.claude/rules/development.md` § "rkyv schema evolution" — future
/// variants need a `V2` envelope.
const FIXTURE_V1: &str = "6b656b2d726f6f742d3031010203046f76657264726976652d726f6f742d63612d7631101112131415161718191a1baabbccddeeff202122232425262728292a2b2c2d2e2f000000000000008b000000b4ffffffb7ffffff04000000b3ffffff14000000bfffffff0c000000c3ffffff06000000c1ffffff10000000";

/// `@property` `@S-02` (S-02-08) — golden-bytes roundtrip: hex-decode
/// `FIXTURE_V1`, rkyv-deserialise into `RootCaKeyEnvelope`,
/// `into_latest()`, assert equality against the canonical `Latest`
/// projection; the archived layout is byte-stable.
#[test]
fn root_ca_key_envelope_v1_golden_bytes_roundtrip() {
    let expected = canonical_v1_payload();
    assert_envelope_v_roundtrip::<RootCaKeyEnvelope>(FIXTURE_V1, &expected);
}

/// `@property` `@S-02` (S-02-09) — discriminant-offset triangulation: an
/// independent pin of the V1 discriminant offset must agree with
/// `RootCaKeyEnvelope::discriminant_offset_from_end()` (two-source guard
/// against unilateral drift of either pin, per ADR-0048).
#[test]
fn root_ca_key_envelope_discriminant_offset_triangulates() {
    assert_discriminant_offset_triangulation::<RootCaKeyEnvelope>(
        canonical_v1_payload(),
        GOLDEN_DISCRIMINANT_OFFSET_V1,
        0,
    );
}

/// `@property` `@S-02` `@error` (S-02-10) — an unknown/forward envelope
/// version surfaces `EnvelopeError` via `probe_known_variant` rather than
/// decoding into garbage (intent fail-fast precursor: the `IntentStore`
/// decode path emits `health.startup.refused` on this error).
///
/// `supported_max == 0` because today's envelope is V1-only; bumping to
/// V2 means re-pinning this assertion in the same commit per
/// `development.md` § "Version-bump procedure".
#[test]
fn root_ca_key_envelope_unknown_version_probe_surfaces_error() {
    assert_unknown_version_probe_surfaces::<RootCaKeyEnvelope>(
        canonical_v1_payload(),
        "RootCaKeyEnvelope",
        0,
    );
}

// ---------------------------------------------------------------------
// Bootstrap helper — generates the canonical V1 bytes on demand for the
// crafter to paste into `FIXTURE_V1` above. Run via:
//
//   cargo nextest run -p overdrive-core --test schema_evolution \
//       -E 'test(/root_ca_key.*print_fixture_v1_bytes/)' --no-capture
//
// Marked `#[ignore]` so it never runs in normal test execution; the
// pinned `FIXTURE_V1` constant is the load-bearing artifact, this is a
// one-shot regeneration aid (mirror alloc_status_row.rs).
// ---------------------------------------------------------------------

#[test]
#[ignore = "fixture regeneration tool — run on demand when bumping a payload variant; the pinned FIXTURE_V<N> constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_V1"
)]
fn print_fixture_v1_bytes() {
    let envelope = RootCaKeyEnvelope::latest(canonical_v1_payload());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("FIXTURE_V1 = \"{}\"", hex::encode(bytes.as_ref()));
}
