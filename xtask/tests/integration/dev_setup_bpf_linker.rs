//! Asserts `cargo xtask dev-setup` plans the correct install argv for
//! the bpf-linker + nightly toolchain + rust-src dependency triple,
//! across the four ProbeContext shapes that arise in practice.
//!
//! Per ADR-0038 §4 (and upstream-issue A1) the dev-setup task is the
//! non-Lima Linux developer surface for installing exactly the same
//! toolchain bits the Lima YAML provisions in `infra/lima/overdrive-dev.yaml`:
//!
//! - `bpf-linker` via `cargo install --locked bpf-linker`
//! - `nightly` rustup toolchain via `rustup toolchain install nightly
//!   --component rust-src --profile minimal`
//! - `rust-src` component on nightly via `rustup component add rust-src
//!   --toolchain nightly` (when the toolchain is already present but
//!   the component is missing).
//!
//! The four tests cover the four planning permutations:
//!
//! 1. bpf-linker missing → planned argv contains `--locked` (AC10).
//! 2. bpf-linker present → no install command planned (AC2).
//! 3. nightly toolchain missing → fresh-toolchain install argv shape.
//! 4. nightly toolchain present, rust-src component missing → component-add
//!    argv shape (NOT a fresh toolchain install).
//!
//! These are pure argv-construction assertions against the planning
//! function — no process spawning, no environment mutation, no
//! filesystem touches. Linux-only by `#[cfg(target_os = "linux")]`
//! because the dev-setup install path is Linux-only per AC7 (macOS
//! short-circuits with a `tracing::warn`).

#![cfg(target_os = "linux")]

use xtask::dev_setup::{Plan, ProbeContext, plan};

/// AC1, AC3, AC10 — when bpf-linker is absent the plan contains a
/// `cargo install --locked bpf-linker` command. The `--locked` literal
/// is the load-bearing assertion per AC10.
#[test]
fn dev_setup_plans_bpf_linker_install_with_locked_when_missing() {
    let probe = ProbeContext {
        bpf_linker_on_path: false,
        nightly_toolchain_installed: true,
        rust_src_on_nightly: true,
    };

    let Plan { commands } = plan(&probe);

    let bpf_linker_cmd = commands
        .iter()
        .find(|c| c.argv.iter().any(|a| a == "bpf-linker"))
        .unwrap_or_else(|| panic!("expected a bpf-linker install command in plan: {commands:?}"));

    assert!(
        bpf_linker_cmd.argv.iter().any(|a| a == "--locked"),
        "expected `--locked` in argv per AC10, got: {:?}",
        bpf_linker_cmd.argv,
    );
    assert!(
        bpf_linker_cmd.argv.first().is_some_and(|a| a == "cargo"),
        "expected argv[0] == \"cargo\", got: {:?}",
        bpf_linker_cmd.argv,
    );
    assert!(
        bpf_linker_cmd.argv.iter().any(|a| a == "install"),
        "expected `install` in argv, got: {:?}",
        bpf_linker_cmd.argv,
    );
}

/// AC2 — when bpf-linker is already on PATH, no install command for
/// it is planned. (The handler will print a tracing::info noop and
/// the argv simply does not appear.)
#[test]
fn dev_setup_plans_no_install_when_bpf_linker_already_on_path() {
    let probe = ProbeContext {
        bpf_linker_on_path: true,
        nightly_toolchain_installed: true,
        rust_src_on_nightly: true,
    };

    let Plan { commands } = plan(&probe);

    assert!(
        !commands.iter().any(|c| c.argv.iter().any(|a| a == "bpf-linker")),
        "expected no bpf-linker install command when already on PATH, got: {commands:?}",
    );
}

/// Upstream-issue A1 — when the nightly toolchain is missing, the
/// plan installs nightly with `--component rust-src --profile minimal`
/// in one shot (mirrors the Lima YAML install at
/// `infra/lima/overdrive-dev.yaml:205`).
#[test]
fn dev_setup_plans_nightly_toolchain_install_with_rust_src_when_missing() {
    let probe = ProbeContext {
        bpf_linker_on_path: true,
        nightly_toolchain_installed: false,
        rust_src_on_nightly: false,
    };

    let Plan { commands } = plan(&probe);

    let nightly_cmd = commands
        .iter()
        .find(|c| {
            c.argv.first().is_some_and(|a| a == "rustup")
                && c.argv.iter().any(|a| a == "toolchain")
                && c.argv.iter().any(|a| a == "install")
        })
        .unwrap_or_else(|| {
            panic!("expected a `rustup toolchain install` command in plan: {commands:?}")
        });

    assert!(
        nightly_cmd.argv.iter().any(|a| a == "nightly"),
        "expected `nightly` in argv, got: {:?}",
        nightly_cmd.argv,
    );
    assert!(
        nightly_cmd.argv.iter().any(|a| a == "--component"),
        "expected `--component` in argv per A1, got: {:?}",
        nightly_cmd.argv,
    );
    assert!(
        nightly_cmd.argv.iter().any(|a| a == "rust-src"),
        "expected `rust-src` in argv per A1, got: {:?}",
        nightly_cmd.argv,
    );
    assert!(
        nightly_cmd.argv.iter().any(|a| a == "--profile"),
        "expected `--profile` in argv per A1, got: {:?}",
        nightly_cmd.argv,
    );
    assert!(
        nightly_cmd.argv.iter().any(|a| a == "minimal"),
        "expected `minimal` in argv per A1, got: {:?}",
        nightly_cmd.argv,
    );
}

/// Upstream-issue A1 — when the nightly toolchain is already
/// installed but the rust-src component is missing on it, the plan
/// uses `rustup component add rust-src --toolchain nightly` (NOT a
/// fresh toolchain install — that would re-download the entire
/// toolchain when only the component is needed).
#[test]
fn dev_setup_plans_only_rust_src_add_when_nightly_present_but_component_missing() {
    let probe = ProbeContext {
        bpf_linker_on_path: true,
        nightly_toolchain_installed: true,
        rust_src_on_nightly: false,
    };

    let Plan { commands } = plan(&probe);

    // Must have a `rustup component add rust-src --toolchain nightly`
    // shape and must NOT have a `rustup toolchain install nightly`
    // shape.
    let component_add = commands.iter().find(|c| {
        c.argv.first().is_some_and(|a| a == "rustup")
            && c.argv.iter().any(|a| a == "component")
            && c.argv.iter().any(|a| a == "add")
    });
    assert!(
        component_add.is_some(),
        "expected `rustup component add` command when only rust-src missing, got: {commands:?}",
    );
    let component_add = component_add.expect("checked above");
    assert!(
        component_add.argv.iter().any(|a| a == "rust-src"),
        "expected `rust-src` in argv, got: {:?}",
        component_add.argv,
    );
    assert!(
        component_add.argv.iter().any(|a| a == "--toolchain"),
        "expected `--toolchain` in argv, got: {:?}",
        component_add.argv,
    );
    assert!(
        component_add.argv.iter().any(|a| a == "nightly"),
        "expected `nightly` in argv, got: {:?}",
        component_add.argv,
    );

    let toolchain_install = commands.iter().find(|c| {
        c.argv.first().is_some_and(|a| a == "rustup")
            && c.argv.iter().any(|a| a == "toolchain")
            && c.argv.iter().any(|a| a == "install")
    });
    assert!(
        toolchain_install.is_none(),
        "must NOT plan a fresh toolchain install when nightly is already \
         present and only rust-src is missing, got: {commands:?}",
    );
}
