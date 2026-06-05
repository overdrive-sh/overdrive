//! Slice 01 / US-WP-1 AC1 (K6 metric) — a durable sequence body
//! contains zero step-machine boilerplate (the O3 structural promise,
//! mechanically asserted per Eclipse H1 / L1, NOT free-hand review).
//!
//! Scenario S-WP-01-02. K6 / O3. An AST check (`syn`) over the
//! `ProvisionRecord` workflow impl body counts step-enum declarations and
//! state-transition `match` arms and asserts the count is zero.
//!
//! # Approach
//!
//! Mirrors the established `tests/public_api_shape.rs` precedent: a
//! `syn`-parsed structural scan, not a free-hand textual grep. The
//! reference body is the `impl Workflow for ProvisionRecord` block in the
//! sibling scaffold `workflow_trait_drives_to_terminal.rs` (the clean
//! author surface from 01-01). The K6 metric is mechanically defined as:
//!
//! * **step-enum declaration** — an `enum` item declared *inside* the
//!   `run` method body. A workflow that hand-rolls a step cursor declares
//!   a `Step`/`State`/`Phase` enum locally; the clean body declares none.
//! * **state-transition `match` arm** — a `match` whose scrutinee is a
//!   locally-declared step-enum value (i.e. a `match` driving the
//!   hand-rolled cursor). The clean body's only `match` is over the
//!   `Result` returned by `ctx.run(...).await`, which is NOT a
//!   step-transition — it is ordinary control flow over a port result.
//!
//! Counting *every* `match` would be wrong (the clean body legitimately
//! matches a `Result`); counting matches *on a locally-declared step
//! enum* is the honest K6 metric.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::collections::HashSet;

use syn::visit::Visit;
use syn::{Expr, ImplItem, Item, Stmt};

/// The source file carrying the canonical clean `ProvisionRecord` body.
// Step 01-03 promoted `ProvisionRecord` (struct + `impl Workflow`) into
// the shared `overdrive-core::testing::workflow` fixture so the sim
// journal test can construct it. The canonical clean `async fn run` body
// this K6 scan reads now lives there, not in the sibling acceptance test.
const PROVISION_RECORD_SCAFFOLD: &str = "src/testing/workflow.rs";

/// One mechanically-detected unit of step-machine boilerplate.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StepMachineFinding {
    /// An `enum` declared inside the workflow body — a hand-rolled step
    /// cursor.
    StepEnumDeclaration { name: String },
    /// A `match` whose scrutinee is a locally-declared step enum — a
    /// hand-rolled state-transition dispatch.
    StateTransitionMatch { on_enum: String },
}

/// Walks a `Workflow::run` body, recording every step-enum declaration
/// and every `match` driven by such an enum.
struct BodyScan {
    /// Names of `enum`s declared locally inside the body.
    local_enums: HashSet<String>,
    /// Local bindings (`let x = <step-enum-ctor>`) whose value is a
    /// locally-declared step enum — the scrutinees a transition `match`
    /// would dispatch on.
    step_bindings: HashSet<String>,
    findings: Vec<StepMachineFinding>,
}

impl BodyScan {
    fn new() -> Self {
        Self { local_enums: HashSet::new(), step_bindings: HashSet::new(), findings: Vec::new() }
    }

    /// First pass — collect locally-declared enum names from a block's
    /// statements so the `match`-scrutinee pass can recognise them.
    fn collect_local_enums(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            if let Stmt::Item(Item::Enum(item_enum)) = stmt {
                let name = item_enum.ident.to_string();
                self.local_enums.insert(name.clone());
                self.findings.push(StepMachineFinding::StepEnumDeclaration { name });
            }
        }
    }

    /// Resolve whether a `match` scrutinee names a locally-declared step
    /// enum — either a path expression into the enum
    /// (`Step::Foo` / `Step`) or a binding initialised from one.
    fn scrutinee_step_enum(&self, scrutinee: &Expr) -> Option<String> {
        match scrutinee {
            // `match Step::Foo { ... }` / `match step_var { ... }`
            Expr::Path(path) => {
                let segments: Vec<String> =
                    path.path.segments.iter().map(|s| s.ident.to_string()).collect();
                // A path whose leading segment is a local enum is a
                // transition match (`Step::Foo`).
                if let Some(first) = segments.first() {
                    if self.local_enums.contains(first) {
                        return Some(first.clone());
                    }
                    // A bare binding previously bound to a step enum.
                    if segments.len() == 1 && self.step_bindings.contains(first) {
                        return Some(first.clone());
                    }
                }
                None
            }
            _ => None,
        }
    }
}

impl<'ast> Visit<'ast> for BodyScan {
    fn visit_block(&mut self, block: &'ast syn::Block) {
        // Collect any enums declared at this block level before recursing
        // so nested matches can see them.
        self.collect_local_enums(&block.stmts);
        // Track `let <name> = <local-enum-ctor>;` bindings.
        for stmt in &block.stmts {
            if let Stmt::Local(local) = stmt {
                if let syn::Pat::Ident(pat_ident) = &local.pat {
                    if let Some(init) = &local.init {
                        if let Some(enum_name) = self.scrutinee_step_enum(&init.expr) {
                            let _ = enum_name;
                            self.step_bindings.insert(pat_ident.ident.to_string());
                        }
                    }
                }
            }
        }
        syn::visit::visit_block(self, block);
    }

    fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
        if let Some(on_enum) = self.scrutinee_step_enum(&node.expr) {
            self.findings.push(StepMachineFinding::StateTransitionMatch { on_enum });
        }
        syn::visit::visit_expr_match(self, node);
    }
}

/// The K6 check, factored as a pure function over source text so it can
/// be exercised against both the clean reference body (positive) and a
/// deliberately-dirty body (negative, in the unit test below). Returns
/// every step-machine finding inside the `ProvisionRecord` `run` body.
fn step_machine_findings_in_provision_record(source: &str) -> Vec<StepMachineFinding> {
    let parsed = syn::parse_file(source).expect("scaffold source parses as Rust");

    let run_body = find_provision_record_run_body(&parsed)
        .expect("scaffold declares `impl Workflow for ProvisionRecord` with an `async fn run`");

    let mut scan = BodyScan::new();
    scan.visit_block(&run_body);
    scan.findings
}

/// Locate the `run` method body inside `impl Workflow for ProvisionRecord`.
fn find_provision_record_run_body(file: &syn::File) -> Option<syn::Block> {
    for item in &file.items {
        let Item::Impl(item_impl) = item else { continue };
        // Must be a trait impl `impl Workflow for <T>`.
        let Some((_, trait_path, _)) = &item_impl.trait_ else { continue };
        let is_workflow = trait_path.segments.last().is_some_and(|seg| seg.ident == "Workflow");
        if !is_workflow {
            continue;
        }
        // Must be for `ProvisionRecord`.
        let syn::Type::Path(self_ty) = &*item_impl.self_ty else { continue };
        let is_provision_record =
            self_ty.path.segments.last().is_some_and(|seg| seg.ident == "ProvisionRecord");
        if !is_provision_record {
            continue;
        }
        for impl_item in &item_impl.items {
            if let ImplItem::Fn(method) = impl_item {
                if method.sig.ident == "run" {
                    return Some(method.block.clone());
                }
            }
        }
    }
    None
}

fn read_scaffold_source() -> String {
    let crate_root = std::env::var_os("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR is always set under `cargo test`");
    let path = std::path::PathBuf::from(crate_root).join(PROVISION_RECORD_SCAFFOLD);
    std::fs::read_to_string(&path).expect(&format!("read {}", path.display()))
}

#[test]
fn provision_record_body_has_zero_step_enum_and_zero_transition_match() {
    let source = read_scaffold_source();
    let findings = step_machine_findings_in_provision_record(&source);

    assert!(
        findings.is_empty(),
        "K6 metric (S-WP-01-02 / O3): the ProvisionRecord workflow body must contain \
         zero step-enum declarations and zero state-transition match arms, but the \
         AST scan found {} step-machine construct(s): {findings:#?}\n\
         A clean durable sequence is one ordinary `async fn run` — no hand-rolled \
         step cursor, no transition dispatch.",
        findings.len(),
    );
}

#[test]
fn step_machine_check_rejects_a_hand_rolled_step_cursor_body() {
    // Negative: a deliberately-dirty workflow body that hand-rolls a step
    // enum AND a transition match. The K6 check MUST flag both — a check
    // that can only ever pass is theatre. This proves the metric has
    // teeth.
    let dirty = r"
        use async_trait::async_trait;
        #[async_trait]
        impl Workflow for ProvisionRecord {
            async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult {
                enum Step {
                    Provision,
                    Done,
                }
                let mut cursor = Step::Provision;
                loop {
                    match cursor {
                        Step::Provision => {
                            cursor = Step::Done;
                        }
                        Step::Done => {
                            return WorkflowResult::Success;
                        }
                    }
                }
            }
        }
    ";

    let findings = step_machine_findings_in_provision_record(dirty);

    assert!(
        findings.iter().any(|f| matches!(f, StepMachineFinding::StepEnumDeclaration { .. })),
        "the K6 check must detect a hand-rolled step-enum declaration, found: {findings:#?}",
    );
    assert!(
        findings.iter().any(|f| matches!(f, StepMachineFinding::StateTransitionMatch { .. })),
        "the K6 check must detect a state-transition match on the step enum, found: {findings:#?}",
    );
}
