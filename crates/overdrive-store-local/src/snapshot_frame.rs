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
//! [`decode`] validates magic and version, then parses the payload via
//! `rkyv::from_bytes`. Any mismatch — wrong magic, unknown version,
//! truncated payload, corrupted rkyv bytes — surfaces as a typed
//! [`FrameError`]. Step 03-02 does not assert on specific error
//! variants (that is step 03-03's scope); this module is written to
//! make step 03-03's job mechanical.
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

/// Current frame version. Stored at offset 4..6 as a little-endian
/// `u16`. Bumping this is a cross-crate breaking change — `RaftStore`
/// consumes the same frame.
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
    #[error("snapshot frame payload is corrupted: {0}")]
    CorruptedPayload(String),
}

/// Encode a list of entries as a framed snapshot byte slice.
///
/// Sorts the entries by key byte-lexicographically before archival so
/// that two stores holding semantically-equal contents produce
/// byte-identical output. Duplicate keys are *not* collapsed here; the
/// caller (typically [`crate::LocalStore::export_snapshot`]) is
/// responsible for producing a unique-key list, which the backing
/// redb schema guarantees.
///
/// Returns [`FrameError::CorruptedPayload`] if rkyv fails to serialise
/// the entries — the only realistic cause is allocator exhaustion on
/// a machine close to OOM. The caller maps this onto a typed
/// `IntentStoreError::SnapshotImport` at the trait boundary.
pub fn encode(entries: &[(Bytes, Bytes)]) -> Result<Vec<u8>, FrameError> {
    // Clone into owned `Vec<(Vec<u8>, Vec<u8>)>` so rkyv derives work
    // off concrete `Vec<u8>` fields. `Bytes` does not implement rkyv's
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
        .map_err(|e| FrameError::CorruptedPayload(e.to_string()))?;
    out.extend_from_slice(&archived);

    Ok(out)
}

/// Decode a framed snapshot byte slice into its entry list.
///
/// Validates the magic and version header, then parses the rkyv
/// payload. Returns a typed [`FrameError`] on any mismatch — step
/// 03-03 asserts on the specific variants for corruption paths.
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
        .map_err(|e| FrameError::CorruptedPayload(e.to_string()))?;

    Ok(decoded.into_iter().map(|(k, v)| (Bytes::from(k), Bytes::from(v))).collect())
}
