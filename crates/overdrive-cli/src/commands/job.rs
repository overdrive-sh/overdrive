//! `overdrive job submit`.
//!
//! Reads a TOML job spec from disk, runs `Job::from_spec` locally for
//! fast-fail validation, POSTs the typed `SubmitJobRequest` to the
//! control plane, and returns a typed [`SubmitOutput`] carrying the
//! `job_id`, derived `intent_key`, canonical `spec_digest`, idempotency
//! `outcome`, endpoint, and operator next-command hint.
//!
//! Per ADR-0020 (drop `commit_index` from Phase 1) the wire shape is
//! `{job_id, spec_digest, outcome}` — the Raft commit-index field was
//! dropped. `spec_digest` is the lowercase-hex SHA-256 of the canonical
//! rkyv-archived `Job` bytes (ADR-0002), 64 characters; `outcome` is
//! `IdempotencyOutcome::{Inserted, Unchanged}`.
//!
//! Per ADR-0011, `Job::from_spec` is THE validating constructor. The
//! CLI runs it client-side for an immediate, operator-facing error
//! that names the offending field without a server round-trip; the
//! server runs it again on ingress for defence-in-depth (ADR-0015).
//!
//! Per `crates/overdrive-cli/CLAUDE.md` the handler is a plain
//! `async fn` that tests call directly — no subprocess, no `println!`.
//! Rendering lives in `crate::render::job_submit_accepted`.

use std::path::PathBuf;

use overdrive_control_plane::api::{IdempotencyOutcome, SubmitJobRequest};
use overdrive_core::aggregate::{AggregateError, IntentKey, Job, JobSpecInput};
use overdrive_core::id::JobId;
use url::Url;

use crate::http_client::{ApiClient, CliError};

/// Arguments to [`submit`].
///
/// `spec` is the path to a TOML file containing a `JobSpecInput`-shaped
/// document; `config_path` locates the operator trust triple, which is
/// the sole source of the control-plane endpoint per whitepaper §8.
#[derive(Debug, Clone)]
pub struct SubmitArgs {
    /// Path to the TOML job spec on disk.
    pub spec: PathBuf,
    /// Path to the Talos-shape trust triple on disk. The endpoint
    /// recorded in the triple is where the POST is issued.
    pub config_path: PathBuf,
}

/// Typed output of a successful `job submit`.
///
/// Carries the server's assigned `job_id`, the derived `intent_key`
/// (`jobs/<id>`), the canonical `spec_digest`, the idempotency
/// `outcome`, the endpoint actually `POST`ed to, and the operator
/// next-command hint.
///
/// Per ADR-0020 the Raft `commit_index` field is dropped — it was an
/// in-memory `u64` and never a substitute for an authoritative
/// observability surface.
///
/// Handlers never render output themselves; the binary wrapper passes
/// this value to [`crate::render::job_submit_accepted`].
#[derive(Debug, Clone)]
pub struct SubmitOutput {
    /// Job ID echoed by the server — matches the `id` field of the
    /// input spec after validation.
    pub job_id: String,
    /// Derived intent-store key — `jobs/<job_id>` per ADR-0011 §`IntentKey`.
    pub intent_key: String,
    /// Lowercase-hex SHA-256 of the canonical rkyv-archived `Job`
    /// bytes (ADR-0002, development.md §Hashing); 64 characters.
    /// Stable across byte-identical resubmissions.
    pub spec_digest: String,
    /// Idempotency outcome echoed by the control plane. `Inserted` on
    /// fresh submission, `Unchanged` on a byte-identical resubmission
    /// at the same intent key per ADR-0015 §4 (amended by ADR-0020).
    pub outcome: IdempotencyOutcome,
    /// Endpoint the POST was issued to, echoed for operator clarity.
    pub endpoint: Url,
    /// Next-command hint the operator can run to inspect allocation
    /// status — `overdrive alloc status --job <job_id>`.
    pub next_command: String,
}

/// Submit a job spec to the control plane.
///
/// # Errors
///
/// * [`CliError::InvalidSpec`] — the TOML file is unreadable,
///   malformed, or fails `Job::from_spec` (zero replicas, zero memory,
///   unparseable ID). Fires BEFORE any HTTP call.
/// * [`CliError::ConfigLoad`] — the trust triple cannot be loaded.
/// * [`CliError::Transport`] — the control plane is unreachable.
/// * [`CliError::HttpStatus`] — the server returned 4xx / 5xx.
/// * [`CliError::BodyDecode`] — the 2xx response body failed to parse.
pub async fn submit(args: SubmitArgs) -> Result<SubmitOutput, CliError> {
    // 1. Read TOML from disk. Missing / unreadable files map to
    //    InvalidSpec with field="spec" so the operator can fix the path.
    let toml_str = std::fs::read_to_string(&args.spec).map_err(|e| CliError::InvalidSpec {
        field: "spec".to_string(),
        message: format!("failed to read `{}`: {e}", args.spec.display()),
    })?;

    // 2. Parse TOML into the shared wire shape. Parse failures map to
    //    InvalidSpec with field="toml" so the operator sees the parser
    //    diagnostic without a cryptic stack trace.
    let spec_input: JobSpecInput =
        toml::from_str(&toml_str).map_err(|e| CliError::InvalidSpec {
            field: "toml".to_string(),
            message: format!("failed to parse TOML: {e}"),
        })?;

    // 3. Client-side validation via the shared ADR-0011 constructor.
    //    Fast-fail BEFORE any HTTP call — operators see the offending
    //    field without a round-trip.
    let _validated: Job = Job::from_spec(spec_input.clone()).map_err(aggregate_to_cli_error)?;

    // 4. Build the typed API client and POST. The endpoint is the one
    //    recorded in the trust triple — the operator config is the
    //    sole source.
    let client = ApiClient::from_config(&args.config_path)?;
    let endpoint = client.base_url().clone();
    let resp = client.submit_job(SubmitJobRequest { spec: spec_input }).await?;

    // 5. Compose the typed output. Intent key is derived via the
    //    shared `IntentKey::for_job` helper (ADR-0011 SSOT) — no
    //    drift-prone second `jobs/` literal in this crate.
    let job_id = parse_response_job_id(&resp.job_id)?;
    let intent_key = IntentKey::for_job(&job_id).as_str().to_string();
    let next_command = format!("overdrive alloc status --job {}", resp.job_id);
    Ok(SubmitOutput {
        job_id: resp.job_id,
        intent_key,
        spec_digest: resp.spec_digest,
        outcome: resp.outcome,
        endpoint,
        next_command,
    })
}

/// Parse a `job_id` string echoed back in a successful 2xx control-plane
/// response into a typed [`JobId`].
///
/// On `JobId::new` failure, the call site at [`submit`] is *post-HTTP*:
/// the server returned a 200 OK whose `job_id` field cannot be parsed by
/// the same validating constructor the spec went through. Per the
/// rustdoc on [`CliError::InvalidSpec`] (client-side spec validation
/// BEFORE any HTTP call) and [`CliError::BodyDecode`] (a successful 2xx
/// response whose body failed to deserialise into the expected typed
/// shape — server-side contract violation), this is a `BodyDecode`
/// shape, not an `InvalidSpec` shape.
pub fn parse_response_job_id(raw: &str) -> Result<JobId, CliError> {
    JobId::new(raw).map_err(|e| CliError::BodyDecode {
        cause: format!("server returned invalid job_id `{raw}`: {e}"),
    })
}

/// Map [`AggregateError`] (from `overdrive_core`) into a
/// [`CliError::InvalidSpec`] with the offending field name and a
/// human-readable reason. Separate from the HTTP-lane 400 mapping —
/// this is strictly client-side, pre-HTTP.
fn aggregate_to_cli_error(err: AggregateError) -> CliError {
    match err {
        AggregateError::Validation { field, message } => {
            CliError::InvalidSpec { field: field.to_string(), message }
        }
        AggregateError::Id(id_err) => {
            CliError::InvalidSpec { field: "id".to_string(), message: id_err.to_string() }
        }
        AggregateError::Resources(msg) => {
            CliError::InvalidSpec { field: "resources".to_string(), message: msg }
        }
    }
}
