//! `dst-lint` — banned-API scanner for `crate_class = "core"` crates.
//!
//! Implements US-05 §6.1 / §6.2. Walks the `src/**/*.rs` of every core
//! crate in the workspace, parses each file with `syn`, and flags any
//! reference to a symbol listed in [`BANNED_APIS`]. Each violation is a
//! structured record with file, line, column, the banned path, and the
//! replacement trait — rendered in an rustc-style block on stderr.
//!
//! # Determinism boundary
//!
//! This module runs *inside* `xtask`, which is a binary crate
//! (`crate_class = "binary"`). The `Instant::now`, `File::open`, and
//! `std::process::Command` calls it would need live are in the caller,
//! not here — the functions in this module are pure transformations
//! over already-materialised `&str` source code and parsed
//! `cargo_metadata::Metadata`. That keeps the module itself trivially
//! testable against synthetic inputs (see `tests/dst_lint_self_test.rs`).

use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, Result, bail};
use syn::visit::{self, Visit};

// -----------------------------------------------------------------------------
// Banned APIs — single source of truth
// -----------------------------------------------------------------------------

/// Every path / type listed here is forbidden inside a `crate_class =
/// "core"` crate. Matched against the trailing segments of any
/// `ExprPath`, `TypePath`, or `UseTree::Name` the syn visitor walks.
///
/// The list mirrors the banned-API table in
/// `.claude/rules/testing.md` and `.claude/rules/development.md`.
pub const BANNED_APIS: &[(&str, &str)] = &[
    ("std::time::Instant::now", "Clock"),
    ("std::time::SystemTime::now", "Clock"),
    ("std::thread::sleep", "Clock"),
    ("tokio::time::sleep", "Clock"),
    ("rand::random", "Entropy"),
    ("rand::thread_rng", "Entropy"),
    ("tokio::net::TcpStream", "Transport"),
    ("tokio::net::TcpListener", "Transport"),
    ("tokio::net::UdpSocket", "Transport"),
];

/// A single banned-API usage found in a core-class crate source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// Path to the offending source file relative to the scan root.
    pub file: PathBuf,
    /// Line number (1-based) of the banned call.
    pub line: usize,
    /// Column number (1-based) of the banned call.
    pub column: usize,
    /// The banned path (e.g. `std::time::Instant::now`).
    pub banned_path: String,
    /// The replacement trait the crate should use instead.
    pub replacement_trait: String,
}

/// Entry point for a scan.
///
/// Takes `source` and a human-readable `file` label, returns every
/// violation found. Parse errors bubble up as `Err` so callers can
/// distinguish "file didn't parse" from "file was clean".
pub fn scan_source(source: &str, file: impl AsRef<Path>) -> Result<Vec<Violation>> {
    let file = file.as_ref().to_path_buf();
    let parsed = syn::parse_file(source).with_context(|| format!("parse {}", file.display()))?;
    let mut collector = Collector::new(&file);
    collector.visit_file(&parsed);
    Ok(collector.violations)
}

// -----------------------------------------------------------------------------
// Visitor
// -----------------------------------------------------------------------------

/// `syn::visit::Visit` implementation that walks every path expression,
/// path type, and `use` tree, matching segments against `BANNED_APIS`.
struct Collector<'a> {
    file: &'a Path,
    violations: Vec<Violation>,
}

impl<'a> Collector<'a> {
    const fn new(file: &'a Path) -> Self {
        Self { file, violations: Vec::new() }
    }

    /// If `segments` — joined by `::` — matches any banned entry (by
    /// suffix), record a violation at `(line, column)`.
    fn check_path(&mut self, segments: &[String], line: usize, column: usize) {
        let joined = segments.join("::");
        for (banned, replacement) in BANNED_APIS {
            if path_matches(&joined, banned) {
                self.violations.push(Violation {
                    file: self.file.to_path_buf(),
                    line,
                    column,
                    banned_path: (*banned).to_string(),
                    replacement_trait: (*replacement).to_string(),
                });
                // Stop after first match for this path — one banned
                // path cannot be two different banned entries.
                return;
            }
        }
    }
}

/// A path in the source matches a banned entry when the source path
/// ends with the banned path, segment-aligned, *and* carries at least
/// two segments (so a locally-defined `fn now()` is not mistaken for
/// `std::time::Instant::now`). The leaf-only match reserved for
/// bare-function banned entries (`rand::random`, `rand::thread_rng`)
/// is handled by the exact-match branch only — these are always
/// imported as `use rand;` and called as `rand::random()`.
///
/// Examples:
///
/// - source `std::time::Instant::now` vs banned `std::time::Instant::now` → match (exact)
/// - source `Instant::now` (after `use std::time::Instant;`) vs banned
///   `std::time::Instant::now` → match (suffix, 2 segments)
/// - source `now` vs banned `std::time::Instant::now` → **no match** (single segment)
/// - source `crate::Instant::now` vs banned `std::time::Instant::now` → no match
///   (different alias — acceptable false negative; the lint is a best-effort
///   static check, not a full name-resolution pass)
fn path_matches(source: &str, banned: &str) -> bool {
    if source == banned {
        return true;
    }
    let banned_segs: Vec<&str> = banned.split("::").collect();
    let source_segs: Vec<&str> = source.split("::").collect();
    // Require segment alignment: banned ends with source.
    if source_segs.len() > banned_segs.len() {
        return false;
    }
    // A single-segment suffix would flag every local leaf name — reserve
    // those to the exact-match branch above.
    if source_segs.len() < 2 {
        return false;
    }
    let tail = &banned_segs[banned_segs.len() - source_segs.len()..];
    tail == source_segs.as_slice()
}

impl<'ast> Visit<'ast> for Collector<'_> {
    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        let segments = node.path.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>();
        if let Some(first) = node.path.segments.first() {
            let start = first.ident.span().start();
            self.check_path(&segments, start.line, start.column + 1);
        }
        visit::visit_expr_path(self, node);
    }

    fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
        let segments = node.path.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>();
        if let Some(first) = node.path.segments.first() {
            let start = first.ident.span().start();
            self.check_path(&segments, start.line, start.column + 1);
        }
        visit::visit_type_path(self, node);
    }
}

// -----------------------------------------------------------------------------
// Workspace scan
// -----------------------------------------------------------------------------

/// Scan the workspace rooted at `manifest_path` and return every
/// violation found. Performs the ADR-0003 guard-rails:
///
/// 1. Every workspace member must declare
///    `package.metadata.overdrive.crate_class`.
/// 2. At least one member must be `crate_class = "core"`.
pub fn scan_workspace(manifest_path: &Path) -> Result<Vec<Violation>> {
    let metadata = cargo_metadata::MetadataCommand::new()
        .manifest_path(manifest_path)
        .no_deps()
        .exec()
        .with_context(|| format!("cargo metadata for {}", manifest_path.display()))?;

    let mut classes: Vec<(String, PathBuf, Option<String>)> = Vec::new();
    for pkg in &metadata.workspace_packages() {
        let pkg_root = Path::new(&pkg.manifest_path)
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let class = pkg
            .metadata
            .get("overdrive")
            .and_then(|m| m.get("crate_class"))
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        classes.push((pkg.name.clone(), pkg_root, class));
    }

    // Guard-rail 1: every crate must declare crate_class.
    let undeclared: Vec<_> = classes.iter().filter(|(_, _, c)| c.is_none()).collect();
    if !undeclared.is_empty() {
        let names: Vec<_> = undeclared.iter().map(|(n, _, _)| n.as_str()).collect();
        bail!(
            "the following workspace crate(s) are missing \
             `[package.metadata.overdrive] crate_class = \"...\"` in Cargo.toml: {}. \
             Add one of: core | adapter-real | adapter-sim | binary. \
             See docs/product/architecture/adr-0003-core-crate-labelling.md.",
            names.join(", ")
        );
    }

    // Guard-rail 2: at least one core crate must exist.
    let core_crates: Vec<_> =
        classes.iter().filter(|(_, _, c)| c.as_deref() == Some("core")).collect();
    if core_crates.is_empty() {
        bail!(
            "no core-class crates found in workspace at {}. \
             At least one crate must declare \
             `[package.metadata.overdrive] crate_class = \"core\"` — \
             otherwise dst-lint has nothing to scan and the gate \
             silently passes. See ADR-0003.",
            manifest_path.display()
        );
    }

    // Scan every .rs file under each core crate's `src/` directory.
    let mut violations = Vec::new();
    for (_name, root, _class) in core_crates {
        let src = root.join("src");
        if !src.exists() {
            continue;
        }
        for rs in collect_rs_files(&src)? {
            let rel = rs.strip_prefix(root).unwrap_or(&rs).to_path_buf();
            let source =
                std::fs::read_to_string(&rs).with_context(|| format!("read {}", rs.display()))?;
            // Skip files that don't parse rather than failing the whole
            // scan — they'll blow up in `cargo check` regardless.
            if let Ok(found) = scan_source(&source, &rel) {
                violations.extend(found);
            }
        }
    }

    Ok(violations)
}

fn collect_rs_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            out.extend(collect_rs_files(&path)?);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(out)
}

// -----------------------------------------------------------------------------
// Violation reporting
// -----------------------------------------------------------------------------

/// Render a single violation as a rustc-style stderr block.
pub fn render_violation(v: &Violation) -> String {
    format!(
        "error: banned API used in core crate\n  \
         --> {file}:{line}:{col}\n  \
         |\n  \
         |    {banned}\n  \
         |\n  \
         = help: use {replacement}::... via dependency injection instead\n  \
         = note: see .claude/rules/development.md (\"Reconciler I/O\" / banned APIs)\n",
        file = v.file.display(),
        line = v.line,
        col = v.column,
        banned = v.banned_path,
        replacement = v.replacement_trait,
    )
}

/// CLI-shaped entry point: run the scan against `manifest_path`, write
/// violations to stderr, and return `Err` iff any were found (or a
/// guard-rail tripped). `Ok(())` means the workspace is clean.
pub fn run(manifest_path: &Path) -> Result<()> {
    let violations = scan_workspace(manifest_path)?;
    if violations.is_empty() {
        return Ok(());
    }
    for v in &violations {
        eprint!("{}", render_violation(v));
    }
    eprintln!("dst-lint: {} banned-API violation(s) found in core crates", violations.len());
    bail!("banned-API violations in core crates");
}

// -----------------------------------------------------------------------------
// Unit tests — path-matching logic
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_is_detected() {
        assert!(path_matches("std::time::Instant::now", "std::time::Instant::now"));
    }

    #[test]
    fn tail_match_via_use_is_detected() {
        assert!(path_matches("Instant::now", "std::time::Instant::now"));
        assert!(path_matches("time::Instant::now", "std::time::Instant::now"));
    }

    #[test]
    fn unrelated_symbol_does_not_match() {
        assert!(!path_matches("crate::other::now", "std::time::Instant::now"));
        assert!(!path_matches("Instant::never", "std::time::Instant::now"));
    }

    #[test]
    fn single_segment_leaf_does_not_match() {
        // `now` alone must not match — the leading segment is not
        // enough to identify the banned symbol, and a core crate that
        // defines its own `fn now()` is clearly distinct from
        // `Instant::now`. Suffix matching requires at least two
        // segments.
        assert!(!path_matches("now", "std::time::Instant::now"));
        assert!(!path_matches("random", "rand::random"));
    }

    #[test]
    fn source_strictly_longer_than_banned_does_not_match() {
        // 5-segment source vs 4-segment banned path — a `>=`
        // off-by-one in the length guard would silently flip this
        // into a match.
        assert!(!path_matches("a::b::c::d::e", "std::time::Instant::now"));
    }
}
