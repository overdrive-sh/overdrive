//! Build script for `overdrive-dataplane` per ADR-0038 §3.2.
//!
//! Two responsibilities:
//!
//! 1. Set `CARGO_WORKSPACE_DIR` via `cargo:rustc-env=` so `env!()`
//!    resolves at compile time inside the `include_bytes!` macro in
//!    `src/lib.rs`. Cargo only sets `CARGO_MANIFEST_DIR` for free;
//!    `CARGO_WORKSPACE_DIR` is not standard and must be emitted here.
//!
//! 2. On Linux build hosts, fail fast with a single-line actionable
//!    diagnostic when the BPF artifact at the stable path
//!    `target/xtask/bpf-objects/overdrive_bpf.o` is missing —
//!    converting the otherwise opaque rustc `file not found in
//!    include_bytes!` failure into a clear "run cargo xtask bpf-build
//!    first" hint. Also emits `cargo:rerun-if-changed=` on the
//!    artifact path so xtask regeneration triggers an incremental
//!    relink of the loader.
//!
//! Per `architecture.md` §3.4 the script makes ZERO recursive cargo
//! invocations and spawns no subprocesses — it is a fail-fast guard
//! whose entire purpose is to surface a clearer error than the rustc
//! default. Recursive cargo from build.rs is a documented anti-pattern
//! (aya-template's default; breaks workspace caching, opaque errors,
//! hostile to incremental rebuilds).
//!
//! On non-Linux build targets the artifact-check is skipped via
//! `#[cfg(target_os = "linux")]` — the `include_bytes!` constant in
//! `src/lib.rs` lives behind the same cfg gate and is never evaluated
//! off-Linux, so the artifact need not exist on developer macOS.

use std::path::PathBuf;

fn main() {
    // CARGO_MANIFEST_DIR is unconditionally set by cargo for every
    // build script invocation; Err here means the build environment
    // itself is broken, in which case panicking with a clear message
    // is the intended outcome.
    let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") else {
        panic!("CARGO_MANIFEST_DIR not set — cargo always sets this for build scripts");
    };
    let manifest_dir = PathBuf::from(manifest_dir);

    // Without any `cargo:rerun-if-changed` directive, Cargo falls back
    // to its default heuristic and re-runs this script on every change
    // to any file in the package. On non-Linux the artifact-check
    // block below is cfg'd out and emits nothing, so anchor the
    // dependency set to `build.rs` itself unconditionally.
    println!("cargo:rerun-if-changed=build.rs");

    // `crates/overdrive-dataplane` → workspace root: pop twice (once
    // for the crate name, once for `crates/`). `None` here means the
    // crate has been moved outside `crates/<name>/`, which is itself
    // an invariant violation.
    let workspace_root = manifest_dir.parent().and_then(|p| p.parent()).map_or_else(
        || {
            panic!(
                "workspace root not found above CARGO_MANIFEST_DIR={}; \
                 expected layout `crates/overdrive-dataplane/`",
                manifest_dir.display()
            )
        },
        std::path::Path::to_path_buf,
    );

    // Emit `CARGO_WORKSPACE_DIR` for `env!()` in `src/lib.rs`. Both
    // Linux and non-Linux targets need this — on macOS the env var is
    // still consulted by `cargo check` even though the
    // `include_bytes!` constant is cfg-gated out.
    println!("cargo:rustc-env=CARGO_WORKSPACE_DIR={}", workspace_root.display());

    // Linux artifact-check shim. macOS short-circuits — the
    // `include_bytes!` in `src/lib.rs` is `#[cfg(target_os = "linux")]`
    // and never evaluated on non-Linux, so the artifact need not be
    // present.
    #[cfg(target_os = "linux")]
    {
        let artifact = workspace_root.join("target/xtask/bpf-objects/overdrive_bpf.o");
        if !artifact.exists() {
            // Build scripts surface diagnostics via stderr; cargo
            // captures and renders the `--- stderr` block on failure.
            // `clippy::print_stderr` is not the right gate for build.rs.
            #[allow(clippy::print_stderr)]
            {
                eprintln!(
                    "error: BPF object not found at {}; run `cargo xtask bpf-build` first",
                    artifact.display()
                );
            }
            std::process::exit(1);
        }
        println!("cargo:rerun-if-changed={}", artifact.display());
    }
}
