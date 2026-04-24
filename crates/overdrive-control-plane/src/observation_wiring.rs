//! `LocalObservationStore` wired as the Phase 1 production observation-store impl.
//!
//! Per ADR-0012 (revised 2026-04-24), the Phase 1 server uses
//! `overdrive-store-local`'s `LocalObservationStore` — a redb-backed
//! real-adapter impl — as the server's `ObservationStore`. The earlier
//! wiring routed through `SimObservationStore`; the revision reverses
//! that to preserve the "production impls live under real adapters"
//! invariant in ADR-0003 and to gain persistence across restarts. Phase
//! 2+ swaps in `CorrosionStore` via a single `Box<dyn ObservationStore>`
//! trait-object replacement — no handler changes.
//!
//! This wiring module remains the seam. Handlers depend on
//! `&dyn ObservationStore`, never on `LocalObservationStore` directly.

use std::path::Path;

use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_store_local::LocalObservationStore;

use crate::error::ControlPlaneError;

/// Filename under `data_dir` where the single-node observation redb
/// file lives. Phase 2 replaces this with a `CorrosionStore` peer; the
/// control-plane boot code continues to hand a directory to the wiring
/// function, and the filename is an implementation detail the Phase 1
/// impl owns.
const OBSERVATION_FILE: &str = "observation.redb";

/// Construct the Phase 1 single-node observation store at
/// `<data_dir>/observation.redb`. Returns a trait-object handle so
/// handlers never name the concrete type.
///
/// The `data_dir` must be the same path the rest of the control plane
/// uses (see `ServerConfig::data_dir`); callers are expected to pass
/// `&config.data_dir`.
pub fn wire_single_node_observation(
    data_dir: &Path,
) -> Result<Box<dyn ObservationStore>, ControlPlaneError> {
    let path = data_dir.join(OBSERVATION_FILE);
    let store = LocalObservationStore::open(&path).map_err(|e| {
        ControlPlaneError::Internal(format!(
            "open LocalObservationStore at {}: {e}",
            path.display()
        ))
    })?;
    Ok(Box::new(store))
}
