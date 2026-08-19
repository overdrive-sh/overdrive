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

    /// Neither `[exec]` nor `[vm]` is present. Replaces the former
    /// `MissingExec` (single cut, ADR-0083 §D4, GH #42).
    #[error("missing required section: exactly one of [exec] or [vm] is required")]
    MissingDriverSection,

    /// Both `[exec]` and `[vm]` are present. Per ADR-0083 §D4, exactly
    /// one driver table is required.
    #[error("both [exec] and [vm] are present; exactly one driver section is required")]
    MultipleDriverSections,

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

    /// `success_threshold = 0` on a readiness probe is rejected per
    /// ADR-0057 §3 — the probe must succeed at least once.
    #[error(
        "[[health_check.readiness]][{probe_idx}]: field `success_threshold` must be > 0 — set a positive value or omit the field to inherit the ADR-0057 default of 1"
    )]
    ProbeSuccessThresholdZero { probe_idx: usize },

    /// `failure_threshold = 0` on a liveness probe is rejected per
    /// ADR-0057 §3 — the probe must tolerate at least one failure
    /// before declaring the workload unhealthy.
    #[error(
        "[[health_check.liveness]][{probe_idx}]: field `failure_threshold` must be > 0 — set a positive value or omit the field to inherit the ADR-0057 default of 3"
    )]
    ProbeFailureThresholdZero { probe_idx: usize },

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

    /// An HTTP probe's `path` is non-empty but does not begin with `/`.
    /// At probe time the path is concatenated into
    /// `http://<host>:<port><path>`; without the leading `/` the URL
    /// collapses to e.g. `http://127.0.0.1:8080health`, which hyper
    /// rejects as an invalid target. That adapter error is swallowed by
    /// the supervised probe loop (warn + continue, no `ProbeResultRow`),
    /// so `startup_attempts` never advances and the startup window never
    /// closes — the service hangs indefinitely. Rejecting at parse time
    /// turns the silent hang into an actionable config error. `probe_idx`
    /// is the 0-indexed position within the per-role array; `path` is the
    /// offending value, verbatim from the operator input.
    #[error(
        "[[health_check.startup]][{probe_idx}]: http probe `path` must be absolute (start with `/`) — got {path:?}; use e.g. `/healthz`"
    )]
    HttpProbePathNotAbsolute {
        /// 0-indexed position within the per-role array.
        probe_idx: usize,
        /// The offending `path` value, verbatim from the operator input.
        path: String,
    },

    // -----------------------------------------------------------------
    // Step 02-02 — Exec probe variant per ADR-0057 §2 / US-03.
    // -----------------------------------------------------------------
    /// `[[health_check.startup]]` with `type = "exec"` carries an empty
    /// `command` array (or omits it entirely). An exec probe MUST name
    /// the binary to spawn; `command[0]` is the binary and
    /// `command[1..]` (plus any `args`) are the argv tail per the
    /// `ExecProber` trait contract. `probe_idx` is the 0-indexed
    /// position within the per-role array.
    #[error(
        "[[health_check.startup]][{probe_idx}]: exec probe is missing required field `command` — add a non-empty array like `command = [\"/usr/bin/healthcheck\"]`"
    )]
    ExecProbeMissingCommand {
        /// 0-indexed position within the per-role array.
        probe_idx: usize,
    },

    // -----------------------------------------------------------------
    // Step 03-01 — Slice 07 kind rejection per US-07 / K5.
    // -----------------------------------------------------------------
    /// A `[[health_check.*]]` array (startup / readiness / liveness)
    /// was declared on a non-Service workload (`[job]` or
    /// `[job]+[schedule]`). Per ADR-0054 / ADR-0055 only Service-kind
    /// workloads carry a probe surface — a Job's success criterion IS
    /// its exit code, and a Schedule composes per-fire workloads whose
    /// probes belong on the Service the Schedule fires.
    ///
    /// `kind` is the offending workload kind in canonical lowercase
    /// (`"job"` / `"schedule"`). `guidance` is the per-kind teaching
    /// text from [`crate::aggregate::probe_descriptor`]
    /// (`JOB_PROBES_GUIDANCE` / `SCHEDULE_PROBES_GUIDANCE`) so the
    /// rejection explains *why* rather than merely forbidding.
    #[error("[[health_check.*]] is not allowed on a {kind} workload — {guidance}")]
    ProbesNotAllowedOnKind {
        /// Offending workload kind in canonical lowercase.
        kind: &'static str,
        /// Per-kind guidance text explaining the rejection.
        guidance: &'static str,
    },

    // -----------------------------------------------------------------
    // Slice 02 — microvm-driver-cloud-hypervisor US-VM-6 / AC-10.
    // -----------------------------------------------------------------
    /// `[vm]` was declared on a `[service]` workload. A VM workload
    /// cannot yet be *served*: the platform has no tap in the guest's
    /// netns, no probe that can reach inside the guest, and no
    /// interception point for the guest's own TCP stack — so a spec the
    /// platform accepted here would schedule, report `Running`, and
    /// receive no traffic.
    ///
    /// The mirror image of [`Self::ProbesNotAllowedOnKind`]: there
    /// Service is the kind being *required*, here it is the kind being
    /// *refused*. Same shape by design — a semantic rejection carrying
    /// `&'static str` guidance that explains *why* and names where each
    /// missing capability is tracked, rather than a bare refusal.
    ///
    /// This variant is *removed*, not relaxed, when VM services become
    /// supported.
    #[error("[vm] is not allowed on a [service] workload — {guidance}")]
    VmNotAllowedOnServiceKind {
        /// Guidance text naming the missing capabilities and their
        /// tracking issues — always [`SERVICE_VM_UNSUPPORTED_GUIDANCE`].
        guidance: &'static str,
    },
}

/// Guidance text surfaced on
/// [`ParseError::VmNotAllowedOnServiceKind`] when an operator declares
/// `[vm]` alongside `[service]` (Slice 02 / US-VM-6 / AC-10).
///
/// Names each capability a served VM workload would require and cites
/// the issue tracking it, so the operator learns *what is missing* and
/// *where to watch for it* rather than merely being refused. A per-kind
/// constant (not a format string) so the operator-facing message is
/// stable and greppable — same discipline as
/// [`crate::aggregate::probe_descriptor::JOB_PROBES_GUIDANCE`].
///
/// The three named gaps and their real tracking issues:
///
/// * guest networking + guest-reachable probes —
///   [GH #257](https://github.com/overdrive-sh/overdrive/issues/257)
///   (tap-in-netns provisioning + guest-reachable probes)
/// * guest-stack mTLS interception —
///   [GH #222](https://github.com/overdrive-sh/overdrive/issues/222)
///   (guest-stack mTLS intercept)
const SERVICE_VM_UNSUPPORTED_GUIDANCE: &str = concat!(
    "a VM workload cannot be served yet: guest networking (a tap in the workload's ",
    "netns) and guest-reachable probes are missing, tracked by GH #257; guest-stack ",
    "mTLS interception is missing, tracked by GH #222. Until then, run the [vm] block ",
    "under [job] (optionally with [schedule]), or keep [service] on [exec].",
);

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

/// Wire-side `[vm]` block (ADR-0083 §D4, GH #42). Mirrors
/// `aggregate::VmInput` in shape, kept private to the new parser surface
/// for the same reason `ExecInput` above is — see that type's doc
/// comment.
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
pub struct VmInput {
    /// Command run INSIDE the guest.
    pub command: String,
    /// Argv passed verbatim to the in-guest command.
    pub args: Vec<String>,
    /// Operator-supplied kernel artifact path (BYO).
    pub kernel: String,
    /// Operator-supplied rootfs artifact path (BYO).
    pub rootfs: String,
}

/// Driver-table dispatch on a `[job]` / `[schedule]` body (ADR-0083 §D4,
/// GH #42). `[exec]` and `[vm]` are siblings; the presence-walk in
/// [`WorkloadSpecInput::from_toml_str`] enforces exactly one is present
/// before either variant is constructed.
///
/// This is a SEPARATE type from `aggregate::DriverInput` — see
/// [`ExecInput`]'s doc comment for why this parser surface keeps its own
/// wire-shape family (rkyv-archivable; the legacy `aggregate::DriverInput`
/// is not).
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
#[serde(rename_all = "snake_case")]
pub enum DriverInput {
    Exec(ExecInput),
    Vm(VmInput),
}

impl DriverInput {
    /// Both variants carry a command; borrow it regardless of kind.
    #[must_use]
    pub fn command(&self) -> &str {
        match self {
            Self::Exec(e) => &e.command,
            Self::Vm(v) => &v.command,
        }
    }
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
    /// Driver-table choice (ADR-0083 §D4, GH #42) — `[exec]` or `[vm]`,
    /// exactly one, enforced by [`WorkloadSpecInput::from_toml_str`]'s
    /// presence-walk before this value is ever constructed.
    pub driver: DriverInput,
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

    /// Borrow the driver-table command as `&str` regardless of kind —
    /// `[exec]`'s command for every kind today; `[job]` / `[schedule]`
    /// may instead carry a `[vm]` in-guest command (ADR-0083 §D4, GH
    /// #42). `Service` stays `[exec]`-only.
    #[must_use]
    pub fn exec_command(&self) -> &str {
        match self {
            Self::Service(s) => &s.exec.command,
            Self::Job(j) => j.driver.command(),
            Self::Schedule(s) => s.job_inner.driver.command(),
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
    /// both `[exec]` and `[vm]` present → `MultipleDriverSections`; neither
    /// present → `MissingDriverSection` (ADR-0083 §D4, GH #42 — replaces
    /// the former `MissingExec`, single cut); missing `[resources]` →
    /// `MissingResources`; missing `cron` in `[schedule]` →
    /// `MissingCron`; underlying TOML parse failures → `Toml(_)`.
    ///
    /// `[vm]` is a job-family driver table only (`[job]` / `[schedule]`).
    /// `[service]` keeps its original `[exec]`-only requirement and now
    /// rejects `[vm]` explicitly with `VmNotAllowedOnServiceKind` — the
    /// named AC-10 rejection (microvm-driver US-VM-6), replacing the
    /// former incidental `MissingDriverSection` fall-out.
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

        let SectionPresence { service: has_service, schedule: has_schedule, vm: has_vm } =
            SectionPresence::validated(table)?;

        // Inner-section deserialisation. Each section is parsed into its
        // typed shape; failures map to ParseError::Field with the
        // section name.
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
            // `[service]` stays `[exec]`-only — `[vm]` + `[service]` is
            // rejected upstream by the driver-table dispatch above (this
            // branch is reached only when `has_exec` is true, per the
            // `if has_service { if !has_exec { ... } }` gate).
            let exec: ExecInput = parse_section(table, "exec")?;
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
            let _ = startup_was_explicit;

            // Step 03-01 / Slice 04 — readiness probe section. Reuses
            // the 02-01 (HTTP) / 02-02 (Exec) / 01-02 (TCP) mechanic
            // parse path via `parse_one_role_probe`; sets
            // `role = Readiness` and applies the ADR-0057 §2 /
            // ADR-0055 §6 `success_threshold` default of 1. Absent
            // section → no readiness probes (the backward-compat
            // default: every backend healthy post-Stable, per
            // S-SHCP-RECON-08b). NO default inference for readiness —
            // unlike startup, an omitted readiness section means "no
            // readiness gate", not "synthesise one".
            let readiness_probes = parse_readiness_probes(table)?;
            // Step 03-02 / Slice 05 — liveness probe section. Reuses the
            // same `type`-discriminated mechanic parse path as startup /
            // readiness; sets `role = Liveness` and applies the ADR-0057
            // §2 / DDD-14 `failure_threshold` default of 3. Absent
            // section → no liveness probes (no liveness gate; the
            // reconciler's liveness branch is a no-op for this Service).
            let liveness_probes = parse_liveness_probes(table)?;

            return Ok(Self::Service(ServiceSpec {
                id,
                replicas,
                exec,
                resources,
                listeners,
                startup_probes,
                readiness_probes,
                liveness_probes,
            }));
        }

        // Step 03-01 / Slice 07 — kind rejection (US-07 / K5). A
        // `[[health_check.*]]` array on a non-Service workload is a
        // category error: a Job's success criterion IS its exit code,
        // and a Schedule composes per-fire workloads whose probes
        // belong on the Service the Schedule fires. Reject with the
        // per-kind guidance from `probe_descriptor` so the operator
        // learns *why*. The Service path above never reaches here, so
        // Service-kind probes parse normally (regression guard
        // S-SHCP-PARSE-07 / S-SHCP-CLI-14).
        if table.contains_key("health_check") {
            let (kind, guidance) = if has_schedule {
                ("schedule", crate::aggregate::probe_descriptor::SCHEDULE_PROBES_GUIDANCE)
            } else {
                ("job", crate::aggregate::probe_descriptor::JOB_PROBES_GUIDANCE)
            };
            return Err(ParseError::ProbesNotAllowedOnKind { kind, guidance });
        }

        // Job-shaped path (with or without [schedule]).
        let job_table = table.get("job").and_then(toml::Value::as_table).ok_or_else(|| {
            ParseError::Field { section: "[job]", message: "must be a table".to_string() }
        })?;
        let id = parse_string_field(job_table, "id", "[job]")?;
        // Driver-table dispatch (ADR-0083 §D4, GH #42): the presence-walk
        // above already enforced exactly one of [exec] / [vm] is present.
        let driver: DriverInput = if has_vm {
            DriverInput::Vm(parse_section(table, "vm")?)
        } else {
            DriverInput::Exec(parse_section(table, "exec")?)
        };
        let job_inner = JobSpec { id, driver, resources };

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

/// Which top-level sections a spec declares, once the kind/driver
/// combination has been proven legal.
///
/// Deciding *whether the combination is admissible at all* is a
/// distinct responsibility from deserialising each section's body, so it
/// lives here and [`WorkloadSpecInput::from_toml_str`] reads the verdict.
/// Only the three flags that path still branches on afterwards are
/// carried — the rest are consumed entirely by the validation.
struct SectionPresence {
    /// `[service]` is present (so this is the Service-kind path).
    service: bool,
    /// `[schedule]` is present (so the job-family path is a Schedule).
    schedule: bool,
    /// `[vm]` is the declared driver table rather than `[exec]`.
    vm: bool,
}

impl SectionPresence {
    /// Walk the top-level table and reject every inadmissible section
    /// combination, in the operator-facing-clarity ordering pinned by
    /// `slice-01-parser-kind-discriminator.md`.
    ///
    /// Kind-discrimination matrix per ADR-0047 §1; driver-table dispatch
    /// per ADR-0083 §D4 (GH #42); the `[service]` + `[vm]` rejection per
    /// microvm-driver US-VM-6 / AC-10.
    fn validated(table: &toml::value::Table) -> Result<Self, ParseError> {
        let has_service = table.contains_key("service");
        let has_job = table.contains_key("job");
        let has_schedule = table.contains_key("schedule");
        let has_exec = table.contains_key("exec");
        let has_vm = table.contains_key("vm");

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

        if has_service {
            // Slice 02 / US-VM-6 (AC-10): `[service]` + `[vm]` is a
            // semantic rejection, not a missing-driver one — the driver
            // table IS present; it is the kind that cannot be served.
            // Checked before the `[exec]` requirement so the operator
            // reads what is actually missing (guest networking, guest-
            // reachable probes, guest-stack mTLS interception) rather
            // than being told to add an `[exec]` block they did not
            // forget. Deliberately scoped to `[service]`: the job-family
            // branch below is untouched, so `[job]` / `[job]+[schedule]`
            // with `[vm]` stay accepted.
            if has_vm {
                return Err(ParseError::VmNotAllowedOnServiceKind {
                    guidance: SERVICE_VM_UNSUPPORTED_GUIDANCE,
                });
            }
            if !has_exec {
                return Err(ParseError::MissingDriverSection);
            }
        } else {
            if has_exec && has_vm {
                return Err(ParseError::MultipleDriverSections);
            }
            if !has_exec && !has_vm {
                return Err(ParseError::MissingDriverSection);
            }
        }

        if !table.contains_key("resources") {
            return Err(ParseError::MissingResources);
        }

        Ok(Self { service: has_service, schedule: has_schedule, vm: has_vm })
    }
}

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
use crate::observation::{ProbeIdx, ProbeRole};

/// Project a parser-loop 0-based position into the `ProbeIdx` carried
/// on the emitted [`ProbeDescriptor`], per ADR-0080 § D1.
///
/// The index is **per-role**: each `[[health_check.<role>]]` array
/// restarts at 0, which is exactly the contract ADR-0057:132-134 and
/// `ProbeResultRow::probe_idx`'s docstring state. The saturating cast
/// is unreachable in practice (a TOML array of >4 billion probe
/// entries), and saturating rather than panicking keeps the parser
/// total.
fn probe_idx_from_position(position: usize) -> ProbeIdx {
    ProbeIdx::new(u32::try_from(position).unwrap_or(u32::MAX))
}

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
            // Sole member of the synthesised startup array (ADR-0058).
            idx: ProbeIdx::new(0),
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
    let mechanic = parse_probe_mechanic(entry, probe_idx)?;

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
        idx: probe_idx_from_position(probe_idx),
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

/// Parse the `type`-discriminated mechanic body shared by every role's
/// probe entries (startup / readiness / liveness). Extracted from
/// `parse_one_startup_probe` so the readiness parser (step 03-01 /
/// Slice 04) reuses the exact 01-02 (TCP) / 02-01 (HTTP) / 02-02 (Exec)
/// mechanic parse paths verbatim rather than forking them.
fn parse_probe_mechanic(
    entry: &toml::value::Table,
    probe_idx: usize,
) -> Result<ProbeMechanic, ParseError> {
    // type field — case-insensitive per § Newtype completeness.
    let type_raw = entry.get("type").ok_or_else(|| ParseError::Field {
        section: "[[health_check.*]]",
        message: format!("entry [{probe_idx}]: required field `type` is missing"),
    })?;
    let type_str = type_raw.as_str().ok_or_else(|| ParseError::Field {
        section: "[[health_check.*]]",
        message: format!("entry [{probe_idx}]: field `type` must be a string"),
    })?;

    match type_str.to_ascii_lowercase().as_str() {
        "tcp" => {
            let port_raw =
                entry.get("port").ok_or(ParseError::TcpProbeMissingPort { probe_idx })?;
            let port_int = port_raw.as_integer().ok_or_else(|| ParseError::Field {
                section: "[[health_check.*]]",
                message: format!("entry [{probe_idx}]: field `port` must be an integer"),
            })?;
            let port_u16 = u16::try_from(port_int).map_err(|_| ParseError::Field {
                section: "[[health_check.*]]",
                message: format!("entry [{probe_idx}]: field `port` must be in 1..=65535"),
            })?;
            if port_u16 == 0 {
                return Err(ParseError::TcpProbeMissingPort { probe_idx });
            }
            let host =
                entry.get("host").and_then(toml::Value::as_str).unwrap_or("0.0.0.0").to_string();
            Ok(ProbeMechanic::Tcp { host, port: port_u16 })
        }
        "http" => parse_http_mechanic(entry, probe_idx),
        "exec" => parse_exec_mechanic(entry, probe_idx),
        other => Err(ParseError::UnknownProbeType { probe_idx, found: other.to_string() }),
    }
}

/// Readiness probe `success_threshold` default per ADR-0057 §2 /
/// ADR-0055 §6 / DDD-8 — one consecutive Pass flips `Backend.healthy`
/// true. Operator-configurable upward.
const READINESS_SUCCESS_THRESHOLD_DEFAULT: u32 = 1;
/// Readiness probe `interval_seconds` default per ADR-0057 §2.
const READINESS_INTERVAL_DEFAULT_S: u32 = 2;

/// Discover and parse `[[health_check.readiness]]` per ADR-0057 §2 /
/// Slice 04. Reuses the role-agnostic [`parse_probe_mechanic`] for the
/// TCP/HTTP/Exec body (no fork of the 01-02/02-01/02-02 paths).
///
/// Unlike startup (which synthesises a default TCP probe per ADR-0058
/// when absent), readiness has NO default-inference: an omitted
/// `[[health_check.readiness]]` section means "no readiness gate", so
/// every backend is `healthy = true` post-Stable (S-SHCP-RECON-08b).
/// Sets `role = Readiness`, `success_threshold = Some(1)` default
/// (configurable upward), and `max_attempts` carried as the parsed /
/// default value (readiness is continuous; `max_attempts` is not a
/// readiness gate but the field is shared on `ProbeDescriptor`).
fn parse_readiness_probes(table: &toml::value::Table) -> Result<Vec<ProbeDescriptor>, ParseError> {
    let readiness_value_opt = table
        .get("health_check")
        .and_then(toml::Value::as_table)
        .and_then(|hc| hc.get("readiness"))
        .cloned();

    let entries: Vec<&toml::value::Table> = match readiness_value_opt.as_ref() {
        None => Vec::new(),
        Some(v) => v.as_array().map_or_else(
            || {
                Err(ParseError::Field {
                    section: "[[health_check.readiness]]",
                    message:
                        "must be an array of tables (or omit the section for no readiness gate)"
                            .to_string(),
                })
            },
            |arr| {
                arr.iter()
                    .map(|entry| {
                        entry.as_table().ok_or_else(|| ParseError::Field {
                            section: "[[health_check.readiness]]",
                            message: "each entry must be a table".to_string(),
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            },
        )?,
    };

    let mut out = Vec::with_capacity(entries.len());
    for (probe_idx, entry) in entries.iter().enumerate() {
        let mechanic = parse_probe_mechanic(entry, probe_idx)?;
        let timeout_seconds = parse_optional_positive_u32(
            entry,
            "timeout_seconds",
            STARTUP_TIMEOUT_DEFAULT_S,
            probe_idx,
        )
        .map_err(|err| map_zero_to_named_error(err, "timeout_seconds", probe_idx))?;
        let interval_seconds = parse_optional_positive_u32(
            entry,
            "interval_seconds",
            READINESS_INTERVAL_DEFAULT_S,
            probe_idx,
        )
        .map_err(|err| map_zero_to_named_error(err, "interval_seconds", probe_idx))?;
        let success_threshold = parse_optional_positive_u32(
            entry,
            "success_threshold",
            READINESS_SUCCESS_THRESHOLD_DEFAULT,
            probe_idx,
        )
        .map_err(|err| map_zero_to_named_error(err, "success_threshold", probe_idx))?;

        out.push(ProbeDescriptor {
            idx: probe_idx_from_position(probe_idx),
            role: ProbeRole::Readiness,
            mechanic,
            timeout_seconds,
            interval_seconds,
            // Readiness is continuous; `max_attempts` is not a readiness
            // gate. Carry the startup default so the shared field has a
            // sane value (the readiness reconcile branch never reads it).
            max_attempts: STARTUP_MAX_ATTEMPTS_DEFAULT,
            failure_threshold: None,
            success_threshold: Some(success_threshold),
            inferred: false,
        });
    }
    Ok(out)
}

/// Liveness probe `failure_threshold` default per ADR-0057 §2 /
/// ADR-0055 §7 / DDD-14 — three consecutive Fails on a Running alloc
/// trigger `Action::RestartAllocation`. Operator-configurable.
const LIVENESS_FAILURE_THRESHOLD_DEFAULT: u32 = 3;
/// Liveness probe `interval_seconds` default per ADR-0057 §2.
const LIVENESS_INTERVAL_DEFAULT_S: u32 = 2;

/// Discover and parse `[[health_check.liveness]]` per ADR-0057 §2 /
/// Slice 05 (step 03-02). Reuses the role-agnostic
/// [`parse_probe_mechanic`] for the TCP/HTTP/Exec body (no fork of the
/// 01-02 / 02-01 / 02-02 paths).
///
/// Like readiness (and unlike startup), liveness has NO
/// default-inference: an omitted `[[health_check.liveness]]` section
/// means "no liveness gate", so the reconciler never restarts on
/// liveness for this Service. Sets `role = Liveness`,
/// `failure_threshold = Some(3)` default (configurable), and carries
/// the startup default for `max_attempts` (liveness is continuous;
/// `max_attempts` is not a liveness gate but the field is shared on
/// `ProbeDescriptor`).
///
/// Liveness probes are app-internal only per research §6.2 best
/// practice 1 / C11 — the parser reuses the same `type`-discriminated
/// mechanic body as every other role (no dependency-checking external
/// service surface).
fn parse_liveness_probes(table: &toml::value::Table) -> Result<Vec<ProbeDescriptor>, ParseError> {
    let liveness_value_opt = table
        .get("health_check")
        .and_then(toml::Value::as_table)
        .and_then(|hc| hc.get("liveness"))
        .cloned();

    let entries: Vec<&toml::value::Table> = match liveness_value_opt.as_ref() {
        None => Vec::new(),
        Some(v) => v.as_array().map_or_else(
            || {
                Err(ParseError::Field {
                    section: "[[health_check.liveness]]",
                    message:
                        "must be an array of tables (or omit the section for no liveness gate)"
                            .to_string(),
                })
            },
            |arr| {
                arr.iter()
                    .map(|entry| {
                        entry.as_table().ok_or_else(|| ParseError::Field {
                            section: "[[health_check.liveness]]",
                            message: "each entry must be a table".to_string(),
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            },
        )?,
    };

    let mut out = Vec::with_capacity(entries.len());
    for (probe_idx, entry) in entries.iter().enumerate() {
        let mechanic = parse_probe_mechanic(entry, probe_idx)?;
        let timeout_seconds = parse_optional_positive_u32(
            entry,
            "timeout_seconds",
            STARTUP_TIMEOUT_DEFAULT_S,
            probe_idx,
        )
        .map_err(|err| map_zero_to_named_error(err, "timeout_seconds", probe_idx))?;
        let interval_seconds = parse_optional_positive_u32(
            entry,
            "interval_seconds",
            LIVENESS_INTERVAL_DEFAULT_S,
            probe_idx,
        )
        .map_err(|err| map_zero_to_named_error(err, "interval_seconds", probe_idx))?;
        let failure_threshold = parse_optional_positive_u32(
            entry,
            "failure_threshold",
            LIVENESS_FAILURE_THRESHOLD_DEFAULT,
            probe_idx,
        )
        .map_err(|err| map_zero_to_named_error(err, "failure_threshold", probe_idx))?;

        out.push(ProbeDescriptor {
            idx: probe_idx_from_position(probe_idx),
            role: ProbeRole::Liveness,
            mechanic,
            timeout_seconds,
            interval_seconds,
            // Liveness is continuous; `max_attempts` is not a liveness
            // gate. Carry the startup default so the shared field has a
            // sane value (the liveness reconcile branch never reads it).
            max_attempts: STARTUP_MAX_ATTEMPTS_DEFAULT,
            failure_threshold: Some(failure_threshold),
            success_threshold: None,
            inferred: false,
        });
    }
    Ok(out)
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
/// - `path` non-empty but not starting with `/` →
///   [`ParseError::HttpProbePathNotAbsolute`]. The path is concatenated
///   into `http://<host>:<port><path>` at probe time; a relative path
///   produces a URL the prober cannot build, which the probe loop
///   silently swallows — so it is rejected at parse time instead.
/// - `port` absent / out of `1..=65535` → reuses the TCP `port`
///   diagnostics shape via [`ParseError::TcpProbeMissingPort`] — the
///   port precondition is identical across mechanics.
fn parse_http_mechanic(
    entry: &toml::value::Table,
    probe_idx: usize,
) -> Result<ProbeMechanic, ParseError> {
    let path_raw = entry.get("path").ok_or(ParseError::HttpProbeMissingPath { probe_idx })?;
    let path = path_raw.as_str().ok_or_else(|| ParseError::Field {
        section: "[[health_check.*]]",
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
    // Path must be absolute. At probe time the runner builds
    // `http://<host>:<port><path>`; a path without a leading `/` yields a
    // malformed URL (`http://127.0.0.1:8080health`) that hyper rejects
    // *after* parse — the supervised probe loop then swallows the adapter
    // error (warn + continue, no `ProbeResultRow`), the startup window
    // never closes, and the service hangs. Reject here so the malformed
    // config never reaches the runtime.
    if !path.starts_with('/') {
        return Err(ParseError::HttpProbePathNotAbsolute { probe_idx, path: path.to_owned() });
    }

    let port_raw = entry.get("port").ok_or(ParseError::TcpProbeMissingPort { probe_idx })?;
    let port_int = port_raw.as_integer().ok_or_else(|| ParseError::Field {
        section: "[[health_check.*]]",
        message: format!("entry [{probe_idx}]: field `port` must be an integer"),
    })?;
    let port = u16::try_from(port_int).map_err(|_| ParseError::Field {
        section: "[[health_check.*]]",
        message: format!("entry [{probe_idx}]: field `port` must be in 1..=65535"),
    })?;
    if port == 0 {
        return Err(ParseError::TcpProbeMissingPort { probe_idx });
    }

    let host = entry.get("host").and_then(toml::Value::as_str).map(str::to_owned);
    Ok(ProbeMechanic::Http { path: path.to_owned(), port, host })
}

/// Parse the `type = "exec"` mechanic body per ADR-0057 §2 / US-03.
///
/// Required field: `command` (a non-empty array of strings; `command[0]`
/// is the binary, `command[1..]` are argv). Optional `args` (an array of
/// strings) is appended to the argv tail — the operator may split the
/// binary and its arguments across the two fields or inline everything
/// in `command`; the parser concatenates them into the single
/// `ProbeMechanic::Exec { command }` vector the `ExecProber` trait
/// consumes (binary at index 0, every other token an argv tail).
///
/// Edge cases:
/// - `command` absent OR an empty array →
///   [`ParseError::ExecProbeMissingCommand`]. An exec probe with no
///   binary to spawn is meaningless.
/// - `command` present but not an array of strings, or `args` not an
///   array of strings → [`ParseError::Field`] with a diagnostic naming
///   the offending field.
fn parse_exec_mechanic(
    entry: &toml::value::Table,
    probe_idx: usize,
) -> Result<ProbeMechanic, ParseError> {
    // `command` is required and must be a non-empty array of strings.
    let command = match entry.get("command") {
        None => return Err(ParseError::ExecProbeMissingCommand { probe_idx }),
        Some(value) => parse_string_array(value, "command", probe_idx)?,
    };
    if command.is_empty() {
        return Err(ParseError::ExecProbeMissingCommand { probe_idx });
    }

    // `args` is optional; absent → empty. Appended to the argv tail of
    // the binary so the final `command` vector is
    // `[binary, command_tail.., args..]`.
    let mut command_line = command;
    if let Some(value) = entry.get("args") {
        let extra = parse_string_array(value, "args", probe_idx)?;
        command_line.extend(extra);
    }

    Ok(ProbeMechanic::Exec { command: command_line })
}

/// Parse a TOML value expected to be an array of strings into a
/// `Vec<String>`. Surfaces a [`ParseError::Field`] naming `field` when
/// the value is not an array, or contains a non-string element.
fn parse_string_array(
    value: &toml::Value,
    field: &str,
    probe_idx: usize,
) -> Result<Vec<String>, ParseError> {
    let arr = value.as_array().ok_or_else(|| ParseError::Field {
        section: "[[health_check.*]]",
        message: format!("entry [{probe_idx}]: field `{field}` must be an array of strings"),
    })?;
    arr.iter()
        .map(|element| {
            element.as_str().map(str::to_owned).ok_or_else(|| ParseError::Field {
                section: "[[health_check.*]]",
                message: format!(
                    "entry [{probe_idx}]: every element of `{field}` must be a string"
                ),
            })
        })
        .collect()
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
            section: "[[health_check.*]]",
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
            section: "[[health_check.*]]",
            message: format!(
                "entry [{probe_idx}]: field `{field}` must be a non-negative integer fitting in u32"
            ),
        })
    })
}

/// Map an `OptionalPositiveU32Error::Zero` to the field-specific
/// named variant (`ProbeTimeoutZero` / `ProbeIntervalZero` /
/// `ProbeMaxAttemptsZero` / `ProbeSuccessThresholdZero` /
/// `ProbeFailureThresholdZero`). A `Field` carries through verbatim.
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
            "success_threshold" => ParseError::ProbeSuccessThresholdZero { probe_idx },
            "failure_threshold" => ParseError::ProbeFailureThresholdZero { probe_idx },
            _ => unreachable!("map_zero_to_named_error called for unknown field: {field}"),
        },
        OptionalPositiveU32Error::Field(parse_error) => parse_error,
    }
}
