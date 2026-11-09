//! `cargo xtask guest-kit-prepare` — the lane setup step of the guest-kit
//! native-metal lanes (GH #295, T-1; EVMOD-GUEST-KIT, ADR-0206).
//!
//! The guest-kit fixture (`overdrive_testing::guest_kit`, `guest_lane`)
//! prepares the artifacts the guest-kit fixture and ordinary Metal nextest
//! dispatcher consume: the forwarder module built for the running kernel, the
//! static scripted peer, the static production init binary, the in-guest
//! libtest binaries, and the host-side dispatcher. Building these before a
//! selected attempt is not the test body's job: an on-demand `cargo` build
//! inside nextest's per-attempt timeout can expire before the guest starts.
//! The lane runs this step first, as its declared setup, and the fixture
//! verifies each artifact at the same target-scoped path this step leaves:
//!
//! ```text
//! cargo xtask metal run -- cargo xtask guest-kit-prepare
//! cargo xtask metal run -- cargo nextest run … --features integration-tests,kvm-tests
//! ```
//!
//! The step is re-run before every lane invocation: it is incremental (each
//! build is a no-op when its inputs are unchanged), and it is what keeps the
//! staged artefacts current with the tree under test.
//!
//! **The artefact paths are a contract with `overdrive-testing`**, which
//! xtask cannot depend on (`.claude/rules/development.md` § "xtask is build
//! / test / dev orchestration"). Both sides spell them identically; a change
//! to one is a change to both:
//!
//! | Artefact | Path | Fixture reader |
//! |---|---|---|
//! | `ovd_fwd.ko` for the running kernel | `<workspace>/target/kmod/<release>/ovd_fwd.ko` (EVMOD-BUILD; workspace-scoped, never the shared cargo target dir; `$OVERDRIVE_KMOD_DIR` in a mutation run) | `forwarder_module::require_forwarder_module`, `guest_kit::GuestKit::provision` |
//! | the static scripted peer | `<musl-triple>/release/overdrive-guest-peer` | `guest_kit::GuestKit::provision` |
//! | the static production init binary | `<target>/<musl-triple>/release/overdrive-init` | `vm_fixture::build_overdrive_init_static` |
//! | an in-guest libtest binary | `guest-kit/in-guest/<package>--<bin>` | `guest_lane::InGuestTests::prepared` |
//! | the nextest guest dispatcher | `<target>/debug/overdrive-fwd-guest-suite` | `OVERDRIVE_METAL_TEST_DISPATCHER` |
//! | the existing Quint CLI runtime closure | `<target>/guest-kit/quint-test-runtime` | the exact active Gateway conformance-test placement |

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use color_eyre::eyre::{Result, WrapErr, bail, eyre};

/// The name of the scripted peer's binary target in `overdrive-testing`.
pub const GUEST_PEER_BIN: &str = "overdrive-guest-peer";
/// The prepared host-side dispatcher used by the nextest run-wrapper.
const GUEST_SUITE_BIN: &str = "overdrive-fwd-guest-suite";

/// Which cargo target of a package an in-guest lane's libtest binary is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InGuestTarget {
    /// A package library's unit-test binary (`cargo test --lib`).
    Lib,
    /// A `bin` target's unit tests (`cargo test --bin <name>`).
    Bin,
    /// An integration-test target (`cargo test --test <name>`).
    Test,
}

impl InGuestTarget {
    /// The `cargo test` selector flag.
    #[must_use]
    pub const fn flag(self) -> &'static str {
        match self {
            Self::Lib => "--lib",
            Self::Bin => "--bin",
            Self::Test => "--test",
        }
    }
}

/// One in-guest lane: a package's libtest binary built with `features` and
/// staged for the guest-kit image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InGuestLane {
    /// The package.
    pub package: &'static str,
    /// The target whose tests run in the guest: a `bin` target or an
    /// integration-test target, named `bin`.
    pub bin: &'static str,
    /// Which kind of target `bin` names.
    pub target: InGuestTarget,
    /// The features the binary is built with.
    pub features: &'static str,
}

/// Every in-guest lane this step stages.
///
/// - `overdrive-init`'s unit tests: the in-guest transport and guest-flow
///   cells (T-2, D-22).
/// - `overdrive-fwd`'s `integration` test binary: V-EV-UNLOAD's ULP negative
///   control and unload stress (U-EV-UNLOAD, T-1), which unload the module
///   and so run only in a guest whose PID 1 holds nothing of it.
/// - `overdrive-dataplane`'s `integration` test binary: the existing seven
///   R5-54 inbound guest-relay cells plus the 36 active real-kernel dataplane
///   bodies independently routed out of the normal three-package selector,
///   executed by the existing `mtls-relay` guest so their BPF, netns, nft,
///   cgroup, XDP and kTLS effects stay inside one disposable guest.
/// - `overdrive-dataplane`'s library test binary: the three existing real-BPF
///   map bodies discovered by the normal package gate, staged beside the
///   integration binary and executed in the same `mtls-relay` guest.
/// - `overdrive-control-plane`'s `acceptance` and `integration` test binaries,
///   `overdrive-cli`'s `integration` test binary, and `overdrive-worker`'s
///   `integration` test binary: the 57 retained real-cgroup cells (including
///   E23's allocation-scoped refused-thread kill write and the existing
///   forwarded-outbound mTLS cell) plus the 18 already-authored STEP 13-04 and five
///   already-authored STEP 13-05 native cells, followed by the 20 existing
///   control-plane native bodies discovered by the normal package gate;
///   executed by the existing guest-suite runner inside the same three fresh
///   disposable guests with a private cgroup-v2 mount. The two kvm-gated
///   integration binaries also stage STEP 14-01's and STEP 14-02's
///   already-authored production serve + deploy cells for nested execution
///   inside that disposable boundary. The worker's standalone `leg_f_repair`
///   binary stages STEP 14-02's host-kernel-shared cell in the outer guest.
pub const IN_GUEST_LANES: &[InGuestLane] = &[
    InGuestLane {
        package: "overdrive-init",
        bin: "overdrive-init",
        target: InGuestTarget::Bin,
        features: "integration-tests,kvm-tests",
    },
    InGuestLane {
        package: "overdrive-fwd",
        bin: "integration",
        target: InGuestTarget::Test,
        features: "integration-tests,kvm-tests",
    },
    InGuestLane {
        package: "overdrive-dataplane",
        bin: "integration",
        target: InGuestTarget::Test,
        features: "integration-tests",
    },
    InGuestLane {
        package: "overdrive-dataplane",
        bin: "overdrive_dataplane",
        target: InGuestTarget::Lib,
        features: "integration-tests",
    },
    InGuestLane {
        package: "overdrive-control-plane",
        bin: "acceptance",
        target: InGuestTarget::Test,
        features: "integration-tests",
    },
    InGuestLane {
        package: "overdrive-control-plane",
        bin: "integration",
        target: InGuestTarget::Test,
        features: "integration-tests,kvm-tests",
    },
    InGuestLane {
        package: "overdrive-cli",
        bin: "integration",
        target: InGuestTarget::Test,
        features: "integration-tests,kvm-tests",
    },
    InGuestLane {
        package: "overdrive-worker",
        bin: "integration",
        target: InGuestTarget::Test,
        features: "integration-tests",
    },
    InGuestLane {
        package: "overdrive-worker",
        bin: "leg_f_repair",
        target: InGuestTarget::Test,
        features: "integration-tests",
    },
];

/// Where the staged libtest binary of `package`'s `bin` target lives.
#[must_use]
pub fn in_guest_binary_path(target_dir: &Path, package: &str, bin: &str) -> PathBuf {
    target_dir.join("guest-kit").join("in-guest").join(format!("{package}--{bin}"))
}

/// Where EVMOD-BUILD leaves the forwarder module built for `release`.
///
/// From this workspace's source: `<workspace>/target/kmod/<release>/ovd_fwd.ko`
/// (round 13, FWD-PREREQ). The workspace root, never the cargo target
/// directory: in the Lima VM `CARGO_TARGET_DIR` is one directory every
/// Conductor workspace shares (`infra/lima/overdrive-dev.yaml`), so a `.ko`
/// there would be whichever workspace built last.
#[must_use]
pub fn forwarder_module_path(workspace_root: &Path, release: &str) -> PathBuf {
    kmod_dir(workspace_root).join(release).join("ovd_fwd.ko")
}

/// `<workspace>/target/kmod` — the directory the mutation wrapper exports as
/// `OVERDRIVE_KMOD_DIR` for its scratch copies.
#[must_use]
pub fn kmod_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("target").join("kmod")
}

/// The static-musl target triple of this host.
///
/// # Errors
///
/// When the host architecture has no supported guest target.
pub fn musl_target_triple() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("x86_64-unknown-linux-musl"),
        "aarch64" => Ok("aarch64-unknown-linux-musl"),
        other => bail!("no guest-kit musl target for host architecture {other}"),
    }
}

/// Where cargo leaves the static scripted peer.
#[must_use]
pub fn guest_peer_path(target_dir: &Path, triple: &str) -> PathBuf {
    target_dir.join(triple).join("release").join(GUEST_PEER_BIN)
}

/// Where the prepared host-side guest dispatcher is written.
#[must_use]
fn guest_suite_path(target_dir: &Path) -> PathBuf {
    target_dir.join("debug").join(GUEST_SUITE_BIN)
}

/// Where the existing Quint test runtime closure is prepared for the one
/// active source-local Gateway conformance test that invokes `quint test`.
/// `overdrive-testing` consumes this exact target-scoped directory; it does
/// not discover or install a tool from the selected-test wrapper.
const QUINT_RUNTIME_DIR: &str = "guest-kit/quint-test-runtime";
const QUINT_RUNTIME_FILES_DIR: &str = "files";
const QUINT_RUNTIME_MANIFEST: &str = "manifest.v1";
const QUINT_RUNTIME_FORMAT: &str = "overdrive-quint-test-runtime-v1";
const QUINT_PACKAGE_NAME: &str = "@informalsystems/quint";
const QUINT_GUEST_COMMAND: &str = "/usr/local/bin/quint";

fn normalized_guest_path(path: &Path, what: &str) -> Result<()> {
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(component, std::path::Component::CurDir | std::path::Component::ParentDir)
        })
    {
        bail!("{what} is not an absolute normalized path: {}", path.display());
    }
    if path.to_str().is_none_or(|value| value.chars().any(char::is_control)) {
        bail!("{what} is not a manifest-safe UTF-8 path: {}", path.display());
    }
    Ok(())
}

fn executable_on_path(name: &str) -> Option<PathBuf> {
    let search = std::env::var_os("PATH")?;
    std::env::split_paths(&search).map(|directory| directory.join(name)).find(|candidate| {
        candidate.is_file()
            && candidate.metadata().is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    })
}

fn quint_launcher_paths(wrapper: &Path) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let script = std::fs::read_to_string(wrapper)
        .wrap_err_with(|| format!("reading Quint launcher {}", wrapper.display()))?;
    let node_bin = script
        .lines()
        .find_map(|line| {
            let path = line.strip_prefix("export PATH=\"")?.split(':').next()?;
            (!path.is_empty()
                && !path.chars().any(|character| matches!(character, '$' | '"' | '\\')))
            .then(|| PathBuf::from(path))
        })
        .ok_or_else(|| {
            eyre!("Quint launcher {} has no literal Node PATH prefix", wrapper.display())
        })?;
    let cli_wrapper = script
        .lines()
        .find_map(|line| line.strip_prefix("exec \"")?.strip_suffix("\" \"$@\""))
        .map(PathBuf::from)
        .ok_or_else(|| {
            eyre!("Quint launcher {} has no literal CLI exec target", wrapper.display())
        })?;
    let quint_home = script
        .lines()
        .find_map(|line| {
            line.strip_prefix("export QUINT_HOME=\"${QUINT_HOME:-")?.strip_suffix("}\"")
        })
        .map(PathBuf::from)
        .ok_or_else(|| {
            eyre!("Quint launcher {} has no literal QUINT_HOME default", wrapper.display())
        })?;
    normalized_guest_path(&node_bin, "Quint Node directory")?;
    normalized_guest_path(&cli_wrapper, "Quint CLI wrapper")?;
    normalized_guest_path(&quint_home, "Quint home directory")?;
    Ok((node_bin, cli_wrapper, quint_home))
}

fn package_manifest(path: &Path) -> Result<serde_json::Value> {
    let bytes = std::fs::read(path).wrap_err_with(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).wrap_err_with(|| format!("parsing {}", path.display()))
}

fn quint_package_root(entry: &Path) -> Result<PathBuf> {
    let mut parent =
        entry.parent().ok_or_else(|| eyre!("Quint CLI entry {} has no parent", entry.display()))?;
    loop {
        let manifest_path = parent.join("package.json");
        if manifest_path.is_file() {
            let manifest = package_manifest(&manifest_path)?;
            if manifest["name"].as_str() == Some(QUINT_PACKAGE_NAME) {
                return parent.canonicalize().wrap_err_with(|| {
                    format!("canonicalizing Quint package {}", parent.display())
                });
            }
        }
        parent = parent
            .parent()
            .ok_or_else(|| eyre!("Quint CLI entry is not inside {QUINT_PACKAGE_NAME}"))?;
    }
}

fn quint_evaluator_version(package_root: &Path) -> Result<String> {
    const VERSION_LINE: &str = "exports.QUINT_EVALUATOR_VERSION = '";
    let source = package_root.join("dist/src/rust/binaryManager.js");
    let contents = std::fs::read_to_string(&source)
        .wrap_err_with(|| format!("reading the Quint evaluator version at {}", source.display()))?;
    let version = contents
        .split_once(VERSION_LINE)
        .and_then(|(_, rest)| rest.split_once('\'').map(|(version, _)| version))
        .filter(|version| !version.is_empty() && !version.chars().any(char::is_control))
        .ok_or_else(|| {
            eyre!("Quint package has no pinned Rust evaluator version at {}", source.display())
        })?;
    Ok(version.to_owned())
}

fn copy_quint_package_tree(
    source: &Path,
    guest: &Path,
    files_root: &Path,
    files: &mut std::collections::BTreeSet<PathBuf>,
) -> Result<()> {
    let mut entries = std::fs::read_dir(source)
        .wrap_err_with(|| format!("listing Quint package {}", source.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort();
    for path in entries {
        if path.file_name().is_some_and(|name| {
            matches!(name.to_str(), Some("test" | "tests" | "coverage" | ".nyc_output" | ".bin"))
        }) {
            continue;
        }
        let file_type = std::fs::symlink_metadata(&path)
            .wrap_err_with(|| format!("reading Quint package entry {}", path.display()))?
            .file_type();
        let guest_path = guest
            .join(path.file_name().ok_or_else(|| {
                eyre!("Quint package entry has no file name: {}", path.display())
            })?);
        if file_type.is_dir() {
            copy_quint_package_tree(&path, &guest_path, files_root, files)?;
        } else if file_type.is_symlink() {
            if path.is_dir() {
                bail!("directory symlink in Quint package closure: {}", path.display());
            }
            copy_file_at_guest_path(&path, &guest_path, files_root)?;
            files.insert(guest_path);
        } else if file_type.is_file() {
            copy_file_at_guest_path(&path, &guest_path, files_root)?;
            files.insert(guest_path);
        }
    }
    Ok(())
}

fn copy_file_at_guest_path(host: &Path, guest: &Path, files_root: &Path) -> Result<()> {
    normalized_guest_path(guest, "prepared Quint guest input")?;
    let relative = guest.strip_prefix("/").map_err(|error| eyre!("{error}"))?;
    let target = files_root.join(relative);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .wrap_err_with(|| format!("creating prepared input parent {}", parent.display()))?;
    }
    std::fs::copy(host, &target)
        .wrap_err_with(|| format!("preparing {} for {}", host.display(), guest.display()))?;
    Ok(())
}

fn write_quint_launcher(path: &Path, node: &Path, entry: &Path) -> Result<()> {
    normalized_guest_path(node, "prepared Quint Node executable")?;
    normalized_guest_path(entry, "prepared Quint CLI entry")?;
    let launcher = format!(
        "#!/bin/sh\nexec {} {} \"$@\"\n",
        shell_quote_path(node)?,
        shell_quote_path(entry)?
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .wrap_err_with(|| format!("creating Quint launcher parent {}", parent.display()))?;
    }
    std::fs::write(path, launcher.as_bytes())
        .wrap_err_with(|| format!("writing prepared Quint launcher {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).wrap_err_with(|| {
        format!("making prepared Quint launcher executable {}", path.display())
    })?;
    Ok(())
}

fn shell_quote_path(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| eyre!("Quint runtime path is not UTF-8: {}", path.display()))?;
    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}

struct QuintRuntimeSource {
    quint_command: PathBuf,
    quint_version: String,
    node_host: PathBuf,
    node_guest_path: PathBuf,
    node_version: String,
    shell_host: PathBuf,
    package_root: PathBuf,
    evaluator_guest_path: PathBuf,
    evaluator_version: String,
    cli_wrapper_guest_path: PathBuf,
    cli_entry_guest_path: PathBuf,
}

fn discover_quint_runtime(quint_command: &Path) -> Result<QuintRuntimeSource> {
    let (node_bin, cli_wrapper_guest_path, quint_home) = quint_launcher_paths(quint_command)?;
    let node_host = node_bin
        .join("node")
        .canonicalize()
        .wrap_err_with(|| format!("resolving Node from Quint PATH {}", node_bin.display()))?;
    let cli_entry_guest_path = cli_wrapper_guest_path.canonicalize().wrap_err_with(|| {
        format!("resolving Quint CLI entry {}", cli_wrapper_guest_path.display())
    })?;
    let package_root = quint_package_root(&cli_entry_guest_path)?;
    let package = package_manifest(&package_root.join("package.json"))?;
    let package_version = package["version"]
        .as_str()
        .ok_or_else(|| eyre!("Quint package has no version at {}", package_root.display()))?
        .to_owned();
    let evaluator_version = quint_evaluator_version(&package_root)?;
    let evaluator_guest_path =
        quint_home.join(format!("rust-evaluator-{evaluator_version}")).join("quint_evaluator");
    normalized_guest_path(&evaluator_guest_path, "Quint Rust evaluator")?;
    if !evaluator_guest_path.is_file() {
        bail!(
            "Quint {package_version} defaults `quint test` to Rust evaluator {evaluator_version}, but its existing host input is absent at {}; prepare the current evaluator before running guest-kit-prepare",
            evaluator_guest_path.display()
        );
    }

    let quint_check = Command::new(quint_command)
        .arg("--version")
        .output()
        .wrap_err_with(|| format!("running {} --version", quint_command.display()))?;
    if !quint_check.status.success() {
        bail!(
            "Quint version check failed (exit {:?}): {}",
            quint_check.status.code(),
            String::from_utf8_lossy(&quint_check.stderr)
        );
    }
    let quint_version = String::from_utf8_lossy(&quint_check.stdout).trim().to_owned();
    if quint_version != package_version {
        bail!(
            "Quint launcher version {quint_version:?} does not match package version {package_version:?}"
        );
    }

    let node_check = Command::new(&node_host)
        .arg("--version")
        .output()
        .wrap_err_with(|| format!("running {} --version", node_host.display()))?;
    if !node_check.status.success() {
        bail!(
            "Node version check failed (exit {:?}): {}",
            node_check.status.code(),
            String::from_utf8_lossy(&node_check.stderr)
        );
    }
    let node_version = String::from_utf8_lossy(&node_check.stdout).trim().to_owned();
    if node_version.is_empty() || node_version.chars().any(char::is_control) {
        bail!("Node returned an invalid version string: {node_version:?}");
    }
    let shell_host = executable_on_path("sh").ok_or_else(|| {
        eyre!("the existing host shell needed by Quint's /bin/sh launcher is absent")
    })?;

    Ok(QuintRuntimeSource {
        quint_command: quint_command.to_owned(),
        quint_version,
        node_guest_path: node_bin.join("node"),
        node_host,
        node_version,
        shell_host,
        package_root,
        evaluator_guest_path,
        evaluator_version,
        cli_wrapper_guest_path,
        cli_entry_guest_path,
    })
}

fn stage_quint_runtime(source: &QuintRuntimeSource, staging: &Path) -> Result<usize> {
    let files_root = staging.join(QUINT_RUNTIME_FILES_DIR);
    std::fs::create_dir_all(&files_root)
        .wrap_err_with(|| format!("creating prepared Quint inputs at {}", files_root.display()))?;

    let launcher_guest_path = PathBuf::from(QUINT_GUEST_COMMAND);
    let shell_guest_path = PathBuf::from("/bin/sh");
    let mut files = std::collections::BTreeSet::new();
    for (host, guest) in [
        (&source.quint_command, &launcher_guest_path),
        (&source.node_host, &source.node_guest_path),
        (&source.shell_host, &shell_guest_path),
        (&source.evaluator_guest_path, &source.evaluator_guest_path),
    ] {
        copy_file_at_guest_path(host, guest, &files_root)?;
        files.insert(guest.clone());
    }
    copy_quint_package_tree(&source.package_root, &source.package_root, &files_root, &mut files)?;

    let inner_launcher_host = files_root
        .join(source.cli_wrapper_guest_path.strip_prefix("/").map_err(|error| eyre!("{error}"))?);
    write_quint_launcher(
        &inner_launcher_host,
        &source.node_guest_path,
        &source.cli_entry_guest_path,
    )?;
    files.insert(source.cli_wrapper_guest_path.clone());
    if !files.contains(&source.cli_entry_guest_path) {
        bail!(
            "prepared Quint package did not include its executable entry {}",
            source.cli_entry_guest_path.display()
        );
    }

    let manifest_path = staging.join(QUINT_RUNTIME_MANIFEST);
    let mut manifest = format!(
        "{QUINT_RUNTIME_FORMAT}\nquint_version={}\nnode_version={}\nrust_evaluator_version={}\nnode_guest_path={}\nrust_evaluator_guest_path={}\nshared=/bin/sh\nshared={}\nshared={}\nlauncher_guest_path={}\ncli_wrapper_guest_path={}\ncli_entry_guest_path={}\n",
        source.quint_version,
        source.node_version,
        source.evaluator_version,
        source.node_guest_path.display(),
        source.evaluator_guest_path.display(),
        source.node_guest_path.display(),
        source.evaluator_guest_path.display(),
        launcher_guest_path.display(),
        source.cli_wrapper_guest_path.display(),
        source.cli_entry_guest_path.display(),
    );
    for guest_path in &files {
        manifest.push_str("file=");
        manifest.push_str(
            guest_path
                .to_str()
                .ok_or_else(|| eyre!("Quint guest path is not UTF-8: {}", guest_path.display()))?,
        );
        manifest.push('\n');
    }
    std::fs::write(&manifest_path, manifest)
        .wrap_err_with(|| format!("writing Quint runtime manifest {}", manifest_path.display()))?;
    Ok(files.len())
}

/// Runs the lane setup: the forwarder module, the scripted peer, the
/// in-guest test binaries — each verified at its contract path.
///
/// # Errors
///
/// The first step that fails, naming the command and its stderr, or the
/// artefact a successful step did not leave at its contract path.
pub fn run(workspace_root: &Path, target_dir: &Path) -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!(
            "guest-kit-prepare builds for the running Linux kernel; run it under `cargo xtask metal run --`"
        );
    }
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .wrap_err("reading the running kernel release")?
        .trim()
        .to_owned();

    // Build both BPF artifacts before staging the in-guest test binary:
    // overdrive-init embeds the guest-only capture object, while the dataplane
    // loader consumes the host object. A clean lane must not compile either
    // against stale or absent BPF.
    let xtask = std::env::current_exe().wrap_err("locating the running xtask")?;
    run_step(
        Command::new(&xtask)
            .args(["bpf-build"])
            .env("OVERDRIVE_BPF_NATIVE", "1")
            .current_dir(workspace_root),
        "cargo xtask bpf-build",
    )?;

    // EVMOD-BUILD: the module against the running kernel's build tree.
    let build_dir = format!("/lib/modules/{release}/build");
    run_step(
        Command::new(xtask)
            .args(["kmod-build", "--kernel-build-dir", &build_dir])
            .current_dir(workspace_root),
        "cargo xtask kmod-build",
    )?;
    require(&forwarder_module_path(workspace_root, &release), "ovd_fwd.ko")?;

    let triple = musl_target_triple()?;
    let mut guest_peer_build = Command::new("cargo");
    guest_peer_build
        .args(["build", "--package", "overdrive-testing", "--bin", GUEST_PEER_BIN])
        .args(["--target", triple, "--release"])
        .current_dir(workspace_root);
    if triple == "x86_64-unknown-linux-musl" {
        guest_peer_build
            .env("CC_x86_64_unknown_linux_musl", "musl-gcc")
            .env("CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER", "musl-gcc");
    }
    run_step(&mut guest_peer_build, "building overdrive-guest-peer")?;
    require(&guest_peer_path(target_dir, triple), GUEST_PEER_BIN)?;

    // GuestKit::provision uses this exact package/target/profile when it
    // re-verifies the fixture's production init. Prepare it before a nextest
    // per-attempt timeout starts so dispatcher provisioning does not compile
    // overdrive-init on the attempt's timed execution path.
    let mut overdrive_init_build = Command::new("cargo");
    overdrive_init_build
        .args(["build", "--package", "overdrive-init", "--target", triple, "--release"])
        .current_dir(workspace_root);
    run_step(&mut overdrive_init_build, "building static overdrive-init")?;
    let overdrive_init = target_dir.join(triple).join("release").join("overdrive-init");
    require(&overdrive_init, "static overdrive-init")?;

    for lane in IN_GUEST_LANES {
        let built = build_in_guest_tests(workspace_root, lane)?;
        let staged = in_guest_binary_path(target_dir, lane.package, lane.bin);
        let dir = staged.parent().ok_or_else(|| eyre!("no parent of {}", staged.display()))?;
        std::fs::create_dir_all(dir).wrap_err_with(|| format!("creating {}", dir.display()))?;
        std::fs::copy(&built, &staged)
            .wrap_err_with(|| format!("staging {} at {}", built.display(), staged.display()))?;
        eprintln!("guest-kit-prepare: {} {} -> {}", lane.package, lane.bin, staged.display());
    }
    let quint_runtime = prepare_quint_runtime(target_dir)?;
    let mut guest_suite_build = Command::new("cargo");
    guest_suite_build
        .args([
            "build",
            "--package",
            "overdrive-testing",
            "--bin",
            GUEST_SUITE_BIN,
            "--features",
            "integration-tests",
        ])
        .current_dir(workspace_root);
    run_step(&mut guest_suite_build, "building the prepared guest dispatcher")?;
    require(&guest_suite_path(target_dir), GUEST_SUITE_BIN)?;
    if let Some((quint_version, node_version, evaluator_version, files)) = quint_runtime {
        eprintln!(
            "guest-kit-prepare: Quint {quint_version}, Node {node_version}, Rust evaluator {evaluator_version}: staged {files} runtime inputs"
        );
    }
    eprintln!("guest-kit-prepare: kernel {release}, target {triple}: the lane is prepared");
    Ok(())
}

fn prepare_quint_runtime(target_dir: &Path) -> Result<Option<(String, String, String, usize)>> {
    let prepared = target_dir.join(QUINT_RUNTIME_DIR);
    let staging = target_dir.join(format!("{QUINT_RUNTIME_DIR}.staging"));
    for path in [&prepared, &staging] {
        if path.exists() {
            std::fs::remove_dir_all(path).wrap_err_with(|| {
                format!("clearing Quint preparation directory {}", path.display())
            })?;
        }
    }
    let Some(quint_command) = executable_on_path("quint") else {
        eprintln!(
            "guest-kit-prepare: quint is absent from host PATH; the exact Quint test will refuse as lane_not_prepared"
        );
        return Ok(None);
    };

    let source = discover_quint_runtime(&quint_command)?;
    let files = stage_quint_runtime(&source, &staging)?;
    std::fs::rename(&staging, &prepared)
        .wrap_err_with(|| format!("publishing prepared Quint runtime {}", prepared.display()))?;

    Ok(Some((source.quint_version, source.node_version, source.evaluator_version, files)))
}

fn run_step(command: &mut Command, what: &str) -> Result<std::process::Output> {
    let output = command.output().wrap_err_with(|| format!("spawning {what}"))?;
    if !output.status.success() {
        bail!(
            "{what} failed (exit {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(output)
}

fn require(path: &Path, artefact: &str) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        bail!("the step succeeded but left no {artefact} at {}", path.display())
    }
}

/// Builds `lane`'s libtest binary; returns the executable cargo reports.
fn build_in_guest_tests(workspace_root: &Path, lane: &InGuestLane) -> Result<PathBuf> {
    let mut command = Command::new("cargo");
    command.args(["test", "--no-run", "--package", lane.package, lane.target.flag()]);
    if lane.target != InGuestTarget::Lib {
        command.arg(lane.bin);
    }
    command
        .args(["--features", lane.features, "--message-format", "json"])
        .current_dir(workspace_root);
    let output = run_step(
        &mut command,
        &format!("building the in-guest tests of {} {}", lane.package, lane.bin),
    )?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let is_test_artifact = message["reason"] == "compiler-artifact"
            && message["profile"]["test"] == true
            && message["target"]["name"] == lane.bin;
        if let (true, Some(executable)) = (is_test_artifact, message["executable"].as_str()) {
            return Ok(PathBuf::from(executable));
        }
    }
    bail!(
        "cargo reported no test executable for {} {} {}",
        lane.package,
        lane.target.flag(),
        lane.bin
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::doc_markdown, reason = "repository-mandated CONTRACT_SHAPE marker")]

    use super::*;

    /// The contract paths `overdrive-testing` reads and the two embedded guest
    /// BPF objects the real `overdrive-init` build script requires, tracks and
    /// exports (see the module table).
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn the_artefact_paths_are_the_pinned_contract() {
        let target = Path::new("/t");
        assert_eq!(
            forwarder_module_path(Path::new("/w"), "7.0.0-34"),
            Path::new("/w/target/kmod/7.0.0-34/ovd_fwd.ko")
        );
        assert_eq!(kmod_dir(Path::new("/w")), Path::new("/w/target/kmod"));
        assert_eq!(
            guest_peer_path(target, "x86_64-unknown-linux-musl"),
            Path::new("/t/x86_64-unknown-linux-musl/release/overdrive-guest-peer")
        );
        assert_eq!(
            in_guest_binary_path(target, "overdrive-init", "overdrive-init"),
            Path::new("/t/guest-kit/in-guest/overdrive-init--overdrive-init")
        );

        let fixture = tempfile::tempdir().expect("temporary build-script workspace");
        let manifest_dir = fixture.path().join("crates/overdrive-init");
        let bpf_dir = fixture.path().join("target/bpf");
        std::fs::create_dir_all(&manifest_dir).expect("create fake init crate");
        std::fs::create_dir_all(&bpf_dir).expect("create fake BPF artifact directory");
        let guest = bpf_dir.join("overdrive_guest_bpf.o");
        let candidate = bpf_dir.join("overdrive_guest_tcp_correlation_candidate_bpf.o");
        std::fs::write(&guest, b"guest").expect("write guest BPF fixture");

        let build_script = fixture.path().join("overdrive-init-build-script");
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/overdrive-init/build.rs");
        let compiled = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg(&source)
            .arg("-o")
            .arg(&build_script)
            .output()
            .expect("compile the real overdrive-init build script");
        assert!(
            compiled.status.success(),
            "build-script compile failed: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let run = || {
            Command::new(&build_script)
                .env("CARGO_MANIFEST_DIR", &manifest_dir)
                .output()
                .expect("run the real overdrive-init build script")
        };

        let missing = run();
        assert!(
            !missing.status.success(),
            "the build script accepted a missing candidate object: {}",
            String::from_utf8_lossy(&missing.stdout)
        );

        std::fs::write(&candidate, b"candidate").expect("write candidate BPF fixture");
        let complete = run();
        assert!(
            complete.status.success(),
            "the build script refused both objects: {}",
            String::from_utf8_lossy(&complete.stderr)
        );
        let directives = String::from_utf8(complete.stdout).expect("build directives are UTF-8");
        for (artifact, export) in [
            (&guest, "OVERDRIVE_GUEST_BPF_OBJECT_PATH"),
            (&candidate, "OVERDRIVE_GUEST_TCP_CORRELATION_CANDIDATE_BPF_OBJECT_PATH"),
        ] {
            assert!(
                directives
                    .lines()
                    .any(|line| line == format!("cargo:rerun-if-changed={}", artifact.display())),
                "the build script did not track {}: {directives}",
                artifact.display()
            );
            assert!(
                directives
                    .lines()
                    .any(|line| line == format!("cargo:rustc-env={export}={}", artifact.display())),
                "the build script did not export {export}: {directives}"
            );
        }
    }

    /// S-12 drift guard (round 13: extended to the module path). xtask may
    /// not depend on `overdrive-testing` (development.md § "xtask is build /
    /// test / dev orchestration"), so the second spelling of each contract
    /// path is read as source text: a change on either side that the other
    /// does not mirror fails here, not as `LaneNotPrepared` on metal.
    #[test]
    fn overdrive_testing_spells_the_same_module_path_and_override() {
        let testing = include_str!("../../crates/overdrive-testing/src/forwarder_module.rs");
        for needle in [
            "workspace_root.join(\"target\").join(\"kmod\")",
            "kmod_dir.join(release).join(\"ovd_fwd.ko\")",
            "pub const KMOD_DIR_ENV: &str = \"OVERDRIVE_KMOD_DIR\";",
            "pub const LOADED_VERSION_PATH: &str = \"/sys/module/ovd_fwd/version\";",
        ] {
            assert!(
                testing.contains(needle),
                "overdrive-testing's forwarder_module.rs no longer spells `{needle}`"
            );
        }
        assert_eq!(crate::kmod_load::LOADED_VERSION, "/sys/module/ovd_fwd/version");
        let mutants = include_str!("mutants.rs");
        assert!(
            mutants.contains("\"OVERDRIVE_KMOD_DIR\""),
            "the mutation wrapper exports the override overdrive-testing reads"
        );
        assert!(
            !testing.contains("cargo_target_dir"),
            "the module lookup never reads the shared cargo target directory"
        );
    }
}
