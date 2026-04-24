//! Step 04-06 — dst-lint catches a reconciler body that smuggles
//! `std::time::Instant::now` past the §18 purity contract.
//!
//! Covers test-scenarios.md §5.10. This module complements the §6.2
//! tests in `dst_lint_banned_apis.rs`: those prove the lint fires on
//! any banned-API shape; this one proves the exact scenario the §18
//! reconciler contract depends on — a reconciler *body* calling the
//! real wall clock — is blocked in core crates, permitted in
//! wiring crates, and that the remediation message steers the reader
//! at the `Clock` trait specifically.
//!
//! The fixture lives at
//! `crates/overdrive-control-plane/tests/fixtures/bad_reconciler.rs.in`
//! (the `.rs.in` extension keeps rustc from compiling it as part of
//! the control-plane crate). Each scenario below reads the fixture
//! bytes, plants them inside a synthetic workspace with a specific
//! `crate_class` labelling, and invokes the compiled `xtask` binary
//! via `cargo` integration-test fixtures — the same driving-port
//! subprocess pattern `dst_lint_banned_apis.rs` uses.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// Absolute path to the compiled `xtask` binary — the driving port
/// for this scenario.
fn xtask_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_xtask"))
}

/// Absolute path to the reconciler-body fixture file. Resolved
/// relative to the xtask crate root because cargo sets cwd to the
/// crate root when running tests.
fn bad_reconciler_fixture() -> PathBuf {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask crate lives directly under the workspace root")
        .to_path_buf();
    workspace_root
        .join("crates")
        .join("overdrive-control-plane")
        .join("tests")
        .join("fixtures")
        .join("bad_reconciler.rs.in")
}

/// Run `xtask dst-lint --manifest-path <path>` and capture the result.
fn run_dst_lint(manifest_path: &Path) -> std::process::Output {
    Command::new(xtask_bin())
        .args(["dst-lint", "--manifest-path"])
        .arg(manifest_path)
        .output()
        .expect("xtask binary must be invokable")
}

/// Scaffold a two-crate workspace under `dir`:
///
/// - A `my-core` crate carrying the label from `reconciler_class`,
///   whose `src/lib.rs` is the contents of the bad-reconciler fixture.
/// - A `my-core-sibling` crate labelled `core` containing nothing
///   banned — dst-lint's `scan_workspace` guard-rail requires at least
///   one core crate to exist before it will scan anything, so when
///   `reconciler_class = "adapter-host"` we still need a clean core
///   crate in the workspace so the guard-rail doesn't fire first.
///
/// Returns the absolute path to the synthetic workspace manifest.
fn scaffold_with_reconciler(dir: &Path, reconciler_class: &str) -> PathBuf {
    let fixture =
        std::fs::read_to_string(bad_reconciler_fixture()).expect("bad_reconciler fixture readable");

    let workspace_toml = "[workspace]\n\
         resolver = \"2\"\n\
         members = [\"my-reconciler\", \"my-core-sibling\"]\n";
    std::fs::write(dir.join("Cargo.toml"), workspace_toml).expect("write workspace Cargo.toml");

    write_crate(dir, "my-reconciler", reconciler_class, &fixture);
    write_crate(dir, "my-core-sibling", "core", "pub fn noop() {}\n");

    dir.join("Cargo.toml")
}

fn write_crate(root: &Path, name: &str, crate_class: &str, body: &str) {
    let crate_dir = root.join(name);
    std::fs::create_dir_all(crate_dir.join("src")).expect("mkdir src");

    let cargo_toml = format!(
        "[package]\n\
         name        = \"{name}\"\n\
         version     = \"0.0.0\"\n\
         edition     = \"2021\"\n\
         publish     = false\n\
         \n\
         [package.metadata.overdrive]\n\
         crate_class = \"{crate_class}\"\n\
         \n\
         [lib]\n\
         path = \"src/lib.rs\"\n"
    );
    std::fs::write(crate_dir.join("Cargo.toml"), cargo_toml).expect("write crate Cargo.toml");
    std::fs::write(crate_dir.join("src").join("lib.rs"), body).expect("write lib.rs");
}

// -----------------------------------------------------------------------------
// §5.10 — reconciler body in core crate must be refused
// -----------------------------------------------------------------------------

#[test]
fn bad_reconciler_in_core_crate_fails_dst_lint_with_named_violation() {
    // Given a core-class crate whose lib.rs contains a reconciler body
    // that reads std::time::Instant::now.
    let tmp = TempDir::new().expect("tempdir");
    let manifest = scaffold_with_reconciler(tmp.path(), "core");

    // When Ana runs cargo xtask dst-lint as a subprocess.
    let out = run_dst_lint(&manifest);

    // Then the subprocess exits with non-zero status.
    assert!(
        !out.status.success(),
        "dst-lint must refuse reconciler-body Instant::now in core; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // And the output names the file (src/lib.rs), a line number, and
    // a column position for the banned call. The fixture places
    // `let _now = Instant::now();` inside `reconcile()`, which lands
    // at src/lib.rs line 24 column 20 — but tests should not be
    // brittle about exact line/column positions across fixture edits,
    // so we only assert the file + that *some* line:column pair is
    // printed.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("src/lib.rs"), "stderr must name the file:\n{stderr}");
    assert!(
        stderr.contains("src/lib.rs:") && stderr.matches(':').count() >= 2,
        "stderr must render file:line:column for the violation:\n{stderr}"
    );

    // And the output names the banned symbol Instant::now.
    assert!(
        stderr.contains("Instant::now"),
        "stderr must name the banned Instant::now symbol:\n{stderr}"
    );
}

// -----------------------------------------------------------------------------
// §5.10 — remediation message names the Clock trait
// -----------------------------------------------------------------------------

#[test]
fn dst_lint_reconciler_violation_remediation_names_clock_trait() {
    // Given the same reconciler-body fixture in a core crate.
    let tmp = TempDir::new().expect("tempdir");
    let manifest = scaffold_with_reconciler(tmp.path(), "core");

    // When dst-lint runs.
    let out = run_dst_lint(&manifest);

    // Then the remediation message steers the reader at the Clock
    // trait specifically — the correct replacement for
    // std::time::Instant::now per the §18 purity contract and
    // .claude/rules/development.md.
    assert!(!out.status.success(), "dst-lint must fail on Instant::now in reconciler body");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Clock"), "stderr must name Clock as the replacement trait:\n{stderr}");
}

// -----------------------------------------------------------------------------
// §5.10 — ADR-0003 wiring exemption: adapter-host crate passes
// -----------------------------------------------------------------------------

#[test]
fn bad_reconciler_in_adapter_host_crate_passes_dst_lint() {
    // Given the identical reconciler-body fixture, but labelled as a
    // wiring crate (crate_class = "adapter-host") per ADR-0003 —
    // adapter-host is where real Clock / Transport / Entropy
    // implementations live, so Instant::now is expected and allowed.
    let tmp = TempDir::new().expect("tempdir");
    let manifest = scaffold_with_reconciler(tmp.path(), "adapter-host");

    // When dst-lint runs.
    let out = run_dst_lint(&manifest);

    // Then the subprocess exits with status zero — dst-lint only
    // scans core-class crates, and the sibling core crate in the
    // scaffold is clean.
    assert!(
        out.status.success(),
        "dst-lint must exempt Instant::now in adapter-host; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
