//! `cargo xtask dev-setup` — non-Lima Linux developer toolchain
//! provisioning for `cargo xtask bpf-build`.
//!
//! Per ADR-0038 §4 / wave-decisions.md D4 / upstream-issue A1, the
//! `bpf-build` task depends on three toolchain bits beyond stable
//! Rust:
//!
//! 1. `bpf-linker` — installed via `cargo install --locked
//!    bpf-linker`. The `--locked` flag is mandatory for
//!    reproducibility (ADR-0038 §4).
//! 2. The `nightly` rustup toolchain — required by
//!    `-Z build-std=core` per architecture.md §3.1.
//! 3. The `rust-src` component on the `nightly` toolchain — required
//!    to build `core` from source under `build-std=core`.
//!
//! The Lima YAML at `infra/lima/overdrive-dev.yaml:204-206` provisions
//! all three at VM creation. This module is the symmetric surface for
//! non-Lima Linux developers (a third surface alongside Lima and the
//! `which_or_hint` runtime check inside `bpf_build()`).
//!
//! ## Architecture
//!
//! Implementation is a two-phase pure planner / impure executor split
//! so the test surface (`xtask/tests/integration/dev_setup_bpf_linker.rs`)
//! can assert on the planned argv shapes without spawning processes.
//! Per AC10 the `--locked` literal in the bpf-linker install argv is
//! load-bearing and tested directly against the planner.
//!
//! ```text
//! probe()  →  ProbeContext   (impure: which / rustup queries)
//! plan()   →  Plan           (pure: argv construction)
//! execute() →  ()            (impure: process spawning)
//! ```
//!
//! ## macOS short-circuit
//!
//! Per AC7 the dev-setup task short-circuits on macOS with a
//! best-effort stderr notice (`eprintln!`) — `bpf-linker` is
//! Linux-only (architecture.md §5) and the macOS dev path is
//! `cargo nextest run --no-run` / `cargo xtask lima run --` for the
//! integration suite.

#![allow(clippy::expect_used)]

use color_eyre::eyre::{Result, eyre};
use std::process::Command;

/// Result of probing the host for the three toolchain dependencies.
///
/// Populated by [`probe`] in production and constructed directly in
/// tests to drive [`plan`] through every permutation.
#[derive(Debug, Clone, Copy)]
pub struct ProbeContext {
    /// `bpf-linker` is on PATH (probed via `which::which`).
    pub bpf_linker_on_path: bool,
    /// The `nightly` rustup toolchain is installed (probed via
    /// `rustup toolchain list`).
    pub nightly_toolchain_installed: bool,
    /// The `rust-src` component is installed on the `nightly`
    /// toolchain (probed via
    /// `rustup component list --toolchain nightly --installed`).
    /// Meaningful only when `nightly_toolchain_installed == true`;
    /// otherwise treat as `false`.
    pub rust_src_on_nightly: bool,
}

/// One planned command — argv form so tests assert on shape without
/// spawning. argv\[0\] is the binary name (resolved via PATH at
/// execute time, same as `Command::new(...)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedCommand {
    pub argv: Vec<String>,
    /// Human-readable label printed before invoking, mirrors the
    /// `sh(label, cmd)` pattern used elsewhere in this crate.
    pub label: String,
}

/// Ordered set of commands to invoke. Empty when the host already has
/// every dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub commands: Vec<PlannedCommand>,
}

/// Pure planner — given a probe context, return the ordered set of
/// install commands needed to bring the host up to the bpf-build
/// dependency baseline.
///
/// Order matters: install the nightly toolchain before any component
/// adds (component-add against a missing toolchain would fail).
/// `bpf-linker` is independent so its position is convention only —
/// last, so a partial run still gets the rustup state right.
#[must_use]
pub fn plan(probe: &ProbeContext) -> Plan {
    #![allow(clippy::trivially_copy_pass_by_ref)] // by-ref keeps test
    // call sites readable as `plan(&ProbeContext { ... })`.
    let mut commands = Vec::new();

    // Step 1 — nightly toolchain. Two cases:
    // - missing entirely: `rustup toolchain install nightly --component
    //   rust-src --profile minimal` (mirrors Lima YAML line 205).
    // - present but rust-src missing: `rustup component add rust-src
    //   --toolchain nightly` (cheaper than reinstalling the full
    //   toolchain just to add a component).
    if !probe.nightly_toolchain_installed {
        commands.push(PlannedCommand {
            argv: vec![
                "rustup".into(),
                "toolchain".into(),
                "install".into(),
                "nightly".into(),
                "--component".into(),
                "rust-src".into(),
                "--profile".into(),
                "minimal".into(),
            ],
            label: "rustup toolchain install nightly --component rust-src --profile minimal".into(),
        });
    } else if !probe.rust_src_on_nightly {
        commands.push(PlannedCommand {
            argv: vec![
                "rustup".into(),
                "component".into(),
                "add".into(),
                "rust-src".into(),
                "--toolchain".into(),
                "nightly".into(),
            ],
            label: "rustup component add rust-src --toolchain nightly".into(),
        });
    }

    // Step 2 — bpf-linker via `cargo install --locked bpf-linker`.
    // `--locked` is mandatory per ADR-0038 §4 / AC3 / AC10.
    if !probe.bpf_linker_on_path {
        commands.push(PlannedCommand {
            argv: vec!["cargo".into(), "install".into(), "--locked".into(), "bpf-linker".into()],
            label: "cargo install --locked bpf-linker".into(),
        });
    }

    Plan { commands }
}

/// Impure probe — query the host for the three toolchain
/// dependencies. Wraps `sh -c "command -v ..."` and
/// `rustup toolchain list` / `rustup component list ...` shells.
///
/// Failures of the probe commands themselves (e.g. `rustup` not on
/// PATH at all) are surfaced as `Ok(ProbeContext { ..., installed:
/// false, ... })` — the plan will then attempt a fresh install which
/// will itself fail with a clearer error if the prerequisite (rustup)
/// is genuinely absent. This matches the existing `which_or_hint`
/// pattern in `main.rs`.
fn probe() -> Result<ProbeContext> {
    let bpf_linker_on_path = Command::new("sh")
        .arg("-c")
        .arg("command -v bpf-linker")
        .status()
        .is_ok_and(|s| s.success());

    // `rustup toolchain list` lists installed toolchains, one per
    // line. The nightly toolchain appears as `nightly-<host-triple>`
    // (e.g. `nightly-x86_64-unknown-linux-gnu`); a substring match on
    // `nightly-` is robust across architectures.
    let nightly_toolchain_installed = Command::new("rustup")
        .args(["toolchain", "list"])
        .output()
        .ok()
        .and_then(|o| if o.status.success() { Some(o.stdout) } else { None })
        .is_some_and(|stdout| {
            String::from_utf8_lossy(&stdout)
                .lines()
                .any(|line| line.trim().starts_with("nightly-") || line.trim() == "nightly")
        });

    let rust_src_on_nightly = if nightly_toolchain_installed {
        Command::new("rustup")
            .args(["component", "list", "--toolchain", "nightly", "--installed"])
            .output()
            .ok()
            .and_then(|o| if o.status.success() { Some(o.stdout) } else { None })
            .is_some_and(|stdout| {
                String::from_utf8_lossy(&stdout)
                    .lines()
                    .any(|line| line.trim().starts_with("rust-src"))
            })
    } else {
        false
    };

    Ok(ProbeContext { bpf_linker_on_path, nightly_toolchain_installed, rust_src_on_nightly })
}

/// Execute one planned command as a child process. Failures surface
/// as a structured `eyre::Report` per AC4.
fn execute_one(cmd: &PlannedCommand) -> Result<()> {
    eprintln!("xtask dev-setup: running {}", cmd.label);
    let (head, tail) = cmd
        .argv
        .split_first()
        .ok_or_else(|| eyre!("internal error: PlannedCommand with empty argv"))?;
    let status = Command::new(head)
        .args(tail)
        .status()
        .map_err(|e| eyre!("failed to spawn `{}`: {e}", cmd.label))?;
    if !status.success() {
        return Err(eyre!("`{}` failed with {status}", cmd.label));
    }
    Ok(())
}

/// Top-level entry point — composes probe + plan + execute.
///
/// Per AC2 prints a noop trace and exits 0 when every dependency is
/// already satisfied. Per AC7 short-circuits on non-Linux with a
/// best-effort stderr notice (`eprintln!`).
///
/// # Errors
///
/// Returns a structured `eyre::Report` when any of the planned
/// install commands exit non-zero (AC4) or when the underlying
/// process spawn fails.
pub fn run() -> Result<()> {
    if !cfg!(target_os = "linux") {
        eprintln!(
            "xtask dev-setup: bpf-linker is Linux-only; macOS dev path is \
             `cargo check --no-run` / `cargo xtask lima run -- ...` for the \
             integration suite. Skipping install."
        );
        return Ok(());
    }

    let probe = probe()?;
    let Plan { commands } = plan(&probe);

    if commands.is_empty() {
        eprintln!(
            "xtask dev-setup: bpf-linker, nightly toolchain, and rust-src \
             component all present — nothing to install."
        );
        return Ok(());
    }

    for cmd in &commands {
        execute_one(cmd)?;
    }

    eprintln!("xtask dev-setup: bpf-build toolchain provisioning complete.");
    Ok(())
}
