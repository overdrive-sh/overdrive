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
//! with no `unsafe` block under its crate-wide `#![deny(unsafe_code)]`.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::traits::driver::ConfinementControl;
use overdrive_core::traits::vmm::{
    Result, VmControl, VmExitWatch, VmProcess, VmTermination, Vmm, VmmDiagnostics,
    VmmDiagnosticsWriter, VmmError, VmmExit, VmmProbeError,
};
use overdrive_core::vm::config::{DiskAttachment, VmConfig, VmNetworkAttachment};
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::{ChildStderr, Command};
use tokio::sync::{Notify, oneshot, watch};

mod launch_seccomp;

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
const REQUIRED_LAUNCH_TOOLS: [&str; 3] = ["prlimit", "setpriv", "ip"];
/// The Cloud Hypervisor child's descriptor for the per-launch TAP queue
/// (`--net fd=[3]`, D-295-R1/R2/R3; ADR-0127, ADR-0128, ADR-0129).
#[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 05-03")]
pub(crate) const VMM_TAP_QUEUE_FD: std::os::fd::RawFd = 3;

#[async_trait]
trait VmmProbeSubstrate: Send + Sync {
    async fn check_reflink(&self, image_dir: PathBuf) -> std::result::Result<(), VmmProbeError>;
    async fn check_cloud_hypervisor(
        &self,
        binary: PathBuf,
    ) -> std::result::Result<(), VmmProbeError>;
    async fn execute_launch_tool(&self, tool: &'static str) -> io::Result<()>;
    /// Prove at boot that the kernel accepts the exact VMM launch seccomp
    /// program and that an exec proceeds under it (D-295-R22, ADR-0143).
    #[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 05-02")]
    async fn check_launch_seccomp(&self) -> std::result::Result<(), VmmProbeError>;
    async fn check_kvm(&self) -> std::result::Result<(), VmmProbeError>;
    async fn check_run_dir(&self, run_dir_root: PathBuf) -> std::result::Result<(), VmmProbeError>;
}

struct RealVmmProbeSubstrate;

#[async_trait]
impl VmmProbeSubstrate for RealVmmProbeSubstrate {
    async fn check_reflink(&self, image_dir: PathBuf) -> std::result::Result<(), VmmProbeError> {
        spawn_blocking_probe(move || probe_reflink(&image_dir)).await
    }

    async fn check_cloud_hypervisor(
        &self,
        binary: PathBuf,
    ) -> std::result::Result<(), VmmProbeError> {
        probe_cloud_hypervisor_capable(&binary).await
    }

    async fn execute_launch_tool(&self, tool: &'static str) -> io::Result<()> {
        Command::new(tool).arg("--version").output().await.map(|_| ())
    }

    #[expect(clippy::todo, reason = "RED scaffold — DELIVER step 05-02")]
    async fn check_launch_seccomp(&self) -> std::result::Result<(), VmmProbeError> {
        todo!("RED scaffold: D-295-R22 check_launch_seccomp — DELIVER step 05-02")
    }

    async fn check_kvm(&self) -> std::result::Result<(), VmmProbeError> {
        spawn_blocking_probe(probe_kvm_reachable).await
    }

    async fn check_run_dir(&self, run_dir_root: PathBuf) -> std::result::Result<(), VmmProbeError> {
        spawn_blocking_probe(move || probe_run_dir(&run_dir_root)).await
    }
}

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
    /// Production boundary for the external operations composed by
    /// [`Vmm::probe`]. The default binding executes the real host probes.
    probe_substrate: Arc<dyn VmmProbeSubstrate>,
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
            probe_substrate: Arc::new(RealVmmProbeSubstrate),
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

    /// Every path a spawn of [`Self::binary`] would have consulted — the
    /// evidence `VmmError::HypervisorAbsent` carries so an operator can
    /// see WHERE the platform looked, not merely that it failed.
    ///
    /// An absolute (or explicitly-relative) binary resolves to exactly
    /// itself; a bare name resolves against each `PATH` entry in order,
    /// which is precisely what `execvp` would have walked.
    fn searched_binary_paths(&self) -> Vec<String> {
        if self.binary.components().count() > 1 {
            return vec![self.binary.display().to_string()];
        }
        let Some(path_var) = std::env::var_os("PATH") else {
            return vec![self.binary.display().to_string()];
        };
        std::env::split_paths(&path_var)
            .map(|dir| dir.join(&self.binary).display().to_string())
            .collect()
    }

    /// Whether the hypervisor binary resolves to an existing file the way
    /// `execvp` would. With the confinement wrapper prepended (§(c)),
    /// `cmd.spawn()` spawns `ip` for a mesh VM and `prlimit` otherwise,
    /// so a genuinely-absent
    /// `cloud-hypervisor` no longer surfaces as a spawn-time `NotFound`
    /// (`setpriv` would instead fail to exec it deep in the chain — §(c)
    /// consequence 1). This pre-check, run BEFORE the wrapper spawn, keeps
    /// `HypervisorAbsent` naming CH's own absence and its searched paths.
    fn hypervisor_present(&self) -> bool {
        self.searched_binary_paths().iter().any(|p| Path::new(p).exists())
    }

    /// Build the confined `cloud-hypervisor` spawn command (§(c)): the
    /// `prlimit`/`setpriv` wrapper prefix, the hypervisor binary, its args
    /// (including `--seccomp`), then `--landlock` and the complete ordered
    /// explicit rules from `VmConfig::landlock_rules`. The FLAG literal
    /// `--landlock-rules` is rendered HERE — the sole site the 01-10 dst-lint
    /// clause sanctions; each rule VALUE comes from the pure
    /// [`LandlockRule::to_rule_arg`].
    /// Extracted from `create` purely to keep it within the line budget.
    fn build_confined_command(&self, config: &VmConfig, wrapper: &[String]) -> Command {
        let (program, prefix_args) = network_launch_prefix(config.network.as_ref(), wrapper);
        let mut cmd = Command::new(program);
        cmd.args(prefix_args);
        cmd.arg(&self.binary)
            .arg("--cpus")
            .arg(format!("boot={}", config.vcpus))
            .arg("--memory")
            .arg(format!("size={}", config.memory.guest_bytes()))
            // §(c-fix.1): CH loads THIS allocation's own kernel copy in the
            // run dir (chown'd to the confined uid), never the operator's
            // master — whose mode/bytes the platform never touches. CH
            // auto-derives the read-only kernel Landlock grant against it.
            .arg("--kernel")
            .arg(config.run_dir.kernel_copy())
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
            .arg("--landlock");
        if let Some(network) = config.network.as_ref() {
            cmd.arg("--net").arg(cloud_hypervisor_network_arg(network));
        }
        for rule in config.landlock_rules() {
            cmd.arg("--landlock-rules").arg(rule.to_rule_arg());
        }
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).kill_on_drop(false);
        cmd
    }
}

/// The single audited launch hook (ADR-0129, ADR-0143): registers one
/// `pre_exec` closure that, in the forked child, (1) marks every descriptor
/// at or above `first_closed` close-on-exec, (2) sets `no_new_privs`, and
/// (3) loads `filter`, returning the first step's `io::Error` on failure.
/// The crate's only production `#[allow(unsafe_code)]`.
#[allow(unsafe_code)]
#[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 05-02")]
#[allow(clippy::needless_pass_by_value, reason = "RED scaffold — DELIVER step 05-02")]
#[expect(clippy::todo, reason = "RED scaffold — DELIVER step 05-02")]
fn register_launch_child_hook(
    cmd: &mut tokio::process::Command,
    first_closed: std::os::fd::RawFd,
    filter: launch_seccomp::VmmLaunchSeccompFilter,
) {
    let _ = (cmd, first_closed, filter);
    todo!("RED scaffold: D-295-R22 register_launch_child_hook — DELIVER step 05-02")
}

fn network_launch_prefix(
    _network: Option<&VmNetworkAttachment>,
    wrapper: &[String],
) -> (String, Vec<String>) {
    (wrapper[0].clone(), wrapper[1..].to_vec())
}

fn cloud_hypervisor_network_arg(attachment: &VmNetworkAttachment) -> String {
    format!(
        "tap={},mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x},offload_tso=off,offload_ufo=off,offload_csum=off",
        attachment.tap,
        attachment.mac[0],
        attachment.mac[1],
        attachment.mac[2],
        attachment.mac[3],
        attachment.mac[4],
        attachment.mac[5],
    )
}

fn classify_launch_spawn_error(
    launched_executable: &OsStr,
    wrapper: &[String],
    source: &io::Error,
) -> VmmError {
    if source.kind() == io::ErrorKind::NotFound && launched_executable == OsStr::new(&wrapper[0]) {
        VmmError::ConfinementUnavailable {
            control: ConfinementControl::UidDrop,
            detail: format!("confinement wrapper {} not found: {source}", wrapper[0]),
        }
    } else {
        VmmError::create(format!(
            "spawning VMM launch executable {} failed: {source}",
            launched_executable.to_string_lossy()
        ))
    }
}

#[async_trait]
impl Vmm for CloudHypervisorVmm {
    fn kind(&self) -> &'static str {
        "cloud-hypervisor"
    }

    async fn probe(&self) -> std::result::Result<(), VmmProbeError> {
        self.probe_substrate.check_reflink(self.image_dir.clone()).await?;

        self.probe_substrate.check_cloud_hypervisor(self.binary.clone()).await?;

        probe_launch_toolchain(self.probe_substrate.as_ref()).await?;

        self.probe_substrate.check_kvm().await?;

        self.probe_substrate.check_run_dir(self.run_dir_root.clone()).await?;

        Ok(())
    }

    // `create` is a cohesive VM-create sequence (clone -> kernel-copy -> confine
    // -> spawn); prep is already extracted into ficlone_rootfs /
    // prepare_confined_paths / hypervisor_present. The ordering across these
    // steps is confinement-critical and review-validated (ADR-0082 4th
    // amendment); splitting further to satisfy the line count would obscure that
    // ordering. `#[allow]` (not `#[expect]`) because the `#[async_trait]`
    // expansion makes `#[expect]` self-fulfilment unreliable here.
    #[allow(clippy::too_many_lines)]
    async fn create(&self, config: &VmConfig) -> Result<VmProcess> {
        let master = config.rootfs.master().to_path_buf();
        let clone_dest = config.rootfs.clone_dest().to_path_buf();
        // A configured master that has disappeared is the ONE absence
        // class here. Every other staging failure stays `Io` / `Create`
        // and reaches the driver's explicit unknown fallback (§D1.1), so
        // a permission error is never relabelled as "not found".
        if let Err(source) = tokio::fs::metadata(&master).await
            && source.kind() == io::ErrorKind::NotFound
        {
            return Err(VmmError::RootfsNotFound { path: master, source });
        }
        let clone_master = master.clone();
        let clone_target = clone_dest.clone();
        tokio::task::spawn_blocking(move || ficlone_rootfs(&clone_master, &clone_target))
            .await
            .map_err(|join_err| VmmError::create(format!("FICLONE task panicked: {join_err}")))??;

        // §(c) consequence 1: the hypervisor is spawned THROUGH the
        // prlimit/setpriv confinement wrapper, so a genuinely-absent
        // `cloud-hypervisor` no longer surfaces as a spawn-time `NotFound`
        // (the wrapper spawns, then `setpriv` fails to exec CH deep in the
        // chain). Detect CH's own absence here — after the clone (so the
        // "clone made then removed on failure" contract is preserved) — so
        // `HypervisorAbsent` keeps naming CH's absence, never the wrapper's.
        if !self.hypervisor_present() {
            let _ = tokio::fs::remove_file(config.rootfs.clone_dest()).await;
            return Err(VmmError::HypervisorAbsent {
                searched: self.searched_binary_paths(),
                source: io::Error::from(io::ErrorKind::NotFound),
            });
        }

        // §(c-fix): CH runs uid-dropped, so before spawn the platform readies
        // this allocation's artifacts for the confined identity: chown the
        // platform-staged rootfs clone, COPY the operator kernel into the run
        // dir, and chown the run dir + its entries. The operator's own kernel
        // and image directories are NEVER mode-widened (B1 fix). Failure here
        // is a confinement-APPLICATION failure — no unconfined fallback is
        // permitted.
        let identity = config.confinement.identity();
        let uid = identity.uid;
        let gid = identity.gid.as_u32();
        let prep_clone = clone_dest.clone();
        let prep_kernel_master = config.kernel.path().to_path_buf();
        let prep_kernel_copy = config.run_dir.kernel_copy();
        let prep_run_dir = config.run_dir.path().to_path_buf();
        let prep = tokio::task::spawn_blocking(move || {
            prepare_confined_paths(
                &prep_clone,
                &prep_kernel_master,
                &prep_kernel_copy,
                &prep_run_dir,
                uid,
                gid,
            )
        })
        .await
        .map_err(|join_err| VmmError::ConfinementUnavailable {
            control: ConfinementControl::UidDrop,
            detail: format!("confine-paths task panicked: {join_err}"),
        })?;
        if let Err(source) = prep {
            let _ = tokio::fs::remove_file(config.rootfs.clone_dest()).await;
            // §(d-fix) M1: applying the confined identity to the per-alloc
            // artifacts (chown/copy of the clone or the kernel copy, run-dir
            // chown) failed — a confinement-APPLICATION failure, mapped to
            // ConfinementUnavailable { UidDrop }, never flattened to
            // Unclassified (`.claude/rules/development.md` § "Errors").
            return Err(VmmError::ConfinementUnavailable {
                control: ConfinementControl::UidDrop,
                detail: format!("prepare confined paths for {}: {source}", clone_dest.display()),
            });
        }

        // §(c): spawn Cloud Hypervisor through the existing confinement
        // wrapper. The persistent TAP is already attached to the shared host
        // bridge; no namespace-exec launcher is required.
        let wrapper = config.confinement.launch_wrapper(config.rlimit_fsize());
        let mut cmd = self.build_confined_command(config, &wrapper);
        let launched_executable: OsString = cmd.as_std().get_program().to_owned();

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
                // Attribute a spawn failure to the process actually passed
                // to `execve`: `ip` for mesh, `prlimit` otherwise. CH's own
                // absence is still owned by the pre-check above.
                return Err(classify_launch_spawn_error(&launched_executable, &wrapper, &source));
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

        // ONE bounded capture per process (§D1.1). The reaper task holds
        // the sole writer; `VmProcess` carries only the reader, so the
        // live deadline snapshot and the final `VmmExit.stderr_tail` are
        // reads of the SAME bytes and cannot disagree.
        let (diagnostics, writer) = VmmDiagnostics::new();
        let exit_diagnostics = diagnostics.clone();

        let live = Arc::clone(&self.live);
        tokio::spawn(async move {
            let reader_handle = stderr_pipe.map(|pipe| spawn_stderr_capture(pipe, writer));

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

            // The final tail is read back off the capture AFTER the last
            // append, never separately assembled.
            let vmm_exit = classify_exit(status, exit_diagnostics.console_tail());
            tracing::info!(
                name: "vmm.process.reaped",
                pid,
                exit_code = ?vmm_exit.exit_code,
                signal = ?vmm_exit.signal,
                "VMM process reaped"
            );
            let _ = exit_tx.send(vmm_exit.clone());
            let _ = state.outcome.send(Some(vmm_exit));
            live.lock().remove(&pid);
        });

        Ok(VmProcess {
            control: VmControl { pid, api_socket: config.run_dir.api_socket() },
            exit: VmExitWatch::new(exit_rx),
            diagnostics,
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

/// Clone `master` to `clone_dest` (in the platform-owned staging root) via
/// the `FICLONE` ioctl — never `cp`. §D6 edge cases: an existing `clone_dest`
/// (a crashed prior launch) is REPLACED, never adopted; on ioctl failure the
/// (possibly empty) destination is removed, never left behind. FICLONE is
/// intra-filesystem, so a `master` on a DIFFERENT filesystem from the staging
/// root returns `EXDEV`; that is the create-time confinement precondition
/// (ADR-0082 2026-08-18 fourth amendment (c-fix.2)) and FAILS CLOSED as
/// `ConfinementUnavailable { UidDrop }` — never a silent operator-dir
/// widening, never a C-1-defeating full-copy fallback.
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
        if err == rustix::io::Errno::XDEV {
            return Err(VmmError::ConfinementUnavailable {
                control: ConfinementControl::UidDrop,
                detail: format!(
                    "rootfs master {} is not on the VM data filesystem (FICLONE EXDEV): {err}",
                    master.display()
                ),
            });
        }
        return Err(VmmError::Io(err.into()));
    }
    Ok(())
}

/// Ready the platform-staged rootfs clone, this allocation's kernel COPY, and
/// the run directory for a uid-dropped hypervisor (ADR-0082 2026-08-18 fourth
/// amendment, B1 fix). Sync — runs on the blocking pool. The governing
/// invariant: the confined identity's DAC access path to every artifact
/// contains ONLY platform-owned directories, so **no operator artifact's mode
/// or bytes is ever changed** — there is nothing to revert and nothing to leak:
///
/// 1. **Rootfs clone** — the platform created it in the platform-owned staging
///    root (`clone_staging_dir`); `chown` it to `uid:gid` so CH opens the
///    `--disk` clone `O_RDWR`. The staging root's set-once `0710` traverse
///    grant (applied at node setup, never here) is what lets the dropped uid
///    reach it — no per-alloc directory widening.
/// 2. **Kernel** — COPY the operator's read-only master into this allocation's
///    run dir (`kernel_copy`) and `chown` the COPY (below, with the run-dir
///    entries). The operator's master is only ever OPENED READ-ONLY by root
///    here; its mode and bytes are untouched (the prior impl's `o+r` on the
///    master, and `o+x` on its directory, are DELETED). CH loads the copy.
/// 3. **Run directory + everything now inside it** — the beacon socket the
///    driver bound before `create`, the guest's `console.log`, and the kernel
///    copy just written — `chown` to `uid:gid`, so CH can bind its own
///    vsock/api sockets, write the console log, connect to the beacon, and
///    read its kernel copy under the dropped uid.
fn prepare_confined_paths(
    clone_dest: &Path,
    kernel_master: &Path,
    kernel_copy: &Path,
    run_dir: &Path,
    uid: u32,
    gid: u32,
) -> io::Result<()> {
    std::os::unix::fs::chown(clone_dest, Some(uid), Some(gid))?;

    // Copy the operator's kernel into this allocation's run dir. `std::fs::copy`
    // only READS the master (mode/bytes untouched) and writes a fresh file the
    // run-dir chown loop below hands to the confined identity.
    std::fs::copy(kernel_master, kernel_copy)?;

    std::os::unix::fs::chown(run_dir, Some(uid), Some(gid))?;
    for entry in std::fs::read_dir(run_dir)? {
        let entry = entry?;
        std::os::unix::fs::chown(entry.path(), Some(uid), Some(gid))?;
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

/// Spawn the SOLE capture task for this process: reads `pipe` in raw byte
/// chunks and appends them to the one bounded capture behind `writer`
/// (§D1.1). Byte-oriented rather than line-oriented because a guest that
/// dies mid-line still printed the bytes that explain WHY — a line reader
/// would silently discard the unterminated final line, which is exactly
/// the output a boot-deadline snapshot most needs.
///
/// Same non-blocking rationale as before: a daemonised grandchild holding
/// the pipe open must never make `terminate` / the reaper hang on EOF.
fn spawn_stderr_capture(
    pipe: ChildStderr,
    writer: VmmDiagnosticsWriter,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut reader = BufReader::new(pipe);
        let mut chunk = [0u8; 4096];
        loop {
            match reader.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(read) => writer.append(&chunk[..read]),
            }
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

/// §(c) consequence 1 — every VMM launch tool (`prlimit`, `setpriv`, and the
/// mesh namespace launcher `ip`) resolves on `PATH`. The hypervisor is spawned
/// THROUGH them (the resolution needs no `unsafe` block under
/// `overdrive-host`'s `#![deny(unsafe_code)]`), so `argv[0]` is `ip` for a
/// mesh VM and `prlimit` otherwise. An unavailable launch tool must refuse the node at
/// boot (wire → probe → use), never surface later as a misclassified
/// `HypervisorAbsent`. A
/// successful spawn of `<tool> --version` (any exit status) proves the tool is
/// executable; any spawn error retains its original I/O kind in the typed
/// launch-tool diagnostic.
async fn probe_launch_toolchain(
    substrate: &dyn VmmProbeSubstrate,
) -> std::result::Result<(), VmmProbeError> {
    for tool in REQUIRED_LAUNCH_TOOLS {
        let result = substrate.execute_launch_tool(tool).await;
        launch_tool_probe_result(tool, result)?;
    }
    Ok(())
}

fn launch_tool_probe_result(
    tool: &str,
    result: io::Result<()>,
) -> std::result::Result<(), VmmProbeError> {
    result.map_err(|source| VmmProbeError::launch_tool_unavailable(tool, source))
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use std::num::NonZeroU8;

    use overdrive_core::cgroup::CgroupPath;
    use overdrive_core::id::AllocationId;
    use overdrive_core::vm::config::{
        Gid, HostArch, KERNEL_MAGIC_WINDOW, KernelCmdline, KernelImage, MemoryPlan, RootfsPlan,
        VmConfinement, VmRunDir, VmmIdentity,
    };

    use super::*;

    /// The probe stages in their accepted order (FD § "[REF] Driven port — VMM launch seccomp filter (D-295-R22) — ACCEPTED 2026-09-24" (the startup probe order)): R2 removes
    /// `ip`, and D-295-R22 inserts `launch-seccomp` after `setpriv`.
    const ACCEPTED_PROBE_ORDER: [&str; 7] =
        ["reflink", "cloud-hypervisor", "prlimit", "setpriv", "launch-seccomp", "kvm", "run-dir"];

    struct RecordingProbeSubstrate {
        visited: Arc<Mutex<Vec<&'static str>>>,
        /// A launch tool whose execution fails, and the failure's kind.
        launch_tool_failure: Option<(&'static str, io::ErrorKind)>,
        /// The launch-seccomp stage's scripted failure; `None` passes.
        launch_seccomp_failure: Mutex<Option<VmmProbeError>>,
    }

    impl RecordingProbeSubstrate {
        fn new(
            visited: &Arc<Mutex<Vec<&'static str>>>,
            launch_tool_failure: Option<(&'static str, io::ErrorKind)>,
            launch_seccomp_failure: Option<VmmProbeError>,
        ) -> Self {
            Self {
                visited: Arc::clone(visited),
                launch_tool_failure,
                launch_seccomp_failure: Mutex::new(launch_seccomp_failure),
            }
        }
    }

    #[async_trait]
    impl VmmProbeSubstrate for RecordingProbeSubstrate {
        async fn check_reflink(
            &self,
            _image_dir: PathBuf,
        ) -> std::result::Result<(), VmmProbeError> {
            self.visited.lock().push("reflink");
            Ok(())
        }

        async fn check_cloud_hypervisor(
            &self,
            _binary: PathBuf,
        ) -> std::result::Result<(), VmmProbeError> {
            self.visited.lock().push("cloud-hypervisor");
            Ok(())
        }

        async fn execute_launch_tool(&self, tool: &'static str) -> io::Result<()> {
            self.visited.lock().push(tool);
            if let Some((failing, kind)) = self.launch_tool_failure
                && failing == tool
            {
                return Err(io::Error::new(kind, format!("injected {kind:?}")));
            }
            Ok(())
        }

        async fn check_launch_seccomp(&self) -> std::result::Result<(), VmmProbeError> {
            self.visited.lock().push("launch-seccomp");
            let scripted = self.launch_seccomp_failure.lock().take();
            scripted.map_or(Ok(()), Err)
        }

        async fn check_kvm(&self) -> std::result::Result<(), VmmProbeError> {
            self.visited.lock().push("kvm");
            Ok(())
        }

        async fn check_run_dir(
            &self,
            _run_dir_root: PathBuf,
        ) -> std::result::Result<(), VmmProbeError> {
            self.visited.lock().push("run-dir");
            Ok(())
        }
    }

    fn vmm_over(substrate: RecordingProbeSubstrate) -> CloudHypervisorVmm {
        CloudHypervisorVmm { probe_substrate: Arc::new(substrate), ..CloudHypervisorVmm::new() }
    }

    fn sample_config(network: Option<VmNetworkAttachment>) -> VmConfig {
        let alloc = AllocationId::new("mesh-argv").unwrap();
        let mut header = vec![0; KERNEL_MAGIC_WINDOW];
        header[..4].copy_from_slice(b"\x7fELF");
        VmConfig {
            alloc: alloc.clone(),
            kernel: KernelImage::validate(
                PathBuf::from("/srv/vm/kernel"),
                HostArch::X86_64,
                &header,
            )
            .unwrap(),
            rootfs: RootfsPlan::for_alloc(
                PathBuf::from("/srv/vm/rootfs.img"),
                1024,
                &alloc,
                Path::new("/srv/vm/clone-staging"),
                Path::new("/srv/vm/clone-index"),
            ),
            cmdline: KernelCmdline::platform_default(HostArch::X86_64),
            memory: MemoryPlan::derive(128 * 1024 * 1024),
            vcpus: NonZeroU8::new(1).unwrap(),
            run_dir: VmRunDir::for_alloc(Path::new("/run/overdrive/vm"), &alloc),
            confinement: VmConfinement::confined(
                VmmIdentity { uid: 991, gid: Gid::new(994), supplementary: vec![] },
                1024,
            ),
            network,
            cgroup_scope: CgroupPath::for_alloc(&alloc),
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-44 — A target with no launch filter starts no microVM (the
    /// probe runs reflink, cloud-hypervisor, prlimit, setpriv, launch-seccomp,
    /// kvm, run-dir in that order, and never executes `ip`).
    /// CONTRACT_SHAPE: pure-function.
    #[allow(
        clippy::doc_markdown,
        reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
    )]
    #[tokio::test]
    #[ignore = "pending DELIVER step 05-02 (S-ND295-44)"]
    async fn vmm_probe_preserves_stage_order_and_rejects_each_injected_ip_execution_failure() {
        // An injected `ip` execution failure can no longer reject the probe:
        // the launcher needs no `ip`, so the probe never executes it.
        for kind in [io::ErrorKind::NotFound, io::ErrorKind::PermissionDenied] {
            let visited = Arc::new(Mutex::new(Vec::new()));
            let vmm = vmm_over(RecordingProbeSubstrate::new(&visited, Some(("ip", kind)), None));

            let outcome = Vmm::probe(&vmm).await;

            assert!(
                outcome.is_ok(),
                "an armed {kind:?} failure of the removed ip tool must never be reached: {outcome:?}",
            );
            assert_eq!(
                *visited.lock(),
                ACCEPTED_PROBE_ORDER,
                "the probe must run every accepted stage in order and never execute ip",
            );
        }

        // Each remaining launch tool's execution failure still rejects the
        // probe at that tool, with its honest diagnostic, before the
        // launch-seccomp, KVM, and run-root stages.
        for (tool, stages_run) in [("prlimit", 3_usize), ("setpriv", 4)] {
            for (kind, diagnosis) in [
                (io::ErrorKind::NotFound, "not found on PATH: injected NotFound"),
                (io::ErrorKind::PermissionDenied, "could not execute: injected PermissionDenied"),
            ] {
                let visited = Arc::new(Mutex::new(Vec::new()));
                let vmm =
                    vmm_over(RecordingProbeSubstrate::new(&visited, Some((tool, kind)), None));

                let error = Vmm::probe(&vmm)
                    .await
                    .expect_err("an injected launch-tool failure must reject the probe");

                assert_eq!(
                    *visited.lock(),
                    ACCEPTED_PROBE_ORDER[..stages_run],
                    "a {tool} failure must stop the probe at {tool}",
                );
                match &error {
                    VmmProbeError::LaunchToolUnavailable { tool: failed, source } => {
                        assert_eq!(failed, tool);
                        assert_eq!(source.kind(), kind);
                    }
                    other => panic!("{tool} {kind:?} received the wrong typed diagnostic: {other}"),
                }
                assert_eq!(
                    error.to_string(),
                    format!("VMM launch tool {tool} {diagnosis}"),
                    "{tool} {kind:?} must retain an honest diagnostic",
                );
            }
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-44 — A target with no launch filter starts no microVM (each
    /// launch-filter probe cause maps to its own typed probe error).
    /// CONTRACT_SHAPE: pure-function.
    #[allow(
        clippy::doc_markdown,
        reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
    )]
    #[tokio::test]
    #[ignore = "pending DELIVER step 05-02 (S-ND295-44)"]
    async fn each_launch_filter_probe_cause_maps_to_its_typed_error() {
        let arch = std::env::consts::ARCH;

        // On every target but x86_64 the real stage has no program to install
        // and names the architecture before any spawn (user ruling 10).
        #[cfg(not(target_arch = "x86_64"))]
        {
            let refusal = RealVmmProbeSubstrate.check_launch_seccomp().await;
            match refusal {
                Err(VmmProbeError::LaunchSeccompUnsupportedArch { target_arch }) => {
                    assert_eq!(target_arch, arch, "the refusal must name the running architecture");
                }
                other => {
                    panic!("the {arch} launch-seccomp stage must refuse as unsupported: {other:?}")
                }
            }
        }

        let sigsys = u8::try_from(libc::SIGSYS).expect("SIGSYS fits the VmmExit signal shape");
        let causes = [
            (
                VmmProbeError::launch_seccomp_unsupported_arch(arch),
                format!("no VMM launch seccomp program for target architecture {arch}"),
            ),
            (
                VmmProbeError::launch_seccomp_install(io::Error::from_raw_os_error(libc::EINVAL)),
                format!(
                    "VMM launch seccomp filter could not be installed: {}",
                    io::Error::from_raw_os_error(libc::EINVAL)
                ),
            ),
            (
                VmmProbeError::launch_seccomp_install(io::Error::from(io::ErrorKind::NotFound)),
                format!(
                    "VMM launch seccomp filter could not be installed: {}",
                    io::Error::from(io::ErrorKind::NotFound)
                ),
            ),
            (
                VmmProbeError::launch_seccomp_probe_exit(Some(1), None),
                "launch tool under the VMM launch seccomp filter ended with exit code Some(1), \
                 signal None"
                    .to_owned(),
            ),
            (
                VmmProbeError::launch_seccomp_probe_exit(None, Some(sigsys)),
                format!(
                    "launch tool under the VMM launch seccomp filter ended with exit code None, \
                     signal Some({sigsys})"
                ),
            ),
        ];

        for (cause, display) in causes {
            let expected = format!("{cause:?}");
            let visited = Arc::new(Mutex::new(Vec::new()));
            let vmm = vmm_over(RecordingProbeSubstrate::new(&visited, None, Some(cause)));

            let error = Vmm::probe(&vmm)
                .await
                .expect_err("a failing launch-seccomp stage must reject the probe");

            let stages = visited.lock().clone();
            assert_eq!(
                stages.last(),
                Some(&"launch-seccomp"),
                "the probe must stop at the launch-seccomp stage for {expected}: {stages:?}",
            );
            let setpriv = stages.iter().position(|stage| *stage == "setpriv");
            let seccomp = stages.iter().position(|stage| *stage == "launch-seccomp");
            assert!(
                setpriv.is_some() && setpriv < seccomp,
                "the launch-seccomp stage must follow setpriv: {stages:?}",
            );
            assert!(
                !stages.contains(&"kvm") && !stages.contains(&"run-dir"),
                "no stage after launch-seccomp may run once it fails: {stages:?}",
            );
            assert_eq!(
                format!("{error:?}"),
                expected,
                "the probe must return the stage's typed launch-seccomp error unchanged",
            );
            assert_eq!(error.to_string(), display, "the pinned diagnostic must render");
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-40 — The VMM adapter launches from the queue and reports queue
    /// failures in their own terms (launch shape).
    /// CONTRACT_SHAPE: pure-function.
    #[allow(
        clippy::doc_markdown,
        reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
    )]
    #[test]
    #[ignore = "pending DELIVER step 05-03 (S-ND295-40)"]
    fn mesh_and_non_mesh_launches_preserve_shape_and_attribute_the_actual_launcher() {
        let attachment = VmNetworkAttachment {
            tap: "ovd-tap-002a".to_owned(),
            mac: [0x02, 0x00, 0x00, 0x00, 0x00, 0x2a],
        };
        let wrapper = vec![
            "prlimit".to_owned(),
            "--fsize=1073741824".to_owned(),
            "setpriv".to_owned(),
            "--reuid=991".to_owned(),
        ];
        let queue_net_arg =
            "fd=[3],mac=02:00:00:00:00:2a,offload_tso=off,offload_ufo=off,offload_csum=off";

        assert_eq!(VMM_TAP_QUEUE_FD, 3, "the queue is handed to Cloud Hypervisor at descriptor 3");
        assert_eq!(
            cloud_hypervisor_network_arg(&attachment),
            queue_net_arg,
            "the network argument must name only descriptor 3 and the guest MAC",
        );
        assert_eq!(
            REQUIRED_LAUNCH_TOOLS.as_slice(),
            ["prlimit", "setpriv"].as_slice(),
            "the direct launcher needs no ip",
        );

        let vmm = CloudHypervisorVmm::new().with_binary(PathBuf::from("/usr/bin/cloud-hypervisor"));
        let command = vmm.build_confined_command(&sample_config(Some(attachment)), &wrapper);
        let command = command.as_std();
        let program = command.get_program().to_string_lossy();
        let args: Vec<_> =
            command.get_args().map(|arg| arg.to_string_lossy().into_owned()).collect();

        assert_eq!(program, "prlimit");
        assert_eq!(
            &args[..4],
            ["--fsize=1073741824", "setpriv", "--reuid=991", "/usr/bin/cloud-hypervisor"],
            "the direct launch must preserve the confinement argv",
        );
        let net_flag = args.iter().position(|arg| arg == "--net").unwrap();
        assert_eq!(
            args[net_flag + 1],
            queue_net_arg,
            "Cloud Hypervisor must import its queue from descriptor 3 with the slot-derived MAC",
        );
        assert!(
            !args.iter().any(|arg| arg.contains("tap=")),
            "Cloud Hypervisor must never open the TAP by name: {args:?}",
        );
        assert!(
            args.windows(2).any(|pair| {
                pair == ["--landlock-rules", "path=/sys/class/net/ovd-tap-002a,access=r"]
            }),
            "mesh VMM confinement must grant read-only access to the exact TAP sysfs directory",
        );

        let command = vmm.build_confined_command(&sample_config(None), &wrapper);
        let command = command.as_std();
        let program = command.get_program().to_string_lossy();
        let args: Vec<_> =
            command.get_args().map(|arg| arg.to_string_lossy().into_owned()).collect();
        assert_eq!(program, "prlimit");
        assert_eq!(
            &args[..4],
            ["--fsize=1073741824", "setpriv", "--reuid=991", "/usr/bin/cloud-hypervisor"],
            "non-mesh launch prefix must remain byte-for-byte unchanged"
        );
        assert!(!args.iter().any(|arg| arg == "--net"));
        assert!(
            !args.iter().any(|arg| arg.contains("/sys/class/net/")),
            "a non-networked VM must not gain a sysfs network grant",
        );

        let non_mesh = classify_launch_spawn_error(
            OsStr::new("prlimit"),
            &wrapper,
            &io::Error::from(io::ErrorKind::NotFound),
        );
        assert!(
            matches!(
                non_mesh,
                VmmError::ConfinementUnavailable { control: ConfinementControl::UidDrop, .. }
            ),
            "missing non-mesh wrapper must preserve the established confinement classification"
        );
    }
}

/// E21's real-kernel cases (D-295-R22, ADR-0143) and S-ND295-40's queue
/// release: a scratch persistent TAP named outside `ovd-tp-` (never bridged,
/// deleted on drop) and re-execs of this test binary (FD § "[REF] Driven port — VMM launch seccomp filter (D-295-R22) — ACCEPTED 2026-09-24" (the testability boundary)).
#[cfg(all(test, feature = "integration-tests"))]
#[allow(unsafe_code)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
#[allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the re-exec child reports on stdout; fixture teardown reports failures on stderr"
)]
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
mod launch_seccomp_kernel {

    use std::num::NonZeroU8;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    use overdrive_core::cgroup::CgroupPath;
    use overdrive_core::id::AllocationId;
    use overdrive_core::vm::config::{
        Gid, HostArch, KERNEL_MAGIC_WINDOW, KernelCmdline, KernelImage, MemoryPlan, RootfsPlan,
        VmConfinement, VmRunDir, VmmIdentity,
    };
    use overdrive_netlink::{Client, block_on_host_netlink, create_persistent_tap};

    use super::{CloudHypervisorVmm, VmConfig, VmNetworkAttachment, io};

    static SCRATCH_SEQUENCE: AtomicU32 = AtomicU32::new(0);

    /// A persistent TAP created by root with owner uid 0, as the shared
    /// guest-network owner creates it (D-295-R4): down, unbridged, named
    /// outside the managed `ovd-tp-` prefix, and deleted on drop.
    struct ScratchTap {
        name: String,
    }

    impl ScratchTap {
        fn create() -> Self {
            let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = format!("nd295k{:x}{sequence:x}", std::process::id());
            create_persistent_tap(&name, 0)
                .unwrap_or_else(|error| panic!("create scratch TAP {name}: {error}"));
            Self { name }
        }
    }

    impl Drop for ScratchTap {
        fn drop(&mut self) {
            let name = self.name.clone();
            let deleted =
                block_on_host_netlink(|| async move { Client::new()?.del_link(&name).await });
            if let Err(error) = deleted {
                eprintln!("scratch TAP {} cleanup failed: {error}", self.name);
            }
        }
    }

    /// An `ifreq` naming `name`, every other byte zero.
    fn named_ifreq(name: &str) -> libc::ifreq {
        assert!(name.len() < libc::IFNAMSIZ, "interface name {name} must fit IFNAMSIZ");
        // SAFETY: `ifreq` is plain old data for which all-zero bytes are valid.
        let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
        for (slot, byte) in request.ifr_name.iter_mut().zip(name.bytes()) {
            *slot = libc::c_char::from_ne_bytes([byte]);
        }
        request
    }

    /// The flags of a vnet-header single-queue TAP queue (FD § "[REF] Driven port — VMM TAP queue attachment (D-295-R1, R2, R3, R4) — ACCEPTED 2026-09-24" (the `attach_tap_queue` contract: effect)).
    fn tap_queue_flags() -> libc::c_short {
        libc::c_short::try_from(libc::IFF_TAP | libc::IFF_NO_PI | libc::IFF_VNET_HDR)
            .expect("TAP queue flags fit ifr_flags")
    }

    /// Attach one vnet-header queue to the scratch TAP with a raw `TUNSETIFF`.
    /// Fixture setup only: the production `attach_tap_queue` is DELIVER 05-03
    /// and carries its own contract (S-ND295-38), so the 05-02 cases attach
    /// their queue without it.
    fn attach_scratch_queue(tap: &str) -> OwnedFd {
        let tun = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/net/tun")
            .expect("open /dev/net/tun (close-on-exec)");
        let mut request = named_ifreq(tap);
        request.ifr_ifru.ifru_flags = tap_queue_flags();
        // SAFETY: `tun` is an open `/dev/net/tun` descriptor and `request` is a
        // live, initialised `ifreq` for the whole call.
        let rc = unsafe { libc::ioctl(tun.as_raw_fd(), libc::TUNSETIFF, &raw mut request) };
        assert_eq!(rc, 0, "attach a queue to scratch TAP {tap}: {}", io::Error::last_os_error());
        OwnedFd::from(tun)
    }

    /// `(carrier_up_count, carrier_down_count)`: a TAP's carrier rises when a
    /// queue attaches and falls when its last queue closes, so the pair counts
    /// attaches and releases even while the TAP is administratively down.
    fn carrier_counts(tap: &str) -> (u64, u64) {
        let read = |counter: &str| -> u64 {
            let path = format!("/sys/class/net/{tap}/{counter}");
            let raw = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {path}: {error}"));
            raw.trim().parse().unwrap_or_else(|error| panic!("parse {path} ({raw:?}): {error}"))
        };
        (read("carrier_up_count"), read("carrier_down_count"))
    }

    /// Every descriptor of this process whose `fdinfo` names `tap`'s queue.
    fn descriptors_holding_queue_of(tap: &str) -> Vec<i32> {
        let listed: Vec<i32> = std::fs::read_dir("/proc/self/fd")
            .expect("list /proc/self/fd")
            .map(|entry| {
                let name = entry.expect("read a /proc/self/fd entry").file_name();
                name.to_string_lossy().parse().expect("descriptor entries are numbers")
            })
            .collect();
        listed
            .into_iter()
            .filter(|fd| match std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}")) {
                Ok(info) => info.lines().any(|line| {
                    line.strip_prefix("iff:").is_some_and(|holder| holder.trim() == tap)
                }),
                // The directory handle used for the listing is closed by now.
                Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                Err(error) => panic!("read /proc/self/fdinfo/{fd}: {error}"),
            })
            .collect()
    }

    /// The owner uid of `path`, or `None` while it does not exist.
    fn owner_uid(path: &Path) -> Option<u32> {
        match std::fs::metadata(path) {
            Ok(metadata) => Some(metadata.uid()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => panic!("stat {}: {error}", path.display()),
        }
    }

    /// A complete VM launch configuration over real files under `root`: a
    /// rootfs master, clone staging and index directories, a kernel image, a
    /// run directory, and an executable stand-in hypervisor binary that the
    /// cases never reach.
    struct LaunchFixture {
        _root: tempfile::TempDir,
        config: VmConfig,
        hypervisor: PathBuf,
    }

    impl LaunchFixture {
        fn new(root: tempfile::TempDir, network: VmNetworkAttachment) -> Self {
            use std::os::unix::fs::PermissionsExt;

            let base = root.path();
            let alloc = AllocationId::new("nd295-launch").expect("allocation id");
            let master = base.join("rootfs.img");
            let master_bytes = 1_u64 << 20;
            std::fs::write(&master, vec![0x5a_u8; 1 << 20]).expect("write the rootfs master");
            let staging = base.join("clone-staging");
            let index = base.join("clone-index");
            std::fs::create_dir(&staging).expect("create the clone staging directory");
            std::fs::create_dir(&index).expect("create the clone index directory");
            let mut header = vec![0; KERNEL_MAGIC_WINDOW];
            header[..4].copy_from_slice(b"\x7fELF");
            let kernel = base.join("vmlinux");
            std::fs::write(&kernel, &header).expect("write the kernel image");
            let run_dir = VmRunDir::for_alloc(&base.join("run"), &alloc);
            std::fs::create_dir_all(run_dir.path()).expect("create the run directory");
            let hypervisor = base.join("cloud-hypervisor");
            std::fs::write(&hypervisor, b"#!/bin/sh\nexit 0\n").expect("write the stand-in binary");
            std::fs::set_permissions(&hypervisor, std::fs::Permissions::from_mode(0o755))
                .expect("make the stand-in binary executable");

            let config = VmConfig {
                alloc: alloc.clone(),
                kernel: KernelImage::validate(kernel, HostArch::X86_64, &header)
                    .expect("kernel image header"),
                rootfs: RootfsPlan::for_alloc(master, master_bytes, &alloc, &staging, &index),
                cmdline: KernelCmdline::platform_default(HostArch::X86_64),
                memory: MemoryPlan::derive(128 * 1024 * 1024),
                vcpus: NonZeroU8::new(1).expect("one vCPU"),
                run_dir,
                confinement: VmConfinement::confined(
                    VmmIdentity { uid: 991, gid: Gid::new(994), supplementary: vec![] },
                    1024,
                ),
                network: Some(network),
                cgroup_scope: CgroupPath::for_alloc(&alloc),
            };
            Self { _root: root, config, hypervisor }
        }

        fn vmm(&self) -> CloudHypervisorVmm {
            CloudHypervisorVmm::new().with_binary(self.hypervisor.clone())
        }
    }

    // ------------------------------------------------------------------
    // x86_64: the production program, installed by the production hook
    // ------------------------------------------------------------------

    #[cfg(target_arch = "x86_64")]
    const CHILD_ROLE_ENV: &str = "OVERDRIVE_LAUNCH_SECCOMP_CHILD";
    #[cfg(target_arch = "x86_64")]
    const CHILD_TAP_ENV: &str = "OVERDRIVE_LAUNCH_SECCOMP_TAP";
    #[cfg(target_arch = "x86_64")]
    const REPORT: &str = "OVERDRIVE_LAUNCH_SECCOMP_REPORT";
    /// The libtest path of [`launch_seccomp_child_role`].
    #[cfg(target_arch = "x86_64")]
    const CHILD_ROLE_TEST: &str = "vmm::launch_seccomp_kernel::launch_seccomp_child_role";

    /// The thirteen TAP-mutating requests (FD § "[REF] Driven port — VMM launch seccomp filter (D-295-R22) — ACCEPTED 2026-09-24" (the thirteen-row deny-list table)), in table order.
    #[cfg(target_arch = "x86_64")]
    const DENIED_REQUESTS: [&str; 13] = [
        "SIOCSIFHWADDR",
        "TUNSETOWNER",
        "TUNSETGROUP",
        "TUNSETPERSIST",
        "TUNSETCARRIER",
        "TUNSETDEBUG",
        "TUNSETLINK",
        "TUNSETTXFILTER",
        "TUNATTACHFILTER",
        "TUNDETACHFILTER",
        "TUNSETSTEERINGEBPF",
        "TUNSETFILTEREBPF",
        "TUNSETQUEUE",
    ];

    /// The six requests Cloud Hypervisor v53's `fd=` path issues and the
    /// read-only request the child uses to restate the header size.
    #[cfg(target_arch = "x86_64")]
    const ALLOWED_REQUESTS: [&str; 7] = [
        "TUNGETIFF",
        "TUNSETIFF",
        "TUNGETVNETHDRSZ",
        "TUNSETVNETHDRSZ",
        "SIOCGIFMTU",
        "SIOCSIFMTU",
        "TUNSETOFFLOAD",
    ];

    #[cfg(target_arch = "x86_64")]
    #[derive(Clone, Copy)]
    enum Launch {
        /// Through the production `register_launch_child_hook` with the
        /// production program.
        Hooked { first_closed: std::os::fd::RawFd },
        /// The contrast: the same child with no hook.
        Unhooked,
    }

    /// One re-exec'd child's exit status and its report lines.
    #[cfg(target_arch = "x86_64")]
    struct ChildRun {
        role: &'static str,
        output: std::process::Output,
    }

    #[cfg(target_arch = "x86_64")]
    impl ChildRun {
        /// The `k=v` fields of every report record of `kind`.
        ///
        /// A record is matched wherever its tag appears on a line, not only
        /// at the line start: under `--nocapture` libtest prints the child
        /// role's `test <name> ... ` prefix without a newline, so the first
        /// record the child prints shares that line.
        fn records(&self, kind: &str) -> Vec<std::collections::BTreeMap<String, String>> {
            String::from_utf8_lossy(&self.output.stdout)
                .lines()
                .filter_map(|line| line.find(REPORT).map(|at| &line[at + REPORT.len()..]))
                .filter_map(|record| {
                    let mut tokens = record.split_whitespace();
                    if tokens.next() != Some(kind) {
                        return None;
                    }
                    let fields = tokens
                        .filter_map(|token| token.split_once('='))
                        .map(|(key, value)| (key.to_owned(), value.to_owned()))
                        .collect();
                    Some(fields)
                })
                .collect()
        }

        fn signal(&self) -> Option<i32> {
            std::os::unix::process::ExitStatusExt::signal(&self.output.status)
        }

        fn assert_completed(&self) {
            assert!(
                self.output.status.success(),
                "child role {} did not complete: {self}",
                self.role
            );
        }
    }

    #[cfg(target_arch = "x86_64")]
    impl std::fmt::Display for ChildRun {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "status {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
                self.output.status,
                String::from_utf8_lossy(&self.output.stdout),
                String::from_utf8_lossy(&self.output.stderr),
            )
        }
    }

    /// Re-exec this test binary as `role`, with `queue` at descriptor 3, and
    /// (for [`Launch::Hooked`]) the production launch hook registered after the
    /// queue mapping, as `create` registers it after the `command-fds` mapping.
    #[cfg(target_arch = "x86_64")]
    fn run_child(
        role: &'static str,
        tap: &ScratchTap,
        queue: &OwnedFd,
        launch: Launch,
    ) -> ChildRun {
        use std::process::Stdio;

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("child launcher runtime");
        let output = runtime.block_on(async {
            let binary = std::env::current_exe().expect("locate this test binary");
            let mut cmd = tokio::process::Command::new(binary);
            cmd.args([CHILD_ROLE_TEST, "--exact", "--include-ignored", "--nocapture"])
                .arg("--test-threads=1")
                .env(CHILD_ROLE_ENV, role)
                .env(CHILD_TAP_ENV, &tap.name)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let queue_fd = queue.as_raw_fd();
            let queue_child_fd = super::VMM_TAP_QUEUE_FD;
            let no_core = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
            // SAFETY: the closure runs in the forked child before exec and
            // issues only `setrlimit`, `fcntl`, and `dup2` raw syscalls on
            // values it captured by copy: no allocation, lock, or formatting.
            // It places the queue at descriptor 3 without close-on-exec,
            // standing in for the `command-fds` mapping, and disables core
            // dumps for the cases that end in `SIGSYS`.
            unsafe {
                cmd.pre_exec(move || {
                    if libc::setrlimit(libc::RLIMIT_CORE, &raw const no_core) == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    let mapped = if queue_fd == queue_child_fd {
                        libc::fcntl(queue_fd, libc::F_SETFD, 0)
                    } else {
                        libc::dup2(queue_fd, queue_child_fd)
                    };
                    if mapped == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            if let Launch::Hooked { first_closed } = launch {
                let filter = super::launch_seccomp::VmmLaunchSeccompFilter::for_target()
                    .expect("the 64-bit x86_64 target has a launch seccomp program");
                super::register_launch_child_hook(&mut cmd, first_closed, filter);
            }
            cmd.output().await.expect("spawn the re-exec'd child")
        });
        ChildRun { role, output }
    }

    /// Inheritable descriptors held across the spawn: a socket and a pipe
    /// opened without their close-on-exec flags, each also duplicated at or
    /// above 64 (`F_DUPFD`, still inheritable) so they sit clear of the
    /// descriptor-3 boundary under test.
    #[cfg(target_arch = "x86_64")]
    struct InheritableDescriptors {
        held: Vec<OwnedFd>,
    }

    #[cfg(target_arch = "x86_64")]
    impl InheritableDescriptors {
        fn open() -> Self {
            use std::os::fd::FromRawFd;

            // SAFETY: socket(2) and pipe(2) without their close-on-exec flags
            // are the deliberate fault stimulus; each descriptor they return
            // is owned below exactly once.
            let socket = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
            assert!(socket >= 0, "open an inheritable socket: {}", io::Error::last_os_error());
            let mut pipe = [0; 2];
            // SAFETY: `pipe` is a live two-element buffer for the call.
            let rc = unsafe { libc::pipe(pipe.as_mut_ptr()) };
            assert_eq!(rc, 0, "open an inheritable pipe: {}", io::Error::last_os_error());
            let mut held = Vec::with_capacity(6);
            for raw in [socket, pipe[0], pipe[1]] {
                // SAFETY: `raw` is open; F_DUPFD returns a new descriptor
                // without FD_CLOEXEC.
                let high = unsafe { libc::fcntl(raw, libc::F_DUPFD, 64) };
                assert!(high >= 64, "duplicate {raw} above 64: {}", io::Error::last_os_error());
                // SAFETY: `raw` and `high` are open descriptors this fixture
                // exclusively owns from here on.
                held.push(unsafe { OwnedFd::from_raw_fd(raw) });
                // SAFETY: as above.
                held.push(unsafe { OwnedFd::from_raw_fd(high) });
            }
            let fixture = Self { held };
            fixture.assert_inheritable();
            fixture
        }

        fn assert_inheritable(&self) {
            for fd in &self.held {
                // SAFETY: F_GETFD only reads the flags of an open descriptor.
                let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
                assert!(
                    flags >= 0 && flags & libc::FD_CLOEXEC == 0,
                    "fixture descriptor {} must stay inheritable (flags {flags})",
                    fd.as_raw_fd(),
                );
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn report(record: std::fmt::Arguments<'_>) {
        println!("{REPORT} {record}");
    }

    #[cfg(target_arch = "x86_64")]
    fn errno_of(rc: libc::c_int) -> i32 {
        if rc == -1 { io::Error::last_os_error().raw_os_error().expect("an OS error") } else { 0 }
    }

    /// The child's open descriptors, excluding the directory handle used to
    /// list them.
    #[cfg(target_arch = "x86_64")]
    fn open_descriptors() -> Vec<i32> {
        let listed: Vec<i32> = std::fs::read_dir("/proc/self/fd")
            .expect("list /proc/self/fd")
            .map(|entry| {
                let name = entry.expect("read a /proc/self/fd entry").file_name();
                name.to_string_lossy().parse().expect("descriptor entries are numbers")
            })
            .collect();
        let mut open: Vec<i32> = listed
            .into_iter()
            // SAFETY: F_GETFD only reads descriptor flags; the closed listing
            // handle answers EBADF.
            .filter(|fd| unsafe { libc::fcntl(*fd, libc::F_GETFD) } != -1)
            .collect();
        open.sort_unstable();
        open
    }

    #[cfg(target_arch = "x86_64")]
    fn joined(fds: &[i32]) -> String {
        fds.iter().map(ToString::to_string).collect::<Vec<_>>().join(",")
    }

    #[cfg(target_arch = "x86_64")]
    fn child_descriptors() {
        let fds = open_descriptors();
        let fd3 = if fds.contains(&3) {
            let info = std::fs::read_to_string("/proc/self/fdinfo/3").expect("read fdinfo 3");
            info.lines()
                .find_map(|line| line.strip_prefix("iff:").map(|tap| tap.trim().to_owned()))
                .unwrap_or_else(|| "not-a-tap-queue".to_owned())
        } else {
            "-".to_owned()
        };
        report(format_args!("descriptors fds={} fd3_iff={fd3}", joined(&fds)));
    }

    /// Issue one denied request on the queue. The arguments are chosen so a
    /// request the filter failed to stop changes nothing on the scratch TAP:
    /// invalid ids and an unset hardware-address family are `EINVAL`, and the
    /// rest restate the TAP's current value.
    #[cfg(target_arch = "x86_64")]
    fn issue_denied(name: &str, fd: std::os::fd::RawFd) -> i32 {
        let mut request = named_ifreq("");
        let mut carrier_on: libc::c_int = 1;
        let mut detach_program: libc::c_int = -1;
        let mut tx_filter = [0_u8; 4];
        let mut empty_program = libc::sock_fprog { len: 0, filter: std::ptr::null_mut() };
        let invalid_id = libc::c_ulong::from(u32::MAX);
        let persist: libc::c_ulong = 1;
        let debug_off: libc::c_ulong = 0;
        let ether = libc::c_ulong::from(libc::ARPHRD_ETHER);
        // SAFETY: each call passes the open queue descriptor and either an
        // integer argument or a pointer to a live, correctly sized stack
        // object that the kernel reads only during the call.
        let rc = unsafe {
            match name {
                "SIOCSIFHWADDR" => libc::ioctl(fd, libc::SIOCSIFHWADDR, &raw mut request),
                "TUNSETOWNER" => libc::ioctl(fd, libc::TUNSETOWNER, invalid_id),
                "TUNSETGROUP" => libc::ioctl(fd, libc::TUNSETGROUP, invalid_id),
                "TUNSETPERSIST" => libc::ioctl(fd, libc::TUNSETPERSIST, persist),
                "TUNSETCARRIER" => libc::ioctl(fd, libc::TUNSETCARRIER, &raw mut carrier_on),
                "TUNSETDEBUG" => libc::ioctl(fd, libc::TUNSETDEBUG, debug_off),
                "TUNSETLINK" => libc::ioctl(fd, libc::TUNSETLINK, ether),
                "TUNSETTXFILTER" => libc::ioctl(fd, libc::TUNSETTXFILTER, tx_filter.as_mut_ptr()),
                "TUNATTACHFILTER" => libc::ioctl(fd, libc::TUNATTACHFILTER, &raw mut empty_program),
                "TUNDETACHFILTER" => libc::ioctl(fd, libc::TUNDETACHFILTER, &raw mut empty_program),
                "TUNSETSTEERINGEBPF" => {
                    libc::ioctl(fd, libc::TUNSETSTEERINGEBPF, &raw mut detach_program)
                }
                "TUNSETFILTEREBPF" => {
                    libc::ioctl(fd, libc::TUNSETFILTEREBPF, &raw mut detach_program)
                }
                "TUNSETQUEUE" => libc::ioctl(fd, libc::TUNSETQUEUE, &raw mut request),
                other => panic!("{other} is not one of the thirteen denied requests"),
            }
        };
        errno_of(rc)
    }

    #[cfg(target_arch = "x86_64")]
    fn issue_denied_requests(fd: std::os::fd::RawFd) -> Vec<(&'static str, i32)> {
        DENIED_REQUESTS.iter().map(|name| (*name, issue_denied(name, fd))).collect()
    }

    /// The `fd=` path's requests on the queue and on an `AF_INET` socket, each
    /// restating the TAP's current state.
    #[cfg(target_arch = "x86_64")]
    fn issue_allowed_requests(queue_fd: std::os::fd::RawFd, tap: &str) -> Vec<(&'static str, i32)> {
        use std::os::fd::FromRawFd;

        // SAFETY: a close-on-exec datagram socket, owned exactly once below.
        let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
        assert!(raw >= 0, "open a socket for the MTU requests: {}", io::Error::last_os_error());
        // SAFETY: `raw` is an open descriptor owned by nothing else.
        let socket = unsafe { OwnedFd::from_raw_fd(raw) };
        let mut read_flags = named_ifreq("");
        let mut reattach = named_ifreq(tap);
        reattach.ifr_ifru.ifru_flags = tap_queue_flags();
        let mut header_size: libc::c_int = 0;
        let mut mtu = named_ifreq(tap);
        let no_offload: libc::c_ulong = 0;
        let socket_fd = socket.as_raw_fd();
        // SAFETY: each call passes an open descriptor and either an integer
        // argument or a pointer to a live, correctly sized stack object; the
        // calls run in order, so each restating request carries the value the
        // read before it observed.
        unsafe {
            vec![
                (
                    "TUNGETIFF",
                    errno_of(libc::ioctl(queue_fd, libc::TUNGETIFF, &raw mut read_flags)),
                ),
                ("TUNSETIFF", errno_of(libc::ioctl(queue_fd, libc::TUNSETIFF, &raw mut reattach))),
                (
                    "TUNGETVNETHDRSZ",
                    errno_of(libc::ioctl(queue_fd, libc::TUNGETVNETHDRSZ, &raw mut header_size)),
                ),
                (
                    "TUNSETVNETHDRSZ",
                    errno_of(libc::ioctl(queue_fd, libc::TUNSETVNETHDRSZ, &raw mut header_size)),
                ),
                ("SIOCGIFMTU", errno_of(libc::ioctl(socket_fd, libc::SIOCGIFMTU, &raw mut mtu))),
                ("SIOCSIFMTU", errno_of(libc::ioctl(socket_fd, libc::SIOCSIFMTU, &raw mut mtu))),
                ("TUNSETOFFLOAD", errno_of(libc::ioctl(queue_fd, libc::TUNSETOFFLOAD, no_offload))),
            ]
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn child_requests(tap: &str) {
        let queue = super::VMM_TAP_QUEUE_FD;
        let first = issue_denied_requests(queue);
        let spawned: Vec<_> =
            (0..3).map(|_| std::thread::spawn(move || issue_denied_requests(queue))).collect();
        for (name, errno) in first {
            report(format_args!("denied thread=main request={name} errno={errno}"));
        }
        for (index, thread) in spawned.into_iter().enumerate() {
            for (name, errno) in thread.join().expect("request thread") {
                report(format_args!("denied thread=spawned-{index} request={name} errno={errno}"));
            }
        }
        for (name, errno) in issue_allowed_requests(queue, tap) {
            report(format_args!("allowed request={name} errno={errno}"));
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn child_tasks() {
        use std::sync::{Arc, Barrier};

        let running = Arc::new(Barrier::new(4));
        let release = Arc::new(Barrier::new(4));
        let threads: Vec<_> = (0..3)
            .map(|_| {
                let running = Arc::clone(&running);
                let release = Arc::clone(&release);
                std::thread::spawn(move || {
                    running.wait();
                    release.wait();
                })
            })
            .collect();
        running.wait();
        let fds = open_descriptors();
        report(format_args!("descriptors fds={}", joined(&fds)));
        for entry in std::fs::read_dir("/proc/self/task").expect("list /proc/self/task") {
            let tid = entry.expect("read a task entry").file_name().to_string_lossy().into_owned();
            let status = std::fs::read_to_string(format!("/proc/self/task/{tid}/status"))
                .expect("read the task status");
            let field = |key: &str| {
                status
                    .lines()
                    .find_map(|line| line.strip_prefix(key).map(|value| value.trim().to_owned()))
                    .unwrap_or_else(|| "absent".to_owned())
            };
            report(format_args!(
                "task tid={tid} no_new_privs={} seccomp={}",
                field("NoNewPrivs:"),
                field("Seccomp:"),
            ));
        }
        release.wait();
        for thread in threads {
            thread.join().expect("task thread");
        }
    }

    /// `__X32_SYSCALL_BIT + 514`: the x32 `ioctl` entry
    /// (`arch/x86/entry/syscalls/syscall_64.tbl:409`).
    #[cfg(target_arch = "x86_64")]
    const X32_IOCTL: libc::c_long = 0x4000_0000 + 514;
    /// i386 `__NR_ioctl` and `__NR_getpid` (`syscall_32.tbl`).
    #[cfg(target_arch = "x86_64")]
    const I386_IOCTL: u32 = 54;
    #[cfg(target_arch = "x86_64")]
    const I386_GETPID: u32 = 20;

    #[cfg(target_arch = "x86_64")]
    fn child_x32() {
        let owner_request = libc::c_long::try_from(libc::TUNSETOWNER).expect("request fits");
        // SAFETY: an x32-ABI `ioctl` on the queue with an invalid owner id.
        // Under the launch filter the kernel ends the process before dispatch;
        // unfiltered, it returns ENOSYS (no x32 ABI) or EINVAL.
        let rc = unsafe {
            libc::syscall(
                X32_IOCTL,
                libc::c_long::from(super::VMM_TAP_QUEUE_FD),
                owner_request,
                libc::c_long::from(u32::MAX),
            )
        };
        let errno =
            if rc == -1 { io::Error::last_os_error().raw_os_error().unwrap_or(0) } else { 0 };
        report(format_args!("x32 returned={rc} errno={errno}"));
    }

    /// One i386 system call through `int 0x80`.
    #[cfg(target_arch = "x86_64")]
    fn int80(nr: u32, arg0: u32, arg1: u32, arg2: u32) -> u32 {
        let mut result = nr;
        // SAFETY: `int 0x80` enters the i386 compat system-call path, or
        // faults where the kernel provides no i386 entry. `ebx` carries the
        // first argument: LLVM reserves `rbx`, so it is exchanged in and
        // restored around the instruction; `r8`-`r11` are declared clobbered.
        unsafe {
            std::arch::asm!(
                "xchg {arg0}, rbx",
                "int 0x80",
                "xchg {arg0}, rbx",
                arg0 = inout(reg) u64::from(arg0) => _,
                inout("eax") result,
                in("ecx") arg1,
                in("edx") arg2,
                out("r8") _,
                out("r9") _,
                out("r10") _,
                out("r11") _,
            );
        }
        result
    }

    #[cfg(target_arch = "x86_64")]
    fn child_i386_ioctl() {
        let owner_request = u32::try_from(libc::TUNSETOWNER).expect("request fits 32 bits");
        let queue = u32::try_from(super::VMM_TAP_QUEUE_FD).expect("descriptor fits 32 bits");
        let returned = int80(I386_IOCTL, queue, owner_request, u32::MAX);
        report(format_args!("i386 returned={returned:#x}"));
    }

    #[cfg(target_arch = "x86_64")]
    fn child_i386_control() {
        let returned = int80(I386_GETPID, 0, 0, 0);
        report(format_args!("i386_control getpid={returned} pid={}", std::process::id()));
    }

    /// The re-exec child role of the x86_64 cases. It does nothing unless a
    /// parent case re-executes this binary with `OVERDRIVE_LAUNCH_SECCOMP_CHILD`
    /// set; it then observes itself and prints `OVERDRIVE_LAUNCH_SECCOMP_REPORT`
    /// records for the parent's oracle.
    #[cfg(target_arch = "x86_64")]
    #[test]
    #[ignore = "re-exec child role: needs the queue and launch hook a launch_seccomp_kernel parent case sets up"]
    fn launch_seccomp_child_role() {
        let Some(role) = std::env::var_os(CHILD_ROLE_ENV) else {
            return;
        };
        let tap = std::env::var(CHILD_TAP_ENV).expect("the parent names the scratch TAP");
        match role.to_str() {
            Some("descriptors") => child_descriptors(),
            Some("requests") => child_requests(&tap),
            Some("tasks") => child_tasks(),
            Some("x32") => child_x32(),
            Some("i386") => child_i386_ioctl(),
            Some("i386-control") => child_i386_control(),
            other => panic!("unknown launch_seccomp child role {other:?}"),
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-41 — The launched child inherits exactly descriptors 0 to 3.
    /// CONTRACT_SHAPE: bounded-change.
    #[cfg(target_arch = "x86_64")]
    #[test]
    #[ignore = "pending DELIVER step 05-02 (S-ND295-41)"]
    fn the_launched_child_inherits_exactly_descriptors_zero_to_three() {
        let tap = ScratchTap::create();
        let queue = attach_scratch_queue(&tap.name);
        let inheritable = InheritableDescriptors::open();

        for (first_closed, expected_fds, expected_fd3) in
            [(super::VMM_TAP_QUEUE_FD + 1, "0,1,2,3", tap.name.as_str()), (3, "0,1,2", "-")]
        {
            let child = run_child("descriptors", &tap, &queue, Launch::Hooked { first_closed });
            child.assert_completed();
            let records = child.records("descriptors");
            assert_eq!(records.len(), 1, "one descriptor report per child: {child}");
            assert_eq!(
                records[0]["fds"], expected_fds,
                "with first_closed = {first_closed} the exec'd child must hold exactly {expected_fds}: {child}",
            );
            assert_eq!(
                records[0]["fd3_iff"], expected_fd3,
                "descriptor 3 must be the scratch TAP queue exactly when first_closed = 4: {child}",
            );
        }
        inheritable.assert_inheritable();
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-43 — The production launch hook denies TAP-mutating requests on
    /// every thread.
    /// CONTRACT_SHAPE: bounded-change.
    #[cfg(target_arch = "x86_64")]
    #[test]
    #[ignore = "pending DELIVER step 05-02 (S-ND295-43)"]
    fn every_denied_request_returns_eperm_on_every_thread_under_the_production_hook() {
        let tap = ScratchTap::create();
        let queue = attach_scratch_queue(&tap.name);

        // Contrast: unfiltered, no denied request is refused with EPERM, so
        // every EPERM below is the launch filter's.
        let control = run_child("requests", &tap, &queue, Launch::Unhooked);
        control.assert_completed();
        let unfiltered = control.records("denied");
        assert_eq!(unfiltered.len(), 13 * 4, "13 requests on 4 threads: {control}");
        for record in &unfiltered {
            assert_ne!(
                record["errno"],
                libc::EPERM.to_string(),
                "unfiltered, {} must not be refused with EPERM: {control}",
                record["request"],
            );
        }

        let filtered = run_child(
            "requests",
            &tap,
            &queue,
            Launch::Hooked { first_closed: super::VMM_TAP_QUEUE_FD + 1 },
        );
        filtered.assert_completed();
        let denied: std::collections::BTreeMap<(String, String), String> = filtered
            .records("denied")
            .into_iter()
            .map(|record| {
                ((record["thread"].clone(), record["request"].clone()), record["errno"].clone())
            })
            .collect();
        for thread in ["main", "spawned-0", "spawned-1", "spawned-2"] {
            for request in DENIED_REQUESTS {
                assert_eq!(
                    denied.get(&(thread.to_owned(), request.to_owned())),
                    Some(&libc::EPERM.to_string()),
                    "{request} on thread {thread} must return EPERM under the launch filter: {filtered}",
                );
            }
        }
        assert_eq!(denied.len(), 13 * 4, "exactly 13 requests on 4 threads: {filtered}");

        let allowed = filtered.records("allowed");
        let issued: Vec<&str> = allowed.iter().map(|record| record["request"].as_str()).collect();
        assert_eq!(issued, ALLOWED_REQUESTS, "every fd= path request was issued: {filtered}");
        for record in &allowed {
            assert_ne!(
                record["errno"],
                libc::EPERM.to_string(),
                "{} must not be refused by the launch filter: {filtered}",
                record["request"],
            );
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-43 — The production launch hook denies TAP-mutating requests on
    /// every thread (per-thread no-new-privileges and filter mode).
    /// CONTRACT_SHAPE: bounded-change.
    #[cfg(target_arch = "x86_64")]
    #[test]
    #[ignore = "pending DELIVER step 05-02 (S-ND295-43)"]
    fn every_filtered_thread_reports_no_new_privs_and_filter_mode() {
        let tap = ScratchTap::create();
        let queue = attach_scratch_queue(&tap.name);

        let child = run_child(
            "tasks",
            &tap,
            &queue,
            Launch::Hooked { first_closed: super::VMM_TAP_QUEUE_FD + 1 },
        );
        child.assert_completed();
        let descriptors = child.records("descriptors");
        assert_eq!(descriptors.len(), 1, "one descriptor report: {child}");
        assert_eq!(
            descriptors[0]["fds"], "0,1,2,3",
            "the descriptor table is exactly 0-3: {child}"
        );
        let tasks = child.records("task");
        assert!(
            tasks.len() >= 4,
            "the reporting thread and three threads created after exec must be listed: {child}",
        );
        for task in &tasks {
            assert_eq!(
                (task["no_new_privs"].as_str(), task["seccomp"].as_str()),
                ("1", "2"),
                "task {} must report NoNewPrivs 1 and Seccomp 2 (filter mode): {child}",
                task["tid"],
            );
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-43 — The production launch hook denies TAP-mutating requests on
    /// every thread (the startup probe installs the launch program).
    /// CONTRACT_SHAPE: bounded-change.
    #[cfg(target_arch = "x86_64")]
    #[tokio::test]
    #[ignore = "pending DELIVER step 05-02 (S-ND295-43)"]
    async fn the_startup_probe_installs_the_exact_launch_program() {
        use super::VmmProbeSubstrate;

        let outcome = super::RealVmmProbeSubstrate.check_launch_seccomp().await;
        assert!(
            outcome.is_ok(),
            "the kernel must accept the exact launch program and prlimit must exec under it: \
             {outcome:?}",
        );
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-43 — The production launch hook denies TAP-mutating requests on
    /// every thread (foreign syscall ABIs end the process).
    /// CONTRACT_SHAPE: bounded-change.
    #[cfg(target_arch = "x86_64")]
    #[test]
    #[ignore = "pending DELIVER step 05-02 (S-ND295-43)"]
    fn foreign_syscall_abis_end_the_filtered_process() {
        let tap = ScratchTap::create();
        let queue = attach_scratch_queue(&tap.name);
        let hooked = Launch::Hooked { first_closed: super::VMM_TAP_QUEUE_FD + 1 };

        // Contrast: unfiltered, the x32 ioctl returns (ENOSYS or EINVAL), so
        // a SIGSYS below is the launch filter's.
        let x32_control = run_child("x32", &tap, &queue, Launch::Unhooked);
        x32_control.assert_completed();
        assert_eq!(x32_control.records("x32").len(), 1, "unfiltered x32 returns: {x32_control}");

        let x32 = run_child("x32", &tap, &queue, hooked);
        assert_eq!(x32.signal(), Some(libc::SIGSYS), "an x32 syscall must end the process: {x32}");
        assert!(x32.records("x32").is_empty(), "the x32 ioctl must never return: {x32}");

        // The i386 entry exists only where the kernel provides IA-32
        // emulation; the unfiltered control says whether it does here.
        let i386_control = run_child("i386-control", &tap, &queue, Launch::Unhooked);
        let i386_entry = i386_control.output.status.success()
            && i386_control
                .records("i386_control")
                .first()
                .is_some_and(|record| record["getpid"] == record["pid"]);

        let i386 = run_child("i386", &tap, &queue, hooked);
        assert!(i386.records("i386").is_empty(), "the i386 ioctl must never return: {i386}");
        assert!(i386.signal().is_some(), "the i386 ioctl child must end on a signal: {i386}");
        if i386_entry {
            assert_eq!(
                i386.signal(),
                Some(libc::SIGSYS),
                "with the i386 entry present, an i386 syscall must end the process: {i386}",
            );
        } else {
            eprintln!(
                "i386 entry not provided by this kernel (control: {i386_control}); the filtered \
                 i386 child ended on signal {:?}",
                i386.signal(),
            );
        }
    }

    /// Restores `PATH` on drop.
    #[cfg(target_arch = "x86_64")]
    struct PathGuard {
        prior: Option<std::ffi::OsString>,
    }

    #[cfg(target_arch = "x86_64")]
    impl PathGuard {
        fn replace(value: &Path) -> Self {
            let prior = std::env::var_os("PATH");
            // SAFETY: `#[serial(env)]` gives this test exclusive use of the
            // process environment, and no other thread of it is running yet.
            unsafe { std::env::set_var("PATH", value) };
            Self { prior }
        }
    }

    #[cfg(target_arch = "x86_64")]
    impl Drop for PathGuard {
        fn drop(&mut self) {
            // SAFETY: as in `replace`; the runtime is dropped before this guard.
            unsafe {
                match &self.prior {
                    Some(prior) => std::env::set_var("PATH", prior),
                    None => std::env::remove_var("PATH"),
                }
            }
        }
    }

    /// The staging root must clone by `FICLONE`, as `create` requires.
    #[cfg(target_arch = "x86_64")]
    fn assert_reflink_capable(dir: &Path) {
        let source = dir.join(".nd295-reflink-probe");
        let clone = dir.join(".nd295-reflink-probe.clone");
        std::fs::write(&source, [0xcd_u8; 4096]).expect("write the reflink probe");
        let src = std::fs::File::open(&source).expect("open the reflink probe");
        let dst = std::fs::File::create_new(&clone).expect("create the reflink clone");
        rustix::fs::ioctl_ficlone(&dst, &src).unwrap_or_else(|error| {
            panic!(
                "precondition: {} must be reflink-capable (the native metal /srv/vm is \
                 XFS-reflink): {error}",
                dir.display()
            )
        });
        drop((src, dst));
        std::fs::remove_file(&clone).expect("remove the reflink clone");
        std::fs::remove_file(&source).expect("remove the reflink probe");
    }

    /// What the process held while `create` was parked on its cleanup await.
    #[cfg(target_arch = "x86_64")]
    #[derive(Debug)]
    struct CleanupAwait {
        queue_descriptors: Vec<i32>,
        clone_present: bool,
        carrier: (u64, u64),
    }

    /// Drive `create` by hand on a runtime with one blocking thread. At every
    /// await it records any descriptor of this process holding the queue.
    /// Once `create` has handed the clone to the confined uid (its last
    /// blocking step before the spawn), a gate occupies the blocking thread,
    /// so the next poll runs the attach and the spawn and parks on the
    /// clone-removal await; the process is observed there, then released.
    #[cfg(target_arch = "x86_64")]
    async fn drive_create_through_its_cleanup_await(
        vmm: &CloudHypervisorVmm,
        config: &VmConfig,
        tap: &str,
    ) -> (super::Result<super::VmProcess>, Option<CleanupAwait>, Vec<Vec<i32>>) {
        use super::Vmm;

        let clone_dest = config.rootfs.clone_dest().to_path_buf();
        let confined_uid = config.confinement.identity().uid;
        let mut create = std::pin::pin!(vmm.create(config));
        let mut held_at_awaits = Vec::new();
        let mut parked = None;
        let mut gate: Option<tokio::task::JoinHandle<()>> = None;
        loop {
            let polled =
                std::future::poll_fn(|cx| std::task::Poll::Ready(create.as_mut().poll(cx))).await;
            if let std::task::Poll::Ready(result) = polled {
                if let Some(gate) = gate {
                    gate.await.expect("the blocking-thread gate completes");
                }
                return (result, parked, held_at_awaits);
            }
            let holders = descriptors_holding_queue_of(tap);
            if !holders.is_empty() {
                held_at_awaits.push(holders);
            }
            if gate.is_none() && owner_uid(&clone_dest) == Some(confined_uid) {
                let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
                let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
                gate = Some(tokio::task::spawn_blocking(move || {
                    started_tx.send(()).expect("announce the gate");
                    release_rx.recv().expect("the case releases the gate");
                }));
                started_rx
                    .recv_timeout(std::time::Duration::from_secs(60))
                    .expect("the gate holds the only blocking thread once the path handover ends");
                let resumed =
                    std::future::poll_fn(|cx| std::task::Poll::Ready(create.as_mut().poll(cx)))
                        .await;
                assert!(
                    resumed.is_pending(),
                    "create must park on its clone-removal await behind the occupied blocking thread",
                );
                parked = Some(CleanupAwait {
                    queue_descriptors: descriptors_holding_queue_of(tap),
                    clone_present: clone_dest.exists(),
                    carrier: carrier_counts(tap),
                });
                release_tx.send(()).expect("release the gate");
            }
            tokio::task::yield_now().await;
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-40 — The VMM adapter launches from the queue and reports queue
    /// failures in their own terms (a failed spawn releases the queue before
    /// any cleanup await).
    /// CONTRACT_SHAPE: bounded-change.
    #[cfg(target_arch = "x86_64")]
    #[test]
    #[serial_test::serial(env)]
    #[ignore = "pending DELIVER step 05-03 (S-ND295-40)"]
    fn a_failed_spawn_releases_the_queue_before_any_cleanup_await() {
        let staging_root = overdrive_testing::vm_fixture::default_staging_root();
        std::fs::create_dir_all(&staging_root).expect("create the VM staging root");
        let root = tempfile::Builder::new()
            .prefix("nd295-launch-")
            .tempdir_in(&staging_root)
            .expect("per-case directory on the VM staging root");
        assert_reflink_capable(root.path());
        let empty_path = root.path().join("empty-path");
        std::fs::create_dir(&empty_path).expect("create an empty PATH directory");
        let tap = ScratchTap::create();
        let fixture = LaunchFixture::new(
            root,
            VmNetworkAttachment {
                tap: tap.name.clone(),
                mac: [0x02, 0x00, 0x00, 0x00, 0x29, 0x5a],
            },
        );
        let clone_dest = fixture.config.rootfs.clone_dest().to_path_buf();
        let vmm = fixture.vmm();
        // The launch wrapper `prlimit` resolves through PATH; an empty PATH
        // makes the spawn itself fail after the queue is attached.
        let path_guard = PathGuard::replace(&empty_path);
        let before = carrier_counts(&tap.name);

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .expect("single-blocking-thread runtime");
        let (result, parked, held_at_awaits) = runtime
            .block_on(drive_create_through_its_cleanup_await(&vmm, &fixture.config, &tap.name));
        drop(runtime);
        drop(path_guard);

        assert!(
            held_at_awaits.is_empty(),
            "the parent must hold no queue descriptor at any await of create: {held_at_awaits:?}",
        );
        let parked = parked.unwrap_or_else(|| {
            panic!("create never handed its clone to the confined uid: {result:?}")
        });
        assert!(parked.clone_present, "the observation must precede the clone removal: {parked:?}");
        assert!(
            parked.queue_descriptors.is_empty(),
            "no descriptor of the parent may hold the queue at the cleanup await: {parked:?}",
        );
        let attached = parked.carrier.0 - before.0;
        let released = parked.carrier.1 - before.1;
        assert!(attached >= 1, "create must attach the queue before its spawn: {parked:?}");
        assert_eq!(
            released, attached,
            "every attached queue must be closed before the cleanup await: {parked:?}",
        );
        match &result {
            Err(super::VmmError::ConfinementUnavailable {
                control: super::ConfinementControl::UidDrop,
                detail,
            }) => assert!(
                detail.contains("prlimit"),
                "the spawn failure must name the launch wrapper: {detail}",
            ),
            other => panic!("create must fail at the wrapper spawn: {other:?}"),
        }
        assert!(!clone_dest.exists(), "the failed create removes its rootfs clone");
        let fresh = overdrive_netlink::attach_tap_queue(&tap.name)
            .expect("after the failed spawn no queue is held, so a fresh attach succeeds");
        assert_eq!(fresh.name(), tap.name);
    }

    // ------------------------------------------------------------------
    // Every other target: no program, so no microVM (user ruling 10)
    // ------------------------------------------------------------------

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-44 — A target with no launch filter starts no microVM (the
    /// probe names the architecture and the launch is refused before any
    /// effect).
    /// CONTRACT_SHAPE: bounded-change.
    #[cfg(not(target_arch = "x86_64"))]
    #[tokio::test]
    #[ignore = "pending DELIVER step 05-02 (S-ND295-44)"]
    async fn launch_on_a_target_without_a_program_is_refused_before_any_effect() {
        use super::{
            ConfinementControl, RealVmmProbeSubstrate, Vmm, VmmError, VmmProbeError,
            VmmProbeSubstrate,
        };

        let arch = std::env::consts::ARCH;
        match RealVmmProbeSubstrate.check_launch_seccomp().await {
            Err(VmmProbeError::LaunchSeccompUnsupportedArch { target_arch }) => {
                assert_eq!(target_arch, arch, "the probe stage must name the running architecture");
            }
            other => {
                panic!("the {arch} launch-seccomp stage must refuse as unsupported: {other:?}")
            }
        }

        let tap = ScratchTap::create();
        let fixture = LaunchFixture::new(
            tempfile::tempdir().expect("per-case directory"),
            VmNetworkAttachment {
                tap: tap.name.clone(),
                mac: [0x02, 0x00, 0x00, 0x00, 0x29, 0x5b],
            },
        );
        let clone_dest = fixture.config.rootfs.clone_dest().to_path_buf();
        let staging =
            clone_dest.parent().expect("the clone lives in the staging dir").to_path_buf();
        let run_dir = fixture.config.run_dir.path().to_path_buf();
        let run_dir_owner = owner_uid(&run_dir);
        let carrier_before = carrier_counts(&tap.name);

        let result = fixture.vmm().create(&fixture.config).await;

        match &result {
            Err(VmmError::ConfinementUnavailable {
                control: ConfinementControl::Seccomp,
                detail,
            }) => {
                assert_eq!(
                    detail,
                    &format!("no VMM launch seccomp program for target architecture {arch}"),
                );
            }
            other => panic!("a launch on {arch} must be refused for its missing filter: {other:?}"),
        }
        assert!(!clone_dest.exists(), "no rootfs clone may be made");
        assert_eq!(
            std::fs::read_dir(&staging).expect("list the staging dir").count(),
            0,
            "the clone staging directory must stay empty",
        );
        assert!(!fixture.config.run_dir.kernel_copy().exists(), "no kernel copy may be made");
        assert_eq!(owner_uid(&run_dir), run_dir_owner, "the run directory must not be handed over");
        assert_eq!(carrier_counts(&tap.name), carrier_before, "no queue may have been attached");
        assert!(descriptors_holding_queue_of(&tap.name).is_empty(), "no queue may be held");
        drop(attach_scratch_queue(&tap.name));
    }
}
