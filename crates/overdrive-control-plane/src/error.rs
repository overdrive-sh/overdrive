//! `ControlPlaneError` — top-level typed error with pass-through `#[from]`.
//!
//! SCAFFOLD: true — created by DISTILL wave for phase-1-control-plane-core.
//!
//! Per ADR-0015, one top-level enum. Exhaustive `to_response` function
//! maps every variant to `(StatusCode, Json<ErrorBody>)`. Body shape is
//! a deliberate RFC 7807-compatible subset so v1.1 upgrade is additive.

use thiserror::Error;

/// Top-level control-plane error.
///
/// SCAFFOLD: true
#[derive(Debug, Error)]
pub enum ControlPlaneError {
    #[error("validation: {field:?}: {message}")]
    Validation { message: String, field: Option<String> },

    #[error("not found: {resource}")]
    NotFound { resource: String },

    #[error("conflict: {message}")]
    Conflict { message: String },

    #[error(transparent)]
    Intent(#[from] overdrive_core::traits::intent_store::IntentStoreError),

    #[error(transparent)]
    Observation(#[from] overdrive_core::traits::observation_store::ObservationStoreError),

    #[error(transparent)]
    Aggregate(#[from] overdrive_core::aggregate::AggregateError),

    #[error("internal: {0}")]
    Internal(String),
}

/// Map a `ControlPlaneError` to `(StatusCode, Json<ErrorBody>)` per
/// ADR-0015 Table §3.
///
/// SCAFFOLD: true — returns a stub shape so the call site compiles;
/// the DELIVER crafter fills in per-variant mapping.
#[allow(clippy::missing_errors_doc)]
pub fn to_response(_err: ControlPlaneError) -> (u16, crate::api::ErrorBody) {
    panic!("Not yet implemented -- RED scaffold")
}
