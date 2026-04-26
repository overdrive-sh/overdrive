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
/// Carries the canonical `spec_digest` (byte-identical to a local
/// `ContentHash::of(rkyv::to_bytes(&Job::from_spec(parsed)))` compute
/// — that's the walking-skeleton guarantee), the number of live
/// allocations for the job, and an operator-facing empty-state message
/// referencing `phase-1-first-workload` when `allocations_total == 0`.
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
    /// `job_id` matches [`Self::job_id`]. Phase 1 is always zero — the
    /// scheduler + driver land in `phase-1-first-workload`.
    pub allocations_total: usize,
    /// Operator-facing empty-state message rendered when
    /// `allocations_total == 0`. Carries a `phase-1-first-workload`
    /// reference so the operator has a pointer to the onboarding step
    /// without consulting docs. Empty string when allocations exist.
    pub empty_state_message: String,
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

    // 1. Establish the job exists (and pull the authoritative
    //    spec_digest). Unknown job → HttpStatus 404.
    let description = client.describe_job(&args.job).await?;

    // 2. Count the allocations for this job. Phase 1 reads an empty
    //    observation store; the count is always zero, but reading it
    //    IS the walking-skeleton proof that the observation path
    //    round-trips (observation rows ship in `phase-1-first-workload`).
    let allocs = client.alloc_status().await?;
    let allocations_total = allocs.rows.iter().filter(|r| r.job_id == args.job).count();

    let empty_state_message = if allocations_total == 0 {
        format!(
            "0 allocations for job {job} — the scheduler + driver land in \
             phase-1-first-workload",
            job = args.job,
        )
    } else {
        String::new()
    };

    Ok(AllocStatusOutput {
        job_id: args.job,
        spec_digest: description.spec_digest,
        allocations_total,
        empty_state_message,
    })
}
