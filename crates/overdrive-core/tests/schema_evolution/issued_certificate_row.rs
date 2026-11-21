//! Schema-evolution golden-bytes test — `IssuedCertificateRowEnvelope` (built-in-ca / GH #28).
//!
//! S-EV-CA-02 (scenarios S-05-06 / S-05-07 / S-05-08). Pins the V1
//! archived layout of the `issued_certificates` observation-row envelope
//! (ADR-0063 D6) so any future commit that appends a field to V1 — rather
//! than minting `V2` — breaks this test, per ADR-0048 § 1 and
//! `.claude/rules/testing.md` § "Archive schema-evolution roundtrip".
//! Mirrors the `AllocStatusRow` / `NodeHealthRow` envelope shape.
//!
//! The V1 payload (`IssuedCertificateRowV1`, research Finding 15) carries:
//! `serial`, `spiffe_id`, `issuer_serial`, `not_before`, `not_after`,
//! `node_id`, `issued_at`, `issuance_ordinal`. These are the audit *inputs*
//! (what was issued) —
//! observation, never intent (the CA *material* is intent, D2). The
//! unknown-version read path **logs-and-skips** the row (observation
//! TOLERATES; asymmetric vs intent's fail-fast at 02-01).
//!
//! **`FIXTURE_V1` is never touched** once minted (`development.md`
//! § "Version-bump procedure").
//!
//! Default lane — pure in-memory rkyv, no I/O.

use overdrive_core::ca::issued_certificate_row::{
    IssuedCertificateRow, IssuedCertificateRowEnvelope, IssuedCertificateRowV1,
};
use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::id::{CertSerial, IssuanceOrdinal, NodeId, SpiffeId};
use overdrive_core::traits::observation_store::ObservationStoreError;
use overdrive_core::wall_clock::UnixInstant;

use super::harness::{
    assert_discriminant_offset_triangulation, assert_unknown_version_probe_surfaces,
};

/// Independent pin of the V1 discriminant offset for triangulation
/// against `IssuedCertificateRowEnvelope::discriminant_offset_from_end()`.
/// The two-source triangulation (this constant vs the trait method) guards
/// against unilateral drift of either pin per ADR-0048; on a `V<N+1>` bump
/// BOTH must update in the same commit. Empirically pinned by regenerating
/// `FIXTURE_V1` and locating the trailing-root discriminant byte (mirror
/// `root_ca_key.rs::GOLDEN_DISCRIMINANT_OFFSET_V1`).
const GOLDEN_DISCRIMINANT_OFFSET_V1: usize = 104;

/// Canonical V1 payload pinned by `FIXTURE_V1` below. The expected
/// projection is built from these values verbatim — change any one of them
/// and the test fails until `FIXTURE_V1` is regenerated.
fn canonical_v1_payload() -> IssuedCertificateRowV1 {
    IssuedCertificateRowV1 {
        serial: CertSerial::new("0a1b2c3d4e5f").expect("valid cert serial"),
        spiffe_id: SpiffeId::new("spiffe://overdrive.test/node/node-01/workload/dns-resolver")
            .expect("valid spiffe id"),
        issuer_serial: CertSerial::new("ffeeddccbbaa").expect("valid issuer serial"),
        not_before: UnixInstant::from_unix_duration(std::time::Duration::from_secs(1_700_000_000)),
        not_after: UnixInstant::from_unix_duration(std::time::Duration::from_secs(1_700_086_400)),
        node_id: NodeId::new("node-01").expect("valid node id"),
        issued_at: UnixInstant::from_unix_duration(std::time::Duration::from_secs(1_700_000_005)),
        issuance_ordinal: IssuanceOrdinal::new(7),
    }
}

/// Hex-encoded rkyv-archived bytes of
/// `IssuedCertificateRowEnvelope::V1(canonical_v1_payload())`.
///
/// Minted by `print_fixture_v1_bytes` (below) once the payload type
/// compiled. Pre-shipment regeneration is allowed under
/// `feedback_single_cut_greenfield_migrations.md`; once V1 ships to a
/// deployed consumer, this constant becomes immutable per
/// `.claude/rules/development.md` § "rkyv schema evolution" — future
/// variants need a `V2` envelope.
const FIXTURE_V1: &str = "3061316232633364346535667370696666653a2f2f6f76657264726976652e746573742f6e6f64652f6e6f64652d30312f776f726b6c6f61642f646e732d7265736f6c76657266666565646463636262616100000000000000000000000000008c000000a0ffffffba000000a4ffffff170000008c000000d2ffffff0000000000f15365000000000000000000000000804255650000000000000000000000006e6f64652d3031ff05f153650000000000000000000000000700000000000000";

/// V1 payload archived under the SECOND-variant envelope root: the explicit
/// `Envelope::V1(frozen V1 payload)` encoding (never through `latest()` or the
/// alias, which would encode V2). Appending the larger V2 variant grows the
/// archive root, so the pre-V2 `FIXTURE_V1` literal no longer decodes (ADR-0345,
/// `development.md` "rkyv schema evolution"); this is that fixture regenerated.
/// Minted by encoding a two-variant envelope `{ V1(IssuedCertificateRowV1),
/// V2(IssuedCertificateRowV2) }` with the canonical V1 payload.
const FIXTURE_V1_UNDER_V2_ROOT: &str = "3061316232633364346535667370696666653a2f2f6f76657264726976652e746573742f6e6f64652f6e6f64652d30312f776f726b6c6f61642f646e732d7265736f6c76657266666565646463636262616100000000000000000000000000008c000000a0ffffffba000000a4ffffff170000008c000000d2ffffff0000000000f15365000000000000000000000000804255650000000000000000000000006e6f64652d3031ff05f15365000000000000000000000000070000000000000000000000000000000000000000000000";

/// The canonical V2 payload pinned by `FIXTURE_V2`: the canonical V1 fields plus
/// `issued_for = Allocation("alloc-dns-resolver-0")`.
const FIXTURE_V2: &str = "3061316232633364346535667370696666653a2f2f6f76657264726976652e746573742f6e6f64652f6e6f64652d30312f776f726b6c6f61642f646e732d7265736f6c766572666665656464636362626161616c6c6f632d646e732d7265736f6c7665722d30000001000000000000008c00000090ffffffba00000094ffffff170000008c000000c2ffffff0000000000f15365000000000000000000000000804255650000000000000000000000006e6f64652d3031ff05f15365000000000000000000000000070000000000000000000000940000007effffff00000000";

fn decoded(hex_bytes: &str) -> Result<IssuedCertificateRow, ObservationStoreError> {
    let bytes = hex::decode(hex_bytes).expect("fixture is valid hex");
    IssuedCertificateRow::from_store_bytes(&bytes, Some("fixture"))
}

/// `@error` (ADR-0345, DD-5) — a V1 row is never translated: a row written by a
/// prior binary carries a per-allocation identity and no `issued_for`, and is
/// discarded with the data directory. Both the pre-V2 literal and the same V1
/// payload archived under the V2-sized root are refused with an
/// `EnvelopeError` (the pre-V2 literal fails to decode as the larger root, the
/// V2-rooted one is a V1 variant the latest projection refuses; the design pins
/// no finer variant), never decoded into the latest row. Observation reads log
/// and skip it (ADR-0048 §3).
///
/// `CONTRACT_SHAPE`: bounded-change.
#[test]
#[should_panic(expected = "a V1 issued-certificate row must be refused")]
fn issued_certificate_row_v1_rows_are_refused_and_never_translated() {
    let pre_v2 = decoded(FIXTURE_V1);
    assert!(pre_v2.is_err(), "a V1 issued-certificate row must be refused, it decoded: {pre_v2:?}");
    let under_v2_root = decoded(FIXTURE_V1_UNDER_V2_ROOT);
    assert!(
        matches!(under_v2_root, Err(ObservationStoreError::Envelope { .. })),
        "a V1 issued-certificate row must be refused with an envelope error, got: {under_v2_root:?}"
    );
}

/// `@property` (ADR-0345) — golden-bytes roundtrip of the V2 row: the pinned
/// archive decodes through the envelope and re-archives to the same octets, so
/// the persisted `issued_for` (here `Allocation("alloc-dns-resolver-0")`) is
/// part of the row and survives the roundtrip.
///
/// `CONTRACT_SHAPE`: pure-function.
#[test]
#[should_panic(expected = "the V2 issued-certificate row must decode")]
fn issued_certificate_row_v2_golden_bytes_roundtrip_preserves_issued_for() {
    let row = decoded(FIXTURE_V2).expect("the V2 issued-certificate row must decode");
    let again = row.archive_for_store().expect("the decoded row archives");
    assert_eq!(
        hex::encode(again.as_ref()),
        FIXTURE_V2,
        "the V2 row re-archives to the pinned octets"
    );
}

/// `@property` (ADR-0345, ADR-0048) — the envelope declares both versions and
/// the re-pinned discriminant offset (the larger V2 variant moves the
/// discriminant from the end of the archive: 104 -> 120, measured on the
/// two-variant archive of both variants).
///
/// `CONTRACT_SHAPE`: pure-function.
#[test]
#[should_panic(expected = "the envelope knows V1 and V2 at offset 120")]
fn issued_certificate_row_envelope_declares_v1_and_v2_with_the_repinned_offset() {
    assert_eq!(
        (
            IssuedCertificateRowEnvelope::known_discriminants(),
            IssuedCertificateRowEnvelope::discriminant_offset_from_end()
        ),
        (&[0u8, 1][..], Some(120)),
        "the envelope knows V1 and V2 at offset 120"
    );
}

/// `@property` `@S-05` (S-05-07) — discriminant-offset triangulation: an
/// independent pin of the V1 discriminant offset must agree with
/// `IssuedCertificateRowEnvelope::discriminant_offset_from_end()`
/// (two-source guard against unilateral drift of either pin, per ADR-0048).
#[test]
fn issued_certificate_row_envelope_discriminant_offset_triangulates() {
    assert_discriminant_offset_triangulation::<IssuedCertificateRowEnvelope>(
        canonical_v1_payload(),
        GOLDEN_DISCRIMINANT_OFFSET_V1,
        0,
    );
}

/// `@property` `@S-05` `@error` (S-05-08) — an unknown/forward envelope
/// version surfaces `EnvelopeError` via `probe_known_variant` rather than
/// decoding into garbage. This is the OBSERVATION read path: the
/// observation decode site logs-and-skips the row (convergence proceeds for
/// surviving rows), asymmetric vs intent's fail-fast (02-01). The probe
/// surface asserted here is the shared primitive both policies consume.
///
/// `supported_max == 0` because today's envelope is V1-only; bumping to V2
/// means re-pinning this assertion in the same commit per `development.md`
/// § "Version-bump procedure".
#[test]
fn issued_certificate_row_envelope_unknown_version_probe_surfaces_error() {
    assert_unknown_version_probe_surfaces::<IssuedCertificateRowEnvelope>(
        canonical_v1_payload(),
        "IssuedCertificateRowEnvelope",
        0,
    );
}

// ---------------------------------------------------------------------
// Bootstrap helper — generates the canonical V1 bytes on demand for the
// crafter to paste into `FIXTURE_V1` above. Run via:
//
//   cargo nextest run -p overdrive-core --test schema_evolution \
//       -E 'test(/issued_certificate_row.*print_fixture_v1_bytes/)' --no-capture
//
// Marked `#[ignore]` so it never runs in normal test execution; the
// pinned `FIXTURE_V1` constant is the load-bearing artifact, this is a
// one-shot regeneration aid (mirror root_ca_key.rs).
// ---------------------------------------------------------------------

#[test]
#[ignore = "fixture regeneration tool — run on demand when bumping a payload variant; the pinned FIXTURE_V<N> constants are the load-bearing artifact"]
#[allow(
    clippy::print_stdout,
    reason = "fixture regeneration tool emits hex to stdout for the human to paste into FIXTURE_V1"
)]
fn print_fixture_v1_bytes() {
    let envelope = IssuedCertificateRowEnvelope::latest(canonical_v1_payload());
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).expect("rkyv archive");
    println!("FIXTURE_V1 = \"{}\"", hex::encode(bytes.as_ref()));
}
