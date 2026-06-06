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
//! Per ADR-0051 (Accepted 2026-05-15) the wire-side workload spec is
//! [`SubmitSpecInput`] — a `oneOf`-discriminated enum at
//! `overdrive_core::api::submit` carrying `Job` / `Service` /
//! `Schedule` arms. The CLI projects parsed TOML onto a
//! `SubmitSpecInput` variant; the server deserialises the same type
//! out of JSON; both route through the per-kind validating
//! constructors (`JobV1::from_submit` / `ServiceV1::from_submit` /
//! `ScheduleV1::from_submit`) at the wire → intent boundary.

// The `utoipa::OpenApi` derive on `OverdriveApi` below expands to code
// using `.for_each(...)` on the collected schemas. The lint fires on
// the macro expansion rather than any source we wrote, and outer
// `#[allow]` attributes do not propagate into the derive. Scope the
// allow to this module, which contains exactly one `utoipa` derive.
#![allow(clippy::needless_for_each)]

use overdrive_core::TransitionReason;
use overdrive_core::aggregate::{DriverInput, ExecInput, JobSpecInput, ResourcesInput};
use overdrive_core::api::describe::{DescribeSpecOutput, ScheduleSpecOutput, ServiceSpecOutput};
use overdrive_core::api::submit::{
    ListenerInput, ScheduleSpecInput, ServiceSpecInput, SubmitSpecInput,
};
use overdrive_core::traits::driver::DriverType;
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::transition_reason::StoppedBy;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Body of `POST /v1/jobs`. Carries the operator-submitted workload
/// spec verbatim per ADR-0051 (Accepted 2026-05-15); the server
/// dispatches on the [`SubmitSpecInput`] variant and routes each
/// arm through the per-kind validating constructor
/// (`JobV1::from_submit` / `ServiceV1::from_submit` /
/// `ScheduleV1::from_submit`) to derive the intent key / digest.
///
/// Per ADR-0051 § 6 the wire-side `kind` tag inside `SubmitSpecInput`
/// is the SOLE workload-kind discriminator carrier — the previous
/// optional top-level `workload_kind: Option<String>` field was
/// deleted in step 02-03b. Legacy clients that sent
/// `workload_kind = "job"` migrate to `spec = { kind: "job", ... }`;
/// `kind` is required (not optional) and the JSON shape is
/// `oneOf`-discriminated.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubmitWorkloadRequest {
    pub spec: SubmitSpecInput,
}

/// Response for `POST /v1/jobs`. Carries `workload_id`, the canonical
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
/// `workload_id` is rendered as a `String` at the wire boundary; the server
/// converts back to `overdrive_core::id::WorkloadId` in handlers.
/// `spec_digest` is the lowercase-hex SHA-256 of the canonical
/// rkyv-archived `Job` bytes (ADR-0002, development.md §Hashing); 64
/// characters.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubmitWorkloadResponse {
    pub workload_id: String,
    pub spec_digest: String,
    pub outcome: IdempotencyOutcome,
    /// Service-issued VIP — populated for `WorkloadKind::Service`
    /// submissions per ADR-0049 (amended 2026-05-15). `None` for
    /// `Job` / `Schedule` submissions (those kinds carry no service-
    /// level VIP). Rendered as the canonical dotted-decimal IPv4
    /// form (e.g. `"10.96.0.2"`).
    ///
    /// Per the Q1.A resolution 2026-05-15: this is a top-level
    /// `Option<String>` rather than a per-kind discriminated oneOf
    /// response shape. The symmetric oneOf shape is deferred (would
    /// pull in Job / Schedule response migration not in this
    /// feature's scope).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vip: Option<String>,
}

/// Response for `POST /v1/jobs/{id}/stop`. Per ADR-0027 the body shape
/// is `{ workload_id, outcome }` where `outcome ∈ { "stopped",
/// "already_stopped" }`. 404 on unknown job (separate path).
///
/// `outcome` is wire-stringly-typed (lowercase JSON via
/// `#[serde(rename_all = "snake_case")]`) so future verbs (start,
/// restart, cancel) can extend the enum additively.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StopWorkloadResponse {
    pub workload_id: String,
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
/// `spec` is typed (`DescribeSpecOutput`), never `serde_json::Value` —
/// the CLI parses this response into a concrete type rather than a value
/// bag. Per ADR-0064 it is a kind-discriminated `oneOf` (the describe-side
/// inverse of the submit-side `SubmitSpecInput` per ADR-0051): `Job` carries
/// the verbatim `JobSpecInput` projection, `Service` carries
/// `ServiceSpecOutput` (with the platform-issued VIP the submit shape
/// structurally forbids), and `Schedule` carries `ScheduleSpecOutput`.
/// `spec_digest` equals the lowercase-hex SHA-256 of the rkyv-archived
/// bytes pulled out of the `IntentStore` — i.e. the same value the original
/// `POST /v1/jobs` returned for this `workload_id`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkloadDescription {
    pub spec: DescribeSpecOutput,
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
/// Per ADR-0033 §1 amended 2026-04-30 / Slice 01 step 01-03 — the
/// envelope carries top-level identity (`workload_id`, `spec_digest`),
/// replica counts (`replicas_desired` / `replicas_running`) projected
/// from the `IntentStore` + observation rows, and a `restart_budget`
/// block hydrated from the `WorkloadLifecycle` reconciler view cache.
///
/// On the empty / 200 path with no rows the envelope still carries
/// `replicas_desired` (from the spec) so the CLI can render an
/// honest empty state — see step 01-03 task description and
/// `wave-decisions.md` [D2].
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct AllocStatusResponse {
    /// Canonical job id this snapshot describes. `None` is reserved
    /// for forward-compat (Phase 2 may add cluster-wide reads); Phase 1
    /// always populates it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload_id: Option<String>,
    /// SHA-256 (hex, 64 chars) of the canonical rkyv-archived `Job`
    /// bytes — see `WorkloadDescription::spec_digest`. Pinned per ADR-0002.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_digest: Option<String>,
    /// Desired replica count from `Job.spec.replicas`.
    #[serde(default)]
    pub replicas_desired: u32,
    /// Number of `rows` whose state is `Running`.
    #[serde(default)]
    pub replicas_running: u32,
    pub rows: Vec<AllocStatusRowBody>,
    /// Aggregate restart-budget block for the job — derived from the
    /// `WorkloadLifecycle` reconciler view cache. `max` is hard-coded to
    /// `RESTART_BACKOFF_CEILING` (5) in Phase 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_budget: Option<RestartBudget>,
    /// Workload-kind discriminator per ADR-0047 §1 / step 02-02 [D4].
    /// The CLI render layer branches on this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<overdrive_core::aggregate::WorkloadKind>,
    /// Service-issued VIP — populated for `WorkloadKind::Service`
    /// reads per ADR-0049 (amended 2026-05-15). `None` for
    /// `Job` / `Schedule` reads. Resolved from the persistent
    /// allocator's content-addressed memo keyed by the workload's
    /// `WorkloadIntent::Service(_).spec_digest()`.
    ///
    /// Per the Q1.A resolution 2026-05-15: per-listener lines do
    /// NOT carry a per-listener VIP — listeners are `(port, protocol)`
    /// only; the Service-level VIP lives at this top-level field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vip: Option<String>,
    /// Operator-declared listeners in declaration order — `(port,
    /// protocol)` only, reusing the typed `overdrive_core::aggregate::
    /// Listener` wire shape. Populated for `WorkloadKind::Service` reads
    /// (projected from the persisted `WorkloadIntent::Service` aggregate's
    /// `listeners`); empty for `Job` / `Schedule` reads (those carry no
    /// listeners). The CLI render layer renders each as `<port>/<protocol>`
    /// so a UDP service's `Proto::Udp` is operator-visible.
    ///
    /// Additive-safe: `skip_serializing_if = "Vec::is_empty"` keeps the
    /// JSON wire backward-compatible for non-Service reads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub listeners: Vec<overdrive_core::aggregate::Listener>,
}

/// Allocation-status row body — extended per ADR-0033 §1 / Slice 01
/// step 01-03.
///
/// `state` is the typed `AllocStateWire` (promoted from `String` per
/// [C9] greenfield single-cut — no parallel legacy field). `reason`
/// remains the typed `Option<TransitionReason>` from ADR-0032 §3
/// Amendment; the renderer calls `TransitionReason::human_readable()`
/// for display. New cause-class payloads carry their structured data
/// directly on the wire, and the row's `error` field carries the
/// verbatim driver detail string (mirrors `AllocStatusRow.detail`).
///
/// `last_transition` is `Option<TransitionRecord>` — populated when
/// the row's `reason` is set; the renderer reads it to produce the
/// `from → to reason: ... source: ... at: ...` block.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct AllocStatusRowBody {
    pub alloc_id: String,
    pub workload_id: String,
    pub node_id: String,
    pub state: AllocStateWire,
    /// Structured cause for this row's most recent transition.
    /// Source-of-truth pin: this enum is identical to the streaming
    /// `LifecycleTransition.reason` surface; byte-equality across
    /// surfaces is structural ([C6]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<overdrive_core::TransitionReason>,
    /// Resource envelope this allocation requested.
    pub resources: ResourcesBody,
    /// Logical-timestamp string of the row's first observed transition
    /// to a non-Pending state. `None` for never-started Pending rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// Phase 2+ — exit code observation. `None` in Phase 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Last-transition structured record. `None` for never-transitioned
    /// rows (e.g. the very first Pending observation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_transition: Option<TransitionRecord>,
    /// Verbatim driver / OS detail text — mirrors the underlying row's
    /// `detail: Option<String>`. This is the audit-preserving sidecar
    /// the typed `reason` payload cannot capture (e.g. raw `errno`
    /// strings).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
/// derived by `utoipa` at compile time; `cargo openapi-gen`
/// writes the YAML rendering of `OverdriveApi::openapi()` to
/// `api/openapi.yaml`; `cargo openapi-check` diffs the live
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
        crate::handlers::submit_workload,
        crate::handlers::describe_workload,
        crate::handlers::stop_workload,
        crate::handlers::cluster_status,
        crate::handlers::alloc_status,
        crate::handlers::node_list,
    ),
    components(schemas(
        SubmitWorkloadRequest,
        SubmitWorkloadResponse,
        IdempotencyOutcome,
        StopWorkloadResponse,
        StopOutcome,
        WorkloadDescription,
        // ADR-0064 describe-side wire shape — `DescribeSpecOutput` is the
        // kind-discriminated `oneOf` response now carried by
        // `WorkloadDescription.spec` (the describe-side inverse of
        // `SubmitSpecInput`). Per-variant payload schemas (Service /
        // Schedule) are registered explicitly so they appear in the
        // OpenAPI document's `components.schemas` section regardless of
        // the renderer's nested-resolution behavior. The Job arm reuses
        // `JobSpecInput`, already registered below.
        DescribeSpecOutput,
        ServiceSpecOutput,
        ScheduleSpecOutput,
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
        // ADR-0051 wire-shape — `SubmitSpecInput` is the discriminated
        // `oneOf` body now carried by `SubmitWorkloadRequest.spec`.
        // Per-variant payload schemas (Service / Schedule) plus the
        // listener leaf are registered explicitly so they appear in the
        // OpenAPI document's `components.schemas` section regardless of
        // the renderer's nested-resolution behavior.
        SubmitSpecInput,
        ServiceSpecInput,
        ScheduleSpecInput,
        ListenerInput,
        // Slice 01 step 01-02 — wire types per DWD-03.
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
        // `StoppedBy` is carried by `ServiceSubmitEvent::Stopped.by`
        // and must be registered so the schema reference resolves.
        StoppedBy,
        // workload-kind-discriminator slice 06 — Service `[[listener]]`
        // spec types per ADR-0047 §1. Registered here so the OpenAPI
        // document carries `Listener` and `ServiceVip` schema references
        // for any future wire surface that names them.
        overdrive_core::aggregate::Listener,
        overdrive_core::aggregate::ServiceVip,
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
// The streaming-event declarations (which carry the same
// `TransitionSource` chain) are deferred to slice 02 step 02-02 so they can
// land in lockstep with the broadcast-channel wiring in `AppState` and
// the NDJSON streaming handler. Both surfaces share the SAME
// `TransitionReason` enum re-exported from `overdrive-core` —
// byte-equality across surfaces is structural, not discipline.
// ---------------------------------------------------------------------------

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
/// `RESTART_BUDGET_MAX` constant in `WorkloadLifecycle::reconcile`); Phase 2+
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
/// for the wire. Conversion is mechanical via [`From<&Resources>`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ResourcesBody {
    pub cpu_milli: u32,
    pub memory_bytes: u64,
}

impl From<&overdrive_core::traits::driver::Resources> for ResourcesBody {
    fn from(r: &overdrive_core::traits::driver::Resources) -> Self {
        Self { cpu_milli: r.cpu_milli, memory_bytes: r.memory_bytes }
    }
}

/// Source of a lifecycle transition — who/what produced the row write.
///
/// Phase 1 has two variants:
///
/// | Variant | When emitted |
/// |---|---|
/// | `Reconciler` | the `WorkloadLifecycle` reconciler converged a state and emitted an `Action::*` that the action shim materialised into an `AllocStatusRow` write |
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
/// `LifecycleEvent` broadcast payload per ADR-0032 §3.
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
    /// `LifecycleEvent.reason` — pinned by
    /// S-AS-02's compile-time witness.
    pub reason: TransitionReason,
    /// Who/what produced this row write.
    pub source: TransitionSource,
    /// Logical-timestamp string for this transition. Stringly-typed on
    /// the wire — see struct-level docs.
    pub at: String,
}

// ---------------------------------------------------------------------------
// Service-health-check-probes wire types — step 01-03e (ADR-0056 / DDD-11).
// ---------------------------------------------------------------------------
//
// `ProbeWitnessWire` / `ServiceFailureReasonWire` are typed aliases to the
// typed `overdrive-core::transition_reason::{ProbeWitness,
// ServiceFailureReason}`. The aliases are the load-bearing structural
// guarantee: every typed variant has a wire projection by construction —
// adding a new variant to either typed enum/struct without updating the
// wire is a compile-time impossibility (the alias forces exhaustive
// coverage). Property-tested at byte-equality in
// `tests/acceptance/service_submit_event_v2.rs` (S-SHCP-WIRE-03,
// S-SHCP-PURITY-03).
//
// `ProbeResultRowJson` is a wire-only mirror of
// `overdrive-core::observation::probe_result_row::ProbeResultRow` (which
// is rkyv-persisted and therefore carries `rkyv::*` derives the wire
// types must NOT inherit). Operator-facing surfaces (HTTP snapshot,
// streaming probe-observation lane in future slices) project from the
// typed row into this struct via `From<&ProbeResultRow>`.

/// Wire projection of `overdrive-core::ProbeWitness` — typed alias.
///
/// `ProbeWitness` already carries `Serialize + Deserialize + ToSchema`
/// (declared in `overdrive-core::transition_reason`). The wire alias
/// pins the structural invariant per ADR-0056 / DDD-10: every typed
/// variant has a wire projection by construction. Renaming /
/// re-shaping the typed `ProbeWitness` updates the wire shape in
/// lockstep — no manual sync required.
pub type ProbeWitnessWire = overdrive_core::transition_reason::ProbeWitness;

/// Wire projection of `overdrive-core::ServiceFailureReason` — typed
/// alias.
///
/// Same shape as `ProbeWitnessWire`: the typed enum already serialises
/// to the wire shape (`tag = "reason", content = "data", rename_all =
/// "snake_case"`); the alias preserves the lockstep guarantee per
/// ADR-0056 §4 / DDD-10. Adding a new variant to `ServiceFailureReason`
/// without adding the wire projection is a compile-time impossibility.
pub type ServiceFailureReasonWire = overdrive_core::transition_reason::ServiceFailureReason;

/// Wire-only mirror of
/// `overdrive-core::observation::probe_result_row::ProbeResultRow`.
///
/// `ProbeResultRow` is rkyv-persisted (carries `rkyv::Archive` /
/// `rkyv::Serialize` / `rkyv::Deserialize`); this wire struct carries
/// only the JSON-shape derives (`Serialize` / `Deserialize` /
/// `ToSchema`) and projects from the typed row via
/// [`From<&ProbeResultRow>`].
///
/// Per ADR-0048: rkyv-persisted types follow the per-type versioned
/// envelope discipline; wire-only types use serde directly. This
/// separation keeps storage and wire concerns decoupled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProbeResultRowJson {
    /// Allocation this probe targets.
    pub alloc_id: String,
    /// 0-indexed position within the role array.
    pub probe_idx: u32,
    /// Which role this probe is (`startup` / `readiness` / `liveness`).
    pub role: String,
    /// Latest observed outcome — `pass` or `fail` with last_fail_reason.
    pub status: ProbeStatusJson,
    /// Wall-clock at which the probe attempt completed. UNIX-epoch
    /// milliseconds; sourced from the ProbeRunner's injected clock.
    pub last_observed_at_unix_ms: u64,
    /// Whether this probe was inferred by the platform per ADR-0058.
    pub inferred: bool,
}

/// Wire projection of `overdrive-core::observation::probe_result_row::
/// ProbeStatus` — `pass` / `fail { last_fail_reason }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", tag = "status", content = "data")]
pub enum ProbeStatusJson {
    Pass,
    Fail { last_fail_reason: String },
}

impl From<&overdrive_core::observation::probe_result_row::ProbeResultRow> for ProbeResultRowJson {
    fn from(row: &overdrive_core::observation::probe_result_row::ProbeResultRow) -> Self {
        use overdrive_core::observation::probe_result_row::{ProbeRole, ProbeStatus};
        let status = match &row.status {
            ProbeStatus::Pass => ProbeStatusJson::Pass,
            ProbeStatus::Fail { last_fail_reason } => {
                ProbeStatusJson::Fail { last_fail_reason: last_fail_reason.clone() }
            }
        };
        // Role projection — matches the `#[serde(rename_all = "snake_case")]`
        // shape declared on `ProbeRole`.
        let role = match row.role {
            ProbeRole::Startup => "startup",
            ProbeRole::Readiness => "readiness",
            ProbeRole::Liveness => "liveness",
        }
        .to_string();
        Self {
            alloc_id: row.alloc_id.to_string(),
            probe_idx: row.probe_idx.get(),
            role,
            status,
            last_observed_at_unix_ms: row.last_observed_at_unix_ms,
            inferred: row.inferred,
        }
    }
}
