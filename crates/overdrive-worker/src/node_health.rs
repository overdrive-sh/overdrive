//! Boot-time `node_health` row writer.
//!
//! Per ADR-0025 (amended by ADR-0029), the worker subsystem writes
//! the local node's `NodeHealthRow` to the `ObservationStore` at
//! startup, before the worker is considered "started." Phase 1
//! single-node ships exactly one row at runtime; Phase 2+ multi-node
//! has every node writing its own.
//!
//! `NodeId` is resolved from the `[node].id` config override, falling
//! back to `hostname` per ADR-0025.
//!
//! # Status — RED scaffold
//!
//! Phase: phase-1-first-workload, slice 4 (US-04 cgroup isolation
//! shares the boot path with the `node_health` writer). Wave: DISTILL.

// `unused_async` lint fires because the panic-bodied scaffold has no
// `.await`. The production signature must remain `async` because the
// real implementation will call `ObservationStore::write` (async).
// Allow the lint while the body is the RED scaffold; remove this
// allow when slice 4 GREEN lands.
#![allow(clippy::unused_async)]

use std::sync::Arc;

use overdrive_core::traits::observation_store::ObservationStore;

/// SCAFFOLD marker.
pub const SCAFFOLD: bool = true;

/// Write the local node's `NodeHealthRow` to the observation store.
/// Idempotent — Phase 1 single-node always writes a row whose primary
/// key is the local `NodeId`.
///
/// # Errors
///
/// Returns an error if the observation store rejects the write or if
/// the `[node].id` config override fails to parse.
///
/// # Panics
///
/// RED scaffold.
pub async fn write_node_health_row(
    _obs: &Arc<dyn ObservationStore>,
    _config: &NodeConfig,
) -> Result<(), NodeHealthWriteError> {
    panic!("Not yet implemented -- RED scaffold")
}

/// Operator-supplied `[node]` config block per ADR-0025.
/// `id` is optional (hostname fallback); `region` defaults to
/// `"local"`; `capacity` is required for Phase 1 (no autodetection
/// yet).
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Optional override; falls back to `hostname` per ADR-0025.
    pub id_override: Option<String>,
    /// Region — Phase 1 default `"local"`.
    pub region: String,
    /// Declared capacity for the local node. Phase 1 requires this
    /// from config; Phase 2+ may auto-detect from the kernel.
    pub capacity: overdrive_core::traits::driver::Resources,
}

/// Errors from [`write_node_health_row`].
#[derive(Debug, thiserror::Error)]
pub enum NodeHealthWriteError {
    /// Failed to resolve `NodeId` (hostname read failed AND no
    /// override was supplied; or override failed to parse).
    #[error("node id resolution failed: {0}")]
    IdResolve(String),
    /// Underlying observation-store write failure.
    #[error("observation store write failed: {0}")]
    Write(String),
}
