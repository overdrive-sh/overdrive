//! Shared request/response types for the Phase 1 REST API.
//!
//! Per ADR-0014 (§Shared types), the CLI imports these same types
//! directly — they ARE the wire contract. The `OpenAPI` schema derived
//! via `utoipa` (ADR-0009) is a byproduct of these types, not a
//! parallel definition.
//!
//! The shapes pinned here are:
//! - Step 02-03 AC — exact field sets; renaming breaks the contract.
//! - ADR-0008 — endpoint table (`POST /v1/jobs`, `GET /v1/jobs/{id}`,
//!   `GET /v1/cluster/info`, `GET /v1/allocs`, `GET /v1/nodes`).
//! - ADR-0015 — `ErrorBody` shape `{error, message, field}`.
//!
//! `JobSpecInput` is re-used from `overdrive_core::aggregate` so there
//! is exactly one definition of the spec shape on the wire. The CLI
//! will construct `JobSpecInput` from its TOML input; the server will
//! deserialise the same type out of JSON; both route through
//! `Job::from_spec` for validation.

// The `utoipa::OpenApi` derive on `OverdriveApi` below expands to code
// using `.for_each(...)` on the collected schemas. The lint fires on
// the macro expansion rather than any source we wrote, and outer
// `#[allow]` attributes do not propagate into the derive. Scope the
// allow to this module, which contains exactly one `utoipa` derive.
#![allow(clippy::needless_for_each)]

use overdrive_core::TransitionReason;
use overdrive_core::aggregate::{DriverInput, ExecInput, JobSpecInput, ResourcesInput};
use overdrive_core::traits::driver::DriverType;
use overdrive_core::traits::observation_store::AllocState;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Body of `POST /v1/jobs`. Carries the operator-submitted job spec
/// verbatim; the server routes it through `Job::from_spec` to validate
/// and derive the intent key / digest.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubmitJobRequest {
    pub spec: JobSpecInput,
}

/// Response for `POST /v1/jobs`. Carries `job_id`, the canonical
/// `spec_digest`, and the idempotency `outcome` per ADR-0008 and
/// ADR-0020.
///
/// Idempotency contract (ADR-0015 §4 amended by ADR-0020):
///
/// * Fresh insert → `outcome = IdempotencyOutcome::Inserted`, HTTP 200.
/// * Byte-identical resubmission of the same spec at the same key →
///   `outcome = IdempotencyOutcome::Unchanged`, HTTP 200. No write
///   occurred; `spec_digest` is stable across N retries.
/// * Different spec at the same key → HTTP 409 Conflict, no `outcome`
///   field on the wire (conflict is an HTTP-status concern, never an
///   enum value — see ADR-0015 §4 amendment via ADR-0020).
///
/// `job_id` is rendered as a `String` at the wire boundary; the server
/// converts back to `overdrive_core::id::JobId` in handlers.
/// `spec_digest` is the lowercase-hex SHA-256 of the canonical
/// rkyv-archived `Job` bytes (ADR-0002, development.md §Hashing); 64
/// characters.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubmitJobResponse {
    pub job_id: String,
    pub spec_digest: String,
    pub outcome: IdempotencyOutcome,
}

/// Response for `POST /v1/jobs/{id}/stop`. Per ADR-0027 the body shape
/// is `{ job_id, outcome }` where `outcome ∈ { "stopped",
/// "already_stopped" }`. 404 on unknown job (separate path).
///
/// `outcome` is wire-stringly-typed (lowercase JSON via
/// `#[serde(rename_all = "snake_case")]`) so future verbs (start,
/// restart, cancel) can extend the enum additively.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StopJobResponse {
    pub job_id: String,
    pub outcome: StopOutcome,
}

/// Outcome of `POST /v1/jobs/{id}/stop` per ADR-0027.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StopOutcome {
    /// First successful stop — the stop intent was newly recorded.
    Stopped,
    /// A stop intent was already on file for this job — no-op.
    AlreadyStopped,
}

/// Outcome of an idempotent `POST /v1/jobs` submission.
///
/// Distinguishes "your spec landed fresh" from "your spec was already
/// there." Conflict (different spec at same key) is an HTTP-status
/// concern (409), never an enumeration value here — see ADR-0015 §4
/// amendment via ADR-0020.
///
/// Wire shape: `"inserted"` | `"unchanged"` (lowercase JSON via
/// `#[serde(rename_all = "lowercase")]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum IdempotencyOutcome {
    /// The handler took the insert branch — `IntentStore::put_if_absent`
    /// returned `PutOutcome::Inserted`.
    Inserted,
    /// The handler took the idempotency branch —
    /// `IntentStore::put_if_absent` returned
    /// `PutOutcome::KeyExists { existing }` and the candidate bytes
    /// were byte-equal to `existing`. No write occurred.
    Unchanged,
}

/// Response for `GET /v1/jobs/{id}`. Carries the re-hydrated spec and
/// the canonical spec digest per ADR-0014 and US-03 AC (amended by
/// ADR-0020).
///
/// `spec` is typed (`JobSpecInput`), never `serde_json::Value` — the
/// CLI parses this response into a concrete type rather than a value
/// bag. `spec_digest` equals the lowercase-hex SHA-256 of the
/// rkyv-archived bytes pulled out of the `IntentStore` — i.e. the
/// same value the original `POST /v1/jobs` returned for this `job_id`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobDescription {
    pub spec: JobSpecInput,
    pub spec_digest: String,
}

/// Response for `GET /v1/cluster/info`.
///
/// Carries mode, region, the reconciler registry, and the broker
/// counters per ADR-0013 and US-04 AC (amended by ADR-0020 — the
/// `commit_index` field is dropped, no replacement).
///
/// Activity-rate observability is provided by `broker.dispatched`
/// (heartbeat reconciler ticks) plus the `reconcilers` list (the
/// "did the runtime register?" wiring witness). A dedicated metrics
/// endpoint covers cluster-level commit-rate signals starting in
/// Phase 5; the dropped in-memory counter was not a substitute for
/// it. See ADR-0020 §Considered alternatives §D for the full
/// rationale.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClusterStatus {
    pub mode: String,
    pub region: String,
    pub reconcilers: Vec<String>,
    pub broker: BrokerCountersBody,
}

/// Broker counters rendered inside `ClusterStatus`. Tracks the
/// evaluation-broker ingress / cancel / dispatch shape from ADR-0013.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, ToSchema)]
pub struct BrokerCountersBody {
    pub queued: u64,
    pub cancelled: u64,
    pub dispatched: u64,
}

/// Response for `GET /v1/allocs`.
///
/// Phase 1 always renders an empty `rows` array per US-03 AC — the
/// allocation-status path is owned by Phase 2. The typed `rows` field
/// is present so the CLI and external clients can parse the response
/// into a concrete shape rather than `serde_json::Value`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct AllocStatusResponse {
    pub rows: Vec<AllocStatusRowBody>,
}

/// Allocation-status row body.
///
/// Phase 1 shape mirrors the observation `AllocStatusRow` projected to
/// the wire — minimal fields matching the whitepaper §4 schema
/// (`alloc_id`, `job_id`, `node_id`, `state`). Phase 2+ adds columns
/// additively.
///
/// `reason` is `Option<String>` — populated when the underlying state
/// carries actionable diagnostic context (currently only Pending rows
/// resulting from a `PlacementError::NoCapacity`). Other states leave
/// it `None`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema, PartialEq, Eq)]
pub struct AllocStatusRowBody {
    pub alloc_id: String,
    pub job_id: String,
    pub node_id: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl AllocStatusRowBody {
    /// Construct a Pending row body decorated with an actionable
    /// diagnostic `reason` — the `JobLifecycle` reconciler calls this
    /// shape when surfacing `PlacementError::NoCapacity` to the CLI.
    #[must_use]
    pub fn pending_with_reason(
        row: &overdrive_core::traits::observation_store::AllocStatusRow,
        reason: String,
    ) -> Self {
        Self {
            alloc_id: row.alloc_id.to_string(),
            job_id: row.job_id.to_string(),
            node_id: row.node_id.to_string(),
            state: row.state.to_string(),
            reason: Some(reason),
        }
    }
}

/// Response for `GET /v1/nodes`. Phase 1 always renders an empty
/// `rows` array per US-03 AC — node ingestion lands in a later phase.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct NodeList {
    pub rows: Vec<NodeRowBody>,
}

/// Node row body. Phase 1 shape mirrors the observation `NodeHealthRow`
/// projected to the wire — minimal fields (`node_id`, `region`). Phase
/// 2+ adds columns additively.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema, PartialEq, Eq)]
pub struct NodeRowBody {
    pub node_id: String,
    pub region: String,
}

/// RFC-7807-compatible subset per ADR-0015.
///
/// The three fields — `error`, `message`, `field` — are pinned;
/// renaming breaks the contract surface the CLI and external clients
/// depend on. `field` is `Option<String>` because not every error class
/// maps to a single field (e.g. transport-layer errors).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
    pub field: Option<String>,
}

/// Root `OpenAPI` document per ADR-0009.
///
/// Every ADR-0008 handler path is listed in `paths(...)` and every
/// request/response DTO in `components(schemas(...))`. The schema is
/// derived by `utoipa` at compile time; `cargo xtask openapi-gen`
/// writes the YAML rendering of `OverdriveApi::openapi()` to
/// `api/openapi.yaml`; `cargo xtask openapi-check` diffs the live
/// render against the checked-in copy and fails on drift.
///
/// Adding a handler requires adding its path here; adding a DTO
/// requires adding its schema. Drift between code and the `OpenAPI`
/// doc is caught by the CI gate, not in review.
#[derive(utoipa::OpenApi)]
#[openapi(
    info(
        title = "Overdrive Control Plane",
        description = "Phase 1 single-mode control-plane REST API (ADR-0008).",
        version = "0.0.0",
    ),
    paths(
        crate::handlers::submit_job,
        crate::handlers::describe_job,
        crate::handlers::stop_job,
        crate::handlers::cluster_status,
        crate::handlers::alloc_status,
        crate::handlers::node_list,
    ),
    components(schemas(
        SubmitJobRequest,
        SubmitJobResponse,
        IdempotencyOutcome,
        StopJobResponse,
        StopOutcome,
        JobDescription,
        ClusterStatus,
        BrokerCountersBody,
        AllocStatusResponse,
        AllocStatusRowBody,
        NodeList,
        NodeRowBody,
        ErrorBody,
        JobSpecInput,
        ResourcesInput,
        ExecInput,
        DriverInput,
        // Slice 01 step 01-02 — wire types per DWD-03.
        TerminalReason,
        AllocStateWire,
        RestartBudget,
        ResourcesBody,
        TransitionSource,
        TransitionRecord,
        // Cause-class enum re-exported from `overdrive-core` per
        // ADR-0032 §3 Amendment so its `ToSchema` derive registers in
        // the OpenAPI document. The streaming surface (slice 02) and
        // the snapshot surface (slice 01 step 01-03) both reference it.
        TransitionReason,
        // `DriverType` carries the new `ToSchema` derive (DWD-03 cross-
        // cutting derive change); the `TransitionSource::Driver`
        // variant references it inline so the schema must register.
        DriverType,
    )),
    tags(
        (name = "jobs", description = "Job lifecycle endpoints"),
        (name = "cluster", description = "Cluster status endpoints"),
        (name = "observation", description = "Observation-store read endpoints"),
    ),
)]
pub struct OverdriveApi;

// ---------------------------------------------------------------------------
// Wire types — Slice 01 GREEN promotions per DWD-03
//
// The four scaffold types from DISTILL — `TerminalReason`, `AllocStateWire`,
// `RestartBudget`, `ResourcesBody` — are promoted to GREEN with full
// `Serialize`/`Deserialize`/`ToSchema`/`Debug`/`Clone`/`PartialEq` derives.
// `TransitionSource` and `TransitionRecord` are the deferred net-new types
// from DWD-03 — they require `ToSchema` on `DriverType` (a cross-cutting
// derive change in `overdrive-core::traits::driver`), which lands in this
// same step.
//
// The streaming `SubmitEvent` declaration (which carries the same
// `TransitionSource` chain) is deferred to slice 02 step 02-02 so it can
// land in lockstep with the broadcast-channel wiring in `AppState` and
// the NDJSON streaming handler. Both surfaces share the SAME
// `TransitionReason` enum re-exported from `overdrive-core` —
// byte-equality across surfaces is structural, not discipline.
// ---------------------------------------------------------------------------

/// Streaming `SubmitEvent::ConvergedFailed` terminal-cause discriminator.
///
/// Phase 1 variants per ADR-0032 §3 (additive going forward —
/// `#[non_exhaustive]`).
///
/// **Amended 2026-04-30 in lockstep with `TransitionReason`'s cause-
/// class refactor**: the variants now carry structured payloads. The
/// inner `cause: TransitionReason` on `BackoffExhausted` and
/// `DriverError` duplicates the most recent cause-class
/// `LifecycleTransition.reason` so a CLI rendering only the terminal
/// line still has structured cause data; the `Timeout` variant carries
/// the configured cap so renderers can say "did not converge in 60s"
/// without reading server config.
///
/// | Variant | When emitted by the streaming handler |
/// |---|---|
/// | `DriverError { cause }` | unrecoverable driver error on a path the reconciler will not retry |
/// | `BackoffExhausted { attempts, cause }` | restart budget hit (5 attempts in Phase 1) |
/// | `Timeout { after_seconds }` | server wall-clock cap fired |
///
/// The CLI maps `ConvergedRunning → 0` and `ConvergedFailed → 1` regardless
/// of the inner `terminal_reason`; the terminal reason controls *rendering*,
/// not exit code (ADR-0032 §9).
///
/// Wire shape via `#[serde(tag = "kind", content = "data", rename_all =
/// "snake_case")]` — same shape as `TransitionReason`. The variants are
/// no longer `Copy`: `BackoffExhausted` and `DriverError` carry an inner
/// `cause: TransitionReason`, which is itself non-`Copy` (cause-class
/// variants own `String` payloads). Consumers either clone (cheap for
/// progress markers, owned-data for cause variants) or take by reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TerminalReason {
    /// Streaming handler observed an unrecoverable driver error on a
    /// path the reconciler will not retry. `cause` is the cause-class
    /// `TransitionReason` that originated the terminal failure.
    DriverError { cause: TransitionReason },
    /// Streaming handler observed `restart_count == max` and the latest
    /// row state is `Failed`. `attempts` is the number of attempts made
    /// (= `RESTART_BUDGET_MAX` in Phase 1, hard-coded to 5); `cause` is
    /// the cause-class `TransitionReason` of the final failed attempt.
    BackoffExhausted { attempts: u32, cause: TransitionReason },
    /// Streaming handler's wall-clock cap fired before any terminal
    /// event arrived. `after_seconds` is the configured cap so the CLI
    /// can render `"did not converge in {after_seconds}s"` without
    /// reading server config.
    Timeout { after_seconds: u32 },
}

/// Wire-shaped projection of the internal `AllocState` enum.
///
/// The internal `AllocState` (in `overdrive-core::traits::observation_store`)
/// derives `rkyv::*` for the observation store. The wire shape needs
/// `Serialize`/`Deserialize`/`ToSchema` and a stable lowercase string repr.
/// Adding all those derives to the internal type would entangle storage
/// and wire concerns, so this mirror enum exists for the wire surface
/// (ADR-0032 §3, reuse-analysis CREATE NEW rationale).
///
/// `Failed` per ADR-0032 §5 — the action shim, when handling
/// `DriverError::StartRejected`, writes `state: Failed` (instead of
/// `Terminated`). The internal `AllocState::Failed` variant landed in
/// step 01-01; this wire type's `Failed` variant projects it.
///
/// Conversion is mechanical via [`From<AllocState>`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum AllocStateWire {
    Pending,
    Running,
    Draining,
    Suspended,
    Terminated,
    /// Per ADR-0032 §5 — distinguishes "operator stopped" from
    /// "driver could not start".
    Failed,
}

impl From<AllocState> for AllocStateWire {
    fn from(state: AllocState) -> Self {
        match state {
            AllocState::Pending => Self::Pending,
            AllocState::Running => Self::Running,
            AllocState::Draining => Self::Draining,
            AllocState::Suspended => Self::Suspended,
            AllocState::Terminated => Self::Terminated,
            AllocState::Failed => Self::Failed,
        }
    }
}

/// Snapshot's restart-budget block per ADR-0033 §1.
///
/// `exhausted` is redundant with `used >= max`; carried explicitly on the
/// wire so a CLI that wants to render the `(backoff exhausted)` annotation
/// does not have to compare two integers each time.
///
/// Phase 1: `max` is hard-coded to 5 (matching the existing
/// `RESTART_BUDGET_MAX` constant in `JobLifecycle::reconcile`); Phase 2+
/// makes it per-job-config (DESIGN [D7]).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct RestartBudget {
    pub used: u32,
    pub max: u32,
    pub exhausted: bool,
}

/// Snapshot's per-row `resources` block per ADR-0033 §1.
///
/// Mirrors the internal `overdrive_core::traits::driver::Resources` shape
/// for the wire. Conversion is mechanical (call site at step 01-03 wires
/// the `From<&Resources>` projection).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ResourcesBody {
    pub cpu_milli: u32,
    pub memory_bytes: u64,
}

/// Source of a lifecycle transition — who/what produced the row write.
///
/// Phase 1 has two variants:
///
/// | Variant | When emitted |
/// |---|---|
/// | `Reconciler` | the `JobLifecycle` reconciler converged a state and emitted an `Action::*` that the action shim materialised into an `AllocStatusRow` write |
/// | `Driver(DriverType)` | the action shim observed a driver `start`/`stop`/`status` result and wrote the row directly (post-spawn settle, immediate failure, etc.) |
///
/// The `Driver(DriverType)` carries the driver kind so a CLI rendering
/// the snapshot can say `from driver=exec` without round-tripping
/// through cluster-info to look up the active drivers. Phase 2+ may add
/// more variants (operator action, gateway redirect, sidecar) — the
/// enum is `#[non_exhaustive]` to make additions additive.
///
/// Wire shape via `#[serde(tag = "kind", content = "data", rename_all =
/// "snake_case")]` — `{"kind": "reconciler"}` for the unit variant,
/// `{"kind": "driver", "data": "exec"}` for the typed variant
/// (`DriverType` itself serialises as a kebab-case string per its own
/// `#[serde(rename_all = "kebab-case")]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TransitionSource {
    /// Reconciler emitted the action that produced this row.
    Reconciler,
    /// Driver (named) produced this row directly.
    Driver(DriverType),
}

/// Lifecycle-transition record carried inside the snapshot's
/// `last_transition` block per ADR-0033 §1 and on the streaming
/// `SubmitEvent::LifecycleTransition` event per ADR-0032 §3.
///
/// Both surfaces share the SAME `TransitionRecord` shape — the
/// type-identity assertion in
/// `tests/acceptance/transition_reason_type_identity.rs` (S-AS-02)
/// pins this at compile time so byte-equality across surfaces is
/// structural rather than discipline.
///
/// `from` is `None` for the very first transition emitted for an
/// allocation (there is no prior state); subsequent transitions carry
/// the previous wire-state. `to` is always populated.
///
/// `at` is the logical-timestamp string from `LogicalTimestamp::Display`
/// (Phase 1: `(counter, writer)` rendered via the existing observation
/// timestamp shape). The wire keeps it stringly-typed because the CLI
/// renders it verbatim and never round-trips it through arithmetic; a
/// future phase that needs structured wall-clock can split into
/// `at_logical` + `at_wallclock` fields additively.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TransitionRecord {
    /// Wire-state the allocation was in before this transition.
    /// `None` for the first transition emitted for an alloc id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<AllocStateWire>,
    /// Wire-state the allocation moved to.
    pub to: AllocStateWire,
    /// Structured cause for this transition. SAME enum as the streaming
    /// `SubmitEvent::LifecycleTransition.reason` — pinned by
    /// S-AS-02's compile-time witness.
    pub reason: TransitionReason,
    /// Who/what produced this row write.
    pub source: TransitionSource,
    /// Logical-timestamp string for this transition. Stringly-typed on
    /// the wire — see struct-level docs.
    pub at: String,
}
