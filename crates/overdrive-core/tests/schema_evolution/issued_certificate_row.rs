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
//! `node_id`, `issued_at`. These are the audit *inputs* (what was issued) —
//! observation, never intent (the CA *material* is intent, D2). The
//! unknown-version read path **logs-and-skips** the row (observation
//! TOLERATES; asymmetric vs intent's fail-fast at 02-01).
//!
//! **`FIXTURE_V1` is never touched** once minted (`development.md`
//! § "Version-bump procedure").
//!
//! Default lane — pure in-memory rkyv, no I/O.

use overdrive_core::ca::issued_certificate_row::{
    IssuedCertificateRowEnvelope, IssuedCertificateRowLatest, IssuedCertificateRowV1,
};
use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::id::{CertSerial, IssuanceOrdinal, NodeId, SpiffeId};
use overdrive_core::wall_clock::UnixInstant;

use super::harness::{
    assert_discriminant_offset_triangulation, assert_envelope_v_roundtrip,
    assert_unknown_version_probe_surfaces,
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
fn canonical_v1_payload() -> IssuedCertificateRowLatest {
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

/// `@property` `@S-05` (S-05-06) — golden-bytes roundtrip for the
/// issued-cert audit row envelope: hex-decode `FIXTURE_V1`,
/// rkyv-deserialise into `IssuedCertificateRowEnvelope`, `into_latest()`,
/// assert equality against the canonical `Latest` projection; the archived
/// layout is byte-stable.
#[test]
fn issued_certificate_row_envelope_v1_golden_bytes_roundtrip() {
    let expected = canonical_v1_payload();
    assert_envelope_v_roundtrip::<IssuedCertificateRowEnvelope>(FIXTURE_V1, &expected);
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
