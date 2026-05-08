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
    let artifact = workspace_root.join("target/bpf/overdrive_bpf.o");

    // Snapshot any existing artifact so the test is reversible: the
    // placeholder produced by the GREEN setup, or a real
    // `cargo xtask bpf-build` output, must survive this test.
    let backup = if artifact.exists() { Some(std::fs::read(&artifact)?) } else { None };

    if artifact.exists() {
        std::fs::remove_file(&artifact)?;
    }

    let output = Command::new("cargo")
        .args(["check", "-p", "overdrive-dataplane"])
        .current_dir(&workspace_root)
        .output()?;

    // Restore the artifact (best-effort) BEFORE asserting so a panic
    // here does not leak a missing-artifact state to later tests.
    if let Some(bytes) = backup {
        if let Some(parent) = artifact.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&artifact, bytes);
    }

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
