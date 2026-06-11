//! Issued-certificate audit row — rkyv versioned envelope (built-in-ca /
//! GH #28, ADR-0063 D6, ADR-0048).
//!
//! `issued_certificates` is an **ObservationStore** audit row — the record
//! of *what was issued*, not the CA material itself. The CA *material*
//! (root key, intermediate keys) is intent (ADR-0063 D2); the *record of an
//! issuance* is observation (D6). It mirrors the `alloc_status` /
//! `node_health` observation-row plumbing: a per-type rkyv versioned
//! envelope wrapping a `V1` payload, with the co-located typed codec that
//! wraps/unwraps the envelope at the persistence boundary.
//!
//! This module is pure rkyv — no crypto backend, no I/O, dst-lint-clean
//! `core`. Gossiped when GH #36 lands; single-node today = local.
//!
//! # Persist inputs, not derived state
//!
//! Per `.claude/rules/development.md` § "Persist inputs, not derived
//! state", every column is an audit *input* — the facts observed at
//! issuance time (`serial`, `spiffe_id`, `issuer_serial`, `not_before`,
//! `not_after`, `node_id`, `issued_at`). Derived classifications (e.g. an
//! "expired / valid" grade) are recomputed at read time against the live
//! clock, never persisted onto the row.
//!
//! # Asymmetric unknown-version handling
//!
//! Per ADR-0048 § 3 the read policy is **asymmetric**: intent fail-fasts
//! (`RootCaKeyRecordV1::from_store_bytes` emits `health.startup.refused`
//! and refuses to start), observation **logs-and-skips** the offending row
//! so convergence proceeds for the surviving rows.
//! [`IssuedCertificateRowV1::from_store_bytes`] implements the observation
//! side — on an [`EnvelopeError`] (unknown future version OR malformed
//! bytes) it logs a single `tracing::warn!` and returns
//! [`ObservationStoreError::Envelope`], which the adapter's row-reader
//! drops while keeping the other rows.
//!
//! # Schema evolution
//!
//! Per ADR-0048 + `.claude/rules/development.md` § "rkyv schema
//! evolution", the public name is an **alias to the payload**
//! ([`IssuedCertificateRow`] `= IssuedCertificateRowV1`); the
//! [`IssuedCertificateRowEnvelope`] enum is `pub` but **NOT re-exported
//! from `lib.rs`** (Layer 1) and direct variant construction is rejected
//! by the `xtask::dst_lint` scanner (Layer 2). Writers wrap via
//! [`IssuedCertificateRowV1::archive_for_store`]; readers project via
//! [`IssuedCertificateRowV1::from_store_bytes`]. The golden-bytes fixture
//! in `tests/schema_evolution/issued_certificate_row.rs` pins the V1
//! layout.

use rkyv::util::AlignedVec;

use crate::codec::{EnvelopeError, VersionedEnvelope, decode_envelope_bytes};
use crate::id::{CertSerial, IssuanceOrdinal, NodeId, SpiffeId};
use crate::traits::observation_store::ObservationStoreError;
use crate::wall_clock::UnixInstant;

// ---------------------------------------------------------------------
// V1 payload + per-type versioned envelope (ADR-0048 alias-to-payload).
// ---------------------------------------------------------------------

/// Public alias to the latest [`IssuedCertificateRow`] payload — callers
/// construct `IssuedCertificateRow { .. }` directly (UI-02
/// alias-to-payload).
pub type IssuedCertificateRow = IssuedCertificateRowV1;

/// Documentation alias for "the latest payload" — used by the
/// schema-evolution harness and the version-bump procedure.
pub type IssuedCertificateRowLatest = IssuedCertificateRowV1;

/// V1 persisted shape of an `issued_certificates` audit row (ADR-0063 D6).
///
/// The audit *inputs* — the facts observed when a certificate was issued:
///
/// * `serial` — the issued leaf/intermediate certificate's serial number.
/// * `spiffe_id` — the SPIFFE identity the certificate was bound to.
/// * `issuer_serial` — serial of the issuing CA certificate (the chain
///   link back to the issuer; lets an auditor walk issuance lineage).
/// * `not_before` — start of the certificate's validity window.
/// * `not_after` — end of the certificate's validity window.
/// * `node_id` — the node whose CA minted this certificate.
/// * `issued_at` — wall-clock observation of when issuance happened.
/// * `issuance_ordinal` — the global monotonic issuance-order rank.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct IssuedCertificateRowV1 {
    /// Serial number of the issued certificate.
    pub serial: CertSerial,
    /// SPIFFE identity the certificate was bound to.
    pub spiffe_id: SpiffeId,
    /// Serial number of the issuing CA certificate.
    pub issuer_serial: CertSerial,
    /// Start of the certificate validity window.
    pub not_before: UnixInstant,
    /// End of the certificate validity window.
    pub not_after: UnixInstant,
    /// Node whose CA minted the certificate.
    pub node_id: NodeId,
    /// Wall-clock instant the issuance was observed.
    pub issued_at: UnixInstant,
    /// The global monotonic issuance-order rank; the consumer's current-cert
    /// projection selects max-ordinal per SPIFFE ID — recency-correct even on
    /// an `issued_at` tie (the equal-`issued_at` same-tick re-issue a fixed
    /// `SimClock` produces). See feature-delta § D1-AMEND.
    pub issuance_ordinal: IssuanceOrdinal,
}

/// Per-type rkyv versioned envelope for [`IssuedCertificateRow`]
/// (ADR-0048 § 1). `pub` due to rustc E0446 in the trait impl; Layer 1
/// enforced by non-re-export from `lib.rs`, Layer 2 by the
/// `xtask::dst_lint` envelope-variant-construction scanner. NOT for direct
/// construction — write through [`IssuedCertificateRowV1::archive_for_store`].
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum IssuedCertificateRowEnvelope {
    V1(IssuedCertificateRowV1),
}

impl VersionedEnvelope for IssuedCertificateRowEnvelope {
    type Latest = IssuedCertificateRowV1;

    fn latest(payload: Self::Latest) -> Self {
        Self::V1(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1),
        }
    }

    /// Discriminant offset for `IssuedCertificateRowEnvelope` archives,
    /// measured from the END of the archive bytes.
    ///
    /// Empirically pinned against canonical V1 payloads of varying
    /// `CertSerial` / `SpiffeId` / `NodeId` string lengths: rkyv 0.8 places
    /// the outer enum's discriminant byte at a fixed offset from the END of
    /// the archive (the variable-length string slabs grow the leading
    /// region; the trailing root structure has a fixed footprint).
    /// Triangulated against `GOLDEN_DISCRIMINANT_OFFSET_V1` in
    /// `tests/schema_evolution/issued_certificate_row.rs`; both update in
    /// lockstep on a `V<N+1>` bump per `development.md` § "Version-bump
    /// procedure".
    ///
    /// Empirically located at `104` by flipping each byte of a canonical V1
    /// archive to an out-of-set tag and observing which one causes rkyv's
    /// bytecheck to surface `invalid discriminant '99' for enum
    /// 'ArchivedIssuedCertificateRowEnvelope'` (the
    /// `probe_discriminant_offset` helper in the schema-evolution fixture).
    /// Re-pinned from `96` → `104` when the `issuance_ordinal` `u64` field was
    /// appended to V1 (greenfield single-cut, feature-delta § D1-AMEND-3): the
    /// 8-byte trailing field pushes the discriminant 8 bytes further from the
    /// archive's end.
    fn discriminant_offset_from_end() -> Option<usize> {
        Some(104)
    }

    fn known_discriminants() -> &'static [u8] {
        // V1 carries rkyv discriminant 0 (declaration order).
        &[0]
    }

    fn type_name() -> &'static str {
        "IssuedCertificateRowEnvelope"
    }
}

impl IssuedCertificateRowV1 {
    /// Archive this row for persistence — wraps in the latest envelope and
    /// rkyv-serialises to canonical bytes.
    ///
    /// # Postconditions
    ///
    /// On `Ok(bytes)`, `bytes` is the canonical rkyv-archived sequence of
    /// `IssuedCertificateRowEnvelope::V1(self.clone())`. Two archivals of
    /// the same logical row produce byte-identical output.
    ///
    /// # Observable invariants
    ///
    /// `IssuedCertificateRowV1::from_store_bytes(&self.archive_for_store()?, None)`
    /// returns `Ok(self_owned)` bit-equivalent to `self`.
    ///
    /// # Errors
    ///
    /// Returns [`EnvelopeError::Malformed`] when the rkyv serialiser fails
    /// (unreachable for valid payloads).
    pub fn archive_for_store(&self) -> Result<AlignedVec, EnvelopeError> {
        let envelope = IssuedCertificateRowEnvelope::latest(self.clone());
        rkyv::to_bytes::<rkyv::rancor::Error>(&envelope)
            .map_err(|source| EnvelopeError::Malformed { source })
    }

    /// Decode persisted bytes back into an [`IssuedCertificateRow`] — the
    /// OBSERVATION read path (log-and-skip; asymmetric vs the intent path's
    /// fail-fast).
    ///
    /// # Edge cases
    ///
    /// * Empty / truncated / corrupt `bytes` → [`EnvelopeError::Malformed`].
    /// * Future-binary `V<N+1>` bytes → [`EnvelopeError::UnknownVersion`].
    ///
    /// # Observable invariants
    ///
    /// On `Err(...)`, exactly one `tracing::warn!` event with
    /// `name: "observation.row.skipped"` fires BEFORE the `Err` return —
    /// per ADR-0048 § 3 (observation log-and-skip policy; asymmetric vs the
    /// intent path which refuses to start). The event carries the optional
    /// `key` (`"<unknown>"` when `None`) and the underlying `envelope_error`
    /// for operator diagnosis. The adapter-side row reader drops THIS row
    /// and keeps the surviving rows — convergence proceeds.
    pub fn from_store_bytes(
        bytes: &[u8],
        key: Option<&str>,
    ) -> Result<Self, ObservationStoreError> {
        match decode_envelope_bytes::<IssuedCertificateRowEnvelope>(bytes) {
            Ok(row) => Ok(row),
            Err(envelope_error) => {
                tracing::warn!(
                    name: "observation.row.skipped",
                    key = key.unwrap_or("<unknown>"),
                    envelope_error = ?envelope_error,
                    "issued_certificate row envelope decode failed; skipping row, \
                     convergence proceeds for surviving rows",
                );
                Err(ObservationStoreError::Envelope { source: envelope_error })
            }
        }
    }
}
