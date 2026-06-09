//! Integration — `overdrive alloc status` surfaces the current
//! `issued_certificates` summary per `built-in-ca-operator-composition`
//! Slice ③ (folds GH #215 consumer-side). DISTILL RED scaffolds.
//!
//! Layer 3 (real `LocalObservationStore`; the `overdrive alloc status` CLI is
//! the driving adapter, run as a real subprocess in Lima). Per Mandate 11
//! example-only; no PBT machinery.
//!
//! Settled design (feature-delta.md D-OC-7; ADR-0063 D6; ADR-0067
//! #215-boundary): `AllocStatusResponse` gains an additive
//! `issued_certificates: Vec<IssuedCertSummary>` field
//! (`skip_serializing_if = "Vec::is_empty"`). `IssuedCertSummary { serial,
//! spiffe_id, issuer_serial, not_after }` carries NO cert bytes and NO key. The
//! server reads `obs.issued_certificate_rows()` and projects, per running
//! alloc, the LATEST-by-`issued_at` row whose `spiffe_id ==
//! SpiffeId::for_allocation(workload_id, alloc_id)`. The CLI renders
//! `serial / spiffe_id / issuer_serial / not_after`.
//!
//! O05 ≠ E03 (feature-delta.md § E03/O05 split): these scenarios capture EDD
//! O05 (operator-legible audit metadata). They do NOT and CANNOT prove the chain
//! verifies — that is E03's exported-PEM `openssl verify` path
//! (`rcgen_ca_chain_verify.rs` + the E03 runner). The summary render MUST NOT be
//! treated as satisfying E03.
//!
//! RED scaffold convention: `#[ignore]` — the blocker is the
//! `AllocStatusResponse.issued_certificates` field + server aggregation + CLI
//! render (Slice ③) do not exist yet AND the surface is the `overdrive alloc
//! status` subprocess (Lima). DELIVER removes `#[ignore]` and lands real
//! assertions.

#![allow(clippy::expect_used, clippy::unwrap_used)]

// S-OC-11 `@integration @real-io @adapter-integration @driving_port @slice-3
// @edd:O05` — after the platform has issued an SVID for a deployed RUNNING
// workload, `overdrive alloc status --job <id>` surfaces an issued-certificate
// summary for the running allocation showing serial / spiffe_id / issuer_serial
// / not_after, and the surfaced serial matches the minted certificate's serial.
// EDD O05 sub-claims 1+2. Universe: the rendered `alloc status` output (the
// issued-certificate section + its fields) + the cross-checked minted serial.
#[test]
#[ignore = "blocked on Slice 3 — AllocStatusResponse.issued_certificates aggregation + CLI render (Lima)"]
fn alloc_status_surfaces_current_issued_certificate_summary() {
    panic!(
        "Not yet implemented -- RED scaffold (S-OC-11 / overdrive alloc status surfaces the \
         current issued_certificates summary: serial / spiffe_id / issuer_serial / not_after, \
         serial matches the minted cert -- EDD O05)"
    );
}

// S-OC-12 `@integration @real-io @error @driving_port @slice-3 @edd:O05` — with
// MULTIPLE `issued_certificates` rows for a running alloc (first issue + a
// re-mint), the summary renders EXACTLY the latest-by-`issued_at` row per
// running alloc (NOT history); the summary contains NO certificate PEM/DER bytes
// and NO private key; a post-restart serial change reads as the current cert,
// not an anomaly. Guards the no-leak invariant (ADR-0067 #215-boundary).
// Universe: the rendered issued-certificate section (exactly one row per running
// alloc, the latest) + the ABSENCE of cert-bytes/key tokens in the output.
#[test]
#[ignore = "blocked on Slice 3 — latest-by-issued_at projection + no-cert-bytes render (Lima)"]
fn issued_certificate_summary_omits_cert_bytes_and_key_latest_by_issued_at() {
    panic!(
        "Not yet implemented -- RED scaffold (S-OC-12 / the summary renders the latest-by-issued_at \
         row per running alloc and carries NO cert bytes / NO key -- EDD O05, #215-boundary)"
    );
}
