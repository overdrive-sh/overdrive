//! Wire-shape `SubmitSpecInput` enum and per-kind payloads per ADR-0051.
//!
//! `SubmitSpecInput` is the JSON body shape for `POST /v1/workloads` (and
//! the streaming sibling). It is the wire-side member of the three-layer
//! Rust type universe — distinct from the parser-side
//! [`crate::aggregate::workload_spec::WorkloadSpec`] (TOML) and the
//! persisted [`crate::aggregate::WorkloadIntent`] (rkyv).
//!
//! Tagged JSON via `#[serde(tag = "kind", rename_all = "snake_case")]`:
//! the `kind` field discriminates `job` / `service` / `schedule` and the
//! per-variant fields populate the rest of the body. `utoipa::ToSchema`
//! renders the enum as a `oneOf`-discriminated schema in the generated
//! OpenAPI document.
//!
//! Listener `vip` is structurally unrepresentable per ADR-0049 § 5 /
//! ADR-0051 § 3; `deny_unknown_fields` rejects any incoming JSON
//! carrying it.

use crate::aggregate::probe_descriptor::ProbeDescriptor;
use crate::aggregate::{DriverInput, JobSpecInput, ResourcesInput};
use serde::{Deserialize, Serialize};

/// HTTP/JSON wire-shape for `POST /v1/workloads` (and the streaming
/// sibling). Per ADR-0051 this is the wire-side member of the three-
/// layer Rust universe — distinct from the parser-side
/// [`crate::aggregate::workload_spec::WorkloadSpec`] (TOML) and the
/// persisted [`crate::aggregate::WorkloadIntent`] (rkyv).
///
/// Tagged JSON via `#[serde(tag = "kind", rename_all = "snake_case")]`
/// — the `kind` field discriminates `job` / `service` / `schedule` and
/// the per-variant fields populate the rest of the body.
///
/// `utoipa::ToSchema` renders this as a `oneOf`-discriminated schema in
/// the generated OpenAPI document per ADR-0051 OQ-7.
///
/// Listener `vip` is structurally unrepresentable per ADR-0049 § 5;
/// `deny_unknown_fields` rejects any incoming JSON carrying it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SubmitSpecInput {
    /// Run-to-completion workload — see
    /// [`crate::aggregate::JobSpecInput`] for the per-kind shape.
    Job(JobSpecInput),
    /// Long-running supervised workload with operator-declared
    /// listeners — see [`ServiceSpecInput`].
    Service(ServiceSpecInput),
    /// Cron-scheduled `Job` — see [`ScheduleSpecInput`].
    Schedule(ScheduleSpecInput),
}

/// HTTP/JSON wire-shape for a Service submission per ADR-0051 § 3.
///
/// Mirrors [`crate::aggregate::JobSpecInput`]'s `(id, replicas,
/// resources, driver)` shape plus operator-declared listeners.
///
/// Per ADR-0049 § 5 and ADR-0051 § 2 listeners carry `(port, protocol)`
/// only — NO operator-supplied VIP. The platform issues VIPs via
/// `ServiceVipAllocator` keyed by
/// `WorkloadIntent::Service(_).spec_digest()` after admission; the
/// operator never names a VIP on the wire or in TOML.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceSpecInput {
    pub id: String,
    pub replicas: u32,
    pub resources: ResourcesInput,
    #[serde(flatten)]
    pub driver: DriverInput,
    /// Operator-declared listeners in declaration order. Validated at
    /// admission inside [`crate::aggregate::ServiceV1::from_submit`]:
    /// at least one element; no two share `(port, protocol)`; protocol
    /// is `tcp` / `udp` only.
    pub listeners: Vec<ListenerInput>,
    /// Operator-declared startup probes, projected from the parser-
    /// side `[[health_check.startup]]` blocks. Per ADR-0057. May
    /// include a platform-synthesised default-TCP probe per ADR-0058
    /// when zero startup probes were declared and at least one
    /// listener is present.
    ///
    /// Defaults to an empty `Vec` on the wire when the client omits
    /// the field — `#[serde(default)]` preserves the legacy
    /// `ServiceSpecInput { id, replicas, resources, driver, listeners
    /// }` shape for callers that have not yet been updated to thread
    /// probes. Once an operator declares probes in TOML, the CLI's
    /// `submit_streaming_service` populates this field.
    #[serde(default)]
    pub startup_probes: Vec<ProbeDescriptor>,
    /// Operator-declared readiness probes, projected from the parser-
    /// side `[[health_check.readiness]]` blocks. Same defaulting policy
    /// as [`Self::startup_probes`]: defaults to an empty `Vec` on the
    /// wire when omitted; the CLI's deploy path populates it from TOML.
    /// Evaluated by the reconciler — a satisfied readiness probe flips
    /// the backend healthy (ADR-0055).
    #[serde(default)]
    pub readiness_probes: Vec<ProbeDescriptor>,
    /// Operator-declared liveness probes, projected from the parser-
    /// side `[[health_check.liveness]]` blocks. Same defaulting policy
    /// as [`Self::startup_probes`]. Evaluated by the reconciler — a
    /// liveness failure past `failure_threshold` emits a restart
    /// (ADR-0055).
    #[serde(default)]
    pub liveness_probes: Vec<ProbeDescriptor>,
}

/// HTTP/JSON wire-shape for a single listener entry.
///
/// Distinct from the parser-side
/// [`crate::aggregate::workload_spec::Listener`] newtype in encoding
/// only: the parser side carries `port: NonZeroU16` and
/// `protocol: Proto` after TOML decoding; the wire side carries
/// `port: u16` and `protocol: String` for JSON deserialise tolerance,
/// with validation deferred to
/// [`crate::aggregate::ServiceV1::from_submit`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ListenerInput {
    /// Listener port — 1..=65535. `port == 0` is rejected at admission
    /// inside `ServiceV1::from_submit`.
    #[schema(value_type = u16, minimum = 1, maximum = 65535)]
    pub port: u16,
    /// L4 protocol — `tcp` / `udp` (case-insensitive). Validated at
    /// admission inside `ServiceV1::from_submit`.
    pub protocol: String,
}

/// HTTP/JSON wire-shape for a Schedule submission per ADR-0051 § 3.
///
/// The per-fire instance is a [`crate::aggregate::JobV1`] per ADR-0047
/// § 1 / ADR-0050 § 2; the schedule adds the cron expression. The
/// inner job body uses the same wire shape standalone Jobs use —
/// operators write the schedule body and the inner job body in the
/// same JSON document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduleSpecInput {
    pub id: String,
    /// Inner job specification fired on each cron tick. Same wire
    /// shape standalone Jobs use.
    pub job: JobSpecInput,
    /// Cron expression. String-shaped on the wire; validated and
    /// projected onto `CronExpr` inside `ScheduleV1::from_submit`.
    pub cron_expr: String,
}
