//! `overdrive alloc status --job <id>`.
//!
//! Reads the canonical `spec_digest` from the control plane's
//! `WorkloadDescription`, counts the allocations reported by
//! `GET /v1/allocs`, and returns a typed [`AllocStatusOutput`] with an
//! explicit empty-state message pointing at the
//! `phase-1-first-workload` onboarding step.
//!
//! Per ADR-0020 (drop `commit_index` from Phase 1) the wire shape of
//! `WorkloadDescription` is `{spec, spec_digest}` — the Raft commit-index
//! field was dropped.
//!
//! Per ADR-0002 + handler contract (`describe_workload`): `spec_digest` is
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

use overdrive_control_plane::api::{AllocStatusResponse, ProbeResultRowJson};
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::observation::probe_result_row::ProbeResultRow;
use serde::Serialize;

use crate::http_client::{ApiClient, CliError};

/// Arguments to [`status`].
///
/// `job` is the canonical job id; `config_path` locates the operator
/// trust triple, which is the sole source of the control-plane
/// endpoint per whitepaper §8.
#[derive(Debug, Clone)]
pub struct StatusArgs {
    /// Canonical `WorkloadId` to describe.
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
/// fields (`workload_id`, `spec_digest`, `allocations_total`,
/// `empty_state_message`) are derived from the envelope at construction
/// time so existing renderers (`render::alloc_status`) keep working.
///
/// Per ADR-0020 the Raft `commit_index` field is dropped.
#[derive(Debug, Clone)]
pub struct AllocStatusOutput {
    /// Canonical job id as echoed by the control plane.
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
    /// Full envelope from the server — slice 01 step 01-03 lets the
    /// renderer surface restart budget, last transition, cause-class
    /// reason text per ADR-0033 §4.
    pub snapshot: AllocStatusResponse,
}

/// Read the canonical `WorkloadDescription` for `args.job` + the allocation
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
    // WorkloadLifecycle view-cache and returns the full envelope; 404 on
    // missing job carries `body.error == "not_found"`.
    let snapshot = client.alloc_status_for_workload(&args.job).await?;

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
        workload_id: args.job,
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
    client.alloc_status_for_workload(&args.job).await
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

/// Operator-facing `--json` view of an alloc status, enriched with the
/// per-probe `ProbeResultRowJson` array per ADR-0033.
///
/// `probes` is `Some([...])` for `WorkloadKind::Service` (even when the
/// array is empty — a Service declares the question) and `None` for
/// Job / Schedule, which serialises to an OMITTED field per the
/// skip-if-none attribute. This is the structural kind-guard mirror of
/// the TUI `probes_section` render.
#[derive(Debug, Clone, Serialize)]
pub struct AllocStatusJsonView {
    /// Workload-kind discriminator — drives the `probes` skip-if-none
    /// guard.
    pub kind: WorkloadKind,
    /// Per-probe observation rows projected to the wire shape. `None`
    /// for non-Service kinds (serialises to an omitted field).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probes: Option<Vec<ProbeResultRowJson>>,
}

/// Marshal an alloc status into its `--json` form.
///
/// Enriched with the per-probe array per ADR-0033. Pure function over
/// already-hydrated inputs — the `probes` field is present (as an
/// array) IFF `kind` is `Service`, and OMITTED for Job / Schedule per
/// US-06.
#[must_use]
pub fn format_alloc_status_json(kind: WorkloadKind, probe_rows: &[ProbeResultRow]) -> String {
    let probes = match kind {
        WorkloadKind::Service => Some(probe_rows.iter().map(ProbeResultRowJson::from).collect()),
        WorkloadKind::Job | WorkloadKind::Schedule => None,
    };
    let view = AllocStatusJsonView { kind, probes };
    // `AllocStatusJsonView` is a plain serde struct over owned
    // `String` / `u32` / fieldless-or-string-keyed enums — serde JSON
    // serialisation of such a value is infallible. `unreachable!`
    // documents the invariant per `.claude/rules/development.md`
    // § "Logically unreachable `None` / `Err`" (no `.expect()` in
    // production library code).
    serde_json::to_string(&view)
        .unwrap_or_else(|_| unreachable!("AllocStatusJsonView is infallibly serde-serialisable"))
}
