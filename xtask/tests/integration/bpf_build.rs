//! Asserts `cargo xtask bpf-build` produces the BPF ELF at the
//! stable path and propagates failures cleanly.
//!
//! Per ADR-0038 §3.1 and architecture.md §3.1, the subcommand:
//! 1. probes `bpf-linker` via `which_or_hint` and exits non-zero
//!    with a structured hint listing all three install paths
//!    (`cargo install --locked bpf-linker`, `cargo xtask dev-setup`,
//!    Lima re-provision) when the linker is absent;
//! 2. invokes `cargo +nightly build --release --target
//!    bpfel-unknown-none -Z build-std=core --features
//!    build-bpf-target --manifest-path crates/overdrive-bpf/Cargo.toml`;
//! 3. copies the produced ELF to
//!    `target/xtask/bpf-objects/overdrive_bpf.o` (the load-bearing
//!    stable path the loader's `include_bytes!` references).
//!
//! Linux-only — `bpfel-unknown-none` cross-tooling is not shipped on
//! macOS (architecture.md §5).

#![cfg(target_os = "linux")]

use serial_test::serial;
use std::path::PathBuf;
use std::process::Command;

/// Walk up from `xtask/`'s manifest dir to the workspace root.
fn workspace_root() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let mut p = PathBuf::from(manifest);
    // CARGO_MANIFEST_DIR points at `<workspace>/xtask`; pop once.
    p.pop();
    p
}

/// `cargo xtask bpf-build` produces the ELF at the stable path.
///
/// The artifact path is load-bearing — the loader's `include_bytes!`
/// reads from exactly this location, and the `build.rs` shim in
/// `overdrive-dataplane` checks for it. Drift breaks both consumers.
#[test]
#[serial(env)]
fn bpf_build_produces_artifact_at_stable_path() {
    let root = workspace_root();
    let artifact = root.join("target/xtask/bpf-objects/overdrive_bpf.o");

    // Ensure a clean slate so the test cannot pass on a stale artifact
    // from a previous run.
    let _ = std::fs::remove_file(&artifact);

    let output = Command::new("cargo")
        .args(["xtask", "bpf-build"])
        .current_dir(&root)
        .output()
        .expect("spawn cargo xtask bpf-build");

    assert!(
        output.status.success(),
        "cargo xtask bpf-build failed (exit {:?})\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    assert!(artifact.exists(), "expected artifact at {} after bpf-build", artifact.display(),);
    let metadata = std::fs::metadata(&artifact).expect("stat artifact");
    assert!(
        metadata.len() > 0,
        "expected real ELF at {} (got zero bytes — placeholder was not replaced)",
        artifact.display(),
    );
}

/// Without `bpf-linker` on PATH, the subcommand fails non-zero and
/// the install hint names at least one of the three documented
/// install paths (so the operator knows what to do next).
///
/// Implementation note: invoke the already-built `xtask` binary
/// directly (via `env!("CARGO_BIN_EXE_xtask")`) rather than through
/// `cargo xtask`. Going through cargo would re-spawn cargo itself,
/// which may re-trigger PATH resolution for rustc / linkers and
/// re-acquire `bpf-linker` from elsewhere on the surface (or fail
/// for unrelated reasons because cargo's own deps are no longer
/// reachable). The direct-binary form exercises exactly the
/// `which_or_hint("bpf-linker", …)` codepath in `bpf_build()` with
/// no other moving parts.
#[test]
#[serial(env)]
fn bpf_build_fails_when_bpf_linker_missing() {
    let root = workspace_root();

    // Strip every PATH entry that contains a `bpf-linker` binary.
    // Because we invoke the xtask binary directly, the child does
    // not need cargo / rustc / linkers on PATH — only `sh` (for
    // `which_or_hint`'s `sh -c "command -v bpf-linker"` probe).
    // /bin and /usr/bin are stable anchors for `sh` on every Linux
    // distro the project supports.
    let path = std::env::var("PATH").expect("PATH set");
    let stripped: Vec<&str> =
        path.split(':').filter(|p| !std::path::Path::new(p).join("bpf-linker").exists()).collect();
    let new_path = stripped.join(":");

    let xtask_bin = env!("CARGO_BIN_EXE_xtask");
    let output = Command::new(xtask_bin)
        .args(["bpf-build"])
        .env("PATH", &new_path)
        .current_dir(&root)
        .output()
        .expect("spawn xtask bpf-build directly");

    assert!(
        !output.status.success(),
        "cargo xtask bpf-build should fail without bpf-linker; got exit {:?}\n--- stderr ---\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("bpf-linker"),
        "stderr should mention bpf-linker.\n--- stderr ---\n{stderr}",
    );
    assert!(
        stderr.contains("cargo install --locked bpf-linker")
            || stderr.contains("cargo xtask dev-setup")
            || stderr.contains("Lima"),
        "stderr should name at least one install path \
         (cargo install --locked bpf-linker / cargo xtask dev-setup / Lima).\n\
         --- stderr ---\n{stderr}",
    );
}
