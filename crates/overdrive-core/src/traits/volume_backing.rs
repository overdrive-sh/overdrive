//! `VolumeBacking` — the node-local volume pool port (GH #348, ADR-0312,
//! ADR-0317, ADR-0318).
//!
//! RED scaffold (DISTILL): the trait and its errors are the shapes the feature
//! delta § Port pins. The behavioural contract is the feature delta § Port;
//! DELIVER writes it into these docstrings.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::id::WorkloadId;
use crate::volume::{ByteCount, VolumeOperation, VolumeSize};

/// The kernel's exclusive claim on a volume's device node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimState {
    /// One non-blocking exclusive open succeeded.
    Free,
    /// The exclusive open failed `EBUSY`.
    Held,
}

/// Failures of the volume pool port.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VolumeBackingError {
    /// The volume has no device node (absent, or present but inactive).
    #[error("volume for workload {workload} not found or inactive")]
    NotFound {
        /// The workload whose volume was asked for.
        workload: WorkloadId,
    },
    /// Not enough free extents.
    #[error("insufficient volume pool space: needed {needed}, free {free:?}")]
    InsufficientSpace {
        /// The declared size.
        needed: VolumeSize,
        /// Free space observed before the create.
        free: ByteCount,
    },
    /// An `lvm` command or device operation failed, or the `ensure` deadline passed.
    #[error("volume pool command failed during {operation:?}: {source}")]
    PoolCommand {
        /// The operation in progress when it failed.
        operation: VolumeOperation,
        /// The underlying failure (kind `TimedOut` when the deadline passed).
        #[source]
        source: std::io::Error,
    },
}

/// The volume pool port. See the feature delta § Port for the behavioural
/// contract (idempotent `ensure`, `claim_state` projection rule).
#[async_trait]
pub trait VolumeBacking: Send + Sync {
    /// Create-if-absent and activate the workload's volume; return its device path.
    async fn ensure(
        &self,
        workload: &WorkloadId,
        size: VolumeSize,
    ) -> Result<PathBuf, VolumeBackingError>;

    /// One non-blocking attempt at the volume's exclusive claim (no persistent
    /// effect; returns without blocking).
    ///
    /// Makes one `O_EXCL` open of the volume's device-mapper node
    /// (`/dev/mapper/overdrive-ovd--<hash>`, the path `ensure` returns) and releases
    /// it at once: `Ok(Free)` iff it took the claim, `Ok(Held)` iff the open
    /// fails `EBUSY`. It does not take the OFD lock (a free claim implies a free
    /// lock, ADR-0317).
    ///
    /// An absent volume, or one that exists but is inactive (it has no device
    /// node, so the open fails `ENOENT`, as after every node reboot), is
    /// `Err(NotFound)`: `NotFound` does not distinguish "no such volume" from
    /// "present but inactive". Any other `open(2)` failure is
    /// `Err(PoolCommand { operation: VolumeOperation::Claim, source })`, where
    /// `Claim` names the `open(2)` of the device node, not an `lvm` command (no
    /// `lvm` is spawned by `claim_state`).
    ///
    /// The launch path treats any `Err` as the end of the launch; only
    /// `Ok(Held)` is polled again.
    async fn claim_state(&self, workload: &WorkloadId) -> Result<ClaimState, VolumeBackingError>;
}

/// The driven port the composition root calls to discover and probe the pool.
#[async_trait]
pub trait VolumePoolProbe: Send + Sync {
    /// Discover and probe the node's volume pool.
    ///
    /// Preconditions: none beyond a constructed probe; the node need not have a
    /// pool.
    ///
    /// - `Ok(None)` iff no partition has the label `overdrive-volumes`; nothing
    ///   is created or removed.
    /// - `Ok(Some(backing))` iff exactly one partition has the label and every
    ///   probe scenario passed; the backing is usable at once and addresses that
    ///   partition's volume group `overdrive`.
    /// - `Err(VolumeProbeError)` iff a labelled partition is present (or the
    ///   label is ambiguous) and a scenario failed; no backing is returned and
    ///   the node has no pool for this boot.
    ///
    /// Effects: it may create and remove one scratch volume named with the
    /// prefix `ovd-probe-`, and removes a leftover one first; it touches no
    /// other volume. On any exit, including `Err`, it leaves no scratch volume
    /// it created, except `ScratchRemoveFailed`, where the scratch volume is the
    /// thing that could not be removed.
    ///
    /// Repeatable: safe to call again after a crash or an `Err`; a second call
    /// after an `Ok` is a fresh probe with the same result on an unchanged node.
    async fn probe(&self) -> Result<Option<Arc<dyn VolumeBacking>>, VolumeProbeError>;
}

/// Which stage of the boot probe failed (ADR-0314).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VolumeProbeScenario {
    /// The `lvm` tools or device-mapper are unusable (also: a labelled partition
    /// matched by more than one device is `VolumeGroupMissing`, not this).
    ToolsOrDeviceMapperUnusable,
    /// The volume group is missing, or is not on the labelled partition.
    VolumeGroupMissing,
    /// The scratch volume could not be created, or its device node did not appear.
    ScratchCreateOrNodeFailed,
    /// The scratch volume could not be removed.
    ScratchRemoveFailed,
}

/// A present-but-broken volume pool: refuses startup (`volume_backing.probe`).
#[derive(Debug, thiserror::Error)]
#[error("volume pool probe failed ({scenario:?}): {detail}")]
pub struct VolumeProbeError {
    /// The failing stage.
    pub scenario: VolumeProbeScenario,
    /// The cause; the error's `source`.
    #[source]
    pub detail: VolumeProbeDetail,
}

/// The cause of a failed boot probe stage.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VolumeProbeDetail {
    /// A spawn, open or read failed.
    #[error("i/o failure: {0}")]
    Io(#[source] std::io::Error),
    /// An `lvm` command exited non-zero or was killed. `stderr` is the
    /// command's captured text, diagnostic only and never parsed.
    #[error("lvm command failed (exit status {exit_status:?}): {stderr}")]
    CommandFailed {
        /// The exit status; `None` when the command was killed by a signal.
        exit_status: Option<i32>,
        /// The command's captured standard error.
        stderr: String,
    },
    /// More than one partition carries the label `overdrive-volumes`.
    #[error("label matched by more than one device: {nodes:?}")]
    AmbiguousLabel {
        /// The matching device nodes.
        nodes: Vec<PathBuf>,
    },
    /// The scratch volume's device node did not appear.
    #[error("scratch volume device node did not appear: {path:?}")]
    NodeMissing {
        /// The expected node.
        path: PathBuf,
    },
}
