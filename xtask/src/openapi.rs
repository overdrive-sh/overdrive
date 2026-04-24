//! `cargo xtask openapi-gen` / `openapi-check` — OpenAPI schema CI gate
//! per ADR-0009.
//!
//! `generate_yaml` renders the live `utoipa::OpenApi`-derived schema
//! from `overdrive-control-plane` to YAML. `check_against_disk` compares
//! the live render against `api/openapi.yaml` at the workspace root and
//! fails with an actionable message when they diverge, naming the first
//! drifted schema and suggesting `cargo xtask openapi-gen` to
//! regenerate. `openapi_gen` / `openapi_check` are the subcommand-level
//! wrappers invoked from `src/main.rs`.
//!
//! Determinism: utoipa 5.x sorts paths and component schemas, so the
//! YAML output is byte-identical across repeat invocations. The
//! acceptance tests assert this.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, WrapErr, bail};
use utoipa::OpenApi as _;

/// Checked-in OpenAPI document path, relative to the workspace root.
/// Per ADR-0009 this lives at the top-level `api/` directory.
pub const OPENAPI_YAML_PATH: &str = "api/openapi.yaml";

/// Render the live OpenAPI schema to YAML.
///
/// The schema is sourced from
/// [`overdrive_control_plane::api::OverdriveApi`]. Output is
/// byte-identical across repeat invocations because utoipa 5.x sorts
/// paths and schemas.
pub fn generate_yaml() -> Result<String> {
    overdrive_control_plane::api::OverdriveApi::openapi()
        .to_yaml()
        .wrap_err("render OverdriveApi::openapi() to YAML")
}

/// Compare the live OpenAPI YAML against a checked-in reference file.
///
/// Returns `Ok(())` iff the live render matches the file byte-for-byte.
/// On drift, returns an error whose `Display` names the first divergent
/// schema / path and suggests `cargo xtask openapi-gen` to regenerate.
pub fn check_against_disk(path: &Path) -> Result<()> {
    let live = generate_yaml()?;
    let on_disk = std::fs::read_to_string(path)
        .wrap_err_with(|| format!("read on-disk OpenAPI YAML at {}", path.display()))?;

    if live == on_disk {
        return Ok(());
    }

    let drifted = first_drift(&live, &on_disk);
    bail!(
        "OpenAPI schema drift detected at {}: first divergence near {}. \
         Run `cargo xtask openapi-gen` to regenerate the checked-in YAML.",
        path.display(),
        drifted,
    );
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
            let line = lines[back];
            if let Some(name) = schema_or_path_name(line) {
                return Some(name);
            }
        }
        None
    };

    let header = scan(live).or_else(|| scan(disk)).unwrap_or_else(|| "<no header>".to_string());
    let l = live.get(idx).copied().unwrap_or("<eof>");
    let d = disk.get(idx).copied().unwrap_or("<eof>");
    format!("`{header}` at line {} (live=`{}` vs on-disk=`{}`)", idx + 1, l.trim(), d.trim(),)
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

/// `cargo xtask openapi-gen` — regenerate `api/openapi.yaml` at the
/// workspace root.
pub fn openapi_gen() -> Result<()> {
    let yaml = generate_yaml()?;
    let path = workspace_root()?.join(OPENAPI_YAML_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .wrap_err_with(|| format!("create parent directory {}", parent.display()))?;
    }
    std::fs::write(&path, yaml)
        .wrap_err_with(|| format!("write OpenAPI YAML to {}", path.display()))?;
    eprintln!("xtask: wrote {}", path.display());
    Ok(())
}

/// `cargo xtask openapi-check` — verify `api/openapi.yaml` matches the
/// live schema.
pub fn openapi_check() -> Result<()> {
    let path = workspace_root()?.join(OPENAPI_YAML_PATH);
    check_against_disk(&path)
}

/// Resolve the workspace root from the current working directory. The
/// subcommand is invoked from the workspace root (both via
/// `cargo xtask` and via the subprocess smoke test); the helper exists
/// so downstream error messages name the discovered root rather than
/// the opaque `.` form of the path.
fn workspace_root() -> Result<PathBuf> {
    std::env::current_dir().wrap_err("determine workspace root from current_dir")
}
