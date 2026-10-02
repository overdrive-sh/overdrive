//! `cargo xtask cloexec-lint` — the creation-time close-on-exec source gate
//! (OBL-295-CLOEXEC; D-295-R3, ADR-0129).
//!
//! Every raw descriptor that first-party code in the `overdrive serve` process
//! creates is created close-on-exec. This gate scans the source of every
//! first-party crate linked into `overdrive serve` and rejects calls under the
//! `libc::`, `nix::`, and `rustix::` paths that create an inheritable
//! descriptor.
//!
//! Shaped like [`crate::dst_lint`]: pure functions over already-read source,
//! with file and metadata access only in [`scan_workspace`] and [`run`]. The
//! scanner is purely syntactic and imports no `overdrive-*` crate.
//!
//! Unlike `dst_lint::scan_workspace`, [`scan_workspace`] fails closed: a file
//! it cannot read or parse is an error, never a skipped file.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use cargo_metadata::{DependencyKind, MetadataCommand, PackageId};
use color_eyre::eyre::{Result, WrapErr, eyre};
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Attribute, Expr, Item, Meta, UseTree};

/// Why a matched call is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloexecRule {
    /// The flag or type argument is resolved and lacks the call's
    /// close-on-exec flag.
    MissingFlag,
    /// The call as written can never set close-on-exec (`accept`, `pipe`,
    /// `dup`, `dup2`, `inotify_init`, `epoll_create`, `fcntl` with `F_DUPFD`,
    /// or a wrapper with no flags argument).
    AlwaysInheritable,
    /// The flag argument is not a literal or constant expression.
    UnresolvedFlag,
}

/// One rejected call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloexecViolation {
    /// The `file` label passed to `scan_source`.
    pub file: PathBuf,
    /// 1-based line of the call.
    pub line: usize,
    /// 1-based column of the call.
    pub column: usize,
    /// The matched call as `<root crate>::<final segment>`, after `use`
    /// renames are resolved (for example `libc::socket` for
    /// `use libc::socket as s; s(..)`).
    pub call: String,
    /// The rule the call violates.
    pub rule: CloexecRule,
}

/// Scan one already-read source file.
///
/// Parses `source` with `syn`; a parse failure is `Err`, distinct from a clean
/// file. Applies the call-family table and the `cloexec-lint: ok` marker
/// (on the line of the call or the line immediately above, suppressing only
/// that line), skips test-only items, and returns violations in source order.
pub fn scan_source(source: &str, file: impl AsRef<Path>) -> Result<Vec<CloexecViolation>> {
    let file = file.as_ref().to_path_buf();
    let parsed = syn::parse_file(source)
        .wrap_err_with(|| format!("parse Rust source {}", file.display()))?;

    let mut imports = ImportMap::default();
    ImportCollector { imports: &mut imports }.visit_file(&parsed);

    let mut scanner = SourceScanner { source, file, imports: &imports, violations: Vec::new() };
    scanner.visit_file(&parsed);
    scanner.violations.sort_by_key(|violation| (violation.line, violation.column));
    Ok(scanner.violations)
}

/// Scan every source file linked into `overdrive serve`.
///
/// Through `cargo_metadata`'s resolved dependency graph, scans the
/// `overdrive-cli` package and every workspace member in its normal
/// (non-dev, non-build) dependency closure: every `src/**/*.rs` file except
/// `src/bin/**`. Fails closed: a file it cannot read, or that
/// [`scan_source`] cannot parse, is `Err` naming that file; metadata without
/// an `overdrive-cli` package is `Err`.
pub fn scan_workspace(manifest_path: &Path) -> Result<Vec<CloexecViolation>> {
    let metadata = MetadataCommand::new()
        .manifest_path(manifest_path)
        .exec()
        .wrap_err_with(|| format!("read Cargo metadata for {}", manifest_path.display()))?;
    let cli = metadata
        .packages
        .iter()
        .find(|package| {
            package.name == "overdrive-cli" && metadata.workspace_members.contains(&package.id)
        })
        .ok_or_else(|| {
            eyre!("workspace {} has no overdrive-cli package", manifest_path.display())
        })?;
    let resolve = metadata.resolve.as_ref().ok_or_else(|| {
        eyre!("Cargo metadata for {} has no resolved graph", manifest_path.display())
    })?;

    let workspace_members: HashSet<PackageId> =
        metadata.workspace_members.iter().cloned().collect();
    let nodes: HashMap<PackageId, _> =
        resolve.nodes.iter().map(|node| (node.id.clone(), node)).collect();
    let mut closure = HashSet::from([cli.id.clone()]);
    let mut pending = VecDeque::from([cli.id.clone()]);

    while let Some(package_id) = pending.pop_front() {
        let Some(node) = nodes.get(&package_id) else {
            return Err(eyre!("Cargo metadata has no node for package {package_id}"));
        };
        for dependency in &node.deps {
            let is_normal =
                dependency.dep_kinds.iter().any(|kind| kind.kind == DependencyKind::Normal);
            if is_normal
                && workspace_members.contains(&dependency.pkg)
                && closure.insert(dependency.pkg.clone())
            {
                pending.push_back(dependency.pkg.clone());
            }
        }
    }

    let mut package_ids: Vec<PackageId> = closure.into_iter().collect();
    package_ids.sort_by_key(ToString::to_string);
    let mut source_files = Vec::new();
    for package_id in package_ids {
        let package = metadata
            .packages
            .iter()
            .find(|package| package.id == package_id)
            .ok_or_else(|| eyre!("resolved workspace package {package_id} is missing"))?;
        let manifest = PathBuf::from(package.manifest_path.as_str());
        let package_root = manifest
            .parent()
            .ok_or_else(|| eyre!("package manifest {} has no parent", manifest.display()))?;
        let source_root = package_root.join("src");
        match fs::read_dir(&source_root) {
            Ok(_) => collect_source_files(&source_root, &source_root, &mut source_files)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .wrap_err_with(|| format!("read source directory {}", source_root.display()));
            }
        }
    }

    source_files.sort();
    let mut violations = Vec::new();
    for source_file in source_files {
        let source = fs::read_to_string(&source_file)
            .wrap_err_with(|| format!("read source file {}", source_file.display()))?;
        let mut found = scan_source(&source, &source_file)
            .wrap_err_with(|| format!("scan source file {}", source_file.display()))?;
        violations.append(&mut found);
    }
    Ok(violations)
}

/// Render one violation as the block [`run`] writes to stderr.
pub fn render_violation(v: &CloexecViolation) -> String {
    format!("{}:{}:{}: {}: {:?}", v.file.display(), v.line, v.column, v.call, v.rule)
}

/// Entry point for `cargo xtask cloexec-lint`: writes each
/// [`render_violation`] block to stderr and returns `Err` when any violation
/// exists.
pub fn run(manifest_path: &Path) -> Result<()> {
    let violations = scan_workspace(manifest_path)?;
    for violation in &violations {
        eprintln!("{}", render_violation(violation));
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(eyre!("close-on-exec source gate found {} violation(s)", violations.len()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CrateRoot {
    Libc,
    Nix,
    Rustix,
}

impl CrateRoot {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "libc" => Some(Self::Libc),
            "nix" => Some(Self::Nix),
            "rustix" => Some(Self::Rustix),
            _ => None,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Libc => "libc",
            Self::Nix => "nix",
            Self::Rustix => "rustix",
        }
    }
}

#[derive(Debug, Clone)]
struct ImportedPath {
    root: CrateRoot,
    path: Vec<String>,
}

#[derive(Default)]
struct ImportMap {
    names: HashMap<String, ImportedPath>,
    globs: Vec<CrateRoot>,
}

impl ImportMap {
    fn add_name(&mut self, local_name: String, target: &[String]) {
        if let Some(root) = target.first().and_then(|name| CrateRoot::parse(name)) {
            self.names.insert(local_name, ImportedPath { root, path: target[1..].to_vec() });
        }
    }

    fn add_glob(&mut self, target: &[String]) {
        if let Some(root) = target.first().and_then(|name| CrateRoot::parse(name)) {
            self.globs.push(root);
        }
    }

    fn resolve_call(&self, path: &syn::Path) -> Option<(CrateRoot, String)> {
        let segments: Vec<String> =
            path.segments.iter().map(|segment| segment.ident.to_string()).collect();
        let first = segments.first()?;
        if let Some(root) = CrateRoot::parse(first) {
            return Some((root, segments.last()?.clone()));
        }
        if let Some(import) = self.names.get(first) {
            let name = if segments.len() > 1 {
                segments.last()?.clone()
            } else {
                import.path.last()?.clone()
            };
            return Some((import.root, name));
        }
        if segments.len() == 1 && is_call_family(first) {
            return self.globs.iter().copied().next().map(|root| (root, first.clone()));
        }
        None
    }

    fn is_imported_constant(&self, path: &syn::Path) -> bool {
        let Some(first) = path.segments.first() else { return false };
        let first = first.ident.to_string();
        CrateRoot::parse(&first).is_some()
            || self.names.contains_key(&first)
            || path.segments.len() > 1
    }
}

struct ImportCollector<'a> {
    imports: &'a mut ImportMap,
}

impl<'ast> Visit<'ast> for ImportCollector<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        if item_requires_test(item_attributes(item)) {
            return;
        }
        if let Item::Use(item_use) = item {
            collect_use_tree(&item_use.tree, &[], self.imports);
        } else {
            syn::visit::visit_item(self, item);
        }
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if !item_requires_test(impl_item_attributes(item)) {
            syn::visit::visit_impl_item(self, item);
        }
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if !item_requires_test(trait_item_attributes(item)) {
            syn::visit::visit_trait_item(self, item);
        }
    }
}

fn collect_use_tree(tree: &UseTree, prefix: &[String], imports: &mut ImportMap) {
    match tree {
        UseTree::Path(path) => {
            let mut nested = prefix.to_vec();
            nested.push(path.ident.to_string());
            collect_use_tree(&path.tree, &nested, imports);
        }
        UseTree::Name(name) => {
            let mut target = prefix.to_vec();
            target.push(name.ident.to_string());
            imports.add_name(name.ident.to_string(), &target);
        }
        UseTree::Rename(rename) => {
            let mut target = prefix.to_vec();
            target.push(rename.ident.to_string());
            imports.add_name(rename.rename.to_string(), &target);
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_tree(item, prefix, imports);
            }
        }
        UseTree::Glob(_) => imports.add_glob(prefix),
    }
}

fn item_requires_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .parse_args::<Meta>()
                .is_ok_and(|predicate| predicate_requires_test(&predicate))
    })
}

fn item_attributes(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        _ => &[],
    }
}

fn impl_item_attributes(item: &syn::ImplItem) -> &[Attribute] {
    match item {
        syn::ImplItem::Const(item) => &item.attrs,
        syn::ImplItem::Fn(item) => &item.attrs,
        syn::ImplItem::Macro(item) => &item.attrs,
        syn::ImplItem::Type(item) => &item.attrs,
        _ => &[],
    }
}

fn trait_item_attributes(item: &syn::TraitItem) -> &[Attribute] {
    match item {
        syn::TraitItem::Const(item) => &item.attrs,
        syn::TraitItem::Fn(item) => &item.attrs,
        syn::TraitItem::Macro(item) => &item.attrs,
        syn::TraitItem::Type(item) => &item.attrs,
        _ => &[],
    }
}

fn foreign_item_attributes(item: &syn::ForeignItem) -> &[Attribute] {
    match item {
        syn::ForeignItem::Fn(item) => &item.attrs,
        syn::ForeignItem::Static(item) => &item.attrs,
        syn::ForeignItem::Type(item) => &item.attrs,
        syn::ForeignItem::Macro(item) => &item.attrs,
        _ => &[],
    }
}

fn predicate_requires_test(predicate: &Meta) -> bool {
    match predicate {
        Meta::Path(path) => path.is_ident("test"),
        Meta::List(list) if list.path.is_ident("all") => list
            .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
            .is_ok_and(|predicates| predicates.iter().any(predicate_requires_test)),
        Meta::List(_) | Meta::NameValue(_) => false,
    }
}

struct SourceScanner<'a> {
    source: &'a str,
    file: PathBuf,
    imports: &'a ImportMap,
    violations: Vec<CloexecViolation>,
}

impl<'ast> Visit<'ast> for SourceScanner<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        if !item_requires_test(item_attributes(item)) {
            syn::visit::visit_item(self, item);
        }
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if !item_requires_test(impl_item_attributes(item)) {
            syn::visit::visit_impl_item(self, item);
        }
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if !item_requires_test(trait_item_attributes(item)) {
            syn::visit::visit_trait_item(self, item);
        }
    }

    fn visit_foreign_item(&mut self, item: &'ast syn::ForeignItem) {
        if !item_requires_test(foreign_item_attributes(item)) {
            syn::visit::visit_foreign_item(self, item);
        }
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        let arguments: Vec<&Expr> = call.args.iter().collect();
        if let Expr::Path(function) = call.func.as_ref()
            && function.qself.is_none()
            && let Some((root, name)) = self.imports.resolve_call(&function.path)
            && let Some(rule) = call_violation(root, &name, &arguments, self.imports)
        {
            let start = function.span().start();
            let violation = CloexecViolation {
                file: self.file.clone(),
                line: start.line,
                column: start.column + 1,
                call: format!("{}::{name}", root.name()),
                rule,
            };
            if !has_exemption_marker(self.source, violation.line) {
                self.violations.push(violation);
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}

fn is_call_family(name: &str) -> bool {
    matches!(
        name,
        "socket"
            | "socketpair"
            | "accept4"
            | "pipe2"
            | "open"
            | "openat"
            | "openat2"
            | "dup3"
            | "fcntl"
            | "memfd_create"
            | "eventfd"
            | "epoll_create"
            | "epoll_create1"
            | "timerfd_create"
            | "signalfd"
            | "inotify_init1"
            | "recvmsg"
            | "recvmmsg"
            | "accept"
            | "pipe"
            | "dup"
            | "dup2"
            | "inotify_init"
    )
}

fn call_violation(
    root: CrateRoot,
    name: &str,
    arguments: &[&Expr],
    imports: &ImportMap,
) -> Option<CloexecRule> {
    if matches!(name, "accept" | "pipe" | "dup" | "dup2" | "inotify_init" | "epoll_create") {
        return Some(CloexecRule::AlwaysInheritable);
    }

    if name == "fcntl" {
        return arguments
            .get(1)
            .filter(|command| expression_has_path_segment(command, "F_DUPFD"))
            .map(|_| CloexecRule::AlwaysInheritable);
    }

    let (index, expected_flag) = match name {
        "socket" | "socketpair" => (if root == CrateRoot::Nix { 2 } else { 1 }, socket_flag(root)),
        "accept4" => (if root == CrateRoot::Libc { 3 } else { 1 }, socket_flag(root)),
        "pipe2" => (usize::from(root == CrateRoot::Libc), open_flag(root)),
        "open" => (1, open_flag(root)),
        "openat" | "openat2" | "dup3" => (2, open_flag(root)),
        "memfd_create" => (1, if root == CrateRoot::Rustix { "CLOEXEC" } else { "MFD_CLOEXEC" }),
        "eventfd" => (1, if root == CrateRoot::Rustix { "CLOEXEC" } else { "EFD_CLOEXEC" }),
        "epoll_create1" => (0, if root == CrateRoot::Rustix { "CLOEXEC" } else { "EPOLL_CLOEXEC" }),
        "timerfd_create" => (1, if root == CrateRoot::Rustix { "CLOEXEC" } else { "TFD_CLOEXEC" }),
        "signalfd" => (2, if root == CrateRoot::Rustix { "CLOEXEC" } else { "SFD_CLOEXEC" }),
        "inotify_init1" => (0, if root == CrateRoot::Rustix { "CLOEXEC" } else { "IN_CLOEXEC" }),
        "recvmsg" => (if root == CrateRoot::Libc { 2 } else { 3 }, message_flag(root)),
        "recvmmsg" => (if root == CrateRoot::Libc { 3 } else { 2 }, message_flag(root)),
        _ => return None,
    };

    match arguments.get(index) {
        None => Some(CloexecRule::AlwaysInheritable),
        Some(argument) if !is_constant_expression(argument, imports) => {
            Some(CloexecRule::UnresolvedFlag)
        }
        Some(argument) if !expression_has_path_segment(argument, expected_flag) => {
            Some(CloexecRule::MissingFlag)
        }
        Some(_) => None,
    }
}

fn socket_flag(root: CrateRoot) -> &'static str {
    if root == CrateRoot::Rustix { "CLOEXEC" } else { "SOCK_CLOEXEC" }
}

fn open_flag(root: CrateRoot) -> &'static str {
    if root == CrateRoot::Rustix { "CLOEXEC" } else { "O_CLOEXEC" }
}

fn message_flag(root: CrateRoot) -> &'static str {
    if root == CrateRoot::Rustix { "CMSG_CLOEXEC" } else { "MSG_CMSG_CLOEXEC" }
}

fn is_constant_expression(expression: &Expr, imports: &ImportMap) -> bool {
    match expression {
        Expr::Lit(_) => true,
        Expr::Path(path) => imports.is_imported_constant(&path.path),
        Expr::Binary(binary) => {
            is_constant_expression(&binary.left, imports)
                && is_constant_expression(&binary.right, imports)
        }
        Expr::Unary(unary) => is_constant_expression(&unary.expr, imports),
        Expr::Paren(paren) => is_constant_expression(&paren.expr, imports),
        Expr::Group(group) => is_constant_expression(&group.expr, imports),
        Expr::Cast(cast) => is_constant_expression(&cast.expr, imports),
        Expr::Call(call) => {
            call.args.is_empty()
                && matches!(call.func.as_ref(), Expr::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "empty"))
        }
        _ => false,
    }
}

fn expression_has_path_segment(expression: &Expr, expected: &str) -> bool {
    struct PathFinder<'a> {
        expected: &'a str,
        found: bool,
    }
    impl<'ast> Visit<'ast> for PathFinder<'_> {
        fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
            self.found |= path.path.segments.iter().any(|segment| segment.ident == self.expected);
            syn::visit::visit_expr_path(self, path);
        }
    }
    let mut finder = PathFinder { expected, found: false };
    finder.visit_expr(expression);
    finder.found
}

fn has_exemption_marker(source: &str, line: usize) -> bool {
    const MARKER: &str = "cloexec-lint: ok";
    let has_reason = |text: &str| {
        text.split_once("//")
            .and_then(|(_, comment)| comment.trim_start().strip_prefix(MARKER))
            .is_some_and(|reason| !reason.trim().is_empty())
    };
    source.lines().nth(line.saturating_sub(1)).is_some_and(has_reason)
        || line > 1 && source.lines().nth(line - 2).is_some_and(has_reason)
}

fn collect_source_files(root: &Path, current: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(current)
        .wrap_err_with(|| format!("read source directory {}", current.display()))?;
    for entry in entries {
        let entry = entry.wrap_err_with(|| format!("read entry in {}", current.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .wrap_err_with(|| format!("inspect source path {}", path.display()))?;
        if file_type.is_dir() {
            if current == root && entry.file_name() == "bin" {
                continue;
            }
            collect_source_files(root, &path, files)?;
        } else if file_type.is_file() && path.extension().is_some_and(|extension| extension == "rs")
        {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
#[expect(
    clippy::doc_markdown,
    reason = "repository-mandated CONTRACT_SHAPE tokens are literal protocol markers"
)]
mod tests {
    //! S-ND295-46 — `scan_source` and `render_violation` over planted sources
    //! (OBL-295-CLOEXEC; feature-delta § "Source gate" and § "Gate entry
    //! point").
    //!
    //! Every planted source starts at column 1 of its literal, so an expected
    //! position reads straight off the source: `line` is the 1-based line of
    //! the call, and `column` is the 1-based column of the first character of
    //! the call's path (the `l` of `libc::socket`, the `s` of a renamed
    //! `s(..)`, the `r` of `recvmsg::<_>(..)`), the anchor `dst_lint` uses for
    //! its own `file:line:col` reports. A call that opens a statement inside a
    //! function body therefore sits at column 5.
    //!
    //! The planted sources are parsed, never compiled. They call `libc`,
    //! `nix`, and `rustix` items with their real signatures, so each call's
    //! flag argument sits where the real API puts it.

    use super::CloexecRule::{AlwaysInheritable, MissingFlag, UnresolvedFlag};
    use super::*;

    /// One planted source file and the exact violations `scan_source` must
    /// report for it, in source order: `(line, column, call, rule)`.
    struct Planted {
        file: &'static str,
        source: &'static str,
        expected: &'static [(usize, usize, &'static str, CloexecRule)],
    }

    type Found = (PathBuf, usize, usize, String, CloexecRule);

    /// The disagreement between `scan_source` and `planted.expected`, if any.
    /// Every reported violation must also carry the `file` label it was
    /// scanned under.
    fn mismatch(planted: &Planted) -> Option<String> {
        let found = scan_source(planted.source, planted.file).unwrap_or_else(|err| {
            panic!("planted source {} must parse: {err:?}\n{}", planted.file, planted.source)
        });
        let actual: Vec<Found> =
            found.into_iter().map(|v| (v.file, v.line, v.column, v.call, v.rule)).collect();
        let expected: Vec<Found> = planted
            .expected
            .iter()
            .map(|&(line, column, call, rule)| {
                (PathBuf::from(planted.file), line, column, call.to_owned(), rule)
            })
            .collect();
        (actual != expected).then(|| {
            format!(
                "{}\n  expected {expected:?}\n  actual   {actual:?}\n--- source ---\n{}",
                planted.file, planted.source
            )
        })
    }

    /// Scan every planted source and report every disagreement at once.
    fn assert_scans(planted: &[Planted]) {
        let mismatches: Vec<String> = planted.iter().filter_map(mismatch).collect();
        assert!(
            mismatches.is_empty(),
            "scan_source disagreed with {} planted source(s):\n\n{}",
            mismatches.len(),
            mismatches.join("\n\n")
        );
    }

    // -----------------------------------------------------------------------
    // One planted source per row of the call-family table. Each flag-family
    // row pairs a call lacking its close-on-exec flag with the same call
    // carrying it, so the rule is proven to read the flag rather than to
    // reject the family outright.
    // -----------------------------------------------------------------------

    /// `socket`, `socketpair`, `accept4`: the type or flag argument lacks
    /// `SOCK_CLOEXEC`.
    const SOCKET_FAMILY: Planted = Planted {
        file: "crates/overdrive-planted/src/socket_family.rs",
        source: "\
fn planted(fd: i32, sv: *mut i32) {
    libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
    libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
    libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET, 0, sv);
    libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0, sv);
    libc::accept4(fd, std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_NONBLOCK);
    libc::accept4(fd, std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC);
}
",
        expected: &[
            (2, 5, "libc::socket", MissingFlag),
            (4, 5, "libc::socketpair", MissingFlag),
            (6, 5, "libc::accept4", MissingFlag),
        ],
    };

    /// `pipe2`, `open`, `openat`, `dup3`: the flag argument lacks
    /// `O_CLOEXEC`. A literal `0` is a resolved flag argument. (`libc` 0.2
    /// exposes no `openat2` function; `openat2` is planted through its
    /// `rustix` wrapper in [`RUSTIX_WRAPPERS`].)
    const OPEN_FAMILY: Planted = Planted {
        file: "crates/overdrive-planted/src/open_family.rs",
        source: "\
fn planted(fds: &mut [i32; 2], path: *const i8, dirfd: i32, oldfd: i32, newfd: i32) {
    libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK);
    libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC);
    libc::pipe2(fds.as_mut_ptr(), 0);
    libc::open(path, libc::O_RDONLY);
    libc::open(path, libc::O_RDONLY | libc::O_CLOEXEC);
    libc::openat(dirfd, path, libc::O_RDWR | libc::O_CREAT, 0o600);
    libc::openat(dirfd, path, libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC, 0o600);
    libc::dup3(oldfd, newfd, 0);
    libc::dup3(oldfd, newfd, libc::O_CLOEXEC);
}
",
        expected: &[
            (2, 5, "libc::pipe2", MissingFlag),
            (4, 5, "libc::pipe2", MissingFlag),
            (5, 5, "libc::open", MissingFlag),
            (7, 5, "libc::openat", MissingFlag),
            (9, 5, "libc::dup3", MissingFlag),
        ],
    };

    /// `fcntl` with `F_DUPFD`: always. `F_DUPFD_CLOEXEC` and a
    /// non-duplicating command (with a variable third argument, as
    /// `overdrive-dataplane/src/mtls/mod.rs` writes it) are not rejected.
    const FCNTL_F_DUPFD: Planted = Planted {
        file: "crates/overdrive-planted/src/fcntl_f_dupfd.rs",
        source: "\
fn planted(fd: i32, flags: i32) {
    libc::fcntl(fd, libc::F_DUPFD, 3);
    libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3);
    libc::fcntl(fd, libc::F_SETFL, flags);
}
",
        expected: &[(2, 5, "libc::fcntl", AlwaysInheritable)],
    };

    /// `memfd_create`: the flag argument lacks `MFD_CLOEXEC`.
    const MEMFD_CREATE: Planted = Planted {
        file: "crates/overdrive-planted/src/memfd_create.rs",
        source: "\
fn planted(name: *const i8) {
    libc::memfd_create(name, libc::MFD_ALLOW_SEALING);
    libc::memfd_create(name, libc::MFD_ALLOW_SEALING | libc::MFD_CLOEXEC);
}
",
        expected: &[(2, 5, "libc::memfd_create", MissingFlag)],
    };

    /// `eventfd`: the flag argument lacks `EFD_CLOEXEC`.
    const EVENTFD: Planted = Planted {
        file: "crates/overdrive-planted/src/eventfd.rs",
        source: "\
fn planted() {
    libc::eventfd(0, libc::EFD_NONBLOCK);
    libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC);
}
",
        expected: &[(2, 5, "libc::eventfd", MissingFlag)],
    };

    /// `epoll_create`: always.
    const EPOLL_CREATE: Planted = Planted {
        file: "crates/overdrive-planted/src/epoll_create.rs",
        source: "\
fn planted() {
    libc::epoll_create(8);
}
",
        expected: &[(2, 5, "libc::epoll_create", AlwaysInheritable)],
    };

    /// `epoll_create1`, `timerfd_create`, `signalfd`, `inotify_init1`: the
    /// flag argument lacks its close-on-exec flag.
    const FLAGGED_CREATE_FAMILY: Planted = Planted {
        file: "crates/overdrive-planted/src/flagged_create_family.rs",
        source: "\
fn planted(mask: *const libc::sigset_t) {
    libc::epoll_create1(0);
    libc::epoll_create1(libc::EPOLL_CLOEXEC);
    libc::timerfd_create(libc::CLOCK_MONOTONIC, libc::TFD_NONBLOCK);
    libc::timerfd_create(libc::CLOCK_MONOTONIC, libc::TFD_NONBLOCK | libc::TFD_CLOEXEC);
    libc::signalfd(-1, mask, libc::SFD_NONBLOCK);
    libc::signalfd(-1, mask, libc::SFD_NONBLOCK | libc::SFD_CLOEXEC);
    libc::inotify_init1(libc::IN_NONBLOCK);
    libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC);
}
",
        expected: &[
            (2, 5, "libc::epoll_create1", MissingFlag),
            (4, 5, "libc::timerfd_create", MissingFlag),
            (6, 5, "libc::signalfd", MissingFlag),
            (8, 5, "libc::inotify_init1", MissingFlag),
        ],
    };

    /// `recvmsg`, `recvmmsg`: the flag argument lacks `MSG_CMSG_CLOEXEC`.
    const RECV_FAMILY: Planted = Planted {
        file: "crates/overdrive-planted/src/recv_family.rs",
        source: "\
fn planted(fd: i32, msg: *mut libc::msghdr, msgvec: *mut libc::mmsghdr) {
    libc::recvmsg(fd, msg, 0);
    libc::recvmsg(fd, msg, libc::MSG_CMSG_CLOEXEC);
    libc::recvmmsg(fd, msgvec, 8, libc::MSG_DONTWAIT, std::ptr::null_mut());
    libc::recvmmsg(fd, msgvec, 8, libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC, std::ptr::null_mut());
}
",
        expected: &[(2, 5, "libc::recvmsg", MissingFlag), (4, 5, "libc::recvmmsg", MissingFlag)],
    };

    /// `accept`, `pipe`, `dup`, `dup2`, `inotify_init`: always; they cannot
    /// set the flag.
    const ALWAYS_INHERITABLE_FAMILY: Planted = Planted {
        file: "crates/overdrive-planted/src/always_inheritable_family.rs",
        source: "\
fn planted(fd: i32, fds: &mut [i32; 2]) {
    libc::accept(fd, std::ptr::null_mut(), std::ptr::null_mut());
    libc::pipe(fds.as_mut_ptr());
    libc::dup(fd);
    libc::dup2(fd, 10);
    libc::inotify_init();
}
",
        expected: &[
            (2, 5, "libc::accept", AlwaysInheritable),
            (3, 5, "libc::pipe", AlwaysInheritable),
            (4, 5, "libc::dup", AlwaysInheritable),
            (5, 5, "libc::dup2", AlwaysInheritable),
            (6, 5, "libc::inotify_init", AlwaysInheritable),
        ],
    };

    /// Calls inside expressions, in the shape of the `nft.rs` and `splice.rs`
    /// sites: the column is the call path's first character, not the start
    /// of the statement. Line 4's `libc` starts at column 23
    /// (`    let fd = unsafe { `); line 9's at column 17 (`    if unsafe { `).
    const CALLS_INSIDE_EXPRESSIONS: Planted = Planted {
        file: "crates/overdrive-planted/src/calls_inside_expressions.rs",
        source: "\
const NETLINK_NETFILTER: i32 = 12;

fn open_with_groups() -> i32 {
    let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, NETLINK_NETFILTER) };
    fd
}

fn pump_pipes(fds: &mut [i32; 2]) -> bool {
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK) } != 0 {
        return false;
    }
    true
}
",
        expected: &[(4, 23, "libc::socket", MissingFlag), (9, 17, "libc::pipe2", MissingFlag)],
    };

    /// Only calls under the `libc::`, `nix::`, and `rustix::` paths are
    /// matched: standard-library and Tokio constructors, a method call, and a
    /// local function that happens to share a family name are not scanned.
    const OUTSIDE_THE_SCANNED_ROOTS: Planted = Planted {
        file: "crates/overdrive-planted/src/outside_the_scanned_roots.rs",
        source: "\
fn planted(path: &str, listener: &std::net::TcpListener) {
    std::fs::File::open(path);
    tokio::fs::File::open(path);
    std::io::pipe();
    listener.accept();
    socket(1, 2, 3);
}

fn socket(domain: i32, ty: i32, protocol: i32) -> i32 {
    domain + ty + protocol
}
",
        expected: &[],
    };

    // -----------------------------------------------------------------------
    // `use` renames and the `nix` / `rustix` wrappers.
    // -----------------------------------------------------------------------

    /// `use libc::socket as s; s(..)` and a crate rename resolve to the
    /// `libc` call.
    const RENAMED_LIBC: Planted = Planted {
        file: "crates/overdrive-planted/src/renamed_libc.rs",
        source: "\
use libc::socket as s;
use libc as c;

fn planted() {
    s(libc::AF_INET, libc::SOCK_STREAM, 0);
    s(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
    c::dup(3);
}
",
        expected: &[(5, 5, "libc::socket", MissingFlag), (7, 5, "libc::dup", AlwaysInheritable)],
    };

    /// `nix` wrappers, imported the way `dns_responder/responder.rs` imports
    /// them: the flag rule reads each wrapper's own flags argument (the third
    /// of `socket`, the fourth of `recvmsg`, the only one of `pipe2`); an
    /// empty flag set lacks the flag; `pipe()` has no flags argument; a
    /// module alias resolves.
    const NIX_WRAPPERS: Planted = Planted {
        file: "crates/overdrive-planted/src/nix_wrappers.rs",
        source: "\
use nix::sys::socket::{recvmsg, socket, AddressFamily, MsgFlags, SockFlag, SockProtocol, SockType, SockaddrIn};
use nix::unistd as unistd_alias;

fn planted(fd: i32, iov: &mut [std::io::IoSliceMut<'_>], cmsg_space: &mut Vec<u8>) {
    socket(AddressFamily::Inet, SockType::Datagram, SockFlag::SOCK_NONBLOCK, SockProtocol::Udp);
    socket(AddressFamily::Inet, SockType::Datagram, SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC, SockProtocol::Udp);
    socket(AddressFamily::Inet, SockType::Datagram, SockFlag::empty(), SockProtocol::Udp);
    recvmsg::<SockaddrIn>(fd, iov, Some(cmsg_space), MsgFlags::MSG_DONTWAIT);
    recvmsg::<SockaddrIn>(fd, iov, Some(cmsg_space), MsgFlags::MSG_CMSG_CLOEXEC);
    recvmsg::<SockaddrIn>(fd, iov, Some(cmsg_space), MsgFlags::empty());
    nix::unistd::pipe2(nix::fcntl::OFlag::O_NONBLOCK);
    nix::unistd::pipe2(nix::fcntl::OFlag::O_NONBLOCK | nix::fcntl::OFlag::O_CLOEXEC);
    nix::unistd::pipe();
    unistd_alias::dup(fd);
}
",
        expected: &[
            (5, 5, "nix::socket", MissingFlag),
            (7, 5, "nix::socket", MissingFlag),
            (8, 5, "nix::recvmsg", MissingFlag),
            (10, 5, "nix::recvmsg", MissingFlag),
            (11, 5, "nix::pipe2", MissingFlag),
            (13, 5, "nix::pipe", AlwaysInheritable),
            (14, 5, "nix::dup", AlwaysInheritable),
        ],
    };

    /// `rustix` wrappers: the flag rule reads the wrapper's `OFlags`
    /// argument, where the close-on-exec flag is spelled `OFlags::CLOEXEC`;
    /// `pipe()` has no flags argument; `rustix` calls outside the families
    /// (`Mode::empty()`) are not matched.
    const RUSTIX_WRAPPERS: Planted = Planted {
        file: "crates/overdrive-planted/src/rustix_wrappers.rs",
        source: "\
use rustix::fs::{Mode, OFlags, ResolveFlags};

fn planted(dirfd: std::os::fd::BorrowedFd<'_>) {
    rustix::fs::openat(dirfd, \"state.redb\", OFlags::RDWR | OFlags::CREATE, Mode::RUSR | Mode::WUSR);
    rustix::fs::openat(dirfd, \"state.redb\", OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC, Mode::RUSR | Mode::WUSR);
    rustix::fs::openat2(dirfd, \"journal\", OFlags::RDONLY, Mode::empty(), ResolveFlags::BENEATH);
    rustix::fs::openat2(dirfd, \"journal\", OFlags::RDONLY | OFlags::CLOEXEC, Mode::empty(), ResolveFlags::BENEATH);
    rustix::pipe::pipe();
}
",
        expected: &[
            (4, 5, "rustix::openat", MissingFlag),
            (6, 5, "rustix::openat2", MissingFlag),
            (8, 5, "rustix::pipe", AlwaysInheritable),
        ],
    };

    /// A flag argument held in a variable is unresolved, even when the
    /// variable was bound to a flagged constant; a variable in a non-flag
    /// argument (line 6's `domain`) is not.
    const UNRESOLVED_FLAG: Planted = Planted {
        file: "crates/overdrive-planted/src/unresolved_flag.rs",
        source: "\
fn planted(domain: i32, flags: i32, oflags: nix::fcntl::OFlag, fds: &mut [i32; 2]) {
    let ty = libc::SOCK_STREAM | libc::SOCK_CLOEXEC;
    libc::socket(libc::AF_INET, ty, 0);
    libc::pipe2(fds.as_mut_ptr(), flags);
    nix::unistd::pipe2(oflags);
    libc::socket(domain, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
}
",
        expected: &[
            (3, 5, "libc::socket", UnresolvedFlag),
            (4, 5, "libc::pipe2", UnresolvedFlag),
            (5, 5, "nix::pipe2", UnresolvedFlag),
        ],
    };

    /// The `// cloexec-lint: ok <reason>` marker on the call's line (line 2)
    /// or the line immediately above it (line 4 for line 5) suppresses that
    /// call. A marker two lines above (line 7 for line 9) suppresses nothing,
    /// and another gate's marker (line 11) is not this gate's exemption.
    const EXEMPTION_MARKER: Planted = Planted {
        file: "crates/overdrive-planted/src/exemption_marker.rs",
        source: "\
fn planted(fds: &mut [i32; 2], flags: i32, fd: i32, msg: *mut libc::msghdr) {
    libc::pipe2(fds.as_mut_ptr(), flags); // cloexec-lint: ok every caller passes O_CLOEXEC in flags
    let _unrelated = fd + 1;
    // cloexec-lint: ok an AF_INET UDP socket cannot carry SCM_RIGHTS
    libc::recvmsg(fd, msg, 0);
    let _unrelated = fd + 2;
    // cloexec-lint: ok two lines above the call, so it covers nothing
    let _unrelated = fd + 3;
    libc::pipe2(fds.as_mut_ptr(), flags);
    libc::recvmsg(fd, msg, 0);
    libc::dup(fd); // dst-lint: hashmap-ok another gate's marker exempts nothing here
}
",
        expected: &[
            (9, 5, "libc::pipe2", UnresolvedFlag),
            (10, 5, "libc::recvmsg", MissingFlag),
            (11, 5, "libc::dup", AlwaysInheritable),
        ],
    };

    /// The same violation in production code (lines 2 and 20) and inside
    /// `#[cfg(test)]` items (a module carrying a further attribute, as
    /// `mtls_intercept.rs` writes it, and a free function): only the
    /// production calls are reported, including the one after the test items.
    const CFG_TEST_ITEMS: Planted = Planted {
        file: "crates/overdrive-planted/src/cfg_test_items.rs",
        source: "\
fn production(fd: i32) {
    libc::dup(fd);
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = \"unit-test bodies\")]
mod tests {
    fn fixture(fd: i32) {
        libc::dup(fd);
        libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
    }
}

#[cfg(test)]
fn fixture_fn(fd: i32) {
    libc::dup(fd);
}

fn after_the_test_items(fd: i32) {
    libc::dup(fd);
}
",
        expected: &[
            (2, 5, "libc::dup", AlwaysInheritable),
            (20, 5, "libc::dup", AlwaysInheritable),
        ],
    };

    /// The test-only boundary (FD § "[REF] Driven port — VMM TAP queue
    /// attachment (D-295-R1, R2, R3, R4) — ACCEPTED 2026-09-24", *Test-only
    /// items*, pinned 2026-09-29, DR-10): an item is test-only when one of its
    /// own `#[cfg(…)]` predicates requires `test` — `test` itself, or an
    /// `all(…)` with an argument that requires `test`, recursively.
    ///
    /// Not reported: the same violation under `all(test, feature = …)`
    /// followed by another attribute (as `overdrive-host`'s
    /// `launch_seccomp_kernel` module is written, line 9), under
    /// `all(feature = …, test)` (line 15), under a nested
    /// `all(…, all(test, …))` (line 20), and on an item whose `#[cfg(test)]`
    /// follows a second `#[cfg(…)]` (line 26). Reported: under
    /// `any(test, …)` (line 31), under `not(test)` (line 36), and on an item
    /// whose only `test` is inside a `cfg_attr(test, …)` (line 41) — none of
    /// those keeps the item out of a non-test build.
    const TEST_ONLY_CFG_PREDICATES: Planted = Planted {
        file: "crates/overdrive-planted/src/test_only_cfg_predicates.rs",
        source: "\
fn production(fd: i32) {
    libc::dup(fd);
}

#[cfg(all(test, feature = \"integration-tests\"))]
#[allow(unsafe_code)]
mod all_test_first {
    fn fixture(fd: i32) {
        libc::dup(fd);
    }
}

#[cfg(all(feature = \"integration-tests\", test))]
fn all_test_last(fd: i32) {
    libc::dup(fd);
}

#[cfg(all(unix, all(test, target_os = \"linux\")))]
fn nested_all(fd: i32) {
    libc::dup(fd);
}

#[cfg(target_os = \"linux\")]
#[cfg(test)]
fn cfg_test_beside_another_cfg(fd: i32) {
    libc::dup(fd);
}

#[cfg(any(test, feature = \"integration-tests\"))]
fn any_test(fd: i32) {
    libc::dup(fd);
}

#[cfg(not(test))]
fn not_test(fd: i32) {
    libc::dup(fd);
}

#[cfg_attr(test, allow(dead_code))]
fn cfg_attr_test(fd: i32) {
    libc::dup(fd);
}
",
        expected: &[
            (2, 5, "libc::dup", AlwaysInheritable),
            (31, 5, "libc::dup", AlwaysInheritable),
            (36, 5, "libc::dup", AlwaysInheritable),
            (41, 5, "libc::dup", AlwaysInheritable),
        ],
    };

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-46 — every rejected call family is reported with its rule
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_rejected_call_family_is_reported_with_its_rule() {
        assert_scans(&[
            SOCKET_FAMILY,
            OPEN_FAMILY,
            FCNTL_F_DUPFD,
            MEMFD_CREATE,
            EVENTFD,
            EPOLL_CREATE,
            FLAGGED_CREATE_FAMILY,
            RECV_FAMILY,
            ALWAYS_INHERITABLE_FAMILY,
            CALLS_INSIDE_EXPRESSIONS,
            OUTSIDE_THE_SCANNED_ROOTS,
        ]);
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-46 — renamed imports and `nix`/`rustix` wrappers resolve to their call
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn renamed_imports_and_nix_or_rustix_wrappers_are_resolved_to_their_call() {
        assert_scans(&[RENAMED_LIBC, NIX_WRAPPERS, RUSTIX_WRAPPERS]);
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-46 — a flag argument held in a variable is rejected as unresolved
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn an_unresolved_flag_argument_is_rejected() {
        assert_scans(&[UNRESOLVED_FLAG]);
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-46 — the exemption marker suppresses only its own line
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn the_exemption_marker_suppresses_only_its_own_line() {
        assert_scans(&[EXEMPTION_MARKER]);
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-46 — `#[cfg(test)]` items are not scanned
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn cfg_test_items_are_not_scanned() {
        assert_scans(&[CFG_TEST_ITEMS]);
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-46 — only a `cfg` predicate that requires `test` exempts an
    /// item; `any`, `not`, and `cfg_attr` never do
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn only_a_cfg_predicate_that_requires_test_exempts_an_item() {
        assert_scans(&[TEST_ONLY_CFG_PREDICATES]);
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-46 — an unparseable source is an error, not a clean file
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn an_unparseable_source_is_an_error_not_a_clean_file() {
        let file = "crates/overdrive-planted/src/unparseable.rs";
        let unparseable = "\
fn broken( {
    libc::dup(fd);
";
        if let Ok(found) = scan_source(unparseable, file) {
            panic!(
                "a source that does not parse must be Err, never a scanned file; got Ok({found:?})"
            );
        }

        // Controls: parseable sources with no call are clean, so the Err
        // above is the parse failure, not a scanner that rejects everything.
        for clean in ["fn clean() {}\n", ""] {
            let found = scan_source(clean, file)
                .unwrap_or_else(|err| panic!("the parseable source {clean:?} must scan: {err:?}"));
            assert!(
                found.is_empty(),
                "the parseable source {clean:?} must be clean; got {found:?}"
            );
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-46 — a rendered violation names its site, call, and rule
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn a_rendered_violation_names_its_site_call_and_rule() {
        // The site values are chosen so no one of them occurs inside another:
        // the path carries no digit, and neither number contains the other.
        let file = "crates/overdrive-dataplane/src/mtls/splice.rs";
        let (line, column, call) = (4817, 263, "nix::recvmsg");
        let rules = [MissingFlag, AlwaysInheritable, UnresolvedFlag];

        let rendered: Vec<String> = rules
            .iter()
            .map(|&rule| {
                render_violation(&CloexecViolation {
                    file: PathBuf::from(file),
                    line,
                    column,
                    call: call.to_owned(),
                    rule,
                })
            })
            .collect();

        let (line_text, column_text) = (line.to_string(), column.to_string());
        for (rule, text) in rules.iter().zip(&rendered) {
            for part in [file, line_text.as_str(), column_text.as_str(), call] {
                assert!(text.contains(part), "the {rule:?} rendering must name {part:?}:\n{text}");
            }
        }

        // The DESIGN does not pin how a rule is spelled. Three renderings of
        // one site that differ only in the rule must differ from each other,
        // so each names its own rule.
        for (i, first) in rendered.iter().enumerate() {
            for second in &rendered[i + 1..] {
                assert_ne!(
                    first, second,
                    "renderings that differ only in the rule must name the rule"
                );
            }
        }
    }
}
