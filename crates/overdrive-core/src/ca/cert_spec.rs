//! Pure certificate-profile policy — `CertRole`, `CertSpec`, `CertSpecError`.
//!
//! This is the **decision** of which X.509 extensions and constraints each
//! certificate role carries, expressed in project newtypes and enums — never
//! `rcgen` types. Per ADR-0063 D5 (reconciliation B) the policy lives in
//! `overdrive-core` (class `core`) so it is DST-testable and dst-lint-clean,
//! and the sim adapter shares the exact same profile surface as the host
//! adapter (the host adapter translates `CertSpec → rcgen::CertificateParams`
//! in a later slice; `rcgen` never appears here).
//!
//! # Type-driven design
//!
//! [`CertRole`] is a sum type — `Root`, `Intermediate { path_len }`, `Svid`.
//! The X.509 profile (`is_ca`, `key_usages`, `key_usage_critical`, `path_len`)
//! is **derived from the role**, so an invalid role/extension combination
//! (e.g. an SVID with `keyCertSign`, or an unbounded intermediate) is
//! unrepresentable: there is no constructor that yields it. See
//! `.claude/rules/development.md` § "Type-driven design".
//!
//! # Feature foundation
//!
//! Step 01-01 builds the `Root` profile and the [`CertSpecError`] taxonomy.
//! The `Intermediate` and `Svid` constructors land in later slices (03 / 04)
//! and EXTEND this module — the sum type and the error taxonomy are shaped
//! once, here, so those slices add constructors rather than re-shaping the
//! policy surface.

use crate::SpiffeId;

/// The X.509 role a certificate plays in the trust hierarchy.
///
/// A sum type rather than a `bool is_ca` + `Option<u8> path_len` pair so that
/// invalid combinations are unrepresentable: an SVID carries no path length,
/// and an [`Intermediate`](CertRole::Intermediate) is always bounded by an
/// explicit `path_len` (an *unbounded* intermediate — one that can mint
/// further CAs — cannot be expressed). Per ADR-0063 D5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CertRole {
    /// Self-signed root CA. `CA:TRUE`, `keyCertSign` + `cRLSign`, NO pathLen.
    Root,
    /// Intermediate (node) CA, `CA:TRUE`, signing-only, bounded by `path_len`.
    /// `path_len = 0` issues leaves only — no further intermediates.
    Intermediate {
        /// `basicConstraints` pathLenConstraint — the maximum number of
        /// intermediate CAs that may follow this one in a chain.
        path_len: u8,
    },
    /// Workload leaf (SVID). `CA:FALSE`, `digitalSignature` only, no pathLen.
    Svid,
}

/// An X.509 key-usage bit carried in a [`CertSpec`] profile.
///
/// The project vocabulary for the key-usage extension, kept abstract from any
/// crypto backend (the host adapter maps each variant to its
/// `rcgen::KeyUsagePurpose` counterpart). The set a [`CertSpec`] carries is
/// derived from its [`CertRole`] — see [`CertSpec::key_usages`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KeyUsage {
    /// `keyCertSign` — authority to sign other certificates (CA roles only).
    KeyCertSign,
    /// `cRLSign` — authority to sign certificate revocation lists (root only).
    CrlSign,
    /// `digitalSignature` — leaf authority to sign (SVID).
    DigitalSignature,
}

impl KeyUsage {
    /// Canonical lowercase string form (the X.509 extension name).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KeyCertSign => "keyCertSign",
            Self::CrlSign => "cRLSign",
            Self::DigitalSignature => "digitalSignature",
        }
    }
}

/// A pure certificate-profile specification.
///
/// Carries the [`CertRole`] and the `subject` identity; every other X.509
/// property a consumer observes — `is_ca`, `key_usages`, `key_usage_critical`,
/// `path_len` — is **derived from the role** via the accessors below. This is
/// the port-exposed surface the host and sim adapters both read; neither reads
/// internal fields.
///
/// Issuance inputs that vary per-mint (serial via the `Entropy` port, the
/// `not_before` / `not_after` validity window) are supplied by the adapter at
/// signing time and are NOT part of this pure policy object — keeping
/// `CertSpec` a deterministic function of `(role, subject)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertSpec {
    role: CertRole,
    subject: SpiffeId,
}

impl CertSpec {
    /// Build the **root** certificate profile for a trust domain.
    ///
    /// The produced profile is `CA:TRUE`, carries `keyCertSign` + `cRLSign`,
    /// marks `keyUsage` critical, and carries NO pathLen constraint
    /// ([`CertRole::Root`]). The `subject` is the trust domain only, with no
    /// path component (research Finding 2) — callers pass a [`SpiffeId`] whose
    /// authority is the bare trust domain.
    ///
    /// Infallible: a root profile is always valid for any trust-domain
    /// subject. Per ADR-0063 D5 (`root(...) -> Self`).
    #[must_use]
    pub const fn root(subject: SpiffeId) -> Self {
        Self { role: CertRole::Root, subject }
    }

    /// The role this profile plays in the trust hierarchy.
    #[must_use]
    pub const fn role(&self) -> CertRole {
        self.role
    }

    /// The subject identity (trust domain for CA roles, workload id for SVID).
    #[must_use]
    pub const fn subject(&self) -> &SpiffeId {
        &self.subject
    }

    /// Whether the profile is a CA (`basicConstraints` CA:TRUE).
    ///
    /// Derived from [`role`](Self::role): `true` for `Root` / `Intermediate`,
    /// `false` for `Svid`.
    #[must_use]
    pub const fn is_ca(&self) -> bool {
        match self.role {
            CertRole::Root | CertRole::Intermediate { .. } => true,
            CertRole::Svid => false,
        }
    }

    /// The `basicConstraints` pathLenConstraint, if any.
    ///
    /// `None` for `Root` (an unconstrained root) and `Svid` (a non-CA leaf);
    /// `Some(path_len)` for an [`Intermediate`](CertRole::Intermediate).
    #[must_use]
    pub const fn path_len(&self) -> Option<u8> {
        match self.role {
            CertRole::Intermediate { path_len } => Some(path_len),
            CertRole::Root | CertRole::Svid => None,
        }
    }

    /// The set of key-usage bits this profile carries, in canonical order.
    ///
    /// Derived from [`role`](Self::role): a root carries
    /// `keyCertSign` + `cRLSign`; an intermediate carries `keyCertSign`; an
    /// SVID carries `digitalSignature`. Returns an owned `Vec` so the slice
    /// is stable regardless of how the variants are stored internally (the
    /// observable contract is the set, not the storage).
    #[must_use]
    pub fn key_usages(&self) -> Vec<KeyUsage> {
        match self.role {
            CertRole::Root => vec![KeyUsage::KeyCertSign, KeyUsage::CrlSign],
            CertRole::Intermediate { .. } => vec![KeyUsage::KeyCertSign],
            CertRole::Svid => vec![KeyUsage::DigitalSignature],
        }
    }

    /// Whether the `keyUsage` extension is marked critical.
    ///
    /// Always `true` for every role — the platform marks `keyUsage` critical
    /// on every certificate it mints (CA roles and leaves alike). A `&self`
    /// accessor (rather than an associated fn) so it reads uniformly with the
    /// other port-exposed profile accessors the adapter consumes; the
    /// `expect` self-removes the day a role makes criticality role-dependent.
    #[must_use]
    #[expect(
        clippy::unused_self,
        reason = "uniform per-spec accessor surface; criticality is role-independent today"
    )]
    pub const fn key_usage_critical(&self) -> bool {
        true
    }
}

/// A certificate-profile construction failure.
///
/// Distinct per failure mode (`.claude/rules/development.md` § "Distinct
/// failure modes get distinct error variants"). In particular an invalid SAN
/// cardinality surfaces [`InvalidSan`](CertSpecError::InvalidSan) — it is NOT
/// flattened into a generic `Internal(String)` catch-all, so the load-bearing
/// single-URI-SAN signal (KPI K2) is never swallowed.
///
/// Step 01-01 establishes the taxonomy and its [`InvalidSan`] variant; the
/// SVID constructor that *returns* it lands in slice 04 and reuses this
/// variant rather than minting a parallel one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CertSpecError {
    /// The requested subject would yield an invalid number of `spiffe://` URI
    /// SANs. A SPIFFE-compliant SVID carries exactly one URI SAN; a projection
    /// yielding zero or two-or-more is rejected before any certificate is
    /// produced (research Finding 2 / KPI K2).
    #[error("invalid SAN cardinality: expected exactly one spiffe:// URI SAN, found {found}")]
    InvalidSan {
        /// The number of URI SANs the rejected request would have produced.
        found: usize,
    },

    /// The subject identity is not valid for the requested role — e.g. a root
    /// or intermediate subject carrying a path component where the trust
    /// domain alone is required.
    #[error("invalid subject for {role:?}: {reason}")]
    InvalidSubject {
        /// The role whose subject contract was violated.
        role: &'static str,
        /// Human-readable explanation of the contract violation.
        reason: &'static str,
    },
}

impl CertSpecError {
    /// Construct an [`InvalidSan`](CertSpecError::InvalidSan) for a rejected
    /// SAN cardinality.
    #[must_use]
    pub const fn invalid_san(found: usize) -> Self {
        Self::InvalidSan { found }
    }

    /// Construct an [`InvalidSubject`](CertSpecError::InvalidSubject).
    #[must_use]
    pub const fn invalid_subject(role: &'static str, reason: &'static str) -> Self {
        Self::InvalidSubject { role, reason }
    }
}
