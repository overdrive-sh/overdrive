//! `overdrive alloc status --job <id>`.
//!
//! Reads the canonical `spec_digest` from the control plane's
//! `JobDescription`, counts the allocations reported by
//! `GET /v1/allocs`, and returns a typed [`AllocStatusOutput`] with an
//! explicit empty-state message pointing at the
//! `phase-1-first-workload` onboarding step.
//!
//! Per ADR-0020 (drop `commit_index` from Phase 1) the wire shape of
//! `JobDescription` is `{spec, spec_digest}` — the Raft commit-index
//! field was dropped.
//!
//! Per ADR-0002 + handler contract (`describe_job`): `spec_digest` is
//! SHA-256 of the exact rkyv bytes the server wrote to the
//! `IntentStore`. The CLI treats it as an opaque hex string and echoes
//! it verbatim; any CLI-side recomputation would drift from the
//! server-authoritative hash. The walking-skeleton gate test locally
//! reproduces the digest and asserts byte-identity — byte-identity is
//! the load-bearing guarantee Phase 1 depends on (the `allocations`
//! subsystem in `phase-1-first-workload` reads the same digest to
//! decide whether to trigger a driver reconcile).
//!
//! Per `crates/overdrive-cli/CLAUDE.md` the handler is a plain
//! `async fn` that tests call directly — no subprocess, no `println!`.
//! Rendering lives in `crate::render::alloc_status`.

use std::path::PathBuf;

use overdrive_control_plane::api::AllocStatusResponse;

use crate::http_client::{ApiClient, CliError};

/// Arguments to [`status`].
///
/// `job` is the canonical job id; `config_path` locates the operator
/// trust triple, which is the sole source of the control-plane
/// endpoint per whitepaper §8.
#[derive(Debug, Clone)]
pub struct StatusArgs {
    /// Canonical `JobId` to describe.
    pub job: String,
    /// Path to the Talos-shape trust triple on disk. The endpoint
    /// recorded in the triple is where the GETs are issued.
    pub config_path: PathBuf,
}

/// Typed output of a successful `alloc status`.
///
/// Slice 01 step 01-03 — the handler now returns the full
/// [`AllocStatusResponse`] envelope so the renderer can produce the
/// journey TUI mockup (per ADR-0033 §4 amended 2026-04-30). Legacy
/// fields (`job_id`, `spec_digest`, `allocations_total`,
/// `empty_state_message`) are derived from the envelope at construction
/// time so existing renderers (`render::alloc_status`) keep working.
///
/// Per ADR-0020 the Raft `commit_index` field is dropped.
#[derive(Debug, Clone)]
pub struct AllocStatusOutput {
    /// Canonical job id as echoed by the control plane.
    pub job_id: String,
    /// SHA-256 (hex) of the archived rkyv bytes of the validated `Job`,
    /// per ADR-0002. Opaque to the CLI — the CLI never recomputes this
    /// client-side, because a second canonicalisation would drift.
    pub spec_digest: String,
    /// Number of allocation rows in the observation store whose
    /// `job_id` matches [`Self::job_id`].
    pub allocations_total: usize,
    /// Operator-facing empty-state message rendered when
    /// `allocations_total == 0`. Carries a `phase-1-first-workload`
    /// reference so the operator has a pointer to the onboarding step
    /// without consulting docs. Empty string when allocations exist.
    pub empty_state_message: String,
    /// Full envelope from the server — slice 01 step 01-03 lets the
    /// renderer surface restart budget, last transition, cause-class
    /// reason text per ADR-0033 §4.
    pub snapshot: AllocStatusResponse,
}

/// Read the canonical `JobDescription` for `args.job` + the allocation
/// count from the observation store.
///
/// Returns `Err(CliError::HttpStatus { status: 404, .. })` for unknown
/// jobs, carrying an actionable `ErrorBody.message` that names the
/// offending job id.
///
/// # Errors
///
/// * [`CliError::ConfigLoad`] — trust triple cannot be loaded.
/// * [`CliError::Transport`] — control plane unreachable.
/// * [`CliError::HttpStatus`] — `GET /v1/jobs/<id>` returned 4xx/5xx.
///   The 404 path (unknown job) is the load-bearing operator-facing
///   error and carries `body.error = "not_found"`.
/// * [`CliError::BodyDecode`] — the server returned a 2xx with a
///   malformed body.
pub async fn status(args: StatusArgs) -> Result<AllocStatusOutput, CliError> {
    let client = ApiClient::from_config(&args.config_path)?;

    // Slice 01 step 01-03 — single round-trip through the snapshot
    // surface. The handler reads IntentStore + Observation rows +
    // JobLifecycle view-cache and returns the full envelope; 404 on
    // missing job carries `body.error == "not_found"`.
    let snapshot = client.alloc_status_for_job(&args.job).await?;

    let allocations_total = snapshot.rows.len();
    let empty_state_message = if allocations_total == 0 {
        format!(
            "0 allocations for job {job} — the scheduler + driver land in \
             phase-1-first-workload",
            job = args.job,
        )
    } else {
        String::new()
    };

    let spec_digest = snapshot.spec_digest.clone().unwrap_or_default();

    Ok(AllocStatusOutput {
        job_id: args.job,
        spec_digest,
        allocations_total,
        empty_state_message,
        snapshot,
    })
}

/// Return the raw [`AllocStatusResponse`] envelope.
///
/// Variant of [`status`] used by tests (notably S-WS-02) that need to
/// assert on the cause-class typed payload byte-equality across
/// streaming + snapshot surfaces. Bypasses the operator-facing
/// derivation step (`empty_state_message`, etc.) so the assertion target
/// is the raw wire shape.
///
/// # Errors
///
/// Same shapes as [`status`].
pub async fn status_snapshot(args: StatusArgs) -> Result<AllocStatusResponse, CliError> {
    let client = ApiClient::from_config(&args.config_path)?;
    client.alloc_status_for_job(&args.job).await
}
