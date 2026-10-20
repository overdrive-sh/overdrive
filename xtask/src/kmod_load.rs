//! `cargo xtask kmod-load` — loads this workspace's forwarder module.
//!
//! The lane setup step that loads the module into the running kernel
//! (FWD-PREREQ, GH #295; feature-delta § *Composition*, "Forwarder module as
//! a lane prerequisite", "`kmod-load`"; EVMOD-LOAD, ADR-0205; MODULE-REFCOUNT,
//! ADR-0198).
//!
//! `overdrive serve` never loads `ovd_fwd`: the appliance image does, before
//! `serve` starts. A test host's lane loads it after `cargo xtask kmod-build`
//! (EVMOD-BUILD) has left this workspace's module at
//! `<workspace>/target/kmod/<uname -r>/ovd_fwd.ko`
//! ([`crate::guest_kit_prepare::forwarder_module_path`]):
//!
//! ```text
//! cargo xtask lima run -- cargo xtask kmod-build
//! cargo xtask lima run -- cargo xtask kmod-load            # strict (CI)
//! cargo xtask lima run -- cargo xtask kmod-load --busy-ok  # hooks
//! ```
//!
//! **Identity (D-20).** A module's identity is its `version` modinfo field,
//! the tree digest `kmod-build` compiles in; the loaded module exposes it as
//! [`LOADED_VERSION`].
//!
//! **The steps (bounded to two passes).** Load and unload are the kernel's
//! own atomic calls and their return codes are the verdicts
//! (`.claude/rules/rust.md`, "Check-and-act must be atomic"); the one
//! separate read, `initstate` after an `EBUSY`, only classifies that `EBUSY`.
//! 1. `finit_module`: `Ok` loads it.
//! 2. On `EEXIST`, compare the loaded `version` with the build's: equal is
//!    done.
//! 3. A stale module is unloaded with `delete_module(O_NONBLOCK)`: `0` or
//!    `ENOENT` returns to step 1; `EWOULDBLOCK` / `EAGAIN` (references held,
//!    `kernel/module/main.c:744-751 @ v7.0`) is busy; `EBUSY` reads
//!    `initstate` (`:964-986`): `coming` / `going` is busy, absent returns to
//!    step 1, `live` retries the unload once — only a second `EBUSY` on a
//!    `live` module is a loaded build with no `module_exit` (`:818-825`),
//!    which only a reboot replaces.
//!
//! A busy refusal names the holder class: a process with `/dev/ovd_fwd` open
//! (pid and `comm`), or socket references still being freed / a context
//! reference not visible in `/proc` (`refcnt` above zero with no fd holder).
//! With `--busy-ok` (the hooks' form) a busy module prints the same message
//! and exits 0, leaving the stale verdict to the `serve` fixtures; every
//! other outcome is the strict form's. No outcome for a build from this
//! source names a reboot.
//!
//! This is build/dev orchestration: it imports no `overdrive-*` crate
//! (`.claude/rules/development.md` § "xtask is build / test / dev
//! orchestration"). The `version` reading is spelled here and in
//! `overdrive-testing` identically.

use std::fmt;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, WrapErr, bail};

use crate::guest_kit_prepare::forwarder_module_path;

/// Where a loaded `ovd_fwd` exposes itself.
pub const LOADED_MODULE_DIR: &str = "/sys/module/ovd_fwd";
/// Its `version` attribute, the tree digest (`kernel/module/main.c:607 @ v7.0`).
pub const LOADED_VERSION: &str = "/sys/module/ovd_fwd/version";
/// Its `initstate` attribute (`kernel/module/main.c:964-986 @ v7.0`).
pub const LOADED_INITSTATE: &str = "/sys/module/ovd_fwd/initstate";
/// Its reference count (`kernel/module/main.c:889-896 @ v7.0`).
pub const LOADED_REFCNT: &str = "/sys/module/ovd_fwd/refcnt";
/// The forwarder's control device; an open descriptor is an open context.
pub const CONTROL_DEVICE: &str = "/dev/ovd_fwd";

/// What the step did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    /// `finit_module` loaded this workspace's build where none was loaded.
    Loaded,
    /// A module was loaded already and its `version` equals this build's.
    AlreadyCurrent,
    /// A stale idle module was unloaded and this workspace's build loaded.
    Reloaded {
        /// The unloaded module's `version`.
        previous: String,
    },
    /// `--busy-ok`: a stale module is busy; nothing was changed.
    BusyTolerated(Busy),
}

/// Who holds a busy module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HolderClass {
    /// Processes with the control device open: each `(pid, comm)`.
    OpenContext(Vec<(u32, String)>),
    /// `refcnt` above zero with no process holding the device open: socket
    /// references still being freed, or a context reference not visible in
    /// `/proc` (an fd in flight via `SCM_RIGHTS`, an `io_uring` fixed file).
    UnseenReferences {
        /// The module's `refcnt`, when readable.
        refcnt: Option<u64>,
    },
    /// `EBUSY` while the module is initialising or going.
    Transitioning,
}

/// A busy stale module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Busy {
    /// The loaded module's `version`.
    pub loaded: String,
    /// This workspace's build's `version`.
    pub built: String,
    /// Who holds it.
    pub holders: HolderClass,
}

impl fmt::Display for Busy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ovd_fwd is busy; retry when its holders are gone (loaded version {}, this \
             workspace's build version {}): ",
            self.loaded, self.built
        )?;
        match &self.holders {
            HolderClass::OpenContext(holders) => {
                write!(f, "an open forwarder context held by")?;
                for (pid, comm) in holders {
                    write!(f, " pid {pid} ({comm})")?;
                }
                Ok(())
            }
            HolderClass::UnseenReferences { refcnt } => write!(
                f,
                "socket references still being freed, or a context reference not visible in \
                 /proc (refcnt {}); if they persist, a socket that had a ULP installed after \
                 attach holds its reference for the boot and the module has logged it as a defect",
                refcnt.map_or_else(|| "unreadable".to_owned(), |r| r.to_string())
            ),
            HolderClass::Transitioning => write!(f, "the module is initialising or going"),
        }
    }
}

/// Why the step refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    /// No built module at the contract path.
    #[error(
        "no ovd_fwd.ko built from this workspace at {path}; run `cargo xtask kmod-build` first"
    )]
    NotBuilt {
        /// The contract path.
        path: PathBuf,
    },
    /// The built module carries no `version`.
    #[error("{path} carries no version (kmod-build compiles the tree digest in as MODULE_VERSION)")]
    NoBuiltVersion {
        /// The built module.
        path: PathBuf,
    },
    /// The stale module is busy (strict form).
    #[error("{0}")]
    Busy(Busy),
    /// A loaded build with no `module_exit`: never unloadable.
    #[error(
        "the loaded ovd_fwd (version {loaded}) has no module_exit and cannot be unloaded; only a \
         reboot of this host replaces it with this workspace's build (version {built})"
    )]
    NoModuleExit {
        /// The loaded module's `version`.
        loaded: String,
        /// This workspace's build's `version`.
        built: String,
    },
    /// The second pass found another workspace's build loaded again.
    #[error(
        "race between workspaces: after unloading, another build (version {loaded}) was loaded \
         before this workspace's (version {built}); rerun `cargo xtask kmod-load`"
    )]
    Race {
        /// The version loaded by the other workspace.
        loaded: String,
        /// This workspace's build's `version`.
        built: String,
    },
    /// A kernel call or sysfs read failed with an errno no step handles.
    #[error("{call}: errno {errno}")]
    Os {
        /// The call.
        call: &'static str,
        /// Its errno.
        errno: i32,
    },
}

/// A module's `initstate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitState {
    /// `live`.
    Live,
    /// `coming`.
    Coming,
    /// `going`.
    Going,
}

/// The kernel surface the steps act on. `Err` carries the raw errno; a
/// sysfs read returns `Ok(None)` when the attribute is absent (`ENOENT`).
pub trait ModuleKernel {
    /// `finit_module` of the built `.ko`.
    fn finit(&mut self) -> std::result::Result<(), i32>;
    /// `delete_module("ovd_fwd", O_NONBLOCK)`.
    fn delete_nonblock(&mut self) -> std::result::Result<(), i32>;
    /// [`LOADED_VERSION`], trimmed.
    fn loaded_version(&mut self) -> std::result::Result<Option<String>, i32>;
    /// [`LOADED_INITSTATE`].
    fn initstate(&mut self) -> std::result::Result<Option<InitState>, i32>;
    /// [`LOADED_REFCNT`].
    fn refcnt(&mut self) -> Option<u64>;
    /// Processes with [`CONTROL_DEVICE`] open, as `(pid, comm)`.
    fn context_holders(&mut self) -> Vec<(u32, String)>;
}

/// The `version=` modinfo field of a module image.
///
/// The entry at the start of a NUL-delimited modinfo string (so not the tail
/// of `srcversion=`), up to its NUL. `None` when absent, unterminated, empty
/// or not UTF-8.
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

/// What one unload attempt concluded.
enum Unload {
    /// Gone: back to step 1.
    Gone,
    /// Busy, with its holder class.
    Busy(HolderClass),
    /// A live module with no `module_exit`.
    NoExit,
}

fn busy_holders(kernel: &mut dyn ModuleKernel) -> HolderClass {
    let holders = kernel.context_holders();
    if holders.is_empty() {
        HolderClass::UnseenReferences { refcnt: kernel.refcnt() }
    } else {
        HolderClass::OpenContext(holders)
    }
}

/// Step 3: unload a stale module, classifying `EBUSY` through `initstate`.
fn unload(kernel: &mut dyn ModuleKernel) -> std::result::Result<Unload, LoadError> {
    let mut retried_live = false;
    loop {
        match kernel.delete_nonblock() {
            Ok(()) => return Ok(Unload::Gone),
            Err(e) if e == libc::ENOENT => return Ok(Unload::Gone),
            Err(e) if e == libc::EWOULDBLOCK || e == libc::EAGAIN => {
                return Ok(Unload::Busy(busy_holders(kernel)));
            }
            Err(e) if e == libc::EBUSY => {
                match kernel.initstate().map_err(|errno| LoadError::Os {
                    call: "read /sys/module/ovd_fwd/initstate",
                    errno,
                })? {
                    None => return Ok(Unload::Gone),
                    Some(InitState::Coming | InitState::Going) => {
                        return Ok(Unload::Busy(HolderClass::Transitioning));
                    }
                    Some(InitState::Live) if retried_live => return Ok(Unload::NoExit),
                    Some(InitState::Live) => retried_live = true,
                }
            }
            Err(errno) => return Err(LoadError::Os { call: "delete_module(ovd_fwd)", errno }),
        }
    }
}

/// The steps over `kernel`, for the build whose `version` is `built`.
///
/// # Errors
///
/// [`LoadError::Busy`] (unless `busy_ok`), [`LoadError::NoModuleExit`],
/// [`LoadError::Race`], or [`LoadError::Os`].
pub fn load(
    kernel: &mut dyn ModuleKernel,
    built: &str,
    busy_ok: bool,
) -> std::result::Result<LoadOutcome, LoadError> {
    let mut unloaded: Option<String> = None;
    for pass in 0..2 {
        match kernel.finit() {
            Ok(()) => {
                return Ok(unloaded
                    .map_or(LoadOutcome::Loaded, |previous| LoadOutcome::Reloaded { previous }));
            }
            Err(e) if e == libc::EEXIST => {}
            Err(errno) => return Err(LoadError::Os { call: "finit_module(ovd_fwd.ko)", errno }),
        }
        let Some(loaded) = kernel
            .loaded_version()
            .map_err(|errno| LoadError::Os { call: "read /sys/module/ovd_fwd/version", errno })?
        else {
            // The `EEXIST` module finished going between the two calls.
            continue;
        };
        if loaded == built {
            return Ok(LoadOutcome::AlreadyCurrent);
        }
        if pass == 1 {
            return Err(LoadError::Race { loaded, built: built.to_owned() });
        }
        match unload(kernel)? {
            Unload::Gone => unloaded = Some(loaded),
            Unload::Busy(holders) => {
                let busy = Busy { loaded, built: built.to_owned(), holders };
                return if busy_ok {
                    Ok(LoadOutcome::BusyTolerated(busy))
                } else {
                    Err(LoadError::Busy(busy))
                };
            }
            Unload::NoExit => {
                return Err(LoadError::NoModuleExit { loaded, built: built.to_owned() });
            }
        }
    }
    // Two passes each found the module gone between `EEXIST` and the read.
    Err(LoadError::Os { call: "finit_module(ovd_fwd.ko) after two passes", errno: libc::EEXIST })
}

/// Runs the step for `workspace_root`.
///
/// # Errors
///
/// A non-Linux host, no built module, or any [`LoadError`].
pub fn run(workspace_root: &Path, busy_ok: bool) -> Result<LoadOutcome> {
    if !cfg!(target_os = "linux") {
        bail!(
            "kmod-load loads into the running Linux kernel; run it under `cargo xtask lima run --`"
        );
    }
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .wrap_err("reading the running kernel release")?
        .trim()
        .to_owned();
    let module = forwarder_module_path(workspace_root, &release);
    let image = match std::fs::read(&module) {
        Ok(image) => image,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(LoadError::NotBuilt { path: module }.into());
        }
        Err(e) => return Err(e).wrap_err_with(|| format!("reading {}", module.display())),
    };
    let Some(built) = version_in(&image) else {
        return Err(LoadError::NoBuiltVersion { path: module }.into());
    };
    let mut kernel = HostKernel { module: module.clone() };
    let outcome = load(&mut kernel, &built, busy_ok)?;
    match &outcome {
        LoadOutcome::Loaded => eprintln!("kmod-load: loaded {}", module.display()),
        LoadOutcome::AlreadyCurrent => eprintln!("kmod-load: ovd_fwd version {built} is loaded"),
        LoadOutcome::Reloaded { previous } => {
            eprintln!("kmod-load: unloaded idle ovd_fwd version {previous}; loaded {built}");
        }
        LoadOutcome::BusyTolerated(busy) => eprintln!("kmod-load (--busy-ok): {busy}"),
    }
    Ok(outcome)
}

/// The running kernel.
struct HostKernel {
    #[cfg_attr(
        not(target_os = "linux"),
        expect(dead_code, reason = "read only by the Linux finit_module")
    )]
    module: PathBuf,
}

fn read_sysfs(path: &str) -> std::result::Result<Option<String>, i32> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s.trim().to_owned())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.raw_os_error().unwrap_or(libc::EIO)),
    }
}

impl ModuleKernel for HostKernel {
    #[cfg(target_os = "linux")]
    fn finit(&mut self) -> std::result::Result<(), i32> {
        use std::os::fd::AsRawFd;

        let file =
            std::fs::File::open(&self.module).map_err(|e| e.raw_os_error().unwrap_or(libc::EIO))?;
        let params = c"";
        // SAFETY: `finit_module(fd, param_values, flags)` reads the module
        // from a descriptor we own for the duration of the call and a valid
        // NUL-terminated (empty) parameter string; flags 0.
        let rc = unsafe {
            libc::syscall(libc::SYS_finit_module, file.as_raw_fd(), params.as_ptr(), 0_i32)
        };
        if rc == 0 { Ok(()) } else { Err(last_errno()) }
    }

    #[cfg(not(target_os = "linux"))]
    fn finit(&mut self) -> std::result::Result<(), i32> {
        Err(libc::ENOSYS)
    }

    #[cfg(target_os = "linux")]
    fn delete_nonblock(&mut self) -> std::result::Result<(), i32> {
        // SAFETY: a valid NUL-terminated module name and the O_NONBLOCK flag;
        // the kernel's reference check decides whether anything is unloaded.
        let rc = unsafe {
            libc::syscall(libc::SYS_delete_module, c"ovd_fwd".as_ptr(), libc::O_NONBLOCK)
        };
        if rc == 0 { Ok(()) } else { Err(last_errno()) }
    }

    #[cfg(not(target_os = "linux"))]
    fn delete_nonblock(&mut self) -> std::result::Result<(), i32> {
        Err(libc::ENOSYS)
    }

    fn loaded_version(&mut self) -> std::result::Result<Option<String>, i32> {
        read_sysfs(LOADED_VERSION)
    }

    fn initstate(&mut self) -> std::result::Result<Option<InitState>, i32> {
        Ok(read_sysfs(LOADED_INITSTATE)?.map(|s| match s.as_str() {
            "coming" => InitState::Coming,
            "going" => InitState::Going,
            _ => InitState::Live,
        }))
    }

    fn refcnt(&mut self) -> Option<u64> {
        read_sysfs(LOADED_REFCNT).ok().flatten().and_then(|s| s.parse().ok())
    }

    fn context_holders(&mut self) -> Vec<(u32, String)> {
        let mut holders = Vec::new();
        let Ok(procs) = std::fs::read_dir("/proc") else { return holders };
        for entry in procs.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else {
                continue;
            };
            let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else { continue };
            let holds = fds.flatten().any(|fd| {
                std::fs::read_link(fd.path())
                    .is_ok_and(|target| target == Path::new(CONTROL_DEVICE))
            });
            if holds {
                let comm = std::fs::read_to_string(entry.path().join("comm"))
                    .map_or_else(|_| "?".to_owned(), |c| c.trim().to_owned());
                holders.push((pid, comm));
            }
        }
        holders
    }
}

#[cfg(target_os = "linux")]
fn last_errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    /// A scripted kernel: each call pops its next scripted answer, and every
    /// call is recorded so a test asserts what was attempted.
    #[derive(Default)]
    struct Scripted {
        finit: VecDeque<std::result::Result<(), i32>>,
        delete: VecDeque<std::result::Result<(), i32>>,
        version: VecDeque<Option<String>>,
        initstate: VecDeque<Option<InitState>>,
        refcnt: Option<u64>,
        holders: Vec<(u32, String)>,
        calls: Vec<&'static str>,
    }

    impl ModuleKernel for Scripted {
        fn finit(&mut self) -> std::result::Result<(), i32> {
            self.calls.push("finit");
            self.finit.pop_front().expect("a scripted finit")
        }
        fn delete_nonblock(&mut self) -> std::result::Result<(), i32> {
            self.calls.push("delete");
            self.delete.pop_front().expect("a scripted delete")
        }
        fn loaded_version(&mut self) -> std::result::Result<Option<String>, i32> {
            self.calls.push("version");
            Ok(self.version.pop_front().expect("a scripted version"))
        }
        fn initstate(&mut self) -> std::result::Result<Option<InitState>, i32> {
            self.calls.push("initstate");
            Ok(self.initstate.pop_front().expect("a scripted initstate"))
        }
        fn refcnt(&mut self) -> Option<u64> {
            self.refcnt
        }
        fn context_holders(&mut self) -> Vec<(u32, String)> {
            self.holders.clone()
        }
    }

    fn v(s: &str) -> Option<String> {
        Some(s.to_owned())
    }

    #[test]
    fn version_is_the_entry_at_a_nul_boundary_not_the_tail_of_srcversion() {
        let image = b"license=GPL\0srcversion=4F2A9C0E1B7D3A65C8E21F0\0version=9c1e\0";
        assert_eq!(version_in(image).as_deref(), Some("9c1e"));
        assert_eq!(version_in(b"license=GPL\0srcversion=4F2A\0"), None);
        assert_eq!(version_in(b"version=X"), None);
        assert_eq!(version_in(b"license=GPL\0version=\0"), None);
    }

    #[test]
    fn an_absent_module_is_loaded() {
        let mut k = Scripted { finit: [Ok(())].into(), ..Scripted::default() };
        assert_eq!(load(&mut k, "aa", false), Ok(LoadOutcome::Loaded));
        assert_eq!(k.calls, ["finit"]);
    }

    #[test]
    fn a_current_module_is_done_without_an_unload() {
        let mut k = Scripted {
            finit: [Err(libc::EEXIST)].into(),
            version: [v("aa")].into(),
            ..Scripted::default()
        };
        assert_eq!(load(&mut k, "aa", false), Ok(LoadOutcome::AlreadyCurrent));
        assert_eq!(k.calls, ["finit", "version"]);
    }

    #[test]
    fn a_stale_idle_module_is_unloaded_then_this_build_loaded() {
        let mut k = Scripted {
            finit: [Err(libc::EEXIST), Ok(())].into(),
            version: [v("bb")].into(),
            delete: [Ok(())].into(),
            ..Scripted::default()
        };
        assert_eq!(
            load(&mut k, "aa", false),
            Ok(LoadOutcome::Reloaded { previous: "bb".to_owned() })
        );
        assert_eq!(k.calls, ["finit", "version", "delete", "finit"]);
    }

    #[test]
    fn a_stale_module_another_process_unloaded_first_is_loaded() {
        let mut k = Scripted {
            finit: [Err(libc::EEXIST), Ok(())].into(),
            version: [v("bb")].into(),
            delete: [Err(libc::ENOENT)].into(),
            ..Scripted::default()
        };
        assert!(matches!(load(&mut k, "aa", false), Ok(LoadOutcome::Reloaded { .. })));
    }

    #[test]
    fn a_stale_busy_module_is_refused_naming_the_open_context_and_never_a_reboot() {
        let mut k = Scripted {
            finit: [Err(libc::EEXIST)].into(),
            version: [v("bb")].into(),
            delete: [Err(libc::EWOULDBLOCK)].into(),
            holders: vec![(4242, "overdrive".to_owned())],
            ..Scripted::default()
        };
        let err = load(&mut k, "aa", false).expect_err("a busy stale module is refused");
        let msg = err.to_string();
        assert!(
            matches!(&err, LoadError::Busy(b) if matches!(b.holders, HolderClass::OpenContext(_)))
        );
        assert!(msg.contains("busy; retry") && msg.contains("pid 4242 (overdrive)"), "{msg}");
        assert!(msg.contains("bb") && msg.contains("aa"), "{msg}");
        assert!(!msg.contains("reboot"), "{msg}");
        assert_eq!(k.calls, ["finit", "version", "delete"], "no load is attempted when busy");
    }

    #[test]
    fn busy_with_no_fd_holder_names_unseen_references_with_the_refcnt() {
        let mut k = Scripted {
            finit: [Err(libc::EEXIST)].into(),
            version: [v("bb")].into(),
            delete: [Err(libc::EAGAIN)].into(),
            refcnt: Some(2),
            ..Scripted::default()
        };
        let msg = load(&mut k, "aa", false).expect_err("busy").to_string();
        assert!(msg.contains("socket references still being freed"), "{msg}");
        assert!(msg.contains("not visible in /proc") && msg.contains("refcnt 2"), "{msg}");
    }

    #[test]
    fn busy_ok_tolerates_a_busy_module_and_exits_successfully() {
        let mut k = Scripted {
            finit: [Err(libc::EEXIST)].into(),
            version: [v("bb")].into(),
            delete: [Err(libc::EWOULDBLOCK)].into(),
            refcnt: Some(1),
            ..Scripted::default()
        };
        assert!(matches!(load(&mut k, "aa", true), Ok(LoadOutcome::BusyTolerated(_))));
    }

    #[test]
    fn ebusy_while_coming_or_going_is_busy() {
        for state in [InitState::Coming, InitState::Going] {
            let mut k = Scripted {
                finit: [Err(libc::EEXIST)].into(),
                version: [v("bb")].into(),
                delete: [Err(libc::EBUSY)].into(),
                initstate: [Some(state)].into(),
                ..Scripted::default()
            };
            assert!(
                matches!(
                    load(&mut k, "aa", false),
                    Err(LoadError::Busy(Busy { holders: HolderClass::Transitioning, .. }))
                ),
                "{state:?}"
            );
        }
    }

    #[test]
    fn ebusy_then_an_absent_initstate_means_gone_and_loads() {
        let mut k = Scripted {
            finit: [Err(libc::EEXIST), Ok(())].into(),
            version: [v("bb")].into(),
            delete: [Err(libc::EBUSY)].into(),
            initstate: [None].into(),
            ..Scripted::default()
        };
        assert!(matches!(load(&mut k, "aa", false), Ok(LoadOutcome::Reloaded { .. })));
    }

    #[test]
    fn ebusy_on_live_is_confirmed_by_a_second_unload_before_any_reboot_verdict() {
        // The first EBUSY came from a `coming` module whose init completed
        // before the read: the second unload succeeds, so no reboot verdict.
        let mut k = Scripted {
            finit: [Err(libc::EEXIST), Ok(())].into(),
            version: [v("bb")].into(),
            delete: [Err(libc::EBUSY), Ok(())].into(),
            initstate: [Some(InitState::Live)].into(),
            ..Scripted::default()
        };
        assert!(matches!(load(&mut k, "aa", false), Ok(LoadOutcome::Reloaded { .. })));
        assert_eq!(k.calls, ["finit", "version", "delete", "initstate", "delete", "finit"]);
    }

    #[test]
    fn a_second_ebusy_on_a_live_module_is_no_module_exit_and_busy_ok_does_not_soften_it() {
        for busy_ok in [false, true] {
            let mut k = Scripted {
                finit: [Err(libc::EEXIST)].into(),
                version: [v("bb")].into(),
                delete: [Err(libc::EBUSY), Err(libc::EBUSY)].into(),
                initstate: [Some(InitState::Live), Some(InitState::Live)].into(),
                ..Scripted::default()
            };
            let err = load(&mut k, "aa", busy_ok).expect_err("permanent EBUSY");
            assert!(matches!(err, LoadError::NoModuleExit { .. }), "{err:?}");
            assert!(err.to_string().contains("no module_exit"), "{err}");
        }
    }

    #[test]
    fn a_second_pass_that_finds_another_build_reports_a_race_without_a_second_unload() {
        let mut k = Scripted {
            finit: [Err(libc::EEXIST), Err(libc::EEXIST)].into(),
            version: [v("bb"), v("cc")].into(),
            delete: [Ok(())].into(),
            ..Scripted::default()
        };
        let err = load(&mut k, "aa", false).expect_err("race");
        assert_eq!(err, LoadError::Race { loaded: "cc".to_owned(), built: "aa".to_owned() });
        assert_eq!(k.calls, ["finit", "version", "delete", "finit", "version"]);
    }

    #[test]
    fn an_unhandled_finit_errno_is_reported() {
        let mut k = Scripted { finit: [Err(libc::EPERM)].into(), ..Scripted::default() };
        assert!(
            matches!(load(&mut k, "aa", false), Err(LoadError::Os { errno, .. }) if errno == libc::EPERM)
        );
    }
}
