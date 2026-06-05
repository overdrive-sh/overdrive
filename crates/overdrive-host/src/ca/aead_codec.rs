//! `RootKeyAeadCodec` — HKDF-SHA256 → AES-256-GCM seal/open for the root CA
//! private key (built-in-ca / GH #28, ADR-0063 D4).
//!
//! The root CA private key is the platform's trust anchor; it is never
//! persisted in plaintext (KPI K3). This host codec implements the at-rest
//! envelope scheme:
//!
//! ```text
//! subkey          = HKDF-SHA256-Expand(
//!                       HKDF-SHA256-Extract(salt, KEK),
//!                       info = "overdrive/ca/root-key/v1",
//!                       L = 32)
//! ciphertext, tag = AES-256-GCM-Seal(subkey, nonce, root_key_der, aad = kek_id)
//! ```
//!
//! `salt` (32 bytes) and `nonce` (12 bytes) are drawn fresh per seal from the
//! crypto-backend CSPRNG (`ring::rand::SystemRandom`). `info` is the
//! domain-separation label [`HKDF_INFO`]. AAD = the `kek_id` bytes binds the
//! ciphertext to its KEK identity (defends KEK-confusion — ADR-0063 D4).
//!
//! # Dependency discipline
//!
//! All HKDF + AES-GCM crypto goes through `ring` (the workspace crypto
//! provider; aws-lc-rs/FIPS is #204). This is `adapter-host` code — `ring`
//! FFI lives here, never in `overdrive-core` (dst-lint). The KEK source is the
//! pure [`Kek`] provider port from `overdrive-core`; this codec resolves KEK
//! material through it and never knows about keyrings.
//!
//! # Persist inputs, not derived state
//!
//! [`seal`](RootKeyAeadCodec::seal) emits a [`RootCaKeyRecord`] carrying only
//! the AEAD *inputs* (`kek_id`, `salt`, `info`, `nonce`, `ciphertext`,
//! `aead_tag`). The plaintext key is a *derived* value recomputed by
//! [`open`](RootKeyAeadCodec::open), never cached at rest — that absence IS the
//! K3 guardrail (`.claude/rules/development.md` § "Persist inputs, not derived
//! state").
//!
//! # Tampered vs wrong-KEK distinction (ADR-0063 D3/D4, an AC)
//!
//! A failed open must distinguish *integrity failure* from *KEK-confusion*.
//! `ring::aead::open_in_place_separate_tag` returns the same opaque
//! `error::Unspecified` for both, so the distinction is drawn **structurally,
//! before the AES-GCM open**: if the supplied KEK's id ≠ the record's `kek_id`
//! (which is also the AAD), it is [`CaError::WrongKek`] — detectable without
//! decrypting. If the ids match but the GCM tag still fails to authenticate, it
//! is [`CaError::TamperedEnvelope`]. The AAD = `kek_id` binding is what makes a
//! mismatched KEK *also* fail authentication, so even if a caller bypassed the
//! id pre-check the open would still reject — the structural pre-check only
//! refines the *error variant*, it does not weaken the cryptographic guard.

use overdrive_core::ca::kek::{Kek, KekError, KekMaterial};
use overdrive_core::ca::root_key_envelope::{KekId, RootCaKeyRecord};
use overdrive_core::traits::ca::CaError;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, Tag, UnboundKey};
use ring::hkdf::{HKDF_SHA256, KeyType, Salt};
use ring::rand::{SecureRandom, SystemRandom};

/// HKDF salt length (bytes) — 256-bit, drawn fresh per seal.
const SALT_LEN: usize = 32;

/// Derived AES-256-GCM subkey length (bytes).
const SUBKEY_LEN: usize = 32;

/// AES-GCM authentication tag length (bytes) — 128-bit.
const TAG_LEN: usize = 16;

/// HKDF domain-separation `info` label (ADR-0063 D4). Varying this label lets
/// the same KEK protect distinct future secrets with no key reuse.
const HKDF_INFO: &[u8] = b"overdrive/ca/root-key/v1";

/// A fixed-length [`KeyType`] for HKDF expand — yields a [`SUBKEY_LEN`]-byte
/// OKM (ring's `hkdf::expand` is generic over output length via `KeyType`).
struct SubkeyLen;

impl KeyType for SubkeyLen {
    fn len(&self) -> usize {
        SUBKEY_LEN
    }
}

/// HKDF-SHA256 → AES-256-GCM seal/open codec for the root CA private key.
///
/// Resolves KEK material through the injected [`Kek`] provider port and draws
/// salt/nonce from the crypto-backend CSPRNG. Holds no secret state — the KEK
/// is resolved per call and never retained.
pub struct RootKeyAeadCodec {
    rng: SystemRandom,
}

impl Default for RootKeyAeadCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl RootKeyAeadCodec {
    /// Construct a codec over the crypto-backend CSPRNG.
    #[must_use]
    pub fn new() -> Self {
        Self { rng: SystemRandom::new() }
    }

    /// Seal `root_key_der` under the KEK named `kek_id`, producing a
    /// persistable [`RootCaKeyRecord`].
    ///
    /// HKDF-SHA256-derives a per-use subkey from the resolved KEK (fresh
    /// random salt), then AES-256-GCM-seals `root_key_der` under it (fresh
    /// random nonce, AAD = `kek_id` bytes). The returned record carries the
    /// AEAD inputs only — never the plaintext key.
    ///
    /// # Errors
    /// * [`CaError::SigningFailed`] when the KEK cannot be resolved, the
    ///   CSPRNG fails, or the AES-GCM seal fails (all map to a single
    ///   adapter-internal failure — none is an attacker-distinguishable
    ///   condition the way open's two failure modes are).
    pub fn seal(
        &self,
        kek: &dyn Kek,
        kek_id: &KekId,
        root_key_der: &[u8],
    ) -> Result<RootCaKeyRecord, CaError> {
        let material = kek.resolve(kek_id).map_err(|err| map_resolve_err(&err))?;

        let mut salt = [0u8; SALT_LEN];
        self.rng.fill(&mut salt).map_err(|_| CaError::signing_failed("CSPRNG salt draw failed"))?;

        let mut nonce_bytes = [0u8; NONCE_LEN];
        self.rng
            .fill(&mut nonce_bytes)
            .map_err(|_| CaError::signing_failed("CSPRNG nonce draw failed"))?;

        let key = derive_subkey(&material, &salt)?;

        // Seal in place: copy the plaintext into a working buffer, encrypt it,
        // and take the tag out separately so it lands in the record's distinct
        // `aead_tag` field (matching the persisted-shape contract).
        let mut in_out = root_key_der.to_vec();
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);
        let tag = key
            .seal_in_place_separate_tag(nonce, Aad::from(aad_bytes(kek_id)), &mut in_out)
            .map_err(|_| CaError::signing_failed("AES-256-GCM seal failed"))?;

        Ok(RootCaKeyRecord {
            kek_id: kek_id.clone(),
            salt: salt.to_vec(),
            info: HKDF_INFO.to_vec(),
            nonce: nonce_bytes.to_vec(),
            ciphertext: in_out,
            aead_tag: tag.as_ref().to_vec(),
        })
    }

    /// Open a sealed [`RootCaKeyRecord`] under the KEK named `supplied_kek_id`,
    /// recovering the byte-identical root key DER.
    ///
    /// # Errors
    /// * [`CaError::WrongKek`] when `supplied_kek_id` ≠ the record's `kek_id`
    ///   (KEK-confusion; detected structurally, before decrypt — the AAD
    ///   binding would also fail authentication, this only refines the variant).
    /// * [`CaError::TamperedEnvelope`] when the ids match but AES-GCM
    ///   authentication fails (corrupt/tampered ciphertext or tag — integrity
    ///   failure under the correct KEK).
    /// * [`CaError::SigningFailed`] when the KEK cannot be resolved or the
    ///   record's nonce is malformed.
    ///
    /// Takes `&self` for API symmetry with [`seal`](Self::seal) even though
    /// open draws no randomness — the codec is the single seal/open surface.
    #[allow(
        clippy::unused_self,
        reason = "API symmetry with seal(); codec is the seal/open surface"
    )]
    pub fn open(
        &self,
        kek: &dyn Kek,
        supplied_kek_id: &KekId,
        record: &RootCaKeyRecord,
    ) -> Result<Vec<u8>, CaError> {
        // KEK-confusion is detected structurally on the bound identity (also
        // the AAD) BEFORE any decrypt — this is what makes WrongKek distinct
        // from TamperedEnvelope (both yield ring's opaque Unspecified).
        if supplied_kek_id != &record.kek_id {
            return Err(CaError::wrong_kek(record.kek_id.clone(), supplied_kek_id.clone()));
        }

        let material = kek.resolve(supplied_kek_id).map_err(|err| map_resolve_err(&err))?;
        let key = derive_subkey_with_info(&material, &record.salt, &record.info)?;

        let nonce_bytes: [u8; NONCE_LEN] = record
            .nonce
            .as_slice()
            .try_into()
            .map_err(|_| CaError::signing_failed("record nonce is not 12 bytes"))?;
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        let tag: Tag = record
            .aead_tag
            .as_slice()
            .try_into()
            .map_err(|_| CaError::tampered_envelope(record.kek_id.clone()))?;

        // Open in place: AES-GCM authenticates ciphertext + tag against the
        // AAD (kek_id). The ids already match here, so an auth failure is
        // integrity failure (tampered), not KEK-confusion.
        let mut in_out = record.ciphertext.clone();
        let plaintext = key
            .open_in_place_separate_tag(
                nonce,
                Aad::from(aad_bytes(&record.kek_id)),
                tag,
                &mut in_out,
                0..,
            )
            .map_err(|_| CaError::tampered_envelope(record.kek_id.clone()))?;

        Ok(plaintext.to_vec())
    }
}

/// The AES-GCM additional-authenticated-data for a record: the `kek_id` bytes,
/// binding the ciphertext to its KEK identity (defends KEK-confusion —
/// ADR-0063 D4).
///
/// Single source of truth for the AAD derivation: both [`seal`] and [`open`]
/// route through here so the two sides cannot diverge on what bytes are
/// authenticated. The AAD is the `kek_id` — never the HKDF `info` label, which
/// is a key-derivation input, not an authentication input.
///
/// [`seal`]: RootKeyAeadCodec::seal
/// [`open`]: RootKeyAeadCodec::open
fn aad_bytes(kek_id: &KekId) -> &[u8] {
    kek_id.as_str().as_bytes()
}

/// HKDF-SHA256-derive the AES-256-GCM subkey from the KEK + salt, using the
/// canonical [`HKDF_INFO`] label (the seal path).
fn derive_subkey(material: &KekMaterial, salt: &[u8]) -> Result<LessSafeKey, CaError> {
    derive_subkey_with_info(material, salt, HKDF_INFO)
}

/// HKDF-SHA256-derive the AES-256-GCM subkey from the KEK + salt + the
/// record's persisted `info` label (the open path uses the stored label so a
/// future domain-separation change is honoured by the record's own bytes).
fn derive_subkey_with_info(
    material: &KekMaterial,
    salt: &[u8],
    info: &[u8],
) -> Result<LessSafeKey, CaError> {
    let prk = Salt::new(HKDF_SHA256, salt).extract(material.expose_secret());
    let info_components = [info];
    let okm = prk
        .expand(&info_components, SubkeyLen)
        .map_err(|_| CaError::signing_failed("HKDF expand failed"))?;
    let mut subkey = [0u8; SUBKEY_LEN];
    okm.fill(&mut subkey).map_err(|_| CaError::signing_failed("HKDF fill failed"))?;
    let unbound = UnboundKey::new(&AES_256_GCM, &subkey)
        .map_err(|_| CaError::signing_failed("AES-256-GCM key construction failed"))?;
    Ok(LessSafeKey::new(unbound))
}

/// Map a KEK-resolution failure into a [`CaError`]. A resolution failure is an
/// adapter-internal signing-path failure (the KEK provider could not supply
/// material) — distinct from the open-time integrity/KEK-confusion variants.
fn map_resolve_err(err: &KekError) -> CaError {
    CaError::signing_failed(format!("KEK resolution failed: {err}"))
}

// Silence the unused-const lint on TAG_LEN when only used in a const-assert
// context below; the assert pins ring's tag width to our record contract.
const _: () = assert!(TAG_LEN == 16, "AES-GCM tag is 16 bytes");
