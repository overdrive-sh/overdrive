//! `ControlPlaneError` — top-level typed error with pass-through `#[from]`.
//!
//! Per ADR-0015, one top-level enum. Exhaustive `to_response` function
//! maps every variant to `(StatusCode, Json<ErrorBody>)`. Body shape is
//! a deliberate RFC 7807-compatible subset so v1.1 upgrade is additive.

use std::fmt;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use thiserror::Error;

use crate::api::ErrorBody;

/// Top-level control-plane error.
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

    /// TLS-bootstrap failure (cert mint, trust-triple I/O, PEM parse,
    /// rustls config). Pass-through embedding per ADR-0015 §Consequences:
    /// preserves the structured upstream chain (`rcgen::Error`,
    /// `io::Error`, `toml::de::Error`, `base64::DecodeError`,
    /// `rustls::Error`) for audit logs and the §12 investigation agent
    /// instead of stringifying it through [`ControlPlaneError::Internal`].
    /// Maps to `500 Internal` on the wire — TLS bootstrap is infra
    /// failure (ADR-0015 §4 Status-code matrix).
    #[error(transparent)]
    Tls(#[from] crate::tls_bootstrap::TlsBootstrapError),

    #[error("internal: {0}")]
    Internal(String),
}

impl ControlPlaneError {
    /// Construct an [`ControlPlaneError::Internal`] from a context label
    /// and an underlying error. The rendered message is
    /// `"{context}: {source}"`, matching the shape call sites previously
    /// built by hand with `format!`.
    ///
    /// Using this constructor over raw `Internal(format!(...))` keeps
    /// the 40-odd infrastructure error sites in this crate consistent
    /// and lets a future `Internal` variant evolution (e.g. structured
    /// `{context, source}`) land without touching every call site.
    pub fn internal(context: impl fmt::Display, source: impl fmt::Display) -> Self {
        Self::Internal(format!("{context}: {source}"))
    }
}

/// Map a `ControlPlaneError` to `(StatusCode, ErrorBody)` per ADR-0015
/// Table §3. Exhaustive at the enum level so a forgotten variant is a
/// compile-time error.
///
/// Returns the body as a plain struct (not `Json<...>`) so callers can
/// decide whether to serialise immediately or attach headers first;
/// [`IntoResponse`] wraps this in `Json(...)` for the axum handler path.
#[must_use]
pub fn to_response(err: ControlPlaneError) -> (StatusCode, ErrorBody) {
    use overdrive_core::aggregate::AggregateError;
    use overdrive_core::traits::intent_store::IntentStoreError;

    match err {
        ControlPlaneError::Validation { message, field } => {
            (StatusCode::BAD_REQUEST, ErrorBody { error: "validation".into(), message, field })
        }
        ControlPlaneError::NotFound { resource } => (
            StatusCode::NOT_FOUND,
            ErrorBody { error: "not_found".into(), message: resource, field: None },
        ),
        ControlPlaneError::Conflict { message } => {
            (StatusCode::CONFLICT, ErrorBody { error: "conflict".into(), message, field: None })
        }
        ControlPlaneError::Intent(IntentStoreError::NotFound) => (
            StatusCode::NOT_FOUND,
            ErrorBody {
                error: "not_found".into(),
                message: "intent-store key not found".into(),
                field: None,
            },
        ),
        ControlPlaneError::Intent(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::Observation(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::Aggregate(e) => {
            // Pull the offending field out of the wrapped `AggregateError`
            // when available, so the ErrorBody's `field` is not always
            // `None` for validation errors routed through `#[from]`.
            let field = match &e {
                AggregateError::Validation { field, .. } => Some((*field).to_string()),
                AggregateError::Id(_) | AggregateError::Resources(_) => None,
            };
            (
                StatusCode::BAD_REQUEST,
                ErrorBody { error: "validation".into(), message: e.to_string(), field },
            )
        }
        ControlPlaneError::Tls(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::Internal(msg) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: msg, field: None },
        ),
    }
}

impl IntoResponse for ControlPlaneError {
    fn into_response(self) -> Response {
        let (status, body) = to_response(self);
        (status, Json(body)).into_response()
    }
}
