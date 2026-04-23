//! Self-test: every entry in `BANNED_APIS` is detected by the scanner.
//!
//! The scanner is the tool; its responsibility is to catch every symbol
//! in its own constant. This test synthesises a one-liner source file
//! per banned entry, feeds it to the in-process scanner, and asserts a
//! violation is reported. If a new `BANNED_APIS` entry is added without
//! corresponding detection logic, this test fires regardless of whether
//! any real core crate exercises the symbol.
//!
//! This is a Tier 1 self-test per the roadmap — lives at the driving
//! port but uses the in-process visitor API rather than a subprocess
//! boundary, because the purpose is to close the loop on the symbol
//! table, not to exercise the CLI.
//!
//! Gated behind the `integration-tests` feature — see the feature
//! comment in `xtask/Cargo.toml` for rationale.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]

use xtask::dst_lint::{BANNED_APIS, Violation, scan_source};

#[test]
fn every_banned_api_is_detected_by_the_scanner() {
    // For each entry in BANNED_APIS, construct a minimal source file
    // that exercises the banned path and verify the scanner flags it.
    for (path, replacement) in BANNED_APIS {
        let source = synthesize_call(path);
        let violations = scan_source(&source, "synthetic.rs").unwrap_or_else(|e| {
            panic!("scan_source must succeed for synthetic input {source:?}: {e}")
        });

        assert!(
            !violations.is_empty(),
            "BANNED_APIS entry {path:?} must be detected; synthetic source:\n{source}"
        );

        // And the detection must name the replacement trait.
        let any_matches = violations.iter().any(|v| v.replacement_trait == *replacement);
        assert!(
            any_matches,
            "violation for {path:?} must reference replacement trait {replacement:?}; got {violations:?}"
        );
    }
}

/// `scan_source` reports 1-based `(line, column)` for the head segment of
/// the matched path. A core crate whose file starts with `pub fn f() {
/// let _ = std::time::Instant::now(); }` must pin the violation to line
/// 1 and column 21 (the character position of `std` in that literal
/// source). This also kills the "replace + with -" and "replace + with *"
/// mutants on the `column + 1` conversion.
#[test]
fn violation_reports_one_based_line_and_column_of_head_segment() {
    // 20-column prefix puts `std` at column 21 (1-based).
    let source = "pub fn f() { let _ = std::time::Instant::now(); }\n";
    let violations =
        scan_source(source, "fixture.rs").expect("scan_source must succeed for valid input");
    assert_eq!(violations.len(), 1, "exactly one violation expected, got {violations:?}");
    let v: &Violation = &violations[0];
    assert_eq!(v.line, 1, "head segment is on line 1");
    assert_eq!(v.column, 22, "`std` starts at 1-based column 22 in the literal source");
    assert_eq!(v.banned_path, "std::time::Instant::now");
    assert_eq!(v.replacement_trait, "Clock");
}

/// `visit_type_path` (types in signatures, not expressions) must report
/// the same 1-based column convention as `visit_expr_path`. This kills
/// the "replace + with -" and "replace + with *" mutants at the
/// `column + 1` conversion in the `TypePath` arm.
#[test]
fn type_path_violation_reports_one_based_column() {
    // Place `tokio` at column 17 (1-based) in the literal source.
    let source = "pub type Sock = tokio::net::TcpStream;\n";
    let violations = scan_source(source, "fixture.rs").expect("scan_source must succeed");
    assert_eq!(violations.len(), 1, "exactly one TypePath violation expected");
    assert_eq!(violations[0].column, 17, "`tokio` starts at 1-based column 17");
    assert_eq!(violations[0].replacement_trait, "Transport");
}

/// Detection must key on the full head-to-tail path, not just the leaf.
/// A `now` symbol that is not `Instant::now` must NOT fire the banned-API
/// lint — otherwise we flag every `fn now()` defined in a core crate.
/// This kills the `banned_head -> xyzzy` and `banned_tail -> String::new`
/// family of mutants: if those helpers return nonsense, the exact-match
/// check still catches `std::time::Instant::now` but the plain `now()`
/// here would also be flagged.
#[test]
fn unrelated_leaf_named_now_is_not_flagged() {
    let source = "pub fn now() -> u32 { 0 }\npub fn call_own() { let _ = now(); }\n";
    let violations = scan_source(source, "fixture.rs").expect("scan_source must succeed");
    assert!(
        violations.is_empty(),
        "a locally-defined fn named `now` must not match the banned table; got {violations:?}"
    );
}

/// A source path that is *strictly longer* than the banned path — more
/// head segments than the banned entry has — must not match. This kills
/// the `len() > len()` vs `len() >= len()` mutant inside `path_matches`:
/// with `>=`, a 4-segment source against a 3-segment banned entry would
/// early-exit as "no match" before comparing, and the suffix check we
/// added would silently under-flag the inverse case.
#[test]
fn source_longer_than_banned_never_matches() {
    // 5 segments; every banned path is at most 4 (`std::time::Instant::now`).
    let source = "pub fn f() { let _ = a::b::c::d::e(); }\n";
    let violations =
        scan_source(source, "fixture.rs").expect("scan_source must succeed for valid input");
    assert!(
        violations.is_empty(),
        "5-segment unrelated path must not match any banned entry; got {violations:?}"
    );
}

/// Build a minimal Rust source file that exercises `path`.
///
/// The scanner matches on the suffix / segment structure of the path,
/// so a faithful reference to the symbol is sufficient — no async or
/// type-resolution machinery required.
fn synthesize_call(path: &str) -> String {
    // For types (TcpStream, TcpListener, UdpSocket) a `type` alias
    // references the symbol in a path position the visitor will walk.
    // For free functions a plain expression call does the same.
    if path.ends_with("TcpStream") || path.ends_with("TcpListener") || path.ends_with("UdpSocket") {
        format!("pub type Alias = {path};\n")
    } else if path.ends_with("sleep") {
        // sleep takes a Duration; the scanner doesn't evaluate the
        // expression, so a stubbed argument is fine.
        format!("pub fn f() {{ {path}(std::time::Duration::from_millis(1)); }}\n")
    } else if path.ends_with("thread_rng") {
        format!("pub fn f() {{ let _ = {path}(); }}\n")
    } else {
        // Instant::now, SystemTime::now, rand::random.
        format!("pub fn f() {{ let _ = {path}(); }}\n")
    }
}
