//! Overdrive Phase 1 single-mode control-plane.
//!
//! SCAFFOLD: true — created by DISTILL wave for phase-1-control-plane-core.
//!
//! This crate composes the intent-side `LocalStore`, the observation-side
//! `SimObservationStore` (Phase 1 production impl per ADR-0012), the
//! `axum` + `rustls` HTTP server (ADR-0008), the `rcgen`-minted ephemeral
//! CA (ADR-0010), the reconciler runtime (ADR-0013), and the shared
//! request/response types (ADR-0014) into the `overdrive serve` binary's
//! server loop.
//!
//! Module layout:
//!
//! | Module | Role |
//! |---|---|
//! | `api` | Shared request/response types (serde + utoipa) |
//! | `handlers` | axum route handlers — submit_job, describe_job, cluster_status, alloc_status, node_list |
//! | `error` | `ControlPlaneError` enum + `to_response` mapping (ADR-0015) |
//! | `tls_bootstrap` | Ephemeral CA + trust triple + rustls config (ADR-0010) |
//! | `reconciler_runtime` | `ReconcilerRuntime` + registry (ADR-0013) |
//! | `eval_broker` | `EvaluationBroker` + cancelable-eval-set (ADR-0013) |
//! | `libsql_provisioner` | Per-primitive libSQL path derivation (ADR-0013) |
//! | `observation_wiring` | `SimObservationStore` single-node wiring (ADR-0012) |
//!
//! Every public function below is a `panic!` stub. The DELIVER crafter
//! replaces bodies as each slice lands.

#![forbid(unsafe_code)]

pub mod api;
pub mod error;
pub mod eval_broker;
pub mod handlers;
pub mod libsql_provisioner;
pub mod observation_wiring;
pub mod reconciler_runtime;
pub mod tls_bootstrap;

use std::net::SocketAddr;
use std::path::PathBuf;

/// Configuration for the Phase 1 control-plane server. Populated at
/// startup from CLI flags and environment.
///
/// SCAFFOLD: true
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Socket address to bind the HTTPS listener. Default
    /// `127.0.0.1:7001` per ADR-0008.
    pub bind: SocketAddr,
    /// Data directory — parent of the redb file, per-primitive libSQL
    /// files, and the trust triple config file.
    pub data_dir: PathBuf,
}

/// Start the control-plane server. Binds the TLS listener, registers the
/// `noop-heartbeat` reconciler, and runs until SIGINT drains in-flight
/// requests per US-02 AC.
///
/// SCAFFOLD: true
pub async fn run_server(_config: ServerConfig) -> Result<(), error::ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}

/// Construct the `noop-heartbeat` reconciler. Exposed as a public factory
/// so the DST harness can register the same instance the control-plane
/// boot registers.
///
/// SCAFFOLD: true
pub fn noop_heartbeat() -> Box<dyn overdrive_core::reconciler::Reconciler> {
    panic!("Not yet implemented -- RED scaffold")
}
