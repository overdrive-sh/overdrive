//! Hand-rolled reqwest client for the Phase 1 control-plane REST API.
//!
//! Per ADR-0014, the CLI reuses the request/response types from
//! `overdrive_control_plane::api` verbatim — there are no shadow types.
//! The client is a thin typed wrapper around `reqwest::Client`:
//!
//! * `ApiClient::from_config(path)` loads the Talos-shape trust triple
//!   from disk (ADR-0010), pins the minted CA as the sole root of
//!   trust, and attaches the client leaf as a PEM identity. Per ADR-0010
//!   Phase 1 the server does not yet validate the client cert; the cert
//!   is attached anyway so Phase 5 mTLS flips a single switch on the
//!   server side.
//! * Five endpoint methods map 1-1 onto ADR-0008's endpoint table and
//!   return typed responses from `overdrive_control_plane::api`.
//! * `CliError` is a small typed enum (`ConfigLoad`, `Transport`,
//!   `HttpStatus`, `BodyDecode`) whose `Display` emits actionable,
//!   operator-readable messages — no raw `reqwest::Error` Debug
//!   format, no low-level transport tokens like `ECONNREFUSED`.

use std::path::Path;

use overdrive_control_plane::api::{
    AllocStatusResponse, ClusterStatus, ErrorBody, JobDescription, NodeList, SubmitJobRequest,
    SubmitJobResponse,
};
use overdrive_control_plane::tls_bootstrap::{TrustTriple, load_trust_triple};
use reqwest::StatusCode;
use thiserror::Error;
use url::Url;

/// Typed errors returned by `ApiClient`. Each variant carries enough
/// structured context for a CLI handler to render an actionable
/// operator-facing message without leaking transport internals.
///
/// Per ADR-0014, the binary boundary (`main.rs`) converts this into an
/// `eyre::Report` at the exit path; callers that need to branch on
/// failure mode (retry, rewrite, abort) match on the variant.
#[derive(Debug, Error)]
pub enum CliError {
    /// Loading / parsing the `~/.overdrive/config` trust triple failed.
    /// The `path` field names the file so the operator can repair it.
    /// `cause` is a short human-readable summary — it is deliberately
    /// NOT a nested `source` error, because the real cause
    /// (`base64::DecodeError`, `serde_yaml::Error`) carries Debug
    /// output that leaks implementation details.
    #[error("failed to load overdrive config from {path}: {cause}")]
    ConfigLoad { path: String, cause: String },

    /// A transport-level failure reaching the control plane (connection
    /// refused, TLS handshake failure, DNS failure, request timeout).
    /// `endpoint` names the URL so the operator can verify the server
    /// is reachable; `cause` is a short, stripped summary with no raw
    /// `reqwest::Error` Debug tokens.
    #[error(
        "failed to reach overdrive control plane at {endpoint}: {cause}\n\
         hint: check that the server is running and the endpoint is correct"
    )]
    Transport { endpoint: String, cause: String },

    /// The server returned a non-2xx response. `body` is the typed
    /// `ErrorBody` per ADR-0015 (`error`, `message`, `field`).
    #[error("control plane returned HTTP {status}: {} ({})", body.error, body.message)]
    HttpStatus {
        /// Numeric HTTP status code (e.g. `400`, `409`, `500`).
        status: u16,
        /// Typed error body per ADR-0015.
        body: ErrorBody,
    },

    /// A successful 2xx response whose body failed to deserialise into
    /// the expected typed shape. This is a server-side contract
    /// violation and deserves its own variant so the CLI can escalate
    /// instead of papering over.
    #[error("failed to decode response body from control plane: {cause}")]
    BodyDecode { cause: String },

    /// Client-side spec validation failed before any HTTP call. Per
    /// ADR-0011 the CLI runs `Job::from_spec` (the same validating
    /// constructor the server uses) locally so operators see the
    /// offending field without a round-trip. `field` names the
    /// offending field in the aggregate's public shape; `message` is
    /// the human-readable reason. Separate variant from
    /// `CliError::HttpStatus { error = "validation", .. }` — the
    /// client-side path never reaches the server.
    #[error("invalid job spec: field `{field}`: {message}")]
    InvalidSpec { field: String, message: String },
}

/// Hand-rolled typed REST client for the Phase 1 control-plane. One
/// method per ADR-0008 endpoint; shared request/response types come
/// from `overdrive_control_plane::api`.
#[derive(Debug, Clone)]
pub struct ApiClient {
    inner: reqwest::Client,
    base: Url,
}

impl ApiClient {
    /// Load the trust triple from `path` (typically
    /// `~/.overdrive/config`) and build a reqwest client that pins the
    /// minted CA and attaches the client leaf identity. The endpoint
    /// is read from the first context's `endpoint` field.
    ///
    /// # Errors
    ///
    /// Returns [`CliError::ConfigLoad`] if the file cannot be read,
    /// parsed, or decoded, or if the minted CA / client identity is
    /// rejected by rustls.
    pub fn from_config(path: &Path) -> Result<Self, CliError> {
        Self::from_config_with_endpoint(path, None)
    }

    /// Same as [`ApiClient::from_config`], but allows overriding the
    /// endpoint recorded in the trust triple. Used by integration tests
    /// that bind the server on an ephemeral port (the port in the
    /// config file is the static configured bind, not the resolved
    /// port), and by the CLI's `--endpoint` flag.
    ///
    /// # Errors
    ///
    /// See [`ApiClient::from_config`].
    pub fn from_config_with_endpoint(
        path: &Path,
        endpoint_override: Option<&str>,
    ) -> Result<Self, CliError> {
        let triple = load_trust_triple(path).map_err(|e| CliError::ConfigLoad {
            path: path.display().to_string(),
            cause: strip_leak(&e.to_string()),
        })?;

        let endpoint_str = endpoint_override.unwrap_or_else(|| triple.endpoint());
        let base = Url::parse(endpoint_str).map_err(|e| CliError::ConfigLoad {
            path: path.display().to_string(),
            cause: format!("invalid endpoint URL `{endpoint_str}`: {e}"),
        })?;

        let inner = build_reqwest_client(&triple)
            .map_err(|cause| CliError::ConfigLoad { path: path.display().to_string(), cause })?;

        Ok(Self { inner, base })
    }

    /// The base URL this client posts to — useful for diagnostics and
    /// for tests asserting the endpoint was honoured.
    #[must_use]
    pub const fn base_url(&self) -> &Url {
        &self.base
    }

    /// `POST /v1/jobs` — submit a job spec to the control plane.
    ///
    /// # Errors
    ///
    /// See [`CliError`] variants.
    pub async fn submit_job(&self, req: SubmitJobRequest) -> Result<SubmitJobResponse, CliError> {
        self.post_typed("v1/jobs", &req).await
    }

    /// `GET /v1/jobs/{id}` — describe a previously-submitted job.
    ///
    /// # Errors
    ///
    /// See [`CliError`] variants.
    pub async fn describe_job(&self, id: &str) -> Result<JobDescription, CliError> {
        self.get_typed(&format!("v1/jobs/{id}")).await
    }

    /// `GET /v1/cluster/info` — read control-plane mode, region,
    /// commit index, reconciler registry, and broker counters.
    ///
    /// # Errors
    ///
    /// See [`CliError`] variants.
    pub async fn cluster_status(&self) -> Result<ClusterStatus, CliError> {
        self.get_typed("v1/cluster/info").await
    }

    /// `GET /v1/allocs` — read allocation-status rows from the
    /// observation store.
    ///
    /// # Errors
    ///
    /// See [`CliError`] variants.
    pub async fn alloc_status(&self) -> Result<AllocStatusResponse, CliError> {
        self.get_typed("v1/allocs").await
    }

    /// `GET /v1/nodes` — read node-health rows from the observation
    /// store.
    ///
    /// # Errors
    ///
    /// See [`CliError`] variants.
    pub async fn node_list(&self) -> Result<NodeList, CliError> {
        self.get_typed("v1/nodes").await
    }

    // ---- Internals ----

    /// Execute `GET <path>` and decode the body as `T`. Wraps the three
    /// steps every GET endpoint repeats — URL build, transport send,
    /// typed decode — so the public endpoint methods stay one-liners.
    async fn get_typed<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, CliError> {
        let url = self.build_url(path)?;
        let resp = self.inner.get(url).send().await.map_err(|e| self.transport_err(&e))?;
        self.decode_typed(resp).await
    }

    /// Execute `POST <path>` with a JSON body and decode the response
    /// as `T`. Counterpart to [`get_typed`] for mutating endpoints.
    ///
    /// [`get_typed`]: ApiClient::get_typed
    async fn post_typed<B, T>(&self, path: &str, body: &B) -> Result<T, CliError>
    where
        B: serde::Serialize + Sync,
        T: serde::de::DeserializeOwned,
    {
        let url = self.build_url(path)?;
        let resp =
            self.inner.post(url).json(body).send().await.map_err(|e| self.transport_err(&e))?;
        self.decode_typed(resp).await
    }

    /// Join a path onto the base URL, surfacing parse failures as
    /// [`CliError::ConfigLoad`] — an invalid URL here means the
    /// recorded endpoint is malformed.
    fn build_url(&self, path: &str) -> Result<Url, CliError> {
        self.base.join(path).map_err(|e| CliError::Transport {
            endpoint: self.base.to_string(),
            cause: format!("invalid URL path `{path}`: {e}"),
        })
    }

    /// Map a reqwest transport error to [`CliError::Transport`] with a
    /// stripped `cause` string. Never returns raw `reqwest::Error`
    /// Debug output or kernel error tokens (`ECONNREFUSED`).
    fn transport_err(&self, err: &reqwest::Error) -> CliError {
        CliError::Transport { endpoint: self.base.to_string(), cause: stringify_reqwest_error(err) }
    }

    /// Decode a reqwest response into a typed 2xx body or a typed
    /// [`CliError::HttpStatus`] on 4xx/5xx. 2xx bodies that fail to
    /// parse become [`CliError::BodyDecode`].
    async fn decode_typed<T: serde::de::DeserializeOwned>(
        &self,
        resp: reqwest::Response,
    ) -> Result<T, CliError> {
        let status = resp.status();
        if status.is_success() {
            return resp
                .json::<T>()
                .await
                .map_err(|e| CliError::BodyDecode { cause: stringify_reqwest_error(&e) });
        }

        // 4xx / 5xx — decode ErrorBody per ADR-0015. If the body is
        // not a valid ErrorBody we fall back to a synthesised ErrorBody
        // so the caller still sees a well-typed variant.
        let status_u16 = status.as_u16();
        let body = resp.json::<ErrorBody>().await.unwrap_or_else(|_| synthesize_error_body(status));
        Err(CliError::HttpStatus { status: status_u16, body })
    }
}

/// Build the reqwest client from a loaded trust triple, returning a
/// stripped cause string on failure so callers can wrap it as
/// [`CliError::ConfigLoad`].
fn build_reqwest_client(triple: &TrustTriple) -> Result<reqwest::Client, String> {
    let ca = reqwest::Certificate::from_pem(triple.ca_cert_pem())
        .map_err(|e| format!("failed to parse CA certificate: {e}"))?;

    // reqwest::Identity::from_pem accepts a concatenated cert+key PEM
    // blob. Glue them together with a newline so the separator is
    // explicit regardless of whether each blob carries a trailing
    // newline.
    let mut identity_pem =
        Vec::with_capacity(triple.client_cert_pem().len() + triple.client_key_pem().len() + 1);
    identity_pem.extend_from_slice(triple.client_cert_pem());
    if !identity_pem.ends_with(b"\n") {
        identity_pem.push(b'\n');
    }
    identity_pem.extend_from_slice(triple.client_key_pem());
    let identity = reqwest::Identity::from_pem(&identity_pem)
        .map_err(|e| format!("failed to parse client identity: {e}"))?;

    reqwest::Client::builder()
        .add_root_certificate(ca)
        .identity(identity)
        .https_only(true)
        .use_rustls_tls()
        .build()
        .map_err(|e| format!("failed to build HTTPS client: {e}"))
}

/// Synthesise a plausible [`ErrorBody`] for a non-2xx response whose
/// body is absent or not a valid `ErrorBody`. Ensures `HttpStatus`
/// always carries a well-typed variant.
fn synthesize_error_body(status: StatusCode) -> ErrorBody {
    ErrorBody {
        error: "unknown".to_owned(),
        message: format!("control plane returned HTTP {status} with no typed body"),
        field: None,
    }
}

/// Render a `reqwest::Error` as a short, operator-readable string that
/// does NOT leak raw tokens like `ECONNREFUSED`, `reqwest::Error`
/// Debug format, or deeply-chained source contexts. The shape is one
/// adjective per failure category — enough context for the operator
/// to act, not enough for an attacker to fingerprint.
fn stringify_reqwest_error(err: &reqwest::Error) -> String {
    let category = if err.is_timeout() {
        "request timed out"
    } else if err.is_connect() {
        "could not connect to server"
    } else if err.is_decode() {
        "response body decode failed"
    } else if err.is_request() {
        "request build failed"
    } else if err.is_body() {
        "request body error"
    } else if err.is_redirect() {
        "too many redirects"
    } else if err.is_status() {
        "unexpected status"
    } else {
        "transport error"
    };
    strip_leak(category)
}

/// Scrub low-level tokens that reveal transport / decoder internals
/// from a message before it reaches the operator. Applied to any
/// message that flows into `CliError` Display output.
fn strip_leak(s: &str) -> String {
    s.replace("ECONNREFUSED", "connection refused")
        .replace("reqwest::Error", "transport error")
        .replace("DecodeError", "decode error")
}
