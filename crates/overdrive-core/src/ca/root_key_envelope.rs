//! Root CA key record — rkyv versioned envelope (built-in-ca / GH #28,
//! ADR-0063 D2/D4, ADR-0048).
//!
//! The root CA private key is never persisted in plaintext. It is sealed
//! by an AEAD (AES-256-GCM) under a key derived (HKDF-SHA-256) from a
//! Key Encryption Key (KEK) the operator controls; only the sealed
//! material and the *inputs* to the seal are durably stored. This module
//! defines the persisted *shape* — the `RootCaKeyRecordV1` payload, its
//! per-type versioned envelope, and the co-located typed codec that
//! wraps/unwraps the envelope at the persistence boundary.
//!
//! The AEAD seal/open codec itself (HKDF derivation + AES-GCM
//! encrypt/decrypt) lands in a later slice and consumes this shape; it
//! does not live here. This module is pure rkyv — no crypto backend, no
//! I/O, dst-lint-clean `core`.
//!
//! # Persist inputs, not derived state
//!
//! Per `.claude/rules/development.md` § "Persist inputs, not derived
//! state", every field of [`RootCaKeyRecordV1`] is an AEAD *input* by
//! construction (`kek_id`, `salt`, `info`, `nonce`, `ciphertext`,
//! `aead_tag`). There is no decoded/plaintext key field — the plaintext
//! root key is a *derived* value recomputed by the open codec from these
//! inputs plus the live KEK, never cached at rest. This is the K3
//! zero-plaintext-at-rest guardrail.
//!
//! # Schema evolution
//!
//! Per ADR-0048 + `.claude/rules/development.md` § "rkyv schema
//! evolution", the public name is an **alias to the payload**
//! ([`RootCaKeyRecord`] `= RootCaKeyRecordV1`); the
//! [`RootCaKeyEnvelope`] enum is `pub` but **NOT re-exported from
//! `lib.rs`** (Layer 1) and direct variant construction is rejected by
//! the `xtask::dst_lint` scanner (Layer 2). Writers wrap via
//! [`RootCaKeyRecordV1::archive_for_store`]; readers project via
//! [`RootCaKeyRecordV1::from_store_bytes`]. The golden-bytes fixture in
//! `tests/schema_evolution/root_ca_key.rs` pins the V1 layout.

use std::path::Path;

use rkyv::util::AlignedVec;

use crate::codec::{EnvelopeError, VersionedEnvelope, decode_envelope_bytes};
use crate::id::IdParseError;
use crate::traits::intent_store::IntentStoreError;

/// Maximum length (chars) of a [`KekId`]. Label-shaped, so the generic
/// label ceiling applies.
const KEK_ID_MAX: usize = 253;

/// Identifier of the Key Encryption Key (KEK) under which the root CA
/// private key is sealed.
///
/// The KEK itself never enters `overdrive-core` — only its stable
/// identifier, which the (future, slice 02-02) KEK provider port
/// resolves to actual key material at seal/open time. Persisting the
/// `KekId` (not the KEK) is the "persist inputs, not derived state"
/// discipline: the binding between a sealed record and its KEK is an
/// input the open path needs, recomputed against the live provider.
///
/// Label-shaped: non-empty, `≤ 253` chars, lowercase ASCII alphanumeric
/// plus `-` / `_` / `.`. Case-insensitive on parse; the canonical form
/// is lowercase.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[serde(try_from = "String", into = "String")]
pub struct KekId(String);

impl KekId {
    /// Validate and construct a [`KekId`].
    ///
    /// # Errors
    ///
    /// * [`IdParseError::Empty`] when `raw` is empty.
    /// * [`IdParseError::TooLong`] when `raw` exceeds [`KEK_ID_MAX`] chars.
    /// * [`IdParseError::InvalidChar`] on any char outside lowercase
    ///   ASCII alphanumeric / `-` / `_` / `.` (after lowercasing).
    pub fn new(raw: &str) -> Result<Self, IdParseError> {
        if raw.is_empty() {
            return Err(IdParseError::Empty { kind: "KekId" });
        }
        if raw.len() > KEK_ID_MAX {
            return Err(IdParseError::TooLong { kind: "KekId", max: KEK_ID_MAX });
        }
        let lowered = raw.to_ascii_lowercase();
        for (index, ch) in lowered.char_indices() {
            if !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')) {
                return Err(IdParseError::InvalidChar { kind: "KekId", ch, index });
            }
        }
        Ok(Self(lowered))
    }

    /// Canonical (lowercase) string form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for KekId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for KekId {
    type Err = IdParseError;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::new(raw)
    }
}

impl TryFrom<String> for KekId {
    type Error = IdParseError;
    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::new(&raw)
    }
}

impl TryFrom<&str> for KekId {
    type Error = IdParseError;
    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        Self::new(raw)
    }
}

impl From<KekId> for String {
    fn from(v: KekId) -> Self {
        v.0
    }
}

// ---------------------------------------------------------------------
// V1 payload + per-type versioned envelope (ADR-0048 alias-to-payload).
// ---------------------------------------------------------------------

/// Public alias to the latest [`RootCaKeyRecord`] payload — callers
/// construct `RootCaKeyRecord { .. }` directly (UI-02 alias-to-payload).
pub type RootCaKeyRecord = RootCaKeyRecordV1;

/// Documentation alias for "the latest payload" — used by the
/// schema-evolution harness and version-bump procedure.
pub type RootCaKeyRecordLatest = RootCaKeyRecordV1;

/// V1 persisted shape of the root CA key record (ADR-0063 D4).
///
/// Carries the AEAD *inputs* exclusively — never a decoded key:
///
/// * `kek_id` — identifier of the KEK the record is sealed under.
/// * `salt` — HKDF-SHA-256 salt.
/// * `info` — HKDF-SHA-256 info / context-binding bytes.
/// * `nonce` — AES-GCM nonce (96-bit / 12-byte for GCM).
/// * `ciphertext` — the sealed root-key DER bytes.
/// * `aead_tag` — the AES-GCM authentication tag (128-bit / 16-byte).
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct RootCaKeyRecordV1 {
    /// Identifier of the KEK under which `ciphertext` is sealed.
    pub kek_id: KekId,
    /// HKDF-SHA-256 salt input to the seal-key derivation.
    pub salt: Vec<u8>,
    /// HKDF-SHA-256 info / context-binding input.
    pub info: Vec<u8>,
    /// AES-GCM nonce (12 bytes for 96-bit GCM nonces).
    pub nonce: Vec<u8>,
    /// Sealed root-key DER bytes (AES-GCM ciphertext).
    pub ciphertext: Vec<u8>,
    /// AES-GCM authentication tag (16 bytes).
    pub aead_tag: Vec<u8>,
}

/// Per-type rkyv versioned envelope for [`RootCaKeyRecord`] (ADR-0048
/// § 1). `pub` due to rustc E0446 in the trait impl; Layer 1 enforced
/// by non-re-export from `lib.rs`, Layer 2 by the `xtask::dst_lint`
/// envelope-variant-construction scanner. NOT for direct construction —
/// write through [`RootCaKeyRecordV1::archive_for_store`].
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum RootCaKeyEnvelope {
    V1(RootCaKeyRecordV1),
}

impl VersionedEnvelope for RootCaKeyEnvelope {
    type Latest = RootCaKeyRecordV1;

    fn latest(payload: Self::Latest) -> Self {
        Self::V1(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1),
        }
    }

    /// Discriminant offset for `RootCaKeyEnvelope` archives, measured
    /// from the END of the archive bytes.
    ///
    /// Empirically pinned against canonical V1 payloads of varying
    /// `Vec<u8>` field lengths: rkyv 0.8 places the outer enum's
    /// discriminant byte at a fixed offset from the END of the archive
    /// (the variable-length slabs grow the leading region; the trailing
    /// root structure has a fixed footprint). Triangulated against
    /// `GOLDEN_DISCRIMINANT_OFFSET_V1` in
    /// `tests/schema_evolution/root_ca_key.rs`; both update in lockstep
    /// on a `V<N+1>` bump per `development.md` § "Version-bump procedure".
    fn discriminant_offset_from_end() -> Option<usize> {
        Some(52)
    }

    fn known_discriminants() -> &'static [u8] {
        // V1 carries rkyv discriminant 0 (declaration order).
        &[0]
    }

    fn type_name() -> &'static str {
        "RootCaKeyEnvelope"
    }
}

impl RootCaKeyRecordV1 {
    /// Archive this record for persistence — wraps in the latest
    /// envelope and rkyv-serialises to canonical bytes.
    ///
    /// # Postconditions
    ///
    /// On `Ok(bytes)`, `bytes` is the canonical rkyv-archived sequence
    /// of `RootCaKeyEnvelope::V1(self.clone())`. Two archivals of the
    /// same logical record produce byte-identical output.
    ///
    /// # Observable invariants
    ///
    /// `RootCaKeyRecordV1::from_store_bytes(&self.archive_for_store()?, p, None)`
    /// returns `Ok(self_owned)` bit-equivalent to `self` for any redb
    /// path `p`.
    ///
    /// # Errors
    ///
    /// Returns [`EnvelopeError::Malformed`] when the rkyv serialiser
    /// fails (unreachable for valid payloads).
    pub fn archive_for_store(&self) -> Result<AlignedVec, EnvelopeError> {
        let envelope = RootCaKeyEnvelope::latest(self.clone());
        rkyv::to_bytes::<rkyv::rancor::Error>(&envelope)
            .map_err(|source| EnvelopeError::Malformed { source })
    }

    /// Decode persisted bytes back into a [`RootCaKeyRecord`].
    ///
    /// # Edge cases
    ///
    /// * Empty / truncated / corrupt `bytes` → [`EnvelopeError::Malformed`].
    /// * Future-binary `V<N+1>` bytes → [`EnvelopeError::UnknownVersion`].
    ///
    /// # Observable invariants
    ///
    /// On `Err(...)`, exactly one `tracing::error!` event with
    /// `name: "health.startup.refused"` fires BEFORE the `Err` return —
    /// per ADR-0048 § 3 (intent fail-fast policy; asymmetric vs the
    /// observation path which logs-and-skips). The event carries the
    /// `redb_path`, the optional `key` (`"<unknown>"` when `None`), and
    /// the underlying `envelope_error` for operator diagnosis.
    pub fn from_store_bytes(
        bytes: &[u8],
        redb_path: &Path,
        key: Option<&str>,
    ) -> Result<Self, IntentStoreError> {
        match decode_envelope_bytes::<RootCaKeyEnvelope>(bytes) {
            Ok(record) => Ok(record),
            Err(envelope_error) => {
                tracing::error!(
                    name: "health.startup.refused",
                    redb_path = %redb_path.display(),
                    key = key.unwrap_or("<unknown>"),
                    envelope_error = ?envelope_error,
                    "root CA key envelope decode failed; control-plane refusing to start",
                );
                Err(IntentStoreError::Envelope {
                    redb_path: redb_path.to_path_buf(),
                    source: envelope_error,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use proptest::prelude::*;

    use super::{KEK_ID_MAX, KekId};
    use crate::id::IdParseError;

    #[test]
    fn case_insensitive_parse_lowercases_canonical() {
        // An uppercase input parses and `Display` / `as_str` emit the
        // lowercase canonical form.
        let parsed = KekId::new("KEK-Root-01").expect("valid mixed-case kek id");
        assert_eq!(parsed.as_str(), "kek-root-01");
        assert_eq!(parsed.to_string(), "kek-root-01");
    }

    #[test]
    fn rejects_empty_with_empty_variant() {
        assert!(matches!(KekId::new(""), Err(IdParseError::Empty { kind: "KekId" })));
    }

    #[test]
    fn rejects_over_length_with_too_long_variant() {
        let too_long = "a".repeat(KEK_ID_MAX + 1);
        assert!(matches!(
            KekId::new(&too_long),
            Err(IdParseError::TooLong { kind: "KekId", max: KEK_ID_MAX })
        ));
    }

    #[test]
    fn rejects_out_of_class_char_with_invalid_char_variant() {
        // Slash and space are both outside the lowercase-alphanumeric +
        // `-`/`_`/`.` class; each surfaces the specific variant, not a
        // generic error.
        assert!(matches!(
            KekId::new("kek/root"),
            Err(IdParseError::InvalidChar { kind: "KekId", ch: '/', .. })
        ));
        assert!(matches!(
            KekId::new("kek root"),
            Err(IdParseError::InvalidChar { kind: "KekId", ch: ' ', .. })
        ));
    }

    proptest! {
        /// Display↔FromStr and serde round-trip parity over the valid
        /// `KekId` input space. The inputs are already canonical (lowercase
        /// alphanumeric + `-`/`_`/`.`, non-empty, `≤ KEK_ID_MAX`), so
        /// `from_str(k.to_string())` and serde JSON round-trip both recover
        /// the original — the two codecs match `Display` / `FromStr`
        /// exactly. Generated values vary per case and are never a fixed
        /// sentinel, so a body returning a constant cannot satisfy this.
        #[test]
        fn display_fromstr_and_serde_round_trip(raw in "[a-z0-9][a-z0-9._-]{0,40}") {
            let k = KekId::new(&raw).expect("generated value is in the valid class");

            // Display ↔ FromStr parity.
            prop_assert_eq!(KekId::from_str(&k.to_string()), Ok(k.clone()));

            // serde ↔ Display parity: JSON form is the quoted canonical
            // string and deserialises back to the same value.
            let json = serde_json::to_string(&k).expect("serialize KekId");
            prop_assert_eq!(json, format!("\"{}\"", k.as_str()));
            let back: KekId = serde_json::from_str(&format!("\"{}\"", k.as_str()))
                .expect("deserialize KekId");
            prop_assert_eq!(back, k);
        }
    }
}
