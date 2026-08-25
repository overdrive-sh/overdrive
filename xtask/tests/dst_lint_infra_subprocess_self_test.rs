//! Self-test for the `ban-infra-subprocess` lint — the FINAL slice of
//! `subprocess-free-veth-provisioner` (GH #233, ADR-0085 D8 / DDD-10).
//!
//! Slice 05 lands a new `xtask::dst_lint::scan_source_infra_subprocess`
//! scanner clause that bans `Command::new("<tool>")` for the seven named
//! infra CLIs (`ip`, `nft`, `ethtool`, `sysctl`, `tc`, `bpftool`,
//! `iptables`) in production `src/**` of runtime crates. This file is its
//! in-process self-test, mirroring `dst_lint_self_test.rs` (`BANNED_APIS`
//! loop) and `dst_lint_live_literal.rs` (scoped literal ban + marker
//! suppression + migrated-tree door-lock), per ADR-0085 D8:
//!
//!   - **Scope:** crates whose `crate_class ∈ {core, adapter-host}`, MINUS
//!     an explicit exclusion of `overdrive-testing` (dev-dep-only Tier-3
//!     fixture that legitimately shells `ip netns add`). `binary`
//!     (`overdrive-cli`, `xtask`) and `adapter-sim` (`overdrive-sim`) are
//!     out of scope by class.
//!   - **What it bans:** the seven NAMED string-literal args to
//!     `Command::new`, NOT `Command::new` generically. Variable-binary
//!     spawns (`Command::new(var)`) and `run_ip()`-style indirection are
//!     out of the literal-only guarantee.
//!   - **Marker:** `// subprocess-ok: <reason>` on the use-site line or the
//!     line immediately above (mirrors `// dst-lint: hashmap-ok`).
//!   - **Exempt:** `#[cfg(test)]` items and `bin/` tooling.
//!
//! GREEN transition (slice 05): each `#[should_panic("RED scaffold")]`
//! attribute is dropped and the `panic!` body replaced with a call to
//! `scan_source_infra_subprocess` (per-source, S-LINT-01..04) or
//! `scan_infra_subprocess_from_manifest` (door-lock, S-LINT-05) over the
//! synthetic source quoted in each test's doc comment, asserting the shape
//! described there. `Violation` carries `{ file, line, column, banned_path,
//! replacement_trait, kind }` — `banned_path` names the flagged `"<tool>"`
//! literal, `line`/`column` are 1-based positive.
//!
//! Gated behind the `integration-tests` feature — same convention as
//! `dst_lint_self_test.rs` / `dst_lint_live_literal.rs`. In-process scanner
//! self-test (the scanner is pure syn AST; the door-lock reads cargo
//! metadata to enumerate the in-scope crates).

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]
// Diagnostic prints in S-LINT-05 surface the offending file/line/column so
// a regression is debuggable from CI logs alone.
#![allow(clippy::print_stderr)]

use xtask::dst_lint::{
    BANNED_INFRA_CLIS, Violation, scan_infra_subprocess_from_manifest, scan_source_infra_subprocess,
};

/// A scoped production `adapter-host` path used across the flag/suppress
/// tests — NOT excluded by scope, so a literal here IS eligible to flag.
const SCOPED_SRC: &str = "crates/overdrive-worker/src/mtls_intercept.rs";

// ---------------------------------------------------------------------------
// S-LINT-01 — a named infra-CLI string literal in a scoped production src
// path is FLAGGED. Loop over all seven banned tool names, exactly as
// `dst_lint_self_test` loops over BANNED_APIS.
// ---------------------------------------------------------------------------

#[test]
fn s_lint_01_named_infra_cli_literal_in_scoped_src_is_flagged() {
    for tool in BANNED_INFRA_CLIS {
        let source = format!(
            "pub fn ensure() {{ let _ = std::process::Command::new(\"{tool}\").arg(\"rule\").output(); }}\n"
        );
        let violations = scan_source_infra_subprocess(&source, SCOPED_SRC)
            .expect(&format!("scan must succeed for synthetic source of {tool:?}"));
        assert_eq!(
            violations.len(),
            1,
            "exactly one violation expected for Command::new({tool:?}); got {violations:?}"
        );
        let v: &Violation = &violations[0];
        assert!(
            v.banned_path.contains(tool),
            "banned_path {:?} must name the flagged {tool:?} literal",
            v.banned_path
        );
        assert!(v.line > 0, "line must be 1-based positive; got {}", v.line);
        assert!(v.column > 0, "column must be 1-based positive; got {}", v.column);
    }
}

// ---------------------------------------------------------------------------
// S-LINT-02 — the `// subprocess-ok: <reason>` marker (use-site line OR the
// line immediately above) SUPPRESSES the violation, for BOTH placements.
// ---------------------------------------------------------------------------

#[test]
fn s_lint_02_subprocess_ok_marker_suppresses_violation() {
    // Above-line placement.
    let source_above = "pub fn ensure() {\n    \
         // subprocess-ok: sanctioned CH confinement wrapper\n    \
         let _ = std::process::Command::new(\"ip\").arg(\"rule\").output();\n}\n";
    let violations = scan_source_infra_subprocess(source_above, SCOPED_SRC)
        .expect("scan must succeed for above-line-marker source");
    assert!(
        violations.is_empty(),
        "an above-line `// subprocess-ok:` marker must suppress the ban; got {violations:?}"
    );

    // Trailing same-line placement.
    let source_trailing = "pub fn ensure() { let _ = std::process::Command::new(\"ip\").output(); } // subprocess-ok: reason\n";
    let violations = scan_source_infra_subprocess(source_trailing, SCOPED_SRC)
        .expect("scan must succeed for trailing-marker source");
    assert!(
        violations.is_empty(),
        "a trailing `// subprocess-ok:` marker must suppress the ban; got {violations:?}"
    );
}

// ---------------------------------------------------------------------------
// S-LINT-03 — `#[cfg(test)]` items and `bin/` tooling paths are EXEMPT, so
// the seven literals inside a test module or a `bin/*.rs` tool are NOT
// flagged.
// ---------------------------------------------------------------------------

#[test]
fn s_lint_03_cfg_test_items_and_bin_tooling_are_exempt() {
    // `#[cfg(test)]` module in a scoped src path — exempt.
    let cfg_test_source = "#[cfg(test)]\nmod tests { fn t() { let _ = std::process::Command::new(\"ip\").output(); } }\n";
    let violations = scan_source_infra_subprocess(cfg_test_source, SCOPED_SRC)
        .expect("scan must succeed for cfg(test) source");
    assert!(
        violations.is_empty(),
        "`Command::new(\"ip\")` inside a #[cfg(test)] module must NOT be flagged; got {violations:?}"
    );

    // `bin/` tooling path — exempt by path, even outside cfg(test).
    let bin_source = "pub fn tool() { let _ = std::process::Command::new(\"ip\").output(); }\n";
    let violations =
        scan_source_infra_subprocess(bin_source, "crates/overdrive-worker/bin/some_tool.rs")
            .expect("scan must succeed for bin/ tooling source");
    assert!(
        violations.is_empty(),
        "`Command::new(\"ip\")` under a bin/ tooling path must NOT be flagged; got {violations:?}"
    );
}

// ---------------------------------------------------------------------------
// S-LINT-04 — `overdrive-testing` is EXCLUDED by scope; the SAME literal at a
// non-excluded adapter-host path IS flagged (the exclusion is path-scoped and
// non-vacuous).
// ---------------------------------------------------------------------------

#[test]
fn s_lint_04_overdrive_testing_is_excluded_by_scope() {
    let source = "pub fn setup() { let _ = std::process::Command::new(\"ip\").args([\"netns\", \"add\", \"x\"]).output(); }\n";

    // Excluded by scope — `overdrive-testing` legitimately shells `ip netns`.
    let violations = scan_source_infra_subprocess(source, "crates/overdrive-testing/src/netns.rs")
        .expect("scan must succeed for overdrive-testing source");
    assert!(
        violations.is_empty(),
        "overdrive-testing/src is excluded by scope; got {violations:?}"
    );

    // Same literal at a non-excluded adapter-host path — IS flagged.
    let violations = scan_source_infra_subprocess(
        source,
        "crates/overdrive-control-plane/src/veth_provisioner.rs",
    )
    .expect("scan must succeed for control-plane source");
    assert!(
        !violations.is_empty(),
        "the same `Command::new(\"ip\")` at a non-excluded adapter-host path MUST be flagged \
         (exclusion is path-scoped, non-vacuous)"
    );
    assert!(
        violations[0].banned_path.contains("ip"),
        "banned_path {:?} must name the `ip` literal",
        violations[0].banned_path
    );
}

// ---------------------------------------------------------------------------
// S-LINT-05 — THE regression guard (the "flips green immediately" gate,
// ADR-0085 D8): after slices 01–04 swap both files, the scanner reports ZERO
// infra-CLI-literal violations across the real in-scope production tree
// (`crate_class ∈ {core, adapter-host}` minus `overdrive-testing`). Mirrors
// `dst_lint_live_literal::s_06_03_dst_lint_passes_on_migrated_codebase`.
// ---------------------------------------------------------------------------

#[test]
fn s_lint_05_scanner_passes_on_the_migrated_tree() {
    let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root =
        crate_dir.parent().expect("xtask crate lives directly under workspace root");
    let manifest = workspace_root.join("Cargo.toml");

    let violations = scan_infra_subprocess_from_manifest(&manifest).expect(&format!(
        "scan_infra_subprocess_from_manifest must succeed for {}",
        manifest.display()
    ));

    if !violations.is_empty() {
        eprintln!(
            "S-LINT-05: {} infra-CLI-literal violation(s) across the in-scope tree:",
            violations.len()
        );
        for v in &violations {
            eprintln!("  {}:{}:{}: {}", v.file.display(), v.line, v.column, v.banned_path);
        }
    }
    assert_eq!(
        violations.len(),
        0,
        "door-lock: zero named infra-CLI `Command::new(\"<tool>\")` literals must remain in \
         production src/** of {{core, adapter-host}} crates (minus overdrive-testing)"
    );
}
