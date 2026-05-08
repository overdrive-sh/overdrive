//! `cargo openapi-gen` / `openapi-check` library — `OpenAPI` schema CI
//! gate per ADR-0009.
//!
//! [`generate_yaml`] renders the live `utoipa::OpenApi`-derived schema
//! from this crate's [`crate::api::OverdriveApi`] to YAML.
//! [`check_against_disk`] compares the live render against
//! `api/openapi.yaml` at the workspace root and fails with an actionable
//! [`OpenApiError::Drift`] when they diverge, naming the first drifted
//! schema and suggesting `cargo openapi-gen` to regenerate.
//!
//! The [`crate::bin::openapi`] binary (`crates/overdrive-control-plane/
//! src/bin/openapi.rs`) is the CLI plumbing that calls these functions
//! from the workspace cargo alias.
//!
//! Determinism: utoipa 5.x sorts paths and component schemas, so the
//! YAML output is byte-identical across repeat invocations. The
//! integration tests in `tests/openapi_gate.rs` assert this.

use std::path::{Path, PathBuf};

use utoipa::OpenApi as _;

/// Checked-in `OpenAPI` document path, relative to the workspace root.
/// Per ADR-0009 this lives at the top-level `api/` directory.
pub const OPENAPI_YAML_PATH: &str = "api/openapi.yaml";

/// Typed error for OpenAPI gate operations. Library boundary —
/// callers branch on variant; the binary in `bin/openapi.rs` converts
/// to `eyre::Report` via `?`.
#[derive(Debug, thiserror::Error)]
pub enum OpenApiError {
    /// `utoipa::OpenApi::to_yaml` rejected the schema (should not happen
    /// in practice — utoipa's serialisation surface is closed) but
    /// surface as a typed variant rather than a panic. Carries the
    /// upstream serialiser's error as `String` to avoid binding this
    /// crate's public error surface to whichever YAML serialiser
    /// `utoipa` happens to use internally (currently `serde_norway`).
    #[error("render OverdriveApi::openapi() to YAML: {0}")]
    Render(String),

    /// The on-disk `api/openapi.yaml` file could not be read.
    #[error("read on-disk OpenAPI YAML at {path}: {source}")]
    ReadOnDisk { path: PathBuf, source: std::io::Error },

    /// The on-disk YAML differs from the live render. The `anchor`
    /// names the closest preceding schema-or-path header and the
    /// raw diverging line on each side, so the operator can locate
    /// the drift without diffing the whole file.
    #[error(
        "OpenAPI schema drift detected at {path}: first divergence near {anchor}. \
         Run `cargo openapi-gen` to regenerate the checked-in YAML."
    )]
    Drift { path: PathBuf, anchor: String },
}

/// Result alias matching the workspace convention.
pub type Result<T, E = OpenApiError> = std::result::Result<T, E>;

/// Render the live `OpenAPI` schema to YAML.
///
/// The schema is sourced from [`crate::api::OverdriveApi`]. Output is
/// byte-identical across repeat invocations because utoipa 5.x sorts
/// paths and schemas.
pub fn generate_yaml() -> Result<String> {
    crate::api::OverdriveApi::openapi().to_yaml().map_err(|e| OpenApiError::Render(e.to_string()))
}

/// Compare the live `OpenAPI` YAML against a checked-in reference file.
///
/// Returns `Ok(())` iff the live render matches the file byte-for-byte.
/// On drift, returns [`OpenApiError::Drift`] whose `Display` names the
/// first divergent schema / path and suggests `cargo openapi-gen` to
/// regenerate.
pub fn check_against_disk(path: &Path) -> Result<()> {
    let live = generate_yaml()?;
    let on_disk = std::fs::read_to_string(path)
        .map_err(|source| OpenApiError::ReadOnDisk { path: path.to_path_buf(), source })?;

    if live == on_disk {
        return Ok(());
    }

    Err(OpenApiError::Drift { path: path.to_path_buf(), anchor: first_drift(&live, &on_disk) })
}

/// Find the first line-level difference between `live` and `on_disk`
/// and return a human-readable anchor for it. The anchor surfaces the
/// diverging content itself — including whatever schema name or path
/// appears on or immediately above the divergent line. The returned
/// string always contains:
///
/// - The raw diverging line from both sides (so the operator can eyeball
///   the diff), and
/// - The closest preceding schema-header or path-header anchor (so the
///   drift location is named).
fn first_drift(live: &str, on_disk: &str) -> String {
    let live_lines: Vec<&str> = live.lines().collect();
    let disk_lines: Vec<&str> = on_disk.lines().collect();
    let max_len = live_lines.len().max(disk_lines.len());

    for idx in 0..max_len {
        let l = live_lines.get(idx).copied().unwrap_or("<eof>");
        let d = disk_lines.get(idx).copied().unwrap_or("<eof>");
        if l != d {
            return anchor_for(&live_lines, &disk_lines, idx);
        }
    }

    // Unreachable in practice — the caller only calls us when
    // `live != on_disk`, which implies at least one differing line.
    "<unknown drift>".to_string()
}

/// Compose the drift anchor: preceding schema/path header (if any) +
/// the raw diverging lines. Including both sides of the diverging line
/// guarantees the drifted identifier appears in the message — whether
/// it drifts as a header (`  NewName:`) or as a `$ref` (`$ref:
/// '#/components/schemas/NewName'`).
fn anchor_for(live: &[&str], disk: &[&str], idx: usize) -> String {
    let scan = |lines: &[&str]| -> Option<String> {
        for back in (0..=idx.min(lines.len().saturating_sub(1))).rev() {
            let candidate = lines[back];
            if let Some(name) = schema_or_path_name(candidate) {
                return Some(name);
            }
        }
        None
    };

    let header = scan(live).or_else(|| scan(disk)).unwrap_or_else(|| "<no header>".to_string());
    let l = live.get(idx).copied().unwrap_or("<eof>");
    let d = disk.get(idx).copied().unwrap_or("<eof>");
    format!("`{header}` at line {} (live=`{}` vs on-disk=`{}`)", idx + 1, l.trim(), d.trim())
}

/// Return the schema or path name anchored by a YAML header line, or
/// `None` if the line is not a header we recognise.
///
/// Recognised shapes (indent-sensitive, matching utoipa 5.x's output):
///
/// - `  SchemaName:` at two-space indent under `components: schemas:`.
/// - `  /v1/foo:` — a path entry under `paths:`.
fn schema_or_path_name(line: &str) -> Option<String> {
    let trimmed = line.trim_end();
    // Path entry — leading spaces then a slash.
    let stripped = trimmed.strip_prefix("  ")?;
    if stripped.starts_with(' ') {
        // Deeper indent than 2 — not a top-level schema or path header.
        return None;
    }
    let without_colon = stripped.strip_suffix(':')?;
    if without_colon.is_empty() {
        return None;
    }
    // Exclude known non-schema top-level keys under `components`.
    if matches!(without_colon, "schemas" | "responses" | "parameters" | "info" | "paths") {
        return None;
    }
    Some(without_colon.to_string())
}
