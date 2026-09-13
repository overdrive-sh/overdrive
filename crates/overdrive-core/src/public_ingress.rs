//! Public-ingress Route, demand, receipt, and generation contracts.
//!
//! SCAFFOLD: true. DISTILL owns the executable contract bodies below. The
//! production methods intentionally remain RED until their DELIVER steps.
//! Source-local properties implement S-PIG-03 through S-PIG-11.

#![expect(clippy::todo, reason = "public-ingress DISTILL RED scaffolds")]
#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]
#![allow(
    clippy::struct_field_names,
    reason = "Route field names are pinned verbatim by the accepted DESIGN contract"
)]

use std::fmt::{self, Display, Formatter};
use std::num::{NonZeroU16, NonZeroU64};
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::codec::{EnvelopeError, VersionedEnvelope};
use crate::dataplane::{Proto, ServiceFrontend};
use crate::gateway_identity::{GatewayIdentityEpoch, GatewayIdentityFacts};
use crate::id::{
    BackendId, CertSerial, CorrelationKey, IdParseError, NodeId, ServiceId, ServiceVip, SpiffeId,
    WorkloadId,
};
use crate::traits::intent_store::IntentStoreError;
use crate::traits::observation_store::{LogicalTimestamp, ObservationStoreError};
use crate::wall_clock::UnixInstant;

macro_rules! canonical_string_newtype {
    ($name:ty, $error:ty) => {
        impl Display for $name {
            fn fmt(&self, _formatter: &mut Formatter<'_>) -> fmt::Result {
                todo!(concat!("SCAFFOLD: ", stringify!($name), " Display"))
            }
        }
        impl FromStr for $name {
            type Err = $error;
            fn from_str(_raw: &str) -> Result<Self, Self::Err> {
                todo!(concat!("SCAFFOLD: ", stringify!($name), " FromStr"))
            }
        }
        impl<'a> TryFrom<&'a str> for $name {
            type Error = $error;
            fn try_from(raw: &'a str) -> Result<Self, Self::Error> {
                raw.parse()
            }
        }
        impl TryFrom<String> for $name {
            type Error = $error;
            fn try_from(raw: String) -> Result<Self, Self::Error> {
                raw.parse()
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

macro_rules! gateway_label {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash,
            Serialize, Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
        )]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);
        impl $name {
            /// Construct and canonicalize this operator-supplied label.
            pub fn new(_raw: &str) -> Result<Self, IdParseError> {
                todo!("SCAFFOLD: public-ingress label constructor")
            }
            /// Borrow the canonical label.
            pub fn as_str(&self) -> &str {
                todo!("SCAFFOLD: public-ingress label accessor")
            }
        }
        impl Display for $name {
            fn fmt(&self, _f: &mut Formatter<'_>) -> fmt::Result {
                todo!("SCAFFOLD: public-ingress label Display")
            }
        }
        impl FromStr for $name {
            type Err = IdParseError;
            fn from_str(raw: &str) -> Result<Self, Self::Err> { Self::new(raw) }
        }
        impl TryFrom<String> for $name {
            type Error = IdParseError;
            fn try_from(raw: String) -> Result<Self, Self::Error> { Self::new(&raw) }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self { value.0 }
        }
    };
}

gateway_label!(/// Stable identity of the singleton Route entity.
    RouteId);
gateway_label!(/// Stable identity of a protected public certified key.
    PublicCertifiedKeyId);

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
        todo!("SCAFFOLD: PublicHostname::as_str")
    }
}

impl Display for PublicHostname {
    fn fmt(&self, _f: &mut Formatter<'_>) -> fmt::Result {
        todo!("SCAFFOLD: PublicHostname Display")
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
        todo!("SCAFFOLD: PublicPath::as_str")
    }
}

impl Display for PublicPath {
    fn fmt(&self, _f: &mut Formatter<'_>) -> fmt::Result {
        todo!("SCAFFOLD: PublicPath Display")
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
        todo!("SCAFFOLD: ServiceListenerReference::service")
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

/// Public Route HTTP input.
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

/// Wire/parser input for an exact Service listener reference.
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
        todo!("SCAFFOLD: Route::route_id")
    }
    pub const fn route_generation(&self) -> RouteGeneration {
        self.route_generation
    }
    pub fn public_hostname(&self) -> &PublicHostname {
        todo!("SCAFFOLD: Route::public_hostname")
    }
    pub fn path_match(&self) -> &PathMatch {
        todo!("SCAFFOLD: Route::path_match")
    }
    pub fn service_listener(&self) -> &ServiceListenerReference {
        todo!("SCAFFOLD: Route::service_listener")
    }
    pub fn public_certified_key_id(&self) -> &PublicCertifiedKeyId {
        todo!("SCAFFOLD: Route::public_certified_key_id")
    }
}

/// Deterministic digest of every Route field.
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
pub struct RouteGeneration([u8; 32]);

impl RouteGeneration {
    pub fn new(_raw: &str) -> Result<Self, DigestHexParseError> {
        todo!("SCAFFOLD: RouteGeneration::new")
    }
    pub fn from_route_fields(
        _route_id: &RouteId,
        _hostname: &PublicHostname,
        _path_match: &PathMatch,
        _listener: &ServiceListenerReference,
        _certified_key_id: &PublicCertifiedKeyId,
    ) -> Self {
        todo!("SCAFFOLD: RouteGeneration::from_route_fields")
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Deterministic digest of a complete ordered certificate chain.
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
pub struct CertifiedKeyGeneration([u8; 32]);
impl CertifiedKeyGeneration {
    pub fn new(_raw: &str) -> Result<Self, DigestHexParseError> {
        todo!("SCAFFOLD: CertifiedKeyGeneration::new")
    }
    pub fn from_ordered_chain_der(_chain: &[Vec<u8>]) -> Self {
        todo!("SCAFFOLD: CertifiedKeyGeneration::from_ordered_chain_der")
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// SHA-256 fingerprint of the public leaf certificate.
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
pub struct CertificateFingerprint([u8; 32]);
impl CertificateFingerprint {
    pub fn new(_raw: &str) -> Result<Self, DigestHexParseError> {
        todo!("SCAFFOLD: CertificateFingerprint::new")
    }
    pub fn from_leaf_der(_leaf: &[u8]) -> Self {
        todo!("SCAFFOLD: CertificateFingerprint::from_leaf_der")
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Closed lowercase SHA-256 parse failures.
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

/// Nonzero process-local revision of the derived frontend demand set.
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
pub struct GatewayFrontendDemandRevision(NonZeroU64);
impl GatewayFrontendDemandRevision {
    pub fn new(_raw: u64) -> Result<Self, GatewayFrontendDemandRevisionParseError> {
        todo!("SCAFFOLD: GatewayFrontendDemandRevision::new")
    }
    pub const fn first() -> Self {
        Self(NonZeroU64::MIN)
    }
    pub fn checked_next(self) -> Result<Self, GatewayFrontendDemandError> {
        todo!("SCAFFOLD: GatewayFrontendDemandRevision::checked_next")
    }
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Frontend-demand revision parse failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GatewayFrontendDemandRevisionParseError {
    #[error("revision is empty")]
    Empty,
    #[error("revision is not canonical decimal")]
    InvalidDecimal,
    #[error("revision must be nonzero")]
    Zero,
}

/// Lifecycle phase of a derived Gateway Application frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayDemandPhase {
    Staged,
    Current,
    Draining,
}

/// One phase-qualified derived frontend demand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayFrontendDemandEntry {
    pub application_generation: GatewayApplicationGenerationId,
    pub service_key: ServiceKey,
    pub frontend: ServiceFrontend,
    pub phase: GatewayDemandPhase,
}

/// Exact observable snapshot of derived gateway frontend demand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayFrontendDemandSnapshot {
    pub revision: GatewayFrontendDemandRevision,
    pub entries: Vec<GatewayFrontendDemandEntry>,
}

/// Exact revision/key acknowledgment carried by a demand-bearing apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayFrontendDemandApply {
    pub revision: GatewayFrontendDemandRevision,
    pub service_key: ServiceKey,
}

/// Read-only demand snapshot port.
pub trait GatewayFrontendDemandRead: Send + Sync {
    fn snapshot(&self) -> Arc<GatewayFrontendDemandSnapshot>;
}

/// Exact-revision apply acknowledgment port.
pub trait GatewayFrontendDemandAcknowledge: Send + Sync {
    fn applied(&self, apply: GatewayFrontendDemandApply) -> GatewayDemandAckOutcome;
}

/// Demand acknowledgment result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayDemandAckOutcome {
    Applied,
    Stale,
}

/// Wake the existing ServiceMapHydrator target for a frontend.
pub trait GatewayFrontendDemandWake: Send + Sync {
    fn wake(&self, service_id: ServiceId);
}

/// Closed derived-demand lifecycle failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GatewayFrontendDemandError {
    #[error("frontend-demand revision exhausted")]
    RevisionExhausted,
    #[error("generation is already staged")]
    AlreadyStaged,
    #[error("generation is unknown")]
    UnknownGeneration,
    #[error("generation is not applied")]
    NotApplied,
    #[error("generation was superseded")]
    Superseded,
    #[error("frontend-demand application timed out")]
    ApplyTimeout,
    #[error("frontend-demand owner is unavailable")]
    OwnerUnavailable,
}

/// Deterministic identity of a coherent Gateway Application generation.
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
pub struct GatewayApplicationGenerationId([u8; 32]);
impl GatewayApplicationGenerationId {
    pub fn new(_raw: &str) -> Result<Self, DigestHexParseError> {
        todo!("SCAFFOLD: GatewayApplicationGenerationId::new")
    }
    pub fn from_inputs(
        _route_generation: RouteGeneration,
        _frontend: ServiceFrontend,
        _certified_key_generation: CertifiedKeyGeneration,
    ) -> Self {
        todo!("SCAFFOLD: GatewayApplicationGenerationId::from_inputs")
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Nonzero Linux socket cookie read from an upstream socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SocketCookie(NonZeroU64);
impl SocketCookie {
    pub fn new(_raw: u64) -> Result<Self, SocketCookieParseError> {
        todo!("SCAFFOLD: SocketCookie::new")
    }
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

canonical_string_newtype!(RouteGeneration, DigestHexParseError);
canonical_string_newtype!(CertifiedKeyGeneration, DigestHexParseError);
canonical_string_newtype!(CertificateFingerprint, DigestHexParseError);
canonical_string_newtype!(GatewayApplicationGenerationId, DigestHexParseError);
canonical_string_newtype!(GatewayFrontendDemandRevision, GatewayFrontendDemandRevisionParseError);
canonical_string_newtype!(SocketCookie, SocketCookieParseError);

/// One exact gateway-connect intent.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GatewayConnectIntent {
    pub socket_cookie: SocketCookie,
    pub service_key: ServiceKey,
}

/// BPF-owned selection result for one exact intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewaySelectionReceipt {
    Selected { socket_cookie: SocketCookie, service_key: ServiceKey, backend_id: BackendId },
    NoBackend { socket_cookie: SocketCookie, service_key: ServiceKey },
}

/// Application generation and exact applied frontend-demand revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct GatewayAppliedGeneration {
    application_generation: GatewayApplicationGenerationId,
    demand_revision: GatewayFrontendDemandRevision,
}

impl GatewayAppliedGeneration {
    #[expect(
        dead_code,
        reason = "DISTILL RED scaffold is activated by Gateway Application promotion"
    )]
    pub(crate) const fn from_promotion(
        application_generation: GatewayApplicationGenerationId,
        demand_revision: GatewayFrontendDemandRevision,
    ) -> Self {
        Self { application_generation, demand_revision }
    }
    pub const fn application_generation(&self) -> GatewayApplicationGenerationId {
        self.application_generation
    }
    pub const fn demand_revision(&self) -> GatewayFrontendDemandRevision {
        self.demand_revision
    }
}

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

/// Protected origin private-key record. Debug is deliberately redacted.
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
    Bound { address: std::net::SocketAddrV4 },
}

/// One phase-qualified application-generation projection.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct GatewayApplicationGenerationStatus {
    pub application_generation: GatewayApplicationGenerationId,
    pub route_id: RouteId,
    pub route_generation: RouteGeneration,
    pub service_key: ServiceKey,
    pub demand_revision: GatewayFrontendDemandRevision,
}

/// Closed reason no complete Gateway Application is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum GatewayApplicationUnavailableCause {
    RouteAbsent,
    RouteUnreadable,
    CertifiedKeyAbsent,
    CertifiedKeyUnusable,
    FrontendUnresolved,
    DemandNotApplied,
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

impl From<GatewayIdentityFacts> for GatewayIdentityStatus {
    fn from(facts: GatewayIdentityFacts) -> Self {
        Self::Current {
            epoch: facts.epoch,
            spiffe_id: facts.spiffe_id,
            serial: facts.serial,
            not_after: facts.not_after,
        }
    }
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
    pub connect_path: GatewayConnectPathStatus,
    pub updated_at: LogicalTimestamp,
}

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

#[cfg(test)]
mod properties {
    use std::time::Duration;

    use super::*;
    use proptest::prelude::*;

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

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER Route value step"]
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

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER public hostname boundary step"]
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

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER Route validation step"]
    fn every_route_input_channel_has_a_cause_distinct_malformed_example() {
        let canonical = || PublicRouteInput {
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
        };

        let mut cases: Vec<RouteErrorCase> = Vec::new();
        let mut bad = canonical();
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
        let mut bad = canonical();
        bad.hostname = "*.example.com".to_owned();
        cases.push((
            bad,
            RouteValidationError::Hostname(PublicHostnameError::Wildcard),
            "hostname",
        ));
        let mut bad = canonical();
        bad.path = "relative".to_owned();
        cases.push((bad, RouteValidationError::Path(PublicPathError::NotAbsolute), "path"));
        let mut bad = canonical();
        bad.path_match = "regex".to_owned();
        cases.push((bad, RouteValidationError::UnknownPathMatch, "path_match"));
        let mut bad = canonical();
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
        let mut bad = canonical();
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
        let mut bad = canonical();
        bad.target.port = 0;
        cases.push((bad, RouteValidationError::PortZero, "port"));
        let mut bad = canonical();
        bad.target.protocol = "sctp".to_owned();
        cases.push((bad, RouteValidationError::UnknownProtocol, "protocol"));
        let mut bad = canonical();
        bad.target.protocol = "udp".to_owned();
        cases.push((bad, RouteValidationError::ProtocolNotTcp, "protocol_not_tcp"));

        for (input, expected, field) in cases {
            let error = Route::from_submit(input).expect_err("malformed partition must fail");
            assert_eq!(error, expected, "{field} returned wrong closed variant");
        }
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn route_target_accepts_only_service_and_rejects_the_old_workload_member() {
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

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER Route path step"]
        fn exact_path_matches_only_byte_equal_paths(path in arb_valid_path(), suffix in "[a-z]{1,8}") {
            let approved = PublicPath::new(&path).expect("generated path is valid");
            let matcher = PathMatch::Exact(approved);
            let extended = format!("{path}{suffix}");
            prop_assert!(matcher.matches_raw_path(&path));
            prop_assert!(!matcher.matches_raw_path(&extended));
        }

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER Route path step"]
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

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER Route generation step"]
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

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER certified-key generation step"]
        fn certified_key_generation_preserves_chain_order(
            leaf in prop::collection::vec(any::<u8>(), 1..64),
            issuer in prop::collection::vec(any::<u8>(), 1..64),
        ) {
            prop_assume!(leaf != issuer);
            let forward = CertifiedKeyGeneration::from_ordered_chain_der(&[leaf.clone(), issuer.clone()]);
            let reverse = CertifiedKeyGeneration::from_ordered_chain_der(&[issuer, leaf]);
            prop_assert_ne!(forward, reverse);
        }

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER application generation step"]
        fn application_generation_changes_when_the_frontend_changes(port in 1u16..u16::MAX) {
            let route = RouteGeneration::new(&"01".repeat(32)).expect("valid generation");
            let key = CertifiedKeyGeneration::new(&"02".repeat(32)).expect("valid generation");
            let vip_a = ServiceVip::new("10.96.0.1".parse().expect("IP")).expect("VIP");
            let vip_b = ServiceVip::new("10.96.0.2".parse().expect("IP")).expect("VIP");
            let a = ServiceFrontend::new(vip_a, NonZeroU16::new(port).expect("nonzero"), Proto::Tcp).expect("frontend");
            let other_port = ServiceFrontend::new(vip_a, NonZeroU16::new(port + 1).expect("nonzero"), Proto::Tcp).expect("frontend");
            let other_vip = ServiceFrontend::new(vip_b, NonZeroU16::new(port).expect("nonzero"), Proto::Tcp).expect("frontend");
            prop_assert_ne!(
                GatewayApplicationGenerationId::from_inputs(route, a, key),
                GatewayApplicationGenerationId::from_inputs(route, other_port, key),
            );
            prop_assert_ne!(
                GatewayApplicationGenerationId::from_inputs(route, a, key),
                GatewayApplicationGenerationId::from_inputs(route, other_vip, key),
            );
        }
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER demand revision step"]
    fn demand_revision_is_nonzero_monotone_and_refuses_wrap() {
        assert_eq!(GatewayFrontendDemandRevision::first().get(), 1);
        assert_eq!(
            GatewayFrontendDemandRevision::new(1)
                .expect("one is valid")
                .checked_next()
                .expect("two is representable")
                .get(),
            2,
        );
        assert_eq!(
            GatewayFrontendDemandRevision::new(u64::MAX)
                .expect("maximum is nonzero")
                .checked_next(),
            Err(GatewayFrontendDemandError::RevisionExhausted),
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER canonical public-ingress newtypes"]
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
            let revision = GatewayFrontendDemandRevision::new(numeric).expect("revision");
            prop_assert_eq!(revision.to_string(), numeric.to_string());
            prop_assert_eq!(numeric.to_string().parse::<GatewayFrontendDemandRevision>(), Ok(revision));
            prop_assert_eq!(GatewayFrontendDemandRevision::try_from(numeric.to_string().as_str()), Ok(revision));
            prop_assert_eq!(GatewayFrontendDemandRevision::try_from(numeric.to_string()), Ok(revision));
            prop_assert_eq!(serde_json::from_str::<GatewayFrontendDemandRevision>(
                &serde_json::to_string(&revision).expect("serialize"),
            ).expect("deserialize"), revision);
            let revision_archive = rkyv::to_bytes::<rkyv::rancor::Error>(&revision)
                .expect("revision rkyv archive");
            prop_assert_eq!(
                rkyv::from_bytes::<GatewayFrontendDemandRevision, rkyv::rancor::Error>(
                    &revision_archive,
                ).expect("revision rkyv valid trip"),
                revision,
            );

            let route = RouteGeneration::new(&digest).expect("Route generation");
            prop_assert_eq!(route.to_string(), digest.as_str());
            prop_assert_eq!(RouteGeneration::try_from(digest.as_str()), Ok(route));
            prop_assert_eq!(RouteGeneration::try_from(digest.clone()), Ok(route));
            prop_assert_eq!(serde_json::from_str::<RouteGeneration>(
                &serde_json::to_string(&route).expect("serialize"),
            ).expect("deserialize"), route);
            let route_archive = rkyv::to_bytes::<rkyv::rancor::Error>(&route)
                .expect("Route generation rkyv archive");
            prop_assert_eq!(
                rkyv::from_bytes::<RouteGeneration, rkyv::rancor::Error>(&route_archive)
                    .expect("Route generation rkyv valid trip"),
                route,
            );
            let certified = CertifiedKeyGeneration::new(&digest).expect("key generation");
            prop_assert_eq!(certified.to_string(), digest.as_str());
            prop_assert_eq!(digest.parse::<CertifiedKeyGeneration>(), Ok(certified));
            prop_assert_eq!(CertifiedKeyGeneration::try_from(digest.as_str()), Ok(certified));
            prop_assert_eq!(CertifiedKeyGeneration::try_from(digest.clone()), Ok(certified));
            prop_assert_eq!(serde_json::from_str::<CertifiedKeyGeneration>(
                &serde_json::to_string(&certified).expect("serialize"),
            ).expect("deserialize"), certified);
            let certified_archive = rkyv::to_bytes::<rkyv::rancor::Error>(&certified)
                .expect("Certified Key generation rkyv archive");
            prop_assert_eq!(
                rkyv::from_bytes::<CertifiedKeyGeneration, rkyv::rancor::Error>(&certified_archive)
                    .expect("Certified Key generation rkyv valid trip"),
                certified,
            );
            let fingerprint = CertificateFingerprint::new(&digest).expect("fingerprint");
            prop_assert_eq!(fingerprint.to_string(), digest.as_str());
            prop_assert_eq!(CertificateFingerprint::try_from(digest.as_str()), Ok(fingerprint));
            prop_assert_eq!(CertificateFingerprint::try_from(digest.clone()), Ok(fingerprint));
            prop_assert_eq!(serde_json::from_str::<CertificateFingerprint>(
                &serde_json::to_string(&fingerprint).expect("serialize"),
            ).expect("deserialize"), fingerprint);
            let fingerprint_archive = rkyv::to_bytes::<rkyv::rancor::Error>(&fingerprint)
                .expect("fingerprint rkyv archive");
            prop_assert_eq!(
                rkyv::from_bytes::<CertificateFingerprint, rkyv::rancor::Error>(
                    &fingerprint_archive,
                ).expect("fingerprint rkyv valid trip"),
                fingerprint,
            );
            let application = GatewayApplicationGenerationId::new(&digest).expect("application generation");
            prop_assert_eq!(application.to_string(), digest.as_str());
            prop_assert_eq!(digest.parse::<GatewayApplicationGenerationId>(), Ok(application));
            prop_assert_eq!(GatewayApplicationGenerationId::try_from(digest.as_str()), Ok(application));
            prop_assert_eq!(GatewayApplicationGenerationId::try_from(digest), Ok(application));
            prop_assert_eq!(serde_json::from_str::<GatewayApplicationGenerationId>(
                &serde_json::to_string(&application).expect("serialize"),
            ).expect("deserialize"), application);
            let application_archive = rkyv::to_bytes::<rkyv::rancor::Error>(&application)
                .expect("Gateway Application generation rkyv archive");
            prop_assert_eq!(
                rkyv::from_bytes::<GatewayApplicationGenerationId, rkyv::rancor::Error>(
                    &application_archive,
                ).expect("Gateway Application generation rkyv valid trip"),
                application,
            );
        }
    }

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER canonical public-ingress newtypes"]
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
        assert_eq!(
            "".parse::<GatewayFrontendDemandRevision>(),
            Err(GatewayFrontendDemandRevisionParseError::Empty),
        );
        for raw in ["+1", "-1", "01", " 1", "1 ", "18446744073709551616"] {
            assert_eq!(
                raw.parse::<GatewayFrontendDemandRevision>(),
                Err(GatewayFrontendDemandRevisionParseError::InvalidDecimal),
                "GatewayFrontendDemandRevision partition {raw:?}",
            );
        }
        assert_eq!(
            "0".parse::<GatewayFrontendDemandRevision>(),
            Err(GatewayFrontendDemandRevisionParseError::Zero),
        );

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

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "fixture regeneration tool for the four public-ingress V1 envelope goldens"]
    #[allow(clippy::print_stdout, reason = "prints one-shot golden fixture hex")]
    fn print_public_ingress_v1_fixture_bytes() {
        let writer = NodeId::new("gateway-node").expect("writer");
        let key_id = PublicCertifiedKeyId("api-origin".to_owned());
        let hostname = PublicHostname("api.example.com".to_owned());
        let key = PublicCertifiedKeyV1 {
            certified_key_id: key_id.clone(),
            public_hostname: hostname,
            certified_key_generation: CertifiedKeyGeneration([1; 32]),
            certificate_fingerprint: CertificateFingerprint([2; 32]),
            certificate_chain_der: vec![vec![3, 4, 5]],
            not_before: UnixInstant::from_unix_duration(Duration::from_secs(1_000)),
            not_after: UnixInstant::from_unix_duration(Duration::from_secs(2_000)),
            provenance: CertifiedKeyProvenance::Manual,
            protected_origin_key: ProtectedOriginKeyV1 {
                kek_id: "overdrive-public-certified-key".to_owned(),
                salt: [6; 32],
                nonce: [7; 12],
                ciphertext_and_tag: vec![8, 9, 10],
            },
        };
        let key_status = PublicCertifiedKeyStatusRowV1 {
            certified_key_id: key_id,
            state: PublicCertifiedKeyStatusState::Absent,
            last_install_failure: None,
            updated_at: LogicalTimestamp { counter: 1, writer: writer.clone() },
        };
        let application_status = GatewayApplicationStatusRowV1 {
            node_id: writer.clone(),
            listener: GatewayListenerStatus::Unbound,
            staged: None,
            current: None,
            draining: vec![],
            unavailable: Some(GatewayApplicationUnavailableCause::RouteAbsent),
            gateway_identity: GatewayIdentityStatus::Absent,
            connect_path: GatewayConnectPathStatus::Unavailable,
            updated_at: LogicalTimestamp { counter: 2, writer },
        };
        let fixtures = [
            (
                "public_route_set",
                rkyv::to_bytes::<rkyv::rancor::Error>(&PublicRouteSetEnvelope::latest(
                    PublicRouteSetV1 { route: None },
                ))
                .expect("Route Set archive"),
            ),
            (
                "public_certified_key",
                rkyv::to_bytes::<rkyv::rancor::Error>(&PublicCertifiedKeyEnvelope::latest(key))
                    .expect("key archive"),
            ),
            (
                "public_certified_key_status_row",
                rkyv::to_bytes::<rkyv::rancor::Error>(
                    &PublicCertifiedKeyStatusRowEnvelope::latest(key_status),
                )
                .expect("key status archive"),
            ),
            (
                "gateway_application_status_row",
                rkyv::to_bytes::<rkyv::rancor::Error>(
                    &GatewayApplicationStatusRowEnvelope::latest(application_status),
                )
                .expect("application status archive"),
            ),
        ];
        for (name, bytes) in fixtures {
            println!("{name}={}", hex::encode(bytes.as_ref()));
        }
    }
}
