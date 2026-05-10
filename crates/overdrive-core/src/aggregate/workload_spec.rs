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

use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    /// `jobs/<id>/kind` intent record per
    /// [`crate::aggregate::IntentKey::for_job_kind`]. The byte is the
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
    /// `workload_kind` field on `SubmitJobRequest` so legacy JSON-
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
// Per-kind specs
// ---------------------------------------------------------------------------

/// Validated `[service]` body — `id`, `replicas`, `[exec]`, `[resources]`,
/// listeners. Slice 01 lands the type with the minimal field set the
/// parser needs to discriminate kinds; Slice 06 will expand the
/// listener carrier shape.
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
pub struct ServiceSpec {
    pub id: String,
    pub replicas: u32,
    pub exec: ExecInput,
    pub resources: ResourcesInput,
}

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
            return Ok(Self::Service(ServiceSpec { id, replicas, exec, resources }));
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
