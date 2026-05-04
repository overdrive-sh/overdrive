//! `ControlPlaneError` — top-level typed error with pass-through `#[from]`.
//!
//! Per ADR-0015, one top-level enum. Exhaustive `to_response` function
//! maps every variant to `(StatusCode, Json<ErrorBody>)`. Body shape is
//! a deliberate RFC 7807-compatible subset so v1.1 upgrade is additive.

use std::fmt;
use std::path::PathBuf;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use thiserror::Error;

use overdrive_core::reconciler::ReconcilerName;

use crate::api::ErrorBody;
use crate::view_store::{ProbeError, ViewStoreError};

/// Boot-time failures from the runtime-owned `ViewStore` (ADR-0035 §4).
///
/// Distinct typed variant per failure mode so the composition root in
/// `overdrive-cli::commands::serve` can branch on `matches!(...)`
/// without `Display`-grepping a stringified `Internal` message. The
/// boot path emits `health.startup.refused` when this variant fires;
/// see `ControlPlaneError::ViewStoreBoot`.
///
/// Pass-through embedding via `#[source]` per
/// `.claude/rules/development.md` § Errors — preserves the structured
/// `ViewStoreError` / `ProbeError` chain through audit logs and the
/// §12 investigation agent instead of stringifying it.
#[derive(Debug, Error)]
pub enum ViewStoreBootError {
    /// `RedbViewStore::open` failed at the production boot path.
    /// Typical causes: missing parent directory create, redb file
    /// corruption, concurrent open in the same process.
    #[error("open RedbViewStore at {path}: {source}")]
    Open {
        /// The resolved redb file path the open targeted.
        path: PathBuf,
        /// Underlying `ViewStoreError` cause.
        #[source]
        source: ViewStoreError,
    },

    /// Earned-Trust startup probe failed during `register`. The
    /// composition root short-circuits boot with `health.startup.refused`
    /// before any reconciler enters the registry.
    #[error("probe failed for reconciler {reconciler}: {source}")]
    Probe {
        /// Name of the reconciler whose `register` call surfaced the
        /// probe failure. Probe is per-call (not per-runtime), so the
        /// failing reconciler is the one that triggered the probe.
        reconciler: ReconcilerName,
        /// Underlying `ProbeError` cause.
        #[source]
        source: ProbeError,
    },

    /// `bulk_load` round-trip failed during `register` (CBOR decode
    /// error or underlying I/O failure). Hard boot failure — the
    /// composition root refuses to come up.
    #[error("bulk_load failed for reconciler {reconciler}: {source}")]
    BulkLoad {
        /// Name of the reconciler whose `register` call attempted the
        /// `bulk_load` round-trip.
        reconciler: ReconcilerName,
        /// Underlying `ViewStoreError` cause.
        #[source]
        source: ViewStoreError,
    },
}

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

    /// Pre-flight cgroup v2 delegation refusal per ADR-0028.
    /// Surfaced from the boot-path pre-flight as `From` conversion;
    /// rendered to the operator via `Display` (multi-line "what / why /
    /// how to fix" shape per nw-ux-tui-patterns) and never reaches an
    /// HTTP response — the listener doesn't bind on this error.
    #[error(transparent)]
    Cgroup(#[from] crate::cgroup_preflight::CgroupPreflightError),

    /// `ViewStore` boot-time failure per ADR-0035 §5 (Earned Trust).
    /// Pass-through embedding so `overdrive-cli::commands::serve` can
    /// `matches!(e, ControlPlaneError::ViewStoreBoot(_))` to emit the
    /// `health.startup.refused` event without `Display`-grepping a
    /// stringified message. Maps to `500 Internal` on the wire — boot
    /// failures never reach an HTTP response in practice (the listener
    /// has not bound yet); the arm exists for enum exhaustiveness.
    #[error(transparent)]
    ViewStoreBoot(#[from] ViewStoreBootError),

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
        ControlPlaneError::Cgroup(e) => (
            // Pre-flight refusal happens BEFORE any listener binds; this
            // arm exists for completeness so the enum match stays
            // exhaustive. In practice a Cgroup error never reaches an
            // HTTP response — the operator sees it on stderr at boot.
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody { error: "internal".into(), message: e.to_string(), field: None },
        ),
        ControlPlaneError::ViewStoreBoot(e) => (
            // Same shape as `Cgroup` above: ViewStore boot failures
            // happen BEFORE the listener binds, so this arm is
            // exhaustiveness-only. The composition root branches on
            // the typed variant (`matches!(e, ViewStoreBoot(_))`) to
            // emit `health.startup.refused`.
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
