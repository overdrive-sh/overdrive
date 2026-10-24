//! `cargo xtask dev-setup` — non-Lima Linux developer toolchain
//! provisioning for `cargo xtask bpf-build`.
//!
//! Per ADR-0038 §4 / wave-decisions.md D4 / upstream-issue A1, the
//! Rust/Aya `bpf-build` path depends on three toolchain bits beyond stable
//! Rust; the approved guest TCP-correlation C/CO-RE object additionally
//! requires Clang 21, `bpftool`, libbpf headers and readable target-kernel BTF:
//!
//! 1. `bpf-linker` — installed via `cargo install --locked
//!    bpf-linker`. The `--locked` flag is mandatory for
//!    reproducibility (ADR-0038 §4).
//! 2. The pinned `BPF_NIGHTLY_TOOLCHAIN` rustup toolchain (a *dated*
//!    nightly, not the floating `nightly` channel — see that
//!    constant's doc comment for why) — required by
//!    `-Z build-std=core` per architecture.md §3.1.
//! 3. The `rust-src` component on `BPF_NIGHTLY_TOOLCHAIN` — required
//!    to build `core` from source under `build-std=core`.
//!
//! The Lima YAML at `infra/lima/overdrive-dev.yaml` provisions this complete
//! toolchain at VM creation. This module is the symmetric surface for
//! non-Lima Linux developers (a third surface alongside Lima and the
//! `bpf_build()` prerequisite checks).
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
use std::process::{Command, Stdio};

use crate::BPF_NIGHTLY_TOOLCHAIN;

/// Result of probing the host for the Rust/Aya and C/CO-RE toolchain
/// dependencies.
///
/// Populated by [`probe`] in production and constructed directly in
/// tests to drive [`plan`] through every permutation.
#[derive(Debug, Clone, Copy)]
pub struct ProbeContext {
    /// `bpf-linker` is on PATH (probed via `which::which`).
    pub bpf_linker_on_path: bool,
    /// The pinned `BPF_NIGHTLY_TOOLCHAIN` rustup toolchain is installed
    /// (probed via `rustup toolchain list`).
    pub nightly_toolchain_installed: bool,
    /// The `rust-src` component is installed on
    /// `BPF_NIGHTLY_TOOLCHAIN` (probed via `rustup component list
    /// --toolchain <BPF_NIGHTLY_TOOLCHAIN> --installed`). Meaningful
    /// only when `nightly_toolchain_installed == true`; otherwise
    /// treat as `false`.
    pub rust_src_on_nightly: bool,
    /// C/CO-RE build prerequisites.
    pub c_core: CCoreProbe,
}

/// Observed C/CO-RE compiler, tool, header and target-BTF prerequisites.
#[derive(Debug, Clone, Copy)]
pub struct CCoreProbe {
    /// Major version reported by the available C BPF compiler.
    pub clang_major: Option<u32>,
    /// `bpftool` can dump the selected target-kernel BTF as C.
    pub bpftool_on_path: bool,
    /// The libbpf CO-RE helper headers are installed.
    pub libbpf_headers_present: bool,
    /// The selected target-kernel BTF file is readable.
    pub target_btf_readable: bool,
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
/// every Rust/Aya and C/CO-RE BPF dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub commands: Vec<PlannedCommand>,
}

/// Pure planner — given a probe context, return the ordered set of
/// install and verification commands needed to bring the host up to the
/// bpf-build dependency baseline.
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

    // Step 1 — the pinned BPF nightly toolchain (`BPF_NIGHTLY_TOOLCHAIN`
    // — see that constant's doc comment for why it's a dated channel,
    // not the floating `nightly` alias). Two cases:
    // - missing entirely: `rustup toolchain install <BPF_NIGHTLY_TOOLCHAIN>
    //   --component rust-src --profile minimal` (mirrors Lima YAML
    //   line 298).
    // - present but rust-src missing: `rustup component add rust-src
    //   --toolchain <BPF_NIGHTLY_TOOLCHAIN>` (cheaper than reinstalling
    //   the full toolchain just to add a component).
    if !probe.nightly_toolchain_installed {
        commands.push(PlannedCommand {
            argv: vec![
                "rustup".into(),
                "toolchain".into(),
                "install".into(),
                BPF_NIGHTLY_TOOLCHAIN.into(),
                "--component".into(),
                "rust-src".into(),
                "--profile".into(),
                "minimal".into(),
            ],
            label: format!(
                "rustup toolchain install {BPF_NIGHTLY_TOOLCHAIN} --component rust-src --profile minimal"
            ),
        });
    } else if !probe.rust_src_on_nightly {
        commands.push(PlannedCommand {
            argv: vec![
                "rustup".into(),
                "component".into(),
                "add".into(),
                "rust-src".into(),
                "--toolchain".into(),
                BPF_NIGHTLY_TOOLCHAIN.into(),
            ],
            label: format!("rustup component add rust-src --toolchain {BPF_NIGHTLY_TOOLCHAIN}"),
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

    let c_core = probe.c_core;
    if c_core.clang_major != Some(21) || !c_core.bpftool_on_path || !c_core.libbpf_headers_present {
        let script = concat!(
            "set -euo pipefail; ",
            "wget -qO /tmp/llvm.sh https://apt.llvm.org/llvm.sh; ",
            "chmod +x /tmp/llvm.sh; ",
            "sudo bash /tmp/llvm.sh 21; ",
            "sudo apt-get update; ",
            "sudo apt-get install -y --no-install-recommends clang-21 libbpf-dev; ",
            "BPFTOOL_CANDIDATE=\"$(apt-cache policy bpftool | awk '$1 == \"Candidate:\" {print $2}')\"; ",
            "if [ -n \"$BPFTOOL_CANDIDATE\" ] && [ \"$BPFTOOL_CANDIDATE\" != \"(none)\" ]; then ",
            "sudo apt-get install -y --no-install-recommends bpftool; ",
            "else sudo apt-get install -y --no-install-recommends ",
            "linux-tools-common linux-tools-generic; ",
            "BPFTOOL_BINARY=\"$(find /usr/lib -path '/usr/lib/linux-tools-*/bpftool' ",
            "-type f -executable -print -quit)\"; ",
            "if [ -z \"$BPFTOOL_BINARY\" ]; then ",
            "echo 'linux-tools-generic did not install an executable bpftool' >&2; exit 1; fi; ",
            "sudo install -m 0755 \"$BPFTOOL_BINARY\" /usr/local/bin/bpftool; fi; ",
            "command -v bpftool; ",
            "bpftool version; ",
            "BTF=\"${OVERDRIVE_BPF_TARGET_BTF:-/sys/kernel/btf/vmlinux}\"; ",
            "test -r \"$BTF\"; ",
            "bpftool btf dump file \"$BTF\" format c >/dev/null"
        );
        commands.push(PlannedCommand {
            argv: vec!["bash".into(), "-c".into(), script.into()],
            label: "install Clang 21, bpftool and libbpf headers".into(),
        });
    }
    if !c_core.target_btf_readable {
        commands.push(PlannedCommand {
            argv: vec![
                "bash".into(),
                "-c".into(),
                "test -r \"${OVERDRIVE_BPF_TARGET_BTF:-/sys/kernel/btf/vmlinux}\"".into(),
            ],
            label: "verify readable target-kernel BTF at /sys/kernel/btf/vmlinux".into(),
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
    // line. The *pinned* BPF nightly appears as
    // `<BPF_NIGHTLY_TOOLCHAIN>-<host-triple>` (e.g.
    // `nightly-2026-08-05-x86_64-unknown-linux-gnu`); a prefix match on
    // `BPF_NIGHTLY_TOOLCHAIN` is robust across architectures. This is
    // deliberately a prefix match against the *dated* channel, not a
    // generic `nightly-` match — the floating `nightly` alias having a
    // newer, LLVM-incompatible build installed must NOT be reported as
    // "satisfied" here (see `BPF_NIGHTLY_TOOLCHAIN`'s doc comment).
    let nightly_toolchain_installed = Command::new("rustup")
        .args(["toolchain", "list"])
        .output()
        .ok()
        .and_then(|o| if o.status.success() { Some(o.stdout) } else { None })
        .is_some_and(|stdout| {
            String::from_utf8_lossy(&stdout)
                .lines()
                .any(|line| line.trim().starts_with(BPF_NIGHTLY_TOOLCHAIN))
        });

    let rust_src_on_nightly = if nightly_toolchain_installed {
        Command::new("rustup")
            .args(["component", "list", "--toolchain", BPF_NIGHTLY_TOOLCHAIN, "--installed"])
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

    let clang_major = ["clang-21", "clang"].into_iter().find_map(|binary| {
        Command::new(binary)
            .arg("--version")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .split(|character: char| !character.is_ascii_digit())
                    .find_map(|word| word.parse().ok())
            })
    });
    let target_btf = std::env::var_os("OVERDRIVE_BPF_TARGET_BTF")
        .map_or_else(|| std::path::PathBuf::from("/sys/kernel/btf/vmlinux"), Into::into);
    let target_btf_readable = std::fs::File::open(&target_btf).is_ok();
    let bpftool_on_path = target_btf_readable
        && Command::new("bpftool")
            .args(["btf", "dump", "file"])
            .arg(&target_btf)
            .args(["format", "c"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
    let libbpf_headers_present = std::path::Path::new("/usr/include/bpf/bpf_helpers.h").is_file();

    Ok(ProbeContext {
        bpf_linker_on_path,
        nightly_toolchain_installed,
        rust_src_on_nightly,
        c_core: CCoreProbe {
            clang_major,
            bpftool_on_path,
            libbpf_headers_present,
            target_btf_readable,
        },
    })
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
