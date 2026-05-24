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
//!
//! RED scaffold — types and validation rules sketched; rkyv +
//! parser-side land in slice 01 / 02 / 03 + slice 07.
// SCAFFOLD: true

#![allow(dead_code)]
#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::struct_field_names,
    reason = "DISTILL RED scaffold; per `.claude/rules/testing.md` § 'RED scaffolds' lints land when DELIVER replaces todo!() bodies + rewrites docs"
)]

use serde::{Deserialize, Serialize};

use crate::observation::ProbeRole;

/// Concrete mechanic for a probe attempt.
///
/// Per ADR-0054: three mechanics, each backed by a distinct port
/// trait (`TcpProber` / `HttpProber` / `ExecProber`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
