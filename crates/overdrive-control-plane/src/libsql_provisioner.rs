//! Per-primitive libSQL path derivation and DB opening.
//!
//! Per ADR-0013 §5, the path shape is
//! `<data_dir>/reconcilers/<reconciler_name>/memory.db`. The provisioner:
//!
//! 1. Creates `data_dir` if it does not yet exist (so `canonicalize`
//!    has a real path to resolve).
//! 2. Canonicalises `data_dir` via `std::fs::canonicalize`, collapsing
//!    symlinks and `..` segments so the defence-in-depth check below
//!    operates on the final resolved path.
//! 3. Concatenates the name-scoped path under `reconcilers/`.
//! 4. Asserts the result starts with `<canonicalised_data_dir>/
//!    reconcilers/` — defence-in-depth if the `ReconcilerName` regex
//!    ever regresses to permit path-separator characters.
//!
//! The `ReconcilerName` regex (`^[a-z][a-z0-9-]{0,62}$`, see
//! `overdrive-core`) is the primary guard — it already rejects `.`,
//! `/`, `\`, and `:`. Step 3 is insurance.

use std::path::{Path, PathBuf};

use overdrive_core::reconciler::ReconcilerName;

use crate::error::ControlPlaneError;

/// Derive the canonicalised libSQL memory-db path for a reconciler.
///
/// Returns `<canonicalise(data_dir)>/reconcilers/<name>/memory.db`.
/// The `data_dir` is created on first call if absent so `canonicalize`
/// has a valid target; the per-reconciler `<name>/` subdirectory is
/// left uncreated — `open_db` materialises it lazily.
///
/// # Errors
///
/// Returns `ControlPlaneError::Internal` if:
/// - `data_dir` cannot be created (permission denied, parent does not
///   exist, etc.)
/// - `canonicalize` fails (e.g. the filesystem rejects the path)
/// - the defence-in-depth `starts_with(reconcilers/)` check fails
///   (should be structurally impossible given `ReconcilerName`'s regex
///   but returned as a guarded error rather than a panic).
pub fn provision_db_path(
    data_dir: &Path,
    name: &ReconcilerName,
) -> Result<PathBuf, ControlPlaneError> {
    // Step 1 — make the data_dir real so canonicalize can resolve it.
    // `create_dir_all` is a no-op if it already exists.
    std::fs::create_dir_all(data_dir).map_err(|e| {
        ControlPlaneError::internal(
            format!("libsql_provisioner: create data_dir {} failed", data_dir.display()),
            e,
        )
    })?;

    // Step 2 — canonicalise.
    let canon = std::fs::canonicalize(data_dir).map_err(|e| {
        ControlPlaneError::internal(
            format!("libsql_provisioner: canonicalize {} failed", data_dir.display()),
            e,
        )
    })?;

    // Step 3 — derive the path.
    let reconcilers_root = canon.join("reconcilers");
    let path = reconcilers_root.join(name.as_str()).join("memory.db");

    // Step 4 — defence-in-depth `starts_with` check.
    if !path.starts_with(&reconcilers_root) {
        return Err(ControlPlaneError::Internal(format!(
            "libsql_provisioner: path traversal detected — {} does not start with {}",
            path.display(),
            reconcilers_root.display()
        )));
    }

    Ok(path)
}

/// Open the libSQL database at `path`, creating the directory tree as
/// needed. Returns a `libsql::Database` handle — the runtime obtains a
/// per-reconciler `Connection` via `db.connect()`.
///
/// Signature deviates from the DISTILL scaffold (`libsql::Connection`
/// return) to match the libsql 0.5 API shape: `Builder::new_local(path)
/// .build().await` yields `Database`, and `Database::connect()` is the
/// sync factory for connections.
///
/// # Errors
///
/// Returns `ControlPlaneError::Internal` if the parent directory
/// cannot be created or the libSQL builder rejects the path.
pub async fn open_db(path: &Path) -> Result<libsql::Database, ControlPlaneError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            ControlPlaneError::internal(
                format!("libsql_provisioner: create parent {} failed", parent.display()),
                e,
            )
        })?;
    }

    libsql::Builder::new_local(path).build().await.map_err(|e| {
        ControlPlaneError::internal(
            format!("libsql_provisioner: open {} failed", path.display()),
            e,
        )
    })
}
