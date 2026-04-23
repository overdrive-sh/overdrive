//! Domain identifiers.
//!
//! Every identifier in Overdrive is a distinct type. Raw strings and
//! integers carrying domain meaning are a project-wide ban — see
//! `.claude/rules/development.md` (*Newtypes — STRICT by default*).
//!
//! # Completeness contract
//!
//! Every newtype in this module implements:
//!
//! * [`FromStr`] — validating, case-insensitive for human-typed IDs.
//! * [`Display`] — the canonical form (lowercase for case-insensitive IDs).
//! * [`Serialize`] / [`Deserialize`] — transparent, matching `Display` /
//!   `FromStr` round-trip.
//! * [`TryFrom<String>`] and `From<Self> for String`.
//! * A `new` constructor that validates and returns `Result`.
//!
//! # What stays case-sensitive
//!
//! [`ContentHash`], [`SchematicId`] (a SHA-256 content hash), and
//! [`CertSerial`] (hex) are case-sensitive — they are not human-typed.

use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

// -----------------------------------------------------------------------------
// Error
// -----------------------------------------------------------------------------

/// Parsing / validation failure for a domain identifier.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdParseError {
    #[error("{kind} must not be empty")]
    Empty { kind: &'static str },

    #[error("{kind} exceeds max length ({max} chars)")]
    TooLong { kind: &'static str, max: usize },

    #[error("{kind} contains invalid character {ch:?} at byte {index}")]
    InvalidChar { kind: &'static str, ch: char, index: usize },

    #[error("{kind} must match format {expected}")]
    InvalidFormat { kind: &'static str, expected: &'static str },

    #[error("SPIFFE ID {0:?} must start with `spiffe://`")]
    SpiffeMissingScheme(String),

    #[error("SPIFFE ID {0:?} has an empty trust domain")]
    SpiffeEmptyTrustDomain(String),

    #[error("SPIFFE ID {0:?} has an empty path")]
    SpiffeEmptyPath(String),

    #[error("content hash must be {expected} hex characters, got {actual}")]
    ContentHashWrongLength { expected: usize, actual: usize },
}

// -----------------------------------------------------------------------------
// DNS-1123-label-like identifiers
// -----------------------------------------------------------------------------
//
// The following newtypes all share the same character class:
//   lowercase ASCII letters, digits, `-`.
//   must start and end with alphanumeric.
//   max 253 chars (DNS name ceiling).
//
// Case-insensitive FromStr; Display emits the lowercased canonical form.

const LABEL_MAX: usize = 253;

fn validate_label(kind: &'static str, raw: &str) -> Result<String, IdParseError> {
    if raw.is_empty() {
        return Err(IdParseError::Empty { kind });
    }
    if raw.len() > LABEL_MAX {
        return Err(IdParseError::TooLong { kind, max: LABEL_MAX });
    }
    let lowered: String = raw.to_ascii_lowercase();
    for (i, ch) in lowered.char_indices() {
        let ok =
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_' || ch == '.';
        if !ok {
            return Err(IdParseError::InvalidChar { kind, ch, index: i });
        }
    }
    // Both of these chain off `is_empty()` rejected above; the iterator
    // is guaranteed to yield. `?`-style fallback would fabricate a second
    // error path that is structurally unreachable.
    let (Some(first), Some(last)) = (lowered.chars().next(), lowered.chars().next_back()) else {
        unreachable!("validate_label non-empty invariant");
    };
    if !first.is_ascii_alphanumeric() || !last.is_ascii_alphanumeric() {
        return Err(IdParseError::InvalidFormat {
            kind,
            expected: "must start and end with an alphanumeric character",
        });
    }
    Ok(lowered)
}

macro_rules! define_label_newtype {
    ($(#[$m:meta])* $name:ident, $kind:literal) => {
        $(#[$m])*
        #[derive(
            Debug,
            Clone,
            PartialEq,
            Eq,
            Hash,
            PartialOrd,
            Ord,
            Serialize,
            Deserialize,
            rkyv::Archive,
            rkyv::Serialize,
            rkyv::Deserialize,
        )]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Construct from a borrowed string, validating and canonicalising.
            pub fn new(raw: &str) -> Result<Self, IdParseError> {
                validate_label($kind, raw).map(Self)
            }

            /// Borrow the canonical string form.
            #[inline]
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;
            fn from_str(raw: &str) -> Result<Self, Self::Err> {
                Self::new(raw)
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdParseError;
            fn try_from(raw: String) -> Result<Self, Self::Error> {
                Self::new(&raw)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = IdParseError;
            fn try_from(raw: &str) -> Result<Self, Self::Error> {
                Self::new(raw)
            }
        }

        impl From<$name> for String {
            fn from(v: $name) -> Self {
                v.0
            }
        }
    };
}

define_label_newtype!(
    /// Identifier for a submitted [`Job`](super) spec.
    JobId, "JobId"
);
define_label_newtype!(
    /// Identifier for a scheduled [`Allocation`](super).
    AllocationId, "AllocationId"
);
define_label_newtype!(
    /// Identifier for a worker / control-plane [`Node`](super).
    NodeId, "NodeId"
);
define_label_newtype!(
    /// Identifier for a [`Policy`](super) (Rego or WASM).
    PolicyId, "PolicyId"
);
define_label_newtype!(
    /// Identifier for a live or archived SRE [`Investigation`](super).
    InvestigationId, "InvestigationId"
);
define_label_newtype!(
    /// Geographical region, e.g. `eu-west-1`.
    Region, "Region"
);

// -----------------------------------------------------------------------------
// SpiffeId
// -----------------------------------------------------------------------------

/// SPIFFE ID for a workload, e.g.
/// `spiffe://overdrive.local/job/payments/alloc/a1b2c3`.
///
/// Construction validates the `spiffe://<trust-domain>/<path>` shape and
/// lowercases the canonical form. The stored value is always lowercased.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SpiffeId {
    canonical: String,
    /// Byte index in `canonical` where the path starts (just after
    /// `spiffe://<trust-domain>`). Enables cheap `trust_domain()` /
    /// `path()` accessors without re-parsing.
    path_start: usize,
}

impl SpiffeId {
    const SCHEME: &'static str = "spiffe://";

    pub fn new(raw: &str) -> Result<Self, IdParseError> {
        let canonical = raw.to_ascii_lowercase();
        let rest = canonical
            .strip_prefix(Self::SCHEME)
            .ok_or_else(|| IdParseError::SpiffeMissingScheme(raw.to_owned()))?;
        let slash = rest.find('/').ok_or_else(|| IdParseError::SpiffeEmptyPath(raw.to_owned()))?;
        if slash == 0 {
            return Err(IdParseError::SpiffeEmptyTrustDomain(raw.to_owned()));
        }
        if slash + 1 >= rest.len() {
            return Err(IdParseError::SpiffeEmptyPath(raw.to_owned()));
        }
        let path_start = Self::SCHEME.len() + slash;
        Ok(Self { canonical, path_start })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.canonical
    }

    /// Trust domain — the segment between `spiffe://` and the first path `/`.
    #[must_use]
    pub fn trust_domain(&self) -> &str {
        &self.canonical[Self::SCHEME.len()..self.path_start]
    }

    /// Path — everything from (and including) the leading `/` onward.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.canonical[self.path_start..]
    }
}

impl Display for SpiffeId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical)
    }
}

impl FromStr for SpiffeId {
    type Err = IdParseError;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::new(raw)
    }
}

impl TryFrom<String> for SpiffeId {
    type Error = IdParseError;
    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::new(&raw)
    }
}

impl TryFrom<&str> for SpiffeId {
    type Error = IdParseError;
    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        Self::new(raw)
    }
}

impl From<SpiffeId> for String {
    fn from(v: SpiffeId) -> Self {
        v.canonical
    }
}

// -----------------------------------------------------------------------------
// ContentHash — SHA-256 (32 bytes, 64 hex chars). Case-sensitive.
// -----------------------------------------------------------------------------

const CONTENT_HASH_HEX_LEN: usize = 64;

/// SHA-256 content hash, rendered as 64 lowercase hex characters.
///
/// Used for every piece of content-addressed data: WASM modules, chunks
/// in `overdrive-fs`, VM images, Raft-log snapshots, diagnostic-probe
/// catalogue entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Construct from a raw 32-byte digest.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrow the raw 32 bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Hash arbitrary data under SHA-256.
    #[must_use]
    pub fn of(data: impl AsRef<[u8]>) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(data.as_ref());
        let digest: [u8; 32] = hasher.finalize().into();
        Self(digest)
    }

    /// Parse from a 64-char lowercase hex string.
    pub fn from_hex(hex_str: &str) -> Result<Self, IdParseError> {
        if hex_str.len() != CONTENT_HASH_HEX_LEN {
            return Err(IdParseError::ContentHashWrongLength {
                expected: CONTENT_HASH_HEX_LEN,
                actual: hex_str.len(),
            });
        }
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(hex_str, &mut bytes).map_err(|_| IdParseError::InvalidFormat {
            kind: "ContentHash",
            expected: "lowercase hex, 64 chars",
        })?;
        Ok(Self(bytes))
    }
}

impl Display for ContentHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for ContentHash {
    type Err = IdParseError;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::from_hex(raw)
    }
}

impl TryFrom<String> for ContentHash {
    type Error = IdParseError;
    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::from_hex(&raw)
    }
}

impl TryFrom<&str> for ContentHash {
    type Error = IdParseError;
    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        Self::from_hex(raw)
    }
}

impl From<ContentHash> for String {
    fn from(v: ContentHash) -> Self {
        v.to_string()
    }
}

// -----------------------------------------------------------------------------
// SchematicId — content hash of a schematic struct. Distinct type for safety.
// -----------------------------------------------------------------------------

/// Image Factory schematic identifier — SHA-256 of the canonical schematic
/// bytes.
///
/// # Canonicalisation — ADR-0002
///
/// `SchematicId` is the SHA-256 of the **rkyv-archived bytes** of the
/// `Schematic` struct, per
/// [ADR-0002 — *`SchematicId` canonicalisation uses rkyv-archived bytes*](
/// ../../../docs/product/architecture/adr-0002-schematic-id-canonicalisation.md).
/// The rkyv archival format is canonical by construction — field order
/// matches the Rust struct definition, no whitespace, no map-key
/// reordering, no float-format variance — which makes the resulting
/// hash deterministic across machines, toolchain versions, and Rust
/// editions.
///
/// JSON/RFC-8785 (JCS) was considered and explicitly rejected for this
/// identifier: the `Schematic` is an internal Overdrive concept with
/// no cross-toolchain consumer, and `development.md`'s hashing guidance
/// ("Internal data → rkyv") places it unambiguously in the rkyv bucket.
///
/// # Phase 1 status
///
/// Phase 1 ships `SchematicId` as a transparent [`ContentHash`] newtype.
/// The `Schematic` struct that it canonicalises — and therefore the
/// concrete `rkyv::to_bytes::<_, 256>(&schematic)?` call site — is
/// deferred to Phase 2 (Image Factory §23 of the whitepaper). Phase 1's
/// contribution is the newtype and the rule documented here, so a
/// future implementer cannot adopt a different canonicalisation without
/// superseding ADR-0002.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SchematicId(ContentHash);

impl SchematicId {
    #[must_use]
    pub const fn new(hash: ContentHash) -> Self {
        Self(hash)
    }

    #[must_use]
    pub const fn content_hash(&self) -> &ContentHash {
        &self.0
    }
}

impl Display for SchematicId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl FromStr for SchematicId {
    type Err = IdParseError;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        ContentHash::from_str(raw).map(Self)
    }
}

// -----------------------------------------------------------------------------
// CertSerial — hex-encoded X.509 serial, case-sensitive, variable length.
// -----------------------------------------------------------------------------

const CERT_SERIAL_MAX_BYTES: usize = 20; // RFC 5280 §4.1.2.2

/// Hex-encoded X.509 certificate serial number (≤ 20 bytes per RFC 5280).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CertSerial(String);

impl CertSerial {
    pub fn new(raw: &str) -> Result<Self, IdParseError> {
        if raw.is_empty() {
            return Err(IdParseError::Empty { kind: "CertSerial" });
        }
        if raw.len() % 2 != 0 {
            return Err(IdParseError::InvalidFormat {
                kind: "CertSerial",
                expected: "even number of hex digits",
            });
        }
        if raw.len() > CERT_SERIAL_MAX_BYTES * 2 {
            return Err(IdParseError::TooLong {
                kind: "CertSerial",
                max: CERT_SERIAL_MAX_BYTES * 2,
            });
        }
        for (i, ch) in raw.char_indices() {
            if !ch.is_ascii_hexdigit() || ch.is_ascii_uppercase() {
                return Err(IdParseError::InvalidChar { kind: "CertSerial", ch, index: i });
            }
        }
        Ok(Self(raw.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for CertSerial {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for CertSerial {
    type Err = IdParseError;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::new(raw)
    }
}

impl TryFrom<String> for CertSerial {
    type Error = IdParseError;
    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::new(&raw)
    }
}

impl From<CertSerial> for String {
    fn from(v: CertSerial) -> Self {
        v.0
    }
}

// -----------------------------------------------------------------------------
// CorrelationKey — derived from (target, spec_hash, purpose). See §18.
// -----------------------------------------------------------------------------

/// Correlation key for external-I/O calls emitted from reconcilers.
///
/// Derived deterministically from `(target, spec_hash, purpose)`. The next
/// reconcile iteration finds the prior call's response by looking up the
/// same key in the `ObservationStore` — decoupling cause from transient
/// request IDs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CorrelationKey(String);

impl CorrelationKey {
    /// Derive a key from its three logical components.
    ///
    /// The inputs are hashed into a content-addressed suffix so the key is
    /// deterministic across processes and nodes.
    #[must_use]
    pub fn derive(target: &str, spec_hash: &ContentHash, purpose: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(target.as_bytes());
        hasher.update([0u8]);
        hasher.update(spec_hash.as_bytes());
        hasher.update([0u8]);
        hasher.update(purpose.as_bytes());
        let digest: [u8; 32] = hasher.finalize().into();
        // First 12 bytes is enough to disambiguate within a cluster's
        // lifetime while keeping keys readable in logs.
        let mut encoded = String::with_capacity(1 + target.len() + 1 + purpose.len() + 1 + 24);
        encoded.push_str(target);
        encoded.push(':');
        encoded.push_str(purpose);
        encoded.push('/');
        for byte in &digest[..12] {
            // `write!` into a `String` is infallible — the `fmt::Result`
            // it returns is only ever `Ok`. Using `_ = ...` avoids an
            // `expect` without fabricating error handling.
            let _ = write!(encoded, "{byte:02x}");
        }
        Self(encoded)
    }

    pub fn new(raw: &str) -> Result<Self, IdParseError> {
        if raw.is_empty() {
            return Err(IdParseError::Empty { kind: "CorrelationKey" });
        }
        if raw.len() > LABEL_MAX {
            return Err(IdParseError::TooLong { kind: "CorrelationKey", max: LABEL_MAX });
        }
        Ok(Self(raw.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for CorrelationKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for CorrelationKey {
    type Err = IdParseError;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::new(raw)
    }
}

impl TryFrom<String> for CorrelationKey {
    type Error = IdParseError;
    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::new(&raw)
    }
}

impl From<CorrelationKey> for String {
    fn from(v: CorrelationKey) -> Self {
        v.0
    }
}

use std::fmt::Write as _;

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_rejects_empty() {
        assert!(matches!(JobId::new(""), Err(IdParseError::Empty { .. })));
    }

    #[test]
    fn label_is_case_insensitive_on_parse_and_lowercases_canonical() {
        let parsed = JobId::new("Payments").unwrap();
        assert_eq!(parsed.as_str(), "payments");
    }

    #[test]
    fn label_rejects_invalid_char() {
        let err = NodeId::new("bad!name").unwrap_err();
        assert!(matches!(err, IdParseError::InvalidChar { .. }));
    }

    #[test]
    fn spiffe_parses_canonical_form() {
        let id = SpiffeId::new("spiffe://overdrive.local/job/payments/alloc/a1b2c3").unwrap();
        assert_eq!(id.trust_domain(), "overdrive.local");
        assert_eq!(id.path(), "/job/payments/alloc/a1b2c3");
    }

    #[test]
    fn spiffe_requires_scheme() {
        let err = SpiffeId::new("overdrive.local/job/x").unwrap_err();
        assert!(matches!(err, IdParseError::SpiffeMissingScheme(_)));
    }

    #[test]
    fn content_hash_round_trips_through_hex() {
        let h = ContentHash::of(b"overdrive");
        let s = h.to_string();
        assert_eq!(s.len(), 64);
        assert_eq!(ContentHash::from_hex(&s).unwrap(), h);
    }

    #[test]
    fn content_hash_rejects_wrong_length() {
        let err = ContentHash::from_hex("abc").unwrap_err();
        assert!(matches!(err, IdParseError::ContentHashWrongLength { .. }));
    }

    #[test]
    fn correlation_key_is_deterministic() {
        let h = ContentHash::of(b"spec");
        let a = CorrelationKey::derive("payments", &h, "register");
        let b = CorrelationKey::derive("payments", &h, "register");
        assert_eq!(a, b);
    }

    #[test]
    fn cert_serial_rejects_uppercase_hex() {
        assert!(matches!(CertSerial::new("ABCD"), Err(IdParseError::InvalidChar { .. })));
    }

    #[test]
    fn serde_round_trips_job_id() {
        let id = JobId::new("payments").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"payments\"");
        let back: JobId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }
}
