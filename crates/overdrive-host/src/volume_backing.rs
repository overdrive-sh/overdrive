//! `LvmVolumeBacking` and `LvmVolumePoolProbe`: the production volume pool binding.
//!
//! GH #348, ADR-0312, ADR-0313, ADR-0314, ADR-0317. The pool is provisioned by the node
//! (one GPT partition labelled `overdrive-volumes` holding a physical volume and the volume
//! group `overdrive`); Overdrive discovers it and never creates it. Every `lvm` spawn goes
//! through [`LvmTool`]: `lvm <subcommand> --config '<pinned>' ...` with
//! `LVM_SYSTEM_DIR=/run/lvm`, so [`LvmVolumeBacking`] is the only place `lvm` runs.

use std::io;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::id::WorkloadId;
use overdrive_core::traits::volume_backing::{
    ClaimState, VolumeBacking, VolumeBackingError, VolumePoolProbe, VolumeProbeDetail,
    VolumeProbeError, VolumeProbeScenario,
};
use overdrive_core::volume::{ByteCount, VolumeOperation, VolumeSize};
use tokio::process::Command;
use tokio::time::Instant;

/// The volume group provisioning creates on the labelled partition.
const VOLUME_GROUP: &str = "overdrive";
/// The kernel `PARTNAME` of the pool's partition.
const PARTITION_LABEL: &str = "overdrive-volumes";
/// Where the kernel lists block devices; each carries a `uevent` with its `PARTNAME`.
const SYSFS_BLOCK_DIR: &str = "/sys/class/block";
/// Where device-mapper creates the nodes of active logical volumes.
const DEVICE_MAPPER_DIR: &str = "/dev/mapper";
/// `lvm`'s system directory on a node whose root is read-only except `/run` (V-1).
const LVM_SYSTEM_DIR: &str = "/run/lvm";
/// The one deadline of an `ensure` call, covering every `lvm` spawn it makes (ADR-0317).
const ENSURE_DEADLINE: Duration = Duration::from_secs(30);
/// The scratch volume the boot probe creates and removes; any `ovd-probe-` volume is a leftover.
const SCRATCH_PREFIX: &str = "ovd-probe-";
const SCRATCH_VOLUME: &str = "ovd-probe-scratch";
/// One extent: the smallest volume `lvcreate` makes in a group of 4 MiB extents.
const SCRATCH_BYTES: u64 = 4 * 1024 * 1024;

/// How `lvm` is spawned: the pinned `--config`, built from the partition node the label found.
#[derive(Debug, Clone)]
struct LvmTool {
    config: String,
}

impl LvmTool {
    fn over(partition: &Path) -> Self {
        Self {
            config: format!(
                "activation{{udev_sync=0 udev_rules=0 monitoring=0}} \
                 devices{{filter=[\"a|^{}$|\",\"r|.*|\"] use_devicesfile=0}} \
                 backup{{backup=0 archive=0}} \
                 global{{use_lvmpolld=0 locking_dir=\"/run/lock/lvm\"}}",
                partition.display()
            ),
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new("lvm");
        command.args(args).arg("--config").arg(&self.config).env("LVM_SYSTEM_DIR", LVM_SYSTEM_DIR);
        command
    }
}

/// The dm name of a logical volume in the `overdrive` group is `<vg>-<lv>` with every `-` inside
/// the group and the volume name doubled, so `ovd-<hash>` is `overdrive-ovd--<hash>`.
fn device_node(logical_volume: &str) -> PathBuf {
    let name = format!("{VOLUME_GROUP}-{}", logical_volume.replace('-', "--"));
    Path::new(DEVICE_MAPPER_DIR).join(name)
}

/// `ovd-` plus the first 32 lowercase hexadecimal characters of the SHA-256 of the workload id
/// (ADR-0312).
fn volume_name(workload: &WorkloadId) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = ring::digest::digest(&ring::digest::SHA256, workload.as_str().as_bytes());
    let mut name = String::with_capacity(4 + 32);
    name.push_str("ovd-");
    for byte in digest.as_ref().iter().take(16) {
        name.push(char::from(HEX[usize::from(byte >> 4)]));
        name.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    name
}

const fn pool_command(operation: VolumeOperation, source: io::Error) -> VolumeBackingError {
    VolumeBackingError::PoolCommand { operation, source }
}

fn timed_out(operation: VolumeOperation) -> VolumeBackingError {
    pool_command(
        operation,
        io::Error::new(io::ErrorKind::TimedOut, "the volume pool ensure deadline passed"),
    )
}

fn is_timed_out(error: &VolumeBackingError) -> bool {
    matches!(error, VolumeBackingError::PoolCommand { source, .. } if source.kind() == io::ErrorKind::TimedOut)
}

/// The names in `lvs --noheadings -o lv_name` output.
fn logical_volume_names(report: &str) -> impl Iterator<Item = &str> {
    report.lines().map(str::trim).filter(|line| !line.is_empty())
}

/// The only place that spawns `lvm`.
///
/// Its constructor is `pub(crate)`, so only [`LvmVolumePoolProbe`] builds one, after the probe
/// scenarios pass; the private fields keep every other crate from constructing it.
#[derive(Debug)]
pub struct LvmVolumeBacking {
    tool: LvmTool,
}

impl LvmVolumeBacking {
    /// A backing over the pool on `partition`, the node the label discovery found.
    pub(crate) fn new(partition: &Path) -> Self {
        Self { tool: LvmTool::over(partition) }
    }

    /// Runs one `lvm` command inside the `ensure` deadline. A command still running at the
    /// deadline is not killed or aborted: the task that owns the child outlives this call, reaps
    /// it when it ends on its own, and discards its output (ADR-0317).
    async fn run(
        &self,
        operation: VolumeOperation,
        args: &[&str],
        deadline: Instant,
    ) -> Result<Output, VolumeBackingError> {
        if Instant::now() >= deadline {
            return Err(timed_out(operation));
        }
        let child = self
            .tool
            .command(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| pool_command(operation, source))?;
        let mut reaper = tokio::spawn(child.wait_with_output());
        // Dropping a `JoinHandle` detaches its task: on the deadline the reaper keeps waiting.
        let finished = match tokio::time::timeout_at(deadline, &mut reaper).await {
            Ok(finished) => finished,
            Err(_elapsed) => return Err(timed_out(operation)),
        };
        let output = finished
            .map_err(|joined| pool_command(operation, io::Error::other(joined)))?
            .map_err(|source| pool_command(operation, source))?;
        if output.status.success() {
            return Ok(output);
        }
        let subcommand = args.first().copied().unwrap_or("lvm");
        Err(pool_command(
            operation,
            io::Error::other(format!(
                "lvm {subcommand} exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )),
        ))
    }

    async fn volume_names(&self, deadline: Instant) -> Result<Vec<String>, VolumeBackingError> {
        let output = self
            .run(
                VolumeOperation::Report,
                &["lvs", "--noheadings", "-o", "lv_name", VOLUME_GROUP],
                deadline,
            )
            .await?;
        let report = String::from_utf8_lossy(&output.stdout);
        Ok(logical_volume_names(&report).map(str::to_owned).collect())
    }

    async fn free_bytes(&self, deadline: Instant) -> Result<u64, VolumeBackingError> {
        let output = self
            .run(
                VolumeOperation::Report,
                &[
                    "vgs",
                    "--noheadings",
                    "--units",
                    "b",
                    "--nosuffix",
                    "-o",
                    "vg_free",
                    VOLUME_GROUP,
                ],
                deadline,
            )
            .await?;
        let report = String::from_utf8_lossy(&output.stdout);
        report.trim().parse().map_err(|_| {
            pool_command(
                VolumeOperation::Report,
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("lvm vgs reported free space {:?}, not a byte count", report.trim()),
                ),
            )
        })
    }

    /// Creates `name` with exactly `size` reserved, written nowhere. A failing `lvcreate` after
    /// which the volume exists (an earlier attempt's command outlived its deadline and finished)
    /// is the existing volume.
    async fn create(
        &self,
        workload: &WorkloadId,
        name: &str,
        size: VolumeSize,
        deadline: Instant,
    ) -> Result<(), VolumeBackingError> {
        let free = self.free_bytes(deadline).await?;
        if free < size.bytes() {
            return Err(VolumeBackingError::InsufficientSpace {
                needed: size,
                free: ByteCount::new(free),
            });
        }
        let bytes = format!("{}b", size.bytes());
        let created = self
            .run(
                VolumeOperation::Create,
                &[
                    "lvcreate",
                    "--zero",
                    "n",
                    "--wipesignatures",
                    "n",
                    "-L",
                    &bytes,
                    "-n",
                    name,
                    VOLUME_GROUP,
                ],
                deadline,
            )
            .await;
        let Err(failure) = created else { return Ok(()) };
        if is_timed_out(&failure) {
            return Err(failure);
        }
        tracing::warn!(
            name: "volume_backing.ensure.create_failed",
            workload = %workload,
            error = %failure,
        );
        if self.volume_names(deadline).await?.iter().any(|existing| existing == name) {
            return Ok(());
        }
        Err(failure)
    }

    async fn activate(
        &self,
        name: &str,
        node: &Path,
        deadline: Instant,
    ) -> Result<(), VolumeBackingError> {
        self.run(
            VolumeOperation::Activate,
            &["lvchange", "-ay", &format!("{VOLUME_GROUP}/{name}")],
            deadline,
        )
        .await?;
        if node_exists(node).await? {
            return Ok(());
        }
        Err(pool_command(
            VolumeOperation::Activate,
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("lvm activated the volume but {} did not appear", node.display()),
            ),
        ))
    }
}

async fn node_exists(node: &Path) -> Result<bool, VolumeBackingError> {
    tokio::fs::try_exists(node)
        .await
        .map_err(|source| pool_command(VolumeOperation::Activate, source))
}

#[async_trait]
impl VolumeBacking for LvmVolumeBacking {
    async fn ensure(
        &self,
        workload: &WorkloadId,
        size: VolumeSize,
    ) -> Result<PathBuf, VolumeBackingError> {
        let deadline = Instant::now() + ENSURE_DEADLINE;
        let name = volume_name(workload);
        let node = device_node(&name);
        if !self.volume_names(deadline).await?.contains(&name) {
            self.create(workload, &name, size, deadline).await?;
        }
        if !node_exists(&node).await? {
            self.activate(&name, &node, deadline).await?;
        }
        Ok(node)
    }

    /// One `O_EXCL` open of the device-mapper node, released at once. On a block device `O_EXCL`
    /// makes the opening file the holder (`blkdev_open`, `vendors/linux/block/fops.c:674-687 @
    /// v7.0`), and a device another holder has claimed fails the open `-EBUSY`
    /// (`bd_prepare_to_claim`, `vendors/linux/block/bdev.c:596-610 @ v7.0`). An inactive volume
    /// has no node, so the open fails `ENOENT`.
    async fn claim_state(&self, workload: &WorkloadId) -> Result<ClaimState, VolumeBackingError> {
        let node = device_node(&volume_name(workload));
        let attempt = tokio::task::spawn_blocking(move || {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_EXCL)
                .open(node)
                .map(drop)
        })
        .await
        .map_err(|joined| pool_command(VolumeOperation::Claim, io::Error::other(joined)))?;
        match attempt {
            Ok(()) => Ok(ClaimState::Free),
            Err(error) if error.raw_os_error() == Some(libc::EBUSY) => Ok(ClaimState::Held),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Err(VolumeBackingError::NotFound { workload: workload.clone() })
            }
            Err(error) => Err(pool_command(VolumeOperation::Claim, error)),
        }
    }
}

/// The production [`VolumePoolProbe`]: discovers the partition labelled `overdrive-volumes`,
/// probes it, and builds the one [`LvmVolumeBacking`] over it.
#[derive(Debug, Default)]
pub struct LvmVolumePoolProbe {
    _private: (),
}

impl LvmVolumePoolProbe {
    /// Infallible and kernel-silent.
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }
}

const fn failure(scenario: VolumeProbeScenario, detail: VolumeProbeDetail) -> VolumeProbeError {
    VolumeProbeError { scenario, detail }
}

/// The device nodes of the partitions whose kernel `PARTNAME` is the pool label, read from sysfs
/// (no udev on the node).
async fn labelled_partitions() -> io::Result<Vec<PathBuf>> {
    let wanted = format!("PARTNAME={PARTITION_LABEL}");
    let mut entries = tokio::fs::read_dir(SYSFS_BLOCK_DIR).await?;
    let mut nodes = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let uevent = match tokio::fs::read_to_string(entry.path().join("uevent")).await {
            Ok(uevent) => uevent,
            // A device removed between the listing and the read is not a candidate.
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if uevent.lines().any(|line| line == wanted) {
            nodes.push(Path::new("/dev").join(entry.file_name()));
        }
    }
    nodes.sort();
    Ok(nodes)
}

/// Runs one probe `lvm` command to its end; a spawn failure is `Io`, a non-zero exit
/// `CommandFailed`, both under `scenario`. Returns its standard output.
async fn probe_command(
    tool: &LvmTool,
    scenario: VolumeProbeScenario,
    args: &[&str],
) -> Result<String, VolumeProbeError> {
    let output = tool
        .command(args)
        .output()
        .await
        .map_err(|source| failure(scenario, VolumeProbeDetail::Io(source)))?;
    if !output.status.success() {
        return Err(failure(
            scenario,
            VolumeProbeDetail::CommandFailed {
                exit_status: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            },
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Removes every scratch volume a crashed probe left (`lvremove -f` removes it active or not).
async fn remove_leftover_scratch(tool: &LvmTool) -> Result<(), VolumeProbeError> {
    let report = probe_command(
        tool,
        VolumeProbeScenario::VolumeGroupMissing,
        &["lvs", "--noheadings", "-o", "lv_name", VOLUME_GROUP],
    )
    .await?;
    for leftover in logical_volume_names(&report).filter(|name| name.starts_with(SCRATCH_PREFIX)) {
        tracing::warn!(name: "volume_backing.probe.leftover_scratch_removed", volume = leftover);
        probe_command(
            tool,
            VolumeProbeScenario::ScratchRemoveFailed,
            &["lvremove", "-f", &format!("{VOLUME_GROUP}/{leftover}")],
        )
        .await?;
    }
    Ok(())
}

/// Creates the scratch volume, checks its node, and removes it. A volume the create left behind
/// is removed before the failure is returned.
async fn probe_scratch(tool: &LvmTool) -> Result<(), VolumeProbeError> {
    let scratch = format!("{VOLUME_GROUP}/{SCRATCH_VOLUME}");
    let created = probe_command(
        tool,
        VolumeProbeScenario::ScratchCreateOrNodeFailed,
        &[
            "lvcreate",
            "--zero",
            "n",
            "--wipesignatures",
            "n",
            "-L",
            "4m",
            "-n",
            SCRATCH_VOLUME,
            VOLUME_GROUP,
        ],
    )
    .await;
    let node = device_node(SCRATCH_VOLUME);
    let outcome = match created {
        Err(error) => Err(error),
        Ok(_) => match tokio::fs::try_exists(&node).await {
            Ok(true) => Ok(()),
            Ok(false) => Err(failure(
                VolumeProbeScenario::ScratchCreateOrNodeFailed,
                VolumeProbeDetail::NodeMissing { path: node },
            )),
            Err(source) => Err(failure(
                VolumeProbeScenario::ScratchCreateOrNodeFailed,
                VolumeProbeDetail::Io(source),
            )),
        },
    };
    if let Err(error) = outcome {
        if let Err(cleanup) = probe_command(
            tool,
            VolumeProbeScenario::ScratchRemoveFailed,
            &["lvremove", "-f", &scratch],
        )
        .await
        {
            tracing::warn!(name: "volume_backing.probe.scratch_cleanup_failed", error = %cleanup);
        }
        return Err(error);
    }
    probe_command(tool, VolumeProbeScenario::ScratchRemoveFailed, &["lvremove", "-f", &scratch])
        .await
        .map(drop)
}

#[async_trait]
impl VolumePoolProbe for LvmVolumePoolProbe {
    async fn probe(&self) -> Result<Option<Arc<dyn VolumeBacking>>, VolumeProbeError> {
        let mut partitions = labelled_partitions().await.map_err(|source| {
            failure(VolumeProbeScenario::ToolsOrDeviceMapperUnusable, VolumeProbeDetail::Io(source))
        })?;
        let partition = match partitions.len() {
            0 => return Ok(None),
            1 => partitions.remove(0),
            _ => {
                return Err(failure(
                    VolumeProbeScenario::VolumeGroupMissing,
                    VolumeProbeDetail::AmbiguousLabel { nodes: partitions },
                ));
            }
        };
        let tool = LvmTool::over(&partition);
        probe_command(
            &tool,
            VolumeProbeScenario::ToolsOrDeviceMapperUnusable,
            &["pvs", "--noheadings", "-o", "pv_name"],
        )
        .await?;
        probe_command(
            &tool,
            VolumeProbeScenario::VolumeGroupMissing,
            &["vgs", "--noheadings", "-o", "vg_name", VOLUME_GROUP],
        )
        .await?;
        remove_leftover_scratch(&tool).await?;
        let free = probe_command(
            &tool,
            VolumeProbeScenario::VolumeGroupMissing,
            &["vgs", "--noheadings", "--units", "b", "--nosuffix", "-o", "vg_free", VOLUME_GROUP],
        )
        .await?;
        let free: u64 = free.trim().parse().map_err(|_| {
            failure(
                VolumeProbeScenario::VolumeGroupMissing,
                VolumeProbeDetail::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("lvm vgs reported free space {:?}, not a byte count", free.trim()),
                )),
            )
        })?;
        if free < SCRATCH_BYTES {
            tracing::warn!(name: "volume_backing.probe.scratch_skipped", free_bytes = free);
        } else {
            probe_scratch(&tool).await?;
        }
        Ok(Some(Arc::new(LvmVolumeBacking::new(&partition))))
    }
}
