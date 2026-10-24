//! Asserts `cargo xtask dev-setup` plans the correct install argv for
//! the Rust/Aya and C/CO-RE BPF build dependencies,
//! across the four `ProbeContext` shapes that arise in practice.
//!
//! Per ADR-0038 §4 (and upstream-issue A1) the dev-setup task is the
//! non-Lima Linux developer surface for installing exactly the same
//! toolchain bits the Lima YAML provisions in `infra/lima/overdrive-dev.yaml`:
//!
//! - `bpf-linker` via `cargo install --locked bpf-linker`
//! - the pinned `BPF_NIGHTLY_TOOLCHAIN` rustup toolchain via `rustup
//!   toolchain install <BPF_NIGHTLY_TOOLCHAIN> --component rust-src
//!   --profile minimal` — a dated nightly, not the floating `nightly`
//!   channel; see that constant's doc comment for why.
//! - `rust-src` component on it via `rustup component add rust-src
//!   --toolchain <BPF_NIGHTLY_TOOLCHAIN>` (when the toolchain is
//!   already present but the component is missing).
//!
//! The four tests cover the four Rust-toolchain planning permutations. The
//! first also keeps the accepted C/CO-RE requirements present on dev setup,
//! Lima and CI so none of the three build surfaces silently drops one.
//!
//! 1. bpf-linker missing → planned argv contains `--locked` (AC10).
//! 2. bpf-linker present → no install command planned (AC2).
//! 3. nightly toolchain missing → fresh-toolchain install argv shape.
//! 4. nightly toolchain present, rust-src component missing → component-add
//!    argv shape (NOT a fresh toolchain install).
//!
//! The three original Rust-toolchain rows remain pure planner assertions. The
//! missing-prerequisite row also drives the real `xtask bpf-build`,
//! `guest-kit-prepare` and `overdrive-init` build-script boundaries in a
//! temporary workspace with fake compilers, then decodes the effective Lima
//! and CI command blocks. The fixture performs no real install or kernel
//! effect.

#![allow(clippy::doc_markdown, reason = "repository-mandated CONTRACT_SHAPE marker")]

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use xtask::BPF_NIGHTLY_TOOLCHAIN;
use xtask::dev_setup::{CCoreProbe, Plan, ProbeContext, plan};

fn write_executable(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write executable fixture");
    let mut permissions = std::fs::metadata(path).expect("stat fixture").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("chmod fixture");
}

fn path_with(fake_bin: &Path) -> std::ffi::OsString {
    let inherited = std::env::var_os("PATH").expect("PATH");
    let paths = std::iter::once(fake_bin.to_path_buf()).chain(std::env::split_paths(&inherited));
    std::env::join_paths(paths).expect("fixture PATH")
}

fn yaml_mapping_section(document: &str, key: &str, indent: usize) -> String {
    let marker = format!("{key}:");
    let mut inside = false;
    let mut section = String::new();
    for line in document.lines() {
        let trimmed = line.trim_start();
        let line_indent = line.len() - trimmed.len();
        if !inside {
            if line_indent == indent && trimmed == marker {
                inside = true;
            }
            continue;
        }
        if !trimmed.is_empty() && !trimmed.starts_with('#') && line_indent <= indent {
            break;
        }
        section.push_str(line);
        section.push('\n');
    }
    assert!(inside, "YAML mapping has no {key:?} at indentation {indent}");
    section
}

fn effective_shell_commands(section: &str) -> Vec<String> {
    let mut physical = Vec::new();
    let lines: Vec<&str> = section.lines().collect();
    let mut at = 0usize;
    while at < lines.len() {
        let line = lines[at];
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let value = trimmed.strip_prefix("script:").or_else(|| trimmed.strip_prefix("run:"));
        let Some(value) = value else {
            at += 1;
            continue;
        };
        let value = value.trim();
        if !value.is_empty() && value != "|" && value != ">" {
            physical.push(value.to_owned());
            at += 1;
            continue;
        }
        at += 1;
        while at < lines.len() {
            let nested = lines[at];
            let nested_trimmed = nested.trim_start();
            let nested_indent = nested.len() - nested_trimmed.len();
            if !nested_trimmed.is_empty() && nested_indent <= indent {
                break;
            }
            if !nested_trimmed.is_empty() && !nested_trimmed.starts_with('#') {
                physical.push(nested_trimmed.to_owned());
            }
            at += 1;
        }
    }

    let mut commands = Vec::new();
    let mut continued = String::new();
    for line in physical {
        let continuation = line.ends_with('\\');
        let fragment = line.trim_end_matches('\\').trim();
        if !continued.is_empty() {
            continued.push(' ');
        }
        continued.push_str(fragment);
        if !continuation {
            commands.push(std::mem::take(&mut continued));
        }
    }
    if !continued.is_empty() {
        commands.push(continued);
    }
    commands
}

fn assert_c_core_provisioning(surface: &str, commands: &[String]) {
    let rendered = commands.join("\n");
    let installs = commands
        .iter()
        .filter(|command| command.contains(" install "))
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(installs.contains("libbpf"), "{surface} does not install libbpf headers: {commands:?}");
    assert!(
        rendered.contains("apt-cache policy bpftool")
            && rendered.contains("Candidate:")
            && rendered.contains("(none)")
            && installs.contains("bpftool")
            && installs.contains("linux-tools-common")
            && installs.contains("linux-tools-generic")
            && rendered.contains("/usr/lib/linux-tools-*/bpftool")
            && rendered.contains("-type f -executable")
            && rendered.contains("/usr/local/bin/bpftool")
            && rendered.contains("command -v bpftool")
            && rendered.contains("bpftool version"),
        "{surface} does not resolve concrete versus virtual bpftool packaging: {commands:?}"
    );
    assert!(
        installs.contains("clang-21")
            || commands.iter().any(|command| {
                command.contains("clang") && command.contains("--version") && command.contains("21")
            }),
        "{surface} does not install or validate Clang major 21: {commands:?}"
    );
    assert!(
        rendered.contains("OVERDRIVE_BPF_TARGET_BTF")
            && rendered.contains("/sys/kernel/btf/vmlinux")
            && rendered.contains("test -r")
            && rendered.contains("bpftool btf dump file")
            && rendered.contains("format c"),
        "{surface} does not validate and dump the declared target-kernel BTF: {commands:?}"
    );
}

#[derive(Debug, Clone, Copy)]
enum MissingCPrerequisite {
    Clang,
    WrongClangMajor,
    Bpftool,
    LibbpfHeaders,
    TargetBtf,
}

fn assert_missing_c_prerequisite_is_planned(
    missing: MissingCPrerequisite,
    commands: &[xtask::dev_setup::PlannedCommand],
) {
    let rendered =
        commands.iter().map(|command| command.argv.join(" ")).collect::<Vec<_>>().join("\n");
    let handled = match missing {
        MissingCPrerequisite::Clang | MissingCPrerequisite::WrongClangMajor => {
            rendered.lines().any(|line| {
                (line.contains(" install ") && line.contains("clang") && line.contains("21"))
                    || (line.contains("clang") && line.contains("--version") && line.contains("21"))
            })
        }
        MissingCPrerequisite::Bpftool => {
            rendered.contains("apt-cache policy bpftool")
                && rendered.contains("Candidate:")
                && rendered.contains("(none)")
                && rendered.contains(" install ")
                && rendered.contains("bpftool")
                && rendered.contains("linux-tools-common")
                && rendered.contains("linux-tools-generic")
                && rendered.contains("/usr/lib/linux-tools-*/bpftool")
                && rendered.contains("-type f -executable")
                && rendered.contains("/usr/local/bin/bpftool")
                && rendered.contains("command -v bpftool")
                && rendered.contains("bpftool version")
                && rendered.contains("bpftool btf dump file")
                && rendered.contains("OVERDRIVE_BPF_TARGET_BTF")
        }
        MissingCPrerequisite::LibbpfHeaders => {
            rendered.lines().any(|line| line.contains(" install ") && line.contains("libbpf"))
        }
        MissingCPrerequisite::TargetBtf => {
            rendered.contains("OVERDRIVE_BPF_TARGET_BTF")
                && rendered.contains("/sys/kernel/btf/vmlinux")
                && rendered.contains("test -r")
        }
    };
    assert!(handled, "{missing:?} is not independently handled by the plan: {commands:?}");
}

fn assert_provisioning_surfaces() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
    let lima = std::fs::read_to_string(workspace.join("infra/lima/overdrive-dev.yaml"))
        .expect("read Lima config");
    let ci = std::fs::read_to_string(workspace.join(".github/workflows/ci.yml"))
        .expect("read CI config");
    let lima_commands = effective_shell_commands(&yaml_mapping_section(&lima, "provision", 0));
    let jobs = yaml_mapping_section(&ci, "jobs", 0);
    let ci_commands = effective_shell_commands(&yaml_mapping_section(&jobs, "bpf-build", 2));
    assert_c_core_provisioning("Lima provisioning", &lima_commands);
    assert_c_core_provisioning("CI bpf-build job", &ci_commands);
}

struct BpfBuildFixture {
    _temp: TempDir,
    root: std::path::PathBuf,
    fake_bin: std::path::PathBuf,
    target_dir: std::path::PathBuf,
    command_log: std::path::PathBuf,
    target_btf: std::path::PathBuf,
}

impl BpfBuildFixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("temporary bpf-build workspace");
        let root = temp.path().to_path_buf();
        let fake_bin = root.join("bin");
        let target_dir = root.join("cargo-target");
        let command_log = root.join("commands.log");
        std::fs::create_dir_all(&fake_bin).expect("create fake command directory");
        std::fs::create_dir_all(root.join("crates/overdrive-bpf/src"))
            .expect("create fake BPF crate");
        std::fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = []\nresolver = \"2\"\n")
            .expect("write fixture workspace");
        std::fs::write(
            root.join("crates/overdrive-bpf/src/tcp_correlation_candidate.bpf.c"),
            "int tcp_correlation_candidate(void) { return 0; }\n",
        )
        .expect("write candidate source fixture");
        let target_btf = root.join("vmlinux.btf");
        std::fs::write(&target_btf, b"btf").expect("write target BTF fixture");
        write_executable(&fake_bin.join("bpf-linker"), "#!/usr/bin/env bash\nexit 0\n");
        write_executable(&fake_bin.join("rustup"), include_str!("fixtures/fake_bpf_rustup.sh"));
        write_executable(&fake_bin.join("bpftool"), include_str!("fixtures/fake_bpftool.sh"));
        let clang = include_str!("fixtures/fake_clang_21.sh");
        write_executable(&fake_bin.join("clang"), clang);
        write_executable(&fake_bin.join("clang-21"), clang);
        Self { _temp: temp, root, fake_bin, target_dir, command_log, target_btf }
    }

    fn run(&self, subcommand: &str) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .arg(subcommand)
            .current_dir(&self.root)
            .env("PATH", path_with(&self.fake_bin))
            .env("CARGO_TARGET_DIR", &self.target_dir)
            .env("OVERDRIVE_BPF_NATIVE", "1")
            .env("OVERDRIVE_BPF_TARGET_BTF", &self.target_btf)
            .env("OVERDRIVE_FAKE_COMMAND_LOG", &self.command_log)
            .output()
            .unwrap_or_else(|error| panic!("run xtask {subcommand}: {error}"))
    }

    fn stable(&self) -> std::path::PathBuf {
        self.root.join("target/bpf")
    }
}

fn assert_bpf_build_surface(fixture: &BpfBuildFixture) -> std::path::PathBuf {
    let built = fixture.run("bpf-build");
    assert!(built.status.success(), "bpf-build failed: {}", String::from_utf8_lossy(&built.stderr));
    let stable = fixture.stable();
    assert_eq!(std::fs::read(stable.join("overdrive_bpf.o")).expect("host object"), b"host");
    assert_eq!(
        std::fs::read(stable.join("overdrive_guest_bpf.o")).expect("guest object"),
        b"guest"
    );
    let candidate = stable.join("overdrive_guest_tcp_correlation_candidate_bpf.o");
    assert_eq!(std::fs::read(&candidate).expect("candidate object"), b"candidate");
    let invocations = std::fs::read_to_string(&fixture.command_log).expect("command log");
    assert!(
        invocations.lines().any(|line| {
            line.starts_with("bpftool\tbtf\tdump\tfile\t")
                && line.contains(fixture.target_btf.to_string_lossy().as_ref())
                && line.ends_with("\tformat\tc")
        }),
        "bpf-build did not generate vmlinux.h from target BTF: {invocations}"
    );
    let target_arch = match std::env::consts::ARCH {
        "x86_64" => "-D__TARGET_ARCH_x86",
        "aarch64" => "-D__TARGET_ARCH_arm64",
        other => panic!("unsupported test host architecture {other}"),
    };
    let clang = invocations
        .lines()
        .find(|line| line.starts_with("clang\t"))
        .unwrap_or_else(|| panic!("bpf-build did not invoke Clang: {invocations}"));
    let argv = clang.split('\t').skip(1).collect::<Vec<_>>();
    for required in ["-g", "-O2", target_arch] {
        assert!(argv.contains(&required), "Clang argv has no exact {required:?} token: {argv:?}");
    }
    let target = argv.iter().position(|argument| *argument == "-target");
    assert_eq!(
        target.and_then(|at| argv.get(at + 1)).copied(),
        Some("bpf"),
        "Clang argv must contain adjacent `-target bpf`: {argv:?}"
    );
    assert!(
        argv.iter().any(|argument| argument.ends_with("tcp_correlation_candidate.bpf.c")),
        "Clang argv does not name the candidate C source: {argv:?}"
    );
    candidate
}

fn assert_guest_kit_runs_bpf_build_first(fixture: &BpfBuildFixture, candidate: &std::path::Path) {
    std::fs::remove_file(candidate).expect("remove candidate before guest-kit composition check");
    std::fs::write(&fixture.command_log, []).expect("clear command log");
    let prepared = fixture.run("guest-kit-prepare");
    assert!(
        candidate.is_file(),
        "guest-kit preparation did not run candidate bpf-build before its later failure: {}",
        String::from_utf8_lossy(&prepared.stderr)
    );
}

/// AC1, AC3, AC10 — when bpf-linker is absent the plan contains a
/// `cargo install --locked bpf-linker` command. The `--locked` literal
/// is the load-bearing assertion per AC10.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn dev_setup_plans_bpf_linker_install_with_locked_when_missing() {
    let probe = ProbeContext {
        bpf_linker_on_path: false,
        nightly_toolchain_installed: true,
        rust_src_on_nightly: true,
        c_core: CCoreProbe {
            clang_major: Some(21),
            bpftool_on_path: true,
            libbpf_headers_present: true,
            target_btf_readable: true,
        },
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

    let complete = CCoreProbe {
        clang_major: Some(21),
        bpftool_on_path: true,
        libbpf_headers_present: true,
        target_btf_readable: true,
    };
    let cases = [
        (MissingCPrerequisite::Clang, CCoreProbe { clang_major: None, ..complete }),
        (MissingCPrerequisite::WrongClangMajor, CCoreProbe { clang_major: Some(20), ..complete }),
        (MissingCPrerequisite::Bpftool, CCoreProbe { bpftool_on_path: false, ..complete }),
        (
            MissingCPrerequisite::LibbpfHeaders,
            CCoreProbe { libbpf_headers_present: false, ..complete },
        ),
        (MissingCPrerequisite::TargetBtf, CCoreProbe { target_btf_readable: false, ..complete }),
    ];
    for (missing, c_core) in cases {
        let Plan { commands } = plan(&ProbeContext {
            bpf_linker_on_path: true,
            nightly_toolchain_installed: true,
            rust_src_on_nightly: true,
            c_core,
        });
        assert!(
            !commands.iter().any(|command| command.argv.iter().any(|arg| arg == "bpf-linker")),
            "{missing:?} handling must not depend on reinstalling present bpf-linker: {commands:?}"
        );
        assert_missing_c_prerequisite_is_planned(missing, &commands);
    }

    assert_provisioning_surfaces();
    let fixture = BpfBuildFixture::new();
    let candidate = assert_bpf_build_surface(&fixture);
    assert_guest_kit_runs_bpf_build_first(&fixture, &candidate);
}

/// AC2 — when bpf-linker is already on PATH, no install command for
/// it is planned. (The handler will print a `tracing::info` noop and
/// the argv simply does not appear.)
#[test]
fn dev_setup_plans_no_install_when_bpf_linker_already_on_path() {
    let probe = ProbeContext {
        bpf_linker_on_path: true,
        nightly_toolchain_installed: true,
        rust_src_on_nightly: true,
        c_core: CCoreProbe {
            clang_major: Some(21),
            bpftool_on_path: true,
            libbpf_headers_present: true,
            target_btf_readable: true,
        },
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
        c_core: CCoreProbe {
            clang_major: Some(21),
            bpftool_on_path: true,
            libbpf_headers_present: true,
            target_btf_readable: true,
        },
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
        nightly_cmd.argv.iter().any(|a| a == BPF_NIGHTLY_TOOLCHAIN),
        "expected `{BPF_NIGHTLY_TOOLCHAIN}` in argv, got: {:?}",
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
        c_core: CCoreProbe {
            clang_major: Some(21),
            bpftool_on_path: true,
            libbpf_headers_present: true,
            target_btf_readable: true,
        },
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
        component_add.argv.iter().any(|a| a == BPF_NIGHTLY_TOOLCHAIN),
        "expected `{BPF_NIGHTLY_TOOLCHAIN}` in argv, got: {:?}",
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
