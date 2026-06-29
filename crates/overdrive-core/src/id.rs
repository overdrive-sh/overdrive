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
use std::net::SocketAddrV4;
use std::num::NonZeroU16;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::dataplane::backend_key::Proto;

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

/// The shared length ceiling for every label-shaped identifier in
/// Overdrive — the DNS-name maximum (RFC 1035). Every DNS-1123-label-like
/// newtype below (`SpiffeId`, `JobId`, `NodeId`, `Region`, …) and
/// [`CorrelationKey`] is bounded by this single const.
///
/// It is `pub` so that *derived* identifiers in downstream crates can size
/// their own ceilings against it rather than inventing a second, smaller
/// magic number. `WorkflowId` (in `overdrive-control-plane`) is the
/// canonical case: it is derived from a [`CorrelationKey`] and MUST be
/// able to hold the full mapped key without truncation — see
/// `.claude/rules/development.md` § "One shared length ceiling for
/// label-shaped ids". A bespoke smaller ceiling there silently
/// re-introduced a journal-key collision (two distinct correlation keys
/// sharing a truncated prefix collapsed to one id).
pub const LABEL_MAX: usize = 253;

/// The DNS single-*label* octet maximum (RFC 1035 §2.3.4): one label of a
/// domain name is hard-capped at 63 octets, distinct from [`LABEL_MAX`] (253),
/// which is the DNS-*name* (whole FQDN) maximum (RFC 1035 §3.1).
///
/// This is a real protocol constant — the `hickory-proto` codec enforces it on
/// the wire — mirroring the codebase's existing `RECONCILER_NAME_MAX` /
/// `WORKFLOW_NAME_MAX` (both 63). It bounds a [`MeshServiceName`]'s `<job>`,
/// which is a single DNS label (the first label of
/// `<job>.svc.overdrive.local`); a `<job>` longer than 63 octets would make
/// `hickory_proto::rr::Name::from_str` reject the rendered name and panic the
/// responder's wire encoder. NOT a "bespoke smaller ceiling" of [`LABEL_MAX`]
/// (`development.md` § "One shared length ceiling for label-shaped ids") — that
/// rule governs *derived* label-shaped ids sized off the DNS-name max; a DNS
/// *label* legitimately uses its own RFC-1035 protocol limit.
pub const DNS_LABEL_OCTET_MAX: usize = 63;

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
    /// Identifier for a submitted workload (Job, Service, or Schedule).
    WorkloadId, "WorkloadId"
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

    /// Derive the SVID identity for a workload allocation:
    /// `spiffe://overdrive.local/job/<workload>/alloc/<alloc>`.
    ///
    /// The canonical allocation-shaped constructor (ADR-0067 D5) — the single
    /// derivation the reconcilers route through, replacing the formerly
    /// duplicated private `mint_alloc_identity` / `mint_identity` helpers.
    ///
    /// Infallible by construction: [`WorkloadId`] and [`AllocationId`] are
    /// already-validated DNS-1123-label-like newtypes whose canonical form is
    /// lowercase ASCII alphanumerics plus `-` / `_` / `.`, leading and trailing
    /// alphanumeric. Interpolated into the fixed `spiffe://overdrive.local/job/
    /// .../alloc/...` shape, the result always has the `spiffe://` scheme, the
    /// non-empty trust domain `overdrive.local`, and a non-empty path — so
    /// [`SpiffeId::new`] cannot reject it. The logically-unreachable
    /// [`unreachable!`] idiom (per `.claude/rules/development.md` § "Logically
    /// unreachable `None` / `Err`") states that invariant; `?` would imply a
    /// valid early-return path and `.expect()` is banned in production library
    /// code.
    #[must_use]
    pub fn for_allocation(workload: &WorkloadId, alloc: &AllocationId) -> Self {
        let raw =
            format!("spiffe://overdrive.local/job/{}/alloc/{}", workload.as_str(), alloc.as_str());
        Self::new(&raw).unwrap_or_else(|_| {
            unreachable!(
                "for_allocation derives a SpiffeId from already-validated \
                 WorkloadId/AllocationId newtypes interpolated into the fixed \
                 spiffe://overdrive.local/job/.../alloc/... shape; SpiffeId::new \
                 cannot reject a valid scheme + non-empty trust domain + \
                 non-empty path"
            )
        })
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
///
/// rkyv-archivable so it can be persisted as a field of rkyv-versioned
/// observation rows (e.g. `IssuedCertificateRowV1`, built-in-ca / GH #28)
/// at the durable-storage boundary — mirroring `SpiffeId` / `NodeId`,
/// which carry the same derives for the same reason.
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
pub struct CertSerial(String);

impl CertSerial {
    pub fn new(raw: &str) -> Result<Self, IdParseError> {
        if raw.is_empty() {
            return Err(IdParseError::Empty { kind: "CertSerial" });
        }
        if !raw.len().is_multiple_of(2) {
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
// IssuanceOrdinal — strictly-monotonic global issuance-order rank.
// -----------------------------------------------------------------------------

/// Strictly-monotonic global issuance ordinal — the issuance-order rank of an
/// `issued_certificates` audit row.
///
/// The consumer-side "current cert" projection selects max-ordinal per SPIFFE
/// ID, so a later issuance always outranks an earlier one even when their
/// `issued_at` clock reads tie (the equal-`issued_at` same-tick re-issue the
/// deterministic `SimClock` produces). An issuance-order **fact**, not derived
/// state (`.claude/rules/development.md` § "Persist inputs, not derived
/// state"): persisted on the row, never recomputed.
///
/// # Wire form
///
/// `Display` emits the decimal `u64`; `FromStr` parses a decimal `u64` into a
/// structured [`IdParseError`]. `Serialize` / `Deserialize` go through the
/// base-10 **string** form (`#[serde(try_from = "String", into = "String")]`),
/// matching `Display` / `FromStr` exactly and mirroring [`CertSerial`] — the
/// canonical wire form is the string, NOT a transparent raw integer. There is
/// no case axis (numeric identifier).
#[derive(
    Clone,
    Copy,
    Debug,
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
pub struct IssuanceOrdinal(u64);

impl IssuanceOrdinal {
    /// Construct an ordinal from its raw `u64` rank.
    ///
    /// Infallible: every `u64` is a valid issuance ordinal, so there is no
    /// value to reject. This is the sanctioned exception to the project's
    /// "no infallible `new()`" newtype rule (`development.md` § Newtype
    /// completeness) — a `Result` here would be dishonest, since no input
    /// can fail.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The raw `u64` rank.
    ///
    /// The `&self` receiver is the shape pinned by feature-delta § D1-AMEND-1
    /// ("exactly `pub const fn as_u64(&self) -> u64`"). `IssuanceOrdinal` is
    /// `Copy`, so clippy's `trivially_copy_pass_by_ref` would prefer `self`;
    /// the pinned contract is honored verbatim and the lint scoped here.
    #[must_use]
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "accessor signature pinned verbatim by feature-delta § D1-AMEND-1"
    )]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

impl Display for IssuanceOrdinal {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl FromStr for IssuanceOrdinal {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(IdParseError::Empty { kind: "IssuanceOrdinal" });
        }
        s.parse::<u64>().map(Self).map_err(|_| IdParseError::InvalidFormat {
            kind: "IssuanceOrdinal",
            expected: "decimal u64 (0..=18446744073709551615)",
        })
    }
}

impl TryFrom<String> for IssuanceOrdinal {
    type Error = IdParseError;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        raw.parse()
    }
}

impl From<IssuanceOrdinal> for String {
    fn from(v: IssuanceOrdinal) -> Self {
        v.0.to_string()
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
// MeshServiceName — `<job>.svc.overdrive.local` mesh-DNS grammar.
// -----------------------------------------------------------------------------

/// Mesh service name — the `<job>.svc.overdrive.local` grammar a workload
/// dials to reach another workload by name (dial-by-name-responder,
/// ADR-0072 / US-DBN-2).
///
/// The newtype carries a validated `<job>` label; the fixed
/// [`SUFFIX`](Self::SUFFIX) is grammar, not stored state. [`as_str`](Self::as_str)
/// returns the canonical lowercase `<job>` label; [`Display`] renders the
/// full `<job>.svc.overdrive.local` name.
///
/// # Grammar
///
/// The bespoke [`FromStr`] enforces a suffix grammar that the shared
/// [`validate_label`] cannot: `validate_label` permits `.` (so `JobId` may
/// be `region.eu-west-1`), which means it would accept any dotted string —
/// it cannot reject the wrong suffix. `MeshServiceName::new` therefore
/// case-folds, strips the terminal `.svc.overdrive.local`, and validates
/// the remaining `<job>` label through `validate_label` (reusing the shared
/// DNS-1123 character-class + [`LABEL_MAX`] ceiling — no bespoke smaller
/// magic number, per `.claude/rules/development.md` § "One shared length
/// ceiling for label-shaped ids").
///
/// # Completeness
///
/// Case-insensitive parse, lowercase canonical `Display`, serde matching
/// `Display`/`FromStr` exactly — the standard human-typed-id shape.
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
pub struct MeshServiceName(String);

impl MeshServiceName {
    /// The fixed mesh-DNS suffix — note NO leading dot; the separator is the
    /// `.` between the `<job>` label and this suffix.
    pub const SUFFIX: &'static str = "svc.overdrive.local";

    /// Construct from a full `<job>.svc.overdrive.local` name, validating the
    /// suffix grammar and canonicalising the `<job>` label to lowercase.
    ///
    /// Case-folds, strips the terminal `.svc.overdrive.local` (the wrong /
    /// missing / non-terminal suffix all surface as an
    /// [`IdParseError::InvalidFormat`]), caps the `<job>` at the DNS
    /// single-label octet limit [`DNS_LABEL_OCTET_MAX`] (63 octets — RFC 1035
    /// §2.3.4; corrected ADR-0072 DDN-7), then validates the remaining `<job>`
    /// label through the shared [`validate_label`] — reusing the DNS-1123
    /// character class and the start/end-alphanumeric rule. An empty,
    /// over-63-octet, or out-of-class label maps to the existing `Empty` /
    /// `TooLong { max: 63 }` / `InvalidChar` / `InvalidFormat` variants.
    ///
    /// The 63-octet cap is the DNS-*label* maximum, NOT [`LABEL_MAX`] (253, the
    /// DNS-*name* max) — a single label is hard-capped at 63 on the wire, so a
    /// longer `<job>` would make `hickory_proto::rr::Name::from_str` reject the
    /// rendered name and panic the responder's encoder. This is a real protocol
    /// constant (mirroring `RECONCILER_NAME_MAX` / `WORKFLOW_NAME_MAX` = 63),
    /// not a bespoke-smaller ceiling of [`LABEL_MAX`] (`development.md` § "One
    /// shared length ceiling for label-shaped ids" governs *derived* ids sized
    /// off the DNS-name max; a DNS label uses its own RFC-1035 limit).
    ///
    /// **v1 single-label contract (ADR-0072:279):** the `<job>` is a *single*
    /// label — single-node, NO namespace segment. The shared [`validate_label`]
    /// PERMITS `.` (id.rs:102) because other label newtypes (`WorkloadId` /
    /// `NodeId`) legitimately carry dotted forms (`region.eu-west-1`); so this
    /// constructor adds a single-label guard ON TOP of `validate_label` — a
    /// dotted `<job>` (e.g. `foo.bar.svc.overdrive.local` → `<job>` =
    /// `"foo.bar"`) is rejected as [`IdParseError::InvalidChar`] at the
    /// offending `.`. A dotted deploy-spec service id is therefore simply not
    /// mesh-dialable by name in v1, which is exactly the design's intended
    /// scope (NO namespace segment), not a regression.
    pub fn new(raw: &str) -> Result<Self, IdParseError> {
        const KIND: &str = "MeshServiceName";
        let lowered = raw.to_ascii_lowercase();
        // The separator-plus-suffix that a valid name must end with:
        // ".svc.overdrive.local". Stripping it leaves the `<job>` label.
        let job = lowered.strip_suffix(Self::SUFFIX).and_then(|s| s.strip_suffix('.')).ok_or(
            IdParseError::InvalidFormat { kind: KIND, expected: "<job>.svc.overdrive.local" },
        )?;
        // Single-label guard: the v1 `<job>` is ONE label with no namespace
        // segment. `validate_label` would accept an interior `.`, so reject a
        // dotted `<job>` here (at the offending `.`'s position within the
        // `<job>` part) BEFORE delegating the remaining (now dot-free) rules.
        if let Some(index) = job.find('.') {
            return Err(IdParseError::InvalidChar { kind: KIND, ch: '.', index });
        }
        // DNS single-label octet cap: the `<job>` is one DNS LABEL (the first
        // label of `<job>.svc.overdrive.local`), hard-capped at
        // `DNS_LABEL_OCTET_MAX` = 63 octets (RFC 1035 §2.3.4; corrected
        // ADR-0072 DDN-7, 2026-06-25). This is a real protocol constant, NOT a
        // bespoke-smaller ceiling of `LABEL_MAX` — a `<job>` longer than 63
        // octets would make `hickory_proto::rr::Name::from_str` reject the
        // rendered name and panic the responder's wire encoder. The check fires
        // UNIFORMLY ahead of `validate_label`'s 253 ceiling, so EVERY over-63
        // label (64, 100, 300, …) reports `TooLong { max: 63 }` — never
        // `max: 253` for the 254+ range.
        if job.len() > DNS_LABEL_OCTET_MAX {
            return Err(IdParseError::TooLong { kind: KIND, max: DNS_LABEL_OCTET_MAX });
        }
        // `validate_label` re-lowercases (a no-op here) and enforces the
        // DNS-1123 label rules (empty / character-class / start-end-alphanumeric)
        // on the now-dot-free, ≤ 63-octet `<job>` part. The reported `kind` is
        // "MeshServiceName" so the error names the rejecting newtype, not the
        // inner helper. (`validate_label`'s own `LABEL_MAX` = 253 check is
        // structurally unreachable here — the 63-octet guard above already
        // rejected anything longer.)
        validate_label(KIND, job).map(Self)
    }

    /// Borrow the canonical lowercase `<job>` label.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for MeshServiceName {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.0, Self::SUFFIX)
    }
}

impl FromStr for MeshServiceName {
    type Err = IdParseError;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::new(raw)
    }
}

impl TryFrom<String> for MeshServiceName {
    type Error = IdParseError;
    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::new(&raw)
    }
}

impl TryFrom<&str> for MeshServiceName {
    type Error = IdParseError;
    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        Self::new(raw)
    }
}

impl From<MeshServiceName> for String {
    fn from(v: MeshServiceName) -> Self {
        v.to_string()
    }
}

// -----------------------------------------------------------------------------
// NameAnswer — the pure v1 DNS query result for the dial-by-name responder.
// -----------------------------------------------------------------------------

/// The pure, wire-free result of a dial-by-name query (dial-by-name-responder,
/// ADR-0072 DDN-4 / D-DBN-2). `answer_for(name, qtype, &index)` returns one of
/// these three variants; `dns_responder::wire::encode` later renders it to DNS
/// bytes. The variant names ARE the contract (ADR-0072 § Pinned signatures).
///
/// # Home-module choice (ADR-0072 § Components latitude)
///
/// ADR-0072 § Component decomposition grants DELIVER latitude on `NameAnswer`'s
/// home — `crates/overdrive-core/src/` "(`id.rs` or a small `dns` module)".
/// This step lands it in `id.rs` alongside [`MeshServiceName`] (the name
/// layer's other core type): the two are the dial-by-name domain vocabulary,
/// co-locating them keeps the name-layer types in one place, avoids a new
/// `lib.rs` `pub mod dns;` re-export, and matches the first-named option in the
/// Component table. Reachable as `overdrive_core::id::NameAnswer`.
///
/// # Hickory-free (DDN-4 / D-DBN-5 ACL boundary)
///
/// `NameAnswer` names ONLY std types (`SocketAddrV4`). It MUST NOT reference any
/// `hickory_proto` type — `overdrive-core` stays hickory-free; only `wire.rs`
/// (in `overdrive-control-plane`) crosses the codec boundary.
///
/// # The v1 answer contract
///
/// - `Records(addrs)` — the name has ≥1 running-AND-healthy IPv4 backend; an
///   `A` query is answered with one A record per addr.
/// - `NoData` — the name IS currently resolvable (≥1 running-and-healthy
///   backend) but has no record of the queried type (`AAAA` in v1, since the
///   substrate is IPv4): NOERROR with no answer + a negative-TTL SOA.
/// - `NxDomain` — 0 running-and-healthy backends (declared-but-not-running,
///   unhealthy, OR unknown all collapse in v1): NXDOMAIN + a negative-TTL SOA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameAnswer {
    /// ≥1 running-AND-healthy IPv4 backend → one A record per addr.
    Records(Vec<SocketAddrV4>),
    /// Live name, no record of the queried type (AAAA in v1) → NODATA + SOA.
    NoData,
    /// 0 running-and-healthy backends → NXDOMAIN + SOA.
    NxDomain,
}

// -----------------------------------------------------------------------------
// Phase 2.2 newtypes — `ServiceVip`, `ServiceId`, `BackendId`.
// RED scaffolds per `docs/feature/phase-2-xdp-service-map/distill/
// wave-decisions.md` DWD-4. Bodies panic until DELIVER fills them
// per the carpaccio slice plan (Slice 02 / Slice 04).
// -----------------------------------------------------------------------------

/// Virtual IP a kernel-side XDP program matches incoming packets
/// against. Stored host-order; converted at the kernel boundary
/// per architecture.md § 11.
///
/// Userspace control-plane newtype only — `service_backends`
/// observation rows continue to carry `vip: Ipv4Addr` as their
/// wire-shape field; the hydrator wraps at the read boundary
/// (architecture.md § 5).
///
/// # Wire form
///
/// `Display` emits the canonical `IpAddr` string form (e.g.
/// `10.0.0.1`, `::1`). `FromStr` parses any [`std::net::IpAddr`]-
/// compatible string. Empty input and non-IP strings surface as
/// structured [`IdParseError`] variants.
#[derive(
    Clone,
    Copy,
    Debug,
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
    utoipa::ToSchema,
)]
#[serde(try_from = "String", into = "String")]
#[schema(value_type = String, example = "10.0.0.1")]
pub struct ServiceVip(std::net::IpAddr);

impl ServiceVip {
    /// Validating constructor over a [`std::net::IpAddr`].
    ///
    /// IPv4 is always accepted today; IPv6 is also accepted at
    /// the type level (per architecture.md § 6 the IPv6 *kernel-
    /// side* path is GH #155 deferral, not a userspace newtype
    /// concern).
    ///
    /// The `Result` return is the project's newtype-completeness
    /// shape (`development.md` § Newtype completeness — *No
    /// infallible `new()` that silently accepts garbage*); even
    /// where every input is currently valid, the return shape is
    /// stable so future range-checks (e.g. rejecting multicast
    /// or unspecified addresses) land additively.
    #[allow(clippy::unnecessary_wraps, clippy::missing_const_for_fn)]
    pub fn new(addr: std::net::IpAddr) -> Result<Self, IdParseError> {
        Ok(Self(addr))
    }

    /// Inner [`std::net::IpAddr`].
    #[must_use]
    pub const fn get(&self) -> std::net::IpAddr {
        self.0
    }

    /// Fallible projection to [`std::net::Ipv4Addr`]. Returns `Some`
    /// when the underlying address is IPv4, `None` for IPv6.
    ///
    /// Phase 1 dataplane code paths (per ADR-0049 § 5) work
    /// exclusively in IPv4; this accessor is the structural seam
    /// between the canonical type (which admits IPv6 forward-compat
    /// per GH #155) and the IPv4-only allocator / `service_backends`
    /// row surface. Maps the older `ipv4_from_vip` helper at
    /// `crates/overdrive-control-plane/src/action_shim/dataplane_update_service.rs:160`
    /// onto the newtype.
    #[must_use]
    pub const fn try_as_ipv4(&self) -> Option<std::net::Ipv4Addr> {
        // mutants: skip — the `None` arm is structurally unreachable in
        // Phase 1: `ServiceVip` is exclusively constructed as IPv4 via the
        // allocator (`VipRange` is IPv4-only per ADR-0049 § 5) and the
        // parser layer does not yet admit IPv6 literals. IPv6
        // forward-compat is tracked in GH #155; the corresponding kill
        // test lands the same commit that admits an IPv6 path.
        match self.0 {
            std::net::IpAddr::V4(v4) => Some(v4),
            std::net::IpAddr::V6(_) => None,
        }
    }
}

impl Display for ServiceVip {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl FromStr for ServiceVip {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(IdParseError::Empty { kind: "ServiceVip" });
        }
        // Accept upper- / lower-case hex digits in IPv6 inputs and
        // delegate to `IpAddr::from_str`. The canonical Display form
        // emitted by `IpAddr` is already lowercase.
        let canonical = s.to_ascii_lowercase();
        let addr =
            std::net::IpAddr::from_str(&canonical).map_err(|_| IdParseError::InvalidFormat {
                kind: "ServiceVip",
                expected: "an IPv4 or IPv6 address (e.g. 10.0.0.1)",
            })?;
        Ok(Self(addr))
    }
}

impl TryFrom<String> for ServiceVip {
    type Error = IdParseError;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::from_str(&raw)
    }
}

impl TryFrom<&str> for ServiceVip {
    type Error = IdParseError;

    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        Self::from_str(raw)
    }
}

impl From<ServiceVip> for String {
    fn from(v: ServiceVip) -> Self {
        v.to_string()
    }
}

/// Identity of a service for control-plane addressing. Maps 1:1
/// to a `MAGLEV_MAP` outer-map key; backed by `u64` content-hash
/// per architecture.md § 6 (the `(VIP, port, scope)` content-hash
/// is computed upstream — the newtype itself is opaque).
///
/// # Wire form
///
/// `Display` emits the decimal `u64` representation. `FromStr`
/// parses decimal `u64`. There is no case axis; the
/// case-insensitivity rule from `development.md` § Newtype
/// completeness applies only to human-typed string identifiers
/// (matches the `BackendId` / `MaglevTableSize` precedent).
#[derive(
    Clone,
    Copy,
    Debug,
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
#[serde(transparent)]
pub struct ServiceId(u64);

impl ServiceId {
    /// Validating constructor over the raw `u64`. Every `u64` is a
    /// valid `ServiceId` — the newtype's role is type-system
    /// distinctness, not runtime range-check. The `Result` return
    /// is the project's newtype-completeness shape — see
    /// [`ServiceVip::new`] for the same rationale.
    #[allow(clippy::unnecessary_wraps, clippy::missing_const_for_fn)]
    pub fn new(value: u64) -> Result<Self, IdParseError> {
        Ok(Self(value))
    }

    /// Inner `u64`.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Derive a content-addressed `ServiceId` from
    /// `(vip, port, proto, purpose)` per ADR-0052 § 1 / ADR-0040
    /// `## Revision 2026-06-03 (companion)`.
    ///
    /// The bytes hashed are the canonical wire encoding of each
    /// input, separated by zero bytes to avoid ambiguous boundaries
    /// (mirrors `CorrelationKey::derive`):
    ///
    /// 1. `vip.to_string().as_bytes()` — `ServiceVip`'s `Display`
    ///    impl is the canonical wire form (`IpAddr::fmt`-derived).
    /// 2. `port.get().to_be_bytes()` — big-endian `u16` so the byte
    ///    sequence is stable across host endianness.
    /// 3. `[proto.as_u8()]` — the IANA L4 protocol byte (TCP=6,
    ///    UDP=17). This is the **proto axis** added by the Model A
    ///    widening: two listeners on the same `(vip, port)` but
    ///    different protocol (the canonical CoreDNS `tcp/53` +
    ///    `udp/53` case) derive DISTINCT `ServiceId`s instead of
    ///    colliding. Inserted at field 5 — after the `port`
    ///    separator, before `purpose` — to match P2-Q4's proto-keyed
    ///    dataplane slots ([`crate::dataplane::backend_key::Proto`]).
    /// 4. `purpose.as_bytes()` — caller-supplied namespacing token,
    ///    canonically `"service-map"` for the bridge.
    ///
    /// The first 8 bytes of the SHA-256 digest are interpreted as a
    /// big-endian `u64` and wrapped in `ServiceId` — unchanged by the
    /// proto-widening, so the rkyv layout of `ServiceId` (a `u64`) is
    /// untouched and NO envelope version bump is warranted. The full
    /// 64 bits give ample collision resistance — `2^32` distinct
    /// `(vip, port, proto)` triples collide with probability ~`2^-32`
    /// (the birthday bound on a 64-bit space), and the project's
    /// production cardinality is far below that.
    ///
    /// Per `.claude/rules/development.md` § "Hashing requires
    /// deterministic serialization": the inputs are wrapped in a
    /// canonical wire form before hashing — `Display` for `ServiceVip`
    /// (deterministic per `IpAddr::fmt`), big-endian bytes for `u16`,
    /// the single IANA byte for `Proto`, raw bytes for the string. No
    /// `serde_json::to_string` is in the loop.
    #[must_use]
    pub fn derive(vip: &ServiceVip, port: NonZeroU16, proto: Proto, purpose: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(vip.to_string().as_bytes());
        hasher.update([0u8]);
        hasher.update(port.get().to_be_bytes());
        hasher.update([0u8]);
        hasher.update([proto.as_u8()]);
        hasher.update([0u8]);
        hasher.update(purpose.as_bytes());
        let digest: [u8; 32] = hasher.finalize().into();
        let mut head = [0u8; 8];
        head.copy_from_slice(&digest[..8]);
        Self(u64::from_be_bytes(head))
    }
}

impl Display for ServiceId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl FromStr for ServiceId {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(IdParseError::Empty { kind: "ServiceId" });
        }
        s.parse::<u64>().map(Self).map_err(|_| IdParseError::InvalidFormat {
            kind: "ServiceId",
            expected: "decimal u64 (0..=18446744073709551615)",
        })
    }
}

/// `BACKEND_MAP` key — a stable monotonic backend identifier
/// shared across services per architecture.md § 6.
///
/// `u32` per architecture.md § 6 / § 10. Display emits the decimal
/// `u32`; `FromStr` parses decimal `u32`. There is no case axis
/// for a numeric identifier — the case-insensitivity rule from
/// `development.md` § Newtype completeness applies only to
/// human-typed string identifiers (matches the `ServiceId` /
/// `MaglevTableSize` precedent).
///
/// # Wire form
///
/// `Serialize` / `Deserialize` use the transparent `u32`
/// representation: JSON form is the bare integer, matching the
/// `ServiceId` precedent for content-derived numeric IDs.
#[derive(
    Clone,
    Copy,
    Debug,
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
#[serde(transparent)]
pub struct BackendId(u32);

impl BackendId {
    /// Validating constructor over the raw `u32`. Every `u32` is a
    /// valid `BackendId` — the newtype's role is type-system
    /// distinctness, not runtime range-check. The `Result` return
    /// is the project's newtype-completeness shape — see
    /// [`ServiceVip::new`] for the same rationale.
    #[allow(clippy::unnecessary_wraps, clippy::missing_const_for_fn)]
    pub fn new(value: u32) -> Result<Self, IdParseError> {
        Ok(Self(value))
    }

    /// Inner `u32`.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl Display for BackendId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl FromStr for BackendId {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(IdParseError::Empty { kind: "BackendId" });
        }
        s.parse::<u32>().map(Self).map_err(|_| IdParseError::InvalidFormat {
            kind: "BackendId",
            expected: "decimal u32 (0..=4294967295)",
        })
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn label_rejects_empty() {
        assert!(matches!(WorkloadId::new(""), Err(IdParseError::Empty { .. })));
    }

    #[test]
    fn label_is_case_insensitive_on_parse_and_lowercases_canonical() {
        let parsed = WorkloadId::new("Payments").unwrap();
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
    fn issuance_ordinal_rejects_empty() {
        assert!(matches!(
            IssuanceOrdinal::from_str(""),
            Err(IdParseError::Empty { kind: "IssuanceOrdinal" })
        ));
    }

    #[test]
    fn issuance_ordinal_rejects_non_numeric() {
        assert!(matches!(
            IssuanceOrdinal::from_str("12x"),
            Err(IdParseError::InvalidFormat { kind: "IssuanceOrdinal", .. })
        ));
    }

    #[test]
    fn issuance_ordinal_as_u64_echoes_constructed_value() {
        assert_eq!(IssuanceOrdinal::new(7).as_u64(), 7);
    }

    #[test]
    fn serde_round_trips_job_id() {
        let id = WorkloadId::new("payments").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"payments\"");
        let back: WorkloadId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    // -------------------------------------------------------------------------
    // `as_str` getters return the exact canonical bytes the constructor stored.
    //
    // These pin the *real* returned string against each type's stored
    // canonical form (SpiffeId lowercases; CertSerial / CorrelationKey echo
    // a verbatim valid input). Property framing over the constructor's input
    // space — rather than a single fixture — is the natural shape: the
    // invariant is "`as_str()` == what `new` stored" for every valid input,
    // and the generated values are never `""` nor any fixed sentinel.
    // -------------------------------------------------------------------------

    #[test]
    fn spiffe_as_str_is_lowercased_canonical_for_mixed_case_input() {
        // Mixed-case input → `new` lowercases the whole string; `as_str`
        // must echo that lowercased canonical verbatim, scheme included.
        let id = SpiffeId::new("SPIFFE://Overdrive.Local/Job/Payments").unwrap();
        assert_eq!(id.as_str(), "spiffe://overdrive.local/job/payments");
    }

    proptest! {
        /// `SpiffeId::as_str()` returns the lowercased canonical form of any
        /// valid input — i.e. exactly `raw.to_ascii_lowercase()`. The body
        /// returning `""` or any constant cannot satisfy this across the
        /// generated input space.
        #[test]
        fn spiffe_as_str_equals_lowercased_input(
            trust in "[a-zA-Z][a-zA-Z0-9.-]{0,30}\\.[a-zA-Z]{2,6}",
            path in "[a-zA-Z0-9][a-zA-Z0-9/._-]{0,40}",
        ) {
            let raw = format!("spiffe://{trust}/{path}");
            let id = SpiffeId::new(&raw).unwrap();
            prop_assert_eq!(id.as_str(), raw.to_ascii_lowercase());
        }

        /// `CertSerial::as_str()` echoes the (already-canonical, lowercase,
        /// even-length hex) input verbatim. Generated from arbitrary bytes
        /// rendered as lowercase hex, so the asserted value varies per case
        /// and is never a fixed string.
        #[test]
        fn cert_serial_as_str_echoes_canonical_input(
            bytes in proptest::collection::vec(any::<u8>(), 1..=CERT_SERIAL_MAX_BYTES),
        ) {
            let canonical = hex::encode(&bytes); // lowercase, even length
            let serial = CertSerial::new(&canonical).unwrap();
            prop_assert_eq!(serial.as_str(), canonical);
        }

        /// `CorrelationKey::new(raw).as_str()` echoes a valid non-empty
        /// bounded input verbatim.
        #[test]
        fn correlation_key_new_as_str_echoes_input(
            raw in "[a-zA-Z0-9:/_.-]{1,64}",
        ) {
            let key = CorrelationKey::new(&raw).unwrap();
            prop_assert_eq!(key.as_str(), raw);
        }

        /// `IssuanceOrdinal` round-trips bit-equivalently across all three
        /// codecs for every `u64`, and all three agree on the SAME canonical
        /// base-10 string form:
        ///
        /// * `Display` / `FromStr` — `parse(display(o)) == o`.
        /// * serde — `from_str(to_string(o)) == o`, and the serialized JSON
        ///   is exactly the quoted decimal (`"<n>"`), proving the pinned
        ///   `#[serde(try_from = "String", into = "String")]` string codec
        ///   (NOT a transparent raw integer).
        /// * `as_u64` — the stored value survives every hop.
        ///
        /// The generated `value` varies per case and is never a fixed
        /// sentinel, so a `Display`/serde body returning a constant cannot
        /// satisfy the property.
        #[test]
        fn issuance_ordinal_round_trips_through_display_fromstr_and_serde(
            value in any::<u64>(),
        ) {
            let ordinal = IssuanceOrdinal::new(value);

            // Display / FromStr round-trip.
            let displayed = ordinal.to_string();
            prop_assert_eq!(&displayed, &value.to_string());
            prop_assert_eq!(IssuanceOrdinal::from_str(&displayed).unwrap(), ordinal);

            // serde round-trip — and the wire form is the QUOTED decimal
            // string (the pinned base-10 string codec), not a bare integer.
            let json = serde_json::to_string(&ordinal).unwrap();
            prop_assert_eq!(&json, &format!("\"{value}\""));
            let back: IssuanceOrdinal = serde_json::from_str(&json).unwrap();
            prop_assert_eq!(back, ordinal);
            prop_assert_eq!(back.as_u64(), value);
        }
    }

    #[test]
    fn correlation_key_derive_as_str_is_non_empty_and_well_formed() {
        // Bonus: the derived form is also surfaced through `as_str` — it
        // carries the `target:purpose/<hex>` shape and is never empty.
        let h = ContentHash::of(b"spec");
        let key = CorrelationKey::derive("payments", &h, "register");
        let s = key.as_str();
        assert!(s.starts_with("payments:register/"));
        assert!(s.len() > "payments:register/".len());
    }
}
