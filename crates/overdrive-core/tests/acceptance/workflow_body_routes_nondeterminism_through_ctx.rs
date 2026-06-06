//! Slice 01 / US-WP-1 AC2 — every non-deterministic input flows through
//! `ctx`, never the ambient runtime (D-INH-4; the replay-equivalence
//! precondition).
//!
//! Scenario S-WP-01-03. O5 precondition. A `dst-lint`-style scan over the
//! workflow impl body finds no `Instant::now()` / `reqwest` /
//! `tokio::time::sleep` / `rand::*`; the side effect is performed inside
//! `ctx.run(...).await` only. Negative testing: a body that smuggles a
//! non-`ctx` source is rejected (the failure case is asserted, not just
//! the happy case).
//!
//! # Approach
//!
//! Mirrors the `tests/public_api_shape.rs` precedent (and the dst-lint
//! xtask, ADR-0006): a `syn`-parsed structural scan of the
//! `impl Workflow for ProvisionRecord` `run` body in the sibling scaffold
//! `workflow_trait_drives_to_terminal.rs`. The scan walks every path
//! expression / call / use-statement inside the body and flags any path
//! that resolves to a banned ambient-nondeterminism source:
//!
//! * `Instant::now` / `SystemTime::now` — ambient wall-clock (time MUST
//!   flow through `ctx.clock()`).
//! * `reqwest::*` — ambient network (effects MUST flow through
//!   `ctx.run(...)`, e.g. `ctx.run(name, async { ctx.transport()... })`).
//! * `tokio::time::sleep` — ambient timer (waits MUST flow through
//!   `ctx.sleep(...)`, slice 02).
//! * `rand::*` / `thread_rng` — ambient RNG (randomness MUST flow through
//!   `ctx.entropy()`).
//!
//! The factoring is a pure function over source text, so the same check
//! runs against the clean reference body (positive) and a deliberately-
//! dirty body that smuggles each banned source (negative).

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use syn::visit::Visit;
use syn::{ImplItem, Item};

/// The source file carrying the canonical clean `ProvisionRecord` body.
// Step 01-03 promoted `ProvisionRecord` (struct + `impl Workflow`) into
// the shared `overdrive-core::testing::workflow` fixture so the sim
// journal test can construct it. The canonical clean `async fn run` body
// this D-INH-4 scan reads now lives there, not in the sibling test.
const PROVISION_RECORD_SCAFFOLD: &str = "src/testing/workflow.rs";

/// One banned ambient-nondeterminism source found inside a workflow body.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NondeterminismViolation {
    /// The rendered path that tripped the scan (e.g. `Instant::now`).
    path: String,
    /// The banned root the path matched (e.g. `Instant`, `reqwest`).
    banned_root: &'static str,
}

/// Banned path *roots*. Any path expression / type / use whose leading
/// segment matches one of these is ambient nondeterminism and must not
/// appear in a workflow body.
const BANNED_ROOTS: &[&str] = &["Instant", "SystemTime", "reqwest", "rand"];

/// Banned *qualified* paths — matched on the full segment sequence
/// because the leading segment alone (`tokio`) is too broad (the body may
/// legitimately reference other `tokio` items). `tokio::time::sleep` is
/// the ambient timer that `ctx.sleep` replaces.
const BANNED_QUALIFIED: &[&[&str]] =
    &[&["tokio", "time", "sleep"], &["tokio", "time", "sleep_until"]];

struct NondeterminismScan {
    violations: Vec<NondeterminismViolation>,
}

impl NondeterminismScan {
    const fn new() -> Self {
        Self { violations: Vec::new() }
    }

    fn check_path(&mut self, path: &syn::Path) {
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        if segments.is_empty() {
            return;
        }

        // Banned root: an ambient source appears as ANY segment of the
        // path. Matching on any segment (not just the leading one) catches
        // both the bare `Instant::now()` and the fully-qualified
        // `std::time::Instant::now()` / `rand::thread_rng()` shapes. The
        // banned roots (`Instant`, `SystemTime`, `reqwest`, `rand`) are
        // distinctive identifiers that never legitimately appear in a
        // clean workflow body, so any-segment matching has no false
        // positives here.
        if let Some(banned) = BANNED_ROOTS.iter().find(|b| segments.iter().any(|s| s == *b)) {
            self.violations
                .push(NondeterminismViolation { path: segments.join("::"), banned_root: banned });
            return;
        }

        // Banned qualified path: match the qualified sequence as any
        // contiguous run of segments (handles a leading `::` / crate
        // qualifier before `tokio::time::sleep`).
        for qualified in BANNED_QUALIFIED {
            if segments
                .windows(qualified.len())
                .any(|w| w.iter().zip(*qualified).all(|(a, b)| a == b))
            {
                self.violations.push(NondeterminismViolation {
                    path: segments.join("::"),
                    banned_root: "tokio::time",
                });
                return;
            }
        }
    }
}

impl<'ast> Visit<'ast> for NondeterminismScan {
    // `visit_path` fires for paths in both expression and type position;
    // overriding it alone (not `visit_expr_path`, whose default impl
    // recurses into the same `Path`) avoids double-counting.
    fn visit_path(&mut self, node: &'ast syn::Path) {
        self.check_path(node);
        syn::visit::visit_path(self, node);
    }
}

/// The D-INH-4 check, factored as a pure function over source text so it
/// runs against both the clean reference body (positive) and a
/// deliberately-dirty body (negative). Returns every ambient-
/// nondeterminism violation inside the `ProvisionRecord` `run` body.
fn nondeterminism_violations_in_provision_record(source: &str) -> Vec<NondeterminismViolation> {
    let parsed = syn::parse_file(source).expect("scaffold source parses as Rust");

    let run_body = find_provision_record_run_body(&parsed)
        .expect("scaffold declares `impl Workflow for ProvisionRecord` with an `async fn run`");

    let mut scan = NondeterminismScan::new();
    scan.visit_block(&run_body);
    scan.violations
}

/// Locate the `run` method body inside `impl Workflow for ProvisionRecord`.
fn find_provision_record_run_body(file: &syn::File) -> Option<syn::Block> {
    for item in &file.items {
        let Item::Impl(item_impl) = item else { continue };
        let Some((_, trait_path, _)) = &item_impl.trait_ else { continue };
        let is_workflow = trait_path.segments.last().is_some_and(|seg| seg.ident == "Workflow");
        if !is_workflow {
            continue;
        }
        let syn::Type::Path(self_ty) = &*item_impl.self_ty else { continue };
        let is_provision_record =
            self_ty.path.segments.last().is_some_and(|seg| seg.ident == "ProvisionRecord");
        if !is_provision_record {
            continue;
        }
        for impl_item in &item_impl.items {
            if let ImplItem::Fn(method) = impl_item
                && method.sig.ident == "run"
            {
                return Some(method.block.clone());
            }
        }
    }
    None
}

/// Scans a body for a `ctx.run(...)` method-call expression — the
/// durable-step await-surface every effect routes through.
struct CtxRunScan {
    found: bool,
}

impl<'ast> Visit<'ast> for CtxRunScan {
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if node.method == "run"
            && let syn::Expr::Path(receiver) = &*node.receiver
            && receiver.path.segments.last().is_some_and(|s| s.ident == "ctx")
        {
            self.found = true;
        }
        syn::visit::visit_expr_method_call(self, node);
    }
}

/// Confirm the clean body's side effect routes through `ctx.run`. The
/// positive contract is two-sided: (a) no banned ambient source, AND
/// (b) the effect is actually performed inside a `ctx.run(...).await`
/// durable step.
fn body_calls_ctx_run(source: &str) -> bool {
    let parsed = syn::parse_file(source).expect("scaffold source parses as Rust");
    let run_body = find_provision_record_run_body(&parsed)
        .expect("scaffold declares `impl Workflow for ProvisionRecord` with an `async fn run`");

    let mut scan = CtxRunScan { found: false };
    scan.visit_block(&run_body);
    scan.found
}

fn read_scaffold_source() -> String {
    let crate_root = std::env::var_os("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR is always set under `cargo test`");
    let path = std::path::PathBuf::from(crate_root).join(PROVISION_RECORD_SCAFFOLD);
    std::fs::read_to_string(&path).expect(&format!("read {}", path.display()))
}

#[test]
fn clean_workflow_body_routes_all_nondeterminism_through_ctx() {
    let source = read_scaffold_source();

    // (a) No ambient nondeterminism source anywhere in the body.
    let violations = nondeterminism_violations_in_provision_record(&source);
    assert!(
        violations.is_empty(),
        "D-INH-4 (S-WP-01-03 / O5): the ProvisionRecord workflow body must route ALL \
         non-determinism through `ctx` — found {} ambient source(s) the scan rejects: \
         {violations:#?}\n\
         Time → `ctx.clock()`, network → `ctx.run(...)`, waits → `ctx.sleep(...)`, \
         randomness → `ctx.entropy()`.",
        violations.len(),
    );

    // (b) The effect is genuinely performed inside `ctx.run(...).await`,
    // not merely absent of banned sources.
    assert!(
        body_calls_ctx_run(&source),
        "D-INH-4 (S-WP-01-03): the ProvisionRecord body must perform its side effect \
         inside a `ctx.run(...).await` durable step — the clean body's only \
         nondeterminism surface.",
    );
}

#[test]
fn workflow_body_smuggling_non_ctx_nondeterminism_is_rejected() {
    // Negative: a deliberately-dirty workflow body that smuggles EACH
    // banned ambient source outside `ctx`. The scan MUST flag every one —
    // a check that only ever passes on the clean body is theatre. This
    // proves the D-INH-4 metric fails closed.
    let dirty = r#"
        use async_trait::async_trait;
        #[async_trait]
        impl Workflow for ProvisionRecord {
            async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult {
                let _now = std::time::Instant::now();
                let _sys = std::time::SystemTime::now();
                let _roll = rand::random::<u64>();
                let _rng = rand::thread_rng();
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                let _ = reqwest::get("http://example.com").await;
                WorkflowResult::Success
            }
        }
    "#;

    let violations = nondeterminism_violations_in_provision_record(dirty);

    // Every banned root must be detected — assert the rejection is
    // comprehensive, not just non-empty.
    let detected_roots: std::collections::HashSet<&'static str> =
        violations.iter().map(|v| v.banned_root).collect();

    for expected in ["Instant", "SystemTime", "rand", "reqwest", "tokio::time"] {
        assert!(
            detected_roots.contains(expected),
            "the D-INH-4 check must reject the smuggled `{expected}` ambient source, \
             but it was not flagged. Detected: {detected_roots:?}; all violations: \
             {violations:#?}",
        );
    }
}
