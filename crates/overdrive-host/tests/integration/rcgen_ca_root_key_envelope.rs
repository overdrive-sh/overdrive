//! Integration — root-key AEAD envelope (built-in-ca / GH #28, ADR-0063 D4).
//!
//! Layer 3 (real HKDF-SHA256 + AES-256-GCM via `ring`; gated
//! `integration-tests`, runs via Lima). Exercises the root-key protection
//! scheme (ADR-0063 D4): HKDF-SHA256-derive a per-use subkey from the KEK,
//! then AES-256-GCM-seal the root private key DER. The sealed record
//! (`RootCaKeyRecordV1`) carries the AEAD *inputs* (`kek_id`, salt, info,
//! nonce, ciphertext, `aead_tag`) — never derived state (D2).
//!
//! This is where KPI K3 (root key never plaintext at rest) is PROVEN by
//! byte-scanning the sealed bytes. Per Mandate 11 these layer-3 tests are
//! EXAMPLE-ONLY (one example per AEAD behaviour / failure mode); the
//! tampered-vs-wrong-KEK distinction is enumerated explicitly, not
//! PBT-generated.
//!
//! Port-to-port litmus: every test drives the `RootKeyAeadCodec` seal/open
//! driving port; delete the codec call-site and the test stays RED.
//!
//! Scenarios trace to US-CA-02 (envelope round-trip, no plaintext key,
//! tamper detection, wrong-KEK distinction). Tags: `@real-io`
//! `@adapter-integration` `@S-02` (round-trip) · `+ @error` (failure paths).

use std::collections::HashMap;
use std::str::FromStr;

use overdrive_core::ca::kek::{KEK_LEN, Kek, KekError, KekMaterial};
use overdrive_core::ca::root_key_envelope::KekId;
use overdrive_core::traits::ca::CaError;
use overdrive_host::ca::RootKeyAeadCodec;
use rcgen::KeyPair;

/// In-memory [`Kek`] provider for the L3 codec tests — maps `KekId` → raw
/// 256-bit material. The production provider (`SystemdCredsKeyring` over the
/// Linux kernel keyring) is a later slice; this fixture is NOT a production
/// keyring impl. It enforces the real provider's precondition: an unregistered
/// id resolves to [`KekError::NotFound`], never a zero/default KEK.
struct FixtureKek {
    keys: HashMap<KekId, [u8; KEK_LEN]>,
}

impl FixtureKek {
    fn with(kek_id: &KekId, bytes: [u8; KEK_LEN]) -> Self {
        let mut keys = HashMap::new();
        keys.insert(kek_id.clone(), bytes);
        Self { keys }
    }

    fn and(mut self, kek_id: &KekId, bytes: [u8; KEK_LEN]) -> Self {
        self.keys.insert(kek_id.clone(), bytes);
        self
    }
}

impl Kek for FixtureKek {
    fn resolve(&self, kek_id: &KekId) -> Result<KekMaterial, KekError> {
        self.keys
            .get(kek_id)
            .map(|bytes| KekMaterial::new(*bytes))
            .ok_or_else(|| KekError::not_found(kek_id.clone()))
    }
}

/// Mint a real P-256 private-key DER (the actual root-key shape the codec
/// seals at rest), via the same `rcgen` backend `RcgenCa::root` uses.
fn real_root_key_der() -> Vec<u8> {
    KeyPair::generate().expect("generate P-256 keypair").serialize_der()
}

fn kek_id(raw: &str) -> KekId {
    KekId::from_str(raw).expect("valid kek id")
}

// ---------------------------------------------------------------------------
// S-02-01 — envelope round-trip under the same KEK (US-CA-02)
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-02` — envelope round-trip: seal a
/// root private key under a KEK via HKDF->AES-256-GCM, then open it with the
/// SAME KEK; the recovered key DER is byte-identical to the original.
#[test]
fn root_key_envelope_seals_and_opens_round_trip_under_same_kek() {
    // GIVEN a real P-256 root key DER and a KEK registered with the provider.
    let key_der = real_root_key_der();
    let id = kek_id("kek-root-01");
    let kek = FixtureKek::with(&id, [0x11; KEK_LEN]);
    let codec = RootKeyAeadCodec::new();

    // WHEN the key is sealed and then opened under the SAME KEK (driving port).
    let record = codec.seal(&kek, &id, &key_der).expect("seal succeeds under the KEK");
    let recovered = codec.open(&kek, &id, &record).expect("open succeeds under the same KEK");

    // THEN the recovered DER is byte-identical to the original.
    assert_eq!(recovered, key_der, "open recovers byte-identical root key DER");

    // AND the record bound itself to the KEK identity (the AAD).
    assert_eq!(&record.kek_id, &id, "record records the sealing KEK identity");
}

// ---------------------------------------------------------------------------
// S-02-02 — no plaintext key bytes at rest (US-CA-02, KPI K3)
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-02` — KPI K3 (guardrail): the
/// sealed `RootCaKeyRecordV1` bytes (and the `IntentStore` blob that wraps
/// them) contain ZERO plaintext private-key bytes. Byte-scan the serialized
/// record for the known plaintext key DER and assert absence.
#[test]
fn root_key_envelope_contains_no_plaintext_key_bytes() {
    // GIVEN a real root key DER sealed under a KEK.
    let key_der = real_root_key_der();
    let id = kek_id("kek-root-01");
    let kek = FixtureKek::with(&id, [0x22; KEK_LEN]);
    let codec = RootKeyAeadCodec::new();
    let record = codec.seal(&kek, &id, &key_der).expect("seal succeeds");

    // WHEN the record is serialized for the IntentStore (the persisted blob).
    let blob = record.archive_for_store().expect("archive record for store");

    // THEN neither the record's ciphertext field nor the full serialized blob
    // contains the plaintext key DER (fail-closed: any occurrence fails).
    assert!(
        !contains_subslice(&record.ciphertext, &key_der),
        "ciphertext field must not contain the plaintext key DER"
    );
    assert!(
        !contains_subslice(blob.as_slice(), &key_der),
        "serialized IntentStore blob must not contain the plaintext key DER (KPI K3)"
    );

    // Sanity: the scan would FIND the DER if it were present — guards against
    // a vacuous-pass scan (e.g. an empty needle or a broken matcher).
    assert!(
        contains_subslice(&key_der, &key_der),
        "byte-scan matcher must find the needle in itself (anti-vacuous-pass)"
    );
}

// ---------------------------------------------------------------------------
// S-02-03 — tampered ciphertext fails distinct from wrong KEK (US-CA-02)
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-02` `@error` — AEAD tamper
/// detection: flipping a byte in the sealed ciphertext makes AES-GCM open
/// FAIL with a `CaError` naming a corrupt/tampered envelope, DISTINCT from
/// the wrong-KEK error variant (the GCM auth tag distinguishes them).
#[test]
fn root_key_envelope_tampered_ciphertext_fails_distinct_from_wrong_kek() {
    // GIVEN a sealed record under a KEK.
    let key_der = real_root_key_der();
    let id = kek_id("kek-root-01");
    let kek = FixtureKek::with(&id, [0x33; KEK_LEN]);
    let codec = RootKeyAeadCodec::new();
    let mut record = codec.seal(&kek, &id, &key_der).expect("seal succeeds");

    // WHEN a single ciphertext byte is flipped (corruption / tamper at rest).
    record.ciphertext[0] ^= 0xFF;

    // THEN opening under the CORRECT KEK fails with the tampered-envelope
    // variant — integrity failure, NOT KEK-confusion.
    let err = codec.open(&kek, &id, &record).expect_err("tampered ciphertext must fail to open");
    assert!(
        matches!(err, CaError::TamperedEnvelope { .. }),
        "a tampered ciphertext fails with TamperedEnvelope, got {err:?}"
    );

    // AND that variant is DISTINCT from the wrong-KEK variant.
    assert!(
        !matches!(err, CaError::WrongKek { .. }),
        "tampered-envelope must be distinct from wrong-KEK"
    );
}

// ---------------------------------------------------------------------------
// S-02-04 — wrong KEK fails distinct from tampered (US-CA-02)
// ---------------------------------------------------------------------------

/// `@real-io` `@adapter-integration` `@S-02` `@error` — wrong-KEK path:
/// opening the sealed record with a DIFFERENT KEK fails with a wrong-KEK
/// `CaError` variant (distinct from tampered-envelope). AAD = `kek_id` binds
/// the ciphertext to the KEK identity (defends KEK-confusion).
#[test]
fn root_key_envelope_wrong_kek_fails_distinct_from_tampered() {
    // GIVEN a record sealed under `kek-root-01`, and a provider that ALSO
    // knows a different KEK `kek-root-02` (so resolution succeeds — the
    // failure is identity mismatch, not a missing key).
    let key_der = real_root_key_der();
    let sealed_id = kek_id("kek-root-01");
    let other_id = kek_id("kek-root-02");
    let kek = FixtureKek::with(&sealed_id, [0x44; KEK_LEN]).and(&other_id, [0x55; KEK_LEN]);
    let codec = RootKeyAeadCodec::new();
    let record = codec.seal(&kek, &sealed_id, &key_der).expect("seal succeeds");

    // WHEN the record is opened under a DIFFERENT KEK identity.
    let err =
        codec.open(&kek, &other_id, &record).expect_err("opening under a different KEK must fail");

    // THEN it fails with the wrong-KEK variant, carrying both identities.
    assert!(
        matches!(
            &err,
            CaError::WrongKek { sealed_under, supplied }
                if sealed_under == &sealed_id && supplied == &other_id
        ),
        "wrong KEK fails with WrongKek naming both identities, got {err:?}"
    );

    // AND that variant is DISTINCT from the tampered-envelope variant.
    assert!(
        !matches!(err, CaError::TamperedEnvelope { .. }),
        "wrong-KEK must be distinct from tampered-envelope"
    );
}

/// True when `haystack` contains `needle` as a contiguous subslice. Used for
/// the KPI K3 byte-scan (fail-closed: any plaintext-key occurrence fails).
fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|window| window == needle)
}
