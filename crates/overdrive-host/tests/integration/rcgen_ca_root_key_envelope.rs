//! Integration — root-key AEAD envelope (DISTILL RED scaffolds, built-in-ca / GH #28).
//!
//! Layer 3 (real HKDF-SHA256 + AES-256-GCM via the crypto backend; gated
//! `integration-tests`, runs via Lima). Exercises the root-key protection
//! scheme (ADR-0063 D4): HKDF-SHA256-derive a per-use subkey from the
//! keyring KEK, then AES-256-GCM-seal the root private key DER. The sealed
//! record (`RootCaKeyRecordV1`) carries the AEAD *inputs* (`kek_id`, salt,
//! info, nonce, ciphertext, `aead_tag`) — never derived state (D2).
//!
//! This is where KPI K3 (root key never plaintext at rest) is PROVEN by
//! byte-scanning the sealed bytes. Per Mandate 11 these layer-3 tests are
//! EXAMPLE-ONLY (one example per AEAD behaviour / failure mode); the
//! tampered-vs-wrong-KEK distinction is enumerated explicitly, not
//! PBT-generated.
//!
//! Scenarios trace to US-CA-02 (envelope round-trip, no plaintext key,
//! tamper detection, wrong-KEK distinction). Tags: `@real-io`
//! `@adapter-integration` `@S-02` (round-trip) · `+ @error` (failure paths).
//!
//! RED scaffold convention: self-contained `panic!` under
//! `#[should_panic(expected = "RED scaffold")]`; no import of unbuilt
//! envelope codec. DELIVER replaces with real seal/open + byte-scan.

/// `@real-io` `@adapter-integration` `@S-02` — envelope round-trip: seal a
/// root private key under a KEK via HKDF->AES-256-GCM, then open it with the
/// SAME KEK; the recovered key DER is byte-identical to the original.
#[test]
#[should_panic(expected = "RED scaffold")]
fn root_key_envelope_seals_and_opens_round_trip_under_same_kek() {
    panic!(
        "Not yet implemented -- RED scaffold (S-02 / HKDF->AES-256-GCM seal then open under the \
         same KEK recovers byte-identical root key DER)"
    );
}

/// `@real-io` `@adapter-integration` `@S-02` — KPI K3 (guardrail): the
/// sealed `RootCaKeyRecordV1` bytes (and the `IntentStore` blob that wraps
/// them) contain ZERO plaintext private-key bytes. Byte-scan the serialized
/// record for the known plaintext key DER and assert absence.
#[test]
#[should_panic(expected = "RED scaffold")]
fn root_key_envelope_contains_no_plaintext_key_bytes() {
    panic!(
        "Not yet implemented -- RED scaffold (S-02 / KPI K3: sealed RootCaKeyRecord bytes contain \
         0 plaintext private-key bytes; byte-scan the serialized record asserts absence)"
    );
}

/// `@real-io` `@adapter-integration` `@S-02` `@error` — AEAD tamper
/// detection: flipping a byte in the sealed ciphertext makes AES-GCM open
/// FAIL with a `CaError` naming a corrupt/tampered envelope, DISTINCT from
/// the wrong-KEK error variant (the GCM auth tag distinguishes them). Sad
/// path, example-based.
#[test]
#[should_panic(expected = "RED scaffold")]
fn root_key_envelope_tampered_ciphertext_fails_distinct_from_wrong_kek() {
    panic!(
        "Not yet implemented -- RED scaffold (S-02 / a one-byte flip in the sealed ciphertext \
         fails AES-GCM open with a corrupt/tampered-envelope CaError, distinct from wrong-KEK)"
    );
}

/// `@real-io` `@adapter-integration` `@S-02` `@error` — wrong-KEK path:
/// opening the sealed record with a DIFFERENT KEK fails with a wrong-KEK
/// `CaError` variant (distinct from tampered-envelope). AAD = `kek_id` binds
/// the ciphertext to the KEK identity (defends KEK-confusion). Sad path.
#[test]
#[should_panic(expected = "RED scaffold")]
fn root_key_envelope_wrong_kek_fails_distinct_from_tampered() {
    panic!(
        "Not yet implemented -- RED scaffold (S-02 / opening the sealed record under a different \
         KEK fails with a wrong-KEK CaError variant, distinct from tampered-envelope)"
    );
}
