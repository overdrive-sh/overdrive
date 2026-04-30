//! Hand-rolled reqwest client for the Phase 1 control-plane REST API.
//!
//! Per ADR-0014, the CLI reuses the request/response types from
//! `overdrive_control_plane::api` verbatim — there are no shadow types.
//! The client is a thin typed wrapper around `reqwest::Client`:
//!
//! * `ApiClient::from_config(path)` loads the trust triple from disk
//!   (ADR-0010 shape, ADR-0019 TOML syntax), pins the minted CA as the
//!   sole root of trust, and attaches the client leaf as a PEM
//!   identity. Per ADR-0010 Phase 1 the server does not yet validate
//!   the client cert; the cert is attached anyway so Phase 5 mTLS
//!   flips a single switch on the server side.
//! * Five endpoint methods map 1-1 onto ADR-0008's endpoint table and
//!   return typed responses from `overdrive_control_plane::api`.
//! * `CliError` is a small typed enum (`ConfigLoad`, `Transport`,
//!   `HttpStatus`, `BodyDecode`) whose `Display` emits actionable,
//!   operator-readable messages — no raw `reqwest::Error` Debug
//!   format, no low-level transport tokens like `ECONNREFUSED`.

use std::path::Path;
use std::time::Duration;

use overdrive_control_plane::api::{
    AllocStatusResponse, ClusterStatus, ErrorBody, JobDescription, NodeList, StopJobResponse,
    SubmitJobRequest, SubmitJobResponse,
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
    /// (`base64::DecodeError`, `toml::de::Error`) carries Debug output
    /// that leaks implementation details.
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
    /// is read from the first context's `endpoint` field — the operator
    /// config is the sole source of the control-plane endpoint per
    /// whitepaper §8 (*Operator Identity and CLI Authentication*).
    ///
    /// # Errors
    ///
    /// Returns [`CliError::ConfigLoad`] if the file cannot be read,
    /// parsed, or decoded, or if the minted CA / client identity is
    /// rejected by rustls, or if the recorded endpoint URL is
    /// malformed.
    pub fn from_config(path: &Path) -> Result<Self, CliError> {
        let triple = load_trust_triple(path).map_err(|e| CliError::ConfigLoad {
            path: path.display().to_string(),
            cause: strip_leak(&e.to_string()),
        })?;

        let endpoint_str = triple.endpoint();
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

    /// `POST /v1/jobs` with `Accept: application/x-ndjson` — drives the
    /// streaming-submit lane per ADR-0032 §3 / architecture.md §10.
    ///
    /// Returns the raw `reqwest::Response` so the caller can iterate
    /// `bytes_stream()` line-by-line without re-buffering. The response
    /// status is checked here BEFORE the body is consumed: 4xx / 5xx
    /// responses parse `ErrorBody` and return [`CliError::HttpStatus`];
    /// transport failures map to [`CliError::Transport`]. The success
    /// path leaves body parsing to the caller (one NDJSON line at a
    /// time).
    ///
    /// # Errors
    ///
    /// * [`CliError::Transport`] — control plane unreachable.
    /// * [`CliError::HttpStatus`] — server returned non-2xx.
    pub async fn submit_job_streaming(
        &self,
        req: SubmitJobRequest,
    ) -> Result<reqwest::Response, CliError> {
        let url = self.build_url("v1/jobs")?;
        let resp = self
            .inner
            .post(url)
            .header(reqwest::header::ACCEPT, "application/x-ndjson")
            .json(&req)
            .send()
            .await
            .map_err(|e| self.transport_err(&e))?;

        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }

        let status_u16 = status.as_u16();
        let body = resp.json::<ErrorBody>().await.unwrap_or_else(|_| synthesize_error_body(status));
        Err(CliError::HttpStatus { status: status_u16, body })
    }

    /// `POST /v1/jobs/{id}/stop` — record a stop intent for a
    /// previously-submitted job. Per ADR-0027.
    ///
    /// Empty request body. Returns `StopJobResponse` on 200 OK with
    /// `outcome ∈ { Stopped, AlreadyStopped }`. A 404 maps to
    /// [`CliError::HttpStatus`] with `body.error == "not_found"`.
    ///
    /// # Errors
    ///
    /// See [`CliError`] variants.
    pub async fn stop_job(&self, id: &str) -> Result<StopJobResponse, CliError> {
        self.post_typed(&format!("v1/jobs/{id}/stop"), &serde_json::json!({})).await
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

    /// `GET /v1/allocs?job=<id>` — full allocation snapshot for a
    /// specific job. Slice 01 step 01-03. 404 on unknown job carries
    /// `body.error == "not_found"` per ADR-0015.
    ///
    /// # Errors
    ///
    /// See [`CliError`] variants.
    pub async fn alloc_status_for_job(
        &self,
        job_id: &str,
    ) -> Result<AllocStatusResponse, CliError> {
        // URL-encode the job_id query parameter via the std url crate
        // (`Url::query_pairs_mut`), avoiding manual escaping.
        let mut url = self.build_url("v1/allocs")?;
        url.query_pairs_mut().append_pair("job", job_id);
        let resp = self.inner.get(url).send().await.map_err(|e| self.transport_err(&e))?;
        self.decode_typed(resp).await
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
    /// recorded endpoint in the loaded trust-triple config is
    /// malformed, not a transport-layer failure.
    fn build_url(&self, path: &str) -> Result<Url, CliError> {
        self.base.join(path).map_err(|e| CliError::ConfigLoad {
            path: self.base.to_string(),
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
        // Bounded connect + total-request deadlines so the CLI cannot
        // hang indefinitely on an unreachable or silent control plane.
        // Chosen to fit the Phase 1 single-node / localhost shape — the
        // operator sees a typed `CliError::Transport` with a "request
        // timed out" cause instead of a process that never returns.
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
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

/// Render a `reqwest::Error` as a short, operator-readable string.
/// Each arm returns a curated `&'static str` chosen to be both
/// operator-actionable and disjoint from the `strip_leak` deny-list
/// (`ECONNREFUSED`, `reqwest::Error`, `DecodeError`) — one adjective
/// per failure category, enough context for the operator to act, not
/// enough for an attacker to fingerprint. No source-chain text is
/// embedded, so `strip_leak` is *not* load-bearing on this path and
/// is deliberately not called: applying it here was historically a
/// no-op that gave a false sense of sanitization. The dynamic scrub
/// is reserved for call sites that interpolate `Display`-of-error
/// directly (see `from_config` and `CliError::ConfigLoad`). The
/// `category_strings_are_leak_free_by_construction` test below pins
/// the disjointness invariant — anyone adding a new arm that
/// embeds dynamic content (e.g. `format!("...: {err}")`) MUST design
/// a scrub strategy for it; the existing `strip_leak` token set is
/// not sufficient for arbitrary `reqwest`/`hyper`/`io` Display text.
///
/// The `is_connect()` arm splits further: if the source chain carries
/// a `rustls::Error::InvalidCertificate(...)`, the failure is a TLS
/// handshake fault (trust material mismatch) rather than a pure TCP
/// `ECONNREFUSED`. The two render distinctly so the operator's hint
/// points at the right remediation — re-run `overdrive serve` to
/// re-mint, vs check the server is running. Per
/// `fix-cli-cannot-reach-control-plane` Step 01-03 (RCA §Secondary
/// fix).
fn stringify_reqwest_error(err: &reqwest::Error) -> String {
    let category = if err.is_timeout() {
        "request timed out"
    } else if err.is_connect() {
        if has_rustls_cert_error(err) {
            "TLS handshake failed (certificate not trusted by the client) \
             — hint: re-run `overdrive serve` to re-mint trust material if the CA \
             was rotated since the operator config was written"
        } else {
            "could not connect to server"
        }
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
    category.to_owned()
}

/// Walk the `Error::source()` chain looking for any
/// `rustls::Error::InvalidCertificate(...)`. Returns `true` when a
/// rustls cert-verification error is present anywhere in the chain.
///
/// This is the discriminator for the `is_connect()` split in
/// `stringify_reqwest_error`: rustls' cert-verification failures
/// surface as `is_connect() == true` because the TCP side completed
/// before the handshake failed, but the cause is trust material, not
/// reachability. Per `fix-cli-cannot-reach-control-plane` Step 01-03.
///
/// We deliberately match ONLY `InvalidCertificate` rather than the
/// broader "any rustls error" — other rustls variants (`PeerMisbehaved`,
/// `AlertReceived`, etc.) describe protocol-level faults the operator
/// cannot fix by re-minting, and the catch-all 'could not connect to
/// server' message is a safer default for those.
fn has_rustls_cert_error(err: &reqwest::Error) -> bool {
    use std::error::Error as _;
    let mut source: Option<&dyn std::error::Error> = err.source();
    while let Some(cause) = source {
        // Direct downcast: rustls::Error appears in the chain when a
        // cert-verification failure was the immediate TLS fault.
        if let Some(rustls_err) = cause.downcast_ref::<rustls::Error>() {
            if matches!(rustls_err, rustls::Error::InvalidCertificate(_)) {
                return true;
            }
        }
        // Fallback: rustls' webpki/aws-lc-rs verifier errors are
        // sometimes wrapped (e.g. via std::io::Error or hyper-tls
        // adapters) such that the concrete `rustls::Error` type does
        // not survive the downcast. Match on Display text as a
        // last-resort discriminator. The exact text varies across
        // rustls versions but consistently mentions either
        // 'InvalidCertificate', 'invalid certificate', or
        // 'UnknownIssuer' / 'unknown issuer'.
        let display = cause.to_string();
        let display_lower = display.to_lowercase();
        if display_lower.contains("invalidcertificate")
            || display_lower.contains("invalid certificate")
            || display_lower.contains("unknownissuer")
            || display_lower.contains("unknown issuer")
            || display_lower.contains("certificate verify failed")
            || display_lower.contains("invalid peer certificate")
        {
            return true;
        }
        source = cause.source();
    }
    false
}

/// Scrub low-level tokens that reveal transport / decoder internals
/// from a message before it reaches the operator. Applied to any
/// message that flows into `CliError` Display output.
fn strip_leak(s: &str) -> String {
    s.replace("ECONNREFUSED", "connection refused")
        .replace("reqwest::Error", "transport error")
        .replace("DecodeError", "decode error")
}

#[cfg(test)]
mod tests {
    /// `stringify_reqwest_error` returns one of a small, hand-curated
    /// set of `&'static str` literals — never an interpolation of a
    /// `reqwest::Error` Display chain. The disjointness contract: each
    /// of those literals is leak-free against `strip_leak`'s deny-list
    /// by construction, which is why the function does not (and must
    /// not need to) call `strip_leak` on its output.
    ///
    /// This test pins the contract by enumerating the literal set
    /// inline and asserting each against the deny-list. The list MUST
    /// mirror the arms in `stringify_reqwest_error` — when a new arm
    /// is added, this test must be updated. If a future arm embeds
    /// dynamic content (e.g. `format!("transport error: {err}")`), the
    /// new string will not be a static literal and the maintainer must
    /// design a scrub strategy specific to that arm — `strip_leak`'s
    /// existing three-token set is not sufficient for arbitrary
    /// `reqwest` / `hyper` / `io` Display text.
    #[test]
    fn category_strings_are_leak_free_by_construction() {
        // Mirror of every literal `stringify_reqwest_error` can return.
        // Keep in sync with the function body above.
        let categories: &[&str] = &[
            "request timed out",
            "TLS handshake failed (certificate not trusted by the client) \
             — hint: re-run `overdrive serve` to re-mint trust material if the CA \
             was rotated since the operator config was written",
            "could not connect to server",
            "response body decode failed",
            "request build failed",
            "request body error",
            "too many redirects",
            "unexpected status",
            "transport error",
        ];

        // The deny-list `strip_leak` scrubs. The contract is that every
        // category string is disjoint from this set, so calling
        // `strip_leak` on the function's output would be a no-op.
        let leak_tokens: &[&str] = &["ECONNREFUSED", "reqwest::Error", "DecodeError"];

        for &category in categories {
            for &token in leak_tokens {
                assert!(
                    !category.contains(token),
                    "category string {category:?} contains leak token {token:?} — \
                     either the literal must change or this arm needs its own scrub \
                     (strip_leak's three-token set is not load-bearing on this path)",
                );
            }
        }
    }
}
