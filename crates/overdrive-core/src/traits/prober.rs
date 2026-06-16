//! Probe port traits — `TcpProber` / `HttpProber` / `ExecProber`.
//!
//! Per ADR-0054 §3 (Port-trait shape) — three separate traits because
//! each mechanic has distinct preconditions, postconditions, and
//! adapter dependency surfaces. A unified trait would conflate
//! contracts; see ADR-0054 Consequences (Negative) for the
//! future-simplification candidate trade-off.
//!
//! Per `.claude/rules/development.md` § "Trait definitions specify
//! behavior, not just signature": each trait method below carries
//! preconditions, postconditions, edge cases, and observable
//! invariants the DST equivalence harness will assert against both
//! production and sim adapter implementations.
//!
//! RED scaffold — production bindings land in `crates/overdrive-
//! worker/src/probe_runner/{tcp,http,exec}_prober.rs`; sim bindings
//! land in `crates/overdrive-sim/src/adapters/probers.rs`.
// SCAFFOLD: true
// __SCAFFOLD__ = true (Python convention; Rust marker is the
// `SCAFFOLD: true` line above + the `todo!()` bodies below)

#![allow(dead_code)]

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Outcome of a single probe attempt.
///
/// `Pass` indicates the probe's success predicate was satisfied within
/// the configured timeout. `Fail` carries a named failure reason
/// suitable for direct operator-visible rendering — these strings end
/// up in `ProbeResultRow.last_fail_reason`, in CLI `alloc status`
/// Probes section, and (for startup probes that exhaust their attempts)
/// in the `ServiceSubmitEvent::Failed { reason: StartupProbeFailed
/// { last_fail, .. } }` wire payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeOutcome {
    /// Probe predicate satisfied within timeout.
    Pass,
    /// Probe predicate not satisfied. `reason` is an
    /// operator-renderable short string ("connection refused",
    /// "HTTP 503", "timeout after 5s", "exit 1", "exec: command not
    /// found"). Reason strings are part of the operator-facing
    /// contract; renaming them is a wire-shape change.
    Fail { reason: String },
}

/// Driven port for TCP-connect probes.
///
/// # Preconditions
/// - `host` is a non-empty `&str` parseable as either an IP literal
///   or a DNS name. Caller MUST have validated this at parse time;
///   the trait MUST NOT redo validation.
/// - `port` is `1..=65535`. Caller validates at parse time.
/// - `timeout` is `Duration::from_millis(>= 1)`. Zero-timeout calls
///   surface as `ProbeFailure::TimeoutZero` at parse time, never
///   reach this trait.
///
/// # Postconditions
/// - Returns `Ok(ProbeOutcome::Pass)` IFF a TCP three-way handshake
///   completes against `host:port` within `timeout`.
/// - Returns `Ok(ProbeOutcome::Fail { reason })` for every other
///   observable outcome: kernel `ECONNREFUSED` → `"connection
///   refused"`; `ETIMEDOUT` or timeout-wrapper fired → `"timeout
///   after <duration>"`; DNS resolution failure → `"dns: <error>"`.
/// - Returns `Err(ProbeFailure)` only for programmer-error or
///   infrastructure-error conditions (constructor panic, system-wide
///   resource exhaustion). These are NOT operator-renderable as
///   probe results — they surface separately as runner-startup
///   refusals.
/// - The connection is dropped immediately on success. No data is
///   sent or expected.
///
/// # Edge cases
/// - `host = "0.0.0.0"` is a valid bind-side wildcard but a
///   client-side target — caller is responsible for translating to
///   the workload's reachable address (typically `"127.0.0.1"` for
///   single-node Phase 1).
/// - IPv6 literal addresses arrive as `"[::1]"` form per the
///   caller-validation contract.
///
/// # Observable invariants
/// - For any `(host, port, timeout)`, `tcp.probe(...)` is idempotent
///   in observable state — two consecutive calls produce the same
///   outcome class against an unchanged listener.
/// - No side effects other than the transient socket — no logging
///   beyond `tracing::trace!`, no metric emission.
#[async_trait]
pub trait TcpProber: Send + Sync + 'static {
    /// Execute a single TCP-connect probe attempt.
    async fn probe(
        &self,
        host: &str,
        port: u16,
        timeout: Duration,
    ) -> Result<ProbeOutcome, ProbeFailure>;
}

/// Driven port for HTTP GET probes (Phase 1 — plain HTTP only per
/// C6; HTTPS / mTLS / gRPC deferred to Phase 3+).
///
/// # Preconditions
/// - `url` is a `&str` parseable as an absolute HTTP URL with `http`
///   scheme. Caller validates at parse time (no `https`, no relative
///   URLs reach this trait).
/// - `timeout` is `Duration::from_millis(>= 1)`.
///
/// # Postconditions
/// - Returns `Ok(ProbeOutcome::Pass)` IFF the GET request completes
///   within `timeout` AND returns an HTTP status in `200..=299`.
/// - Returns `Ok(ProbeOutcome::Fail { reason })` for:
///   - HTTP 3xx (redirect) → `"HTTP <code>"`. Probe does NOT follow
///     redirects (per US-02 AC; research § 6.1 Pitfall 5).
///   - HTTP 4xx / 5xx → `"HTTP <code>"`.
///   - Connection error → `"connection refused"` / `"connection
///     reset"` / `"dns: <error>"`.
///   - Timeout → `"timeout after <duration>"`.
/// - HTTP request body is empty. Response body is discarded
///   regardless of size — only status code matters.
/// - HTTP method is GET only in Phase 1. POST and custom methods
///   deferred to Phase 2 per US-02 Technical Notes.
/// - Returns `Err(ProbeFailure)` only for programmer-error
///   conditions (URL parse failure escaped caller validation,
///   underlying transport panic).
///
/// # Edge cases
/// - 3xx responses are Fail (not Pass, not redirect-follow). This is
///   the load-bearing divergence from naive HTTP-client semantics —
///   research § 6.1 Pitfall 5 documents the masking effect of
///   redirect-follow on health checks.
/// - HTTP/2 server-push frames are ignored. HTTP/1.1 keep-alive may
///   be reused at the connection-pool layer (`hyper-util` default).
///
/// # Observable invariants
/// - Outcome class is a deterministic function of the response's
///   first byte (status line) — body content cannot change the
///   verdict.
/// - The probe never writes to disk, never spawns subprocesses,
///   never modifies process-global state.
#[async_trait]
pub trait HttpProber: Send + Sync + 'static {
    /// Execute a single HTTP GET probe attempt against `url`.
    async fn probe(&self, url: &str, timeout: Duration) -> Result<ProbeOutcome, ProbeFailure>;
}

/// Driven port for exec probes — spawn a command inside the
/// workload's cgroup scope.
///
/// # Preconditions
/// - `command` is `&[String]` with `command.len() >= 1`. Empty
///   command surfaces as `ParseError::ExecProbeMissingCommand` at
///   parse time.
/// - `cgroup_scope_path` is the absolute path of the workload's
///   cgroup scope (e.g. `/sys/fs/cgroup/overdrive.slice/workloads.
///   slice/alloc-payments-0.scope`). Caller (ProbeRunner) sources
///   this from the AllocationSpec / ExecDriver coordination per
///   ADR-0059.
/// - `timeout` is `Duration::from_millis(>= 1)`.
///
/// # Postconditions
/// - Returns `Ok(ProbeOutcome::Pass)` IFF the spawned process exits
///   with status `0` within `timeout`.
/// - Returns `Ok(ProbeOutcome::Fail { reason })` for:
///   - Non-zero exit → `"exit <N>"`.
///   - Timeout (process SIGKILLed at timeout boundary) →
///     `"timeout after <duration>"`.
///   - `execve` failure (binary not on PATH inside cgroup namespace,
///     no execute permission) → `"exec: command not found"` /
///     `"exec: permission denied"`.
/// - Returns `Err(ProbeFailure::ExecSpawnFailed { reason })` ONLY
///   for cgroup-placement-layer failures (ENOSPC / EACCES / ENOENT /
///   EBUSY on the cgroup write itself, per ADR-0054 § 3 QR2
///   amendment). The runner does NOT auto-retry these; retry-on-
///   cgroup-error is a DELIVER-wave policy decision deliberately
///   deferred so the trait contract stays stable.
///
/// # Edge cases
/// - The spawned process inherits the workload's mount + network
///   namespace via cgroup placement per C7 / ADR-0059. Sim adapter
///   does NOT assert cgroup membership — that's a Tier 3 concern.
/// - SIGKILL on timeout uses `cgroup.kill` (Linux 5.14+) per
///   ADR-0059 § 3 — always available on the pinned 6.18 appliance
///   kernel (ADR-0068). (The `child.kill()` / process-group SIGKILL
///   reaps are belt-and-braces handle/grandchild cleanup, not a
///   kernel-version fallback.)
/// - The probe's stdout / stderr are discarded by default. (Phase 2+
///   may add capture; not in Phase 1.)
///
/// # Observable invariants
/// - The probe process is a member of `cgroup_scope_path` per its
///   `/proc/<pid>/cgroup` readout. Asserted by Tier 3 integration
///   test under `crates/overdrive-worker/tests/integration/
///   exec_probe_cgroup_membership.rs`.
/// - The probe does NOT leak descendant processes — `cgroup.kill`
///   mass-kills any fork descendants. Operator-facing caveat per
///   ADR-0059 Consequences (Negative): exec probes that fork
///   workload-side children may have those children reaped on
///   timeout cleanup.
#[async_trait]
pub trait ExecProber: Send + Sync + 'static {
    /// Execute a single exec probe attempt.
    async fn probe(
        &self,
        command: &[String],
        cgroup_scope_path: &str,
        timeout: Duration,
    ) -> Result<ProbeOutcome, ProbeFailure>;
}

/// Probe-runner-internal failure variants — distinct from
/// `ProbeOutcome::Fail` which represents an observed-as-failed probe
/// outcome that flows into operator-visible state.
///
/// `ProbeFailure` variants represent programmer-error or
/// infrastructure-error conditions that prevent the probe from
/// executing AT ALL. These surface as runner-startup refusals or as
/// `health.startup.refused` events, never as `ProbeResultRow`
/// payloads.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProbeFailure {
    /// Cgroup-placement-layer failure during exec probe spawn (per
    /// ADR-0054 §3 QR2 amendment). Reason carries the underlying
    /// errno or syscall name in the same shape as `execve` failures.
    /// Runner does NOT auto-retry.
    #[error("exec probe spawn failed: {reason}")]
    ExecSpawnFailed { reason: String },
    /// URL parse or transport-layer construction failure that
    /// escaped caller validation. Programmer error; surfaces via
    /// Earned Trust gate failure at runner startup.
    #[error("invalid probe target: {reason}")]
    InvalidTarget { reason: String },
}

/// Result alias used throughout the prober trait surface.
pub type ProbeResult<T> = std::result::Result<T, ProbeFailure>;

#[cfg(test)]
mod tests {
    // RED scaffold — no tests in the trait module itself. Trait
    // contracts are exercised by:
    //
    // - `crates/overdrive-worker/tests/acceptance/probe_runner_*.rs`
    //   (Sim adapter contract — Tier 1 / default lane)
    // - `crates/overdrive-worker/tests/integration/probe_*_real.rs`
    //   (Production adapter against real loopback / real cgroup —
    //   Tier 3 / integration-tests feature)
    // - DST equivalence harness in `crates/overdrive-sim/src/
    //   invariants/prober_equivalence.rs` (lands in DELIVER per
    //   ADR-0054 §7).
}
