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
//!    `target/bpf/overdrive_bpf.o` is missing —
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
//!
//! ## `OVERDRIVE_BPF_OBJECT` override
//!
//! When set, the env var `OVERDRIVE_BPF_OBJECT` overrides the
//! workspace-relative artifact path. The script uses the env var's
//! value verbatim (must be an absolute path; not validated here —
//! cargo's existence-check below catches typos).
//!
//! This is the override `cargo xtask mutants` sets on every
//! cargo-mutants invocation. cargo-mutants creates a per-mutant copy
//! of the source tree under `/tmp/cargo-mutants-*/` and runs cargo
//! from there — but it does NOT copy `target/`. Without an absolute
//! path pointing at the *original* tree's BPF object, every mutant of
//! `overdrive-dataplane` panics here with "BPF object not found",
//! marks itself unviable, and the kill-rate signal collapses to zero.
//! See `xtask::mutants::bpf_object_env_override` for the wrapper-side
//! documentation and the rationale for choosing this mechanism over
//! `--copy-target` / `--in-place`.
//!
//! Regular `cargo {check,test,build}` invocations leave the env var
//! unset; the workspace-relative fallback applies and the build script
//! behaves exactly as before this override was introduced.

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

    // Re-run if the override env var changes. Without this directive
    // cargo caches the build script's output and a fresh
    // `OVERDRIVE_BPF_OBJECT` value would not invalidate the cached
    // build, so the script would silently keep using the old artifact
    // path (or the workspace fallback). Emit unconditionally — cheap
    // and matches the contract for the override.
    println!("cargo:rerun-if-env-changed=OVERDRIVE_BPF_OBJECT");

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

    // Emit `CARGO_WORKSPACE_DIR` for `env!()` in `src/lib.rs`.
    println!("cargo:rustc-env=CARGO_WORKSPACE_DIR={}", workspace_root.display());

    // `OVERDRIVE_BPF_OBJECT` override (see module docstring).
    // Empty values are treated as "unset" so a stray
    // `OVERDRIVE_BPF_OBJECT=` does not silently cripple the
    // fallback — the canonical "unset" shape from cargo's env
    // plumbing is `None`, but a user-supplied empty value goes
    // through as `Some("")`, which would then resolve `path.exists()`
    // against `""` and fail every time. Treat both as "not set".
    let artifact = std::env::var_os("OVERDRIVE_BPF_OBJECT")
        .filter(|v| !v.is_empty())
        .map_or_else(|| workspace_root.join("target/bpf/overdrive_bpf.o"), PathBuf::from);
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

    // Emit the resolved artifact path as a rustc-env so the
    // `include_bytes!` macro in `src/lib.rs` (and the matching
    // copy in `tests/integration/reverse_nat_e2e.rs`) consumes the
    // override transparently. Without this, those macros would
    // resolve `concat!(env!("CARGO_WORKSPACE_DIR"), "/target/...")`,
    // which under `cargo-mutants` points at the per-mutant copy
    // `/tmp/cargo-mutants-*.tmp/target/...` — a path that does not
    // exist because cargo-mutants does not copy `target/`. With
    // `OVERDRIVE_BPF_OBJECT_PATH` emitted here, lib.rs reads the
    // absolute path the wrapper supplied and `include_bytes!`
    // resolves to the original tree's artifact.
    println!("cargo:rustc-env=OVERDRIVE_BPF_OBJECT_PATH={}", artifact.display());
}
