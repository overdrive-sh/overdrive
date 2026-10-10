//! `cargo xtask conformance-lint` — a syntactic gate against hollow
//! `quint-connect` conformance drivers.
//!
//! A conformance driver replays traces generated from a Quint spec against the
//! production owner and compares the driver's `State` with the spec's state
//! after every step (`.claude/rules/testing.md` § "Quint conformance"). A
//! driver is hollow when it compiles and runs green while driving nothing:
//!
//! * an always-false `cfg` (`#![cfg(any())]`, `cfg(not(all()))`, `cfg(false)`)
//!   turns the file into an empty compilation unit, so `cargo check` stays
//!   green over a body that references absent types;
//! * a `switch!` arm, or the driver method it dispatches to, has an empty
//!   body, so the step advances the model and leaves the implementation
//!   untouched;
//! * `State::from_driver` reads a `Sim*` adapter or only the driver's own
//!   bookkeeping, so the comparison checks the model against itself.
//!
//! This module scans every quint-connect driver for those three shapes, and
//! three more that make a conformance target hollow or absent:
//!
//! * a driver pins none of the spec's named runs: it has `#[quint_run]`
//!   randomized traces but no `#[quint_test]`, or a `#[quint_test]` names a run
//!   the spec (and the files it imports) does not declare;
//! * a file references a Quint spec — a string literal naming `specs/quint/`
//!   or a `.qnt` file — without using quint-connect, i.e. checks the
//!   implementation against the spec by some other mechanism;
//! * test or conformance code spawns the `quint` CLI or REPL itself as an
//!   oracle (`Command::new("quint")`, through a `const`, or via a package
//!   runner such as `npx`).
//!
//! The scan covers every Rust file under `crates/*/src/`, `crates/*/tests/`
//! and `tests/conformance/`. A file that never mentions quint or `.qnt` is
//! skipped unparsed; a driver is a file that uses quint-connect (a
//! `quint_connect` path or a `#[quint_run]` / `#[quint_test]` attribute).
//! Doc comments do not count as spec references. A `#[quint_test]` spec path
//! is resolved against the owning crate's manifest directory, as quint-connect
//! resolves it. The `integration-tests` gate is fine; only always-false `cfg`s
//! are flagged. It is shaped like [`crate::cloexec_lint`]: pure functions over
//! already-read source ([`scan_source`], [`scan_file`]), file access only in
//! [`scan_workspace`] and [`run`]. The scanner is purely syntactic, reads Quint
//! specs with the lexical [`spec_index`], and imports no `overdrive-*` crate. It
//! fails closed: a file or spec it cannot read or parse is an error, never a
//! skipped file.
//!
//! One exemption marker exists, for the case the rule permits: a `Sim*`
//! adapter that is the authentic arbiter of a driven-port fact. Write
//! `// conformance-lint: sim-arbiter-ok <reason>` on the line of the read or
//! the line immediately above it. The marker suppresses `sim-in-projection`
//! on that line only, requires a non-empty reason, and counts the read as an
//! owner-side observation for `projection-without-owner-accessor`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, WrapErr, eyre};
use proc_macro2::{Span, TokenStream, TokenTree};

use crate::quint::spec_index;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{
    Attribute, Block, Expr, Fields, FnArg, Ident, ImplItem, ImplItemFn, ItemImpl, ItemStruct,
    ItemUse, LitBool, Meta, Pat, Stmt, Token, Type, UseTree, braced, parenthesized,
};

/// The marker that exempts a justified `Sim*` arbiter read.
pub const SIM_ARBITER_MARKER: &str = "conformance-lint: sim-arbiter-ok";

/// Why a construct is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConformanceRule {
    /// A `cfg` predicate that can never be true gates the driver out of the
    /// build.
    AlwaysFalseCfg,
    /// A `switch!` arm, or the driver method it dispatches to, does nothing.
    EmptyActionHandler,
    /// `State::from_driver` reads a `Sim*` adapter without the arbiter marker.
    SimInProjection,
    /// `State::from_driver` makes no call on a production owner field.
    ProjectionWithoutOwnerAccessor,
    /// A file references a Quint spec but does not use quint-connect.
    SpecWithoutQuintConnect,
    /// Test or conformance code spawns the `quint` CLI (or its REPL) itself.
    QuintCliOracle,
    /// A quint-connect driver pins none of the spec's named runs.
    NoQuintTest,
    /// A `#[quint_test]` names a run its spec does not declare.
    UnknownQuintRun,
}

impl ConformanceRule {
    /// Kebab-case rule name used in diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AlwaysFalseCfg => "always-false-cfg",
            Self::EmptyActionHandler => "empty-action-handler",
            Self::SimInProjection => "sim-in-projection",
            Self::ProjectionWithoutOwnerAccessor => "projection-without-owner-accessor",
            Self::SpecWithoutQuintConnect => "spec-without-quint-connect",
            Self::QuintCliOracle => "quint-cli-oracle",
            Self::NoQuintTest => "no-quint-test",
            Self::UnknownQuintRun => "unknown-quint-run",
        }
    }

    /// The remedy printed with every violation of this rule.
    pub const fn remedy(self) -> &'static str {
        match self {
            Self::AlwaysFalseCfg => {
                "gate the driver on `integration-tests` (in a crate: \
                 `#[cfg(all(test, feature = \"integration-tests\"))]`) and mark an inactive driver \
                 `#[ignore = \"pending DELIVER <step>\"]`; an absent production API is a \
                 DESIGN gap to surface, not something to gate out"
            }
            Self::EmptyActionHandler => {
                "every mapped Quint action must call the production owner through a driving \
                 port; let `switch!` fail on unmapped actions instead of a silent `_ => {}`"
            }
            Self::SimInProjection => {
                "project State from the owner's pinned accessors; a `Sim*` read is allowed only \
                 for the authentic arbiter of a driven-port fact, marked \
                 `// conformance-lint: sim-arbiter-ok <reason>` and paired with a Tier-3 test"
            }
            Self::ProjectionWithoutOwnerAccessor => {
                "State must be read through the accessors the design pins on the production \
                 owner, not from the driver's own maps, counters or literals"
            }
            Self::SpecWithoutQuintConnect => {
                "the only conformance mechanism is quint-connect trace replay of a stateful spec; \
                 drive the production owner from a quint-connect driver, and model a stateless \
                 definition as a state machine of the production-reachable transitions"
            }
            Self::QuintCliOracle => {
                "never use the quint CLI or REPL as an oracle; replay the spec's traces with \
                 quint-connect (`#[quint_test]` / `#[quint_run]`) against the production owner"
            }
            Self::NoQuintTest => {
                "pin each of the spec's named runs with `#[quint_test(spec, main, test = \"<run>\")]` \
                 beside the `#[quint_run]` randomized traces; a spec without named runs is a \
                 DESIGN gap"
            }
            Self::UnknownQuintRun => {
                "name a `run` the spec (or a file it imports) declares, with the spec path \
                 relative to the owning crate's manifest directory"
            }
        }
    }
}

/// One rejected construct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceViolation {
    /// The `file` label passed to [`scan_source`].
    pub file: PathBuf,
    /// 1-based line.
    pub line: usize,
    /// 1-based column.
    pub column: usize,
    /// The rule the construct violates.
    pub rule: ConformanceRule,
    /// What was found, in one sentence.
    pub detail: String,
}

/// Scan one already-read conformance driver.
///
/// Parses `source` with `syn`; a parse failure (including an unparseable
/// `switch!` body) is `Err`, distinct from a clean file. Returns violations in
/// source order.
pub fn scan_source(source: &str, file: impl AsRef<Path>) -> Result<Vec<ConformanceViolation>> {
    let file = file.as_ref().to_path_buf();
    let parsed = syn::parse_file(source)
        .wrap_err_with(|| format!("parse Rust source {}", file.display()))?;
    let mut report = Reporter { file, out: Vec::new() };

    // (a) always-false cfg anywhere in the file.
    let mut cfgs = CfgScanner::default();
    cfgs.visit_file(&parsed);
    for (span, text) in cfgs.hits {
        report.push(
            span,
            ConformanceRule::AlwaysFalseCfg,
            format!("`{text}` can never be true, so the driver compiles to nothing"),
        );
    }

    let mut items = ItemCollector::default();
    items.visit_file(&parsed);
    let structs: BTreeMap<String, FieldMap> =
        items.structs.iter().map(|item| (item.ident.to_string(), field_map(item))).collect();
    let mut inherent: MethodsByType<'_> = BTreeMap::new();
    for item in items.impls.iter().filter(|item| item.trait_.is_none()) {
        let Some(type_name) = self_type_name(item) else { continue };
        let methods = inherent.entry(type_name).or_default();
        for impl_item in &item.items {
            if let ImplItem::Fn(method) = impl_item {
                methods.insert(method.sig.ident.to_string(), method);
            }
        }
    }

    // (b) empty switch! arms and empty dispatched-to driver methods.
    for item in items.impls.iter().filter(|item| trait_name(item).as_deref() == Some("Driver")) {
        check_driver_impl(item, &inherent, &mut report)?;
    }

    // (c) State::from_driver projection.
    for item in items.impls.iter().filter(|item| trait_name(item).as_deref() == Some("State")) {
        for impl_item in &item.items {
            if let ImplItem::Fn(method) = impl_item
                && method.sig.ident == "from_driver"
            {
                check_projection(method, &structs, &inherent, source, &mut report);
            }
        }
    }

    let mut out = report.out;
    out.sort_by_key(|v| (v.line, v.column, v.rule));
    out.dedup();
    Ok(out)
}

type MethodsByType<'ast> = BTreeMap<String, BTreeMap<String, &'ast ImplItemFn>>;

struct Reporter {
    file: PathBuf,
    out: Vec<ConformanceViolation>,
}

impl Reporter {
    fn push(&mut self, span: Span, rule: ConformanceRule, detail: String) {
        let start = span.start();
        self.out.push(ConformanceViolation {
            file: self.file.clone(),
            line: start.line,
            column: start.column + 1,
            rule,
            detail,
        });
    }
}

fn check_driver_impl(
    item: &ItemImpl,
    inherent: &MethodsByType<'_>,
    report: &mut Reporter,
) -> Result<()> {
    let Some(type_name) = self_type_name(item) else { return Ok(()) };
    let mut switches = SwitchCollector::default();
    switches.visit_item_impl(item);
    let mut dispatched = BTreeSet::new();
    for tokens in switches.bodies {
        let switch: Switch = syn::parse2(tokens).wrap_err_with(|| {
            format!(
                "parse switch! body in `impl Driver for {type_name}` in {}",
                report.file.display()
            )
        })?;
        for case in switch.cases {
            match (&case.action, &case.handler) {
                (Some(action), None) => {
                    dispatched.insert(action.to_string());
                }
                (action, Some(handler)) if is_stub_expr(handler) => {
                    let label = action.as_ref().map_or_else(|| "_".to_owned(), ToString::to_string);
                    report.push(
                        case.span,
                        ConformanceRule::EmptyActionHandler,
                        format!("switch! arm `{label}` has an empty handler"),
                    );
                }
                (_, Some(handler)) => {
                    let mut calls = SelfMethodCalls::default();
                    calls.visit_expr(handler);
                    dispatched.extend(calls.names);
                }
                (None, None) => {}
            }
        }
    }
    let Some(methods) = inherent.get(&type_name) else { return Ok(()) };
    for name in &dispatched {
        if let Some(method) = methods.get(name)
            && is_stub_block(&method.block)
        {
            report.push(
                method.sig.ident.span(),
                ConformanceRule::EmptyActionHandler,
                format!(
                    "action handler `{type_name}::{name}` (dispatched from switch!) has an \
                     empty body"
                ),
            );
        }
    }
    Ok(())
}

fn check_projection(
    method: &ImplItemFn,
    structs: &BTreeMap<String, FieldMap>,
    inherent: &MethodsByType<'_>,
    source: &str,
    report: &mut Reporter,
) {
    let Some((param, driver_type)) = driver_param(method) else { return };
    let empty_fields = FieldMap::new();
    let empty_methods = BTreeMap::new();
    let fields = driver_type.as_ref().and_then(|t| structs.get(t)).unwrap_or(&empty_fields);
    let methods = driver_type.as_ref().and_then(|t| inherent.get(t)).unwrap_or(&empty_methods);
    let mut scan = ProjectionScan {
        fields,
        methods,
        visited: BTreeSet::new(),
        sim_hits: Vec::new(),
        accessor_found: false,
    };
    scan.scan_block(&method.block, &param);
    for (span, what) in std::mem::take(&mut scan.sim_hits) {
        if has_arbiter_marker(source, span.start().line) {
            scan.accessor_found = true;
        } else {
            report.push(
                span,
                ConformanceRule::SimInProjection,
                format!("`from_driver` reads `Sim*` state ({what})"),
            );
        }
    }
    if !scan.accessor_found {
        report.push(
            method.sig.ident.span(),
            ConformanceRule::ProjectionWithoutOwnerAccessor,
            "`from_driver` makes no call on a production owner field".to_owned(),
        );
    }
}

/// Whether `source` uses quint-connect: a `quint_connect` path or `use`, or a
/// `#[quint_run]` / `#[quint_test]` attribute.
///
/// A cheap substring check gates the parse so unrelated files are never
/// parsed; a file that mentions quint but cannot be parsed is `Err`.
pub fn is_quint_driver(source: &str, file: impl AsRef<Path>) -> Result<bool> {
    if !["quint_connect", "quint_run", "quint_test"].iter().any(|needle| source.contains(needle)) {
        return Ok(false);
    }
    let parsed = syn::parse_file(source)
        .wrap_err_with(|| format!("parse Rust source {}", file.as_ref().display()))?;
    let mut detector = QuintDetector::default();
    detector.visit_file(&parsed);
    Ok(detector.found)
}

#[derive(Default)]
struct QuintDetector {
    found: bool,
}

fn use_tree_mentions_quint(tree: &UseTree) -> bool {
    match tree {
        UseTree::Path(path) => path.ident == "quint_connect" || use_tree_mentions_quint(&path.tree),
        UseTree::Name(name) => name.ident == "quint_connect",
        UseTree::Rename(rename) => rename.ident == "quint_connect",
        UseTree::Group(group) => group.items.iter().any(use_tree_mentions_quint),
        UseTree::Glob(_) => false,
    }
}

impl<'ast> Visit<'ast> for QuintDetector {
    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        self.found |= use_tree_mentions_quint(&item.tree);
        syn::visit::visit_item_use(self, item);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.found |= path.segments.iter().any(|segment| segment.ident == "quint_connect");
        syn::visit::visit_path(self, path);
    }

    fn visit_attribute(&mut self, attr: &'ast Attribute) {
        self.found |=
            attr.path().segments.last().is_some_and(|segment| {
                segment.ident == "quint_run" || segment.ident == "quint_test"
            });
        syn::visit::visit_attribute(self, attr);
    }
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries =
        fs::read_dir(dir).wrap_err_with(|| format!("read directory {}", dir.display()))?;
    for entry in entries {
        let entry = entry.wrap_err_with(|| format!("read entry in {}", dir.display()))?;
        let path = entry.path();
        let kind = entry.file_type().wrap_err_with(|| format!("stat {}", path.display()))?;
        if kind.is_dir() {
            collect_rs_files(&path, out)?;
        } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Candidate files under the workspace `root`: `crates/*/src/**/*.rs`,
/// `crates/*/tests/**/*.rs` and `tests/conformance/**/*.rs`, sorted.
///
/// Fails closed: a missing `crates` or `tests/conformance` directory is `Err`
/// naming the path.
pub fn candidate_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let crates = root.join("crates");
    let members = fs::read_dir(&crates)
        .wrap_err_with(|| format!("read crates directory {}", crates.display()))?;
    for member in members {
        let member = member.wrap_err_with(|| format!("read entry in {}", crates.display()))?;
        for sub in ["src", "tests"] {
            let dir = member.path().join(sub);
            if dir.is_dir() {
                collect_rs_files(&dir, &mut files)?;
            }
        }
    }
    collect_rs_files(&conformance_root(root), &mut files)?;
    files.sort();
    Ok(files)
}

/// One `#[quint_test]` attribute: the spec it names, as written (relative to
/// the owning crate's manifest directory), and its run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuintTestRef {
    /// The `spec = "…"` argument.
    pub spec: String,
    /// The `test = "…"` argument: the name of a `run` in the spec.
    pub run: String,
    /// 1-based line of the attribute.
    pub line: usize,
    /// 1-based column of the attribute.
    pub column: usize,
}

/// What [`scan_file`] found in one file.
#[derive(Debug, Default)]
pub struct FileScan {
    /// Violations, in source order.
    pub violations: Vec<ConformanceViolation>,
    /// The file's `#[quint_test]` attributes, for [`scan_workspace`] to
    /// resolve against their specs.
    pub quint_tests: Vec<QuintTestRef>,
}

/// Whether a file can concern Quint at all; every other file is skipped
/// unparsed.
fn mentions_quint(source: &str) -> bool {
    source.contains("quint") || source.contains(".qnt")
}

/// Scan one already-read file for every rule that needs no other file.
///
/// - Any file whose code (not its doc comments) holds a string literal
///   naming a Quint spec — a `specs/quint/` path or a `.qnt` file — must use
///   quint-connect (`spec-without-quint-connect`).
/// - No file may spawn the `quint` CLI: `Command::new` with the quint program,
///   directly or through a `const`, or a package runner (`npx`, `deno`, …)
///   given a `quint` argument (`quint-cli-oracle`).
/// - A quint-connect driver gets every [`scan_source`] rule, and must carry at
///   least one `#[quint_test]` (`no-quint-test`).
///
/// A file that mentions quint but cannot be parsed is `Err`.
pub fn scan_file(source: &str, file: impl AsRef<Path>) -> Result<FileScan> {
    let file = file.as_ref().to_path_buf();
    if !mentions_quint(source) {
        return Ok(FileScan::default());
    }
    let parsed = syn::parse_file(source)
        .wrap_err_with(|| format!("parse Rust source {}", file.display()))?;
    let tokens: TokenStream =
        source.parse().map_err(|e| eyre!("lex Rust source {}: {e}", file.display()))?;
    let mut report = Reporter { file: file.clone(), out: Vec::new() };
    let driver = is_quint_driver(source, &file)?;

    if !driver && let Some((span, text)) = first_spec_literal(tokens) {
        report.push(
            span,
            ConformanceRule::SpecWithoutQuintConnect,
            format!("references the Quint spec {text} but does not use quint-connect"),
        );
    }

    let mut consts = StrConsts::default();
    consts.visit_file(&parsed);
    let mut spawns = QuintSpawns { consts: &consts.values, hits: Vec::new() };
    spawns.visit_file(&parsed);
    for (span, what) in spawns.hits {
        report.push(span, ConformanceRule::QuintCliOracle, what);
    }

    let mut quint_tests = Vec::new();
    if driver {
        let mut attrs = QuintAttrs::default();
        attrs.visit_file(&parsed);
        if attrs.tests.is_empty() {
            let (line, column) = attrs.first_run.map_or((1, 1), |span| {
                let start = span.start();
                (start.line, start.column + 1)
            });
            report.out.push(ConformanceViolation {
                file: file.clone(),
                line,
                column,
                rule: ConformanceRule::NoQuintTest,
                detail: "the driver has no `#[quint_test]`: none of the spec's named runs is \
                         replayed"
                    .to_owned(),
            });
        }
        quint_tests = attrs.tests;
        report.out.append(&mut scan_source(source, &file)?);
    }

    let mut violations = report.out;
    violations.sort_by_key(|v| (v.line, v.column, v.rule));
    violations.dedup();
    Ok(FileScan { violations, quint_tests })
}

/// The first string literal in `tokens` that names a Quint spec, skipping
/// doc attributes (`///` and `//!` comments lex as `#[doc = "…"]`).
fn first_spec_literal(tokens: TokenStream) -> Option<(Span, String)> {
    let mut iter = tokens.into_iter().peekable();
    while let Some(tree) = iter.next() {
        match tree {
            TokenTree::Punct(p) if p.as_char() == '#' => {
                let mut look = iter.clone();
                if matches!(look.peek(), Some(TokenTree::Punct(bang)) if bang.as_char() == '!') {
                    look.next();
                }
                if let Some(TokenTree::Group(g)) = look.next()
                    && g.delimiter() == proc_macro2::Delimiter::Bracket
                    && matches!(g.stream().into_iter().next(), Some(TokenTree::Ident(i)) if i == "doc")
                {
                    iter = look;
                }
            }
            TokenTree::Group(g) => {
                if let Some(hit) = first_spec_literal(g.stream()) {
                    return Some(hit);
                }
            }
            TokenTree::Literal(lit) => {
                let text = lit.to_string();
                if names_spec(&text) {
                    return Some((lit.span(), text));
                }
            }
            TokenTree::Ident(_) | TokenTree::Punct(_) => {}
        }
    }
    None
}

/// Whether a literal's source text names a Quint spec: a `specs/quint/` path
/// or a `.qnt` file.
fn names_spec(text: &str) -> bool {
    text.contains("specs/quint/")
        || text.match_indices(".qnt").any(|(at, _)| {
            !text[at + 4..].chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
}

/// `const` / `static` string items of a file, by name.
#[derive(Default)]
struct StrConsts {
    values: BTreeMap<String, String>,
}

fn str_lit(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => Some(s.value()),
        Expr::Reference(r) => str_lit(&r.expr),
        Expr::Group(g) => str_lit(&g.expr),
        Expr::Paren(p) => str_lit(&p.expr),
        _ => None,
    }
}

impl<'ast> Visit<'ast> for StrConsts {
    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        if let Some(v) = str_lit(&item.expr) {
            self.values.insert(item.ident.to_string(), v);
        }
        syn::visit::visit_item_const(self, item);
    }

    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
        if let Some(v) = str_lit(&item.expr) {
            self.values.insert(item.ident.to_string(), v);
        }
        syn::visit::visit_item_static(self, item);
    }
}

/// Package runners that can launch the Quint CLI from its npm package.
const PACKAGE_RUNNERS: [&str; 8] = ["npx", "bunx", "pnpx", "npm", "pnpm", "yarn", "deno", "node"];

/// Whether `program` (as passed to `Command::new`) is the quint CLI.
fn is_quint_program(program: &str) -> bool {
    let base = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let base = base.strip_suffix(".cmd").or_else(|| base.strip_suffix(".exe")).unwrap_or(base);
    base == "quint"
}

fn is_quint_package_arg(arg: &str) -> bool {
    arg == "quint" || arg.starts_with("@informalsystems/quint")
}

/// Calls that spawn the quint CLI.
struct QuintSpawns<'c> {
    consts: &'c BTreeMap<String, String>,
    hits: Vec<(Span, String)>,
}

impl QuintSpawns<'_> {
    /// The program a `Command::new(<program>)` call names, if it is one.
    fn command_program(&self, expr: &Expr) -> Option<String> {
        let Expr::Call(call) = expr else { return None };
        let Expr::Path(func) = &*call.func else { return None };
        let segments: Vec<String> =
            func.path.segments.iter().map(|segment| segment.ident.to_string()).collect();
        if !segments.ends_with(&["Command".to_owned(), "new".to_owned()]) {
            return None;
        }
        let arg = call.args.first()?;
        str_lit(arg).or_else(|| match arg {
            Expr::Path(path) => {
                path.path.get_ident().and_then(|ident| self.consts.get(&ident.to_string()).cloned())
            }
            _ => None,
        })
    }

    fn arg_values(expr: &Expr) -> Vec<String> {
        match expr {
            Expr::Array(array) => array.elems.iter().filter_map(str_lit).collect(),
            Expr::Reference(r) => Self::arg_values(&r.expr),
            Expr::Macro(m) if m.mac.path.is_ident("vec") => m
                .mac
                .parse_body_with(syn::punctuated::Punctuated::<Expr, Token![,]>::parse_terminated)
                .map(|elems| elems.iter().filter_map(str_lit).collect())
                .unwrap_or_default(),
            other => str_lit(other).into_iter().collect(),
        }
    }
}

impl<'ast> Visit<'ast> for QuintSpawns<'_> {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        let expr = Expr::Call(call.clone());
        if let Some(program) = self.command_program(&expr)
            && is_quint_program(&program)
        {
            self.hits.push((
                call.span(),
                format!("spawns the quint CLI (`Command::new(\"{program}\")`) as an oracle"),
            ));
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "arg" || call.method == "args" {
            let mut root = &*call.receiver;
            while let Expr::MethodCall(inner) = root {
                root = &inner.receiver;
            }
            if let Some(runner) = self.command_program(root)
                && PACKAGE_RUNNERS.contains(&runner.rsplit('/').next().unwrap_or(&runner))
                && let Some(arg) =
                    call.args.iter().flat_map(Self::arg_values).find(|a| is_quint_package_arg(a))
            {
                self.hits.push((
                    call.span(),
                    format!("spawns the quint CLI through `{runner} {arg}` as an oracle"),
                ));
            }
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

/// The quint-connect test attributes of a driver.
#[derive(Default)]
struct QuintAttrs {
    tests: Vec<QuintTestRef>,
    first_run: Option<Span>,
}

impl<'ast> Visit<'ast> for QuintAttrs {
    fn visit_attribute(&mut self, attr: &'ast Attribute) {
        let last = attr.path().segments.last().map(|segment| segment.ident.to_string());
        match last.as_deref() {
            Some("quint_run") => {
                self.first_run.get_or_insert_with(|| attr.span());
            }
            Some("quint_test") => {
                let (mut spec, mut run) = (None, None);
                // A malformed attribute fails to compile; only read what parses.
                let _ = attr.parse_nested_meta(|meta| {
                    let value = meta.value()?;
                    if meta.path.is_ident("spec") {
                        spec = Some(value.parse::<syn::LitStr>()?.value());
                    } else if meta.path.is_ident("test") {
                        run = Some(value.parse::<syn::LitStr>()?.value());
                    } else {
                        value.parse::<Expr>()?;
                    }
                    Ok(())
                });
                let start = attr.span().start();
                if let (Some(spec), Some(run)) = (spec, run) {
                    self.tests.push(QuintTestRef {
                        spec,
                        run,
                        line: start.line,
                        column: start.column + 1,
                    });
                }
            }
            _ => {}
        }
        syn::visit::visit_attribute(self, attr);
    }
}

/// The directory of the crate that owns `file`: its nearest ancestor holding
/// a `Cargo.toml`, below `root`. quint-connect resolves a spec path against
/// it (the test's working directory).
fn crate_dir(root: &Path, file: &Path) -> PathBuf {
    file.ancestors()
        .skip(1)
        .take_while(|dir| dir.starts_with(root))
        .find(|dir| dir.join("Cargo.toml").is_file())
        .unwrap_or(root)
        .to_path_buf()
}

/// Every run declared by `spec` or a file it imports (transitively). An
/// unreadable or unindexable spec is `Err`.
fn reachable_runs(spec: &Path) -> Result<BTreeSet<String>> {
    let mut runs = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut todo = vec![spec.to_path_buf()];
    while let Some(next) = todo.pop() {
        if !seen.insert(next.clone()) {
            continue;
        }
        let body = fs::read_to_string(&next)
            .wrap_err_with(|| format!("read Quint spec {}", next.display()))?;
        let idx = spec_index::index(&body)
            .map_err(|e| eyre!("index Quint spec {}: {e}", next.display()))?;
        runs.extend(idx.run_names().map(str::to_owned));
        let dir = next.parent().unwrap_or_else(|| Path::new("."));
        for import in &idx.imports {
            let mut path = dir.join(import);
            if path.extension().is_none_or(|ext| ext != "qnt") {
                path.set_extension("qnt");
            }
            if path.is_file() {
                todo.push(path);
            }
        }
    }
    Ok(runs)
}

/// The `unknown-quint-run` violation of one `#[quint_test]`, if any.
fn check_quint_test(
    root: &Path,
    file: &Path,
    r: &QuintTestRef,
) -> Result<Option<ConformanceViolation>> {
    let base = crate_dir(root, file);
    let spec = base.join(&r.spec);
    let detail = if spec.is_file() {
        let runs = reachable_runs(&spec)?;
        if runs.contains(&r.run) {
            return Ok(None);
        }
        let declared = if runs.is_empty() {
            "none".to_owned()
        } else {
            runs.iter().map(|run| format!("`{run}`")).collect::<Vec<_>>().join(", ")
        };
        format!(
            "`#[quint_test]` names run `{}`, which {} and the files it imports do not declare \
             (declared: {declared})",
            r.run, r.spec
        )
    } else {
        format!(
            "`#[quint_test]` names the spec {}, which does not exist (resolved against {})",
            r.spec,
            base.display()
        )
    };
    Ok(Some(ConformanceViolation {
        file: file.to_path_buf(),
        line: r.line,
        column: r.column,
        rule: ConformanceRule::UnknownQuintRun,
        detail,
    }))
}

/// Scan every candidate file in the workspace rooted at `root`: the
/// [`scan_file`] rules, then each `#[quint_test]` against its spec.
///
/// Fails closed: an unreadable file, a quint-mentioning file that cannot be
/// parsed, or a spec a `#[quint_test]` names that cannot be read or indexed,
/// is `Err` naming the path.
pub fn scan_workspace(root: &Path) -> Result<Vec<ConformanceViolation>> {
    let mut violations = Vec::new();
    for path in candidate_files(root)? {
        let source = fs::read_to_string(&path)
            .wrap_err_with(|| format!("read candidate file {}", path.display()))?;
        let scan = scan_file(&source, &path)?;
        violations.extend(scan.violations);
        for r in &scan.quint_tests {
            violations.extend(check_quint_test(root, &path, r)?);
        }
    }
    Ok(violations)
}

/// The root of the root-level, multi-owner conformance package under `root`.
pub fn conformance_root(root: &Path) -> PathBuf {
    root.join("tests").join("conformance")
}

/// The workspace root for the root manifest at `manifest_path`.
pub fn workspace_root(manifest_path: &Path) -> PathBuf {
    manifest_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

/// Render one violation as the line [`run`] writes to stderr.
pub fn render_violation(v: &ConformanceViolation) -> String {
    format!(
        "{}:{}:{}: conformance-lint [{}]: {}\n    fix: {} (.claude/rules/testing.md § \"Quint conformance\")",
        v.file.display(),
        v.line,
        v.column,
        v.rule.as_str(),
        v.detail,
        v.rule.remedy(),
    )
}

/// Entry point for `cargo xtask conformance-lint`: writes each
/// [`render_violation`] block to stderr and returns `Err` when any violation
/// exists.
pub fn run(manifest_path: &Path) -> Result<()> {
    let violations = scan_workspace(&workspace_root(manifest_path))?;
    for violation in &violations {
        eprintln!("{}", render_violation(violation));
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(eyre!("conformance-driver gate found {} violation(s)", violations.len()))
    }
}

// ---------------------------------------------------------------------------
// (a) cfg evaluation
// ---------------------------------------------------------------------------

#[derive(Default)]
struct CfgScanner {
    hits: Vec<(Span, String)>,
}

impl<'ast> Visit<'ast> for CfgScanner {
    fn visit_attribute(&mut self, attr: &'ast Attribute) {
        if attr.path().is_ident("cfg")
            && let Meta::List(list) = &attr.meta
            && eval_cfg(&list.tokens) == Some(false)
        {
            let text = format!("cfg({})", list.tokens).replace(" (", "(").replace("( ", "(");
            self.hits.push((attr.span(), text));
        }
        syn::visit::visit_attribute(self, attr);
    }
}

/// Statically evaluate a `cfg` predicate. `None` means "depends on the build"
/// (a feature, target, or other configuration name).
fn eval_cfg(tokens: &TokenStream) -> Option<bool> {
    if let Ok(lit) = syn::parse2::<LitBool>(tokens.clone()) {
        return Some(lit.value);
    }
    let Ok(Meta::List(list)) = syn::parse2::<Meta>(tokens.clone()) else { return None };
    let args: Vec<Option<bool>> = split_top_level(&list.tokens).iter().map(eval_cfg).collect();
    let name = list.path.get_ident()?.to_string();
    match name.as_str() {
        "any" if args.contains(&Some(true)) => Some(true),
        "any" if args.iter().all(|a| *a == Some(false)) => Some(false),
        "all" if args.contains(&Some(false)) => Some(false),
        "all" if args.iter().all(|a| *a == Some(true)) => Some(true),
        "not" if args.len() == 1 => args[0].map(|value| !value),
        _ => None,
    }
}

fn split_top_level(tokens: &TokenStream) -> Vec<TokenStream> {
    let mut parts = Vec::new();
    let mut current = TokenStream::new();
    for tree in tokens.clone() {
        if matches!(&tree, TokenTree::Punct(p) if p.as_char() == ',') {
            parts.push(std::mem::take(&mut current));
        } else {
            current.extend([tree]);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

// ---------------------------------------------------------------------------
// item collection helpers
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ItemCollector<'ast> {
    impls: Vec<&'ast ItemImpl>,
    structs: Vec<&'ast ItemStruct>,
}

impl<'ast> Visit<'ast> for ItemCollector<'ast> {
    fn visit_item_impl(&mut self, item: &'ast ItemImpl) {
        self.impls.push(item);
        syn::visit::visit_item_impl(self, item);
    }

    fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
        self.structs.push(item);
        syn::visit::visit_item_struct(self, item);
    }
}

fn last_ident_of_type(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(path) => path.path.segments.last().map(|s| s.ident.to_string()),
        Type::Reference(reference) => last_ident_of_type(&reference.elem),
        Type::Paren(paren) => last_ident_of_type(&paren.elem),
        _ => None,
    }
}

fn self_type_name(item: &ItemImpl) -> Option<String> {
    last_ident_of_type(&item.self_ty)
}

fn trait_name(item: &ItemImpl) -> Option<String> {
    item.trait_
        .as_ref()
        .and_then(|(_, path, _)| path.segments.last())
        .map(|segment| segment.ident.to_string())
}

/// How a driver field is classified for the projection rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    /// The type names a `Sim*` adapter.
    Sim,
    /// A std collection, primitive or wrapper of one: driver bookkeeping.
    Bookkeeping,
    /// Anything else: assumed to be (or contain) the production owner.
    Owner,
}

type FieldMap = BTreeMap<String, FieldKind>;

fn field_map(item: &ItemStruct) -> FieldMap {
    let Fields::Named(named) = &item.fields else { return FieldMap::new() };
    named
        .named
        .iter()
        .filter_map(|field| {
            field.ident.as_ref().map(|ident| (ident.to_string(), classify(&field.ty)))
        })
        .collect()
}

fn is_sim_ident(ident: &Ident) -> bool {
    let name = ident.to_string();
    name.strip_prefix("Sim").and_then(|rest| rest.chars().next()).is_some_and(char::is_uppercase)
}

fn classify(ty: &Type) -> FieldKind {
    #[derive(Default)]
    struct SimFinder {
        found: bool,
    }
    impl<'ast> Visit<'ast> for SimFinder {
        fn visit_ident(&mut self, ident: &'ast Ident) {
            self.found |= is_sim_ident(ident);
        }
    }
    let mut finder = SimFinder::default();
    finder.visit_type(ty);
    if finder.found {
        return FieldKind::Sim;
    }
    if is_bookkeeping(ty) { FieldKind::Bookkeeping } else { FieldKind::Owner }
}

fn is_bookkeeping(ty: &Type) -> bool {
    const WRAPPERS: &[&str] = &["Arc", "Rc", "Box", "Mutex", "RwLock", "RefCell", "Cell"];
    const BOOKKEEPING: &[&str] = &[
        "BTreeMap", "BTreeSet", "HashMap", "HashSet", "Vec", "VecDeque", "String", "Option",
        "bool", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128",
        "isize", "f32", "f64", "char",
    ];
    match ty {
        Type::Tuple(_) | Type::Array(_) | Type::Slice(_) => true,
        Type::Reference(reference) => is_bookkeeping(&reference.elem),
        Type::Paren(paren) => is_bookkeeping(&paren.elem),
        Type::Path(path) => {
            let Some(last) = path.path.segments.last() else { return false };
            let name = last.ident.to_string();
            if WRAPPERS.contains(&name.as_str()) {
                if let syn::PathArguments::AngleBracketed(args) = &last.arguments
                    && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
                {
                    return is_bookkeeping(inner);
                }
                return false;
            }
            BOOKKEEPING.contains(&name.as_str())
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// (b) switch! parsing and stub detection
// ---------------------------------------------------------------------------

#[derive(Default)]
struct SwitchCollector {
    bodies: Vec<TokenStream>,
}

impl<'ast> Visit<'ast> for SwitchCollector {
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac.path.segments.last().is_some_and(|segment| segment.ident == "switch") {
            self.bodies.push(mac.tokens.clone());
        }
        syn::visit::visit_macro(self, mac);
    }
}

/// Mirror of `quint_connect::switch!`'s grammar: `step { case, … }` where a
/// case is `_ => expr` or `Action[(picks)] [=> expr]`.
struct Switch {
    cases: Vec<Case>,
}

struct Case {
    span: Span,
    action: Option<Ident>,
    handler: Option<Expr>,
}

impl Parse for Switch {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let _step: Ident = input.parse()?;
        let body;
        braced!(body in input);
        let mut cases = Vec::new();
        while !body.is_empty() {
            let case: Case = body.parse()?;
            let is_block = matches!(case.handler, Some(Expr::Block(_)));
            cases.push(case);
            if !body.is_empty() {
                if is_block {
                    let _ = body.parse::<Option<Token![,]>>()?;
                } else {
                    body.parse::<Token![,]>()?;
                }
            }
        }
        Ok(Self { cases })
    }
}

impl Parse for Case {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let (span, action) = if input.peek(Token![_]) {
            let underscore: Token![_] = input.parse()?;
            (underscore.span, None)
        } else {
            let action: Ident = input.parse()?;
            if input.peek(syn::token::Paren) {
                let picks;
                parenthesized!(picks in input);
                let _: TokenStream = picks.parse()?;
            }
            (action.span(), Some(action))
        };
        let handler = if input.peek(Token![=>]) {
            input.parse::<Token![=>]>()?;
            Some(input.parse()?)
        } else {
            None
        };
        Ok(Self { span, action, handler })
    }
}

fn is_stub_block(block: &Block) -> bool {
    match block.stmts.as_slice() {
        [] => true,
        [Stmt::Expr(expr, _)] => is_stub_expr(expr),
        [Stmt::Macro(mac)] => is_stub_macro(&mac.mac),
        _ => false,
    }
}

fn is_stub_macro(mac: &syn::Macro) -> bool {
    mac.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "todo" || segment.ident == "unimplemented")
}

fn is_stub_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Block(block) => is_stub_block(&block.block),
        Expr::Tuple(tuple) => tuple.elems.is_empty(),
        Expr::Paren(paren) => is_stub_expr(&paren.expr),
        Expr::Macro(mac) => is_stub_macro(&mac.mac),
        Expr::Call(call) => {
            matches!(&*call.func, Expr::Path(path) if path.path.is_ident("Ok"))
                && call.args.len() == 1
                && call.args.first().is_some_and(is_stub_expr)
        }
        _ => false,
    }
}

#[derive(Default)]
struct SelfMethodCalls {
    names: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for SelfMethodCalls {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if matches!(&*call.receiver, Expr::Path(path) if path.path.is_ident("self")) {
            self.names.insert(call.method.to_string());
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

// ---------------------------------------------------------------------------
// (c) from_driver projection
// ---------------------------------------------------------------------------

/// The `from_driver` parameter name and the driver's type name, if resolvable.
fn driver_param(method: &ImplItemFn) -> Option<(String, Option<String>)> {
    let FnArg::Typed(arg) = method.sig.inputs.first()? else { return None };
    let Pat::Ident(ident) = &*arg.pat else { return None };
    Some((ident.ident.to_string(), last_ident_of_type(&arg.ty)))
}

/// `root.f1.f2…` → `(root, [f1, f2, …])`, peeling `&`, `*` and parentheses.
fn field_chain(expr: &Expr) -> Option<(String, Vec<String>)> {
    match expr {
        Expr::Path(path) => path.path.get_ident().map(|ident| (ident.to_string(), Vec::new())),
        Expr::Field(field) => {
            let (root, mut chain) = field_chain(&field.base)?;
            if let syn::Member::Named(name) = &field.member {
                chain.push(name.to_string());
            } else {
                chain.push(String::new());
            }
            Some((root, chain))
        }
        Expr::Reference(reference) => field_chain(&reference.expr),
        Expr::Paren(paren) => field_chain(&paren.expr),
        Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Deref(_)) => field_chain(&unary.expr),
        _ => None,
    }
}

struct ProjectionScan<'a> {
    fields: &'a FieldMap,
    methods: &'a BTreeMap<String, &'a ImplItemFn>,
    visited: BTreeSet<String>,
    sim_hits: Vec<(Span, String)>,
    accessor_found: bool,
}

impl ProjectionScan<'_> {
    fn scan_block(&mut self, block: &Block, root: &str) {
        let mut visitor = ProjectionVisitor { scan: self, root: root.to_owned() };
        visitor.visit_block(block);
    }

    fn kind(&self, field: &str) -> FieldKind {
        self.fields.get(field).copied().unwrap_or(FieldKind::Owner)
    }
}

struct ProjectionVisitor<'s, 'a> {
    scan: &'s mut ProjectionScan<'a>,
    root: String,
}

impl ProjectionVisitor<'_, '_> {
    /// Classify a field chain rooted at the driver. Returns `true` when the
    /// expression was a driver-rooted chain (and is fully handled).
    fn classify_chain(&mut self, expr: &Expr, via_call: bool) -> bool {
        let Some((root, chain)) = field_chain(expr) else { return false };
        if root != self.root {
            return false;
        }
        let Some(first) = chain.first() else { return true };
        match self.scan.kind(first) {
            FieldKind::Sim => {
                self.scan.sim_hits.push((expr.span(), format!("field `{first}`")));
            }
            FieldKind::Owner if via_call => self.scan.accessor_found = true,
            FieldKind::Owner | FieldKind::Bookkeeping => {}
        }
        true
    }
}

impl<'ast> Visit<'ast> for ProjectionVisitor<'_, '_> {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        match expr {
            Expr::MethodCall(call) => {
                let receiver_is_root = matches!(
                    &*call.receiver,
                    Expr::Path(path) if path.path.is_ident(self.root.as_str())
                );
                if receiver_is_root {
                    // A method on the driver itself: follow its body (once).
                    let name = call.method.to_string();
                    if let Some(method) = self.scan.methods.get(&name).copied()
                        && self.scan.visited.insert(name)
                    {
                        self.scan.scan_block(&method.block, "self");
                    }
                } else if !self.classify_chain(&call.receiver, true) {
                    self.visit_expr(&call.receiver);
                }
                for arg in &call.args {
                    self.visit_expr(arg);
                }
            }
            Expr::Call(call) => {
                self.visit_expr(&call.func);
                for arg in &call.args {
                    if !self.classify_chain(arg, true) {
                        self.visit_expr(arg);
                    }
                }
            }
            Expr::Field(_) => {
                if !self.classify_chain(expr, false) {
                    syn::visit::visit_expr(self, expr);
                }
            }
            _ => syn::visit::visit_expr(self, expr),
        }
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if let Some(segment) = path.segments.iter().find(|segment| is_sim_ident(&segment.ident)) {
            self.scan.sim_hits.push((path.span(), format!("path `{}`", segment.ident)));
        }
        syn::visit::visit_path(self, path);
    }
}

fn has_arbiter_marker(source: &str, line: usize) -> bool {
    let has_reason = |text: &str| {
        text.split_once("//")
            .and_then(|(_, comment)| comment.trim_start().strip_prefix(SIM_ARBITER_MARKER))
            .is_some_and(|reason| !reason.trim().is_empty())
    };
    source.lines().nth(line.saturating_sub(1)).is_some_and(has_reason)
        || line > 1 && source.lines().nth(line - 2).is_some_and(has_reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(source: &str) -> Vec<(usize, ConformanceRule)> {
        scan_source(source, "fixture.rs")
            .expect("fixture parses")
            .into_iter()
            .map(|v| (v.line, v.rule))
            .collect()
    }

    fn count(found: &[(usize, ConformanceRule)], rule: ConformanceRule) -> usize {
        found.iter().filter(|(_, r)| *r == rule).count()
    }

    /// The shape of the hollow driver this gate exists for: always-false gate,
    /// empty handlers, a silent catch-all, and a projection built from a
    /// `Sim*` adapter plus the driver's own maps.
    const HOLLOW: &str = r"
#![cfg(any())]
use std::collections::BTreeMap;
use std::sync::Arc;
use quint_connect::{Driver, Result, State, Step, switch};

struct LeaseState { kern: BTreeMap<u32, u32>, lease: BTreeMap<u32, u32>, cursor: u32 }

impl State<LeaseDriver> for LeaseState {
    fn from_driver(driver: &LeaseDriver) -> Result<Self> {
        let held = driver.cid_claim.held_cids();
        let mut kern: BTreeMap<u32, u32> = (0..driver.span).map(|o| (o, 0u32)).collect();
        for cid in held { kern.insert(cid, *driver.holder.get(&cid).unwrap_or(&0)); }
        let lease = driver.observe_leases();
        let cursor = driver.observe_cursor();
        Ok(LeaseState { kern, lease, cursor })
    }
}

struct LeaseDriver {
    cid_claim: Arc<SimGuestCidClaim>,
    span: u32,
    holder: BTreeMap<u32, u32>,
}

impl LeaseDriver {
    fn observe_leases(&self) -> BTreeMap<u32, u32> { self.holder.clone() }
    fn observe_cursor(&self) -> u32 { 0 }
    fn init(&mut self) {}
    fn survivor_create(&mut self, o: u32) { self.holder.insert(o, 1); }
    fn assign_begin(&mut self, _a: u32) {}
    fn stop(&mut self, _a: u32) { Ok(()) }
    fn boot(&mut self) { todo!() }
}

impl Driver for LeaseDriver {
    type State = LeaseState;
    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            init,
            SurvivorCreate(o: u32) => self.survivor_create(o),
            AssignBegin(a: u32) => self.assign_begin(a),
            Stop(a: u32) => { self.stop(a) }
            Boot => self.boot(),
            Redeploy(a: u32) => {},
            _ => {}
        })
    }
}
";

    #[test]
    fn hollow_driver_trips_every_rule() {
        let found = rules(HOLLOW);
        assert_eq!(count(&found, ConformanceRule::AlwaysFalseCfg), 1, "{found:?}");
        // init, assign_begin, stop, boot (empty methods) + Redeploy + `_` arms.
        assert_eq!(count(&found, ConformanceRule::EmptyActionHandler), 6, "{found:?}");
        // `driver.cid_claim` (a Sim field).
        assert_eq!(count(&found, ConformanceRule::SimInProjection), 1, "{found:?}");
        assert_eq!(count(&found, ConformanceRule::ProjectionWithoutOwnerAccessor), 1, "{found:?}");
        // survivor_create has a body; it is not reported.
        let details = scan_source(HOLLOW, "fixture.rs").expect("fixture parses");
        assert!(details.iter().all(|v| !v.detail.contains("survivor_create")), "{details:#?}");
    }

    const HEALTHY: &str = r#"
#![cfg(feature = "integration-tests")]
use std::sync::Arc;
use quint_connect::{Driver, Result, State, Step, quint_run, switch};

struct LeaseState { lease: Vec<u32>, cursor: u32, kern: Vec<u32> }

impl State<LeaseDriver> for LeaseState {
    fn from_driver(driver: &LeaseDriver) -> Result<Self> {
        let obs = driver.pool.observe_leases()?;
        // conformance-lint: sim-arbiter-ok the kernel holder table is the claim port's arbiter; Tier-3 twin in cid_claim_real.rs
        let kern = driver.cid_claim.held_cids();
        Ok(LeaseState { lease: obs.leases, cursor: obs.cursor, kern })
    }
}

struct LeaseDriver {
    pool: GuestAddressPool,
    cid_claim: Arc<SimGuestCidClaim>,
}

impl LeaseDriver {
    fn init(&mut self) { self.pool.reset(); }
    fn assign_begin(&mut self, a: u32) -> anyhow::Result<()> { self.pool.assign(a) }
}

impl Driver for LeaseDriver {
    type State = LeaseState;
    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            init,
            AssignBegin(a: u32) => self.assign_begin(a)?,
            Stop(a: u32) => { self.pool.stop(a)?; }
            _ => anyhow::bail!("unmapped action {}", step.action_taken),
        })
    }
}

#[cfg(test)]
mod nested {
    #[cfg(all(feature = "integration-tests", unix))]
    fn gated() {}
}

#[quint_run(spec = "../../specs/quint/x/lease.qnt", main = "lease_ok")]
#[ignore = "pending DELIVER 03-01"]
fn lease_conformance() -> impl Driver { LeaseDriver::new() }
"#;

    #[test]
    fn healthy_driver_passes() {
        let found = scan_source(HEALTHY, "fixture.rs").expect("fixture parses");
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn always_false_cfg_shapes_are_detected_anywhere() {
        let src = r#"
#[cfg(any())]
mod pending {}
#[cfg(not(all()))]
fn a() {}
#[cfg(false)]
fn b() {}
#[cfg(all(feature = "integration-tests", any()))]
fn c() {}
#[cfg(any(any(), not(any(feature = "x", all()))))]
fn d() {}
#[cfg(any(feature = "x", any()))]
fn depends() {}
#[cfg(not(any()))]
fn always_true() {}
"#;
        let found = rules(src);
        let lines: Vec<usize> = found
            .iter()
            .filter(|(_, r)| *r == ConformanceRule::AlwaysFalseCfg)
            .map(|(l, _)| *l)
            .collect();
        assert_eq!(lines, vec![2, 4, 6, 8, 10], "{found:?}");
    }

    #[test]
    fn unmarked_or_reasonless_sim_read_trips_and_marker_covers_only_its_line() {
        let src = r"
impl State<D> for S {
    fn from_driver(driver: &D) -> Result<Self> {
        let a = driver.owner.view();
        // conformance-lint: sim-arbiter-ok
        let b = driver.sim.held();
        let c = SimClock::now_of(&driver.owner);
        Ok(S { a, b, c })
    }
}
struct D { owner: Owner, sim: SimThing }
";
        let found = rules(src);
        assert_eq!(
            found
                .iter()
                .filter(|(_, r)| *r == ConformanceRule::SimInProjection)
                .map(|(l, _)| *l)
                .collect::<Vec<_>>(),
            vec![6, 7],
            "{found:?}"
        );
        assert_eq!(count(&found, ConformanceRule::ProjectionWithoutOwnerAccessor), 0);
    }

    #[test]
    fn projection_through_driver_helper_that_reads_owner_passes() {
        let src = r"
impl State<D> for S {
    fn from_driver(driver: &D) -> Result<Self> { Ok(S { x: driver.observe() }) }
}
struct D { pool: Pool, seen: Vec<u32> }
impl D { fn observe(&self) -> u32 { self.pool.cursor() } }
";
        assert!(rules(src).is_empty());
        let bookkeeping_only = r"
impl State<D> for S {
    fn from_driver(driver: &D) -> Result<Self> { Ok(S { x: driver.observe(), y: driver.seen.len() }) }
}
struct D { pool: Pool, seen: Vec<u32> }
impl D { fn observe(&self) -> u32 { self.seen.clone().len() as u32 } }
";
        assert_eq!(
            rules(bookkeeping_only),
            vec![(3, ConformanceRule::ProjectionWithoutOwnerAccessor)]
        );
    }

    #[test]
    fn render_names_file_line_and_rule() {
        let v = scan_source(HOLLOW, "tests/conformance/tests/x_conformance.rs")
            .expect("fixture parses")
            .into_iter()
            .next()
            .expect("at least one violation");
        let text = render_violation(&v);
        assert!(
            text.starts_with(
                "tests/conformance/tests/x_conformance.rs:2:1: conformance-lint [always-false-cfg]"
            ),
            "{text}"
        );
    }

    fn fixture_workspace() -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(root.path().join("crates/owner/src/deep")).expect("mkdir");
        fs::create_dir_all(root.path().join("crates/owner/tests")).expect("mkdir");
        fs::create_dir_all(root.path().join("tests/conformance/tests")).expect("mkdir");
        fs::create_dir_all(root.path().join("tests/conformance/src")).expect("mkdir");
        fs::create_dir_all(root.path().join("specs/quint/x")).expect("mkdir");
        fs::write(root.path().join("crates/owner/Cargo.toml"), "[package]\nname = \"owner\"\n")
            .expect("write");
        fs::write(root.path().join("specs/quint/x/lease.qnt"), LEASE_SPEC).expect("write");
        fs::write(root.path().join("specs/quint/x/lease_ok.qnt"), LEASE_INSTANCE).expect("write");
        root
    }

    /// A stateful spec with one named run, and an instance file importing it.
    const LEASE_SPEC: &str = "module lease {\n  var n: int\n  action init = n' = 0\n  action step = n' = n + 1\n  run happyPath = init.then(step)\n}\n";
    const LEASE_INSTANCE: &str = "module lease_ok { import lease.* from \"./lease\" }\n";

    /// A hollow driver placed as a gated module inside a crate's `src/`.
    const HOLLOW_IN_CRATE: &str = r#"
#[cfg(all(test, feature = "integration-tests"))]
mod conformance {
    use quint_connect::{Driver, Result, State, Step, switch};
    struct S { x: u32 }
    struct D { seen: Vec<u32> }
    impl State<D> for S {
        fn from_driver(driver: &D) -> Result<Self> { Ok(S { x: driver.seen.len() as u32 }) }
    }
    impl Driver for D {
        type State = S;
        fn step(&mut self, step: &Step) -> Result {
            switch!(step { Stop(a: u32) => {}, _ => {} })
        }
    }
}
"#;

    /// A healthy driver in the same placement; the `integration-tests` gate is
    /// not an always-false cfg.
    const HEALTHY_IN_CRATE: &str = r#"
#[cfg(all(test, feature = "integration-tests"))]
mod conformance {
    use quint_connect::{Driver, Result, State, Step, switch};
    struct S { x: u32 }
    struct D { owner: Owner }
    impl State<D> for S {
        fn from_driver(driver: &D) -> Result<Self> { Ok(S { x: driver.owner.cursor() }) }
    }
    impl Driver for D {
        type State = S;
        fn step(&mut self, step: &Step) -> Result {
            switch!(step {
                Stop(a: u32) => self.owner.stop(a)?,
                _ => anyhow::bail!("unmapped"),
            })
        }
    }
    #[quint_test(spec = "../../specs/quint/x/lease.qnt", main = "lease", test = "happyPath")]
    fn happy_path() -> impl Driver { D::new() }
    #[quint_run(spec = "../../specs/quint/x/lease.qnt", main = "lease")]
    fn random() -> impl Driver { D::new() }
}
"#;

    #[test]
    fn scan_workspace_finds_in_crate_drivers_and_flags_only_the_hollow_one() {
        let root = fixture_workspace();
        fs::write(root.path().join("crates/owner/src/deep/hollow.rs"), HOLLOW_IN_CRATE)
            .expect("write");
        fs::write(root.path().join("crates/owner/src/healthy.rs"), HEALTHY_IN_CRATE)
            .expect("write");
        fs::write(root.path().join("tests/conformance/tests/multi.rs"), HOLLOW).expect("write");
        let found = scan_workspace(root.path()).expect("scan");
        assert!(found.iter().any(|v| v.file.ends_with("deep/hollow.rs")), "{found:#?}");
        assert!(found.iter().any(|v| v.file.ends_with("tests/multi.rs")), "{found:#?}");
        assert!(found.iter().all(|v| !v.file.ends_with("healthy.rs")), "{found:#?}");
        assert!(
            found.iter().all(
                |v| v.rule != ConformanceRule::AlwaysFalseCfg || !v.file.ends_with("hollow.rs")
            ),
            "the integration-tests gate must not be flagged: {found:#?}"
        );
    }

    #[test]
    fn non_quint_src_file_is_ignored_even_when_it_looks_hollow() {
        let root = fixture_workspace();
        // Always-false cfg and an empty `Driver` impl, but no quint-connect use.
        let src = "#[cfg(any())]\nmod m {}\nimpl Driver for D { fn step(&mut self) { switch!(step { _ => {} }) } }\n";
        fs::write(root.path().join("crates/owner/src/plain.rs"), src).expect("write");
        // A file that mentions quint only in prose or a string is not a driver.
        fs::write(
            root.path().join("crates/owner/src/prose.rs"),
            "// see quint_connect docs\nfn f() -> &'static str { \"quint_run\" }\n",
        )
        .expect("write");
        assert!(scan_workspace(root.path()).expect("scan").is_empty());
    }

    #[test]
    fn scan_workspace_fails_closed() {
        let root = fixture_workspace();
        fs::write(root.path().join("crates/owner/src/broken.rs"), "use quint_connect::; fn (")
            .expect("write");
        assert!(scan_workspace(root.path()).is_err());
        assert!(scan_workspace(&root.path().join("missing")).is_err());
    }

    #[test]
    fn quint_driver_detection_covers_use_path_and_attributes() {
        for src in [
            "use quint_connect::Driver;",
            "use quint_connect::{Driver, State};",
            "fn f() { let _ = quint_connect::Step::default(); }",
            "#[quint_run(spec = \"x\")] fn t() {}",
            "#[quint_connect::quint_test(spec = \"x\")] fn t() {}",
        ] {
            assert!(is_quint_driver(src, "f.rs").expect("parses"), "{src}");
        }
        assert!(!is_quint_driver("fn f() {}", "f.rs").expect("parses"));
    }

    fn file_rules(source: &str) -> Vec<(usize, ConformanceRule)> {
        scan_file(source, "fixture.rs")
            .expect("fixture parses")
            .violations
            .into_iter()
            .map(|v| (v.line, v.rule))
            .collect()
    }

    #[test]
    fn a_spec_reference_without_quint_connect_is_flagged_but_a_doc_mention_is_not() {
        // The tests/conformance prefix_landing shape: the spec path built into
        // a const, evaluated by some other mechanism.
        let src = r#"//! Spec: `specs/quint/x/landing.qnt`, referenced in place.
/// Replays nothing; see specs/quint/x/landing.qnt.
const SPEC: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../specs/quint/x/landing.qnt");
fn lands() -> bool { true }
"#;
        assert_eq!(file_rules(src), vec![(4, ConformanceRule::SpecWithoutQuintConnect)]);
        // Only doc comments mention the spec: not a reference.
        let doc_only =
            "//! Spec: `specs/quint/x/landing.qnt`.\n/// see `prefix_boot.qnt`\nfn f() {}\n";
        assert!(file_rules(doc_only).is_empty(), "{:?}", file_rules(doc_only));
        // A bare `.qnt` file literal counts; a longer extension does not.
        assert_eq!(
            file_rules("fn f() -> &'static str { \"owner_flows.qnt\" }"),
            vec![(1, ConformanceRule::SpecWithoutQuintConnect)]
        );
        assert!(file_rules("fn f() -> &'static str { \"quint.qnty\" }").is_empty());
        // In a quint-connect driver the reference is the point.
        let driver =
            "#[quint_test(spec = \"../../specs/quint/x/lease.qnt\", test = \"r\")]\nfn t() {}\n";
        assert_eq!(count(&file_rules(driver), ConformanceRule::SpecWithoutQuintConnect), 0);
    }

    #[test]
    fn spawning_the_quint_cli_is_flagged_in_every_shape() {
        let src = r#"
use std::process::Command;
const QUINT: &str = "quint";
fn a() { Command::new("quint").arg("repl"); }
fn b() { std::process::Command::new("/usr/local/bin/quint").arg("test"); }
fn c() { Command::new(QUINT).arg("run"); }
fn d() { Command::new("npx").arg("@informalsystems/quint").arg("repl"); }
fn e() { tokio::process::Command::new("npx").args(["quint", "test"]); }
fn f() { Command::new("quint.cmd"); }
"#;
        let lines: Vec<usize> = file_rules(src)
            .into_iter()
            .filter(|(_, r)| *r == ConformanceRule::QuintCliOracle)
            .map(|(l, _)| l)
            .collect();
        assert_eq!(lines, vec![4, 5, 6, 7, 8, 9]);
        // Not the quint CLI.
        let fine = r#"
use std::process::Command;
fn a() { Command::new("rustc").arg("--edition=2024"); }
fn b() { Command::new("quintessence"); }
fn c() { Command::new("npx").arg("prettier"); }
fn d() -> &'static str { "quint" }
"#;
        assert!(file_rules(fine).is_empty(), "{:?}", file_rules(fine));
    }

    #[test]
    fn a_driver_without_a_quint_test_is_flagged_at_its_first_quint_run() {
        let src = "use quint_connect::Driver;\n\n#[quint_run(spec = \"s.qnt\", main = \"m\")]\nfn random() -> impl Driver { D::new() }\n";
        assert_eq!(count(&file_rules(src), ConformanceRule::NoQuintTest), 1);
        assert!(
            file_rules(src).contains(&(3, ConformanceRule::NoQuintTest)),
            "{:?}",
            file_rules(src)
        );
        let pinned = format!(
            "{src}#[quint_test(spec = \"s.qnt\", main = \"m\", test = \"happy\")]\nfn happy() -> impl Driver {{ D::new() }}\n"
        );
        assert_eq!(count(&file_rules(&pinned), ConformanceRule::NoQuintTest), 0);
        let scan = scan_file(&pinned, "fixture.rs").expect("parses");
        assert_eq!(
            scan.quint_tests,
            vec![QuintTestRef { spec: "s.qnt".into(), run: "happy".into(), line: 5, column: 1 }]
        );
        // A non-driver is never required to carry one.
        assert_eq!(count(&file_rules("fn quint_helper() {}"), ConformanceRule::NoQuintTest), 0);
    }

    #[test]
    fn a_quint_test_must_name_a_run_its_spec_declares() {
        let root = fixture_workspace();
        let driver = |test: &str, spec: &str| {
            format!(
                "use quint_connect::Driver;\n#[quint_test(spec = \"{spec}\", main = \"lease_ok\", test = \"{test}\")]\nfn t() -> impl Driver {{ D::new() }}\n"
            )
        };
        let path = root.path().join("crates/owner/tests/lease_conformance.rs");
        // Declared by the spec, or by a file the named spec imports.
        for spec in ["../../specs/quint/x/lease.qnt", "../../specs/quint/x/lease_ok.qnt"] {
            fs::write(&path, driver("happyPath", spec)).expect("write");
            let found = scan_workspace(root.path()).expect("scan");
            assert!(found.iter().all(|v| v.rule != ConformanceRule::UnknownQuintRun), "{found:#?}");
        }
        fs::write(&path, driver("happyPth", "../../specs/quint/x/lease.qnt")).expect("write");
        let found = scan_workspace(root.path()).expect("scan");
        let unknown: Vec<_> =
            found.iter().filter(|v| v.rule == ConformanceRule::UnknownQuintRun).collect();
        assert_eq!(unknown.len(), 1, "{found:#?}");
        assert!(
            unknown[0].detail.contains("names run `happyPth`")
                && unknown[0].detail.contains("(declared: `happyPath`)"),
            "{}",
            unknown[0].detail
        );
        assert_eq!((unknown[0].line, unknown[0].file.clone()), (2, path.clone()));
        fs::write(&path, driver("happyPath", "../../specs/quint/x/missing.qnt")).expect("write");
        let found = scan_workspace(root.path()).expect("scan");
        assert!(
            found.iter().any(|v| v.rule == ConformanceRule::UnknownQuintRun
                && v.detail.contains("does not exist")),
            "{found:#?}"
        );
    }

    #[test]
    fn scope_covers_crate_tests_and_the_whole_conformance_package() {
        let root = fixture_workspace();
        let oracle = "fn f() { std::process::Command::new(\"quint\").arg(\"repl\"); }\n";
        fs::write(root.path().join("crates/owner/tests/oracle.rs"), oracle).expect("write");
        fs::write(root.path().join("tests/conformance/src/landing.rs"), oracle).expect("write");
        let found = scan_workspace(root.path()).expect("scan");
        for file in ["crates/owner/tests/oracle.rs", "tests/conformance/src/landing.rs"] {
            assert!(
                found
                    .iter()
                    .any(|v| v.file.ends_with(file) && v.rule == ConformanceRule::QuintCliOracle),
                "{file}: {found:#?}"
            );
        }
        let files = candidate_files(root.path()).expect("candidates");
        assert!(files.iter().any(|f| f.ends_with("crates/owner/tests/oracle.rs")));
        assert!(files.iter().all(|f| f.extension().is_some_and(|e| e == "rs")));
    }

    #[test]
    fn workspace_root_resolves_relative_to_manifest() {
        assert_eq!(workspace_root(Path::new("Cargo.toml")), Path::new("."));
        assert_eq!(workspace_root(Path::new("/w/Cargo.toml")), Path::new("/w"));
        assert_eq!(conformance_root(Path::new("/w")), Path::new("/w/tests/conformance"));
    }
}
