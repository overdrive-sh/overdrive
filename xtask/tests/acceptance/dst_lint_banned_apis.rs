//! Acceptance scenarios for US-05 §6.1 and §6.2 — the `dst-lint` gate.
//!
//! Each scenario invokes `cargo xtask dst-lint` as a subprocess (driving
//! port discipline per DWD-04). Most scenarios scaffold a synthetic
//! workspace inside a `tempfile::TempDir` whose crates carry
//! `[package.metadata.overdrive] crate_class = "..."` declarations, plant
//! a banned call (or not), and assert on `cargo xtask dst-lint`'s
//! observable outcome: exit status and stderr.
//!
//! The "clean workspace" scenario runs against the real Overdrive
//! workspace (this repository) — the Phase-1 core must be silent under
//! the lint gate, otherwise the whole point of the gate has already been
//! violated.
//!
//! To keep subprocess invocations realistic we pass each synthetic
//! workspace through the `--manifest-path` argument on the real xtask
//! binary. `cargo xtask dst-lint` then resolves workspace metadata
//! rooted at that manifest.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// Absolute path to the `xtask` binary produced by the current cargo
/// test invocation. We avoid shelling out through `cargo xtask …`
/// because that would re-enter Cargo and recompile the workspace for
/// every scenario. Running the already-built `xtask` binary directly
/// keeps the tests fast and the subprocess boundary exactly where the
/// roadmap requires it (the compiled CLI is the driving port).
fn xtask_bin() -> PathBuf {
    // `CARGO_BIN_EXE_xtask` is only defined inside tests when the xtask
    // crate declares a `[[bin]]` of that name — which it does.
    PathBuf::from(env!("CARGO_BIN_EXE_xtask"))
}

/// Path to the real Overdrive workspace `Cargo.toml` — used by the
/// clean-workspace scenario. We locate it relative to
/// `CARGO_MANIFEST_DIR` (the xtask crate root) because cargo sets cwd
/// to the crate root when running tests.
fn real_workspace_manifest() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir
        .parent()
        .expect("xtask crate lives directly under the workspace root")
        .join("Cargo.toml")
}

/// Run `xtask dst-lint --manifest-path <path>` and capture the result.
fn run_dst_lint(manifest_path: &Path) -> std::process::Output {
    Command::new(xtask_bin())
        .args(["dst-lint", "--manifest-path"])
        .arg(manifest_path)
        .output()
        .expect("xtask binary must be invokable")
}

/// Scaffold a synthetic workspace inside `dir` with the given crates.
/// Each `(name, crate_class, body)` triple becomes
/// `<dir>/<name>/Cargo.toml` + `<dir>/<name>/src/lib.rs`. When
/// `crate_class` is `None` the `[package.metadata.overdrive]` block is
/// omitted entirely — used for the "unlabelled crate" scenario.
fn scaffold_workspace(dir: &Path, crates: &[(&str, Option<&str>, &str)]) -> PathBuf {
    let members = crates.iter().map(|(n, _, _)| format!("\"{n}\"")).collect::<Vec<_>>().join(", ");
    let workspace_toml = format!(
        "[workspace]\n\
         resolver = \"2\"\n\
         members = [{members}]\n"
    );
    std::fs::write(dir.join("Cargo.toml"), workspace_toml).expect("write workspace Cargo.toml");

    for (name, crate_class, body) in crates {
        let crate_dir = dir.join(name);
        std::fs::create_dir_all(crate_dir.join("src")).expect("mkdir src");

        let metadata_block = crate_class.map_or_else(String::new, |class| {
            format!("[package.metadata.overdrive]\ncrate_class = \"{class}\"\n")
        });
        let cargo_toml = format!(
            "[package]\n\
             name        = \"{name}\"\n\
             version     = \"0.0.0\"\n\
             edition     = \"2021\"\n\
             publish     = false\n\
             \n\
             {metadata_block}\n\
             [lib]\n\
             path = \"src/lib.rs\"\n"
        );
        std::fs::write(crate_dir.join("Cargo.toml"), cargo_toml).expect("write crate Cargo.toml");
        std::fs::write(crate_dir.join("src").join("lib.rs"), body).expect("write lib.rs");
    }

    dir.join("Cargo.toml")
}

// -----------------------------------------------------------------------------
// §6.1 scenario 2 — "silent on clean core crates"
// -----------------------------------------------------------------------------

#[test]
fn dst_lint_exits_zero_on_the_real_overdrive_workspace() {
    // Given the current workspace — Phase 1 core must not contain any
    // banned API, otherwise the invariant the lint gate encodes is
    // already broken in main.
    let manifest = real_workspace_manifest();

    // When Ana runs cargo xtask dst-lint as a subprocess.
    let out = run_dst_lint(&manifest);

    // Then the subprocess exits with status zero.
    assert!(
        out.status.success(),
        "dst-lint must be green on the real workspace; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// -----------------------------------------------------------------------------
// §6.1 scenario 3 — "permits wiring crates to use real implementations"
// -----------------------------------------------------------------------------

#[test]
fn dst_lint_permits_adapter_host_crate_using_instant_now() {
    // Given a non-core wiring crate (crate_class = "adapter-host") that
    // constructs a real clock using Instant::now internally. We also
    // include a clean core crate so the fail-fast "no core crate"
    // guard-rail does not fire — the scenario under test is the
    // wiring-crate *exemption*, not the empty-core guard.
    let tmp = TempDir::new().expect("tempdir");
    let manifest = scaffold_workspace(
        tmp.path(),
        &[
            ("my-core", Some("core"), "pub fn noop() {}\n"),
            (
                "my-adapter",
                Some("adapter-host"),
                "pub fn now() -> std::time::Instant {\n    std::time::Instant::now()\n}\n",
            ),
        ],
    );

    // When Ana runs cargo xtask dst-lint as a subprocess.
    let out = run_dst_lint(&manifest);

    // Then the subprocess exits with status zero and no violation is
    // reported for the wiring crate.
    assert!(
        out.status.success(),
        "dst-lint must permit Instant::now in adapter-host crate; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// -----------------------------------------------------------------------------
// §6.2 — "banned-API detection" (four scenarios)
// -----------------------------------------------------------------------------

#[test]
fn dst_lint_blocks_instant_now_in_core_crate() {
    // Given a core crate contains a source line calling
    // std::time::Instant::now.
    let tmp = TempDir::new().expect("tempdir");
    let manifest = scaffold_workspace(
        tmp.path(),
        &[(
            "my-core",
            Some("core"),
            "pub fn tick() -> std::time::Instant {\n    std::time::Instant::now()\n}\n",
        )],
    );

    // When Ana runs cargo xtask dst-lint as a subprocess.
    let out = run_dst_lint(&manifest);

    // Then the subprocess exits with non-zero status.
    assert!(!out.status.success(), "dst-lint must fail on Instant::now in core");

    // And the output names the file, line, and column of the banned call.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("src/lib.rs"), "stderr must name the file:\n{stderr}");
    assert!(stderr.contains(":2:"), "stderr must name line 2 of the synthetic crate:\n{stderr}");

    // And the output names Clock as the replacement trait.
    assert!(stderr.contains("Clock"), "stderr must name Clock:\n{stderr}");

    // And the output references .claude/rules/development.md.
    assert!(
        stderr.contains(".claude/rules/development.md"),
        "stderr must link to development.md:\n{stderr}"
    );
}

#[test]
fn dst_lint_blocks_rand_random_in_core_crate() {
    let tmp = TempDir::new().expect("tempdir");
    let manifest = scaffold_workspace(
        tmp.path(),
        &[("my-core", Some("core"), "pub fn roll() -> u64 {\n    rand::random::<u64>()\n}\n")],
    );

    let out = run_dst_lint(&manifest);

    assert!(!out.status.success(), "dst-lint must fail on rand::random in core");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("src/lib.rs"), "stderr must name the file:\n{stderr}");
    assert!(stderr.contains(":2:"), "stderr must name line 2:\n{stderr}");
    assert!(stderr.contains("Entropy"), "stderr must name Entropy:\n{stderr}");
}

#[test]
fn dst_lint_blocks_thread_sleep_in_core_crate() {
    let tmp = TempDir::new().expect("tempdir");
    let manifest = scaffold_workspace(
        tmp.path(),
        &[(
            "my-core",
            Some("core"),
            "pub fn wait() {\n    std::thread::sleep(std::time::Duration::from_millis(1));\n}\n",
        )],
    );

    let out = run_dst_lint(&manifest);

    assert!(!out.status.success(), "dst-lint must fail on std::thread::sleep in core");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Clock"),
        "stderr must name Clock::sleep as the replacement:\n{stderr}"
    );
}

#[test]
fn dst_lint_blocks_tokio_tcp_stream_in_core_crate() {
    // We only need the type name to be referenced — no actual async code
    // or compile has to succeed. The lint is a pure syntactic scan.
    let tmp = TempDir::new().expect("tempdir");
    let manifest = scaffold_workspace(
        tmp.path(),
        &[("my-core", Some("core"), "pub type Stream = tokio::net::TcpStream;\n")],
    );

    let out = run_dst_lint(&manifest);

    assert!(!out.status.success(), "dst-lint must fail on tokio::net::TcpStream in core");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Transport"),
        "stderr must name Transport as the replacement:\n{stderr}"
    );
}

// -----------------------------------------------------------------------------
// §6.2 — fail-fast scenarios
// -----------------------------------------------------------------------------

#[test]
fn dst_lint_fails_fast_when_core_class_set_is_empty() {
    // Given every workspace crate is labelled something other than "core".
    let tmp = TempDir::new().expect("tempdir");
    let manifest = scaffold_workspace(
        tmp.path(),
        &[
            ("my-adapter", Some("adapter-host"), "pub fn noop() {}\n"),
            ("my-binary", Some("binary"), "pub fn noop() {}\n"),
        ],
    );

    let out = run_dst_lint(&manifest);

    assert!(!out.status.success(), "dst-lint must fail when no core crate exists");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no core-class crates"),
        "stderr must report empty core set:\n{stderr}"
    );
}

#[test]
fn dst_lint_fails_fast_when_a_crate_is_unlabelled() {
    // Given a workspace crate is missing the
    // package.metadata.overdrive.crate_class key.
    let tmp = TempDir::new().expect("tempdir");
    let manifest = scaffold_workspace(
        tmp.path(),
        &[
            ("my-core", Some("core"), "pub fn noop() {}\n"),
            ("my-undeclared", None, "pub fn noop() {}\n"),
        ],
    );

    let out = run_dst_lint(&manifest);

    assert!(!out.status.success(), "dst-lint must fail when a crate is unlabelled");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("my-undeclared"), "stderr must name the unlabelled crate:\n{stderr}");
    assert!(
        stderr.contains("crate_class"),
        "stderr must mention the missing metadata key:\n{stderr}"
    );
}

// -----------------------------------------------------------------------------
// §"Ordered-collection choice" — bare HashMap / HashSet ban in core crates.
// Mechanically enforces the rule from `.claude/rules/development.md` landed
// in commit e50146a (Step-ID 01-06). See `docs/feature/
// fix-eval-broker-drain-determinism/deliver/rca-context.md` for the defect
// chain that motivated the rule (RCA root cause #5).
// -----------------------------------------------------------------------------

#[test]
fn dst_lint_blocks_hashmap_in_core_crate() {
    // Given a core crate uses bare std::collections::HashMap (the rule from
    // step 01-06 forbids this without an explicit justification comment).
    let tmp = TempDir::new().expect("tempdir");
    let manifest = scaffold_workspace(
        tmp.path(),
        &[(
            "my-core",
            Some("core"),
            "pub fn build() -> std::collections::HashMap<String, u32> { \
             std::collections::HashMap::new() }\n",
        )],
    );

    // When Ana runs cargo xtask dst-lint as a subprocess.
    let out = run_dst_lint(&manifest);

    // Then the subprocess exits with non-zero status.
    assert!(!out.status.success(), "dst-lint must fail on bare HashMap in core");

    // And the output names the file and the offending line.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("src/lib.rs"), "stderr must name the file:\n{stderr}");
    assert!(stderr.contains(":1:"), "stderr must name line 1 of the synthetic crate:\n{stderr}");

    // And the output names BTreeMap as the recommended replacement.
    assert!(stderr.contains("BTreeMap"), "stderr must name BTreeMap as the replacement:\n{stderr}");

    // And the output cites the rules file rather than smuggling in install hints
    // (per user-memory feedback_no_user_install_instructions).
    assert!(
        stderr.contains(".claude/rules/development.md"),
        "stderr must link to development.md:\n{stderr}"
    );
}

#[test]
fn dst_lint_permits_hashmap_with_justification_comment_in_core_crate() {
    // Given a core crate uses bare HashMap but documents the choice with the
    // sanctioned `// dst-lint: hashmap-ok <reason>` escape comment immediately
    // above the use site.
    let tmp = TempDir::new().expect("tempdir");
    let manifest = scaffold_workspace(
        tmp.path(),
        &[(
            "my-core",
            Some("core"),
            "// dst-lint: hashmap-ok bounded cache, point access only\n\
             pub fn build() -> std::collections::HashMap<String, u32> { \
             std::collections::HashMap::new() }\n",
        )],
    );

    // When Ana runs cargo xtask dst-lint as a subprocess.
    let out = run_dst_lint(&manifest);

    // Then the subprocess exits cleanly — the justification is accepted.
    assert!(
        out.status.success(),
        "dst-lint must permit HashMap with justification comment; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn dst_lint_permits_hashmap_in_adapter_host_crate() {
    // Given a non-core wiring crate (crate_class = "adapter-host") uses bare
    // HashMap. We include a clean core crate so the fail-fast "no core crate"
    // guard-rail does not fire — the scenario under test is the wiring-crate
    // *exemption*, not the empty-core guard.
    let tmp = TempDir::new().expect("tempdir");
    let manifest = scaffold_workspace(
        tmp.path(),
        &[
            ("my-core", Some("core"), "pub fn noop() {}\n"),
            (
                "my-adapter",
                Some("adapter-host"),
                "pub fn build() -> std::collections::HashMap<String, u32> { \
                 std::collections::HashMap::new() }\n",
            ),
        ],
    );

    // When Ana runs cargo xtask dst-lint as a subprocess.
    let out = run_dst_lint(&manifest);

    // Then the subprocess exits cleanly — adapter-host is out of scope.
    assert!(
        out.status.success(),
        "dst-lint must permit HashMap in adapter-host crate; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn dst_lint_blocks_hashset_in_core_crate() {
    // Given a core crate uses bare std::collections::HashSet — the same rule
    // from step 01-06 covers HashSet alongside HashMap, since the iteration
    // order on HashSet has the same per-process random RandomState issue.
    let tmp = TempDir::new().expect("tempdir");
    let manifest = scaffold_workspace(
        tmp.path(),
        &[(
            "my-core",
            Some("core"),
            "pub fn build() -> std::collections::HashSet<String> { \
             std::collections::HashSet::new() }\n",
        )],
    );

    // When Ana runs cargo xtask dst-lint as a subprocess.
    let out = run_dst_lint(&manifest);

    // Then the subprocess exits with non-zero status.
    assert!(!out.status.success(), "dst-lint must fail on bare HashSet in core");

    // And the output names BTreeSet as the recommended replacement.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("BTreeSet"), "stderr must name BTreeSet as the replacement:\n{stderr}");
}
