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

/// Type names whose iteration order is a per-process nondeterminism
/// source — banned in `crate_class = "core"` crates per
/// `.claude/rules/development.md` § "Ordered-collection choice".
///
/// Matched on the LAST segment of any `ExprPath` / `TypePath` the visitor
/// walks. This catches every common reference shape: bare `HashMap<K, V>`,
/// `collections::HashMap`, `std::collections::HashMap`, and the
/// expression form `HashMap::new()`.
///
/// # Escape hatch
///
/// A use site documented with `// dst-lint: hashmap-ok <reason>` on the
/// preceding line (or as a trailing same-line comment) is permitted —
/// see [`collect_hashmap_ok_lines`]. The justification text after the
/// keyword is mandatory in the rule but the scanner accepts any
/// trailing content (the reason is for human reviewers).
///
/// The defect class this catches is documented in
/// `docs/feature/fix-eval-broker-drain-determinism/deliver/rca-context.md`
/// (RCA root cause #5) and was landed as a project-wide rule in commit
/// `e50146a` (Step-ID 01-06).
pub const BANNED_TYPES: &[(&str, &str)] = &[("HashMap", "BTreeMap"), ("HashSet", "BTreeSet")];

/// Marker comment that suppresses [`BANNED_TYPES`] violations on the
/// next line (or the same line as a trailing comment). Format is fixed:
/// `// dst-lint: hashmap-ok` followed optionally by ` <reason>`.
const HASHMAP_OK_MARKER: &str = "dst-lint: hashmap-ok";

/// What kind of banned reference was found.
///
/// `BannedKind::Api` covers function / type symbols banned because the
/// real implementation is a determinism boundary (`Instant::now`,
/// `tokio::net::TcpStream`, etc.) — replacement is a trait under
/// dependency injection. `BannedKind::OrderedCollection` covers
/// `HashMap` / `HashSet`, banned because their iteration order is a
/// per-process nondeterminism source — replacement is the
/// corresponding ordered collection (`BTreeMap`, `BTreeSet`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannedKind {
    /// Determinism-boundary API listed in [`BANNED_APIS`].
    Api,
    /// Ordered-collection type listed in [`BANNED_TYPES`].
    OrderedCollection,
}

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
    /// The replacement trait or type the crate should use instead.
    pub replacement_trait: String,
    /// Which kind of ban this is — disambiguates the help text rendered
    /// in the stderr block.
    pub kind: BannedKind,
}

/// Entry point for a scan.
///
/// Takes `source` and a human-readable `file` label, returns every
/// violation found. Parse errors bubble up as `Err` so callers can
/// distinguish "file didn't parse" from "file was clean".
pub fn scan_source(source: &str, file: impl AsRef<Path>) -> Result<Vec<Violation>> {
    let file = file.as_ref().to_path_buf();
    let parsed = syn::parse_file(source).with_context(|| format!("parse {}", file.display()))?;
    let hashmap_ok_lines = collect_hashmap_ok_lines(source);
    let mut collector = Collector::new(&file, hashmap_ok_lines);
    collector.visit_file(&parsed);
    Ok(collector.violations)
}

/// Pre-pass over raw source text — return the 1-based line numbers
/// that carry a `// dst-lint: hashmap-ok …` marker comment.
///
/// `syn` strips comments during parsing, so we cannot recover the
/// marker from the AST. A per-line text scan is sufficient because the
/// marker syntax is fixed: `//` followed by the [`HASHMAP_OK_MARKER`]
/// constant, with arbitrary trailing reason text.
///
/// A use site on line N is permitted if either line `N - 1` or line
/// `N` carries the marker (preceding-line and trailing same-line forms,
/// matching the `#[allow(...)]` attribute convention reviewers expect).
fn collect_hashmap_ok_lines(source: &str) -> std::collections::BTreeSet<usize> {
    // dst-lint: hashmap-ok the marker set is point-accessed only via
    // `contains`, never iterated; BTreeSet keeps determinism by default
    // even though correctness here would tolerate HashSet.
    let mut out = std::collections::BTreeSet::new();
    for (idx, line) in source.lines().enumerate() {
        if let Some(comment_start) = line.find("//") {
            let comment_body = &line[comment_start + 2..];
            // Trim leading whitespace from the comment body so
            // `// dst-lint: hashmap-ok` and `//   dst-lint: hashmap-ok`
            // both match.
            if comment_body.trim_start().starts_with(HASHMAP_OK_MARKER) {
                out.insert(idx + 1); // lines are 1-based externally.
            }
        }
    }
    out
}

// -----------------------------------------------------------------------------
// Visitor
// -----------------------------------------------------------------------------

/// `syn::visit::Visit` implementation that walks every path expression,
/// path type, and `use` tree, matching segments against [`BANNED_APIS`]
/// and the LAST segment against [`BANNED_TYPES`].
struct Collector<'a> {
    file: &'a Path,
    violations: Vec<Violation>,
    /// 1-based line numbers carrying a `// dst-lint: hashmap-ok` marker.
    /// A [`BannedKind::OrderedCollection`] violation on line N is
    /// suppressed if `N - 1` or `N` is in this set.
    hashmap_ok_lines: std::collections::BTreeSet<usize>,
}

impl<'a> Collector<'a> {
    const fn new(file: &'a Path, hashmap_ok_lines: std::collections::BTreeSet<usize>) -> Self {
        Self { file, violations: Vec::new(), hashmap_ok_lines }
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
                    kind: BannedKind::Api,
                });
                // Stop after first match for this path — one banned
                // path cannot be two different banned entries.
                return;
            }
        }

        // [`BANNED_TYPES`] match on the LAST segment of the source path.
        // This catches every common reference shape (`HashMap<K, V>`,
        // `collections::HashMap`, `std::collections::HashMap`,
        // `HashMap::new()`) without the suffix-alignment caveat that
        // makes [`path_matches`] reject single-leading-segment forms
        // like bare `HashMap::new()`.
        if let Some(last) = segments.last() {
            for (banned, replacement) in BANNED_TYPES {
                if last == banned {
                    if self.is_hashmap_ok_suppressed(line) {
                        return;
                    }
                    self.violations.push(Violation {
                        file: self.file.to_path_buf(),
                        line,
                        column,
                        banned_path: (*banned).to_string(),
                        replacement_trait: (*replacement).to_string(),
                        kind: BannedKind::OrderedCollection,
                    });
                    return;
                }
            }
        }
    }

    /// Is the given 1-based line covered by a preceding-line or
    /// same-line `// dst-lint: hashmap-ok` marker?
    fn is_hashmap_ok_suppressed(&self, line: usize) -> bool {
        self.hashmap_ok_lines.contains(&line)
            || (line > 0 && self.hashmap_ok_lines.contains(&(line - 1)))
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
             Add one of: core | adapter-host | adapter-sim | binary. \
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
    let (header, help, note) = match v.kind {
        BannedKind::Api => (
            "error: banned API used in core crate",
            format!("use {}::... via dependency injection instead", v.replacement_trait),
            "see .claude/rules/development.md (\"Reconciler I/O\" / banned APIs)",
        ),
        BannedKind::OrderedCollection => (
            "error: banned ordered-collection type used in core crate",
            format!(
                "use {} instead, or document the choice with `// dst-lint: hashmap-ok <reason>`",
                v.replacement_trait,
            ),
            "see .claude/rules/development.md (\"Ordered-collection choice\")",
        ),
    };
    format!(
        "{header}\n  \
         --> {file}:{line}:{col}\n  \
         |\n  \
         |    {banned}\n  \
         |\n  \
         = help: {help}\n  \
         = note: {note}\n",
        file = v.file.display(),
        line = v.line,
        col = v.column,
        banned = v.banned_path,
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
// Structural inspector — JobLifecycle::reconcile body
// -----------------------------------------------------------------------------
//
// Scenario 3.3 of phase-1-first-workload Slice 3A.2: the `JobLifecycle`
// reconciler's `reconcile` body must contain no banned API call,
// including `.await` (which dst-lint's project-wide scanner does NOT
// catch — the rule is "no `.await` inside `reconcile`" per ADR-0013 §2,
// not a banned-symbol rule). This inspector walks the impl block, finds
// the `fn reconcile` body, and asserts the absence of:
//
//   - `Instant::now` / `SystemTime::now`
//   - `tokio::time::sleep` / `std::thread::sleep`
//   - `rand::*`
//   - `.await` expressions
//
// Run as a `#[cfg(test)]` unit test in this crate; fails with a
// non-zero exit code if any banned construct appears in the body.

/// Banned constructs inside a `reconcile` body. Mirrors the
/// `BANNED_APIS` shape but adds `.await` (an expression form, not a
/// path) and `rand::*` (any segment-suffix match against `rand`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileBodyViolation {
    /// The kind of forbidden construct.
    pub kind: ReconcileBodyViolationKind,
    /// 1-based line within the source file.
    pub line: usize,
    /// 1-based column within the source line.
    pub column: usize,
}

/// Categorical kinds of forbidden constructs in `reconcile` bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileBodyViolationKind {
    /// `.await` expression.
    Await,
    /// Banned API path (e.g. `Instant::now`).
    BannedApi {
        /// The banned path.
        path: String,
    },
    /// `rand::*` reference.
    Rand,
}

/// Inspector visitor — walks an impl-block body looking for forbidden
/// constructs.
struct ReconcileBodyInspector {
    violations: Vec<ReconcileBodyViolation>,
}

impl<'ast> Visit<'ast> for ReconcileBodyInspector {
    fn visit_expr_await(&mut self, node: &'ast syn::ExprAwait) {
        let span = node.await_token.span;
        let start = span.start();
        self.violations.push(ReconcileBodyViolation {
            kind: ReconcileBodyViolationKind::Await,
            line: start.line,
            column: start.column + 1,
        });
        // Continue walking inner expressions.
        visit::visit_expr_await(self, node);
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        let segments: Vec<String> =
            node.path.segments.iter().map(|s| s.ident.to_string()).collect();
        self.check_segments(&segments, &node.path.segments[0].ident.span());
        visit::visit_expr_path(self, node);
    }

    fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
        let segments: Vec<String> =
            node.path.segments.iter().map(|s| s.ident.to_string()).collect();
        self.check_segments(&segments, &node.path.segments[0].ident.span());
        visit::visit_type_path(self, node);
    }
}

impl ReconcileBodyInspector {
    fn check_segments(&mut self, segments: &[String], span: &proc_macro2::Span) {
        let joined = segments.join("::");
        let start = span.start();
        let line = start.line;
        let column = start.column + 1;

        // `rand::*` — any path with `rand` as the leading or sole
        // segment (`rand::random`, `rand::thread_rng`, `rand::Rng`, ...)
        if segments.first().map(String::as_str) == Some("rand") {
            self.violations.push(ReconcileBodyViolation {
                kind: ReconcileBodyViolationKind::Rand,
                line,
                column,
            });
            return;
        }

        for (banned, _replacement) in BANNED_APIS {
            if path_matches(&joined, banned) {
                self.violations.push(ReconcileBodyViolation {
                    kind: ReconcileBodyViolationKind::BannedApi {
                        path: (*banned).to_string(),
                    },
                    line,
                    column,
                });
                return;
            }
        }
    }
}

/// Inspect a Rust source file for any `JobLifecycle::reconcile` body
/// that contains a banned construct.
///
/// Returns a list of every forbidden construct found inside the
/// `fn reconcile` body of the `impl Reconciler for JobLifecycle`
/// block. An empty Vec means the body is clean.
///
/// # Errors
///
/// Propagates any `syn::parse_file` failure as `Err`.
pub fn inspect_job_lifecycle_reconcile_body(
    source: &str,
) -> Result<Vec<ReconcileBodyViolation>> {
    let parsed = syn::parse_file(source).context("parse source")?;
    let mut found_impl = false;
    let mut violations = Vec::new();

    for item in &parsed.items {
        let syn::Item::Impl(item_impl) = item else { continue };

        // Match `impl Reconciler for JobLifecycle`.
        let Some((_, trait_path, _)) = &item_impl.trait_ else { continue };
        let trait_name = trait_path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        if trait_name != "Reconciler" {
            continue;
        }

        let syn::Type::Path(type_path) = &*item_impl.self_ty else { continue };
        let self_name = type_path
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        if self_name != "JobLifecycle" {
            continue;
        }

        found_impl = true;

        // Walk every fn item for `reconcile`.
        for impl_item in &item_impl.items {
            let syn::ImplItem::Fn(fn_item) = impl_item else { continue };
            if fn_item.sig.ident != "reconcile" {
                continue;
            }
            let mut inspector = ReconcileBodyInspector { violations: Vec::new() };
            inspector.visit_block(&fn_item.block);
            violations.extend(inspector.violations);
        }
    }

    if !found_impl {
        bail!("could not locate `impl Reconciler for JobLifecycle` in source");
    }

    Ok(violations)
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

    // -------------------------------------------------------------
    // Reconcile-body inspector tests (scenario 3.3)
    // -------------------------------------------------------------

    /// Path to the real `overdrive-core` reconciler source — relative
    /// to `xtask/Cargo.toml`'s manifest dir, which is `xtask/`.
    fn real_core_reconciler_source_path() -> std::path::PathBuf {
        let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        crate_dir
            .parent()
            .expect("xtask crate lives directly under workspace root")
            .join("crates/overdrive-core/src/reconciler.rs")
    }

    #[test]
    fn inspector_flags_await_inside_reconcile_body() {
        let source = r#"
            pub trait Reconciler {}
            pub struct JobLifecycle;
            impl Reconciler for JobLifecycle {
                fn reconcile(&self) {
                    let _x = some_future().await;
                }
            }
        "#;
        let violations =
            inspect_job_lifecycle_reconcile_body(source).expect("source must parse");
        assert!(
            violations
                .iter()
                .any(|v| matches!(v.kind, ReconcileBodyViolationKind::Await)),
            ".await must be flagged; got {violations:?}"
        );
    }

    #[test]
    fn inspector_flags_instant_now_inside_reconcile_body() {
        let source = r#"
            pub trait Reconciler {}
            pub struct JobLifecycle;
            impl Reconciler for JobLifecycle {
                fn reconcile(&self) {
                    let _ = std::time::Instant::now();
                }
            }
        "#;
        let violations =
            inspect_job_lifecycle_reconcile_body(source).expect("source must parse");
        assert!(
            violations.iter().any(|v| matches!(
                &v.kind,
                ReconcileBodyViolationKind::BannedApi { path } if path.contains("Instant::now")
            )),
            "Instant::now must be flagged; got {violations:?}"
        );
    }

    #[test]
    fn inspector_flags_rand_inside_reconcile_body() {
        let source = r#"
            pub trait Reconciler {}
            pub struct JobLifecycle;
            impl Reconciler for JobLifecycle {
                fn reconcile(&self) {
                    let _x = rand::random::<u64>();
                }
            }
        "#;
        let violations =
            inspect_job_lifecycle_reconcile_body(source).expect("source must parse");
        assert!(
            violations.iter().any(|v| matches!(v.kind, ReconcileBodyViolationKind::Rand)),
            "rand::* must be flagged; got {violations:?}"
        );
    }

    #[test]
    fn inspector_passes_clean_body() {
        let source = r#"
            pub trait Reconciler {}
            pub struct JobLifecycle;
            impl Reconciler for JobLifecycle {
                fn reconcile(&self, tick: &TickContext) -> Vec<u8> {
                    let _now = tick.now;
                    vec![]
                }
            }
        "#;
        let violations =
            inspect_job_lifecycle_reconcile_body(source).expect("source must parse");
        assert!(
            violations.is_empty(),
            "clean body must produce zero violations; got {violations:?}"
        );
    }

    /// Scenario 3.3 — the real `JobLifecycle::reconcile` body inside
    /// `crates/overdrive-core/src/reconciler.rs` must contain no
    /// banned construct.
    #[test]
    fn job_lifecycle_reconcile_body_passes_dst_lint() {
        let path = real_core_reconciler_source_path();
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let violations = inspect_job_lifecycle_reconcile_body(&source)
            .expect("overdrive-core reconciler.rs must parse");
        assert!(
            violations.is_empty(),
            "JobLifecycle::reconcile body must contain no banned construct \
             (.await, Instant::now, SystemTime::now, rand::*, tokio::time::sleep, \
             std::thread::sleep); found {} violation(s): {:#?}",
            violations.len(),
            violations
        );
    }
}
