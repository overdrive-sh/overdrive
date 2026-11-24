//! A VM's cgroup scope has one remover: the populated-first `kill_scope`
//! (RB-5, N-1; ADR-0351; feature delta § "A VM scope has one remover", item 5).
//!
//! # Scenario family
//!
//! The lease gate of ADR-0156 treats "scope absent" as "every descriptor of the
//! VMM is closed". That holds only if no code removes a scope while its tasks
//! are still exiting. `VmHostState::kill_scope` waits for `populated 0` before
//! it removes the directory. A second remover, such as the VM driver's bare
//! `CgroupManager::remove_workload_scope`, defeats the gate. This test is the
//! gate's structural defence: it scans the production `src/` of every workspace
//! crate and names every call that removes a cgroup directory.
//!
//! # What counts as a remover
//!
//! * a method call `.remove_dir(..)` (the `CgroupFs` port method), anywhere;
//! * a call of a function named `remove_dir` or `remove_dir_all` whose argument
//!   expression mentions `scope` or `cgroup` (a path, a binding or a string), or
//!   that sits inside a function whose name does.
//!
//! The port method's definition and the adapters' `remove_dir` bodies are not
//! calls. Test code (`#[cfg(test)]` modules, `#[test]` functions, `tests/`) is
//! not production.
//!
//! # Allowed callers (decided under the design's authority, ADR-0351)
//!
//! `RealVmHostState::kill_scope` (`overdrive-host/src/vm_host_state.rs`) and the
//! `CgroupFs` adapters' own startup probes, which remove a probe directory they
//! created, never a VM scope: `overdrive-host/src/cgroup_fs.rs` and
//! `overdrive-sim/src/adapters/cgroup_fs.rs`.
//!
//! # Tier and placement
//!
//! Tier 1 layer 1, default lane, `host_pure` (a read of source files). An AST
//! scan with `syn`, the precedent of `workflow_body_has_no_step_machine.rs`.
//! PENDING (RED): today `CgroupManager::remove_workload_scope` removes a scope
//! with a bare `rmdir`.

#![allow(clippy::expect_used, clippy::panic, clippy::doc_markdown)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use syn::visit::Visit;
use syn::{Attribute, Expr, ExprCall, ExprMethodCall, ImplItemFn, ItemFn, ItemMod};

/// The files whose removals are the allowed ones, relative to `crates/`.
const ALLOWED_FILES: [&str; 3] = [
    "overdrive-host/src/vm_host_state.rs",
    "overdrive-host/src/cgroup_fs.rs",
    "overdrive-sim/src/adapters/cgroup_fs.rs",
];

/// One call that removes a cgroup directory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Remover {
    file: String,
    call: String,
}

fn is_test_attr(attr: &Attribute) -> bool {
    let path = attr.path();
    if path.is_ident("test") {
        return true;
    }
    if path.is_ident("cfg") {
        let mut found = false;
        let _parsed = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("test") {
                found = true;
            }
            Ok(())
        });
        return found;
    }
    false
}

/// Collects the identifiers and string literals an expression mentions.
#[derive(Default)]
struct Mentions(Vec<String>);

impl<'ast> Visit<'ast> for Mentions {
    fn visit_ident(&mut self, ident: &'ast syn::Ident) {
        self.0.push(ident.to_string().to_lowercase());
    }

    fn visit_lit_str(&mut self, lit: &'ast syn::LitStr) {
        self.0.push(lit.value().to_lowercase());
    }
}

fn mentions_a_scope(args: &syn::punctuated::Punctuated<Expr, syn::token::Comma>) -> bool {
    let mut mentions = Mentions::default();
    for arg in args {
        mentions.visit_expr(arg);
    }
    mentions.0.iter().any(|word| word.contains("scope") || word.contains("cgroup"))
}

struct Scan {
    file: String,
    /// The names of the functions the visitor is inside, innermost last. A
    /// removal inside a function named for a scope is a scope removal even
    /// when its argument is an anonymous `path`.
    functions: Vec<String>,
    found: BTreeSet<Remover>,
}

impl Scan {
    fn inside_a_scope_function(&self) -> bool {
        self.functions.iter().any(|name| name.contains("scope") || name.contains("cgroup"))
    }
}

impl<'ast> Visit<'ast> for Scan {
    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        if module.attrs.iter().any(is_test_attr) {
            return;
        }
        syn::visit::visit_item_mod(self, module);
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        if function.attrs.iter().any(is_test_attr) {
            return;
        }
        self.functions.push(function.sig.ident.to_string().to_lowercase());
        syn::visit::visit_item_fn(self, function);
        self.functions.pop();
    }

    fn visit_impl_item_fn(&mut self, function: &'ast ImplItemFn) {
        if function.attrs.iter().any(is_test_attr) {
            return;
        }
        self.functions.push(function.sig.ident.to_string().to_lowercase());
        syn::visit::visit_impl_item_fn(self, function);
        self.functions.pop();
    }

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        if call.method == "remove_dir" {
            self.found
                .insert(Remover { file: self.file.clone(), call: "`.remove_dir(..)`".to_owned() });
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(path) = call.func.as_ref()
            && let Some(last) = path.path.segments.last()
        {
            let name = last.ident.to_string();
            if (name == "remove_dir" || name == "remove_dir_all")
                && (mentions_a_scope(&call.args) || self.inside_a_scope_function())
            {
                self.found
                    .insert(Remover { file: self.file.clone(), call: format!("`{name}(..)`") });
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}

fn scan_source(file: &str, source: &str) -> BTreeSet<Remover> {
    let parsed = syn::parse_file(source).unwrap_or_else(|error| panic!("parse {file}: {error}"));
    let mut scan = Scan { file: file.to_owned(), functions: Vec::new(), found: BTreeSet::new() };
    scan.visit_file(&parsed);
    scan.found
}

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crates dir").to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Every cgroup-directory remover of every crate's production `src/`.
fn workspace_removers() -> BTreeSet<Remover> {
    let crates = crates_dir();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&crates).expect("read crates").flatten() {
        let src = entry.path().join("src");
        if src.is_dir() {
            rust_files(&src, &mut files);
        }
    }
    let mut found = BTreeSet::new();
    for file in files {
        let relative =
            file.strip_prefix(&crates).expect("under crates").to_string_lossy().replace('\\', "/");
        let source = std::fs::read_to_string(&file).expect("read source");
        found.extend(scan_source(&relative, &source));
    }
    found
}

/// PENDING (RB-5, ADR-0351). No production caller outside the allow-list
/// removes a cgroup directory. Today `CgroupManager::remove_workload_scope`
/// does, with a bare `rmdir` that removes a scope under a live exiting task.
///
/// CONTRACT_SHAPE: pure-function. Universe: the set of removers in the
/// workspace's production source.
#[test]
fn a_vm_scope_is_removed_only_by_the_populated_first_scope_kill() {
    let offenders: Vec<Remover> = workspace_removers()
        .into_iter()
        .filter(|remover| !ALLOWED_FILES.contains(&remover.file.as_str()))
        .collect();

    assert!(
        offenders.is_empty(),
        "a second remover of a VM scope exists; only `VmHostState::kill_scope` may remove one: {offenders:#?}"
    );
}

/// GUARD (non-vacuity). The scan sees the one allowed remover, so an empty
/// offender list means "none", not "the scan found nothing".
#[test]
fn guard_the_scan_sees_the_one_allowed_remover() {
    let removers = workspace_removers();

    assert!(
        removers.iter().any(|remover| remover.file == "overdrive-host/src/vm_host_state.rs"),
        "the scan must see `kill_scope`'s own removal, got {removers:#?}"
    );
}

/// GUARD (the scanner's own cases). A scope removal is flagged, a run-directory
/// removal and test code are not.
#[test]
fn guard_the_scan_flags_a_scope_removal_and_ignores_a_run_dir_removal_and_test_code() {
    let source = r"
        async fn bad(fs: &dyn Fs, dir: &Path) { fs.remove_dir(dir).await; }
        async fn also_bad(scope_path: PathBuf) { let _ = tokio::fs::remove_dir(&scope_path).await; }
        async fn fine(run_dir: PathBuf) { let _ = tokio::fs::remove_dir_all(&run_dir).await; }
        #[cfg(test)]
        mod tests { fn t(fs: &dyn Fs, d: &Path) { fs.remove_dir(d); } }
        #[test]
        fn single() { std::fs::remove_dir(&cgroup_dir).unwrap(); }
    ";

    let found = scan_source("sample.rs", source);

    assert_eq!(found.len(), 2, "the two scope removals, nothing else: {found:#?}");
}
