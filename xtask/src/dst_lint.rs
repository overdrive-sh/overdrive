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

/// Path prefixes whose use inside an `async fn` body in an
/// `adapter-host`-class crate is forbidden because they perform
/// blocking I/O on the tokio executor. Production code must use
/// `tokio::fs::*` (preferred — same syscall, async API) or
/// `tokio::task::spawn_blocking` (escape hatch — sync closure runs on
/// the blocking pool). Mirrors `.claude/rules/development.md` §
/// Concurrency & async.
///
/// The set is matched as a *path prefix*: any source path that, after
/// segment alignment, starts with `std::fs::` is flagged. This catches
/// `std::fs::write`, `std::fs::create_dir_all`, `std::fs::read_to_string`,
/// `std::fs::File::create`, etc., without enumerating each leaf.
///
/// # Escape hatch
///
/// `tokio::task::spawn_blocking(|| { … })` runs its sync closure on the
/// blocking pool. The visitor does not flag `std::fs::*` references
/// inside a sync closure body, regardless of whether the closure is
/// lexically nested inside an `async fn` — see [`AsyncBlockingIoCollector`].
///
/// `#[cfg(test)]` items (modules and individual fns) are exempt — test
/// fixture setup may use sync `std::fs` without penalty per the rule.
/// The same exemption also covers the [`BANNED_APIS`] scanner above —
/// test helpers that synthesise `TickContext` inputs may call
/// `Instant::now`, `SystemTime::now`, etc. without flag. The
/// [`BANNED_TYPES`] (`HashMap` / `HashSet`) clause is **not** exempted:
/// iteration-order invariants per `.claude/rules/development.md` §
/// "Ordered-collection choice" still apply to test code that asserts
/// on DST trajectories.
///
/// # Why prefix instead of leaf
///
/// A prefix match keeps the rule honest as the standard library grows.
/// New blocking entry points appear (`std::fs::soft_link`,
/// `std::fs::try_exists`, …) without us updating an enum. The cost is
/// a possible false positive on a path like `my_crate::std::fs::*` —
/// acceptable since that is not a shape any sane code uses.
const BANNED_BLOCKING_IO_PREFIX: &str = "std::fs";

/// Replacement-trait label rendered in the help text for the
/// `std::fs`-in-async-fn lint clause. Conceptually paired with
/// `tokio::fs::*` and `tokio::task::spawn_blocking`, but the existing
/// `Violation` struct expects a single string.
const BANNED_BLOCKING_IO_REPLACEMENT: &str = "tokio::fs / tokio::task::spawn_blocking";

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
    /// Sync `std::fs::*` inside an `async fn` body — banned in
    /// `adapter-host`-class crates per
    /// `.claude/rules/development.md` § Concurrency & async.
    BlockingIoInAsync,
    /// `"live"` literal in CLI render or command source — banned per
    /// the `workload-kind-discriminator` Slice 01 regression guard
    /// (US-06). The literal `"live"` was used as a hard-coded
    /// duration in render output for in-progress workloads; it must
    /// be replaced with a measured duration. This rule fires only on
    /// source files under `crates/overdrive-cli/src/render.rs` or
    /// `crates/overdrive-cli/src/commands/`.
    LiveLiteralBanned,
    /// `let _probe_runner = ...` or `let _ = probe_runner` patterns
    /// inside `crates/overdrive-control-plane/src/` — banned per
    /// GAP-5 from `.context/01-03-structural-gap-audit.md`. The
    /// pre-patch binary composition root constructed an
    /// `Arc<ProbeRunner>` from the Earned-Trust gate, then discarded
    /// it via `let _probe_runner = probe_runner;` — net effect: every
    /// production `ExecDriver::on_alloc_running` /
    /// `on_alloc_terminal` call took the trait-default no-op path
    /// because the driver was constructed *before* the runner and
    /// never threaded it via `.with_probe_runner(...)`. This clause
    /// is the structural defense against re-introducing the discard
    /// shape; the production composition must go through
    /// `compose_production_driver(...)`, which threads the runner
    /// via `ExecDriver::new(...).with_probe_runner(Arc::clone(&runner))`
    /// and returns `(Arc<dyn Driver>, Arc<ProbeRunner>)` so the
    /// destructure `let (driver, _) = compose_production_driver(...)`
    /// at the binary boundary discards a positional tuple slot (a
    /// structurally distinct pattern) rather than naming a bound
    /// variable. See GAP-4 + GAP-5 closure commit for the patch.
    UnderscoreBindingProbeRunner,
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
///
/// # `#[cfg(test)]` exemption
///
/// [`BannedKind::Api`] violations are suppressed inside any
/// `#[cfg(test)]`-attributed module, free fn, or impl method. Test
/// fixtures legitimately call `Instant::now`, `SystemTime::now`, and
/// other determinism boundaries to build `TickContext` inputs — the
/// rule in `.claude/rules/development.md` § "Reconciler I/O" bans
/// wall-clock inside `reconcile` *bodies*, not inside test helpers
/// that synthesise the inputs `reconcile` receives.
///
/// [`BannedKind::OrderedCollection`] is NOT exempted — iteration-order
/// invariants from `.claude/rules/development.md` § "Ordered-collection
/// choice" still apply to test code that asserts on DST trajectories.
struct Collector<'a> {
    file: &'a Path,
    violations: Vec<Violation>,
    /// 1-based line numbers carrying a `// dst-lint: hashmap-ok` marker.
    /// A [`BannedKind::OrderedCollection`] violation on line N is
    /// suppressed if `N - 1` or `N` is in this set.
    hashmap_ok_lines: std::collections::BTreeSet<usize>,
    /// Number of `#[cfg(test)]`-attributed items currently open. When
    /// non-zero, suppress [`BannedKind::Api`] flagging — test fixtures
    /// may legitimately use wall-clock / RNG APIs to synthesise
    /// reconciler inputs.
    cfg_test_depth: usize,
}

impl<'a> Collector<'a> {
    const fn new(file: &'a Path, hashmap_ok_lines: std::collections::BTreeSet<usize>) -> Self {
        Self { file, violations: Vec::new(), hashmap_ok_lines, cfg_test_depth: 0 }
    }

    /// If `segments` — joined by `::` — matches any banned entry (by
    /// suffix), record a violation at `(line, column)`.
    fn check_path(&mut self, segments: &[String], line: usize, column: usize) {
        let joined = segments.join("::");
        for (banned, replacement) in BANNED_APIS {
            if path_matches(&joined, banned) {
                // `#[cfg(test)]` exemption — suppress BannedKind::Api
                // violations inside test modules / fns. HashMap rule
                // is handled separately below and is NOT exempted.
                if self.cfg_test_depth > 0 {
                    return;
                }
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
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if has_cfg_test_attr(&node.attrs) {
            self.cfg_test_depth += 1;
            visit::visit_item_fn(self, node);
            self.cfg_test_depth -= 1;
            return;
        }
        visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if has_cfg_test_attr(&node.attrs) {
            self.cfg_test_depth += 1;
            visit::visit_impl_item_fn(self, node);
            self.cfg_test_depth -= 1;
            return;
        }
        visit::visit_impl_item_fn(self, node);
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if has_cfg_test_attr(&node.attrs) {
            self.cfg_test_depth += 1;
            visit::visit_item_mod(self, node);
            self.cfg_test_depth -= 1;
            return;
        }
        visit::visit_item_mod(self, node);
    }

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

/// Free-function form of `AsyncBlockingIoCollector::has_cfg_test_attr`,
/// shared between the [`Collector`] (banned-API scanner) and the
/// [`AsyncBlockingIoCollector`] (`std::fs`-in-`async-fn` scanner). See
/// [`AsyncBlockingIoCollector::has_cfg_test_attr`] for the canonical
/// rustdoc — recognises only the `#[cfg(test)]` literal form to avoid
/// silently exempting production code via `#[cfg(any(test, …))]` or
/// `#[cfg(not(…))]` shapes that evaluate to false at compile time.
fn has_cfg_test_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        if !a.path().is_ident("cfg") {
            return false;
        }
        let mut found = false;
        let _ = a.parse_nested_meta(|meta| {
            if meta.path.is_ident("test") {
                found = true;
            }
            Ok(())
        });
        found
    })
}

// -----------------------------------------------------------------------------
// std::fs-in-async-fn lint clause (adapter-host crates)
// -----------------------------------------------------------------------------

/// Visitor that flags `std::fs::*` references inside an `async fn` body
/// in an `adapter-host`-class crate.
///
/// Tracks two pieces of context as it walks the AST:
///
/// - `inside_async`: the closest enclosing fn / closure / `async {}` block
///   is async. `std::fs::*` references are flagged only when this is
///   `true` AND `cfg_test_depth == 0`.
/// - `cfg_test_depth`: depth counter for `#[cfg(test)]` items currently
///   open. Tests use `std::fs` for fixture setup and the rule explicitly
///   exempts them.
///
/// The visitor relies on `syn` parsing the file pre-macro-expansion, so
/// `#[async_trait]` impls and `async fn` in trait impls (Rust 1.75+) both
/// look like a regular `async fn` to the syntactic walk.
struct AsyncBlockingIoCollector<'a> {
    file: &'a Path,
    violations: Vec<Violation>,
    /// Whether the closest enclosing fn / closure / `async {}` block is
    /// an async context.
    inside_async: bool,
    /// Number of `#[cfg(test)]`-attributed items currently open. When
    /// non-zero, suppress flagging — the rule exempts tests.
    cfg_test_depth: usize,
}

impl<'a> AsyncBlockingIoCollector<'a> {
    const fn new(file: &'a Path) -> Self {
        Self { file, violations: Vec::new(), inside_async: false, cfg_test_depth: 0 }
    }

    /// Does `attrs` carry a `#[cfg(test)]` attribute? Recognises the
    /// canonical form only — `#[cfg(test)]`. More elaborate predicates
    /// (`#[cfg(any(test, …))]`, `#[cfg(not(…))]`) intentionally are not
    /// matched, because they may evaluate to false at compile time and
    /// would silently exempt production code from the rule.
    ///
    /// Thin wrapper around the free function [`has_cfg_test_attr`],
    /// shared between this collector and the banned-API [`Collector`].
    fn has_cfg_test_attr(attrs: &[syn::Attribute]) -> bool {
        has_cfg_test_attr(attrs)
    }
}

impl<'ast> Visit<'ast> for AsyncBlockingIoCollector<'_> {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if Self::has_cfg_test_attr(&node.attrs) {
            self.cfg_test_depth += 1;
            visit::visit_item_fn(self, node);
            self.cfg_test_depth -= 1;
            return;
        }
        let was = self.inside_async;
        self.inside_async = node.sig.asyncness.is_some();
        visit::visit_item_fn(self, node);
        self.inside_async = was;
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if Self::has_cfg_test_attr(&node.attrs) {
            self.cfg_test_depth += 1;
            visit::visit_impl_item_fn(self, node);
            self.cfg_test_depth -= 1;
            return;
        }
        let was = self.inside_async;
        self.inside_async = node.sig.asyncness.is_some();
        visit::visit_impl_item_fn(self, node);
        self.inside_async = was;
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if Self::has_cfg_test_attr(&node.attrs) {
            self.cfg_test_depth += 1;
            visit::visit_item_mod(self, node);
            self.cfg_test_depth -= 1;
            return;
        }
        visit::visit_item_mod(self, node);
    }

    fn visit_expr_async(&mut self, node: &'ast syn::ExprAsync) {
        let was = self.inside_async;
        self.inside_async = true;
        visit::visit_expr_async(self, node);
        self.inside_async = was;
    }

    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        // A closure resets the async context to whatever the closure
        // *itself* declares. A sync closure inside `async fn`, e.g.
        // `tokio::task::spawn_blocking(|| { … })`, must NOT inherit the
        // surrounding async context — that closure body runs on the
        // blocking pool, which is the documented escape hatch.
        let was = self.inside_async;
        self.inside_async = node.asyncness.is_some();
        visit::visit_expr_closure(self, node);
        self.inside_async = was;
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if self.inside_async && self.cfg_test_depth == 0 {
            let segments: Vec<String> =
                node.path.segments.iter().map(|s| s.ident.to_string()).collect();
            if let Some(first) = node.path.segments.first() {
                let start = first.ident.span().start();
                self.check_blocking_io(&segments, start.line, start.column + 1);
            }
        }
        visit::visit_expr_path(self, node);
    }

    fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
        if self.inside_async && self.cfg_test_depth == 0 {
            let segments: Vec<String> =
                node.path.segments.iter().map(|s| s.ident.to_string()).collect();
            if let Some(first) = node.path.segments.first() {
                let start = first.ident.span().start();
                self.check_blocking_io(&segments, start.line, start.column + 1);
            }
        }
        visit::visit_type_path(self, node);
    }
}

impl AsyncBlockingIoCollector<'_> {
    /// Match `segments` (joined by `::`) as a *prefix* against
    /// [`BANNED_BLOCKING_IO_PREFIX`]. A prefix match catches every
    /// blocking entry point under `std::fs` — present and future —
    /// without enumerating each.
    ///
    /// Segment-aligned: `std::fs::write` matches `std::fs`; `std::fsx`
    /// (no such thing today, but) does not. The `use std::fs;`-shortened
    /// form `fs::write` also matches via the same suffix-of-banned-path
    /// rule used by [`path_matches`] — the source path `fs::write` ends
    /// with the suffix `fs::write` of the prefix `std::fs`, and segment
    /// alignment passes.
    fn check_blocking_io(&mut self, segments: &[String], line: usize, column: usize) {
        let joined = segments.join("::");
        if blocking_io_path_matches(&joined, BANNED_BLOCKING_IO_PREFIX) {
            self.violations.push(Violation {
                file: self.file.to_path_buf(),
                line,
                column,
                banned_path: joined,
                replacement_trait: BANNED_BLOCKING_IO_REPLACEMENT.to_owned(),
                kind: BannedKind::BlockingIoInAsync,
            });
        }
    }
}

/// Does a source path overlap the blocking-io prefix?
///
/// The check is two-direction:
///
/// 1. The source path ends with `prefix` (e.g. source `std::fs` matches
///    prefix `std::fs`; source `fs` after `use std::fs;` also matches
///    via the suffix-alignment rule used by [`path_matches`]).
/// 2. The source path *starts with* `prefix` followed by `::` (e.g.
///    source `std::fs::write` starts with `std::fs::`).
///
/// Either match flags the path. Single-segment leaves like `fs` alone
/// are NOT flagged — too easy to alias-clash with a local `fs` module.
fn blocking_io_path_matches(source: &str, prefix: &str) -> bool {
    if path_matches(source, prefix) {
        return true;
    }
    // Prefix match — `std::fs::write` starts with `std::fs::`.
    let with_sep = format!("{prefix}::");
    if source.starts_with(&with_sep) {
        return true;
    }
    // Suffix-of-prefix form: `use std::fs; … fs::write(…)` — source
    // path `fs::write` whose head segment `fs` is the trailing segment
    // of the banned prefix.
    let prefix_segs: Vec<&str> = prefix.split("::").collect();
    let source_segs: Vec<&str> = source.split("::").collect();
    if let Some(last_prefix_seg) = prefix_segs.last() {
        if source_segs.len() >= 2 && source_segs.first() == Some(last_prefix_seg) {
            return true;
        }
    }
    false
}

/// Scan a single source file for the `std::fs::*` inside `async fn`
/// rule. Returns every violation found.
///
/// Used in two places:
///
/// 1. [`scan_workspace`] applies this to every `.rs` file under each
///    `adapter-host`-class crate's `src/` directory.
/// 2. The xtask self-test (`xtask/tests/dst_lint_async_fs_self_test.rs`)
///    drives synthetic source through this entry point directly.
///
/// # Errors
///
/// Propagates `syn::parse_file` failures so callers can distinguish
/// parse errors from "file was clean".
pub fn scan_source_async_fs(source: &str, file: impl AsRef<Path>) -> Result<Vec<Violation>> {
    let file = file.as_ref().to_path_buf();
    let parsed = syn::parse_file(source).with_context(|| format!("parse {}", file.display()))?;
    let mut collector = AsyncBlockingIoCollector::new(&file);
    collector.visit_file(&parsed);
    Ok(collector.violations)
}

// -----------------------------------------------------------------------------
// `"live"` literal grep gate (CLI render / command source)
// -----------------------------------------------------------------------------
//
// Slice 01 of `workload-kind-discriminator` (US-06). The literal `"live"`
// was used as a hard-coded duration in CLI render output ("(took live)")
// for in-progress workloads; this gate fires when any file under
// `crates/overdrive-cli/src/render.rs` or
// `crates/overdrive-cli/src/commands/` contains the bare `"live"`
// string-literal token in code (NOT in `//`, `///`, or `/* */`
// comments). Comments and docstrings are fine — they may explain the
// historical bug.

/// Rule label rendered in the violation help text. The rule name doubles
/// as a grep target so reviewers can find every reference.
const LIVE_LITERAL_RULE_LABEL: &str = "live-literal-banned: replace with a measured duration";

/// Rule label for the underscore-binding-probe-runner gate. The rule
/// name doubles as a grep target so reviewers can find every reference.
const UNDERSCORE_BINDING_PROBE_RUNNER_RULE_LABEL: &str = "underscore-binding-probe-runner: thread Arc<ProbeRunner> via \
     ExecDriver::new(...).with_probe_runner(Arc::clone(&runner))";

/// Tokenizer state for [`scan_source_live_literal`]. Module-scoped so
/// the function body stays free of item-after-statement clippy
/// complaints; the enum is implementation-private to the live-literal
/// gate.
enum LiveLiteralScanState {
    Code,
    LineComment,
    BlockComment,
    StringLit,
}

/// Scan a single source file for the `"live"` literal in code, ignoring
/// occurrences inside `//`-line, `///`-doc, and `/* */`-block comments.
///
/// Used in two places:
///
/// 1. [`scan_workspace`] applies this to render / command source under
///    the CLI crate.
/// 2. The xtask self-test (`xtask/tests/dst_lint_live_literal.rs`)
///    drives synthetic source through this entry point directly.
///
/// # Errors
///
/// Currently infallible — kept as `Result` for parity with the other
/// `scan_source_*` entry points in this module.
pub fn scan_source_live_literal(source: &str, file: impl AsRef<Path>) -> Result<Vec<Violation>> {
    use LiveLiteralScanState as State;

    let file = file.as_ref().to_path_buf();
    let mut violations = Vec::new();

    let bytes = source.as_bytes();
    let mut state = State::Code;
    let mut line = 1usize;
    let mut col = 1usize;
    let mut current_string_start: Option<(usize, usize, usize)> = None; // (line, col, byte_idx_of_open_quote)
    let mut i = 0usize;

    while i < bytes.len() {
        let c = bytes[i];
        match state {
            State::Code => {
                if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    state = State::LineComment;
                    i += 2;
                    col += 2;
                    continue;
                } else if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    state = State::BlockComment;
                    i += 2;
                    col += 2;
                    continue;
                } else if c == b'"' {
                    state = State::StringLit;
                    current_string_start = Some((line, col, i));
                }
            }
            State::LineComment => {
                if c == b'\n' {
                    state = State::Code;
                }
            }
            State::BlockComment => {
                if c == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    state = State::Code;
                    i += 2;
                    col += 2;
                    continue;
                }
            }
            State::StringLit => {
                if c == b'\\' && i + 1 < bytes.len() {
                    // Skip escape sequence: advance over the next byte.
                    i += 2;
                    col += 2;
                    continue;
                }
                if c == b'"' {
                    // String closes — inspect the literal between the
                    // opening quote and this position.
                    if let Some((open_line, open_col, open_idx)) = current_string_start.take() {
                        let inner = &source[open_idx + 1..i];
                        if inner == "live" {
                            violations.push(Violation {
                                file: file.clone(),
                                line: open_line,
                                column: open_col,
                                banned_path: "\"live\"".to_string(),
                                replacement_trait: LIVE_LITERAL_RULE_LABEL.to_string(),
                                kind: BannedKind::LiveLiteralBanned,
                            });
                        }
                    }
                    state = State::Code;
                }
            }
        }
        if c == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
        i += 1;
    }

    Ok(violations)
}

/// Decide whether a path under a `crate_class = "binary"` (or otherwise)
/// crate is in scope for the `live-literal-banned` rule. The rule
/// targets only:
///
/// - `crates/overdrive-cli/src/render.rs` (or any file directly named
///   `render.rs` under the CLI crate's `src/`)
/// - any `.rs` file under `crates/overdrive-cli/src/commands/`
///
/// The check is path-string-based (substring match) for simplicity;
/// path normalisation is handled by the caller passing relative paths
/// rooted at the workspace.
fn live_literal_path_in_scope(rel_path: &Path) -> bool {
    let s = rel_path.to_string_lossy().replace('\\', "/");
    if s.contains("crates/overdrive-cli/src/render.rs") || s.contains("overdrive-cli/src/render.rs")
    {
        return true;
    }
    let is_rs = rel_path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("rs"));
    if !is_rs {
        return false;
    }
    if s.contains("crates/overdrive-cli/src/commands/") {
        return true;
    }
    // Also accept the relative form (without the `crates/` prefix) for
    // when scan_workspace strips the crate root before invoking us.
    if s.contains("overdrive-cli/src/commands/") {
        return true;
    }
    false
}

// -----------------------------------------------------------------------------
// Underscore-binding-probe-runner scanner — GAP-5 closure
// -----------------------------------------------------------------------------
//
// Pre-patch the binary composition root at `crates/overdrive-control-plane/
// src/lib.rs::run_server_with_obs_and_driver` built `ExecDriver` BEFORE the
// `ProbeRunner` Earned-Trust gate ran, then discarded the resulting
// `Arc<ProbeRunner>` into `let _probe_runner = probe_runner;`. Net effect:
// every production `ExecDriver::on_alloc_running` / `on_alloc_terminal`
// invocation took the trait-default no-op path — the probe subsystem was
// structurally dead despite shipping green tests against the
// `with_probe_runner`-equipped fixture path.
//
// This scanner is the structural defense: any line under
// `crates/overdrive-control-plane/src/` matching either pattern
//
//   - `let _probe_runner` (bare or with type annotation or `=` continuation)
//   - `let _ = probe_runner`
//
// fails the gate. The legitimate destructure shape used inside
// `compose_production_driver`'s caller (`let (driver, _) = compose_production_driver(...)`)
// does NOT match either pattern: it discards a positional tuple slot, not a
// bound variable, and the helper's `Arc<dyn Driver>` retains the runner Arc
// via `.with_probe_runner(...)`. See GAP-4 + GAP-5 closure commit.
//
// Purely syntactic line scan with comment-aware tokenization (so a
// docstring or `//`-comment that names the literal pattern does not
// trigger). No `overdrive-*` import — keeps the xtask boundary intact
// per `.claude/rules/development.md` § "xtask is build / test / dev
// orchestration, NOT a runtime entry point".

/// Walk `source` line-by-line and flag any `let _probe_runner` or
/// `let _ = probe_runner` pattern outside `//`-line, `///`-doc, and
/// `/* */`-block comments.
///
/// Match shape (after comment stripping and whitespace normalisation):
///
/// - `let _probe_runner` — followed by anything (`;`, `:` for a type
///   annotation, `=` for an initialiser, end-of-line). Anchored on
///   the literal `_probe_runner` identifier; a `let _probe_runner_x`
///   distinct binding does NOT match.
/// - `let _ = probe_runner` — followed by `;`, `,`, `.method(...)`,
///   `?`, or end-of-line. Anchored on the literal `probe_runner`
///   identifier on the right-hand side.
///
/// The match is whole-token: `_probe_runner` matches but
/// `__probe_runner` and `_probe_runner_clone` do NOT (next char after
/// the match must be a non-identifier char).
///
/// # Errors
///
/// Currently infallible — kept as `Result` for parity with the other
/// `scan_source_*` entry points in this module.
pub fn scan_source_underscore_binding_probe_runner(
    source: &str,
    file: impl AsRef<Path>,
) -> Result<Vec<Violation>> {
    let file = file.as_ref().to_path_buf();
    let mut violations = Vec::new();

    // Comment-strip pass: produce a per-line copy of `source` with
    // `//` line comments and `/* */` block comments replaced by spaces
    // (preserving line numbers and column offsets). String literals
    // are preserved as-is — neither pattern can legitimately appear
    // inside a string in production code, and false positives there
    // are acceptable.
    let stripped = strip_comments_preserving_layout(source);

    for (idx, line) in stripped.lines().enumerate() {
        let line_no = idx + 1;
        // Pattern 1: `let _probe_runner` as a whole-identifier match.
        if let Some(col) = find_whole_token(line, "let _probe_runner") {
            violations.push(Violation {
                file: file.clone(),
                line: line_no,
                column: col + 1,
                banned_path: "let _probe_runner".to_string(),
                replacement_trait: UNDERSCORE_BINDING_PROBE_RUNNER_RULE_LABEL.to_string(),
                kind: BannedKind::UnderscoreBindingProbeRunner,
            });
            continue;
        }
        // Pattern 2: `let _ = probe_runner` (whitespace flexible).
        if let Some(col) = find_let_underscore_equals_probe_runner(line) {
            violations.push(Violation {
                file: file.clone(),
                line: line_no,
                column: col + 1,
                banned_path: "let _ = probe_runner".to_string(),
                replacement_trait: UNDERSCORE_BINDING_PROBE_RUNNER_RULE_LABEL.to_string(),
                kind: BannedKind::UnderscoreBindingProbeRunner,
            });
        }
    }

    Ok(violations)
}

/// Tokenizer state for [`strip_comments_preserving_layout`]. Module-
/// scoped to satisfy `clippy::items_after_statements` — the enum cannot
/// live inside the function body.
enum StripCommentsState {
    Code,
    LineComment,
    BlockComment,
    StringLit,
}

/// Replace `//`-line and `/* */`-block comments in `source` with spaces,
/// preserving every newline so line and column numbers stay aligned with
/// the original source.
fn strip_comments_preserving_layout(source: &str) -> String {
    use StripCommentsState as State;
    let bytes = source.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    let mut state = State::Code;

    while i < bytes.len() {
        let c = bytes[i];
        match state {
            State::Code => {
                if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    state = State::LineComment;
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                    continue;
                } else if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    state = State::BlockComment;
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                    continue;
                } else if c == b'"' {
                    state = State::StringLit;
                    out.push(c);
                } else {
                    out.push(c);
                }
            }
            State::LineComment => {
                if c == b'\n' {
                    state = State::Code;
                    out.push(b'\n');
                } else {
                    out.push(b' ');
                }
            }
            State::BlockComment => {
                if c == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    state = State::Code;
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                    continue;
                }
                out.push(if c == b'\n' { b'\n' } else { b' ' });
            }
            State::StringLit => {
                if c == b'\\' && i + 1 < bytes.len() {
                    out.push(c);
                    out.push(bytes[i + 1]);
                    i += 2;
                    continue;
                }
                if c == b'"' {
                    state = State::Code;
                }
                out.push(c);
            }
        }
        i += 1;
    }

    String::from_utf8(out).unwrap_or_else(|_| source.to_string())
}

/// Find the first byte offset of `needle` in `line` such that the byte
/// immediately following the match is NOT an identifier-continuation
/// character (letter, digit, or `_`). Returns `None` if no whole-token
/// match exists.
///
/// This guards against `_probe_runner_x` / `let _probe_runner_2` style
/// distinct identifiers being flagged when the rule targets exactly the
/// `_probe_runner` binding.
fn find_whole_token(line: &str, needle: &str) -> Option<usize> {
    let mut start = 0usize;
    while let Some(rel) = line[start..].find(needle) {
        let abs = start + rel;
        let end = abs + needle.len();
        let next_is_ident =
            line.as_bytes().get(end).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_');
        if !next_is_ident {
            return Some(abs);
        }
        start = abs + needle.len();
    }
    None
}

/// Find the first column of `let _ = probe_runner` on `line`, with
/// flexible whitespace around `_`, `=`, and `probe_runner`. The match
/// is whole-token on `probe_runner` (the byte after must not be an
/// identifier-continuation character).
fn find_let_underscore_equals_probe_runner(line: &str) -> Option<usize> {
    // Cheap pre-filter — if the line doesn't even contain `let` and
    // `probe_runner`, skip the regex-ish scan.
    if !line.contains("let") || !line.contains("probe_runner") {
        return None;
    }
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i + 3 <= bytes.len() {
        if &bytes[i..i + 3] == b"let" {
            // Anchor: `let` must be preceded by start-of-line or
            // non-identifier char (so `width_let` doesn't trigger).
            let prev_ok = if i == 0 {
                true
            } else {
                let p = bytes[i - 1];
                !(p.is_ascii_alphanumeric() || p == b'_')
            };
            // Followed by whitespace (otherwise `letchar` etc. could match).
            let after = bytes.get(i + 3).copied();
            let after_ok = after.is_some_and(|b| b == b' ' || b == b'\t');
            if prev_ok && after_ok {
                // Scan past `let` + WS, expect `_`, then `=`, then
                // `probe_runner`.
                let mut j = i + 3;
                while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'_' {
                    // The `_` must be a whole token: next char NOT
                    // an identifier continuation.
                    let after_us = bytes.get(j + 1).copied();
                    let lone_underscore =
                        after_us.is_none_or(|b| !(b.is_ascii_alphanumeric() || b == b'_'));
                    if lone_underscore {
                        j += 1;
                        while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                            j += 1;
                        }
                        if j < bytes.len() && bytes[j] == b'=' {
                            j += 1;
                            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                                j += 1;
                            }
                            // Now expect literal `probe_runner` as
                            // whole token.
                            let rest = &line[j..];
                            if rest.starts_with("probe_runner") {
                                let end = j + "probe_runner".len();
                                let after_id = bytes.get(end).copied();
                                let whole = after_id
                                    .is_none_or(|b| !(b.is_ascii_alphanumeric() || b == b'_'));
                                if whole {
                                    return Some(i);
                                }
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Decide whether `rel_path` is under
/// `crates/overdrive-control-plane/src/` — the scope of the
/// underscore-binding-probe-runner gate.
fn underscore_binding_probe_runner_path_in_scope(rel_path: &Path) -> bool {
    let s = rel_path.to_string_lossy().replace('\\', "/");
    let is_rs = rel_path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("rs"));
    if !is_rs {
        return false;
    }
    s.contains("crates/overdrive-control-plane/src/") || s.contains("overdrive-control-plane/src/")
}

// -----------------------------------------------------------------------------
// Envelope scaffolding scanners — RED scaffolds per ADR-0048
// -----------------------------------------------------------------------------

// -----------------------------------------------------------------------------
// Envelope variant-construction scanner — Layer 2 of ADR-0048 § 2
// -----------------------------------------------------------------------------
//
// Walks `overdrive-core` source for `<Envelope>::V<N>(...)` literal
// expression patterns outside the defining module's own
// `impl VersionedEnvelope for <Envelope>` block (where `fn latest`
// and `fn into_latest` legitimately produce variants) and any
// `impl From<...> for <Envelope-related>` block. Purely syntactic
// (no `overdrive-*` import) per the xtask boundary in
// `.claude/rules/development.md` § "xtask is build / test / dev
// orchestration, NOT a runtime entry point".

/// Match heuristics for `<Envelope>::V<N>(...)`-shaped call expressions.
///
/// A call expression's callee path must satisfy:
///
/// - Exactly two path segments (e.g. `AllocStatusRowEnvelope::V1`).
/// - The penultimate segment's identifier ends with `Envelope`.
/// - The last segment matches `V<N>` for `N >= 1` (e.g. `V1`, `V2`, ...).
///
/// `Self::V1(...)` inside an `impl <Envelope>` block does NOT match
/// here (the penultimate segment is `Self`, not `<Envelope>`). This
/// keeps the `fn latest` / `fn into_latest` canonical impl shapes
/// from triggering the lint by accident — they construct via
/// `Self::V1(...)`, which is the textually-distinct pattern Layer 2
/// is happy to allow inside the defining module's own `impl` block.
fn is_envelope_variant_call(call: &syn::ExprCall) -> Option<String> {
    let syn::Expr::Path(path_expr) = &*call.func else { return None };
    let segments: Vec<&syn::PathSegment> = path_expr.path.segments.iter().collect();
    if segments.len() < 2 {
        return None;
    }
    let envelope_ident = segments[segments.len() - 2].ident.to_string();
    let variant_ident = segments[segments.len() - 1].ident.to_string();
    if !envelope_ident.ends_with("Envelope") {
        return None;
    }
    if !is_variant_v_n(&variant_ident) {
        return None;
    }
    Some(format!("{envelope_ident}::{variant_ident}"))
}

/// Is `ident` of the shape `V<digits>` with at least one digit?
fn is_variant_v_n(ident: &str) -> bool {
    let Some(rest) = ident.strip_prefix('V') else { return false };
    !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())
}

/// Visitor tracking the enclosing-fn / enclosing-impl context.
///
/// A variant-construction call is suppressed (allowed) when *either*:
///
/// 1. The enclosing `impl` is an `impl From<...> for <X>` block — any
///    inter-version `From` conversion may legitimately wrap into a
///    variant of the destination envelope's payload type.
/// 2. The enclosing fn is named `latest` or `into_latest` — the trait
///    impl methods that construct variants by design. (Note: those
///    methods use `Self::V<N>(...)` per the in-repo convention and
///    therefore already fall outside the
///    [`is_envelope_variant_call`] heuristic. The fn-name allow-list
///    is here as a robustness backstop in case a future impl uses
///    the explicit `<Envelope>::V<N>(...)` form inside its own
///    `latest` / `into_latest` body.)
struct EnvelopeVariantCollector<'a> {
    file: &'a Path,
    violations: Vec<Violation>,
    /// Depth of currently-open `impl From<X> for Y` blocks.
    in_from_impl_depth: usize,
    /// Depth of currently-open `latest` / `into_latest` fn bodies.
    in_allowed_fn_depth: usize,
}

impl<'a> EnvelopeVariantCollector<'a> {
    const fn new(file: &'a Path) -> Self {
        Self { file, violations: Vec::new(), in_from_impl_depth: 0, in_allowed_fn_depth: 0 }
    }

    const fn is_allowed_context(&self) -> bool {
        self.in_from_impl_depth > 0 || self.in_allowed_fn_depth > 0
    }
}

/// Does `item_impl` represent `impl From<...> for <Y>`?
fn is_from_impl(item_impl: &syn::ItemImpl) -> bool {
    let Some((_, trait_path, _)) = &item_impl.trait_ else { return false };
    trait_path.segments.last().is_some_and(|seg| seg.ident == "From")
}

/// Is `ident` the name of a context that may construct envelope
/// variants — i.e. `latest` or `into_latest`?
fn is_allowed_fn_name(ident: &syn::Ident) -> bool {
    ident == "latest" || ident == "into_latest"
}

impl<'ast> Visit<'ast> for EnvelopeVariantCollector<'_> {
    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        let pushed = is_from_impl(node);
        if pushed {
            self.in_from_impl_depth += 1;
        }
        visit::visit_item_impl(self, node);
        if pushed {
            self.in_from_impl_depth -= 1;
        }
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        let pushed = is_allowed_fn_name(&node.sig.ident);
        if pushed {
            self.in_allowed_fn_depth += 1;
        }
        visit::visit_item_fn(self, node);
        if pushed {
            self.in_allowed_fn_depth -= 1;
        }
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        let pushed = is_allowed_fn_name(&node.sig.ident);
        if pushed {
            self.in_allowed_fn_depth += 1;
        }
        visit::visit_impl_item_fn(self, node);
        if pushed {
            self.in_allowed_fn_depth -= 1;
        }
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let Some(banned_path) = is_envelope_variant_call(node) {
            if !self.is_allowed_context() {
                // Locate the head segment span for accurate reporting.
                if let syn::Expr::Path(path_expr) = &*node.func {
                    if let Some(first) = path_expr.path.segments.first() {
                        let start = first.ident.span().start();
                        self.violations.push(Violation {
                            file: self.file.to_path_buf(),
                            line: start.line,
                            column: start.column + 1,
                            banned_path,
                            replacement_trait:
                                "<Envelope>::latest(payload) (codec-internal wrapping site)"
                                    .to_owned(),
                            kind: BannedKind::Api,
                        });
                    }
                }
            }
        }
        visit::visit_expr_call(self, node);
    }
}

/// Scan `source` for `<Envelope>::V<N>(...)` literal call expressions
/// outside the defining module's own `impl From<...>` blocks and
/// `fn latest` / `fn into_latest` bodies.
///
/// Purely syntactic — no `overdrive-*` import per the xtask boundary
/// in `.claude/rules/development.md` § "xtask is build / test / dev
/// orchestration, NOT a runtime entry point".
///
/// Returns one [`Violation`] per offending site naming the file path,
/// line number, column, and offending construction pattern.
///
/// # Errors
///
/// Returns the empty vector on parse failure — the source is expected
/// to be `overdrive-core` `.rs` text that already type-checks via
/// `cargo check` before the lint runs. A parse failure means a
/// transient bad source (mid-edit, malformed merge); the lint is
/// best-effort and ignores it. Mirrors the convention used by the
/// other `scan_source_*` entry points in this module.
pub fn scan_for_envelope_variant_construction(source: &str, path: &Path) -> Vec<Violation> {
    let Ok(parsed) = syn::parse_file(source) else { return Vec::new() };
    let mut collector = EnvelopeVariantCollector::new(path);
    collector.visit_file(&parsed);
    collector.violations
}

/// Layer-2 coverage gate per ADR-0048 § 6.
///
/// Walks `<crate_root>/src/` for `enum *Envelope` definitions with
/// `V<N>(<Payload>)` variants; for each envelope, verifies that a
/// file exists at `<crate_root>/tests/schema_evolution/<envelope_snake>.rs`
/// and contains a `FIXTURE_V<N>: &str` constant for every historical
/// variant. Returns one [`Violation`] per missing fixture file or
/// constant. Purely syntactic (no `overdrive-*` import).
///
/// Closes the loop on the schema-evolution defense: without this
/// clause the structural defense degrades to "did the author
/// remember to add a fixture?" — a non-mechanical check that fails
/// open under reviewer fatigue (S-EV-06 closing-the-loop guarantee).
pub fn scan_for_envelope_fixture_coverage(crate_root: &Path) -> Vec<Violation> {
    let mut violations = Vec::new();
    let src = crate_root.join("src");
    if !src.exists() {
        return violations;
    }
    let Ok(rs_files) = collect_rs_files(&src) else { return violations };

    for rs in rs_files {
        let Ok(source) = std::fs::read_to_string(&rs) else { continue };
        let Ok(parsed) = syn::parse_file(&source) else { continue };

        for envelope in collect_envelope_definitions(&parsed) {
            let snake = envelope_name_to_snake_case(&envelope.name);
            let fixture_rel: PathBuf =
                ["tests", "schema_evolution", &format!("{snake}.rs")].iter().collect();
            let fixture_abs = crate_root.join(&fixture_rel);

            if !fixture_abs.is_file() {
                // Missing file — emit one violation per envelope at the
                // enum-definition site.
                violations.push(Violation {
                    file: rs.strip_prefix(crate_root).unwrap_or(&rs).to_path_buf(),
                    line: envelope.line,
                    column: envelope.column,
                    banned_path: format!(
                        "{} (missing fixture file: {})",
                        envelope.name,
                        fixture_rel.display(),
                    ),
                    replacement_trait: format!(
                        "create {} with `const FIXTURE_V<N>: &str = \"...\"` for each variant",
                        fixture_rel.display(),
                    ),
                    kind: BannedKind::Api,
                });
                continue;
            }

            // File exists — parse it and collect the `FIXTURE_V<N>`
            // constants present, then diff against the envelope's
            // variant list.
            let Ok(fixture_source) = std::fs::read_to_string(&fixture_abs) else {
                continue;
            };
            let present_fixtures = collect_fixture_constants(&fixture_source);
            for variant_n in &envelope.variant_ns {
                let needle = format!("FIXTURE_V{variant_n}");
                if !present_fixtures.contains(&needle) {
                    violations.push(Violation {
                        file: fixture_rel.clone(),
                        line: envelope.line,
                        column: envelope.column,
                        banned_path: format!(
                            "{}::V{} (missing {})",
                            envelope.name, variant_n, needle
                        ),
                        replacement_trait: format!(
                            "add `const {needle}: &str = \"...\"` pinning the archived bytes \
                             of the V{variant_n} payload",
                        ),
                        kind: BannedKind::Api,
                    });
                }
            }
        }
    }

    violations
}

/// Information extracted from one `enum *Envelope { V<N>(...) }` item.
struct EnvelopeDefinition {
    /// Envelope type name, e.g. `AllocStatusRowEnvelope`.
    name: String,
    /// Variant numbers in declaration order, e.g. `[1, 2, 3]` for
    /// `enum Foo { V1(_), V2(_), V3(_) }`.
    variant_ns: Vec<u32>,
    /// 1-based source line of the enum declaration head.
    line: usize,
    /// 1-based column of the enum declaration head.
    column: usize,
}

/// Walk every item of `parsed` and return one [`EnvelopeDefinition`]
/// per `enum *Envelope` whose variants are at least partially of the
/// `V<N>(<Payload>)` shape. An envelope with zero `V<N>` variants is
/// not an envelope per the convention; skip it (no coverage to
/// enforce).
fn collect_envelope_definitions(parsed: &syn::File) -> Vec<EnvelopeDefinition> {
    let mut out = Vec::new();
    for item in &parsed.items {
        let syn::Item::Enum(item_enum) = item else { continue };
        let name = item_enum.ident.to_string();
        if !name.ends_with("Envelope") {
            continue;
        }
        let mut variant_ns = Vec::new();
        for variant in &item_enum.variants {
            let v_ident = variant.ident.to_string();
            let Some(rest) = v_ident.strip_prefix('V') else { continue };
            if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            // Only count tuple-style `V<N>(<Payload>)` variants; reject
            // unit (`V1`) and struct (`V1 { … }`) variants because the
            // payload-carrying shape is the canonical envelope form per
            // ADR-0048.
            if !matches!(variant.fields, syn::Fields::Unnamed(_)) {
                continue;
            }
            let Ok(n) = rest.parse::<u32>() else { continue };
            variant_ns.push(n);
        }
        if variant_ns.is_empty() {
            continue;
        }
        let start = item_enum.ident.span().start();
        out.push(EnvelopeDefinition {
            name,
            variant_ns,
            line: start.line,
            column: start.column + 1,
        });
    }
    out
}

/// Walk `parsed` and return the names of every top-level `const`
/// item whose ident starts with `FIXTURE_V`. Comparison is on the
/// raw ident string; we do not parse the `<N>` numeric suffix here
/// because the caller looks up by full name (`FIXTURE_V<N>`).
fn collect_fixture_constants(source: &str) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    let Ok(parsed) = syn::parse_file(source) else { return out };
    for item in &parsed.items {
        if let syn::Item::Const(item_const) = item {
            let name = item_const.ident.to_string();
            if name.starts_with("FIXTURE_V") {
                out.insert(name);
            }
        }
    }
    out
}

/// Convert a `CamelCase` envelope ident to the expected
/// `snake_case` fixture-file stem, after stripping the trailing
/// `Envelope` suffix.
///
/// Examples (the load-bearing five today):
/// - `AllocStatusRowEnvelope`            → `alloc_status_row`
/// - `NodeHealthRowEnvelope`             → `node_health_row`
/// - `ServiceHydrationResultRowEnvelope` → `service_hydration_result_row`
/// - `ServiceBackendRowEnvelope`         → `service_backend_row`
/// - `JobEnvelope`                       → `job`
///
/// Algorithm: strip the `Envelope` suffix if present, then lowercase
/// each char while inserting a `_` before every uppercase letter that
/// is preceded by a lowercase letter or digit (the canonical
/// `CamelCase` → `snake_case` boundary). Pure ASCII; no external crate.
fn envelope_name_to_snake_case(name: &str) -> String {
    let stem = name.strip_suffix("Envelope").unwrap_or(name);
    let mut out = String::with_capacity(stem.len() + 4);
    let mut prev_was_lower_or_digit = false;
    for ch in stem.chars() {
        if ch.is_ascii_uppercase() {
            if prev_was_lower_or_digit {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            prev_was_lower_or_digit = false;
        } else {
            out.push(ch);
            prev_was_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        }
    }
    out
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
    for (_name, root, _class) in &core_crates {
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
            // Layer 2 of ADR-0048 § 2 — flag `<Envelope>::V<N>(...)`
            // literal constructions outside `impl From<...>` blocks
            // and `fn latest` / `fn into_latest` bodies. Purely
            // syntactic; runs over the same source tree as the
            // banned-API scanner above.
            violations.extend(scan_for_envelope_variant_construction(&source, &rel));
        }

        // Layer-2 fixture-coverage gate per ADR-0048 § 6 — for every
        // `enum *Envelope` defined under `src/`, ensure the matching
        // `tests/schema_evolution/<envelope_snake>.rs` exists and
        // pins `FIXTURE_V<N>` for every variant. The closing-the-loop
        // gate that prevents a future PR from adding a new envelope
        // and forgetting the fixture (S-EV-06).
        violations.extend(scan_for_envelope_fixture_coverage(root));
    }

    // Scan adapter-host crates with the std::fs-in-async-fn rule. The
    // banned-API table is for `core` only; this clause applies to
    // `adapter-host` exclusively because that is where async fn impls
    // of the port traits live and where the tokio executor would be
    // blocked by sync std::fs calls. See
    // `.claude/rules/development.md` § Concurrency & async.
    let adapter_host_crates: Vec<_> =
        classes.iter().filter(|(_, _, c)| c.as_deref() == Some("adapter-host")).collect();
    for (_name, root, _class) in adapter_host_crates {
        let src = root.join("src");
        if !src.exists() {
            continue;
        }
        for rs in collect_rs_files(&src)? {
            let rel = rs.strip_prefix(root).unwrap_or(&rs).to_path_buf();
            let source =
                std::fs::read_to_string(&rs).with_context(|| format!("read {}", rs.display()))?;
            if let Ok(found) = scan_source_async_fs(&source, &rel) {
                violations.extend(found);
            }
        }
    }

    // Scan the CLI crate's render / command source for the `"live"`
    // literal regression-guard rule (Slice 01 of
    // `workload-kind-discriminator`, US-06). Path-based scoping picks
    // out only `crates/overdrive-cli/src/render.rs` and any file
    // under `crates/overdrive-cli/src/commands/**/*.rs`.
    for (_name, root, _class) in &classes {
        let src = root.join("src");
        if !src.exists() {
            continue;
        }
        for rs in collect_rs_files(&src)? {
            let rel_to_workspace = rs
                .strip_prefix(
                    Path::new(&metadata.workspace_root.as_str())
                        .canonicalize()
                        .unwrap_or_else(|_| metadata.workspace_root.clone().into_std_path_buf()),
                )
                .unwrap_or(&rs)
                .to_path_buf();
            if !live_literal_path_in_scope(&rel_to_workspace) {
                continue;
            }
            let source =
                std::fs::read_to_string(&rs).with_context(|| format!("read {}", rs.display()))?;
            if let Ok(found) = scan_source_live_literal(&source, &rel_to_workspace) {
                violations.extend(found);
            }
        }
    }

    // Scan `crates/overdrive-control-plane/src/` for the
    // underscore-binding-probe-runner regression guard (GAP-5 closure
    // from `.context/01-03-structural-gap-audit.md`).
    violations.extend(scan_underscore_binding_probe_runner(&classes, &metadata)?);

    Ok(violations)
}

/// Dispatch the underscore-binding-probe-runner gate across the
/// workspace. Path-scoped so the gate only fires on the binary
/// composition-root crate (`crates/overdrive-control-plane/src/`),
/// where the pre-patch shape lived; any non-control-plane source file
/// with a similarly-named `probe_runner` binding is out of scope.
fn scan_underscore_binding_probe_runner(
    classes: &[(String, PathBuf, Option<String>)],
    metadata: &cargo_metadata::Metadata,
) -> Result<Vec<Violation>> {
    let mut violations = Vec::new();
    for (_name, root, _class) in classes {
        let src = root.join("src");
        if !src.exists() {
            continue;
        }
        for rs in collect_rs_files(&src)? {
            let rel_to_workspace = rs
                .strip_prefix(
                    Path::new(&metadata.workspace_root.as_str())
                        .canonicalize()
                        .unwrap_or_else(|_| metadata.workspace_root.clone().into_std_path_buf()),
                )
                .unwrap_or(&rs)
                .to_path_buf();
            if !underscore_binding_probe_runner_path_in_scope(&rel_to_workspace) {
                continue;
            }
            let source =
                std::fs::read_to_string(&rs).with_context(|| format!("read {}", rs.display()))?;
            if let Ok(found) =
                scan_source_underscore_binding_probe_runner(&source, &rel_to_workspace)
            {
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
        BannedKind::BlockingIoInAsync => (
            "error: blocking std::fs::* inside async fn (adapter-host crate)",
            format!(
                "use {} — sync std::fs::* blocks the tokio executor and is forbidden \
                 inside async fn in adapter-host crates",
                v.replacement_trait,
            ),
            "see .claude/rules/development.md (\"Concurrency & async\")",
        ),
        BannedKind::LiveLiteralBanned => (
            "error: `\"live\"` literal in CLI render or command source",
            format!(
                "{} — replace the literal with a measured duration (e.g. \
                 the actual elapsed time of the workload)",
                v.replacement_trait,
            ),
            "see docs/feature/workload-kind-discriminator/slices/slice-01-parser-kind-discriminator.md",
        ),
        BannedKind::UnderscoreBindingProbeRunner => (
            "error: underscore-bound or discarded `probe_runner` in control-plane source",
            format!(
                "{} — thread the runner via \
                 `ExecDriver::new(...).with_probe_runner(Arc::clone(&runner))` \
                 (use the `compose_production_driver(...)` helper at the \
                 binary boundary). The discard shape silently disables every \
                 production probe-supervisor lifecycle hook.",
                v.replacement_trait,
            ),
            "see GAP-5 from .context/01-03-structural-gap-audit.md",
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
// Structural inspector — WorkloadLifecycle::reconcile body
// -----------------------------------------------------------------------------
//
// Scenario 3.3 of phase-1-first-workload Slice 3A.2: the `WorkloadLifecycle`
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
                    kind: ReconcileBodyViolationKind::BannedApi { path: (*banned).to_string() },
                    line,
                    column,
                });
                return;
            }
        }
    }
}

/// Inspect a Rust source file for any `WorkloadLifecycle::reconcile` body
/// that contains a banned construct.
///
/// Returns a list of every forbidden construct found inside the
/// `fn reconcile` body of the `impl Reconciler for WorkloadLifecycle`
/// block. An empty Vec means the body is clean.
///
/// # Errors
///
/// Propagates any `syn::parse_file` failure as `Err`.
pub fn inspect_workload_lifecycle_reconcile_body(
    source: &str,
) -> Result<Vec<ReconcileBodyViolation>> {
    inspect_reconciler_body(source, "WorkloadLifecycle")
}

/// Inspect a Rust source file for any `ServiceMapHydrator::reconcile`
/// body that contains a banned construct.
///
/// Returns a list of every forbidden construct found inside the
/// `fn reconcile` body of the `impl Reconciler for ServiceMapHydrator`
/// block. An empty Vec means the body is clean.
///
/// The `ServiceMapHydrator` reconciler (Slice 08 / S-2.2-30) carries
/// the same ADR-0035 §2 purity contract as `WorkloadLifecycle` — sync, no
/// `.await`, no wall-clock reads, no DB handle. This function is the
/// mechanical enforcement gate for that invariant.
///
/// # Errors
///
/// Propagates any `syn::parse_file` failure as `Err`.
pub fn inspect_service_map_hydrator_reconcile_body(
    source: &str,
) -> Result<Vec<ReconcileBodyViolation>> {
    inspect_reconciler_body(source, "ServiceMapHydrator")
}

/// Shared inspector: locates `impl Reconciler for {self_name}` in
/// `source`, finds the `fn reconcile` body, and walks it for forbidden
/// constructs.
///
/// Factored out of the per-reconciler public entry points to avoid
/// duplication. Both `WorkloadLifecycle` and `ServiceMapHydrator` share the
/// same purity rules; the only variable is the concrete type name.
fn inspect_reconciler_body(source: &str, self_name: &str) -> Result<Vec<ReconcileBodyViolation>> {
    let parsed = syn::parse_file(source).context("parse source")?;
    let mut found_impl = false;
    let mut violations = Vec::new();

    for item in &parsed.items {
        let syn::Item::Impl(item_impl) = item else { continue };

        // Match `impl Reconciler for {self_name}`.
        let Some((_, trait_path, _)) = &item_impl.trait_ else { continue };
        let trait_name =
            trait_path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
        if trait_name != "Reconciler" {
            continue;
        }

        let syn::Type::Path(type_path) = &*item_impl.self_ty else { continue };
        let impl_self_name =
            type_path.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
        if impl_self_name != self_name {
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
        bail!("could not locate `impl Reconciler for {self_name}` in source");
    }

    Ok(violations)
}

/// Inspect a Rust source file for the Earned-Trust gate on
/// `ProbeRunner`.
///
/// Confirms that `impl ProbeRunner { ... }` declares an
/// `async fn probe(&self) -> ...` method (the sacrificial-loopback
/// probe per ADR-0054 §7).
///
/// Returns `Ok(())` when the method is present; returns
/// `Err(eyre::eyre!(...))` when the impl block exists but the
/// method is missing, OR when no `impl ProbeRunner` block is found
/// at all. The composition-root invocation lands in step 01-03d;
/// THIS scanner clause is the structural defense against the
/// method being removed.
///
/// Purely syntactic — no `overdrive-*` import. Per
/// `.claude/rules/development.md` § "xtask is build / test / dev
/// orchestration" the scanner walks `syn::parse_file` output and
/// never compiles against the worker crate.
///
/// # Errors
///
/// Propagates any `syn::parse_file` failure as `Err`. Returns
/// `Err` when the `impl ProbeRunner` block is missing OR present
/// but missing the `probe` method.
pub fn inspect_probe_runner_earned_trust_method(source: &str) -> Result<()> {
    let parsed = syn::parse_file(source).context("parse source")?;
    let mut found_impl = false;
    let mut found_method = false;

    for item in &parsed.items {
        let syn::Item::Impl(item_impl) = item else { continue };
        // Inherent impl only (no trait). `impl ProbeRunner { ... }`.
        if item_impl.trait_.is_some() {
            continue;
        }
        let syn::Type::Path(type_path) = &*item_impl.self_ty else { continue };
        let impl_self_name =
            type_path.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
        if impl_self_name != "ProbeRunner" {
            continue;
        }
        found_impl = true;
        for impl_item in &item_impl.items {
            let syn::ImplItem::Fn(fn_item) = impl_item else { continue };
            if fn_item.sig.ident == "probe" && fn_item.sig.asyncness.is_some() {
                // Confirm the signature shape — `&self` receiver,
                // zero non-receiver params, returns `Result<...>`.
                let has_self_receiver = fn_item
                    .sig
                    .inputs
                    .first()
                    .is_some_and(|arg| matches!(arg, syn::FnArg::Receiver(_)));
                let non_receiver_args = fn_item.sig.inputs.len().saturating_sub(1);
                if has_self_receiver && non_receiver_args == 0 {
                    found_method = true;
                    break;
                }
            }
        }
        if found_method {
            break;
        }
    }

    if !found_impl {
        bail!("could not locate `impl ProbeRunner {{ ... }}` in source");
    }
    if !found_method {
        bail!(
            "Earned-Trust gate per ADR-0054 §7 is missing: `impl ProbeRunner` \
             must declare `async fn probe(&self) -> Result<(), ProbeRunnerError>` \
             (no non-receiver parameters)"
        );
    }
    Ok(())
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
    // #[cfg(test)] exemption — banned-API scanner (UI-03 remediation)
    // -------------------------------------------------------------

    #[test]
    fn cfg_test_module_exempts_instant_now_violations() {
        // The exact shape UI-03 documents: a `tick(counter)` test helper
        // inside `#[cfg(test)] mod tests {}` calls `Instant::now()`.
        // Must produce ZERO violations.
        let source = r"
            pub fn production() {}

            #[cfg(test)]
            mod tests {
                use std::time::{Duration, Instant};

                fn tick(counter: u64) -> u64 {
                    let _now = Instant::now();
                    let _deadline = Instant::now() + Duration::from_secs(1);
                    counter
                }
            }
        ";
        let violations =
            scan_source(source, std::path::Path::new("synthetic.rs")).expect("source must parse");
        assert!(
            violations.is_empty(),
            "Instant::now inside #[cfg(test)] mod must be exempt; got {violations:?}"
        );
    }

    #[test]
    fn instant_now_outside_cfg_test_is_still_flagged() {
        // Inverse: same `Instant::now()` call WITHOUT the `#[cfg(test)]`
        // attribute — the scanner must still flag it.
        let source = r"
            use std::time::Instant;
            pub fn tick() -> Instant {
                Instant::now()
            }
        ";
        let violations =
            scan_source(source, std::path::Path::new("synthetic.rs")).expect("source must parse");
        assert!(
            violations.iter().any(|v| v.banned_path == "std::time::Instant::now"),
            "Instant::now outside #[cfg(test)] must be flagged; got {violations:?}"
        );
    }

    #[test]
    fn cfg_test_fn_attr_exempts_instant_now() {
        // `#[cfg(test)]` directly on a free fn (not on the enclosing
        // module) also exempts the body.
        let source = r"
            use std::time::Instant;

            #[cfg(test)]
            fn test_tick() -> Instant {
                Instant::now()
            }
        ";
        let violations =
            scan_source(source, std::path::Path::new("synthetic.rs")).expect("source must parse");
        assert!(
            violations.is_empty(),
            "Instant::now inside #[cfg(test)] fn must be exempt; got {violations:?}"
        );
    }

    #[test]
    fn cfg_test_does_not_exempt_hashmap_violations() {
        // HashMap is BannedKind::OrderedCollection, NOT exempted from
        // the cfg(test) carve-out — iteration-order invariants apply
        // to test code that asserts on DST trajectories.
        let source = r"
            #[cfg(test)]
            mod tests {
                use std::collections::HashMap;
                fn build() -> HashMap<u64, u64> {
                    HashMap::new()
                }
            }
        ";
        let violations =
            scan_source(source, std::path::Path::new("synthetic.rs")).expect("source must parse");
        assert!(
            violations.iter().any(|v| matches!(v.kind, BannedKind::OrderedCollection)),
            "HashMap inside #[cfg(test)] must STILL be flagged; got {violations:?}"
        );
    }

    // -------------------------------------------------------------
    // Reconcile-body inspector tests (scenario 3.3)
    // -------------------------------------------------------------

    /// Path to a reconciler source file inside `overdrive-core` —
    /// relative to the workspace root.
    fn reconciler_source_path(filename: &str) -> std::path::PathBuf {
        let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        crate_dir
            .parent()
            .expect("xtask crate lives directly under workspace root")
            .join(format!("crates/overdrive-core/src/reconcilers/{filename}"))
    }

    #[test]
    fn inspector_flags_await_inside_reconcile_body() {
        let source = r"
            pub trait Reconciler {}
            pub struct WorkloadLifecycle;
            impl Reconciler for WorkloadLifecycle {
                fn reconcile(&self) {
                    let _x = some_future().await;
                }
            }
        ";
        let violations =
            inspect_workload_lifecycle_reconcile_body(source).expect("source must parse");
        assert!(
            violations.iter().any(|v| matches!(v.kind, ReconcileBodyViolationKind::Await)),
            ".await must be flagged; got {violations:?}"
        );
    }

    #[test]
    fn inspector_flags_instant_now_inside_reconcile_body() {
        let source = r"
            pub trait Reconciler {}
            pub struct WorkloadLifecycle;
            impl Reconciler for WorkloadLifecycle {
                fn reconcile(&self) {
                    let _ = std::time::Instant::now();
                }
            }
        ";
        let violations =
            inspect_workload_lifecycle_reconcile_body(source).expect("source must parse");
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
        let source = r"
            pub trait Reconciler {}
            pub struct WorkloadLifecycle;
            impl Reconciler for WorkloadLifecycle {
                fn reconcile(&self) {
                    let _x = rand::random::<u64>();
                }
            }
        ";
        let violations =
            inspect_workload_lifecycle_reconcile_body(source).expect("source must parse");
        assert!(
            violations.iter().any(|v| matches!(v.kind, ReconcileBodyViolationKind::Rand)),
            "rand::* must be flagged; got {violations:?}"
        );
    }

    #[test]
    fn inspector_passes_clean_body() {
        let source = r"
            pub trait Reconciler {}
            pub struct WorkloadLifecycle;
            impl Reconciler for WorkloadLifecycle {
                fn reconcile(&self, tick: &TickContext) -> Vec<u8> {
                    let _now = tick.now;
                    vec![]
                }
            }
        ";
        let violations =
            inspect_workload_lifecycle_reconcile_body(source).expect("source must parse");
        assert!(
            violations.is_empty(),
            "clean body must produce zero violations; got {violations:?}"
        );
    }

    /// Scenario 3.3 — the real `WorkloadLifecycle::reconcile` body inside
    /// `crates/overdrive-core/src/reconcilers/workload_lifecycle.rs`
    /// must contain no banned construct.
    #[test]
    fn workload_lifecycle_reconcile_body_passes_dst_lint() {
        let path = reconciler_source_path("workload_lifecycle.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let violations = inspect_workload_lifecycle_reconcile_body(&source)
            .expect("workload_lifecycle.rs must parse");
        assert!(
            violations.is_empty(),
            "WorkloadLifecycle::reconcile body must contain no banned construct \
             (.await, Instant::now, SystemTime::now, rand::*, tokio::time::sleep, \
             std::thread::sleep); found {} violation(s): {:#?}",
            violations.len(),
            violations
        );
    }

    /// S-2.2-30 — the real `ServiceMapHydrator::reconcile` body inside
    /// `crates/overdrive-core/src/reconcilers/service_map_hydrator.rs`
    /// must contain no banned construct per ADR-0035 §2 / ADR-0013 §2.
    ///
    /// This is the static-analysis counterpart to the runtime
    /// `ReconcilerIsPure` DST invariant: it gates at PR time that the
    /// `ServiceMapHydrator` reconciler carries no `.await`, no
    /// `Instant::now`, no `SystemTime::now`, no direct DB handle.
    #[test]
    fn service_map_hydrator_reconcile_body_passes_dst_lint() {
        let path = reconciler_source_path("service_map_hydrator.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let violations = inspect_service_map_hydrator_reconcile_body(&source)
            .expect("service_map_hydrator.rs must parse");
        assert!(
            violations.is_empty(),
            "ServiceMapHydrator::reconcile body must contain no banned construct \
             (.await, Instant::now, SystemTime::now, rand::*, tokio::time::sleep, \
             std::thread::sleep); found {} violation(s): {:#?}",
            violations.len(),
            violations
        );
    }

    // -------------------------------------------------------------
    // Envelope variant-construction scanner — Layer 2 of ADR-0048 § 2
    // -------------------------------------------------------------

    /// S-EV-02b.1 — a plain expression-site construction of
    /// `<Envelope>::V<N>(...)` outside any `impl From<...>` block and
    /// outside any `fn latest` / `fn into_latest` body MUST be flagged.
    #[test]
    fn s_ev_02b_1_envelope_v1_call_outside_allowed_sites_is_flagged() {
        let source = r"
            pub fn build_a_row() {
                let payload = make_payload();
                let _ = AllocStatusRowEnvelope::V1(payload);
            }
        ";
        let violations =
            scan_for_envelope_variant_construction(source, std::path::Path::new("synthetic.rs"));
        assert!(
            violations.iter().any(|v| v.banned_path == "AllocStatusRowEnvelope::V1"),
            "<Envelope>::V<N> construction outside allowed sites must be flagged; got {violations:?}"
        );
    }

    /// S-EV-02b.2 — a construction inside a `fn into_latest` body is
    /// the canonical Layer 2 allowed site. Even though the in-repo
    /// convention writes `Self::V1(v1)` (which the scanner ignores
    /// because the head segment is not an `*Envelope`), this test
    /// uses the explicit `<Envelope>::V1(...)` form to exercise the
    /// fn-name allow-list.
    #[test]
    fn s_ev_02b_2_envelope_v1_inside_into_latest_is_allowed() {
        let source = r"
            pub enum FooEnvelope { V1(FooV1) }
            pub struct FooV1;
            impl FooEnvelope {
                pub fn into_latest(self) -> Result<FooV1, ()> {
                    match self {
                        FooEnvelope::V1(payload) => Ok(payload),
                    }
                }
            }
            pub fn rewrap(p: FooV1) -> FooEnvelope {
                // This is OUTSIDE into_latest; deliberately flagged.
                FooEnvelope::V1(p)
            }
        ";
        let violations =
            scan_for_envelope_variant_construction(source, std::path::Path::new("synthetic.rs"));
        // The into_latest body's `FooEnvelope::V1(payload)` is in a
        // pattern position (match arm), not a call expression, so it
        // wouldn't fire anyway — but the body would also be exempt
        // by enclosing-fn name. The standalone `rewrap` call IS
        // flagged. We assert the lint hit exactly that line.
        assert_eq!(
            violations.len(),
            1,
            "exactly one violation expected (the rewrap fn); got {violations:?}"
        );
        assert_eq!(violations[0].banned_path, "FooEnvelope::V1");
    }

    /// S-EV-02b.2b — symmetrical case: a construction *inside* an
    /// `impl From<...>` block (the legitimate inter-version
    /// conversion shape) MUST be allowed.
    #[test]
    fn s_ev_02b_3_envelope_v_n_inside_from_impl_is_allowed() {
        let source = r"
            pub struct FooV1;
            pub struct FooV2;
            pub enum FooEnvelope { V1(FooV1), V2(FooV2) }
            impl From<FooV1> for FooV2 {
                fn from(_v1: FooV1) -> Self {
                    // Rare in practice — From normally returns the
                    // payload, not an envelope — but the lint MUST
                    // allow it because operator intent is clear.
                    let _envelope = FooEnvelope::V2(FooV2);
                    FooV2
                }
            }
        ";
        let violations =
            scan_for_envelope_variant_construction(source, std::path::Path::new("synthetic.rs"));
        assert!(
            violations.is_empty(),
            "construction inside `impl From<...>` block must be allowed; got {violations:?}"
        );
    }

    /// S-EV-02b.4 — source that goes through `<Envelope>::latest(...)`
    /// at the persistence boundary and `envelope.into_latest()?` on
    /// the read side produces zero violations. This is the in-repo
    /// canonical pattern.
    #[test]
    fn s_ev_02b_4_clean_source_produces_no_violations() {
        let source = r"
            pub struct AllocStatusRowV1;
            pub enum AllocStatusRowEnvelope { V1(AllocStatusRowV1) }
            impl AllocStatusRowEnvelope {
                pub fn latest(payload: AllocStatusRowV1) -> Self {
                    Self::V1(payload)
                }
                pub fn into_latest(self) -> Result<AllocStatusRowV1, ()> {
                    match self {
                        Self::V1(v1) => Ok(v1),
                    }
                }
            }
            pub fn persist(row: AllocStatusRowV1) {
                let _envelope = AllocStatusRowEnvelope::latest(row);
            }
        ";
        let violations =
            scan_for_envelope_variant_construction(source, std::path::Path::new("synthetic.rs"));
        assert!(
            violations.is_empty(),
            "clean source must produce zero violations; got {violations:?}"
        );
    }

    /// `is_variant_v_n` reference table — accept `V1`, `V2`, `V42`;
    /// reject `V`, `VX`, `View`, `Variant`.
    #[test]
    fn variant_v_n_matcher_accepts_v_followed_by_digits_only() {
        assert!(is_variant_v_n("V1"));
        assert!(is_variant_v_n("V2"));
        assert!(is_variant_v_n("V42"));
        assert!(!is_variant_v_n("V"));
        assert!(!is_variant_v_n("VX"));
        assert!(!is_variant_v_n("View"));
        assert!(!is_variant_v_n("Variant"));
        assert!(!is_variant_v_n("v1")); // lowercase v rejected
    }

    // -------------------------------------------------------------
    // Envelope fixture-coverage scanner — S-EV-06 (closing-the-loop
    // gate per ADR-0048 § 6).
    // -------------------------------------------------------------

    /// Reference table for `envelope_name_to_snake_case` — the five
    /// real envelopes today. Pins behavior so a future contributor
    /// adding `FooBarBazEnvelope` knows the expected fixture-file
    /// stem without re-reading the algorithm.
    #[test]
    fn envelope_name_to_snake_case_handles_real_envelopes() {
        assert_eq!(envelope_name_to_snake_case("AllocStatusRowEnvelope"), "alloc_status_row");
        assert_eq!(envelope_name_to_snake_case("NodeHealthRowEnvelope"), "node_health_row");
        assert_eq!(
            envelope_name_to_snake_case("ServiceHydrationResultRowEnvelope"),
            "service_hydration_result_row",
        );
        assert_eq!(envelope_name_to_snake_case("ServiceBackendRowEnvelope"), "service_backend_row");
        assert_eq!(envelope_name_to_snake_case("JobEnvelope"), "job");
    }

    /// Build a synthetic crate-root inside `tmp` with `src/lib.rs`
    /// containing `lib_rs_body`. `tests/schema_evolution/<file>` is
    /// created when `fixture_file_body` is `Some((file, body))`.
    /// Returns the crate root path.
    fn build_synthetic_crate_root(
        tmp: &std::path::Path,
        lib_rs_body: &str,
        fixture_file: Option<(&str, &str)>,
    ) -> std::path::PathBuf {
        let crate_root = tmp.join("fake_crate");
        std::fs::create_dir_all(crate_root.join("src")).expect("create_dir_all src must succeed");
        std::fs::write(crate_root.join("src/lib.rs"), lib_rs_body)
            .expect("write src/lib.rs must succeed");
        std::fs::create_dir_all(crate_root.join("tests/schema_evolution"))
            .expect("create_dir_all tests/schema_evolution must succeed");
        if let Some((file_name, body)) = fixture_file {
            std::fs::write(crate_root.join("tests/schema_evolution").join(file_name), body)
                .expect("write fixture file must succeed");
        }
        crate_root
    }

    /// S-EV-06.1 — an envelope defined in `src/` with NO matching
    /// fixture file under `tests/schema_evolution/` MUST be flagged.
    /// The violation message names the envelope and the missing
    /// fixture-file path.
    #[test]
    fn s_ev_06_1_envelope_without_fixture_file_is_flagged() {
        let tmp = tempfile::tempdir().expect("tempdir must succeed");
        // Note: no fixture file passed -> `tests/schema_evolution/`
        // is created empty.
        let crate_root = build_synthetic_crate_root(
            tmp.path(),
            "
                pub enum FooEnvelope { V1(FooV1), V2(FooV2) }
                pub struct FooV1;
                pub struct FooV2;
            ",
            None,
        );

        let violations = scan_for_envelope_fixture_coverage(&crate_root);
        assert!(
            !violations.is_empty(),
            "envelope without fixture file must be flagged; got 0 violations"
        );
        assert!(
            violations.iter().any(|v| v.banned_path.contains("FooEnvelope")),
            "violation must name FooEnvelope; got {violations:?}"
        );
        assert!(
            violations.iter().any(|v| v.banned_path.contains("tests/schema_evolution/foo.rs")
                || v.replacement_trait.contains("tests/schema_evolution/foo.rs")),
            "violation must name expected fixture path 'tests/schema_evolution/foo.rs'; \
             got {violations:?}"
        );
    }

    /// S-EV-06.2 — an envelope's fixture file exists but is missing
    /// a `FIXTURE_V<N>` constant for one of the envelope's variants.
    /// The violation message names the missing variant and the
    /// expected `FIXTURE_V<N>` constant name.
    #[test]
    fn s_ev_06_2_envelope_with_file_but_missing_variant_fixture_is_flagged() {
        let tmp = tempfile::tempdir().expect("tempdir must succeed");
        let crate_root = build_synthetic_crate_root(
            tmp.path(),
            "
                pub enum FooEnvelope { V1(FooV1), V2(FooV2) }
                pub struct FooV1;
                pub struct FooV2;
            ",
            Some((
                "foo.rs",
                r#"
                    const FIXTURE_V1: &str = "deadbeef";
                    #[test] fn v1_decodes() {}
                "#,
            )),
        );

        let violations = scan_for_envelope_fixture_coverage(&crate_root);
        assert!(
            !violations.is_empty(),
            "envelope with file but missing FIXTURE_V2 must be flagged; got 0 violations"
        );
        assert!(
            violations.iter().any(|v| v.banned_path.contains("FooEnvelope::V2")
                || v.banned_path.contains("FIXTURE_V2")
                || v.replacement_trait.contains("FIXTURE_V2")),
            "violation must name FooEnvelope::V2 / FIXTURE_V2; got {violations:?}"
        );
    }

    /// S-EV-06.3 — both variants pinned: scanner returns zero
    /// violations.
    #[test]
    fn s_ev_06_3_complete_coverage_produces_no_violations() {
        let tmp = tempfile::tempdir().expect("tempdir must succeed");
        let crate_root = build_synthetic_crate_root(
            tmp.path(),
            "
                pub enum FooEnvelope { V1(FooV1), V2(FooV2) }
                pub struct FooV1;
                pub struct FooV2;
            ",
            Some((
                "foo.rs",
                r#"
                    const FIXTURE_V1: &str = "deadbeef";
                    const FIXTURE_V2: &str = "cafef00d";
                    #[test] fn v1_decodes() {}
                    #[test] fn v2_decodes() {}
                "#,
            )),
        );

        let violations = scan_for_envelope_fixture_coverage(&crate_root);
        assert!(
            violations.is_empty(),
            "complete coverage must produce zero violations; got {violations:?}"
        );
    }

    /// S-EV-06.4 — the real `overdrive-core` crate-root passes the
    /// coverage gate. All five expected envelope types
    /// (`AllocStatusRowEnvelope`, `NodeHealthRowEnvelope`,
    /// `ServiceHydrationResultRowEnvelope`, `ServiceBackendRowEnvelope`,
    /// `JobEnvelope`) are pinned by `FIXTURE_V1` constants in the
    /// matching `tests/schema_evolution/<envelope>.rs` files.
    #[test]
    fn s_ev_06_4_real_overdrive_core_passes() {
        let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let overdrive_core = crate_dir
            .parent()
            .expect("xtask crate lives directly under workspace root")
            .join("crates/overdrive-core");
        let violations = scan_for_envelope_fixture_coverage(&overdrive_core);
        assert!(
            violations.is_empty(),
            "real overdrive-core crate-root must produce zero violations; \
             found {} violation(s): {:#?}",
            violations.len(),
            violations
        );
    }

    // -------------------------------------------------------------
    // ProbeRunner Earned-Trust gate — ADR-0054 §7 / AC #4
    // -------------------------------------------------------------

    /// Path to the real production `ProbeRunner` impl block.
    fn probe_runner_source_path() -> std::path::PathBuf {
        let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        crate_dir
            .parent()
            .expect("xtask crate lives directly under workspace root")
            .join("crates/overdrive-worker/src/probe_runner/mod.rs")
    }

    /// AC #4 (a) — the scanner clause MUST pass against the real
    /// `crates/overdrive-worker/src/probe_runner/mod.rs` source.
    /// Failure means the Earned-Trust gate has been removed from the
    /// runtime path; refuse to merge.
    #[test]
    fn earned_trust_gate_present_on_real_probe_runner_impl() {
        let path = probe_runner_source_path();
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        inspect_probe_runner_earned_trust_method(&source)
            .expect("real ProbeRunner impl block must declare `async fn probe(&self) -> ...`");
    }

    /// AC #4 (b) — synthetic source with `impl ProbeRunner { ... }`
    /// but no `probe` method MUST fire the lint.
    #[test]
    fn earned_trust_gate_fires_when_probe_method_missing() {
        let source = r"
            pub struct ProbeRunner;
            impl ProbeRunner {
                pub fn new() -> Self { Self }
                pub fn other_method(&self) {}
            }
        ";
        let err = inspect_probe_runner_earned_trust_method(source)
            .expect_err("missing `probe` method must fire the lint");
        let msg = format!("{err}");
        assert!(msg.contains("Earned-Trust"), "error names the missing gate; got {msg:?}");
    }

    /// Synthetic source with a fully-formed `impl ProbeRunner { async
    /// fn probe(&self) -> ... }` block MUST pass.
    #[test]
    fn earned_trust_gate_passes_on_synthetic_clean_impl() {
        let source = r"
            pub struct ProbeRunner;
            pub struct ProbeRunnerError;
            impl ProbeRunner {
                pub async fn probe(&self) -> Result<(), ProbeRunnerError> { Ok(()) }
            }
        ";
        inspect_probe_runner_earned_trust_method(source).expect("clean synthetic impl must pass");
    }

    /// A sync (non-`async`) `probe` method on `impl ProbeRunner`
    /// MUST NOT count as the Earned-Trust gate. The runtime invokes
    /// `.await` on the result; a sync stub would silently bypass the
    /// real adapter call.
    #[test]
    fn earned_trust_gate_rejects_sync_probe_method() {
        let source = r"
            pub struct ProbeRunner;
            impl ProbeRunner {
                pub fn probe(&self) {}  // not async — rejected
            }
        ";
        let _err = inspect_probe_runner_earned_trust_method(source)
            .expect_err("sync `probe` method must NOT satisfy the gate");
    }

    /// A `probe` method that takes extra arguments MUST NOT count as
    /// the Earned-Trust gate. The composition-root invocation per
    /// ADR-0054 §7 calls `runner.probe().await` with no arguments;
    /// a parameterised signature is a different method (the
    /// per-tick `probe_once_and_record`).
    #[test]
    fn earned_trust_gate_rejects_parameterised_probe_method() {
        let source = r"
            pub struct ProbeRunner;
            impl ProbeRunner {
                pub async fn probe(&self, host: &str) {}
            }
        ";
        let _err = inspect_probe_runner_earned_trust_method(source)
            .expect_err("parameterised `probe` method must NOT satisfy the gate");
    }

    /// Source with no `impl ProbeRunner` block at all surfaces an
    /// `Err` rather than silently passing — defends against the
    /// "renamed the type" failure mode.
    #[test]
    fn earned_trust_gate_errors_when_impl_block_absent() {
        let source = r"
            pub struct SomeOtherType;
            impl SomeOtherType {
                pub async fn probe(&self) {}
            }
        ";
        let err = inspect_probe_runner_earned_trust_method(source)
            .expect_err("missing impl ProbeRunner block must fire the lint");
        let msg = format!("{err}");
        assert!(
            msg.contains("impl ProbeRunner"),
            "error names the missing impl block; got {msg:?}"
        );
    }

    // -------------------------------------------------------------
    // Underscore-binding-probe-runner scanner — GAP-5 closure
    // -------------------------------------------------------------

    /// Pattern 1: `let _probe_runner = foo;` MUST be flagged.
    #[test]
    fn underscore_binding_probe_runner_let_bare_form_flagged() {
        let source = r"
            pub fn boot() {
                let probe_runner = make_runner();
                let _probe_runner = probe_runner;
            }
        ";
        let violations = scan_source_underscore_binding_probe_runner(
            source,
            std::path::Path::new("synthetic.rs"),
        )
        .expect("source must scan");
        assert_eq!(
            violations.len(),
            1,
            "let _probe_runner = ... must trigger exactly one violation; got {violations:?}"
        );
        assert_eq!(violations[0].kind, BannedKind::UnderscoreBindingProbeRunner);
        assert_eq!(violations[0].banned_path, "let _probe_runner");
    }

    /// Pattern 1 with a type annotation: `let _probe_runner: Arc<...> = ...;`
    #[test]
    fn underscore_binding_probe_runner_let_with_type_annotation_flagged() {
        let source = r"
            pub fn boot() {
                let _probe_runner: Arc<ProbeRunner> = make_runner();
            }
        ";
        let violations = scan_source_underscore_binding_probe_runner(
            source,
            std::path::Path::new("synthetic.rs"),
        )
        .expect("source must scan");
        assert_eq!(
            violations.len(),
            1,
            "let _probe_runner: T = ... must trigger; got {violations:?}"
        );
    }

    /// Pattern 2: `let _ = probe_runner;` (destructure-discard escape
    /// hatch) MUST be flagged.
    #[test]
    fn underscore_binding_probe_runner_let_underscore_equals_flagged() {
        let source = r"
            pub fn boot() {
                let probe_runner = make_runner();
                let _ = probe_runner;
            }
        ";
        let violations = scan_source_underscore_binding_probe_runner(
            source,
            std::path::Path::new("synthetic.rs"),
        )
        .expect("source must scan");
        assert_eq!(
            violations.len(),
            1,
            "let _ = probe_runner must trigger exactly one violation; got {violations:?}"
        );
        assert_eq!(violations[0].kind, BannedKind::UnderscoreBindingProbeRunner);
        assert_eq!(violations[0].banned_path, "let _ = probe_runner");
    }

    /// Pattern 2 with flexible whitespace: `let  _   =  probe_runner ;`
    #[test]
    fn underscore_binding_probe_runner_let_underscore_equals_whitespace_flexible() {
        let source = "pub fn boot() { let   _    =    probe_runner; }";
        let violations = scan_source_underscore_binding_probe_runner(
            source,
            std::path::Path::new("synthetic.rs"),
        )
        .expect("source must scan");
        assert_eq!(
            violations.len(),
            1,
            "whitespace-flexible `let _ = probe_runner` must trigger; got {violations:?}"
        );
    }

    /// Production destructure shape: `let (driver, _) = compose_production_driver(...).await?;`
    /// MUST NOT be flagged. The discard is a positional tuple slot, not
    /// a bound `_probe_runner` variable, and the helper retains the
    /// runner Arc via `.with_probe_runner(...)` on the driver.
    #[test]
    fn underscore_binding_probe_runner_destructure_tuple_slot_not_flagged() {
        let source = r"
            pub async fn run_server() -> Result<()> {
                let (driver, _) = compose_production_driver(
                    tcp_prober,
                    http_prober,
                    exec_prober,
                    cgroup_root,
                    clock,
                )
                .await?;
                run_server_with_obs_and_driver(config, obs, driver).await
            }
        ";
        let violations = scan_source_underscore_binding_probe_runner(
            source,
            std::path::Path::new("synthetic.rs"),
        )
        .expect("source must scan");
        assert!(
            violations.is_empty(),
            "tuple-slot discard `let (driver, _) = ...` must NOT trigger; got {violations:?}"
        );
    }

    /// Comments naming the literal pattern MUST NOT trigger the gate.
    /// The structural defense is against actual code, not documentation
    /// that describes the failure mode.
    #[test]
    fn underscore_binding_probe_runner_comments_not_flagged() {
        let source = r"
            pub fn boot() {
                // Pre-patch we wrote: let _probe_runner = probe_runner;
                /* Now we write: let _ = probe_runner; */
                /// Docstring: let _probe_runner = foo;
                let probe_runner = make_runner();
                run_with(probe_runner);
            }
        ";
        let violations = scan_source_underscore_binding_probe_runner(
            source,
            std::path::Path::new("synthetic.rs"),
        )
        .expect("source must scan");
        assert!(
            violations.is_empty(),
            "comments naming the literal pattern must NOT trigger; got {violations:?}"
        );
    }

    /// Whole-token matching: `_probe_runner_x` and similar distinct
    /// identifiers MUST NOT match the `_probe_runner` token. Same for
    /// `probe_runner_clone` on the RHS of `let _ = ...`.
    #[test]
    fn underscore_binding_probe_runner_distinct_identifiers_not_flagged() {
        let source = r"
            pub fn boot() {
                let _probe_runner_x = make_runner();
                let _ = probe_runner_clone;
                let _ = probe_runner_handle.spawn();
            }
        ";
        let violations = scan_source_underscore_binding_probe_runner(
            source,
            std::path::Path::new("synthetic.rs"),
        )
        .expect("source must scan");
        assert!(violations.is_empty(), "distinct identifiers must NOT trigger; got {violations:?}");
    }

    /// The actual current `crates/overdrive-control-plane/src/lib.rs`
    /// MUST scan clean — the production composition root uses
    /// `compose_production_driver(...)` + the tuple-slot discard
    /// pattern that does not match either banned shape.
    #[test]
    fn underscore_binding_probe_runner_real_control_plane_lib_clean() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask/.. is workspace root")
            .join("crates/overdrive-control-plane/src/lib.rs");
        let source = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("read {}: {e}", path.display());
        });
        let violations = scan_source_underscore_binding_probe_runner(
            &source,
            std::path::Path::new("crates/overdrive-control-plane/src/lib.rs"),
        )
        .expect("source must scan");
        assert!(
            violations.is_empty(),
            "real control-plane lib.rs must be clean post-GAP-4+5 patch; got {violations:?}"
        );
    }

    /// Path-scoping: source outside `crates/overdrive-control-plane/src/`
    /// is not gated by `scan_workspace`'s dispatch loop. Verify the
    /// scope predicate directly.
    #[test]
    fn underscore_binding_probe_runner_path_scope_in_crate() {
        assert!(underscore_binding_probe_runner_path_in_scope(std::path::Path::new(
            "crates/overdrive-control-plane/src/lib.rs"
        )));
        assert!(underscore_binding_probe_runner_path_in_scope(std::path::Path::new(
            "crates/overdrive-control-plane/src/worker/mod.rs"
        )));
    }

    /// Path-scoping: source outside the control-plane crate's `src/`
    /// is OUT of scope.
    #[test]
    fn underscore_binding_probe_runner_path_scope_out_of_crate() {
        assert!(!underscore_binding_probe_runner_path_in_scope(std::path::Path::new(
            "crates/overdrive-worker/src/probe_runner/mod.rs"
        )));
        assert!(!underscore_binding_probe_runner_path_in_scope(std::path::Path::new(
            "crates/overdrive-control-plane/tests/acceptance/probe_runner_composition.rs"
        )));
        assert!(!underscore_binding_probe_runner_path_in_scope(std::path::Path::new(
            "crates/overdrive-cli/src/main.rs"
        )));
    }
}
