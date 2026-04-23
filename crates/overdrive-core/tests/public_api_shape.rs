//! §2.3 — Public-API-shape invariant.
//!
//! Scenario from
//! `docs/feature/phase-1-foundation/distill/test-scenarios.md` §2.3:
//!
//! > Given the overdrive-core public API inventory captured for Phase 1
//! > When Ana inspects every exported function signature
//! > Then no exported parameter accepts a bare String or &str identifier
//! >   for which a matching newtype exists
//!
//! # Approach
//!
//! Per the step 01-02 roadmap notes — "keep the test brittle-but-honest" —
//! this is a structural scan of the crate's `src/**/*.rs` with `syn`. We
//! walk every exported function signature (free `pub fn`, inherent `impl`
//! methods on `pub` types, and trait methods on `pub` traits) and flag
//! any parameter whose *name* identifies it as an identifier (e.g.
//! `job_id`, `node_id`, `spiffe_id`, `region`, `correlation_key`, …)
//! that is typed as a bare `&str` / `&mut str` / `String`.
//!
//! # Why not `cargo public-api` or `trybuild`
//!
//! * `cargo public-api` is a CLI tool, not a library; pulling it in as a
//!   dev-dep adds a large dependency graph for a single static scan.
//! * `trybuild` compile-fail tests would need a sentinel file per newtype
//!   — fine in principle but actually *less* comprehensive than walking
//!   every public signature.
//!
//! ADR-0005 already names this file shape (`newtype_static_api.rs` —
//! static scan); this implementation just realises it. ADR-0006 reuses
//! `syn` for the `dst-lint` xtask, so the dependency is paying for
//! multiple invariants.
//!
//! # Invariant-scope
//!
//! The scan is deliberately **NOT** a full static analyser. We look at
//! *parameter names*, not types-mentioned-elsewhere. A raw `&[u8]` used
//! for byte-level storage (`IntentStore::put(&self, key, value)`) is
//! fine — those aren't identifier parameters. The identifier-name list
//! below is the contract surface; adding a new newtype later requires
//! extending the list so the scan catches misuse.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use syn::visit::Visit;
use syn::{FnArg, ImplItem, Item, Pat, Signature, TraitItem, Type, TypeReference, Visibility};

// -----------------------------------------------------------------------------
// The identifier-name contract.
//
// Parameter names that are *semantically* an identifier and therefore
// MUST be typed as a newtype, not a bare string. The scan is case-
// insensitive and matches on exact parameter name.
// -----------------------------------------------------------------------------

const IDENTIFIER_PARAM_NAMES: &[&str] = &[
    // Label-class newtypes
    "job_id",
    "alloc_id",
    "allocation_id",
    "node_id",
    "policy_id",
    "investigation_id",
    "region",
    // Structured newtypes
    "spiffe_id",
    "correlation_key",
    "cert_serial",
    "content_hash",
    "schematic_id",
];

// -----------------------------------------------------------------------------
// Visitor
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct Violation {
    file: PathBuf,
    function: String,
    param: String,
    type_rendered: String,
}

struct Scan {
    file: PathBuf,
    item_path: Vec<String>,
    /// Every violation found. Order is source order; the summary
    /// sorts for stable diagnostics.
    violations: Vec<Violation>,
    /// When we descend into a non-`pub` scope (e.g. `impl` for a private
    /// type, or a module with `pub(crate)` visibility), we still want to
    /// recurse — `syn::visit` does that by default — but we suppress
    /// recording violations. This counter tracks nesting depth.
    private_depth: u32,
}

impl Scan {
    const fn new(file: PathBuf) -> Self {
        Self { file, item_path: Vec::new(), violations: Vec::new(), private_depth: 0 }
    }

    const fn in_public_scope(&self) -> bool {
        self.private_depth == 0
    }

    fn qualified_fn_name(&self, sig: &Signature) -> String {
        let mut parts: Vec<String> = self.item_path.clone();
        parts.push(sig.ident.to_string());
        parts.join("::")
    }

    fn check_signature(&mut self, sig: &Signature) {
        if !self.in_public_scope() {
            return;
        }
        for input in &sig.inputs {
            let FnArg::Typed(pat_type) = input else {
                continue; // skip `self`
            };
            let Pat::Ident(pat_ident) = &*pat_type.pat else {
                continue; // skip destructuring patterns (rare in signatures)
            };
            let name = pat_ident.ident.to_string();
            if !IDENTIFIER_PARAM_NAMES.iter().any(|candidate| candidate.eq_ignore_ascii_case(&name))
            {
                continue;
            }
            if is_bare_string_type(&pat_type.ty) {
                self.violations.push(Violation {
                    file: self.file.clone(),
                    function: self.qualified_fn_name(sig),
                    param: name,
                    type_rendered: render_type(&pat_type.ty),
                });
            }
        }
    }
}

impl<'ast> Visit<'ast> for Scan {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        let pushed_private = !is_public(&node.vis);
        if pushed_private {
            self.private_depth += 1;
        }
        self.item_path.push(node.ident.to_string());
        syn::visit::visit_item_mod(self, node);
        self.item_path.pop();
        if pushed_private {
            self.private_depth -= 1;
        }
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        let pushed_private = !is_public(&node.vis);
        if pushed_private {
            self.private_depth += 1;
        }
        self.check_signature(&node.sig);
        syn::visit::visit_item_fn(self, node);
        if pushed_private {
            self.private_depth -= 1;
        }
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        // For an inherent `impl SomeType { ... }`, the public-or-not-ness
        // of individual methods is decided by the `vis` on each ImplItem.
        // For a trait `impl Trait for T { ... }`, every method is
        // effectively as public as the trait itself; we conservatively
        // treat those as in-scope.
        let label = render_type(&node.self_ty);
        self.item_path.push(format!("<impl {label}>"));
        syn::visit::visit_item_impl(self, node);
        self.item_path.pop();
    }

    fn visit_impl_item(&mut self, node: &'ast ImplItem) {
        if let ImplItem::Fn(f) = node {
            let pushed_private = !is_public(&f.vis);
            if pushed_private {
                self.private_depth += 1;
            }
            self.check_signature(&f.sig);
            syn::visit::visit_impl_item_fn(self, f);
            if pushed_private {
                self.private_depth -= 1;
            }
        } else {
            syn::visit::visit_impl_item(self, node);
        }
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        let pushed_private = !is_public(&node.vis);
        if pushed_private {
            self.private_depth += 1;
        }
        self.item_path.push(node.ident.to_string());
        syn::visit::visit_item_trait(self, node);
        self.item_path.pop();
        if pushed_private {
            self.private_depth -= 1;
        }
    }

    fn visit_trait_item(&mut self, node: &'ast TraitItem) {
        if let TraitItem::Fn(f) = node {
            // Trait methods inherit the trait's visibility — already
            // accounted for at `visit_item_trait` level.
            self.check_signature(&f.sig);
        }
        syn::visit::visit_trait_item(self, node);
    }
}

const fn is_public(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
}

/// Returns `true` iff the type is a bare `&str`, `&mut str`, or `String`.
///
/// Deliberately does NOT flag `&[u8]` (byte-level storage), `String`-
/// wrapping newtypes (they're the whole point), or `Option<String>` —
/// the invariant is about the primary parameter type, and wrapping
/// layers have their own audit surface.
fn is_bare_string_type(ty: &Type) -> bool {
    match ty {
        Type::Reference(TypeReference { elem, .. }) => {
            matches!(&**elem, Type::Path(p) if path_is_single("str", p))
        }
        Type::Path(p) => path_is_single("String", p),
        _ => false,
    }
}

fn path_is_single(name: &str, p: &syn::TypePath) -> bool {
    p.qself.is_none()
        && p.path.segments.len() == 1
        && p.path.segments[0].ident == name
        && matches!(p.path.segments[0].arguments, syn::PathArguments::None)
}

fn render_type(ty: &Type) -> String {
    use quote_compat::render;
    render(ty)
}

/// Minimal type-to-string renderer that does not require `quote`.
/// `syn::Type` exposes tokens via `ToTokens`; we format them manually to
/// avoid pulling in another dev-dep.
mod quote_compat {
    use super::Type;

    pub fn render(ty: &Type) -> String {
        match ty {
            Type::Path(p) => {
                let mut s = String::new();
                for (i, seg) in p.path.segments.iter().enumerate() {
                    if i > 0 {
                        s.push_str("::");
                    }
                    s.push_str(&seg.ident.to_string());
                }
                s
            }
            Type::Reference(r) => {
                let inner = render(&r.elem);
                if r.mutability.is_some() { format!("&mut {inner}") } else { format!("&{inner}") }
            }
            _ => "<?>".into(),
        }
    }
}

// -----------------------------------------------------------------------------
// Walk the crate's `src/**/*.rs`.
// -----------------------------------------------------------------------------

fn src_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` resolves to the crate root at test time.
    let crate_root = std::env::var_os("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR is always set under `cargo test`");
    PathBuf::from(crate_root).join("src")
}

fn collect_rs_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_rs_files_into(root, &mut out);
    out.sort();
    out
}

fn collect_rs_files_into(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("readable dir entry");
        let path = entry.path();
        let file_type = entry.file_type().expect("file type");
        if file_type.is_dir() {
            collect_rs_files_into(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn overdrive_core_public_api_has_no_bare_string_identifier_params() {
    let root = src_root();
    let files = collect_rs_files(&root);
    assert!(!files.is_empty(), "expected at least one .rs file under {root:?}");

    let mut all_violations: Vec<Violation> = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file).expect(&format!("read {file:?}"));
        let parsed = syn::parse_file(&source).expect(&format!("parse {file:?} as rust"));
        let mut scan = Scan::new(file.clone());
        for item in &parsed.items {
            match item {
                Item::Fn(_) | Item::Impl(_) | Item::Trait(_) | Item::Mod(_) => {
                    scan.visit_item(item);
                }
                _ => {}
            }
        }
        all_violations.extend(scan.violations);
    }

    if !all_violations.is_empty() {
        let mut sorted = all_violations;
        sorted.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then_with(|| a.function.cmp(&b.function))
                .then_with(|| a.param.cmp(&b.param))
        });
        let mut report = String::from(
            "overdrive-core public API exposes bare String/&str identifier \
             parameters (see `docs/feature/phase-1-foundation/distill/\
             test-scenarios.md` §2.3):\n",
        );
        for v in &sorted {
            let _ = writeln!(
                report,
                "  {file}: `{function}` — `{param}: {ty}` (expected a newtype)",
                file = v.file.display(),
                function = v.function,
                param = v.param,
                ty = v.type_rendered,
            );
        }
        report.push_str(
            "\nFix: replace the bare string with the matching newtype from \
             `overdrive_core::id`. See `.claude/rules/development.md` \
             §Newtypes.\n",
        );
        panic!("{report}");
    }
}
