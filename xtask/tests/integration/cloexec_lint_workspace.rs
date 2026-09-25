//! S-ND295-46 — the close-on-exec gate's workspace scan (OBL-295-CLOEXEC;
//! feature-delta § "Gate entry point").
//!
//! Drives [`xtask::cloexec_lint::scan_workspace`] over the real workspace and
//! over fixture workspaces built in a `tempfile::TempDir`. `scan_workspace`
//! runs `cargo metadata`, a subprocess, so these bodies live in the
//! `integration-tests`-gated binary like the other `tests/integration/`
//! bodies.
//!
//! Each fixture workspace is a real workspace that `cargo metadata` resolves
//! offline: path dependencies only. Its planted sources are parsed, never
//! compiled, so they call `libc` without depending on it. The DESIGN does not
//! pin whether a violation's `file` label is absolute or workspace-relative,
//! so the fixture bodies identify a violation by its file name, which is
//! unique within each fixture.
//!
//! Every planted violation sits at line 2, column 5: the first character of
//! the call's path, on the first line of a function body.

#![expect(
    clippy::doc_markdown,
    reason = "repository-mandated CONTRACT_SHAPE tokens are literal protocol markers"
)]

use std::fs;
use std::path::{Path, PathBuf};

use cargo_metadata::MetadataCommand;
use color_eyre::eyre::Report;
use tempfile::TempDir;
use xtask::cloexec_lint::{CloexecRule, CloexecViolation, render_violation, scan_workspace};

/// The real workspace manifest: `xtask`'s parent directory.
fn real_workspace_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one directory below the workspace root")
        .join("Cargo.toml")
}

/// A workspace on disk, removed when dropped.
struct FixtureWorkspace {
    root: TempDir,
}

impl FixtureWorkspace {
    /// A workspace whose root manifest lists `members`, relative to its root.
    fn new(members: &[&str]) -> Self {
        let fixture =
            Self { root: TempDir::new().expect("create the fixture workspace directory") };
        let members: Vec<String> = members.iter().map(|member| format!("\"{member}\"")).collect();
        fixture.write(
            "Cargo.toml",
            &format!("[workspace]\nmembers = [{}]\nresolver = \"2\"\n", members.join(", ")),
        );
        fixture
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.root.path().join(relative);
        let parent = path.parent().expect("a fixture file has a parent directory");
        fs::create_dir_all(parent)
            .unwrap_or_else(|err| panic!("create {}: {err}", parent.display()));
        fs::write(&path, contents).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
    }

    fn manifest_path(&self) -> PathBuf {
        self.root.path().join("Cargo.toml")
    }

    /// The fixture's workspace package names, sorted, after asserting that
    /// `cargo metadata` resolves its dependency graph. This precondition
    /// attributes an `Err` from `scan_workspace` to the scanned workspace,
    /// never to a malformed fixture.
    fn resolved_package_names(&self) -> Vec<String> {
        let manifest = self.manifest_path();
        let metadata =
            MetadataCommand::new().manifest_path(&manifest).exec().unwrap_or_else(|err| {
                panic!(
                    "fixture workspace {} must resolve under cargo metadata: {err}",
                    manifest.display()
                )
            });
        assert!(
            metadata.resolve.is_some(),
            "cargo metadata must resolve the fixture's dependency graph"
        );
        let mut names: Vec<String> =
            metadata.workspace_packages().iter().map(|package| package.name.clone()).collect();
        names.sort();
        names
    }
}

/// A package manifest: the `[package]` table followed by `rest`.
fn package_manifest(name: &str, rest: &str) -> String {
    format!(
        "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n{rest}"
    )
}

/// The whole cause chain of an `eyre` report, outermost first.
fn error_chain(err: &Report) -> String {
    err.chain().map(ToString::to_string).collect::<Vec<_>>().join(": ")
}

type Located = (String, usize, usize, String, CloexecRule);

/// Violations as `(file name, line, column, call, rule)`, sorted by site.
fn located(violations: Vec<CloexecViolation>) -> Vec<Located> {
    let mut located: Vec<Located> = violations
        .into_iter()
        .map(|v| {
            let name = v
                .file
                .file_name()
                .unwrap_or_else(|| panic!("violation label {} has no file name", v.file.display()))
                .to_string_lossy()
                .into_owned();
            (name, v.line, v.column, v.call, v.rule)
        })
        .collect();
    located.sort_by(|a, b| (&a.0, a.1, a.2).cmp(&(&b.0, b.1, b.2)));
    located
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-46 — the `overdrive serve` closure creates no inheritable descriptor
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 05-04 (S-ND295-46)"]
fn the_serve_closure_creates_no_inheritable_descriptor() {
    let manifest = real_workspace_manifest();
    let violations = scan_workspace(&manifest).unwrap_or_else(|err| {
        panic!(
            "scan_workspace must scan the real workspace at {}: {}",
            manifest.display(),
            error_chain(&err)
        )
    });
    let rendered: Vec<String> = violations.iter().map(render_violation).collect();
    assert!(
        violations.is_empty(),
        "every descriptor the overdrive serve closure creates must be created close-on-exec; \
         {} violation(s):\n{}",
        violations.len(),
        rendered.join("\n")
    );
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-46 — an unparseable serve source fails the scan instead of being skipped
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 05-04 (S-ND295-46)"]
fn an_unparseable_serve_source_fails_the_scan_instead_of_being_skipped() {
    let fixture = FixtureWorkspace::new(&["crates/overdrive-cli"]);
    fixture.write(
        "crates/overdrive-cli/Cargo.toml",
        &package_manifest(
            "overdrive-cli",
            "\n[[bin]]\nname = \"overdrive\"\npath = \"src/main.rs\"\n",
        ),
    );
    // A file that parses and carries a violation: a scan that skipped the
    // unparseable file would return `Ok` with this violation instead of `Err`.
    fixture.write(
        "crates/overdrive-cli/src/main.rs",
        "mod net;\n\nfn main() {\n    libc::dup(0);\n}\n",
    );
    fixture.write("crates/overdrive-cli/src/net/mod.rs", "mod unparseable_listener;\n");
    fixture.write(
        "crates/overdrive-cli/src/net/unparseable_listener.rs",
        "pub fn listen( -> i32 {\n    libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0)\n",
    );
    assert_eq!(fixture.resolved_package_names(), ["overdrive-cli"], "fixture precondition");

    let err = match scan_workspace(&fixture.manifest_path()) {
        Ok(violations) => panic!(
            "a serve source that does not parse must fail the scan, never be skipped; \
             got Ok({violations:?})"
        ),
        Err(err) => err,
    };
    let chain = error_chain(&err);
    assert!(
        chain.contains("unparseable_listener.rs"),
        "the scan error must name the file it could not parse; got: {chain}"
    );
}

/// The serve-closure fixture's members.
const CLOSURE_MEMBERS: &[&str] = &[
    "crates/overdrive-cli",
    "crates/overdrive-serve-dep",
    "crates/overdrive-transitive",
    "crates/overdrive-dev-only",
    "crates/overdrive-build-only",
    "crates/overdrive-guest-init",
];

/// The serve-closure fixture's files other than the root manifest, as
/// `(path, contents)`.
///
/// Inside the closure: the `overdrive` binary root of `overdrive-cli` and a
/// nested module of a transitive normal dependency, each with one violation.
/// Outside it, each with one violation of its own: an auxiliary binary under
/// `overdrive-cli/src/bin/`; a crate-root `bin/` tool, declared the way the
/// real crate declares `bin/coinflip_helper.rs`; a dev-dependency; a
/// build-dependency; and a binary no member depends on (the `overdrive-init`
/// shape).
const CLOSURE_FILES: &[(&str, &str)] = &[
    (
        "crates/overdrive-cli/Cargo.toml",
        "\
[package]
name = \"overdrive-cli\"
version = \"0.0.0\"
edition = \"2021\"
publish = false

[[bin]]
name = \"overdrive\"
path = \"src/main.rs\"

[[bin]]
name = \"gate_helper\"
path = \"bin/gate_helper.rs\"

[dependencies]
overdrive-serve-dep = { path = \"../overdrive-serve-dep\" }

[dev-dependencies]
overdrive-dev-only = { path = \"../overdrive-dev-only\" }

[build-dependencies]
overdrive-build-only = { path = \"../overdrive-build-only\" }
",
    ),
    ("crates/overdrive-cli/src/main.rs", "mod serve_listener;\n\nfn main() {}\n"),
    (
        "crates/overdrive-cli/src/serve_listener.rs",
        "pub fn bind(fd: i32) {\n    libc::dup(fd);\n}\n",
    ),
    ("crates/overdrive-cli/src/bin/aux_tool.rs", "fn main() {\n    libc::dup(0);\n}\n"),
    ("crates/overdrive-cli/bin/gate_helper.rs", "fn main() {\n    libc::dup(1);\n}\n"),
    (
        "crates/overdrive-serve-dep/Cargo.toml",
        "\
[package]
name = \"overdrive-serve-dep\"
version = \"0.0.0\"
edition = \"2021\"
publish = false

[dependencies]
overdrive-transitive = { path = \"../overdrive-transitive\" }
",
    ),
    ("crates/overdrive-serve-dep/src/lib.rs", "pub fn noop() {}\n"),
    (
        "crates/overdrive-transitive/Cargo.toml",
        "[package]\nname = \"overdrive-transitive\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n",
    ),
    ("crates/overdrive-transitive/src/lib.rs", "pub mod net;\n"),
    ("crates/overdrive-transitive/src/net/mod.rs", "pub mod transitive_listener;\n"),
    (
        "crates/overdrive-transitive/src/net/transitive_listener.rs",
        "pub fn listen() {\n    libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);\n}\n",
    ),
    (
        "crates/overdrive-dev-only/Cargo.toml",
        "[package]\nname = \"overdrive-dev-only\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n",
    ),
    ("crates/overdrive-dev-only/src/lib.rs", "pub mod dev_only_fixture;\n"),
    (
        "crates/overdrive-dev-only/src/dev_only_fixture.rs",
        "pub fn fixture() {\n    libc::dup(2);\n}\n",
    ),
    (
        "crates/overdrive-build-only/Cargo.toml",
        "[package]\nname = \"overdrive-build-only\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n",
    ),
    ("crates/overdrive-build-only/src/lib.rs", "pub mod build_only_helper;\n"),
    (
        "crates/overdrive-build-only/src/build_only_helper.rs",
        "pub fn helper() {\n    libc::dup(3);\n}\n",
    ),
    (
        "crates/overdrive-guest-init/Cargo.toml",
        "\
[package]
name = \"overdrive-guest-init\"
version = \"0.0.0\"
edition = \"2021\"
publish = false

[[bin]]
name = \"overdrive-guest-init\"
path = \"src/guest_init.rs\"
",
    ),
    ("crates/overdrive-guest-init/src/guest_init.rs", "fn main() {\n    libc::dup(4);\n}\n"),
];

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-46 — auxiliary binaries outside the serve closure are not scanned
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 05-04 (S-ND295-46)"]
fn auxiliary_binaries_outside_the_serve_closure_are_not_scanned() {
    let fixture = FixtureWorkspace::new(CLOSURE_MEMBERS);
    for (path, contents) in CLOSURE_FILES {
        fixture.write(path, contents);
    }
    assert_eq!(
        fixture.resolved_package_names(),
        [
            "overdrive-build-only",
            "overdrive-cli",
            "overdrive-dev-only",
            "overdrive-guest-init",
            "overdrive-serve-dep",
            "overdrive-transitive",
        ],
        "fixture precondition"
    );

    let violations = scan_workspace(&fixture.manifest_path())
        .unwrap_or_else(|err| panic!("the fixture workspace must scan: {}", error_chain(&err)));
    assert_eq!(
        located(violations),
        [
            (
                "serve_listener.rs".to_owned(),
                2,
                5,
                "libc::dup".to_owned(),
                CloexecRule::AlwaysInheritable
            ),
            (
                "transitive_listener.rs".to_owned(),
                2,
                5,
                "libc::socket".to_owned(),
                CloexecRule::MissingFlag
            ),
        ],
        "only the overdrive-cli package and its normal dependency closure are scanned: \
         not src/bin/, not the crate-root bin/, not dev-, build-, or unrelated members"
    );
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-46 — a workspace without the `overdrive-cli` package is an error
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 05-04 (S-ND295-46)"]
fn a_workspace_without_the_cli_package_is_an_error() {
    let fixture = FixtureWorkspace::new(&["crates/overdrive-control-plane"]);
    fixture.write(
        "crates/overdrive-control-plane/Cargo.toml",
        &package_manifest("overdrive-control-plane", ""),
    );
    // A violation the scan could report if it fell back to another package.
    fixture.write(
        "crates/overdrive-control-plane/src/lib.rs",
        "pub fn listen() {\n    libc::dup(0);\n}\n",
    );
    assert_eq!(
        fixture.resolved_package_names(),
        ["overdrive-control-plane"],
        "fixture precondition"
    );

    if let Ok(violations) = scan_workspace(&fixture.manifest_path()) {
        panic!(
            "a workspace without an overdrive-cli package has no overdrive serve closure and \
             must be an error; got Ok({violations:?})"
        );
    }
}
