//! Per-primitive libSQL path derivation and DB opening.
//!
//! SCAFFOLD: true — created by DISTILL wave for phase-1-control-plane-core.
//!
//! Per ADR-0013 §5, the path shape is
//! `<data_dir>/reconcilers/<reconciler_name>/memory.db`. The provisioner:
//!
//! 1. Canonicalises `data_dir` via `std::fs::canonicalize` at startup.
//! 2. Concatenates the name-scoped path.
//! 3. Asserts the result starts with `<canonicalised_data_dir>/
//!    reconcilers/` — defence-in-depth if the `ReconcilerName` regex
//!    ever regresses.
//! 4. Creates the directory tree and opens the libSQL file.

use std::path::PathBuf;

use overdrive_core::reconciler::ReconcilerName;

use crate::error::ControlPlaneError;

/// Derive the canonicalised libSQL memory-db path for a reconciler.
///
/// SCAFFOLD: true
pub fn provision_db_path(
    _data_dir: &std::path::Path,
    _name: &ReconcilerName,
) -> Result<PathBuf, ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}

/// Open the libSQL database at `path`, creating the directory tree as
/// needed. Returns a handle the runtime will wrap as the reconciler's
/// `&Db` parameter.
///
/// SCAFFOLD: true
pub fn open_db(_path: &std::path::Path) -> Result<libsql::Connection, ControlPlaneError> {
    panic!("Not yet implemented -- RED scaffold")
}
