//! Public-ingress Route, certified-key, generation and status contracts.
//!
//! SCAFFOLD: true. The data shapes, derives, trivial accessors and envelope
//! wiring below are the DESIGN-pinned contract; every validating constructor,
//! parser, digest derivation, matcher and store codec body is DELIVER's and
//! fails with SCAFFOLD text. The source-local properties at the foot of the
//! file specify S-PIG-03 through S-PIG-10, S-PIG-44 and S-PIG-55.

#![allow(
    clippy::todo,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_self,
    clippy::struct_field_names,
    reason = "public-ingress DISTILL scaffold: bodies are DELIVER's; Route field names are pinned verbatim by the accepted DESIGN contract"
)]

use std::fmt::{self, Display, Formatter};
use std::net::SocketAddrV4;
use std::num::{NonZeroU16, NonZeroU64};
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ca::issue_failure::SvidIssueFailure;
use crate::codec::{EnvelopeError, VersionedEnvelope};
use crate::dataplane::{Proto, ServiceFrontend};
use crate::gateway_identity::GatewayIdentityEpoch;
use crate::id::{
    CertSerial, CorrelationKey, IdParseError, NodeId, ServiceVip, SpiffeId, WorkloadId,
};
pub use crate::id::{PublicCertifiedKeyId, RouteId};
use crate::traits::intent_store::IntentStoreError;
use crate::traits::observation_store::{LogicalTimestamp, ObservationStoreError};
use crate::wall_clock::UnixInstant;

// ---------------------------------------------------------------------------
// Public hostname and path
// ---------------------------------------------------------------------------

/// Canonical public DNS hostname.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[serde(try_from = "String", into = "String")]
pub struct PublicHostname(String);

impl PublicHostname {
    /// Validate and ASCII-case-fold a public hostname.
    pub fn new(_raw: &str) -> Result<Self, PublicHostnameError> {
        todo!("SCAFFOLD: PublicHostname::new")
    }

    /// Borrow the canonical lowercase hostname.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for PublicHostname {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for PublicHostname {
    type Err = PublicHostnameError;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::new(raw)
    }
}

impl TryFrom<String> for PublicHostname {
    type Error = PublicHostnameError;
    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::new(&raw)
    }
}

impl From<PublicHostname> for String {
    fn from(value: PublicHostname) -> Self {
        value.0
    }
}

/// Closed public-hostname validation failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PublicHostnameError {
    #[error("public hostname is empty")]
    Empty,
    #[error("public hostname is too long")]
    TooLong,
    #[error("public hostname is not ASCII")]
    NonAscii,
    #[error("wildcard public hostnames are not supported")]
    Wildcard,
    #[error("IP literals are not public hostnames")]
    IpLiteral,
    #[error("public hostname must not have a terminal dot")]
    TerminalDot,
    #[error("public hostname contains an empty label")]
    EmptyLabel,
    #[error("public hostname label is too long")]
    LabelTooLong,
    #[error("public hostname label is invalid")]
    InvalidLabel,
}

/// Valid raw absolute request path, excluding query and fragment.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[serde(try_from = "String", into = "String")]
pub struct PublicPath(String);

impl PublicPath {
    /// Validate a raw absolute request path.
    pub fn new(_raw: &str) -> Result<Self, PublicPathError> {
        todo!("SCAFFOLD: PublicPath::new")
    }

    /// Borrow the original raw path.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for PublicPath {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for PublicPath {
    type Err = PublicPathError;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::new(raw)
    }
}

impl TryFrom<String> for PublicPath {
    type Error = PublicPathError;
    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::new(&raw)
    }
}

impl From<PublicPath> for String {
    fn from(value: PublicPath) -> Self {
        value.0
    }
}

/// Closed public-path validation failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PublicPathError {
    #[error("public path is empty")]
    Empty,
    #[error("public path is not absolute")]
    NotAbsolute,
    #[error("public path contains a control byte")]
    ContainsControl,
    #[error("public path contains a space")]
    ContainsSpace,
    #[error("public path contains a query")]
    ContainsQuery,
    #[error("public path contains a fragment")]
    ContainsFragment,
    #[error("public path contains malformed percent encoding")]
    MalformedPercentEncoding,
}

/// Exact or segment-aware prefix matching over a raw request path.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum PathMatch {
    Exact(PublicPath),
    SegmentPrefix(PublicPath),
}

impl PathMatch {
    /// Match the raw URI path without decoding or normalization.
    #[must_use]
    pub fn matches_raw_path(&self, _raw_request_path: &str) -> bool {
        todo!("SCAFFOLD: PathMatch::matches_raw_path")
    }
}

// ---------------------------------------------------------------------------
// Route
// ---------------------------------------------------------------------------

/// Stable reference to one TCP listener declared by a Service workload.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ServiceListenerReference {
    service: WorkloadId,
    port: NonZeroU16,
    protocol: Proto,
}

impl ServiceListenerReference {
    /// Construct a TCP-only Service-listener reference.
    pub fn new(
        _service: WorkloadId,
        _port: NonZeroU16,
        _protocol: Proto,
    ) -> Result<Self, RouteValidationError> {
        todo!("SCAFFOLD: ServiceListenerReference::new")
    }

    /// Referenced Service's logical workload identity.
    pub fn service(&self) -> &WorkloadId {
        &self.service
    }

    /// Referenced listener port.
    pub const fn port(&self) -> NonZeroU16 {
        self.port
    }

    /// Referenced listener protocol.
    pub const fn protocol(&self) -> Proto {
        self.protocol
    }
}

/// Public Route HTTP wire input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PublicRouteInput {
    pub id: String,
    pub hostname: String,
    pub path: String,
    pub path_match: String,
    pub certified_key: String,
    pub target: ServiceListenerReferenceInput,
}

/// Public Route TOML-parser input.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PublicRouteSpecInput {
    pub id: String,
    pub hostname: String,
    pub path: String,
    pub path_match: String,
    pub certified_key: String,
    pub target: ServiceListenerReferenceInput,
}

/// Wire/parser input for an exact Service listener reference. The member is
/// exactly `service`; there is no `workload` alias.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceListenerReferenceInput {
    pub service: String,
    pub port: u16,
    pub protocol: String,
}

/// One validated public Route.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Route {
    route_id: RouteId,
    route_generation: RouteGeneration,
    public_hostname: PublicHostname,
    path_match: PathMatch,
    service_listener: ServiceListenerReference,
    public_certified_key_id: PublicCertifiedKeyId,
}

impl Route {
    /// Validate a Route submitted at the operator HTTP boundary.
    pub fn from_submit(_input: PublicRouteInput) -> Result<Self, RouteValidationError> {
        todo!("SCAFFOLD: Route::from_submit")
    }

    pub fn route_id(&self) -> &RouteId {
        &self.route_id
    }

    pub const fn route_generation(&self) -> RouteGeneration {
        self.route_generation
    }

    pub fn public_hostname(&self) -> &PublicHostname {
        &self.public_hostname
    }

    pub fn path_match(&self) -> &PathMatch {
        &self.path_match
    }

    pub fn service_listener(&self) -> &ServiceListenerReference {
        &self.service_listener
    }

    pub fn public_certified_key_id(&self) -> &PublicCertifiedKeyId {
        &self.public_certified_key_id
    }
}

/// Closed Route-validation failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RouteValidationError {
    #[error("invalid Route ID: {0}")]
    RouteId(IdParseError),
    #[error("invalid certified-key ID: {0}")]
    CertifiedKeyId(IdParseError),
    #[error("invalid workload ID: {0}")]
    Service(IdParseError),
    #[error("invalid public hostname: {0}")]
    Hostname(PublicHostnameError),
    #[error("invalid public path: {0}")]
    Path(PublicPathError),
    #[error("unknown path match")]
    UnknownPathMatch,
    #[error("listener port is zero")]
    PortZero,
    #[error("unknown listener protocol")]
    UnknownProtocol,
    #[error("public ingress requires TCP")]
    ProtocolNotTcp,
}

/// Route application outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum RouteApplyOutcome {
    Declared,
    Replaced,
    Unchanged,
}

/// Route withdrawal outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum RouteWithdrawOutcome {
    Withdrawn,
    Unchanged,
}

// ---------------------------------------------------------------------------
// Digest and decimal newtypes
// ---------------------------------------------------------------------------

/// Closed lowercase SHA-256 hex parse failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DigestHexParseError {
    #[error("digest is empty")]
    Empty,
    #[error("digest length is {got}, expected 64")]
    InvalidLength { got: usize },
    #[error("digest contains a non-hex character")]
    NonHex,
    #[error("uppercase digest is noncanonical")]
    Uppercase,
}

/// Shared surface of the four SHA-256 wrappers: lowercase-hex `Display`,
/// case-sensitive `FromStr`/`TryFrom` through `new`, matching string serde.
macro_rules! digest_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            rkyv::Archive,
            rkyv::Serialize,
            rkyv::Deserialize,
        )]
        pub struct $name([u8; 32]);

        impl $name {
            /// The raw digest bytes.
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.write_str(&hex::encode(self.0))
            }
        }

        impl FromStr for $name {
            type Err = DigestHexParseError;
            fn from_str(raw: &str) -> Result<Self, Self::Err> {
                Self::new(raw)
            }
        }

        impl<'a> TryFrom<&'a str> for $name {
            type Error = DigestHexParseError;
            fn try_from(raw: &'a str) -> Result<Self, Self::Error> {
                Self::new(raw)
            }
        }

        impl TryFrom<String> for $name {
            type Error = DigestHexParseError;
            fn try_from(raw: String) -> Result<Self, Self::Error> {
                Self::new(&raw)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let raw = String::deserialize(deserializer)?;
                raw.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

digest_newtype!(
    /// Deterministic digest of every Route field.
    RouteGeneration
);
digest_newtype!(
    /// Deterministic digest of a complete ordered certificate chain.
    CertifiedKeyGeneration
);
digest_newtype!(
    /// SHA-256 fingerprint of the public leaf certificate.
    CertificateFingerprint
);
digest_newtype!(
    /// Deterministic identity of a coherent Gateway Application generation.
    GatewayApplicationGenerationId
);

impl RouteGeneration {
    /// Parse exactly 64 lowercase hexadecimal characters.
    pub fn new(_raw: &str) -> Result<Self, DigestHexParseError> {
        todo!("SCAFFOLD: RouteGeneration::new")
    }

    /// Hash every Route field in the DESIGN-pinned order.
    pub fn from_route_fields(
        _route_id: &RouteId,
        _hostname: &PublicHostname,
        _path_match: &PathMatch,
        _listener: &ServiceListenerReference,
        _certified_key_id: &PublicCertifiedKeyId,
    ) -> Self {
        todo!("SCAFFOLD: RouteGeneration::from_route_fields")
    }
}

impl CertifiedKeyGeneration {
    /// Parse exactly 64 lowercase hexadecimal characters.
    pub fn new(_raw: &str) -> Result<Self, DigestHexParseError> {
        todo!("SCAFFOLD: CertifiedKeyGeneration::new")
    }

    /// Hash the ordered certificate chain DER.
    pub fn from_ordered_chain_der(_chain: &[Vec<u8>]) -> Self {
        todo!("SCAFFOLD: CertifiedKeyGeneration::from_ordered_chain_der")
    }
}

impl CertificateFingerprint {
    /// Parse exactly 64 lowercase hexadecimal characters.
    pub fn new(_raw: &str) -> Result<Self, DigestHexParseError> {
        todo!("SCAFFOLD: CertificateFingerprint::new")
    }

    /// Hash the leaf certificate DER.
    pub fn from_leaf_der(_leaf: &[u8]) -> Self {
        todo!("SCAFFOLD: CertificateFingerprint::from_leaf_der")
    }
}

impl GatewayApplicationGenerationId {
    /// Parse exactly 64 lowercase hexadecimal characters.
    pub fn new(_raw: &str) -> Result<Self, DigestHexParseError> {
        todo!("SCAFFOLD: GatewayApplicationGenerationId::new")
    }

    /// Hash the three resolved inputs of a Gateway Application generation.
    pub fn from_inputs(
        _route_generation: RouteGeneration,
        _frontend: ServiceFrontend,
        _certified_key_generation: CertifiedKeyGeneration,
    ) -> Self {
        todo!("SCAFFOLD: GatewayApplicationGenerationId::from_inputs")
    }
}

/// Nonzero Linux socket cookie read from an upstream socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SocketCookie(NonZeroU64);

impl SocketCookie {
    /// Validate a raw cookie.
    pub fn new(_raw: u64) -> Result<Self, SocketCookieParseError> {
        todo!("SCAFFOLD: SocketCookie::new")
    }

    /// The nonzero value.
    pub const fn get(self) -> NonZeroU64 {
        self.0
    }
}

/// Closed socket-cookie parse failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SocketCookieParseError {
    #[error("socket cookie is empty")]
    Empty,
    #[error("socket cookie is not canonical decimal")]
    InvalidDecimal,
    #[error("socket cookie must be nonzero")]
    Zero,
}

impl Display for SocketCookie {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for SocketCookie {
    type Err = SocketCookieParseError;
    fn from_str(_raw: &str) -> Result<Self, Self::Err> {
        todo!("SCAFFOLD: SocketCookie FromStr")
    }
}

impl<'a> TryFrom<&'a str> for SocketCookie {
    type Error = SocketCookieParseError;
    fn try_from(raw: &'a str) -> Result<Self, Self::Error> {
        raw.parse()
    }
}

impl TryFrom<String> for SocketCookie {
    type Error = SocketCookieParseError;
    fn try_from(raw: String) -> Result<Self, Self::Error> {
        raw.parse()
    }
}

impl Serialize for SocketCookie {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SocketCookie {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// Exact Service-map lookup key supplied for one gateway connect.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct ServiceKey {
    vip: ServiceVip,
    port: NonZeroU16,
    protocol: Proto,
}

impl ServiceKey {
    /// Project a resolved Service frontend to its lookup key.
    pub fn from_frontend(_frontend: ServiceFrontend) -> Self {
        todo!("SCAFFOLD: ServiceKey::from_frontend")
    }

    pub const fn vip(&self) -> ServiceVip {
        self.vip
    }

    pub const fn port(&self) -> NonZeroU16 {
        self.port
    }

    pub const fn protocol(&self) -> Proto {
        self.protocol
    }
}

// ---------------------------------------------------------------------------
// Route set (singleton) persistence
// ---------------------------------------------------------------------------

/// Typed persisted Route-set decode failures.
#[derive(Debug, Error)]
pub enum RouteCodecError {
    #[error("Route envelope is invalid: {0}")]
    Envelope(EnvelopeError),
    #[error("persisted Route is invalid: {0}")]
    InvalidRoute(RouteValidationError),
}

/// Singleton persisted Route-set payload.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct PublicRouteSetV1 {
    pub route: Option<Route>,
}

/// Versioned singleton Route-set record stored at `public-ingress/route-set`.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum PublicRouteSetEnvelope {
    V1(PublicRouteSetV1),
}

impl VersionedEnvelope for PublicRouteSetEnvelope {
    type Latest = PublicRouteSetV1;

    fn latest(payload: Self::Latest) -> Self {
        Self::V1(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1),
        }
    }
}

impl PublicRouteSetV1 {
    pub fn archive_for_store(&self) -> Result<rkyv::util::AlignedVec, EnvelopeError> {
        todo!("SCAFFOLD: PublicRouteSetV1::archive_for_store")
    }

    pub fn from_store_bytes(
        _bytes: &[u8],
        _redb_path: &Path,
        _key: Option<&str>,
    ) -> Result<Self, IntentStoreError> {
        todo!("SCAFFOLD: PublicRouteSetV1::from_store_bytes")
    }
}

// ---------------------------------------------------------------------------
// Public certified-key custody records
// ---------------------------------------------------------------------------

/// Producer provenance retained for a protected public certified key.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum CertifiedKeyProvenance {
    Manual,
    Workflow { correlation: CorrelationKey },
}

/// Current time-derived usability failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum CertifiedKeyUsabilityFailure {
    NotYetValid,
    Expired,
}

/// Closed redacted certified-key failure category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum CertifiedKeyFailure {
    SourceOpen,
    SourceMetadata,
    SourceOwnership,
    SourcePermissions,
    SourceBounds,
    PemShape,
    CertificateProfile,
    HostnameMismatch,
    KeyMismatch,
    Protection,
    Persistence,
    RustlsConfiguration,
}

/// Closed public-certified-key AEAD adapter failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PublicCertifiedKeyAeadError {
    #[error("public certified-key KEK unavailable")]
    KekUnavailable,
    #[error("public certified-key seal failed")]
    SealFailed,
    #[error("public certified-key open failed")]
    OpenFailed,
    #[error("public certified-key authentication failed")]
    AuthenticationFailed,
}

/// Domain-bound AAD inputs for one protected public private key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicCertifiedKeyProtectionContext {
    pub certified_key_id: PublicCertifiedKeyId,
    pub public_hostname: PublicHostname,
    pub generation: CertifiedKeyGeneration,
}

/// Protected origin private-key record. `Debug` is deliberately redacted.
#[derive(Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ProtectedOriginKeyV1 {
    pub kek_id: String,
    pub salt: [u8; 32],
    pub nonce: [u8; 12],
    pub ciphertext_and_tag: Vec<u8>,
}

impl fmt::Debug for ProtectedOriginKeyV1 {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProtectedOriginKeyV1")
            .field("kek_id", &self.kek_id)
            .field("salt", &"[REDACTED]")
            .field("nonce", &"[REDACTED]")
            .field("ciphertext_and_tag", &"[REDACTED]")
            .finish()
    }
}

/// Complete durable public certified-key generation.
#[derive(Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct PublicCertifiedKeyV1 {
    pub certified_key_id: PublicCertifiedKeyId,
    pub public_hostname: PublicHostname,
    pub certified_key_generation: CertifiedKeyGeneration,
    pub certificate_fingerprint: CertificateFingerprint,
    pub certificate_chain_der: Vec<Vec<u8>>,
    pub not_before: UnixInstant,
    pub not_after: UnixInstant,
    pub provenance: CertifiedKeyProvenance,
    pub protected_origin_key: ProtectedOriginKeyV1,
}

impl fmt::Debug for PublicCertifiedKeyV1 {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublicCertifiedKeyV1")
            .field("certified_key_id", &self.certified_key_id)
            .field("public_hostname", &self.public_hostname)
            .field("certified_key_generation", &self.certified_key_generation)
            .field("certificate_fingerprint", &self.certificate_fingerprint)
            .field("certificate_chain_der", &"[REDACTED]")
            .field("not_before", &self.not_before)
            .field("not_after", &self.not_after)
            .field("provenance", &"[REDACTED]")
            .field("protected_origin_key", &"[REDACTED]")
            .finish()
    }
}

/// Versioned protected certified-key record stored at
/// `public-ingress/certified-key/<PublicCertifiedKeyId>`.
#[derive(Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum PublicCertifiedKeyEnvelope {
    V1(PublicCertifiedKeyV1),
}

impl VersionedEnvelope for PublicCertifiedKeyEnvelope {
    type Latest = PublicCertifiedKeyV1;

    fn latest(payload: Self::Latest) -> Self {
        Self::V1(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1),
        }
    }
}

impl PublicCertifiedKeyV1 {
    pub fn archive_for_store(&self) -> Result<rkyv::util::AlignedVec, EnvelopeError> {
        todo!("SCAFFOLD: PublicCertifiedKeyV1::archive_for_store")
    }

    pub fn from_store_bytes(
        _bytes: &[u8],
        _redb_path: &Path,
        _key: Option<&str>,
    ) -> Result<Self, IntentStoreError> {
        todo!("SCAFFOLD: PublicCertifiedKeyV1::from_store_bytes")
    }
}

// ---------------------------------------------------------------------------
// Observation status rows
// ---------------------------------------------------------------------------

/// Redacted custody status state.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum PublicCertifiedKeyStatusState {
    Absent,
    Usable {
        generation: CertifiedKeyGeneration,
        certificate_fingerprint: CertificateFingerprint,
        public_hostname: PublicHostname,
        not_before: UnixInstant,
        not_after: UnixInstant,
        provenance: CertifiedKeyProvenance,
    },
    Unusable {
        generation: CertifiedKeyGeneration,
        cause: CertifiedKeyUsabilityFailure,
    },
}

/// Redacted public-certified-key LWW status row.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct PublicCertifiedKeyStatusRowV1 {
    pub certified_key_id: PublicCertifiedKeyId,
    pub state: PublicCertifiedKeyStatusState,
    pub last_install_failure: Option<CertifiedKeyFailure>,
    pub updated_at: LogicalTimestamp,
}

/// Versioned custody status row, table `observation_public_certified_key_status`.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum PublicCertifiedKeyStatusRowEnvelope {
    V1(PublicCertifiedKeyStatusRowV1),
}

impl VersionedEnvelope for PublicCertifiedKeyStatusRowEnvelope {
    type Latest = PublicCertifiedKeyStatusRowV1;

    fn latest(payload: Self::Latest) -> Self {
        Self::V1(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1),
        }
    }
}

impl PublicCertifiedKeyStatusRowV1 {
    pub fn archive_for_store(&self) -> Result<rkyv::util::AlignedVec, EnvelopeError> {
        todo!("SCAFFOLD: PublicCertifiedKeyStatusRowV1::archive_for_store")
    }

    pub fn from_store_bytes(
        _bytes: &[u8],
        _key: Option<&str>,
    ) -> Result<Self, ObservationStoreError> {
        todo!("SCAFFOLD: PublicCertifiedKeyStatusRowV1::from_store_bytes")
    }
}

/// Public listener fact owned by Gateway Application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum GatewayListenerStatus {
    Unbound,
    Bound { address: SocketAddrV4 },
}

/// One phase-qualified application-generation projection.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct GatewayApplicationGenerationStatus {
    pub application_generation: GatewayApplicationGenerationId,
    pub route_id: RouteId,
    pub route_generation: RouteGeneration,
    pub service_key: ServiceKey,
}

/// Closed reason no complete Gateway Application is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum GatewayApplicationUnavailableCause {
    RouteAbsent,
    RouteUnreadable,
    CertifiedKeyAbsent,
    CertifiedKeyUnusable,
    FrontendUnresolved,
    GatewayIdentityUnusable,
    ConnectPathUnavailable,
    ListenerBindFailed,
    WatchGap,
}

/// Non-secret gateway identity projection.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum GatewayIdentityStatus {
    Absent,
    Current {
        epoch: GatewayIdentityEpoch,
        spiffe_id: SpiffeId,
        serial: CertSerial,
        not_after: UnixInstant,
    },
    Unusable,
}

/// Whether the registered connect/receipt path is trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum GatewayConnectPathStatus {
    Unavailable,
    Ready,
}

/// Redacted Gateway Application LWW status row.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct GatewayApplicationStatusRowV1 {
    pub node_id: NodeId,
    pub listener: GatewayListenerStatus,
    pub staged: Option<GatewayApplicationGenerationStatus>,
    pub current: Option<GatewayApplicationGenerationStatus>,
    pub draining: Vec<GatewayApplicationGenerationStatus>,
    pub unavailable: Option<GatewayApplicationUnavailableCause>,
    pub gateway_identity: GatewayIdentityStatus,
    /// Monotone count of failed gateway SVID issuances (base plus process
    /// record); `0` iff `last_issue_failure` is `None`.
    pub issue_failures: u64,
    /// Latest failed issuance, depth 1.
    pub last_issue_failure: Option<SvidIssueFailure>,
    pub connect_path: GatewayConnectPathStatus,
    pub updated_at: LogicalTimestamp,
}

/// Versioned Gateway Application status row, table `observation_gateway_application_status`.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum GatewayApplicationStatusRowEnvelope {
    V1(GatewayApplicationStatusRowV1),
}

impl VersionedEnvelope for GatewayApplicationStatusRowEnvelope {
    type Latest = GatewayApplicationStatusRowV1;

    fn latest(payload: Self::Latest) -> Self {
        Self::V1(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1),
        }
    }
}

impl GatewayApplicationStatusRowV1 {
    pub fn archive_for_store(&self) -> Result<rkyv::util::AlignedVec, EnvelopeError> {
        todo!("SCAFFOLD: GatewayApplicationStatusRowV1::archive_for_store")
    }

    pub fn from_store_bytes(
        _bytes: &[u8],
        _key: Option<&str>,
    ) -> Result<Self, ObservationStoreError> {
        todo!("SCAFFOLD: GatewayApplicationStatusRowV1::from_store_bytes")
    }
}

// ---------------------------------------------------------------------------
// Source-local acceptance properties (S-PIG-03 .. S-PIG-10, S-PIG-44, S-PIG-55)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod properties {
    #![allow(
        clippy::expect_used,
        clippy::doc_markdown,
        reason = "fixture preconditions panic with their cause; CONTRACT_SHAPE is required metadata"
    )]

    use std::num::NonZeroU16;

    use proptest::prelude::*;

    use super::*;

    type RouteErrorCase = (PublicRouteInput, RouteValidationError, &'static str);

    fn arb_valid_path() -> impl Strategy<Value = String> {
        let atom = prop_oneof![
            "[a-z0-9._~-]".prop_map(|value| value),
            (any::<u8>()).prop_map(|byte| format!("%{byte:02X}")),
        ];
        prop::collection::vec(atom, 0..24).prop_map(|atoms| format!("/{}", atoms.concat()))
    }

    fn valid_label() -> impl Strategy<Value = String> {
        "[a-z](?:[a-z0-9-]{0,10}[a-z0-9])?"
    }

    fn canonical_input() -> PublicRouteInput {
        PublicRouteInput {
            id: "public-api".to_owned(),
            hostname: "api.example.com".to_owned(),
            path: "/".to_owned(),
            path_match: "segment_prefix".to_owned(),
            certified_key: "api-origin".to_owned(),
            target: ServiceListenerReferenceInput {
                service: "api".to_owned(),
                port: 8080,
                protocol: "tcp".to_owned(),
            },
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// S-PIG-03: an exact path matches only the byte-equal request path.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER route-value-types"]
        fn exact_path_matches_only_byte_equal_paths(path in arb_valid_path(), suffix in "[a-z]{1,8}") {
            let approved = PublicPath::new(&path).expect("generated path is valid");
            let matcher = PathMatch::Exact(approved);
            let extended = format!("{path}{suffix}");
            prop_assert!(matcher.matches_raw_path(&path));
            prop_assert!(!matcher.matches_raw_path(&extended));
        }

        /// S-PIG-04: a segment prefix matches the path itself and its child
        /// segments, never an adjacent sibling that merely shares the bytes.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER route-value-types"]
        fn segment_prefix_never_crosses_a_segment_boundary(segment in "[a-z]{1,16}", tail in "[a-z]{1,16}") {
            let raw = format!("/{segment}");
            let matcher = PathMatch::SegmentPrefix(
                PublicPath::new(&raw).expect("generated path is valid"),
            );
            let child = format!("{raw}/{tail}");
            let adjacent = format!("{raw}{tail}");
            prop_assert!(matcher.matches_raw_path(&raw));
            prop_assert!(matcher.matches_raw_path(&child));
            prop_assert!(!matcher.matches_raw_path(&adjacent));
        }

        /// S-PIG-08: the Route generation is deterministic and changes with
        /// every one of the six Route fields it covers.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER route-value-types"]
        fn route_generation_is_deterministic_and_field_sensitive(
            route_id in valid_label(),
            service in valid_label(),
            port in 1u16..u16::MAX,
        ) {
            let route_id = RouteId::new(&route_id).expect("valid Route ID");
            let host = PublicHostname::new("api.example.com").expect("valid hostname");
            let path = PathMatch::SegmentPrefix(PublicPath::new("/api").expect("valid path"));
            let listener = ServiceListenerReference::new(
                WorkloadId::new(&service).expect("valid Service ID"),
                NonZeroU16::new(port).expect("nonzero"),
                Proto::Tcp,
            ).expect("TCP listener");
            let key = PublicCertifiedKeyId::new("api-origin").expect("valid key ID");
            let first = RouteGeneration::from_route_fields(&route_id, &host, &path, &listener, &key);
            let second = RouteGeneration::from_route_fields(&route_id, &host, &path, &listener, &key);
            prop_assert_eq!(first, second);

            let other_route_id = RouteId::new(&format!("{route_id}-other")).expect("distinct Route ID");
            let other_host = PublicHostname::new("other.example.com").expect("valid hostname");
            let exact_path = PathMatch::Exact(PublicPath::new("/api").expect("valid path"));
            let other_path = PathMatch::SegmentPrefix(PublicPath::new("/v2").expect("valid path"));
            let other_service = ServiceListenerReference::new(
                WorkloadId::new(&format!("{service}-other")).expect("distinct Service ID"),
                NonZeroU16::new(port).expect("nonzero"),
                Proto::Tcp,
            ).expect("TCP listener");
            let other_port = ServiceListenerReference::new(
                WorkloadId::new(&service).expect("valid Service ID"),
                NonZeroU16::new(port + 1).expect("nonzero"),
                Proto::Tcp,
            ).expect("TCP listener");
            let other_key = PublicCertifiedKeyId::new("other-origin").expect("valid key ID");
            let mutations = [
                RouteGeneration::from_route_fields(&other_route_id, &host, &path, &listener, &key),
                RouteGeneration::from_route_fields(&route_id, &other_host, &path, &listener, &key),
                RouteGeneration::from_route_fields(&route_id, &host, &exact_path, &listener, &key),
                RouteGeneration::from_route_fields(&route_id, &host, &other_path, &listener, &key),
                RouteGeneration::from_route_fields(&route_id, &host, &path, &other_service, &key),
                RouteGeneration::from_route_fields(&route_id, &host, &path, &other_port, &key),
                RouteGeneration::from_route_fields(&route_id, &host, &path, &listener, &other_key),
            ];
            for changed in mutations {
                prop_assert_ne!(first, changed);
            }
        }

        /// S-PIG-09: the certified-key generation covers the ordered chain,
        /// so reordering the chain changes it.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER route-value-types"]
        fn certified_key_generation_preserves_chain_order(
            leaf in prop::collection::vec(any::<u8>(), 1..64),
            issuer in prop::collection::vec(any::<u8>(), 1..64),
        ) {
            prop_assume!(leaf != issuer);
            let forward = CertifiedKeyGeneration::from_ordered_chain_der(&[leaf.clone(), issuer.clone()]);
            let again = CertifiedKeyGeneration::from_ordered_chain_der(&[leaf.clone(), issuer.clone()]);
            let reverse = CertifiedKeyGeneration::from_ordered_chain_der(&[issuer, leaf]);
            prop_assert_eq!(forward, again);
            prop_assert_ne!(forward, reverse);
        }

        /// S-PIG-10: the application generation is a function of exactly the
        /// route generation, the resolved frontend and the certified-key
        /// generation; it takes no demand, revision or backend input.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER route-value-types"]
        fn application_generation_changes_with_each_of_its_three_inputs(port in 1u16..u16::MAX) {
            let route = RouteGeneration::new(&"01".repeat(32)).expect("valid generation");
            let other_route = RouteGeneration::new(&"03".repeat(32)).expect("valid generation");
            let key = CertifiedKeyGeneration::new(&"02".repeat(32)).expect("valid generation");
            let other_key = CertifiedKeyGeneration::new(&"04".repeat(32)).expect("valid generation");
            let vip_a = ServiceVip::new("10.96.0.1".parse().expect("IP")).expect("VIP");
            let vip_b = ServiceVip::new("10.96.0.2".parse().expect("IP")).expect("VIP");
            let a = ServiceFrontend::new(vip_a, NonZeroU16::new(port).expect("nonzero"), Proto::Tcp).expect("frontend");
            let other_port = ServiceFrontend::new(vip_a, NonZeroU16::new(port + 1).expect("nonzero"), Proto::Tcp).expect("frontend");
            let other_vip = ServiceFrontend::new(vip_b, NonZeroU16::new(port).expect("nonzero"), Proto::Tcp).expect("frontend");
            let base = GatewayApplicationGenerationId::from_inputs(route, a, key);
            prop_assert_eq!(base, GatewayApplicationGenerationId::from_inputs(route, a, key));
            prop_assert_ne!(base, GatewayApplicationGenerationId::from_inputs(route, other_port, key));
            prop_assert_ne!(base, GatewayApplicationGenerationId::from_inputs(route, other_vip, key));
            prop_assert_ne!(base, GatewayApplicationGenerationId::from_inputs(other_route, a, key));
            prop_assert_ne!(base, GatewayApplicationGenerationId::from_inputs(route, a, other_key));
        }

        /// S-PIG-44: every public-ingress numeric and digest newtype
        /// round-trips its canonical string through `Display`, `FromStr`,
        /// `TryFrom`, string serde and (digests) rkyv.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER route-value-types"]
        fn public_ingress_newtypes_roundtrip_their_canonical_strings(
            numeric in 1u64..u64::MAX,
            digest in "[0-9a-f]{64}",
        ) {
            let cookie = SocketCookie::new(numeric).expect("nonzero cookie");
            prop_assert_eq!(cookie.to_string(), numeric.to_string());
            prop_assert_eq!(numeric.to_string().parse::<SocketCookie>(), Ok(cookie));
            prop_assert_eq!(SocketCookie::try_from(numeric.to_string().as_str()), Ok(cookie));
            prop_assert_eq!(SocketCookie::try_from(numeric.to_string()), Ok(cookie));
            prop_assert_eq!(serde_json::from_str::<SocketCookie>(
                &serde_json::to_string(&cookie).expect("serialize"),
            ).expect("deserialize"), cookie);

            macro_rules! assert_digest_roundtrip {
                ($ty:ty) => {{
                    let value = <$ty>::new(&digest).expect("canonical digest");
                    prop_assert_eq!(value.to_string(), digest.as_str());
                    prop_assert_eq!(digest.parse::<$ty>(), Ok(value));
                    prop_assert_eq!(<$ty>::try_from(digest.as_str()), Ok(value));
                    prop_assert_eq!(<$ty>::try_from(digest.clone()), Ok(value));
                    prop_assert_eq!(serde_json::from_str::<$ty>(
                        &serde_json::to_string(&value).expect("serialize"),
                    ).expect("deserialize"), value);
                    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&value)
                        .expect("rkyv archive");
                    prop_assert_eq!(
                        rkyv::from_bytes::<$ty, rkyv::rancor::Error>(&archived)
                            .expect("rkyv valid trip"),
                        value,
                    );
                }};
            }
            assert_digest_roundtrip!(RouteGeneration);
            assert_digest_roundtrip!(CertifiedKeyGeneration);
            assert_digest_roundtrip!(CertificateFingerprint);
            assert_digest_roundtrip!(GatewayApplicationGenerationId);
        }
    }

    /// S-PIG-05: hostname and path validation cover every closed error
    /// partition exactly.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER route-value-types"]
    fn hostname_and_path_validation_cover_closed_error_partitions() {
        let invalid_hosts = [
            ("", PublicHostnameError::Empty),
            (&"a".repeat(254), PublicHostnameError::TooLong),
            ("tést.example", PublicHostnameError::NonAscii),
            ("*.example.com", PublicHostnameError::Wildcard),
            ("127.0.0.1", PublicHostnameError::IpLiteral),
            ("api.example.com.", PublicHostnameError::TerminalDot),
            ("api..example.com", PublicHostnameError::EmptyLabel),
            (&format!("{}.example", "a".repeat(64)), PublicHostnameError::LabelTooLong),
            ("-api.example.com", PublicHostnameError::InvalidLabel),
        ];
        for (host, expected) in invalid_hosts {
            assert_eq!(PublicHostname::new(host), Err(expected), "{host:?}");
        }
        let invalid_paths = [
            ("", PublicPathError::Empty),
            ("relative", PublicPathError::NotAbsolute),
            ("/line\nfeed", PublicPathError::ContainsControl),
            ("/has space", PublicPathError::ContainsSpace),
            ("/a?b", PublicPathError::ContainsQuery),
            ("/a#b", PublicPathError::ContainsFragment),
            ("/bad%2", PublicPathError::MalformedPercentEncoding),
            ("/bad%", PublicPathError::MalformedPercentEncoding),
            ("/bad%GG", PublicPathError::MalformedPercentEncoding),
            ("/bad%2G", PublicPathError::MalformedPercentEncoding),
        ];
        for (path, expected) in invalid_paths {
            assert_eq!(PublicPath::new(path), Err(expected), "{path:?}");
        }
    }

    /// S-PIG-06: a hostname label accepts 62 and 63 octets and refuses 64.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER route-value-types"]
    fn hostname_label_boundaries_cover_sixty_two_sixty_three_and_sixty_four_octets() {
        let label_62 = "a".repeat(62);
        let label_63 = "a".repeat(63);
        let label_64 = "a".repeat(64);
        assert!(PublicHostname::new(&format!("{label_62}.example")).is_ok());
        assert!(PublicHostname::new(&format!("{label_63}.example")).is_ok());
        assert_eq!(
            PublicHostname::new(&format!("{label_64}.example")),
            Err(PublicHostnameError::LabelTooLong),
        );
    }

    /// S-PIG-07: every Route input channel has a cause-distinct malformed
    /// example, each answered by its own closed variant.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER route-value-types"]
    fn every_route_input_channel_has_a_cause_distinct_malformed_example() {
        let mut cases: Vec<RouteErrorCase> = Vec::new();
        let mut bad = canonical_input();
        bad.id = "*".to_owned();
        cases.push((
            bad,
            RouteValidationError::RouteId(IdParseError::InvalidChar {
                kind: "RouteId",
                ch: '*',
                index: 0,
            }),
            "route_id",
        ));
        let mut bad = canonical_input();
        bad.hostname = "*.example.com".to_owned();
        cases.push((
            bad,
            RouteValidationError::Hostname(PublicHostnameError::Wildcard),
            "hostname",
        ));
        let mut bad = canonical_input();
        bad.path = "relative".to_owned();
        cases.push((bad, RouteValidationError::Path(PublicPathError::NotAbsolute), "path"));
        let mut bad = canonical_input();
        bad.path_match = "regex".to_owned();
        cases.push((bad, RouteValidationError::UnknownPathMatch, "path_match"));
        let mut bad = canonical_input();
        bad.certified_key = "*".to_owned();
        cases.push((
            bad,
            RouteValidationError::CertifiedKeyId(IdParseError::InvalidChar {
                kind: "PublicCertifiedKeyId",
                ch: '*',
                index: 0,
            }),
            "certified_key",
        ));
        let mut bad = canonical_input();
        bad.target.service = "*".to_owned();
        cases.push((
            bad,
            RouteValidationError::Service(IdParseError::InvalidChar {
                kind: "WorkloadId",
                ch: '*',
                index: 0,
            }),
            "service",
        ));
        let mut bad = canonical_input();
        bad.target.port = 0;
        cases.push((bad, RouteValidationError::PortZero, "port"));
        let mut bad = canonical_input();
        bad.target.protocol = "sctp".to_owned();
        cases.push((bad, RouteValidationError::UnknownProtocol, "protocol"));
        let mut bad = canonical_input();
        bad.target.protocol = "udp".to_owned();
        cases.push((bad, RouteValidationError::ProtocolNotTcp, "protocol_not_tcp"));

        for (input, expected, field) in cases {
            let error = Route::from_submit(input).expect_err("malformed partition must fail");
            assert_eq!(error, expected, "{field} returned wrong closed variant");
        }
    }

    /// S-PIG-44: each noncanonical numeric and digest partition is refused
    /// with its own closed variant.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER route-value-types"]
    fn public_ingress_newtypes_reject_every_noncanonical_partition_exactly() {
        assert_eq!("".parse::<SocketCookie>(), Err(SocketCookieParseError::Empty));
        for raw in ["+1", "-1", "01", " 1", "1 ", "18446744073709551616"] {
            assert_eq!(
                raw.parse::<SocketCookie>(),
                Err(SocketCookieParseError::InvalidDecimal),
                "SocketCookie partition {raw:?}",
            );
        }
        assert_eq!("0".parse::<SocketCookie>(), Err(SocketCookieParseError::Zero));

        macro_rules! assert_digest_rejections {
            ($ty:ty) => {{
                assert_eq!(<$ty>::new(""), Err(DigestHexParseError::Empty));
                assert_eq!(
                    <$ty>::new(&"a".repeat(63)),
                    Err(DigestHexParseError::InvalidLength { got: 63 }),
                );
                assert_eq!(<$ty>::new(&"A".repeat(64)), Err(DigestHexParseError::Uppercase));
                assert_eq!(<$ty>::new(&"g".repeat(64)), Err(DigestHexParseError::NonHex));
            }};
        }
        assert_digest_rejections!(RouteGeneration);
        assert_digest_rejections!(CertifiedKeyGeneration);
        assert_digest_rejections!(CertificateFingerprint);
        assert_digest_rejections!(GatewayApplicationGenerationId);
    }

    /// S-PIG-55: both Route parsers accept exactly `target.service`; the old
    /// `target.workload` member has no JSON or TOML alias.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER route-value-types"]
    fn route_target_accepts_only_service_and_rejects_the_old_workload_member() {
        let current_json = r#"{
            "id":"public-api","hostname":"api.example.com","path":"/",
            "path_match":"segment_prefix","certified_key":"api-origin",
            "target":{"service":"api","port":8080,"protocol":"tcp"}
        }"#;
        let accepted = serde_json::from_str::<PublicRouteInput>(current_json)
            .expect("target.service is the wire member");
        let route = Route::from_submit(accepted).expect("a service target is a valid Route");
        assert_eq!(route.service_listener().service().as_str(), "api");

        let legacy_json = r#"{
            "id":"public-api","hostname":"api.example.com","path":"/",
            "path_match":"segment_prefix","certified_key":"api-origin",
            "target":{"workload":"api","port":8080,"protocol":"tcp"}
        }"#;
        let json_error = serde_json::from_str::<PublicRouteInput>(legacy_json)
            .expect_err("target.workload has no JSON alias")
            .to_string();
        assert!(json_error.contains("unknown field `workload`"));
        assert!(json_error.contains("`service`"));

        let legacy_toml = r#"
            id = "public-api"
            hostname = "api.example.com"
            path = "/"
            path_match = "segment_prefix"
            certified_key = "api-origin"
            [target]
            workload = "api"
            port = 8080
            protocol = "tcp"
        "#;
        let toml_error = toml::from_str::<PublicRouteSpecInput>(legacy_toml)
            .expect_err("target.workload has no TOML alias")
            .to_string();
        assert!(toml_error.contains("unknown field `workload`"));
    }
}
