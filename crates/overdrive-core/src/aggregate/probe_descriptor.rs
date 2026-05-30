//! `ProbeDescriptor` — validated intent-side probe declaration.
//!
//! Per ADR-0057 §1: probes are declared in TOML under
//! `[[health_check.startup]]` / `[[health_check.readiness]]` /
//! `[[health_check.liveness]]`. After parse-time validation, each
//! row becomes a `ProbeDescriptor` and is rkyv-archived as part of
//! the `ServiceSpec` aggregate (envelope V1→V2 bump per ADR-0057).
//!
//! Per ADR-0058 §1 ("honest by default"): if zero startup probes are
//! declared AND at least one `[[listener]]` is present, the parser
//! synthesises a single `ProbeDescriptor` with
//! `mechanic: ProbeMechanic::Tcp { host, port: listeners[0].port }`
//! and `inferred: true`. An empty `[[health_check.startup]] = []`
//! array is the explicit opt-out (preserves Phase 1 first-Running
//! semantics).

use serde::{Deserialize, Serialize};

use crate::observation::ProbeRole;

/// Per-kind guidance text surfaced on
/// [`crate::aggregate::workload_spec::ParseError::ProbesNotAllowedOnKind`]
/// when an operator declares a `[[health_check.*]]` array on a
/// non-Service workload (Slice 07 / US-07 / K5).
///
/// The guidance explains *why* the kind has no probe surface, so the
/// rejection reads as a teaching moment rather than an opaque "not
/// allowed". The text is a per-kind constant (not a format string)
/// so the operator-facing message is stable and greppable.
///
/// Job: a run-to-completion workload's success criterion IS its exit
/// code — there is no "is it ready to serve?" question to answer, so
/// readiness/liveness/startup probes are meaningless.
pub const JOB_PROBES_GUIDANCE: &str = "Job has no readiness question; on completion is enough.";

/// Schedule guidance — see [`JOB_PROBES_GUIDANCE`].
///
/// Schedule: a cron-scheduled job composes a fresh per-fire workload
/// each tick; the durable thing a probe would gate is the Service the
/// Schedule fires against, not the Schedule envelope itself.
pub const SCHEDULE_PROBES_GUIDANCE: &str =
    "Schedule composes per-fire; declare probes on the Service the Schedule fires.";

/// Concrete mechanic for a probe attempt.
///
/// Per ADR-0054: three mechanics, each backed by a distinct port
/// trait (`TcpProber` / `HttpProber` / `ExecProber`). Step 01-02
/// lands the `Tcp` variant; `Http` lands in step 02-01 and `Exec`
/// in step 02-02 — all three are part of the enum so the
/// `ServiceSpec` envelope shape is stable across slices.
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
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProbeMechanic {
    /// TCP-connect against `host:port`. Default for inferred probes
    /// per ADR-0058 (host defaults to bind-side wildcard
    /// translated to loopback at probe time).
    Tcp { host: String, port: u16 },
    /// HTTP GET against `http://host:port<path>`. Phase 1 = plain
    /// HTTP only per C6. Method = GET only per US-02. 3xx → Fail
    /// per US-02 AC (no redirect-follow).
    Http { path: String, port: u16, host: Option<String> },
    /// Spawn `command[0]` with `command[1..]` as args, inside the
    /// workload's cgroup per ADR-0059 / C7. Exit 0 = Pass.
    Exec { command: Vec<String> },
}

impl ProbeMechanic {
    /// Validate field values. Returns `Ok(())` if valid, or a message
    /// describing the first invalid field.
    ///
    /// Centralises the invariants so the TOML parser path
    /// (`parse_http_mechanic` in `workload_spec.rs`) and the API
    /// admission path (`ServiceV1::from_submit`) converge on the same
    /// checks. Neither path can drift independently.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Http { path, port, .. } => {
                if path.is_empty() {
                    return Err("http probe `path` must be non-empty".to_owned());
                }
                if path.contains("https://") {
                    return Err(
                        "https:// URLs are not supported in Phase 1 (plain HTTP only per ADR-0057 C6) — use a plain `path` like `/healthz`"
                            .to_owned(),
                    );
                }
                if !path.starts_with('/') {
                    return Err(format!("http probe `path` must start with `/` — got {path:?}"));
                }
                if *port == 0 {
                    return Err("http probe `port` must be in 1..=65535".to_owned());
                }
            }
            Self::Tcp { port, .. } => {
                if *port == 0 {
                    return Err("tcp probe `port` must be in 1..=65535".to_owned());
                }
            }
            Self::Exec { command } => {
                if command.is_empty() {
                    return Err("exec probe `command` must be a non-empty array".to_owned());
                }
            }
        }
        Ok(())
    }
}

/// Validated probe declaration after TOML parse + defaults applied.
///
/// Per ADR-0057 §2 (TOML defaults table):
/// - `timeout_seconds`: default 5 (diverges from K8s 1s; justified
///   by 5s being the operational sweet spot per research).
/// - `interval_seconds`: startup=2, readiness=2, liveness=10.
/// - `max_attempts`: 30 (yields `startup_deadline = 60s` for
///   startup probes per US-01 Technical Notes).
/// - `failure_threshold` (liveness only): 3.
/// - `success_threshold` (readiness only): 1 (per ADR-0055 §6 /
///   P2-Q8); configurable upward.
///
/// `inferred` distinguishes platform-synthesised default probes
/// (per ADR-0058) from operator-declared probes. Renderer surfaces
/// as `(inferred)` suffix per US-06 AC.
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
pub struct ProbeDescriptor {
    pub role: ProbeRole,
    pub mechanic: ProbeMechanic,
    pub timeout_seconds: u32,
    pub interval_seconds: u32,
    pub max_attempts: u32,
    /// Liveness only. `None` for startup / readiness.
    pub failure_threshold: Option<u32>,
    /// Readiness only. `None` for startup / liveness. Default 1.
    pub success_threshold: Option<u32>,
    /// `true` IFF this descriptor was synthesised by the platform's
    /// default-TCP inference rule per ADR-0058. `false` for every
    /// operator-declared probe.
    pub inferred: bool,
}
