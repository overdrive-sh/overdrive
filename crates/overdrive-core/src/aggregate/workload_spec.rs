//! `WorkloadSpec` tagged enum + `WorkloadSpecInput` custom Deserialize.
//!
//! Slice 01 of `workload-kind-discriminator` per ADR-0047. Introduces the
//! workload-kind discriminator at the parser boundary as the new
//! abstraction every downstream slice depends on.
//!
//! # Why a custom Deserialize, not `#[serde(untagged)]`
//!
//! Per ADR-0047 §2 the parser MUST produce error messages that name the
//! offending TOML sections explicitly. `#[serde(untagged)]` collapses to
//! a generic "data did not match any variant of untagged enum" message —
//! useless to operators. The custom impl walks the TOML `Value::Table`
//! by section presence: `[service]` alone → `Service`, `[job]` alone →
//! `Job`, `[job]+[schedule]` → `Schedule`. Mixed-kind specs are rejected
//! with structured `ParseError` variants whose `Display` form names the
//! offending section names.
//!
//! # Coexistence with the legacy `Job` aggregate
//!
//! Slice 01 ships the parser-side abstraction additively. The legacy
//! `aggregate::Job` / `aggregate::JobSpecInput` types remain in
//! `aggregate/mod.rs` as the production path until downstream slices
//! (02–06) migrate every reader to `WorkloadSpec`. Per the slice spec:
//! > `WorkloadSpec::Service` (no submit semantics yet — that's still
//! > the legacy code path in this slice; full Service-side wiring is
//! > Slice 04 vocabulary preservation).
//!
//! # Cron validation
//!
//! `CronExpr` is a Phase-1 String-shaped newtype that validates
//! non-empty after trim. Richer cron syntax validation is tracked under
//! GH #166 — Slice 05 will land semantic parsing.

use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::dataplane::backend_key::Proto;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Structured parser error for `WorkloadSpecInput`. Every variant's
/// `Display` form names the offending section(s) and suggests the
/// corrective action — per ADR-0047 §2 / Slice 01 AC.
#[derive(Debug, Error)]
pub enum ParseError {
    /// Both `[service]` and `[job]` are present. Per ADR-0047 §1, exactly
    /// one is required.
    #[error(
        "both [service] and [job] sections are present; exactly one of [service] or [job] is required"
    )]
    MixedServiceAndJob,

    /// `[schedule]` appears without `[job]`. Per ADR-0047 §1, the
    /// `[schedule]` section is only valid alongside `[job]`.
    #[error("[schedule] is only valid alongside [job]; [job] section is missing")]
    ScheduleWithoutJob,

    /// `[schedule]` appears with `[service]`. Same rule as
    /// `ScheduleWithoutJob` — kept distinct for operator-facing clarity.
    #[error(
        "[schedule] is only valid alongside [job]; found [service] instead — exactly one of [service] or [job] is required"
    )]
    ScheduleWithService,

    /// Neither `[service]` nor `[job]` is present.
    #[error("missing required section: exactly one of [service] or [job] is required")]
    MissingKindSection,

    /// `[exec]` is missing.
    #[error("missing required section: [exec]")]
    MissingExec,

    /// `[resources]` is missing.
    #[error("missing required section: [resources]")]
    MissingResources,

    /// `cron` field missing or empty inside `[schedule]`.
    #[error("[schedule]: required field `cron` is missing or empty")]
    MissingCron,

    /// Underlying TOML parse failure (malformed input, type mismatch).
    #[error("TOML parse error: {0}")]
    Toml(String),

    /// A field within an otherwise valid section failed to deserialise.
    #[error("{section}: {message}")]
    Field {
        /// Section name (e.g. `[service]`, `[exec]`).
        section: &'static str,
        /// Per-field reason.
        message: String,
    },

    // -----------------------------------------------------------------
    // Slice 06 — Service `[[listener]]` validation errors per
    // test-scenarios.md §8 (S-08-03..S-08-06) and ADR-0047 §1.
    // -----------------------------------------------------------------
    /// `[service]` body has no `[[listener]]` blocks. Per S-08-03 a
    /// Service requires ≥1 listener.
    #[error("a [service] requires at least one [[listener]] block")]
    ListenerMissing,

    /// Two `[[listener]]` blocks share the same `(vip, port, protocol)`
    /// triple. Per S-08-04 — when both vip are `None`, comparison falls
    /// back to `(port, protocol)` only.
    #[error("duplicate [[listener]] triple: {triple}")]
    ListenerDuplicate {
        /// Human-readable rendering of the offending triple
        /// (e.g. `(vip=10.0.0.1, port=8080, protocol=tcp)` or
        /// `(vip=none, port=8080, protocol=tcp)`).
        triple: String,
    },

    /// A `[[listener]]` carried a protocol value outside the
    /// `tcp` / `udp` set. Per S-08-05 the supported set is named
    /// in the error message verbatim.
    #[error("unsupported listener protocol {value:?} (supported protocols: tcp, udp)")]
    ListenerUnsupportedProtocol {
        /// Verbatim operator-supplied protocol token.
        value: String,
    },

    /// A `[[listener]]` carried `port = 0`. Per S-08-06 the port must
    /// be in 1..=65535.
    #[error("listener port must be in 1..=65535")]
    ListenerPortZero,

    /// A TOML section carried a field the parser does not accept.
    ///
    /// Per `service-vip-allocator` step 02-01 / ADR-0049 § 5 — the
    /// operator-supplied `vip` field on `[[listener]]` was removed
    /// from the [`Listener`] struct: VIPs are platform-issued via
    /// `ServiceVipAllocator` keyed by `spec_digest` and are
    /// structurally unrepresentable in the operator-facing spec. The
    /// parser rejects any unknown field with this typed variant; the
    /// `Display` form names the offending field AND tells the
    /// operator to remove it.
    #[error(
        "{section}: unknown field `{field}` — remove the `{field}` field; VIPs are platform-issued and not operator-configurable"
    )]
    UnknownField {
        /// Section name (e.g. `[[listener]]`).
        section: &'static str,
        /// The offending field token, verbatim from the operator
        /// input.
        field: String,
    },

    // -----------------------------------------------------------------
    // Step 01-02 — service-health-check-probes per ADR-0057 §3.
    // -----------------------------------------------------------------
    /// `[[health_check.startup]]` with `type = "tcp"` is missing the
    /// `port` field. `probe_idx` is the 0-indexed position within
    /// the per-role array.
    #[error(
        "[[health_check.startup]][{probe_idx}]: tcp probe is missing required field `port` — add `port = <listener_port>`"
    )]
    TcpProbeMissingPort {
        /// 0-indexed position within the per-role array.
        probe_idx: usize,
    },

    /// `timeout_seconds = 0` is rejected — probe attempts MUST have a
    /// non-zero timeout per ADR-0057 §3.
    #[error(
        "[[health_check.*]][{probe_idx}]: field `timeout_seconds` must be > 0 — set a positive value or omit the field to inherit the ADR-0057 default of 5"
    )]
    ProbeTimeoutZero { probe_idx: usize },

    /// `interval_seconds = 0` is rejected — probes MUST tick at a
    /// non-zero cadence per ADR-0057 §3.
    #[error(
        "[[health_check.*]][{probe_idx}]: field `interval_seconds` must be > 0 — set a positive value or omit the field to inherit the ADR-0057 default of 2 (startup/readiness) / 10 (liveness)"
    )]
    ProbeIntervalZero { probe_idx: usize },

    /// `max_attempts = 0` on a startup probe is rejected per ADR-0057
    /// §3 (startup-only field).
    #[error(
        "[[health_check.startup]][{probe_idx}]: field `max_attempts` must be > 0 — set a positive value or omit the field to inherit the ADR-0057 default of 30"
    )]
    ProbeMaxAttemptsZero { probe_idx: usize },

    /// `type = "<value>"` is not one of the recognised mechanics
    /// (`tcp` for step 01-02; `http` and `exec` land in later slices).
    #[error(
        "[[health_check.*]][{probe_idx}]: unknown probe type `{found}` (supported types: tcp; http and exec land in later slices)"
    )]
    UnknownProbeType {
        probe_idx: usize,
        /// Verbatim operator-supplied `type` token.
        found: String,
    },

    // -----------------------------------------------------------------
    // Step 02-01 — HTTP probe variant per ADR-0057 §2 / US-02.
    // -----------------------------------------------------------------
    /// `[[health_check.startup]]` with `type = "http"` is missing the
    /// required `path` field. `probe_idx` is the 0-indexed position
    /// within the per-role array.
    #[error(
        "[[health_check.startup]][{probe_idx}]: http probe is missing required field `path` — add `path = \"/healthz\"`"
    )]
    HttpProbeMissingPath {
        /// 0-indexed position within the per-role array.
        probe_idx: usize,
    },

    /// An HTTP probe carries an `https://` URL. Phase 1 supports plain
    /// HTTP only per ADR-0057 C6; HTTPS / mTLS / gRPC are deferred to
    /// Phase 3+. `probe_idx` is the 0-indexed position within the
    /// per-role array.
    #[error(
        "[[health_check.startup]][{probe_idx}]: https:// URLs are not supported in Phase 1 (plain HTTP only per ADR-0057 C6) — use a plain `path` like `/healthz`"
    )]
    HttpsNotSupported {
        /// 0-indexed position within the per-role array.
        probe_idx: usize,
    },
}

// ---------------------------------------------------------------------------
// Discriminator
// ---------------------------------------------------------------------------

/// Three-way kind discriminator. Mirrors the variant tags of
/// [`WorkloadSpec`] and [`WorkloadSpecInput`].
///
/// `Default == Service` per ADR-0037 Amendment 2026-05-10 / ADR-0047 §1:
/// before slice 02-04 the reconciler was kind-agnostic and emulated the
/// Service shape (long-running, restart-budget-driven). Defaulting to
/// `Service` preserves that behavior at every call site that has not
/// yet been wired through to populate the kind explicitly.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    utoipa::ToSchema,
)]
pub enum WorkloadKind {
    /// Long-running supervised workload — a `[service]` body in TOML.
    #[default]
    Service,
    /// Run-to-completion workload — a `[job]` body in TOML.
    Job,
    /// Cron-scheduled job — `[job] + [schedule]` co-presence in TOML.
    Schedule,
}

impl WorkloadKind {
    /// Single-byte discriminator written to / read from the
    /// `workloads/<id>/kind` intent record per
    /// [`crate::aggregate::IntentKey::for_workload_kind`]. The byte is the
    /// canonical persisted form — readable in hex dumps, parseable
    /// without rkyv, and stable across future variant additions
    /// (`Self::default()` is the back-compat fallback for an unknown
    /// byte, preserving the kind-agnostic Service shape).
    #[must_use]
    pub const fn discriminator_byte(self) -> u8 {
        match self {
            Self::Service => b's',
            Self::Job => b'j',
            Self::Schedule => b'c',
        }
    }

    /// Inverse of [`Self::discriminator_byte`]. Unknown bytes default to
    /// `Self::Service` per ADR-0047 §1 — preserves kind-agnostic
    /// behavior at any consumer site reading a forward-compatible byte
    /// it does not yet recognise.
    #[must_use]
    pub const fn from_discriminator_byte(byte: u8) -> Self {
        match byte {
            b'j' => Self::Job,
            b'c' => Self::Schedule,
            _ => Self::Service,
        }
    }

    /// Canonical lowercase string form. Used as the wire-side
    /// `workload_kind` field on `SubmitWorkloadRequest` so legacy JSON-
    /// inspecting clients see a human-readable value, and as the
    /// inverse of [`Self::from_wire_str`].
    #[must_use]
    pub const fn wire_str(self) -> &'static str {
        match self {
            Self::Service => "service",
            Self::Job => "job",
            Self::Schedule => "schedule",
        }
    }

    /// Parse the wire-side string form. Unknown values fall back to
    /// `Self::Service` per ADR-0047 §1 forward-compat (a client may
    /// send a value the server does not yet recognise; preserve
    /// kind-agnostic behavior rather than fail).
    #[must_use]
    pub fn from_wire_str(s: &str) -> Self {
        match s {
            "job" => Self::Job,
            "schedule" => Self::Schedule,
            _ => Self::Service,
        }
    }
}

// ---------------------------------------------------------------------------
// Cron expression newtype (Phase-1 string-shaped; #166 tracks richer validation)
// ---------------------------------------------------------------------------

/// Cron expression carried on a [`ScheduleSpec`].
///
/// Phase-1 validation is "non-empty after trim". Richer syntax
/// validation (5-field vs 7-field, range checks, alias expansion) is
/// tracked under [GH #166](https://github.com/overdrive-sh/overdrive/issues/166).
/// Slice 05 will land semantic parsing — until then the field is a
/// honest String wrapper that preserves operator input verbatim.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    utoipa::ToSchema,
)]
#[serde(transparent)]
pub struct CronExpr(String);

impl CronExpr {
    /// Validating constructor. Returns `Err` if the input is empty after
    /// trim. Casing and interior whitespace are preserved verbatim.
    pub fn new(raw: impl Into<String>) -> Result<Self, ParseError> {
        let raw = raw.into();
        if raw.trim().is_empty() {
            return Err(ParseError::MissingCron);
        }
        Ok(Self(raw))
    }

    /// Borrow the cron expression as `&str`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CronExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// Inner shape — exec / resources (wire-side twins for the parser)
// ---------------------------------------------------------------------------

/// Wire-side `[exec]` block. Mirrors `aggregate::ExecInput` in shape,
/// kept private to the new parser surface to avoid coupling to the
/// legacy aggregate path while Slice 01 ships the discriminator
/// additively.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    utoipa::ToSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ExecInput {
    pub command: String,
    pub args: Vec<String>,
}

/// Wire-side `[resources]` block.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    utoipa::ToSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ResourcesInput {
    pub cpu_milli: u32,
    pub memory_bytes: u64,
}

// ---------------------------------------------------------------------------
// Service listener types — Slice 06 of `workload-kind-discriminator`.
// Per ADR-0047 §1 + #164 converged decisions:
//   * `[[listener]]` is a top-level array-of-tables alongside [service]
//     (NOT nested inside [service]).
//   * The Listener carries the existing `overdrive-core::Proto` newtype —
//     NOT a second `Protocol` enum.
//   * `ServiceVip` is the canonical newtype at
//     `overdrive_core::id::ServiceVip` (wraps `std::net::IpAddr`).
//     Phase 1 admits IPv4 only per ADR-0049 § 5; the type's IPv6 capacity
//     is forward-compat for GH #155. Listeners parse IPv4 strings at
//     the TOML boundary and wrap into the canonical newtype.
// ---------------------------------------------------------------------------

pub use crate::id::ServiceVip;

/// A single `[[listener]]` block on a `[service]` body.
///
/// Per ADR-0047 §1 (Service listener fields) and ADR-0049 § 5 the
/// listener identity is the `(port, protocol)` pair — `port` is
/// non-zero (rejected at parse time per S-08-06) and `protocol` is
/// `tcp` / `udp` only via the existing [`Proto`] newtype
/// (case-insensitive parse, lowercase canonical render).
///
/// The operator-supplied `vip` field was removed in
/// `service-vip-allocator` step 02-01: VIPs are platform-issued via
/// `ServiceVipAllocator` keyed by `spec_digest` and are structurally
/// unrepresentable in the operator-facing spec. The parser rejects
/// any TOML carrying `vip` on a `[[listener]]` block with a typed
/// [`ParseError::UnknownField`] variant.
///
/// Distinct from the dataplane-layer `Backend` per design Reuse
/// Analysis — the spec-layer `Listener` is the OPERATOR-DECLARED
/// intent, while a `Backend` is the dataplane's per-replica realised
/// endpoint. The two carry the same [`Proto`] newtype but live in
/// different bounded contexts.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    utoipa::ToSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Listener {
    /// Listener port — 1..=65535. `port = 0` is rejected at parse time
    /// per S-08-06 (`ParseError::ListenerPortZero`).
    #[schema(value_type = u16, minimum = 1, maximum = 65535)]
    pub port: NonZeroU16,
    /// L4 protocol — `tcp` or `udp` only. Case-insensitive at parse
    /// time; lowercase on canonical render.
    pub protocol: Proto,
}

// ---------------------------------------------------------------------------
// Per-kind specs
// ---------------------------------------------------------------------------

// `ServiceSpec` (= `ServiceSpecV2`) lives in
// `crate::aggregate::service_spec`. Per ADR-0057 step 01-02 the type
// carries three `Vec<ProbeDescriptor>` fields (startup / readiness /
// liveness) and is wrapped by `ServiceSpecEnvelope`. We re-import here
// so the parser-side enum types (`WorkloadSpec`, `WorkloadSpecInput`)
// continue to use the bare `ServiceSpec` name unchanged.
pub use crate::aggregate::service_spec::ServiceSpec;

/// Validated `[job]` body. `replicas` is intentionally absent — Job is
/// run-to-completion per ADR-0047 §1.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    utoipa::ToSchema,
)]
pub struct JobSpec {
    pub id: String,
    pub exec: ExecInput,
    pub resources: ResourcesInput,
}

/// Validated `[job] + [schedule]` body. The schedule's inner job is the
/// same shape as a standalone Job; the cron expression is the
/// schedule-only addition.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    utoipa::ToSchema,
)]
pub struct ScheduleSpec {
    pub job_inner: JobSpec,
    pub cron_expr: CronExpr,
}

// ---------------------------------------------------------------------------
// Aggregate
// ---------------------------------------------------------------------------

/// The `WorkloadSpec` aggregate — Slice 01 of
/// `workload-kind-discriminator`. Carries the parsed-and-validated
/// operator declaration, kind-discriminated.
///
/// Per ADR-0047 §1 a tagged enum, NOT three independent types. Future
/// kinds (`Function` for FaaS, `MicroVm`, `Wasm`) append as new variants
/// without changing existing variants.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub enum WorkloadSpec {
    Service(ServiceSpec),
    Job(JobSpec),
    Schedule(ScheduleSpec),
}

// ---------------------------------------------------------------------------
// Wire-shape input (custom Deserialize on `from_toml_str`)
// ---------------------------------------------------------------------------

/// Wire-shape input — what the parser produces from raw TOML before
/// validating constructors apply.
///
/// Per ADR-0047 §2 `WorkloadSpecInput::from_toml_str` is the single
/// driving port for the parser. The custom impl walks the parsed TOML
/// `Value::Table` by section presence and produces typed
/// [`ParseError`]s naming the offending sections.
///
/// The `Deserialize` derive is for completeness — JSON ingress of an
/// already-discriminated `WorkloadSpec` body uses the standard tagged
/// enum form. The TOML lane is the section-presence path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkloadSpecInput {
    Service(ServiceSpec),
    Job(JobSpec),
    Schedule(ScheduleSpec),
}

impl WorkloadSpecInput {
    /// The kind discriminator without unwrapping the inner body.
    #[must_use]
    pub const fn kind(&self) -> WorkloadKind {
        match self {
            Self::Service(_) => WorkloadKind::Service,
            Self::Job(_) => WorkloadKind::Job,
            Self::Schedule(_) => WorkloadKind::Schedule,
        }
    }

    /// Borrow the workload identifier as `&str` regardless of kind.
    /// Convenience for assertions on the kind-discriminator surface.
    #[must_use]
    pub fn id_as_str(&self) -> &str {
        match self {
            Self::Service(s) => &s.id,
            Self::Job(j) => &j.id,
            Self::Schedule(s) => &s.job_inner.id,
        }
    }

    /// Borrow the cron expression as `&str` if kind is `Schedule`.
    #[must_use]
    pub fn cron_expr_str(&self) -> Option<&str> {
        match self {
            Self::Schedule(s) => Some(s.cron_expr.as_str()),
            _ => None,
        }
    }

    /// Borrow the `[exec]` command as `&str` regardless of kind.
    #[must_use]
    pub fn exec_command(&self) -> &str {
        match self {
            Self::Service(s) => &s.exec.command,
            Self::Job(j) => &j.exec.command,
            Self::Schedule(s) => &s.job_inner.exec.command,
        }
    }

    /// Parse a `WorkloadSpecInput` from raw TOML bytes.
    ///
    /// Per ADR-0047 §2 this is the single driving port for the parser.
    /// Section presence is the kind discriminator; mixed-kind specs are
    /// rejected with structured [`ParseError`]s naming the offending
    /// sections.
    ///
    /// # Errors
    ///
    /// Returns `Err(ParseError::*)` for every invalid section
    /// combination per the AC matrix in `slice-01-parser-kind-discriminator.md`:
    /// `[service]+[job]` → `MixedServiceAndJob`; `[schedule]` alone →
    /// `ScheduleWithoutJob`; `[schedule]+[service]` → `ScheduleWithService`;
    /// missing `[exec]` → `MissingExec`; missing `[resources]` →
    /// `MissingResources`; missing `cron` in `[schedule]` →
    /// `MissingCron`; underlying TOML parse failures → `Toml(_)`.
    pub fn from_toml_str(src: &str) -> Result<Self, ParseError> {
        // Parse to a generic TOML value so we can inspect section presence
        // before mapping to the variant. `toml` is a dev-dep on this
        // crate today; per ADR-0047 §2 the parser lives at the
        // overdrive-core boundary so every consumer routes through the
        // same custom Deserialize.
        let value: toml::Value =
            src.parse().map_err(|e: toml::de::Error| ParseError::Toml(e.to_string()))?;
        let table = value
            .as_table()
            .ok_or_else(|| ParseError::Toml("top-level TOML must be a table".to_string()))?;

        let has_service = table.contains_key("service");
        let has_job = table.contains_key("job");
        let has_schedule = table.contains_key("schedule");
        let has_exec = table.contains_key("exec");
        let has_resources = table.contains_key("resources");

        // Kind-discrimination matrix per ADR-0047 §1.
        // Rejection ordering matches the operator-facing-clarity ordering
        // in slice-01-parser-kind-discriminator.md.
        if has_service && has_job {
            return Err(ParseError::MixedServiceAndJob);
        }
        if has_schedule && has_service {
            return Err(ParseError::ScheduleWithService);
        }
        if has_schedule && !has_job {
            return Err(ParseError::ScheduleWithoutJob);
        }
        if !has_service && !has_job {
            return Err(ParseError::MissingKindSection);
        }
        if !has_exec {
            return Err(ParseError::MissingExec);
        }
        if !has_resources {
            return Err(ParseError::MissingResources);
        }

        // Inner-section deserialisation. Each section is parsed into its
        // typed shape; failures map to ParseError::Field with the
        // section name.
        let exec: ExecInput = parse_section(table, "exec")?;
        let resources: ResourcesInput = parse_section(table, "resources")?;

        if has_service {
            // [service] body fields directly under the [service] table.
            let svc_table =
                table.get("service").and_then(toml::Value::as_table).ok_or_else(|| {
                    ParseError::Field {
                        section: "[service]",
                        message: "must be a table".to_string(),
                    }
                })?;
            let id = parse_string_field(svc_table, "id", "[service]")?;
            let replicas = parse_u32_field_default(svc_table, "replicas", 1, "[service]")?;
            // [[listener]] is a top-level array-of-tables ALONGSIDE
            // [service] (NOT nested under it) per #164 converged
            // decision. Walk the top-level table for a `listener` key
            // whose value is an array.
            let listeners = parse_listeners(table)?;

            // Step 01-02 — ADR-0057 §1 [[health_check.startup]] TCP
            // variant + ADR-0058 default-inference. Discover the
            // `health_check.startup` value (absent vs explicit-empty
            // vs populated) and either parse declared probes or
            // synthesise the default TCP probe against `listeners[0]`.
            let (startup_probes, startup_was_explicit) = parse_startup_probes(table, &listeners)?;
            // Readiness/liveness population is owned by later slices
            // (02-01 / 02-02). For this step they remain empty —
            // ServiceSpecV2 envelope shape is stable across slices.
            let _ = startup_was_explicit;

            return Ok(Self::Service(ServiceSpec {
                id,
                replicas,
                exec,
                resources,
                listeners,
                startup_probes,
                readiness_probes: vec![],
                liveness_probes: vec![],
            }));
        }

        // Job-shaped path (with or without [schedule]).
        let job_table = table.get("job").and_then(toml::Value::as_table).ok_or_else(|| {
            ParseError::Field { section: "[job]", message: "must be a table".to_string() }
        })?;
        let id = parse_string_field(job_table, "id", "[job]")?;
        let job_inner = JobSpec { id, exec, resources };

        if has_schedule {
            let sched_table =
                table.get("schedule").and_then(toml::Value::as_table).ok_or_else(|| {
                    ParseError::Field {
                        section: "[schedule]",
                        message: "must be a table".to_string(),
                    }
                })?;
            let cron_raw = parse_string_field(sched_table, "cron", "[schedule]")
                .map_err(|_| ParseError::MissingCron)?;
            let cron_expr = CronExpr::new(cron_raw)?;
            return Ok(Self::Schedule(ScheduleSpec { job_inner, cron_expr }));
        }

        Ok(Self::Job(job_inner))
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Deserialise a top-level TOML section into a typed shape, mapping
/// failures to `ParseError::Field` with the section name.
fn parse_section<T>(table: &toml::value::Table, name: &'static str) -> Result<T, ParseError>
where
    T: serde::de::DeserializeOwned,
{
    let value = table.get(name).ok_or_else(|| ParseError::Field {
        section: section_label(name),
        message: format!("missing required [{name}] section"),
    })?;
    let cloned = value.clone();
    cloned
        .try_into::<T>()
        .map_err(|e| ParseError::Field { section: section_label(name), message: e.to_string() })
}

/// Pull a required `String`-typed field out of a TOML table.
fn parse_string_field(
    table: &toml::value::Table,
    field: &str,
    section: &'static str,
) -> Result<String, ParseError> {
    table
        .get(field)
        .ok_or_else(|| ParseError::Field {
            section,
            message: format!("required field `{field}` is missing"),
        })
        .and_then(|v| {
            v.as_str().map(str::to_owned).ok_or_else(|| ParseError::Field {
                section,
                message: format!("field `{field}` must be a string"),
            })
        })
}

/// Pull an optional `u32`-typed field out of a TOML table, defaulting
/// to `default` when absent.
fn parse_u32_field_default(
    table: &toml::value::Table,
    field: &str,
    default: u32,
    section: &'static str,
) -> Result<u32, ParseError> {
    table.get(field).map_or(Ok(default), |v| {
        v.as_integer().and_then(|i| u32::try_from(i).ok()).ok_or_else(|| ParseError::Field {
            section,
            message: format!("field `{field}` must be a non-negative integer fitting in u32"),
        })
    })
}

/// Map an internal section identifier (`exec`, `resources`, `service`,
/// `job`, `schedule`) to its operator-facing display label
/// (`[exec]`, `[resources]`, `[service]`, `[job]`, `[schedule]`).
const fn section_label(name: &str) -> &'static str {
    match name.as_bytes() {
        b"exec" => "[exec]",
        b"resources" => "[resources]",
        b"service" => "[service]",
        b"job" => "[job]",
        b"schedule" => "[schedule]",
        _ => "<unknown section>",
    }
}

/// Parse the top-level `[[listener]]` array-of-tables and validate per
/// Slice 06 of `workload-kind-discriminator`.
///
/// Validation rules per `test-scenarios.md` §8:
/// * MUST be non-empty (`ParseError::ListenerMissing`).
/// * No two entries share `(vip, port, protocol)` — comparison falls
///   back to `(port, protocol)` only when both vips are `None`.
/// * `port` is non-zero (`ParseError::ListenerPortZero`).
/// * `protocol` is `tcp` / `udp` only (case-insensitive parse).
fn parse_listeners(table: &toml::value::Table) -> Result<Vec<Listener>, ParseError> {
    let arr = table.get("listener").map_or(Ok::<&[toml::Value], ParseError>(&[]), |v| {
        v.as_array().map(std::vec::Vec::as_slice).ok_or_else(|| ParseError::Field {
            section: "[[listener]]",
            message: "must be an array of tables".to_string(),
        })
    })?;

    if arr.is_empty() {
        return Err(ParseError::ListenerMissing);
    }

    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        let entry_table = entry.as_table().ok_or_else(|| ParseError::Field {
            section: "[[listener]]",
            message: "each [[listener]] entry must be a table".to_string(),
        })?;
        let listener = parse_one_listener(entry_table)?;
        out.push(listener);
    }

    // Uniqueness check on the (port, protocol) pair. Per ADR-0049 § 5
    // / service-vip-allocator step 02-01 the `vip` axis was removed —
    // VIPs are platform-issued at the service level, so two listeners
    // sharing the same (port, protocol) on the same Service are
    // always a duplicate regardless of any (non-existent) per-listener
    // VIP.
    for i in 0..out.len() {
        for j in (i + 1)..out.len() {
            let a = &out[i];
            let b = &out[j];
            if a.port == b.port && a.protocol == b.protocol {
                let pair = format_listener_pair(*a);
                return Err(ParseError::ListenerDuplicate { triple: pair });
            }
        }
    }

    Ok(out)
}

/// Parse a single `[[listener]]` entry into a [`Listener`]. Caller is
/// responsible for the array-level validation; this fn handles the
/// per-entry field-shape validation only.
fn parse_one_listener(entry: &toml::value::Table) -> Result<Listener, ParseError> {
    // port — required integer in 1..=65535.
    let port_raw = entry.get("port").ok_or_else(|| ParseError::Field {
        section: "[[listener]]",
        message: "required field `port` is missing".to_string(),
    })?;
    let port_int = port_raw.as_integer().ok_or_else(|| ParseError::Field {
        section: "[[listener]]",
        message: "field `port` must be an integer".to_string(),
    })?;
    if port_int == 0 {
        return Err(ParseError::ListenerPortZero);
    }
    let port_u16 = u16::try_from(port_int).map_err(|_| ParseError::Field {
        section: "[[listener]]",
        message: "field `port` must be in 1..=65535".to_string(),
    })?;
    let port = NonZeroU16::new(port_u16).ok_or(ParseError::ListenerPortZero)?;

    // protocol — required string, case-insensitive `tcp` / `udp`.
    let proto_raw = entry.get("protocol").ok_or_else(|| ParseError::Field {
        section: "[[listener]]",
        message: "required field `protocol` is missing".to_string(),
    })?;
    let proto_str = proto_raw.as_str().ok_or_else(|| ParseError::Field {
        section: "[[listener]]",
        message: "field `protocol` must be a string".to_string(),
    })?;
    let protocol = match proto_str.to_ascii_lowercase().as_str() {
        "tcp" => Proto::Tcp,
        "udp" => Proto::Udp,
        _ => {
            return Err(ParseError::ListenerUnsupportedProtocol { value: proto_str.to_string() });
        }
    };

    // Reject unknown fields per ADR-0049 § 5 / service-vip-allocator
    // step 02-01. The operator-supplied `vip` field was removed from
    // [`Listener`]; the parser-level rejection makes it structurally
    // unrepresentable. Any unknown field surfaces with a typed
    // [`ParseError::UnknownField`] whose `Display` form names the
    // offending field AND tells the operator to remove it.
    //
    // We special-case `vip` for the targeted guidance text; other
    // unknown fields share the same variant but fall back to the
    // generic message via the `Display` impl.
    for key in entry.keys() {
        if !matches!(key.as_str(), "port" | "protocol") {
            return Err(ParseError::UnknownField { section: "[[listener]]", field: key.clone() });
        }
    }

    Ok(Listener { port, protocol })
}

/// Render a listener `(port, protocol)` pair for diagnostic messages.
/// Used by [`ParseError::ListenerDuplicate`]. Per ADR-0049 § 5 the
/// per-listener `vip` axis was removed; the diagnostic surface still
/// uses the `triple` field name on the variant for source compat with
/// existing callers, but the rendered form names only the two-axis
/// pair.
fn format_listener_pair(l: Listener) -> String {
    format!("(port={}, protocol={})", l.port.get(), l.protocol)
}

// ---------------------------------------------------------------------------
// Step 01-02 — [[health_check.startup]] (TCP only) parsing + ADR-0058
// default-inference. HTTP / Exec mechanics land in slices 02-01 / 02-02.
// ---------------------------------------------------------------------------

use crate::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use crate::observation::ProbeRole;

/// ADR-0057 §2 default values for a startup probe — operator omits
/// `timeout_seconds` / `interval_seconds` / `max_attempts` → these apply.
const STARTUP_TIMEOUT_DEFAULT_S: u32 = 5;
const STARTUP_INTERVAL_DEFAULT_S: u32 = 2;
const STARTUP_MAX_ATTEMPTS_DEFAULT: u32 = 30;

/// Discover and parse `[[health_check.startup]]` per ADR-0057 §1 +
/// apply the ADR-0058 default-inference rule.
///
/// Returns `(probes, was_explicit)` where `was_explicit` is `true` iff
/// the operator wrote `[[health_check.startup]]` (whether as an array
/// of tables OR `health_check.startup = []` — both shapes are
/// "explicit"). When the operator omits the section entirely AND the
/// service has at least one listener, the parser synthesises a single
/// default TCP probe per ADR-0058.
///
/// Per DDD-16 the empty-array shape is the explicit opt-out: zero
/// probes survive (preserves Phase-1 first-Running semantics).
fn parse_startup_probes(
    table: &toml::value::Table,
    listeners: &[Listener],
) -> Result<(Vec<ProbeDescriptor>, bool), ParseError> {
    // The TOML shape is `[[health_check.startup]]` (array of tables) or
    // `health_check.startup = []` (explicit-empty literal). Both
    // descend through a nested-table chain: `health_check.startup` is
    // a sub-key on a `health_check` table when written via the array-
    // of-tables shape, AND when written via the inline-array shape.
    let (startup_value_opt, was_explicit) = table
        .get("health_check")
        .and_then(toml::Value::as_table)
        .and_then(|hc| hc.get("startup"))
        .map_or((None, false), |v| (Some(v.clone()), true));

    let entries: Vec<&toml::value::Table> = match startup_value_opt.as_ref() {
        None => Vec::new(),
        Some(v) => v.as_array().map_or_else(
            || {
                Err(ParseError::Field {
                    section: "[[health_check.startup]]",
                    message: "must be an array of tables (or `health_check.startup = []` for the explicit-empty opt-out)".to_string(),
                })
            },
            |arr| {
                arr.iter()
                    .map(|entry| {
                        entry.as_table().ok_or_else(|| ParseError::Field {
                            section: "[[health_check.startup]]",
                            message: "each entry must be a table".to_string(),
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            },
        )?,
    };

    // Default-inference per ADR-0058: zero probes declared AND no
    // explicit shape AND at least one listener -> synthesise default.
    if !was_explicit && !listeners.is_empty() {
        let first = &listeners[0];
        let inferred = ProbeDescriptor {
            role: ProbeRole::Startup,
            mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_string(), port: first.port.get() },
            timeout_seconds: STARTUP_TIMEOUT_DEFAULT_S,
            interval_seconds: STARTUP_INTERVAL_DEFAULT_S,
            max_attempts: STARTUP_MAX_ATTEMPTS_DEFAULT,
            failure_threshold: None,
            success_threshold: None,
            inferred: true,
        };
        return Ok((vec![inferred], false));
    }

    // Parse each declared entry into a ProbeDescriptor.
    let mut out = Vec::with_capacity(entries.len());
    for (probe_idx, entry) in entries.iter().enumerate() {
        out.push(parse_one_startup_probe(entry, probe_idx)?);
    }
    Ok((out, was_explicit))
}

/// Parse one `[[health_check.startup]]` entry. TCP variant only this
/// step; HTTP / Exec land in slices 02-01 / 02-02.
fn parse_one_startup_probe(
    entry: &toml::value::Table,
    probe_idx: usize,
) -> Result<ProbeDescriptor, ParseError> {
    // type field — case-insensitive per § Newtype completeness.
    let type_raw = entry.get("type").ok_or_else(|| ParseError::Field {
        section: "[[health_check.startup]]",
        message: format!("entry [{probe_idx}]: required field `type` is missing"),
    })?;
    let type_str = type_raw.as_str().ok_or_else(|| ParseError::Field {
        section: "[[health_check.startup]]",
        message: format!("entry [{probe_idx}]: field `type` must be a string"),
    })?;

    let mechanic = match type_str.to_ascii_lowercase().as_str() {
        "tcp" => {
            let port_raw =
                entry.get("port").ok_or(ParseError::TcpProbeMissingPort { probe_idx })?;
            let port_int = port_raw.as_integer().ok_or_else(|| ParseError::Field {
                section: "[[health_check.startup]]",
                message: format!("entry [{probe_idx}]: field `port` must be an integer"),
            })?;
            let port_u16 = u16::try_from(port_int).map_err(|_| ParseError::Field {
                section: "[[health_check.startup]]",
                message: format!("entry [{probe_idx}]: field `port` must be in 1..=65535"),
            })?;
            if port_u16 == 0 {
                return Err(ParseError::TcpProbeMissingPort { probe_idx });
            }
            let host =
                entry.get("host").and_then(toml::Value::as_str).unwrap_or("0.0.0.0").to_string();
            ProbeMechanic::Tcp { host, port: port_u16 }
        }
        "http" => parse_http_mechanic(entry, probe_idx)?,
        other => {
            return Err(ParseError::UnknownProbeType { probe_idx, found: other.to_string() });
        }
    };

    let timeout_seconds =
        parse_optional_positive_u32(entry, "timeout_seconds", STARTUP_TIMEOUT_DEFAULT_S, probe_idx)
            .map_err(|err| map_zero_to_named_error(err, "timeout_seconds", probe_idx))?;
    let interval_seconds = parse_optional_positive_u32(
        entry,
        "interval_seconds",
        STARTUP_INTERVAL_DEFAULT_S,
        probe_idx,
    )
    .map_err(|err| map_zero_to_named_error(err, "interval_seconds", probe_idx))?;
    let max_attempts =
        parse_optional_positive_u32(entry, "max_attempts", STARTUP_MAX_ATTEMPTS_DEFAULT, probe_idx)
            .map_err(|err| map_zero_to_named_error(err, "max_attempts", probe_idx))?;

    Ok(ProbeDescriptor {
        role: ProbeRole::Startup,
        mechanic,
        timeout_seconds,
        interval_seconds,
        max_attempts,
        failure_threshold: None,
        success_threshold: None,
        inferred: false,
    })
}

/// Parse the `type = "http"` mechanic body per ADR-0057 §2 / US-02.
///
/// Required fields: `path` (absolute request path) and `port`.
/// Optional: `host` (defaults to `0.0.0.0` at probe time, carried as
/// `None` through parse). Phase 1 is plain HTTP only — any `https://`
/// scheme in `path` is rejected with
/// [`ParseError::HttpsNotSupported`] per ADR-0057 C6.
///
/// Edge cases:
/// - `path` absent → [`ParseError::HttpProbeMissingPath`].
/// - `path` containing `https://` → [`ParseError::HttpsNotSupported`]
///   (checked BEFORE the plain-`http://` strip so an `https://` URL
///   pasted as the path is never silently accepted).
/// - `port` absent / out of `1..=65535` → reuses the TCP `port`
///   diagnostics shape via [`ParseError::TcpProbeMissingPort`] — the
///   port precondition is identical across mechanics.
fn parse_http_mechanic(
    entry: &toml::value::Table,
    probe_idx: usize,
) -> Result<ProbeMechanic, ParseError> {
    let path_raw = entry.get("path").ok_or(ParseError::HttpProbeMissingPath { probe_idx })?;
    let path = path_raw.as_str().ok_or_else(|| ParseError::Field {
        section: "[[health_check.startup]]",
        message: format!("entry [{probe_idx}]: field `path` must be a string"),
    })?;
    // Phase 1 plain-HTTP-only gate per ADR-0057 C6. Reject any
    // `https://` URL pasted into `path` BEFORE any other path handling.
    if path.contains("https://") {
        return Err(ParseError::HttpsNotSupported { probe_idx });
    }
    if path.is_empty() {
        return Err(ParseError::HttpProbeMissingPath { probe_idx });
    }

    let port_raw = entry.get("port").ok_or(ParseError::TcpProbeMissingPort { probe_idx })?;
    let port_int = port_raw.as_integer().ok_or_else(|| ParseError::Field {
        section: "[[health_check.startup]]",
        message: format!("entry [{probe_idx}]: field `port` must be an integer"),
    })?;
    let port = u16::try_from(port_int).map_err(|_| ParseError::Field {
        section: "[[health_check.startup]]",
        message: format!("entry [{probe_idx}]: field `port` must be in 1..=65535"),
    })?;
    if port == 0 {
        return Err(ParseError::TcpProbeMissingPort { probe_idx });
    }

    let host = entry.get("host").and_then(toml::Value::as_str).map(str::to_owned);
    Ok(ProbeMechanic::Http { path: path.to_owned(), port, host })
}

/// Local intermediate-error variant for the zero-field rejection
/// pipeline. Allows [`parse_optional_positive_u32`] to surface a
/// generic "field is zero" outcome that the caller maps to one of
/// the field-specific named ParseError variants
/// (`ProbeTimeoutZero` / `ProbeIntervalZero` / `ProbeMaxAttemptsZero`).
enum OptionalPositiveU32Error {
    Zero,
    Field(ParseError),
}

/// Pull an optional positive-u32 field from a probe entry, defaulting
/// to `default` when absent. Returns `OptionalPositiveU32Error::Zero`
/// if the operator explicitly wrote `0`; the caller maps to the
/// field-specific named ParseError.
fn parse_optional_positive_u32(
    entry: &toml::value::Table,
    field: &str,
    default: u32,
    probe_idx: usize,
) -> Result<u32, OptionalPositiveU32Error> {
    let Some(value) = entry.get(field) else {
        return Ok(default);
    };
    let int = value.as_integer().ok_or_else(|| {
        OptionalPositiveU32Error::Field(ParseError::Field {
            section: "[[health_check.startup]]",
            message: format!(
                "entry [{probe_idx}]: field `{field}` must be a non-negative integer fitting in u32"
            ),
        })
    })?;
    if int == 0 {
        return Err(OptionalPositiveU32Error::Zero);
    }
    u32::try_from(int).map_err(|_| {
        OptionalPositiveU32Error::Field(ParseError::Field {
            section: "[[health_check.startup]]",
            message: format!(
                "entry [{probe_idx}]: field `{field}` must be a non-negative integer fitting in u32"
            ),
        })
    })
}

/// Map an `OptionalPositiveU32Error::Zero` to the field-specific
/// named variant (`ProbeTimeoutZero` / `ProbeIntervalZero` /
/// `ProbeMaxAttemptsZero`). A `Field` carries through verbatim.
fn map_zero_to_named_error(
    err: OptionalPositiveU32Error,
    field: &str,
    probe_idx: usize,
) -> ParseError {
    match err {
        OptionalPositiveU32Error::Zero => match field {
            "timeout_seconds" => ParseError::ProbeTimeoutZero { probe_idx },
            "interval_seconds" => ParseError::ProbeIntervalZero { probe_idx },
            "max_attempts" => ParseError::ProbeMaxAttemptsZero { probe_idx },
            _ => unreachable!("map_zero_to_named_error only called for the three known fields"),
        },
        OptionalPositiveU32Error::Field(parse_error) => parse_error,
    }
}
