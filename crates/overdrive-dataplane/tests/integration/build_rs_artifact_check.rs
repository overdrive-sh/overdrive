//! Asserts the `build.rs` artifact-check diagnostic shape on Linux.
//!
//! Per ADR-0038 §3.2 / `architecture.md` §3.2 the loader's `build.rs`
//! is a fail-fast guard whose entire purpose is to convert the
//! otherwise opaque rustc `file not found in include_bytes!` failure
//! into a single-line actionable diagnostic that names the fix
//! (`cargo xtask bpf-build`).
//!
//! This test removes the placeholder/real artifact, runs `cargo check
//! -p overdrive-dataplane` as a subprocess, and asserts:
//!
//! 1. exit code is non-zero, AND
//! 2. stderr names the artifact path AND `cargo xtask bpf-build`.
//!
//! Gated behind the `integration-tests`
//! feature on the crate via the `tests/integration.rs` entrypoint per
//! `.claude/rules/testing.md` § Layout. `#[serial(env)]` because the
//! test mutates the on-disk artifact (process-global file).

#![allow(clippy::print_stderr)]

use std::path::PathBuf;
use std::process::Command;

use serial_test::serial;

#[test]
#[serial(env)]
fn build_rs_emits_diagnostic_when_artifact_missing() -> Result<(), Box<dyn std::error::Error>> {
    // Skip under mutation testing. `cargo xtask mutants` sets
    // `OVERDRIVE_BPF_OBJECT` to an absolute path in the original tree
    // (so per-mutant copies under `/tmp/cargo-mutants-*/` resolve the
    // artifact correctly — see `crates/overdrive-dataplane/build.rs`
    // module docstring and `xtask::mutants::bpf_object_env_override`).
    // This test deliberately removes the artifact and asserts
    // build.rs's "BPF object not found" diagnostic — but under the
    // override, the build script consults the env var first and finds
    // the file at the original tree's location regardless of any
    // local removal in the mutant copy. The test is a build-script
    // shape assertion, not a logic property; CI (normal runs) and
    // local dev exercise it. Skipping here keeps the mutation gate
    // honest without weakening the build-script contract.
    if std::env::var_os("OVERDRIVE_BPF_OBJECT").is_some() {
        eprintln!("[skip] build_rs_artifact_check: OVERDRIVE_BPF_OBJECT set (under mutation test)");
        return Ok(());
    }

    let workspace_root = workspace_root();

    // Point OVERDRIVE_BPF_OBJECT at a guaranteed-nonexistent path.
    // This forces cargo to re-run build.rs via
    // `cargo:rerun-if-env-changed=OVERDRIVE_BPF_OBJECT` and makes
    // build.rs resolve the artifact against a path that does not
    // exist — triggering the "BPF object not found" diagnostic.
    //
    // Previous approach (delete the real artifact, run bare
    // `cargo check`) was unreliable: cargo's build-script cache from
    // the outer compilation (seconds earlier, artifact present) is
    // not always invalidated by file deletion within the same build
    // session, so `cargo check` returned exit 0 from the cached
    // output.
    let nonexistent = workspace_root.join("target/bpf/_nonexistent_test_artifact.o");

    let output = Command::new("cargo")
        .args(["check", "-p", "overdrive-dataplane"])
        .env("OVERDRIVE_BPF_OBJECT", &nonexistent)
        .current_dir(&workspace_root)
        .output()?;

    assert!(
        !output.status.success(),
        "cargo check should fail when BPF artifact missing (exit code {:?})",
        output.status.code()
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("BPF object not found at"),
        "stderr should name the missing artifact path. Got:\n{stderr}"
    );
    assert!(
        stderr.contains("cargo xtask bpf-build"),
        "stderr should name the fix command `cargo xtask bpf-build`. Got:\n{stderr}"
    );

    Ok(())
}

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR resolves to crates/overdrive-dataplane at
    // test compile time. Pop twice → workspace root (parent of crates/).
    let manifest = env!("CARGO_MANIFEST_DIR");
    let mut p = PathBuf::from(manifest);
    p.pop();
    p.pop();
    p
}
