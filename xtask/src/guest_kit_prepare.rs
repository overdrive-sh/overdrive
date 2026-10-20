//! `cargo xtask guest-kit-prepare` — the lane setup step of the guest-kit
//! native-metal lanes (GH #295, T-1; EVMOD-GUEST-KIT, ADR-0206).
//!
//! The guest-kit fixture (`overdrive_testing::guest_kit`, `guest_lane`)
//! needs three build artefacts before any of its tests can run: the
//! forwarder module built for the running kernel, the static scripted guest
//! program, and the libtest binaries the in-guest lane runs inside a guest.
//! Building them is not a test's job: a nested `cargo` inside a test body
//! couples a test result to a build result and contends for the target-dir
//! lock the running `nextest` holds. So the lane runs this step first, as
//! its declared setup, and the fixture only checks that each artefact is
//! where this step leaves it, refusing with a precondition message naming
//! this command when one is missing:
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
//! | an in-guest libtest binary | `guest-kit/in-guest/<package>--<bin>` | `guest_lane::InGuestTests::prepared` |

use std::path::{Path, PathBuf};
use std::process::Command;

use color_eyre::eyre::{Result, WrapErr, bail, eyre};

/// The name of the scripted peer's binary target in `overdrive-testing`.
pub const GUEST_PEER_BIN: &str = "overdrive-guest-peer";

/// One in-guest lane: a package's libtest binary built with `features` and
/// staged for the guest-kit image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InGuestLane {
    /// The package.
    pub package: &'static str,
    /// Its `bin` target whose unit tests run in the guest.
    pub bin: &'static str,
    /// The features the binary is built with.
    pub features: &'static str,
}

/// Every in-guest lane this step stages.
pub const IN_GUEST_LANES: &[InGuestLane] = &[InGuestLane {
    package: "overdrive-init",
    bin: "overdrive-init",
    features: "integration-tests,kvm-tests",
}];

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

    // EVMOD-BUILD: the module against the running kernel's build tree.
    let xtask = std::env::current_exe().wrap_err("locating the running xtask")?;
    let build_dir = format!("/lib/modules/{release}/build");
    run_step(
        Command::new(xtask)
            .args(["kmod-build", "--kernel-build-dir", &build_dir])
            .current_dir(workspace_root),
        "cargo xtask kmod-build",
    )?;
    require(&forwarder_module_path(workspace_root, &release), "ovd_fwd.ko")?;

    let triple = musl_target_triple()?;
    run_step(
        Command::new("cargo")
            .args(["build", "--package", "overdrive-testing", "--bin", GUEST_PEER_BIN])
            .args(["--target", triple, "--release"])
            .current_dir(workspace_root),
        "building overdrive-guest-peer",
    )?;
    require(&guest_peer_path(target_dir, triple), GUEST_PEER_BIN)?;

    for lane in IN_GUEST_LANES {
        let built = build_in_guest_tests(workspace_root, lane)?;
        let staged = in_guest_binary_path(target_dir, lane.package, lane.bin);
        let dir = staged.parent().ok_or_else(|| eyre!("no parent of {}", staged.display()))?;
        std::fs::create_dir_all(dir).wrap_err_with(|| format!("creating {}", dir.display()))?;
        std::fs::copy(&built, &staged)
            .wrap_err_with(|| format!("staging {} at {}", built.display(), staged.display()))?;
        eprintln!("guest-kit-prepare: {} {} -> {}", lane.package, lane.bin, staged.display());
    }
    eprintln!("guest-kit-prepare: kernel {release}, target {triple}: the lane is prepared");
    Ok(())
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
    let output = run_step(
        Command::new("cargo")
            .args(["test", "--no-run", "--package", lane.package, "--bin", lane.bin])
            .args(["--features", lane.features, "--message-format", "json"])
            .current_dir(workspace_root),
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
    bail!("cargo reported no test executable for {} bin {}", lane.package, lane.bin)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The contract paths `overdrive-testing` reads (see the module table).
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
