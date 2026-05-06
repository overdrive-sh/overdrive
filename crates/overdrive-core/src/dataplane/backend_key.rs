//! `BackendKey` — STRICT newtype for the `REVERSE_NAT_MAP` key.
//!
//! Triple `(ip, port, proto)` per
//! `docs/feature/phase-2-xdp-service-map/discuss/user-stories.md` US-05
//! AC #1. Used as the kernel-side key for reverse-NAT lookups: when a
//! backend response packet hits the egress path, the kernel looks up
//! `(backend_ip, backend_port, proto)` → `Vip` to rewrite the source
//! address back to the original VIP the client connected to.
//!
//! # Wire form
//!
//! `Display` emits the canonical `"<ip>:<port>/<proto>"` form (e.g.
//! `10.0.0.1:8080/tcp`). `FromStr` parses the same shape;
//! case-insensitive on the proto token (matches the `ServiceVip` IPv6
//! hex casing precedent for human-typed identifiers per
//! `.claude/rules/development.md` § Newtype completeness).
//!
//! `Serialize` / `Deserialize` use the structured form via
//! `#[serde(try_from = "String", into = "String")]` so wire payloads
//! carrying malformed inputs are rejected at the deserialisation
//! boundary, not silently accepted. JSON form is the canonical string
//! form — preserves audit-log readability while staying canonical for
//! content-hashing.
//!
//! # Endianness lockstep (architecture.md § 11)
//!
//! Stored host-order on the userspace side. The kernel-side egress
//! program converts at the read boundary against incoming wire-order
//! packets. Userspace stores host-order without flipping — the same
//! lockstep contract `ServiceMapHandle` and `BackendMapHandle` carry.
//!
//! # IANA proto codes
//!
//! `Proto::Tcp = 6`, `Proto::Udp = 17` per RFC 1700 / IANA
//! protocol-numbers registry. The two L4 protocols Phase 2.2 supports;
//! IPv6 / ICMP / SCTP are GH #155 / future-phase deferrals.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

use core::fmt;
use std::net::Ipv4Addr;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// L4 protocol discriminator. Only `Tcp` and `Udp` are recognised in
/// Phase 2.2 — these are the two protocols the egress reverse-NAT
/// path supports per architecture.md § 6.
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
#[serde(rename_all = "lowercase")]
pub enum Proto {
    /// TCP. IANA proto number 6.
    Tcp,
    /// UDP. IANA proto number 17.
    Udp,
}

impl Proto {
    /// IANA proto number — TCP=6, UDP=17.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Tcp => 6,
            Self::Udp => 17,
        }
    }

    /// Canonical lowercase token form used in [`BackendKey`]'s
    /// `Display` output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

impl fmt::Display for Proto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<u8> for Proto {
    type Error = ParseError;

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            6 => Ok(Self::Tcp),
            17 => Ok(Self::Udp),
            other => Err(ParseError::UnknownProto(other.to_string())),
        }
    }
}

/// `REVERSE_NAT_MAP` key — `(ip, port, proto)` triple. Stored
/// host-order; kernel-side egress program converts at read boundary
/// per architecture.md § 11.
///
/// Constructed via [`BackendKey::new`] (infallible — every
/// `(Ipv4Addr, u16, Proto)` triple is structurally valid) or
/// [`BackendKey::from_str`] (validating). Raw fields for any persisted
/// reverse-NAT key are a blocking violation per
/// `.claude/rules/development.md` § Newtype completeness.
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
pub struct BackendKey {
    /// Backend IPv4 address. Host-order on the userspace side.
    pub ip: Ipv4Addr,
    /// Backend port. Host-order on the userspace side.
    pub port: u16,
    /// L4 protocol — TCP or UDP.
    pub proto: Proto,
}

/// Parse / validation failure for [`BackendKey`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// Empty input. Distinct from [`Self::Malformed`] so callers can
    /// surface a precise diagnostic rather than a generic parse error.
    #[error("BackendKey input must not be empty")]
    Empty,

    /// Input did not match the canonical `"<ip>:<port>/<proto>"`
    /// shape. Carries the rejected substring for operator-facing
    /// diagnostics.
    #[error("BackendKey malformed: {0}")]
    Malformed(String),

    /// Proto token was syntactically present but did not match `tcp`
    /// or `udp`. Phase 2.2 supports exactly these two L4 protocols
    /// (architecture.md § 6).
    #[error("BackendKey unknown proto: {0:?} (expected tcp or udp)")]
    UnknownProto(String),
}

impl BackendKey {
    /// Infallible constructor. Every `(Ipv4Addr, u16, Proto)` triple
    /// is a valid key — the newtype's role is type-system distinctness
    /// and wire-form lockstep, not runtime range-check.
    #[must_use]
    pub const fn new(ip: Ipv4Addr, port: u16, proto: Proto) -> Self {
        Self { ip, port, proto }
    }
}

impl fmt::Display for BackendKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}/{}", self.ip, self.port, self.proto)
    }
}

impl FromStr for BackendKey {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(ParseError::Empty);
        }
        let (addr_port, proto_str) = s.rsplit_once('/').ok_or_else(|| {
            ParseError::Malformed(format!(
                "missing '/' proto separator (expected '<ip>:<port>/<proto>'): {s:?}"
            ))
        })?;
        let (ip_str, port_str) = addr_port.rsplit_once(':').ok_or_else(|| {
            ParseError::Malformed(format!(
                "missing ':' port separator (expected '<ip>:<port>/<proto>'): {s:?}"
            ))
        })?;
        let ip = ip_str
            .parse::<Ipv4Addr>()
            .map_err(|e| ParseError::Malformed(format!("invalid IPv4 {ip_str:?}: {e}")))?;
        let port = port_str
            .parse::<u16>()
            .map_err(|e| ParseError::Malformed(format!("invalid port {port_str:?}: {e}")))?;
        let proto = match proto_str.to_ascii_lowercase().as_str() {
            "tcp" => Proto::Tcp,
            "udp" => Proto::Udp,
            other => return Err(ParseError::UnknownProto(other.to_owned())),
        };
        Ok(Self::new(ip, port, proto))
    }
}

impl TryFrom<String> for BackendKey {
    type Error = ParseError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str(&s)
    }
}

impl TryFrom<&str> for BackendKey {
    type Error = ParseError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::from_str(s)
    }
}

impl From<BackendKey> for String {
    fn from(v: BackendKey) -> Self {
        v.to_string()
    }
}
