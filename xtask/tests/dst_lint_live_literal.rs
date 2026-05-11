//! Self-test for the `live-literal-banned` rule introduced in
//! `workload-kind-discriminator` Slice 01.
//!
//! The rule flags any source file under `crates/overdrive-cli/src/render.rs`
//! or `crates/overdrive-cli/src/commands/` that contains the literal
//! `"live"` outside comments / docstrings / `#[cfg(test)]` modules.
//!
//! Scenarios from
//! `docs/feature/workload-kind-discriminator/distill/test-scenarios.md`
//! §6 (S-06-01 .. S-06-03).
//!
//! Gated behind the `integration-tests` feature — same convention as
//! `dst_lint_self_test.rs`.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]
// Diagnostic prints in S-06-03 surface the offending file/line/column so
// a regression is debuggable from CI logs alone — the test is the
// canonical place where eprintln! is the right shape.
#![allow(clippy::print_stderr)]

use xtask::dst_lint::scan_source_live_literal;

// ---------------------------------------------------------------------------
// S-06-01 — dst-lint rejects "live" literal in render or command source
// ---------------------------------------------------------------------------

#[test]
fn s_06_01_dst_lint_rejects_live_literal() {
    // Synthetic source that pretends to be a render-path file with a
    // `"live"` literal in code (NOT in a comment).
    let source = r#"
fn render_duration(took: Duration) -> String {
    format!("(took {})", "live")
}
"#;
    let violations = scan_source_live_literal(source, "crates/overdrive-cli/src/render.rs")
        .expect("scan_source_live_literal must succeed for valid input");
    assert!(
        !violations.is_empty(),
        "live-literal-banned must flag a `\"live\"` string literal in render code"
    );

    // The violation must name the rule label.
    let v = &violations[0];
    assert_eq!(v.banned_path, "\"live\"");
    assert!(
        v.replacement_trait.contains("live-literal-banned")
            || v.replacement_trait.contains("measured duration"),
        "violation must reference the rule label or its remediation; got: {}",
        v.replacement_trait
    );
    // Line / column should pin the literal location.
    assert!(v.line > 0, "line must be 1-based positive; got {}", v.line);
    assert!(v.column > 0, "column must be 1-based positive; got {}", v.column);
}

// ---------------------------------------------------------------------------
// S-06-02 — dst-lint allows "live" in comments and docstrings
// ---------------------------------------------------------------------------

#[test]
fn s_06_02_dst_lint_allows_live_in_comments() {
    // The literal `"live"` in a `//` comment must NOT trip the rule.
    let source_line_comment = r#"
// historical: the literal "live" used to be here
fn render_duration(_took: Duration) -> String {
    String::from("(took 12ms)")
}
"#;
    let violations =
        scan_source_live_literal(source_line_comment, "crates/overdrive-cli/src/render.rs")
            .expect("scan_source_live_literal must succeed");
    assert!(
        violations.is_empty(),
        "`\"live\"` inside a // comment must NOT be flagged; got: {violations:?}"
    );

    // Same for `///` doc comments.
    let source_doc_comment = r#"
/// Renders a duration. Historically the literal "live" was used here.
fn render_duration(_took: Duration) -> String {
    String::from("(took 12ms)")
}
"#;
    let violations =
        scan_source_live_literal(source_doc_comment, "crates/overdrive-cli/src/render.rs")
            .expect("scan_source_live_literal must succeed");
    assert!(
        violations.is_empty(),
        "`\"live\"` inside a /// doc comment must NOT be flagged; got: {violations:?}"
    );

    // And in a `/* … */` block comment.
    let source_block_comment = r#"
/* historical: "live" used to be a sentinel here */
fn render_duration(_took: Duration) -> String {
    String::from("(took 12ms)")
}
"#;
    let violations =
        scan_source_live_literal(source_block_comment, "crates/overdrive-cli/src/render.rs")
            .expect("scan_source_live_literal must succeed");
    assert!(
        violations.is_empty(),
        "`\"live\"` inside a /* */ block comment must NOT be flagged; got: {violations:?}"
    );
}

// ---------------------------------------------------------------------------
// S-06-03 — dst-lint passes on the migrated codebase
// ---------------------------------------------------------------------------

#[test]
fn s_06_03_dst_lint_passes_on_migrated_codebase() {
    // Walk the real `crates/overdrive-cli/src/render.rs` and
    // `crates/overdrive-cli/src/commands/**/*.rs` and assert zero
    // violations. This is the K1 regression-guard scenario.
    let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root =
        crate_dir.parent().expect("xtask crate lives directly under workspace root");
    let render = workspace_root.join("crates/overdrive-cli/src/render.rs");
    let commands_dir = workspace_root.join("crates/overdrive-cli/src/commands");

    let mut total_violations = 0usize;
    let mut targets: Vec<std::path::PathBuf> = Vec::new();
    if render.exists() {
        targets.push(render);
    }
    if commands_dir.exists() {
        collect_rs_files(&commands_dir, &mut targets);
    }
    for target in &targets {
        let source = std::fs::read_to_string(target)
            .expect(&format!("must be able to read render-path source at {}", target.display()));
        let violations = scan_source_live_literal(&source, target)
            .expect(&format!("scan_source_live_literal must succeed for {}", target.display()));
        if !violations.is_empty() {
            eprintln!(
                "S-06-03: {} live-literal violation(s) in {}:",
                violations.len(),
                target.display()
            );
            for v in &violations {
                eprintln!("  line {}, col {}: {}", v.line, v.column, v.banned_path);
            }
        }
        total_violations += violations.len();
    }
    assert_eq!(
        total_violations, 0,
        "K1 regression guard: zero `\"live\"` literals must remain in CLI render/command source"
    );
}

fn collect_rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}
