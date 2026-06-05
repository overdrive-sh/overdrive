//! Schema-evolution golden-bytes test — `IssuedCertificateRowEnvelope` (built-in-ca / GH #28).
//!
//! S-EV-CA-02. Pins the V1 archived layout of the `issued_certificates`
//! observation-row envelope (ADR-0063 D6) so any future commit that appends
//! a field to V1 — rather than minting `V2` — breaks this test, per
//! ADR-0048 § 1 and `.claude/rules/testing.md` § "Archive schema-evolution
//! roundtrip". Mirrors `AllocStatusRow` / `NodeHealthRow` envelope shape.
//!
//! The V1 payload (`IssuedCertificateRowV1`, research Finding 15) carries:
//! `serial`, `spiffe_id`, `issuer_serial`, `not_before`, `not_after`,
//! `node_id`, `issued_at`. These are the audit *inputs* (what was issued) —
//! observation, never intent (the CA *material* is intent, D2).
//!
//! **`FIXTURE_V1` is never touched** once minted (`development.md`
//! § "Version-bump procedure").
//!
//! Default lane — pure in-memory rkyv, no I/O.
//!
//! RED scaffold (DISTILL): `IssuedCertificateRowEnvelope` does not exist
//! yet. DELIVER mints `IssuedCertificateRowV1`, generates `FIXTURE_V1`,
//! pins the discriminant offset via byte flip, and replaces these
//! scaffolds with the real assertions.

/// `@property` `@S-05` — golden-bytes roundtrip for the issued-cert audit
/// row envelope: hex-decode `FIXTURE_V1`, rkyv-deserialise, `into_latest()`,
/// assert canonical `Latest` equality. Archived layout byte-stable.
#[test]
#[should_panic(expected = "RED scaffold")]
fn issued_certificate_row_envelope_v1_golden_bytes_roundtrip() {
    panic!(
        "Not yet implemented -- RED scaffold (S-05 / IssuedCertificateRowEnvelope FIXTURE_V1 \
         golden-bytes roundtrip, mirror schema_evolution/alloc_status_row.rs)"
    );
}

/// `@property` `@S-05` — discriminant-offset triangulation against
/// `IssuedCertificateRowEnvelope::discriminant_offset_from_end()`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn issued_certificate_row_envelope_discriminant_offset_triangulates() {
    panic!(
        "Not yet implemented -- RED scaffold (S-05 / IssuedCertificateRowEnvelope discriminant \
         offset triangulation against discriminant_offset_from_end())"
    );
}

/// `@property` `@S-05` `@error` — unknown/forward version surfaces
/// `EnvelopeError` (observation decode path logs + skips the row;
/// convergence proceeds for surviving rows, per ADR-0048 asymmetric
/// unknown-handling — observation tolerates, intent refuses).
#[test]
#[should_panic(expected = "RED scaffold")]
fn issued_certificate_row_envelope_unknown_version_probe_surfaces_error() {
    panic!(
        "Not yet implemented -- RED scaffold (S-05 / IssuedCertificateRowEnvelope unknown-version \
         probe surfaces EnvelopeError; observation log-and-skip, not a garbage decode)"
    );
}
