//! Host [`Vmm`] binding — real `cloud-hypervisor` process spawn, the
//! per-launch `FICLONE` rootfs clone, and the Earned-Trust probe.
//!
//! Production binding of the [`Vmm`] port trait (ADR-0082 §D1). The sim
//! counterpart is `overdrive_sim::SimVmm`. See
//! `overdrive_core::traits::vmm::Vmm` for the full port-trait contract
//! (preconditions, postconditions, edge cases, observable invariants) —
//! this adapter implements that contract; it does not restate it.
//!
//! # `terminate` cooperates with `create`'s reaper
//!
//! [`Vmm::terminate`] takes only a bare [`VmControl`] — no handle to the
//! spawned [`tokio::process::Child`], which is exclusively owned by the
//! background task `create` spawns (only one thing may ever call
//! `Child::wait`, or the two callers race the kernel's zombie-reap). So
//! `create` also installs a per-pid [`VmProcessState`] into `self.live`:
//! a `Notify` `terminate` uses to ask the reaper to kill the child now,
//! and a `watch::Sender<Option<VmmExit>>` that lets ANY later `terminate`
//! call — before or after the process actually exits — observe the
//! outcome without racing the reaper's own `child.wait()`.
//!
//! # `FICLONE` is real, self-applied, and never `cp`
//!
//! Per ADR-0082 §D5's "self-application" rule: the boot-time probe proves
//! the substrate is reflink-capable ONCE; `create`'s per-launch clone
//! re-proves it on every launch via the same ioctl, directly — never
//! `cp --reflink=auto`, which silently degrades to a full copy on
//! `EOPNOTSUPP`/`EXDEV` with no error (P4: 0.015s/+0MiB vs 3.970s/+4096MiB).
//! [`rustix::fs::ioctl_ficlone`] is a SAFE wrapper (the `unsafe` is
//! encapsulated inside `rustix`), which is what lets this crate call it
//! under its crate-wide `#![forbid(unsafe_code)]`.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::traits::driver::STDERR_TAIL_LINES;
use overdrive_core::traits::vmm::{
    Result, VmControl, VmExitWatch, VmProcess, VmTermination, Vmm, VmmError, VmmExit, VmmProbeError,
};
use overdrive_core::vm::config::{DiskAttachment, VmConfig};
use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{ChildStderr, Command};
use tokio::sync::{Notify, oneshot, watch};

/// Default probe target for §D5 scenario 1 (VM image directory reflink
/// capability) — overridable via [`CloudHypervisorVmm::with_image_dir`].
const DEFAULT_IMAGE_DIR: &str = "/srv/vm";
/// Default probe target for §D5 scenario 5 (run-directory root
/// creatable/bindable) — overridable via
/// [`CloudHypervisorVmm::with_run_dir_root`].
const DEFAULT_RUN_DIR_ROOT: &str = "/run/overdrive/vm";
/// `/dev/kvm`'s well-known path — named once so the probe and its
/// diagnostics never drift.
const KVM_DEVICE_PATH: &str = "/dev/kvm";
/// Bytes written for the executed FICLONE self-test (§D5 scenario 1) —
/// matches `overdrive_testing::vm_fixture`'s own probe size so a sparse
/// file can never trivially "succeed."
const REFLINK_PROBE_BYTES: usize = 8 * 1024 * 1024;
/// Bounded yield budget for catching trailing stderr output after the
/// process exits, before snapshotting the tail ring. Mirrors
/// `overdrive_worker::driver::spawn_exit_watcher`'s cooperative-yield
/// pattern — never a `Clock::sleep` (per `.claude/rules/development.md`
/// § "Production code is not shaped by simulation").
const STDERR_DRAIN_MAX_YIELDS: u32 = 16;

/// Per-pid bookkeeping `create` installs so a later, independently-called
/// `terminate` can observe/await/force the SAME spawned process's exit.
/// See the module doc's "`terminate` cooperates with `create`'s reaper".
struct VmProcessState {
    /// Fired (at most meaningfully once) by `terminate` to ask the
    /// reaper task to kill the child now, instead of waiting for it to
    /// exit on its own.
    kill_now: Notify,
    /// `None` until the reaper's `child.wait()` resolves; `Some` exactly
    /// once thereafter. A `watch` (not a `oneshot`) so a `terminate`
    /// call arriving AFTER the process already exited still observes
    /// the outcome immediately via a fresh subscriber's `borrow()`, and
    /// one arriving BEFORE can `changed()`-await it — both without
    /// racing the reaper's own `child.wait()`.
    outcome: watch::Sender<Option<VmmExit>>,
}

/// Production [`Vmm`] binding: real `cloud-hypervisor` process spawn.
///
/// The sim counterpart is `overdrive_sim::SimVmm` — swap at the wiring
/// boundary; no call site should need both.
///
/// # Construction
///
/// ```
/// use overdrive_host::CloudHypervisorVmm;
/// let vmm = CloudHypervisorVmm::new();
/// ```
#[derive(Clone)]
pub struct CloudHypervisorVmm {
    /// Resolved (or bare, `PATH`-relative) `cloud-hypervisor` binary.
    binary: PathBuf,
    /// Probe target for §D5 scenario 1.
    image_dir: PathBuf,
    /// Probe target for §D5 scenario 5.
    run_dir_root: PathBuf,
    /// Live spawned processes, keyed by pid. See [`VmProcessState`].
    live: Arc<Mutex<BTreeMap<u32, Arc<VmProcessState>>>>,
}

impl Default for CloudHypervisorVmm {
    fn default() -> Self {
        Self::new()
    }
}

impl CloudHypervisorVmm {
    /// Construct with the default probe targets (`/srv/vm`,
    /// `/run/overdrive/vm`) and a bare, `PATH`-resolved `cloud-hypervisor`
    /// binary.
    #[must_use]
    pub fn new() -> Self {
        Self {
            binary: PathBuf::from("cloud-hypervisor"),
            image_dir: PathBuf::from(DEFAULT_IMAGE_DIR),
            run_dir_root: PathBuf::from(DEFAULT_RUN_DIR_ROOT),
            live: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// **TEST-ONLY scoping.** Override the `cloud-hypervisor` binary
    /// path. Not a port-trait injection builder (see
    /// `RealCgroupFs::with_probe_root`'s docs for why that distinction
    /// matters here) — an internal adapter knob on a single field.
    #[must_use]
    pub fn with_binary(mut self, binary: PathBuf) -> Self {
        self.binary = binary;
        self
    }

    /// **TEST-ONLY scoping.** Override the probe's VM-image-directory
    /// reflink-capability target (§D5 scenario 1).
    #[must_use]
    pub fn with_image_dir(mut self, dir: PathBuf) -> Self {
        self.image_dir = dir;
        self
    }

    /// **TEST-ONLY scoping.** Override the probe's run-directory-root
    /// creatable/bindable target (§D5 scenario 5).
    #[must_use]
    pub fn with_run_dir_root(mut self, dir: PathBuf) -> Self {
        self.run_dir_root = dir;
        self
    }
}

#[async_trait]
impl Vmm for CloudHypervisorVmm {
    fn kind(&self) -> &'static str {
        "cloud-hypervisor"
    }

    async fn probe(&self) -> std::result::Result<(), VmmProbeError> {
        let image_dir = self.image_dir.clone();
        spawn_blocking_probe(move || probe_reflink(&image_dir)).await?;

        probe_cloud_hypervisor_capable(&self.binary).await?;

        spawn_blocking_probe(probe_kvm_reachable).await?;

        let run_dir_root = self.run_dir_root.clone();
        spawn_blocking_probe(move || probe_run_dir(&run_dir_root)).await?;

        Ok(())
    }

    async fn create(&self, config: &VmConfig) -> Result<VmProcess> {
        let master = config.rootfs.master().to_path_buf();
        let clone_dest = config.rootfs.clone_dest().to_path_buf();
        tokio::task::spawn_blocking(move || ficlone_rootfs(&master, &clone_dest))
            .await
            .map_err(|join_err| VmmError::create(format!("FICLONE task panicked: {join_err}")))??;

        let mut cmd = Command::new(&self.binary);
        cmd.arg("--cpus")
            .arg(format!("boot={}", config.vcpus))
            .arg("--memory")
            .arg(format!("size={}", config.memory.guest_bytes()))
            .arg("--kernel")
            .arg(config.kernel.path())
            .arg("--cmdline")
            .arg(config.cmdline.as_str())
            .arg("--disk")
            .arg(DiskAttachment::new(config.rootfs.clone_dest().to_path_buf(), false).to_disk_arg())
            .arg("--serial")
            .arg(format!("file={}", config.run_dir.console_log().display()))
            .arg("--console")
            .arg("off")
            .arg("--vsock")
            .arg(format!("cid=3,socket={}", config.run_dir.vsock_socket().display()))
            .arg("--api-socket")
            .arg(config.run_dir.api_socket())
            .arg("--seccomp")
            .arg(config.confinement.seccomp_arg())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(false);

        // `let-else` is deliberately NOT used here (unlike the `child.id()`
        // check below): the `Err` arm needs the `io::Error` detail, and
        // extracting it from a `let-else`-failed scrutinee would need an
        // `unwrap_err`-shaped call this workspace's lint profile forbids
        // outside tests.
        #[allow(clippy::manual_let_else, clippy::single_match_else)]
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(source) => {
                // §D6: the spawn failed after the clone succeeded — remove
                // it. No partial artifact escapes a failed `create`.
                let _ = tokio::fs::remove_file(config.rootfs.clone_dest()).await;
                return Err(VmmError::create(format!(
                    "spawning {} failed: {source}",
                    self.binary.display()
                )));
            }
        };

        let Some(pid) = child.id() else {
            let _ = tokio::fs::remove_file(config.rootfs.clone_dest()).await;
            return Err(VmmError::create(
                "spawned cloud-hypervisor child reported no pid".to_string(),
            ));
        };
        let stderr_pipe = child.stderr.take();

        let (exit_tx, exit_rx) = oneshot::channel::<VmmExit>();
        let (outcome_tx, _outcome_rx) = watch::channel::<Option<VmmExit>>(None);
        let state = Arc::new(VmProcessState { kill_now: Notify::new(), outcome: outcome_tx });
        self.live.lock().insert(pid, Arc::clone(&state));

        let live = Arc::clone(&self.live);
        tokio::spawn(async move {
            let ring: Arc<Mutex<VecDeque<String>>> =
                Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES)));
            let reader_handle =
                stderr_pipe.map(|pipe| spawn_stderr_tail_reader(pipe, Arc::clone(&ring)));

            let status = tokio::select! {
                biased;
                status = child.wait() => status,
                () = state.kill_now.notified() => {
                    let _ = child.start_kill();
                    child.wait().await
                }
            };

            if let Some(handle) = reader_handle {
                for _ in 0..STDERR_DRAIN_MAX_YIELDS {
                    if handle.is_finished() {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
                drop(handle);
            }
            let stderr_tail = {
                let guard = ring.lock();
                if guard.is_empty() {
                    None
                } else {
                    Some(guard.iter().cloned().collect::<Vec<_>>().join("\n"))
                }
            };

            let vmm_exit = classify_exit(status, stderr_tail);
            let _ = exit_tx.send(vmm_exit.clone());
            let _ = state.outcome.send(Some(vmm_exit));
            live.lock().remove(&pid);
        });

        Ok(VmProcess {
            control: VmControl { pid, api_socket: config.run_dir.api_socket() },
            exit: VmExitWatch::new(exit_rx),
        })
    }

    async fn terminate(&self, control: &VmControl, grace: Duration) -> Result<VmTermination> {
        let Some(state) = self.live.lock().get(&control.pid).cloned() else {
            // No record: either this adapter instance never spawned this
            // pid, or the reaper already observed and pruned it. Both
            // collapse to §D6's "already gone" edge case.
            return Ok(VmTermination::Killed);
        };

        let mut rx = state.outcome.subscribe();
        if rx.borrow().is_some() {
            // §D6: "already gone" is ALWAYS Killed, idempotently — even
            // when the process in fact exited cleanly before this call
            // observed it.
            return Ok(VmTermination::Killed);
        }

        if grace.is_zero() {
            // §D6: kill immediately, with no await on the grace window
            // itself (the reap still has to happen to report Killed).
            state.kill_now.notify_one();
            let _ = rx.changed().await;
            return Ok(VmTermination::Killed);
        }

        let exited_within_grace = tokio::select! {
            biased;
            changed = rx.changed() => changed.is_ok(),
            () = tokio::time::sleep(grace) => false,
        };

        if exited_within_grace {
            let exit = rx.borrow().clone().unwrap_or_else(|| {
                unreachable!("changed() resolves Ok only after outcome is Some")
            });
            return Ok(VmTermination::ExitedWithinGrace(exit));
        }

        state.kill_now.notify_one();
        let _ = rx.changed().await;
        Ok(VmTermination::Killed)
    }
}

// ---------------------------------------------------------------------
// create() helpers
// ---------------------------------------------------------------------

/// Clone `master` to `clone_dest` via the `FICLONE` ioctl on `master`'s
/// own filesystem — never `cp`. §D6 edge cases: an existing `clone_dest`
/// (a crashed prior launch) is REPLACED, never adopted; on ioctl failure
/// the (possibly empty) destination is removed, never left behind.
fn ficlone_rootfs(master: &Path, clone_dest: &Path) -> Result<()> {
    if clone_dest.exists() {
        std::fs::remove_file(clone_dest).map_err(VmmError::Io)?;
    }
    let src = std::fs::File::open(master).map_err(VmmError::Io)?;
    let dst = std::fs::File::options()
        .write(true)
        .create_new(true)
        .open(clone_dest)
        .map_err(VmmError::Io)?;
    if let Err(err) = rustix::fs::ioctl_ficlone(&dst, &src) {
        drop(dst);
        let _ = std::fs::remove_file(clone_dest);
        return Err(VmmError::Io(err.into()));
    }
    Ok(())
}

/// Classify a resolved (or failed-to-observe) child exit into the
/// adapter-agnostic [`VmmExit`] shape.
fn classify_exit(
    status: io::Result<std::process::ExitStatus>,
    stderr_tail: Option<String>,
) -> VmmExit {
    match status {
        Ok(status) => VmmExit {
            exit_code: status.code(),
            signal: status.signal().and_then(|s| u8::try_from(s).ok()),
            stderr_tail,
        },
        Err(_io_err) => VmmExit { exit_code: None, signal: None, stderr_tail },
    }
}

/// Spawn a task that reads `pipe` line-by-line into a shared bounded ring
/// (capacity [`STDERR_TAIL_LINES`]) — mirrors
/// `overdrive_worker::driver::spawn_stderr_tail_reader` exactly (same
/// non-blocking-snapshot rationale: a daemonised grandchild holding the
/// pipe open must never make `terminate`/the reaper hang waiting for EOF).
fn spawn_stderr_tail_reader(
    pipe: ChildStderr,
    ring: Arc<Mutex<VecDeque<String>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut reader = BufReader::new(pipe).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let mut guard = ring.lock();
            if guard.len() == STDERR_TAIL_LINES {
                guard.pop_front();
            }
            guard.push_back(line);
        }
    })
}

// ---------------------------------------------------------------------
// probe() helpers — ADR-0082 §D5's five fault-injection scenarios
// ---------------------------------------------------------------------

/// Runs a sync probe closure on the blocking pool — every scenario below
/// does real synchronous filesystem/ioctl work, which
/// `.claude/rules/development.md` § "No blocking `std::fs::*` inside
/// `async fn`" forbids running directly on the async body. A panic
/// inside the closure (never expected in practice) surfaces as
/// `RunDirUnusable` with a synthetic source — the closest-fitting
/// variant for "the probe infrastructure itself broke," not a substrate
/// lie.
async fn spawn_blocking_probe<F>(f: F) -> std::result::Result<(), VmmProbeError>
where
    F: FnOnce() -> std::result::Result<(), VmmProbeError> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_err) => Err(VmmProbeError::run_dir_unusable(
            PathBuf::new(),
            io::Error::other(format!("probe task panicked: {join_err}")),
        )),
    }
}

/// §D5 scenario 1 — an EXECUTED `FICLONE` self-test against `dir`, never
/// an `fstype` string comparison.
fn probe_reflink(dir: &Path) -> std::result::Result<(), VmmProbeError> {
    std::fs::create_dir_all(dir).map_err(|source| {
        VmmProbeError::reflink_unsupported(dir.to_path_buf(), probe_fstype(dir), source)
    })?;

    let probe = dir.join(format!(".overdrive-vmm-probe-{}", uuid::Uuid::new_v4()));
    let clone = probe.with_extension("clone");
    let cleanup = || {
        let _ = std::fs::remove_file(&probe);
        let _ = std::fs::remove_file(&clone);
    };

    if let Err(source) = std::fs::write(&probe, vec![0xCD_u8; REFLINK_PROBE_BYTES]) {
        cleanup();
        return Err(VmmProbeError::reflink_unsupported(
            dir.to_path_buf(),
            probe_fstype(dir),
            source,
        ));
    }

    let result = (|| -> io::Result<()> {
        let src = std::fs::File::open(&probe)?;
        let dst = std::fs::File::options().write(true).create_new(true).open(&clone)?;
        rustix::fs::ioctl_ficlone(&dst, &src).map_err(io::Error::from)
    })();

    cleanup();
    result.map_err(|source| {
        VmmProbeError::reflink_unsupported(dir.to_path_buf(), probe_fstype(dir), source)
    })
}

/// Best-effort `stat -f -c %T <dir>` for [`VmmProbeError::ReflinkUnsupported`]'s
/// diagnostic `fstype` field. INFALLIBLE by design (mirrors
/// `overdrive_testing::vm_fixture`'s `resolve_on_path`) — this is
/// diagnostic context assembled AFTER the real `FICLONE` probe already
/// failed, never the gating check itself.
fn probe_fstype(dir: &Path) -> String {
    std::process::Command::new("stat")
        .args(["-f", "-c", "%T"])
        .arg(dir)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

/// §D5 scenarios 2+3 — `cloud-hypervisor --help` carries `--landlock`,
/// AND the host kernel exposes the Landlock LSM
/// (`/sys/kernel/security/lsm`).
async fn probe_cloud_hypervisor_capable(binary: &Path) -> std::result::Result<(), VmmProbeError> {
    let version_output =
        Command::new(binary).arg("--version").output().await.map_err(|source| {
            VmmProbeError::landlock_flag_absent(
                binary.to_path_buf(),
                format!("spawn failed: {source}"),
            )
        })?;
    let version = String::from_utf8_lossy(&version_output.stdout).trim().to_owned();

    let help_output = Command::new(binary).arg("--help").output().await.map_err(|source| {
        VmmProbeError::landlock_flag_absent(binary.to_path_buf(), format!("spawn failed: {source}"))
    })?;
    let help_text = format!(
        "{}{}",
        String::from_utf8_lossy(&help_output.stdout),
        String::from_utf8_lossy(&help_output.stderr),
    );
    if !help_text.contains("--landlock") {
        return Err(VmmProbeError::landlock_flag_absent(binary.to_path_buf(), version));
    }

    let lsms = tokio::fs::read_to_string("/sys/kernel/security/lsm").await.unwrap_or_default();
    if !lsms.split(',').any(|lsm| lsm.trim() == "landlock") {
        return Err(VmmProbeError::landlock_lsm_absent(lsms.trim().to_owned()));
    }
    Ok(())
}

/// §D5 scenario 4 — `/dev/kvm` openable `O_RDWR` under the current
/// identity. `uid`/`gid`/`mode` in the resulting error are the DEVICE's
/// own ownership/permission bits (mirrors
/// `overdrive_testing::vm_fixture`'s `describe_kvm_device_mode`), not
/// the calling process's.
fn probe_kvm_reachable() -> std::result::Result<(), VmmProbeError> {
    let meta = std::fs::metadata(KVM_DEVICE_PATH);
    let (uid, gid, mode) =
        meta.as_ref().map_or((0, 0, 0), |m| (m.uid(), m.gid(), m.mode() & 0o777));
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(KVM_DEVICE_PATH)
        .map(|_handle| ())
        .map_err(|source| VmmProbeError::kvm_unreachable(uid, gid, mode, source))
}

/// §D5 scenario 5 — an EXECUTED `mkdir` → `bind` → `unlink` round-trip on
/// a probe-scoped subdirectory of `root`. Never asserts `fstype`
/// (ADR-0082 §D5's corrected scenario 5 — absence-after-reboot is what
/// the reap needs, not a filesystem-type comparison).
fn probe_run_dir(root: &Path) -> std::result::Result<(), VmmProbeError> {
    // Short identifier + a 1-char socket filename -- keeps headroom
    // against the UNIX-domain-socket `SUN_LEN` ceiling (108 bytes,
    // `sockaddr_un.sun_path`) even when `root` itself is long. A full
    // UUID + a descriptive socket filename can overrun it (observed:
    // `path must be shorter than SUN_LEN` against a real, long
    // `run_dir_root` on the metal box).
    let short_id = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let probe_dir = root.join(format!(".p{short_id}"));
    std::fs::create_dir_all(&probe_dir)
        .map_err(|source| VmmProbeError::run_dir_unusable(root.to_path_buf(), source))?;

    let socket_path = probe_dir.join("s");
    let bind_result = std::os::unix::net::UnixListener::bind(&socket_path);
    let cleanup_result = std::fs::remove_dir_all(&probe_dir);

    match bind_result {
        Ok(_listener) => cleanup_result
            .map_err(|source| VmmProbeError::run_dir_unusable(root.to_path_buf(), source)),
        Err(source) => {
            let _ = cleanup_result;
            Err(VmmProbeError::run_dir_unusable(root.to_path_buf(), source))
        }
    }
}
