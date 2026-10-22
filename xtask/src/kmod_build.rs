//! `cargo xtask kmod-build` — build the forwarder module for one kernel.
//!
//! EVMOD-BUILD (ADR-0203): the C source lives under `kmod/ovd_fwd/`, its
//! build output lives under the workspace root's `target/kmod/<release>/`,
//! and its `MODULE_VERSION` is the SHA-256 digest of sorted, non-ignored
//! relative paths and file contents.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use color_eyre::eyre::{Result, WrapErr, bail};
use sha2::{Digest, Sha256};

const MODULE_SOURCE: &str = "kmod/ovd_fwd";
const MODULE_NAME: &str = "ovd_fwd";

/// Build `ovd_fwd.ko` against `kernel_build_dir`, or the running kernel's
/// build tree when it is omitted.
///
/// The module is written to `<workspace>/target/kmod/<kernel-release>/`.
pub fn run(workspace_root: &Path, kernel_build_dir: Option<&Path>) -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("kmod-build needs a Linux kernel build tree; run it under `cargo xtask lima run --`");
    }

    let workspace_root = workspace_root.canonicalize().wrap_err("resolving workspace root")?;
    let source_dir = workspace_root.join(MODULE_SOURCE);
    if !source_dir.is_dir() {
        bail!("forwarder module source directory is missing: {}", source_dir.display());
    }

    let running_release = running_kernel_release()?;
    let kernel_build_dir = resolve_kernel_build_dir(kernel_build_dir, &running_release)?;
    let release = release_for_build_tree(&kernel_build_dir, &running_release)?;
    let digest = source_digest(&workspace_root)?;
    let output_dir = workspace_root.join("target/kmod").join(&release);
    std::fs::create_dir_all(&output_dir)
        .wrap_err_with(|| format!("creating module output directory {}", output_dir.display()))?;

    checkpatch(&kernel_build_dir, &source_dir)?;
    require_sparse()?;
    build_module(&kernel_build_dir, &source_dir, &output_dir, &digest)?;

    let module = output_dir.join(format!("{MODULE_NAME}.ko"));
    if !module.is_file() {
        bail!("kernel build succeeded without producing {}", module.display());
    }
    eprintln!("kmod-build: built {} (MODULE_VERSION {})", module.display(), digest);
    Ok(())
}

fn running_kernel_release() -> Result<String> {
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .wrap_err("reading the running kernel release")?;
    let release = release.trim();
    if release.is_empty() || release.contains('/') || release == "." || release == ".." {
        bail!("the running kernel returned an invalid release: {release:?}");
    }
    Ok(release.to_owned())
}

fn resolve_kernel_build_dir(requested: Option<&Path>, running_release: &str) -> Result<PathBuf> {
    let build_dir = requested.map_or_else(
        || PathBuf::from(format!("/lib/modules/{running_release}/build")),
        Path::to_path_buf,
    );
    build_dir
        .canonicalize()
        .wrap_err_with(|| format!("resolving kernel build tree {}", build_dir.display()))
}

fn release_for_build_tree(kernel_build_dir: &Path, running_release: &str) -> Result<String> {
    let running_build_dir = PathBuf::from(format!("/lib/modules/{running_release}/build"));
    if running_build_dir.canonicalize().ok().as_deref() == Some(kernel_build_dir) {
        return Ok(running_release.to_owned());
    }
    kernel_release(kernel_build_dir)
}

fn kernel_release(kernel_build_dir: &Path) -> Result<String> {
    let mut command = Command::new("make");
    command.args(["-s", "-C"]).arg(kernel_build_dir).arg("kernelrelease");
    let output = checked_output(command, "reading kernel release from build tree")?;
    let release = String::from_utf8(output.stdout).wrap_err("kernelrelease output is not UTF-8")?;
    let release = release.trim();
    if release.is_empty() || release.contains('/') || release == "." || release == ".." {
        bail!("kernel build tree returned an invalid release: {release:?}");
    }
    Ok(release.to_owned())
}

fn source_digest(workspace_root: &Path) -> Result<String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(workspace_root)
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "-z", "--"])
        .arg(MODULE_SOURCE);
    let output = checked_output(command, "listing non-ignored forwarder module files")?;
    let mut paths = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8(path.to_vec()).wrap_err("module path from git is not UTF-8"))
        .collect::<Result<Vec<_>>>()?;
    paths.sort();
    if paths.is_empty() {
        bail!("no non-ignored files found under {MODULE_SOURCE}");
    }

    let prefix = format!("{MODULE_SOURCE}/");
    let mut digest = Sha256::new();
    for path in paths {
        let relative = path.strip_prefix(&prefix).ok_or_else(|| {
            color_eyre::eyre::eyre!("git returned an out-of-tree module path: {path}")
        })?;
        let bytes = relative.as_bytes();
        let contents = std::fs::read(workspace_root.join(&path))
            .wrap_err_with(|| format!("reading module source {path}"))?;
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
        digest.update((contents.len() as u64).to_be_bytes());
        digest.update(contents);
    }
    Ok(hex::encode(digest.finalize()))
}

fn checkpatch(kernel_build_dir: &Path, source_dir: &Path) -> Result<()> {
    let checkpatch = kernel_build_dir.join("scripts/checkpatch.pl");
    if !checkpatch.is_file() {
        bail!("checkpatch.pl is missing from kernel build tree: {}", checkpatch.display());
    }
    for file in [
        source_dir.join("Kbuild"),
        source_dir.join("ovd_fwd.c"),
        source_dir.join("include/uapi/linux/ovd_fwd.h"),
    ] {
        let mut command = Command::new("perl");
        command.arg(&checkpatch).args(["--strict", "--no-tree", "--file"]).arg(&file);
        run_checked(command, &format!("checkpatch --strict {}", file.display()))?;
    }
    Ok(())
}

fn require_sparse() -> Result<()> {
    let output = Command::new("sh")
        .args(["-c", "command -v sparse >/dev/null 2>&1"])
        .output()
        .wrap_err("checking for sparse")?;
    if !output.status.success() {
        bail!("sparse is required for `C=1`; install sparse on the build host");
    }
    Ok(())
}

fn build_module(
    kernel_build_dir: &Path,
    source_dir: &Path,
    output_dir: &Path,
    digest: &str,
) -> Result<()> {
    let module_version = format!(r#"KCFLAGS=-DOVD_FWD_MODULE_VERSION=\"{digest}\""#);
    let mut command = Command::new("make");
    command
        .arg("-C")
        .arg(kernel_build_dir)
        .arg(format!("M={}", source_dir.display()))
        .arg(format!("MO={}", output_dir.display()))
        .args(["W=1", "C=1"])
        .arg(module_version)
        .arg("modules");
    run_kernel_build(command)
}

fn run_kernel_build(mut command: Command) -> Result<()> {
    let output = command.output().wrap_err("starting kernel module build")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    std::io::stdout()
        .lock()
        .write_all(stdout.as_bytes())
        .wrap_err("writing kernel build stdout")?;
    std::io::stderr()
        .lock()
        .write_all(stderr.as_bytes())
        .wrap_err("writing kernel build stderr")?;
    if stdout.contains("C=1 specified, but") || stderr.contains("C=1 specified, but") {
        bail!("the kernel build tree disabled sparse despite `C=1`; install a compatible sparse");
    }
    if !output.status.success() {
        bail!("building ovd_fwd.ko with W=1 and C=1 failed with {}", output.status);
    }
    Ok(())
}

fn checked_output(mut command: Command, description: &str) -> Result<Output> {
    let output = command.output().wrap_err_with(|| format!("starting {description}"))?;
    if !output.status.success() {
        bail!(
            "{description} failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output)
}

fn run_checked(mut command: Command, description: &str) -> Result<()> {
    eprintln!("kmod-build: {description}");
    let status = command.status().wrap_err_with(|| format!("starting {description}"))?;
    if !status.success() {
        bail!("{description} failed with {status}");
    }
    Ok(())
}
