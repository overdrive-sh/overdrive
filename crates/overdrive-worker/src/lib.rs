//! Overdrive worker subsystem — `ExecDriver`, workload-cgroup
//! management, and the boot-time `node_health` row writer.
//!
//! Per ADR-0029, the worker subsystem is its own crate (class
//! `adapter-host`) so that the boundary between the control-plane and
//! the worker is enforced at compile time. The control-plane crate
//! sees only the `Driver` trait surface (from `overdrive-core`); the
//! impl is plugged in by the binary at `AppState` construction time.
//!
// `forbid(unsafe_code)` is intentionally NOT set: `Driver::stop` on
// Linux invokes `libc::kill(pid, SIGTERM)`, which requires `unsafe`.
// Per `.claude/rules/development.md`, the worker crate is class
// `adapter-host` — host-OS interaction is its raison d'être. The
// workspace-wide `unsafe_op_in_unsafe_fn = deny` lint still requires
// every `unsafe { ... }` block to be explicit, with a `// SAFETY:`
// comment documenting the precondition.
#![deny(unsafe_op_in_unsafe_fn)]
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

pub mod cgroup_manager;
pub mod driver;
pub mod node_health;
// SCAFFOLD: true — service-health-check-probes feature.
// ProbeRunner subsystem per ADR-0054 §2. Lands GREEN across slices
// 01 (TCP / Earned Trust), 02 (HTTP), 03 (Exec).
pub mod probe_runner;

pub use cgroup_manager::{CgroupManager, CgroupPath};
pub use driver::ExecDriver;
pub use node_health::{NodeConfig, NodeHealthWriteError};

use std::sync::Arc;

use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::ObservationStore;

/// Worker-startup boundary per ADR-0029.
///
/// Performs every step the worker subsystem needs to "be started"
/// before the control-plane accepts traffic. Phase 1 single-purpose:
/// writes the local node's `NodeHealthRow` to the `ObservationStore`
/// per ADR-0025 step 5.
///
/// The helper exists as the contract-boundary entry point so the
/// control-plane composition root (`run_server_with_obs_and_driver`)
/// only knows the worker subsystem by its `start_local_node` driving
/// port — never by the internal `node_health` module's helpers. Phase
/// 2+ additions (heartbeat reconciler scheduling, capacity probe,
/// driver-readiness handshake) extend this function without changing
/// the boundary.
///
/// `Clock` is required at construction per
/// `.claude/rules/development.md` § "Port-trait dependencies": the
/// caller injects the host or sim implementation explicitly so tests
/// cannot silently inherit wall-clock behaviour by forgetting to
/// override.
///
/// # Errors
///
/// Returns the typed [`NodeHealthWriteError`] from the inner writer
/// when `NodeId` resolution or the obs-store write fails. The
/// composition root converts via `#[from]` into the top-level
/// `ControlPlaneError::NodeHealthWrite` variant — never flattened to
/// `Internal(String)` per `.claude/rules/development.md` § "Never
/// flatten a typed error to `Internal(String)` at a composition
/// boundary".
pub async fn start_local_node(
    obs: &Arc<dyn ObservationStore>,
    config: &NodeConfig,
    clock: &Arc<dyn Clock>,
) -> Result<(), NodeHealthWriteError> {
    node_health::write_node_health_row(obs, config, clock).await
}
