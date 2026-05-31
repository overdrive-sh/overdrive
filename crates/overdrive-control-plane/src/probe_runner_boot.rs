//! Composition-root `ProbeRunner` Earned-Trust boot helper per
//! service-health-check-probes step 01-03d / ADR-0054 § 7.
//!
//! Per `.claude/rules/development.md` principle 12 ("Earned-Trust"):
//! every adapter / subsystem declares a `probe(&self)` method
//! exercising its critical dependency; the composition root invokes
//! every registered `probe()` at startup BEFORE serving any request;
//! failure surfaces as a typed bootstrap error with a structured
//! `tracing::error!(name: "health.startup.refused", ...)` event and
//! a non-zero CLI exit.
//!
//! This module is the single composition-root call site that wires
//! the three probe adapters into a [`ProbeRunner`], runs its
//! Earned-Trust gate, and emits the canonical refusal event on
//! failure. The structural defense against the call site being
//! removed is the `xtask::dst_lint` ProbeRunner-declaration scanner
//! clause (landed in step 01-03c). NOTE: that scanner today enforces
//! only the method *declaration*, not the call site; see step 01-03d
//! AC #5 — a follow-up scope question to extend the scanner to walk
//! call sites would belong on a separate slice if the user wants the
//! structural defense at the call-site level too.

use std::sync::Arc;

use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::prober::{ExecProber, HttpProber, TcpProber};
use overdrive_worker::probe_runner::ProbeRunner;

use crate::error::{ControlPlaneError, ProbeRunnerBootError};

/// Construct a [`ProbeRunner`] from the injected adapter triple and
/// run its Earned-Trust gate per ADR-0054 § 7. Returns the live
/// `Arc<ProbeRunner>` on success; on failure emits the canonical
/// `health.startup.refused` tracing event and returns the typed
/// `ControlPlaneError::ProbeRunnerBoot` so the CLI binary boundary
/// can convert to a non-zero exit per `.claude/rules/development.md`
/// principle 12.
///
/// The probe gate runs BEFORE the listener binds — same boot-path
/// shape as `EbpfDataplane::probe()` per architecture.md § 5.4 and
/// the existing `health.startup.refused` event for `dataplane.probe`
/// in `run_server_with_obs_and_driver`.
///
/// # Errors
///
/// Returns [`ControlPlaneError::ProbeRunnerBoot`] when the
/// sacrificial-loopback probe does not return Pass — the TCP
/// adapter is wired but cannot complete a round-trip against
/// `127.0.0.1`. Typically signals that the loopback interface is
/// down, the sim adapter was given a Fail outcome (acceptance-test
/// injection), or a probe-adapter regression has broken the connect
/// path.
pub async fn compose_and_probe_runner_gate(
    tcp_prober: Arc<dyn TcpProber>,
    http_prober: Arc<dyn HttpProber>,
    exec_prober: Arc<dyn ExecProber>,
    clock: Arc<dyn Clock>,
    observation_store: Arc<dyn ObservationStore>,
) -> Result<Arc<ProbeRunner>, ControlPlaneError> {
    let runner =
        Arc::new(ProbeRunner::new(tcp_prober, http_prober, exec_prober, clock, observation_store));
    match runner.probe().await {
        Ok(()) => Ok(runner),
        Err(source) => {
            // Structured tracing event per ADR-0054 § 7. The
            // `name:` slot matches the canonical refusal vocabulary
            // shared by `dataplane.probe` / `cgroup.preflight` etc.;
            // `reason` distinguishes which subsystem refused.
            tracing::error!(
                name: "health.startup.refused",
                reason = "probe_runner.earned_trust",
                error = %source,
                "ProbeRunner Earned-Trust probe failed; refusing to boot"
            );
            Err(ControlPlaneError::from(ProbeRunnerBootError::Probe { source }))
        }
    }
}
