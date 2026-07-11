//! `overdrive workload restart <id>` / `overdrive workload describe <id>`
//! — the operator-facing workload-lifecycle namespace.
//!
//! New top-level `workload` namespace (NOT under `job`, #220-aligned)
//! carrying the operator-facing restart verb that drives the
//! `POST /v1/workloads/{id}/restart` route shipped by step 01-03, plus the
//! `describe` inspection verb (#220) that reads the canonical
//! `spec_digest` + live allocation rows via `GET /v1/allocs`. Per ADR-0073
//! § "The six pinned signatures" item 1 this is the DESIGN-table-recorded
//! home for `RestartArgs` / `RestartOutput` / `restart` — a separate
//! module from `deploy.rs` because these are distinct
//! workload-lifecycle concerns, not a deploy/submit concern.
//!
//! Per `crates/overdrive-cli/CLAUDE.md` the handlers are plain `async fn`
//! that integration tests call directly — no subprocess, no `println!`.
//! Rendering lives in `crate::render::workload_restart_accepted` /
//! `crate::render::workload_describe`.

use std::path::PathBuf;

use overdrive_control_plane::api::{AllocStatusResponse, ProbeResultRowJson, RestartOutcome};
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::WorkloadId;
use overdrive_core::observation::probe_result_row::ProbeResultRow;
use serde::Serialize;
use url::Url;

use crate::http_client::{ApiClient, CliError};

/// Arguments to [`restart`]. Mirrors `crate::commands::deploy::StopArgs`.
#[derive(Debug, Clone)]
pub struct RestartArgs {
    /// Canonical `WorkloadId` to restart. Validated client-side via
    /// `WorkloadId::new` before any HTTP call so operators see the
    /// offending byte without a round-trip.
    pub id: String,
    /// Path to the trust triple. Same conventions as
    /// `crate::commands::deploy::DeployArgs` — the recorded endpoint is
    /// where the POST is issued.
    pub config_path: PathBuf,
}

/// Typed output of `overdrive workload restart`.
///
/// Carries the server's echoed `workload_id`, the `outcome` (`Restarted`
/// vs `Resumed`), and the endpoint the POST was issued to. Mirrors
/// `crate::commands::deploy::StopOutput`.
#[derive(Debug, Clone)]
pub struct RestartOutput {
    /// Workload ID echoed by the server.
    pub workload_id: String,
    /// Restart outcome echoed by the control plane — `Restarted` when no
    /// live stop intent was on file, `Resumed` when an operator-stop
    /// sentinel was present at the check-exists read (ADR-0073 item 2).
    pub outcome: RestartOutcome,
    /// Endpoint the POST was issued to, echoed for operator clarity.
    pub endpoint: Url,
}

/// Replace a declared workload's backend instance with a fresh one by
/// driving `POST /v1/workloads/{id}/restart`.
///
/// Per ADR-0073: returns 200 OK with `outcome = Restarted` when no live
/// stop intent existed at the handler's check-exists read, and
/// `outcome = Resumed` when an operator-stop sentinel was present.
/// Returns 404 if the workload was never declared.
///
/// # Errors
///
/// * [`CliError::InvalidSpec`] — `id` does not parse as a canonical `WorkloadId`.
/// * [`CliError::ConfigLoad`] — trust triple unloadable.
/// * [`CliError::Transport`] — control plane unreachable.
/// * [`CliError::HttpStatus`] — server returned non-2xx (404 unknown,
///   with `body.error == "not_found"`).
/// * [`CliError::BodyDecode`] — 2xx body decode failed.
pub async fn restart(args: RestartArgs) -> Result<RestartOutput, CliError> {
    // Client-side validation — fail fast on malformed ids before any
    // HTTP call, same discipline as `commands::deploy::stop`.
    let _ = WorkloadId::new(&args.id)
        .map_err(|e| CliError::InvalidSpec { field: "id".to_string(), message: e.to_string() })?;

    let client = ApiClient::from_config(&args.config_path)?;
    let endpoint = client.base_url().clone();
    let resp = client.restart_workload(&args.id).await?;

    Ok(RestartOutput { workload_id: resp.workload_id, outcome: resp.outcome, endpoint })
}

// ---------------------------------------------------------------------------
// `overdrive workload describe <id>` — the operator inspection verb (#220).
//
// Reads the canonical `spec_digest` from the control plane's
// `WorkloadDescription`, counts the allocations reported by
// `GET /v1/allocs`, and returns a typed [`WorkloadDescribeOutput`] with an
// explicit empty-state message pointing at the `phase-1-first-workload`
// onboarding step.
//
// Per ADR-0020 (drop `commit_index` from Phase 1) the wire shape of
// `WorkloadDescription` is `{spec, spec_digest}` — the Raft commit-index
// field was dropped.
//
// Per ADR-0002 + handler contract (`describe_workload`): `spec_digest` is
// SHA-256 of the exact rkyv bytes the server wrote to the `IntentStore`.
// The CLI treats it as an opaque hex string and echoes it verbatim; any
// CLI-side recomputation would drift from the server-authoritative hash.
//
// The backing transport stays `GET /v1/allocs` via
// `ApiClient::alloc_status_for_workload` — rendering lives in
// `crate::render::workload_describe`.
// ---------------------------------------------------------------------------

/// Arguments to [`describe`].
///
/// `id` is the canonical workload id; `config_path` locates the operator
/// trust triple, which is the sole source of the control-plane
/// endpoint per whitepaper §8.
#[derive(Debug, Clone)]
pub struct DescribeArgs {
    /// Canonical `WorkloadId` to describe.
    pub id: String,
    /// Path to the Talos-shape trust triple on disk. The endpoint
    /// recorded in the triple is where the GETs are issued.
    pub config_path: PathBuf,
}

/// Typed output of a successful `overdrive workload describe`.
///
/// The handler returns the full [`AllocStatusResponse`] envelope so the
/// renderer can produce the kind-aware describe view (per ADR-0033 §4
/// amended 2026-04-30). Legacy fields (`workload_id`, `spec_digest`,
/// `allocations_total`, `empty_state_message`) are derived from the
/// envelope at construction time so the renderer (`render::workload_describe`)
/// keeps working.
///
/// Per ADR-0020 the Raft `commit_index` field is dropped.
#[derive(Debug, Clone)]
pub struct WorkloadDescribeOutput {
    /// Canonical workload id as echoed by the control plane.
    pub workload_id: String,
    /// SHA-256 (hex) of the archived rkyv bytes of the validated `Job`,
    /// per ADR-0002. Opaque to the CLI — the CLI never recomputes this
    /// client-side, because a second canonicalisation would drift.
    pub spec_digest: String,
    /// Number of allocation rows in the observation store whose
    /// `workload_id` matches [`Self::workload_id`].
    pub allocations_total: usize,
    /// Operator-facing empty-state message rendered when
    /// `allocations_total == 0`. Carries a `phase-1-first-workload`
    /// reference so the operator has a pointer to the onboarding step
    /// without consulting docs. Empty string when allocations exist.
    pub empty_state_message: String,
    /// Full envelope from the server — lets the renderer surface restart
    /// budget, last transition, cause-class reason text per ADR-0033 §4.
    pub snapshot: AllocStatusResponse,
}

/// Read the canonical `WorkloadDescription` for `args.id` + the allocation
/// count from the observation store.
///
/// Returns `Err(CliError::HttpStatus { status: 404, .. })` for unknown
/// workloads, carrying an actionable `ErrorBody.message` that names the
/// offending workload id.
///
/// # Errors
///
/// * [`CliError::ConfigLoad`] — trust triple cannot be loaded.
/// * [`CliError::Transport`] — control plane unreachable.
/// * [`CliError::HttpStatus`] — `GET /v1/workloads/<id>` returned 4xx/5xx.
///   The 404 path (unknown workload) is the load-bearing operator-facing
///   error and carries `body.error = "not_found"`.
/// * [`CliError::BodyDecode`] — the server returned a 2xx with a
///   malformed body.
pub async fn describe(args: DescribeArgs) -> Result<WorkloadDescribeOutput, CliError> {
    let client = ApiClient::from_config(&args.config_path)?;

    // Single round-trip through the snapshot surface. The handler reads
    // IntentStore + Observation rows + WorkloadLifecycle view-cache and
    // returns the full envelope; 404 on missing workload carries
    // `body.error == "not_found"`.
    let snapshot = client.alloc_status_for_workload(&args.id).await?;

    let allocations_total = snapshot.rows.len();
    let empty_state_message = if allocations_total == 0 {
        format!(
            "0 allocations for job {job} — the scheduler + driver land in \
             phase-1-first-workload",
            job = args.id,
        )
    } else {
        String::new()
    };

    let spec_digest = snapshot.spec_digest.clone().unwrap_or_default();

    Ok(WorkloadDescribeOutput {
        workload_id: args.id,
        spec_digest,
        allocations_total,
        empty_state_message,
        snapshot,
    })
}

/// Return the raw [`AllocStatusResponse`] envelope.
///
/// Variant of [`describe`] used by tests (notably S-WS-02) that need to
/// assert on the cause-class typed payload byte-equality across
/// streaming + snapshot surfaces. Bypasses the operator-facing
/// derivation step (`empty_state_message`, etc.) so the assertion target
/// is the raw wire shape.
///
/// # Errors
///
/// Same shapes as [`describe`].
pub async fn describe_snapshot(args: DescribeArgs) -> Result<AllocStatusResponse, CliError> {
    let client = ApiClient::from_config(&args.config_path)?;
    client.alloc_status_for_workload(&args.id).await
}

// ---------------------------------------------------------------------------
// JSON-mode marshalling — slice 06 step 02-03 (ADR-0033 enrichment).
// ---------------------------------------------------------------------------
//
// The `probes` field is carried on the JSON view per the ADR-0033
// enrichment shape and is OMITTED entirely for non-Service kinds via
// `#[serde(skip_serializing_if = "Option::is_none")]` — Job / Schedule
// allocs have no readiness/liveness question, so the field is absent
// (not `null`) per US-06.

/// Operator-facing `--json` view of a workload describe, enriched with the
/// per-probe `ProbeResultRowJson` array per ADR-0033.
///
/// `probes` is `Some([...])` for `WorkloadKind::Service` (even when the
/// array is empty — a Service declares the question) and `None` for
/// Job / Schedule, which serialises to an OMITTED field per the
/// skip-if-none attribute. This is the structural kind-guard mirror of
/// the TUI `probes_section` render.
#[derive(Debug, Clone, Serialize)]
pub struct WorkloadDescribeJsonView {
    /// Workload-kind discriminator — drives the `probes` skip-if-none
    /// guard.
    pub kind: WorkloadKind,
    /// Per-probe observation rows projected to the wire shape. `None`
    /// for non-Service kinds (serialises to an omitted field).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probes: Option<Vec<ProbeResultRowJson>>,
}

/// Marshal a workload describe into its `--json` form.
///
/// Enriched with the per-probe array per ADR-0033. Pure function over
/// already-hydrated inputs — the `probes` field is present (as an
/// array) IFF `kind` is `Service`, and OMITTED for Job / Schedule per
/// US-06.
#[must_use]
pub fn format_workload_describe_json(kind: WorkloadKind, probe_rows: &[ProbeResultRow]) -> String {
    let probes = match kind {
        WorkloadKind::Service => Some(probe_rows.iter().map(ProbeResultRowJson::from).collect()),
        WorkloadKind::Job | WorkloadKind::Schedule => None,
    };
    let view = WorkloadDescribeJsonView { kind, probes };
    // `WorkloadDescribeJsonView` is a plain serde struct over owned
    // `String` / `u32` / fieldless-or-string-keyed enums — serde JSON
    // serialisation of such a value is infallible. `unreachable!`
    // documents the invariant per `.claude/rules/development.md`
    // § "Logically unreachable `None` / `Err`" (no `.expect()` in
    // production library code).
    serde_json::to_string(&view).unwrap_or_else(|_| {
        unreachable!("WorkloadDescribeJsonView is infallibly serde-serialisable")
    })
}
