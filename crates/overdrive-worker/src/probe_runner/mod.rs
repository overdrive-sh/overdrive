//! `ProbeRunner` subsystem — per-alloc-per-probe tokio task graph
//! that ticks declared/inferred probes and writes
//! `ProbeResultRow`s to the `ObservationStore`.
//!
//! Per ADR-0054:
//! - `overdrive-worker` placement (probe execution is observation
//!   production — belongs to the machine running the workload per
//!   C1).
//! - Per-alloc supervisor + per-probe-instance tokio task shape
//!   (matches K8s prober.Manager archetype; D-02).
//! - Three port traits per ADR-0054 §3 (`TcpProber` / `HttpProber` /
//!   `ExecProber`); each backed by a production adapter
//!   (`TokioTcpProber` / `HyperHttpProber` / `CgroupExecProber`)
//!   and a sim adapter (in `crates/overdrive-sim/src/adapters/
//!   probers.rs`).
//! - Earned Trust gate at composition root (DDD-21): `probe()`
//!   runs after construction and before serving any request;
//!   failure refuses startup via `health.startup.refused`.
//!
//! RED scaffold — module tree + entry-point declared. Per-probe
//! task graph + Earned Trust gate land in slice 01; HTTP / Exec
//! prober production bindings land in slice 02 / 03.
// SCAFFOLD: true

#![allow(dead_code)]
#![expect(clippy::todo, reason = "RED scaffold; lands GREEN across slices 01-03")]
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

pub mod exec_prober;
pub mod http_prober;
pub mod tcp_prober;

pub use exec_prober::CgroupExecProber;
pub use http_prober::HyperHttpProber;
pub use tcp_prober::TokioTcpProber;

use std::sync::Arc;

use overdrive_core::traits::prober::{ExecProber, HttpProber, TcpProber};

/// Subsystem entry point — owned by the worker's composition root.
///
/// Per ADR-0054 §2: per-alloc supervisor supervises N per-probe
/// tokio tasks. Each task ticks its probe on the configured
/// interval, writes a `ProbeResultRow` to the `ObservationStore`,
/// and surrenders on its `CancellationToken.child_token()` when
/// the alloc reaches a terminal state.
///
/// Production wiring at the composition root passes
/// `Arc<TokioTcpProber>` / `Arc<HyperHttpProber>` /
/// `Arc<CgroupExecProber>`. Tests pass the sim equivalents.
///
/// RED scaffold — constructor + `start_alloc` / `stop_alloc` /
/// `probe` (Earned Trust gate) land in slice 01.
#[allow(
    clippy::struct_field_names,
    reason = "Per-mechanic prober field naming is operator-readable; renaming loses the per-mechanic split documented in ADR-0054 §3"
)]
pub struct ProbeRunner {
    tcp_prober: Arc<dyn TcpProber>,
    http_prober: Arc<dyn HttpProber>,
    exec_prober: Arc<dyn ExecProber>,
}

impl ProbeRunner {
    /// Construct a `ProbeRunner` with injected adapters. Per
    /// `.claude/rules/development.md` § "Port-trait dependencies":
    /// adapters are MANDATORY constructor parameters — no
    /// `with_xxx` builder, no default-to-production inside the
    /// constructor.
    pub fn new(
        tcp_prober: Arc<dyn TcpProber>,
        http_prober: Arc<dyn HttpProber>,
        exec_prober: Arc<dyn ExecProber>,
    ) -> Self {
        Self { tcp_prober, http_prober, exec_prober }
    }

    /// Earned Trust gate per DDD-21 + ADR-0054 §7. Runs after
    /// construction and before the runtime serves any request.
    /// Sacrificial-listener path validates the TCP adapter end-to-
    /// end; a failure refuses startup with structured
    /// `health.startup.refused` event.
    ///
    /// Per ADR-0054 §7 the sacrificial listener binds to
    /// `127.0.0.1:0` (kernel-assigned port; no race per the
    /// risk-table mitigation).
    #[allow(
        clippy::unused_async,
        reason = "RED scaffold; GREEN body in slice-01 will .await on injected probers"
    )]
    pub async fn probe(&self) -> Result<(), ProbeRunnerError> {
        todo!("RED scaffold: ProbeRunner::probe (Earned Trust gate) — lands GREEN in slice-01")
    }
}

/// Errors surfaced by the runner subsystem.
///
/// `EarnedTrustFailure` is the variant that triggers
/// `health.startup.refused` per ADR-0054 §7.
#[derive(Debug, thiserror::Error)]
pub enum ProbeRunnerError {
    #[error("Earned Trust probe failed: {reason}")]
    EarnedTrustFailure { reason: String },
}
