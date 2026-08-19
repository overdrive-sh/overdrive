//! Library surface for `xtask` — exposes modules that integration tests
//! need to reach without going through the subprocess boundary.
//!
//! The binary entry point (`cargo xtask <cmd>`) lives in `src/main.rs`;
//! the shared implementations live here.

#![allow(clippy::expect_used, clippy::print_stderr, clippy::unnecessary_wraps)]

pub mod dev_setup;
pub mod dst_lint;
pub mod mutants;
pub mod yaml_free_cli;

/// The dated `rustup` nightly channel used for every kernel-side build.
///
/// Covers `bpfel-unknown-none` build and lint invocations — `cargo xtask
/// bpf-build`, `cargo xtask bpf-clippy`, and the `cargo xtask dev-setup`
/// / Lima provisioning surfaces that install it.
///
/// **Why pinned, not the floating `nightly` channel.** `bpf-linker`
/// (installed via `cargo install --locked bpf-linker` per ADR-0038
/// §4) links BPF object files as LLVM bitcode; its default feature
/// set (`rust-llvm-22` as of the 0.10.x series) requires the LLVM
/// major version embedded in the active `rustc` to match the LLVM
/// major bpf-linker itself was built against — see
/// <https://github.com/aya-rs/bpf-linker/blob/main/BUILDING.md>
/// ("the LLVM version used by bpf-linker must match the LLVM version
/// used by the Rust toolchain you intend to use"). `bpf-linker`
/// 0.10.4 (crates.io, released 2026-07-12 — the latest release as of
/// this writing) tops out at the `llvm-22` Cargo feature; there is no
/// `llvm-23` feature upstream yet.
///
/// The floating `nightly` channel bumped its embedded LLVM from
/// 22.1.8 to 23.1.0 between the 2026-08-04 and 2026-08-06 nightly
/// builds (confirmed by bisecting dated nightlies — `rustc
/// +nightly-2026-08-05 --version --verbose` still reports `LLVM
/// version: 22.1.8`; `+nightly-2026-08-07` reports `23.1.0`). Any
/// `bpf-build` / `bpf-clippy` run against a floating `nightly` picked
/// up after that bump fails the link step with `Error: failure
/// linking module ...` — the classic bpf-linker/rustc LLVM-version
/// skew signature, not a `bpf-linker` install defect.
///
/// **Re-pinning.** Bump this constant (and the matching literal in
/// `infra/lima/overdrive-dev.yaml`'s nightly-install and
/// readiness-probe steps) once `bpf-linker` ships an `llvm-23`
/// feature — or earlier, if a newer LLVM-22-era dated nightly is
/// preferred. Verify any candidate date the same way this one was
/// verified: `rustc +nightly-<date> --version --verbose | grep LLVM`
/// must report a major version the currently-installed `bpf-linker`
/// release actually supports (check that release's `[features]`
/// block in its `Cargo.toml` on crates.io/GitHub for the highest
/// `llvm-NN` it declares).
pub const BPF_NIGHTLY_TOOLCHAIN: &str = "nightly-2026-08-05";
