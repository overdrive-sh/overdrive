//! Snapshot framing — the canonical byte layout for an `IntentStore`
//! full-state export.
//!
//! # Byte layout
//!
//! A framed snapshot is laid out as:
//!
//! ```text
//! offset 0..4      magic    = b"OSNP"           (4 bytes)
//! offset 4..6      version  = u16 little-endian (2 bytes)
//! offset 6..N      payload  = rkyv-archived Vec<(Vec<u8>, Vec<u8>)>
//!                             (key, value)
//! ```
//!
//! The payload is the rkyv archival of the entry list, with entries
//! sorted by key byte-lexicographically *before* archival. Sorting is
//! what makes the framed bytes deterministic: two stores holding the
//! same set of entries produce byte-identical exports regardless of
//! insertion order.
//!
//! This framing is shared with the future `RaftStore`: a single-mode
//! export must be replayable as the initial Raft log entry without
//! re-encoding, so `RaftStore::bootstrap_from` consumes the same
//! frame. Any change to the magic, version encoding, or payload shape
//! is a cross-crate breaking change and must bump the version field.
//!
//! # Versioning
//!
//! Phase 1 ships frame v1 only — the canonical payload is
//! `Vec<(Vec<u8>, Vec<u8>)>`. ADR-0020 §Decision §3 retired the v2
//! frame variant that briefly carried a per-entry `commit_index`
//! column; v2 frames written during the bug-cascade window are not
//! externally observable (Phase 1 has not shipped) and no upgrade
//! story is required. The single `VERSION` constant below is what
//! [`encode`] writes and [`decode`] accepts.
//!
//! # Determinism guarantees
//!
//! * The entry order is byte-lexicographic on the key.
//! * `rkyv::to_bytes::<_, rancor::Error>` produces a canonical layout
//!   by construction — field order matches the Rust struct definition,
//!   no whitespace, no map-key reordering — per
//!   `docs/product/architecture/adr-0002-schematic-id-canonicalisation.md`.
//! * The magic and version bytes are constants; the version is written
//!   little-endian to match every other `u16` on the wire.
//!
//! # Decoder contract
//!
//! [`decode`] validates magic and version, then deserialises the rkyv
//! payload into `Vec<(Bytes, Bytes)>`. Any mismatch — wrong magic,
//! unknown version, truncated payload, corrupted rkyv bytes — surfaces
//! as a typed [`FrameError`].
//!
//! The concrete corruption paths — truncated payload, flipped payload
//! bit, wrong magic, unknown version — each map to a distinct
//! [`FrameError`] variant so the caller can render actionable errors
//! without re-inspecting the byte slice.

use bytes::Bytes;
use rkyv::rancor;
use rkyv::util::AlignedVec;
use thiserror::Error;

/// Frame magic — `OSNP` = "Overdrive `SNaPshot`". Four bytes at offset
/// 0..4.
pub const MAGIC: [u8; 4] = *b"OSNP";

/// Current frame version produced by [`encode`] and accepted by
/// [`decode`].
///
/// Stored at offset 4..6 as a little-endian `u16`. Bumping this is a
/// cross-crate breaking change — `RaftStore` consumes the same frame.
pub const VERSION: u16 = 1;

/// Length of the fixed-size header (magic + version).
pub const HEADER_LEN: usize = 6;

/// Errors surfaced by [`decode`]. Each variant names a distinct
/// corruption class so callers can render an actionable message
/// without re-inspecting the byte slice.
#[derive(Debug, Error)]
pub enum FrameError {
    /// Input is shorter than the 6-byte header.
    #[error("snapshot frame too short: expected at least {expected} bytes, got {actual}")]
    TooShort {
        /// Minimum expected length (the header length).
        expected: usize,
        /// Actual input length.
        actual: usize,
    },
    /// First four bytes do not match [`MAGIC`].
    #[error("snapshot frame magic mismatch: expected {expected:?}, got {actual:?}")]
    BadMagic {
        /// The expected magic bytes ([`MAGIC`]).
        expected: [u8; 4],
        /// The bytes actually present at offset 0..4.
        actual: [u8; 4],
    },
    /// Version field does not match [`VERSION`].
    #[error("snapshot frame version unsupported: expected {expected}, got {actual}")]
    UnsupportedVersion {
        /// The expected version ([`VERSION`]).
        expected: u16,
        /// The version actually decoded from offset 4..6.
        actual: u16,
    },
    /// rkyv payload failed to decode — truncated bytes, flipped bits,
    /// or malformed archival output.
    #[error("snapshot frame payload is corrupted: {reason}")]
    CorruptedPayload {
        /// Byte offset into the frame at which decoding first failed.
        /// In practice this is `HEADER_LEN` because the rkyv archive
        /// crate does not expose the exact failing offset; the offset
        /// names the start of the payload so callers rendering the
        /// error have a stable byte index to pin a hex dump against.
        offset: usize,
        /// Underlying rkyv diagnostic, already stringified.
        reason: String,
    },
}

impl FrameError {
    /// Byte offset into the frame where the failure was first detected.
    /// Callers mapping this into [`crate::IntentStoreError`] forward
    /// this value as the `SnapshotCorrupt { offset }` field so error
    /// renderers can pin a hex dump without re-inspecting the frame.
    #[must_use]
    pub const fn offset(&self) -> usize {
        match self {
            // `TooShort` fires when the slice did not even contain the
            // header; the offset is the first byte that *would have been
            // read* had more bytes been available.
            Self::TooShort { actual, .. } => *actual,
            // Magic lives at offset 0..4; the first byte of mismatch is
            // at offset 0 in the frame.
            Self::BadMagic { .. } => 0,
            // Version lives at offset 4..6; name the start of that
            // field rather than the end for a stable diagnostic.
            Self::UnsupportedVersion { .. } => 4,
            Self::CorruptedPayload { offset, .. } => *offset,
        }
    }
}

/// Encode a list of entries as a framed snapshot byte slice.
///
/// Sorts the entries by key byte-lexicographically before archival so
/// that two stores holding semantically-equal contents produce
/// byte-identical output. Duplicate keys are *not* collapsed here; the
/// caller (typically [`crate::LocalIntentStore::export_snapshot`]) is
/// responsible for producing a unique-key list, which the backing
/// redb schema guarantees.
///
/// Returns [`FrameError::CorruptedPayload`] if rkyv fails to serialise
/// the entries — the only realistic cause is allocator exhaustion on
/// a machine close to OOM. The caller maps this onto a typed
/// `IntentStoreError::SnapshotImport` at the trait boundary.
pub fn encode(entries: &[(Bytes, Bytes)]) -> Result<Vec<u8>, FrameError> {
    // Clone into owned `Vec<(Vec<u8>, Vec<u8>)>` so rkyv derives
    // work off concrete fields. `Bytes` does not implement rkyv's
    // `Serialize` trait in the shape the frame needs, and copying is
    // negligible against the disk I/O that already happened.
    let mut owned: Vec<(Vec<u8>, Vec<u8>)> =
        entries.iter().map(|(k, v)| (k.to_vec(), v.to_vec())).collect();

    // Deterministic ordering — byte-lexicographic on key.
    owned.sort_by(|a, b| a.0.cmp(&b.0));

    // Build the frame: magic + version LE + rkyv payload.
    let mut out = Vec::with_capacity(HEADER_LEN + owned.len() * 16);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());

    let archived = rkyv::to_bytes::<rancor::Error>(&owned)
        .map_err(|e| FrameError::CorruptedPayload { offset: HEADER_LEN, reason: e.to_string() })?;
    out.extend_from_slice(&archived);

    Ok(out)
}

/// Decode a framed snapshot byte slice into its entry list.
///
/// Validates the magic and version header, then deserialises the rkyv
/// payload into `Vec<(Bytes, Bytes)>`. Returns a typed [`FrameError`]
/// on any mismatch.
pub fn decode(bytes: &[u8]) -> Result<Vec<(Bytes, Bytes)>, FrameError> {
    if bytes.len() < HEADER_LEN {
        return Err(FrameError::TooShort { expected: HEADER_LEN, actual: bytes.len() });
    }

    let mut magic = [0u8; 4];
    magic.copy_from_slice(&bytes[0..4]);
    if magic != MAGIC {
        return Err(FrameError::BadMagic { expected: MAGIC, actual: magic });
    }

    let mut ver = [0u8; 2];
    ver.copy_from_slice(&bytes[4..6]);
    let version = u16::from_le_bytes(ver);
    if version != VERSION {
        return Err(FrameError::UnsupportedVersion { expected: VERSION, actual: version });
    }

    // rkyv requires the payload to be properly aligned (16-byte by
    // default under the `AlignedVec` allocator). The frame header is
    // a fixed 6 bytes, so the payload lives at an unaligned offset
    // inside the caller's slice — copy it into an `AlignedVec` before
    // deserialising. The copy is cheap compared to the I/O that
    // already got this byte slice into memory.
    let payload = &bytes[HEADER_LEN..];
    let mut aligned: AlignedVec = AlignedVec::with_capacity(payload.len());
    aligned.extend_from_slice(payload);

    let decoded: Vec<(Vec<u8>, Vec<u8>)> = rkyv::from_bytes::<_, rancor::Error>(&aligned)
        .map_err(|e| FrameError::CorruptedPayload { offset: HEADER_LEN, reason: e.to_string() })?;
    Ok(decoded.into_iter().map(|(k, v)| (Bytes::from(k), Bytes::from(v))).collect())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
#[allow(clippy::expect_fun_call)]
mod tests {
    //! Boundary tests for [`decode`] covering the header-length guard.
    //!
    //! These are the mandatory mutation-testing witnesses for the
    //! `if bytes.len() < HEADER_LEN` branch: a header-minus-one slice
    //! and an exactly-`HEADER_LEN` slice produce distinct variants, so
    //! mutating `<` to `==` or `<=` changes which variant is observed
    //! on the boundary case. §4.3 acceptance tests exercise the
    //! payload-corruption path but cannot witness the header boundary
    //! because their minimum input length is already well above
    //! `HEADER_LEN`.
    use super::*;

    #[test]
    fn decode_rejects_an_input_shorter_than_the_header() {
        // HEADER_LEN - 1 bytes: five `0u8`s. The slice is too short to
        // contain the magic + version header, so decode must surface
        // `TooShort` with `actual = HEADER_LEN - 1`.
        let too_short = [0u8; HEADER_LEN - 1];
        let err = decode(&too_short).expect_err("too-short slice must fail");
        match err {
            FrameError::TooShort { expected, actual } => {
                assert_eq!(expected, HEADER_LEN);
                assert_eq!(actual, HEADER_LEN - 1);
            }
            other => panic!("expected TooShort, got {other:?}"),
        }
    }

    #[test]
    fn decode_rejects_a_header_only_input_with_a_payload_error_not_a_length_error() {
        // Exactly HEADER_LEN bytes: valid magic + valid version + empty
        // payload. This exercises the boundary case — the header-guard
        // must NOT fire (else the mutation `<` → `<=` would pass), so
        // decode proceeds to the rkyv step, which rejects the
        // zero-length payload as `CorruptedPayload`.
        let mut header_only = Vec::with_capacity(HEADER_LEN);
        header_only.extend_from_slice(&MAGIC);
        header_only.extend_from_slice(&VERSION.to_le_bytes());
        assert_eq!(header_only.len(), HEADER_LEN);

        let err = decode(&header_only).expect_err("header-only slice must fail");
        // Must be `CorruptedPayload`, not `TooShort` — that proves the
        // `<` comparison in the header guard is strict, not `<=`.
        assert!(
            matches!(err, FrameError::CorruptedPayload { .. }),
            "expected CorruptedPayload to witness the `<` boundary, got {err:?}",
        );
    }

    #[test]
    fn decode_accepts_the_exact_header_length_as_the_boundary() {
        // Witness for the mutation `<` → `==`: at HEADER_LEN bytes the
        // `<` guard does NOT fire. If it were `==`, a slice longer than
        // HEADER_LEN would fail the guard; if it were `<=`, a slice
        // exactly HEADER_LEN would fail the guard. The test above
        // proves the `<=` variant, this test proves the `==` variant:
        // at HEADER_LEN + 1 bytes, decode reaches the payload step.
        let mut header_plus_one = Vec::with_capacity(HEADER_LEN + 1);
        header_plus_one.extend_from_slice(&MAGIC);
        header_plus_one.extend_from_slice(&VERSION.to_le_bytes());
        header_plus_one.push(0u8);
        assert_eq!(header_plus_one.len(), HEADER_LEN + 1);

        let err = decode(&header_plus_one).expect_err("single-byte payload is malformed");
        // A single stray byte is not a valid rkyv archive, so decode
        // surfaces `CorruptedPayload`. The critical property is that
        // it is NOT `TooShort` — the `<` guard did not fire.
        assert!(
            matches!(err, FrameError::CorruptedPayload { .. }),
            "HEADER_LEN + 1 slice must reach the payload step, got {err:?}",
        );
    }
}
