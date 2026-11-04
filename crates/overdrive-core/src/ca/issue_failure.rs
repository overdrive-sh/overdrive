//! Shared value types of a rejected SVID issuance (gateway and workload).
//!
//! The failure class only, never the source text of the CA or audit error.
//! Both halves of the SVID issuance failure occurrence carry these values.

use crate::wall_clock::UnixInstant;

/// Failure class of a rejected issuance; the `CaIssuanceError` variant class
/// only, never its source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum SvidIssueFailureCause {
    /// The CA could not sign.
    Ca,
    /// The `issued_certificates` audit write failed.
    Audit,
}

/// Depth-1 snapshot of the latest failed issuance. Not nestable: it carries
/// no failure of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct SvidIssueFailure {
    /// Failure class.
    pub cause: SvidIssueFailureCause,
    /// The injected clock's `unix_now()` when the executor observed the `Err`.
    pub at: UnixInstant,
}
