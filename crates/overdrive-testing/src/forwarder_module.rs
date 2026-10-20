//! The forwarder-module lane prerequisite of every `serve` test.
//!
//! FWD-PREREQ, GH #295; feature-delta § *Composition*, "Forwarder module as
//! a lane prerequisite" ("Which module is loaded", "The fixture's check").
//!
//! `overdrive serve` never loads `ovd_fwd` (EVMOD-LOAD): on the appliance the
//! host image loads it before `serve` starts; on a test host the lane's setup
//! does, with `cargo xtask kmod-build` then `cargo xtask kmod-load`. Every
//! `serve` boot runs the real phase-1 forwarder probe, so a `serve` test
//! depends on the module being loaded — and, because every Conductor
//! workspace shares the one Lima VM, on the loaded module being *this
//! workspace's* build.
//!
//! **Identity (D-20).** A module's identity is its `version` modinfo field:
//! the SHA-256 tree digest of `kmod/ovd_fwd/` (every file git does not
//! ignore, `Kbuild` and the UAPI header included) that `kmod-build` computes
//! once and compiles in as `MODULE_VERSION`. The loaded module exposes it as
//! `/sys/module/ovd_fwd/version` (`kernel/module/main.c:607 @ v7.0`).
//! `srcversion` is not used: it digests same-directory dependencies only
//! (`scripts/mod/sumversion.c:366-367 @ v7.0`), so it misses the UAPI header.
//!
//! [`require_forwarder_module`] compares the two values before a `serve`
//! test boots and refuses as a missing or stale lane prerequisite naming the
//! step to run. A stale module's remedy is `cargo xtask kmod-load`, which
//! reloads it once it is idle (MODULE-REFCOUNT): never a reboot.
//!
//! **The built module's path is a contract with `xtask`** (which cannot
//! depend on this crate): `<workspace>/target/kmod/<kernel-release>/ovd_fwd.ko`
//! — the workspace root, never the cargo target directory (which every Lima
//! workspace shares). Under `cargo xtask mutants` the wrapper exports
//! [`KMOD_DIR_ENV`] naming the original tree's `<workspace>/target/kmod`,
//! read first here (the `OVERDRIVE_BPF_OBJECT` precedent). Both spellings are
//! pinned by `the_module_path_is_the_pinned_contract` here and
//! `xtask::guest_kit_prepare`'s `the_artefact_paths_are_the_pinned_contract`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::vm_fixture;

/// The environment variable the mutation wrapper sets to the original
/// tree's `<workspace>/target/kmod` (test support and xtask only).
pub const KMOD_DIR_ENV: &str = "OVERDRIVE_KMOD_DIR";

/// Where the loaded module exposes its `version` (the tree digest;
/// `MODINFO_ATTR(version)`, `kernel/module/main.c:607 @ v7.0`).
pub const LOADED_VERSION_PATH: &str = "/sys/module/ovd_fwd/version";

/// The directory the lane's `kmod-build` writes into for this workspace:
/// `$OVERDRIVE_KMOD_DIR` when set, else `<workspace>/target/kmod`.
#[must_use]
pub fn kmod_dir() -> PathBuf {
    std::env::var_os(KMOD_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map_or_else(|| kmod_dir_under(&vm_fixture::workspace_root()), PathBuf::from)
}

/// `<workspace_root>/target/kmod` — the workspace-scoped build directory.
#[must_use]
pub fn kmod_dir_under(workspace_root: &Path) -> PathBuf {
    workspace_root.join("target").join("kmod")
}

/// `<kmod_dir>/<release>/ovd_fwd.ko`.
#[must_use]
pub fn built_module_path(kmod_dir: &Path, release: &str) -> PathBuf {
    kmod_dir.join(release).join("ovd_fwd.ko")
}

/// Why the lane prerequisite does not hold.
#[derive(Debug, thiserror::Error)]
pub enum ForwarderPrerequisiteError {
    /// The running kernel release could not be read.
    #[error("reading the running kernel release: {source}")]
    Release {
        /// The error.
        #[source]
        source: io::Error,
    },
    /// This workspace has no built module for the running kernel.
    #[error(
        "lane prerequisite missing: no ovd_fwd.ko built from this workspace at {path}; run \
         `cargo xtask lima run -- cargo xtask kmod-build` (or the same under `cargo xtask metal \
         run --`), then `cargo xtask kmod-load`"
    )]
    NotBuilt {
        /// Where `kmod-build` leaves it.
        path: PathBuf,
    },
    /// The built module carries no `version` modinfo field.
    #[error(
        "the built module {path} carries no version (kmod-build compiles the tree digest in as \
         MODULE_VERSION); rebuild it with `cargo xtask kmod-build`"
    )]
    NoBuiltVersion {
        /// The built module.
        path: PathBuf,
    },
    /// No `ovd_fwd` is loaded in the running kernel.
    #[error(
        "lane prerequisite missing: ovd_fwd is not loaded; run `cargo xtask lima run -- cargo \
         xtask kmod-load` (or the same under `cargo xtask metal run --`)"
    )]
    NotLoaded,
    /// A loaded module's `version` could not be read.
    #[error("reading {path}: {source}")]
    LoadedRead {
        /// The sysfs attribute.
        path: PathBuf,
        /// The error.
        #[source]
        source: io::Error,
    },
    /// The built module could not be read.
    #[error("reading {path}: {source}")]
    BuiltRead {
        /// The built module.
        path: PathBuf,
        /// The error.
        #[source]
        source: io::Error,
    },
    /// The loaded module is not this workspace's build.
    #[error(
        "stale lane prerequisite: the loaded ovd_fwd has version {loaded}, but this workspace's \
         build {path} has version {built}. Run `cargo xtask lima run -- cargo xtask kmod-load` \
         (or the same under `cargo xtask metal run --`): it reloads the module once it is idle"
    )]
    Stale {
        /// The loaded module's `version`.
        loaded: String,
        /// This workspace's build's `version`.
        built: String,
        /// This workspace's build.
        path: PathBuf,
    },
}

/// The `version=` modinfo field of a module image.
///
/// Modinfo strings are NUL-terminated `key=value` pairs in `.modinfo`, so the
/// key matches only at the start of an entry (after a NUL, or at offset 0):
/// the `version=` inside `srcversion=` is not this field. `None` when absent,
/// unterminated, empty or not UTF-8.
#[must_use]
pub fn version_in(module_image: &[u8]) -> Option<String> {
    const KEY: &[u8] = b"version=";
    let at = module_image
        .windows(KEY.len())
        .enumerate()
        .find(|(k, w)| *w == KEY && (*k == 0 || module_image[k - 1] == 0))
        .map(|(k, _)| k)?;
    let rest = module_image.get(at + KEY.len()..)?;
    let end = rest.iter().position(|b| *b == 0)?;
    let value = std::str::from_utf8(rest.get(..end)?).ok()?;
    (!value.is_empty()).then(|| value.to_owned())
}

/// The verdict over the two observed values: `loaded` is `None` when no
/// `ovd_fwd` is loaded.
///
/// # Errors
///
/// [`ForwarderPrerequisiteError::NotLoaded`] or
/// [`ForwarderPrerequisiteError::Stale`].
pub fn verdict(
    loaded: Option<&str>,
    built: &str,
    built_path: &Path,
) -> Result<(), ForwarderPrerequisiteError> {
    match loaded {
        None => Err(ForwarderPrerequisiteError::NotLoaded),
        Some(loaded) if loaded.trim() == built => Ok(()),
        Some(loaded) => Err(ForwarderPrerequisiteError::Stale {
            loaded: loaded.trim().to_owned(),
            built: built.to_owned(),
            path: built_path.to_path_buf(),
        }),
    }
}

/// The FWD-PREREQ check a `serve` test fixture runs before it boots.
///
/// # Errors
///
/// A missing build ([`ForwarderPrerequisiteError::NotBuilt`]), a missing
/// load ([`ForwarderPrerequisiteError::NotLoaded`]) or a foreign load
/// ([`ForwarderPrerequisiteError::Stale`]), each naming its remedy.
pub fn require_forwarder_module() -> Result<(), ForwarderPrerequisiteError> {
    let release = fs::read_to_string("/proc/sys/kernel/osrelease")
        .map_err(|source| ForwarderPrerequisiteError::Release { source })?;
    let path = built_module_path(&kmod_dir(), release.trim());
    let image = match fs::read(&path) {
        Ok(image) => image,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(ForwarderPrerequisiteError::NotBuilt { path });
        }
        Err(source) => return Err(ForwarderPrerequisiteError::BuiltRead { path, source }),
    };
    let Some(built) = version_in(&image) else {
        return Err(ForwarderPrerequisiteError::NoBuiltVersion { path });
    };
    let loaded = match fs::read_to_string(LOADED_VERSION_PATH) {
        Ok(loaded) => Some(loaded),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(ForwarderPrerequisiteError::LoadedRead {
                path: PathBuf::from(LOADED_VERSION_PATH),
                source,
            });
        }
    };
    verdict(loaded.as_deref(), &built, &path)
}

/// [`require_forwarder_module`] for a fixture: panics with the
/// prerequisite's message (it names the lane step, never a boot failure).
///
/// # Panics
///
/// When the lane prerequisite does not hold.
pub fn expect_forwarder_module() {
    if let Err(e) = require_forwarder_module() {
        panic!("{e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The contract path `xtask::guest_kit_prepare::forwarder_module_path`
    /// spells identically (S-12 drift guard).
    #[test]
    fn the_module_path_is_the_pinned_contract() {
        let dir = kmod_dir_under(Path::new("/w"));
        assert_eq!(dir, Path::new("/w/target/kmod"));
        assert_eq!(
            built_module_path(&dir, "7.0.0-34"),
            Path::new("/w/target/kmod/7.0.0-34/ovd_fwd.ko")
        );
    }

    #[test]
    fn version_is_the_entry_at_a_nul_boundary_not_the_tail_of_srcversion() {
        let image = b"\x7fELF..license=GPL\0srcversion=4F2A9C0E1B7D3A65C8E21F0\0version=9c1e\0";
        assert_eq!(version_in(image).as_deref(), Some("9c1e"));
        // `srcversion=` alone carries no `version` field.
        assert_eq!(version_in(b"license=GPL\0srcversion=4F2A\0"), None);
        assert_eq!(version_in(b"version=ab\0").as_deref(), Some("ab"));
    }

    #[test]
    fn a_module_without_version_or_with_an_unterminated_or_empty_one_has_none() {
        assert_eq!(version_in(b"license=GPL\0"), None);
        assert_eq!(version_in(b"license=GPL\0version=ABC"), None);
        assert_eq!(version_in(b"license=GPL\0version=\0"), None);
    }

    #[test]
    fn equal_values_pass_after_trimming_the_sysfs_newline() {
        assert!(verdict(Some("ABC\n"), "ABC", Path::new("/k")).is_ok());
    }

    #[test]
    fn no_loaded_module_is_not_loaded_and_a_foreign_one_is_stale_naming_both_values() {
        assert!(matches!(
            verdict(None, "ABC", Path::new("/k")),
            Err(ForwarderPrerequisiteError::NotLoaded)
        ));
        match verdict(Some("DEF\n"), "ABC", Path::new("/k")) {
            Err(e @ ForwarderPrerequisiteError::Stale { .. }) => {
                let msg = e.to_string();
                assert!(msg.contains("DEF") && msg.contains("ABC"), "{msg}");
                assert!(msg.contains("cargo xtask kmod-load"), "{msg}");
                assert!(!msg.contains("reboot"), "MODULE-REFCOUNT: never a reboot: {msg}");
            }
            other => panic!("expected Stale, got {other:?}"),
        }
    }
}
