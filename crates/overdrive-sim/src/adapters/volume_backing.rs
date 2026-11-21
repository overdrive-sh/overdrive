//! `SimVolumeBacking` — the simulation twin of the volume pool port.
//!
//! GH #348. It models the observable behaviour of `LvmVolumeBacking` exactly: an idempotent
//! `ensure` that creates then activates, a `claim_state` that reports `NotFound` for an absent or
//! inactive volume, and the same device-node name. It holds no timer and models no `ensure`
//! deadline: the adapter owns that bound, and the sim returns by itself. The inspection and
//! fault-injection methods are test-support surface (`.claude/rules/design.md` scope note). A
//! backing has no "pool absent" route: it exists only after a probe found a pool; a node without
//! one composes `SimVolumePoolProbe::absent()` and the driver holds no backing.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use overdrive_core::id::WorkloadId;
use overdrive_core::traits::volume_backing::{
    ClaimState, VolumeBacking, VolumeBackingError, VolumePoolProbe, VolumeProbeDetail,
    VolumeProbeError,
};
use overdrive_core::volume::{ByteCount, VolumeOperation, VolumeSize};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

/// Hex characters of the workload id's SHA-256 that name its volume (ADR-0312).
const NAME_HEX_CHARS: usize = 32;

/// Who holds a volume's exclusive claim.
#[derive(Clone, Default)]
enum Holder {
    /// Nobody has the device open.
    #[default]
    None,
    /// A holder whose claim outlives it until a `drain`.
    Held,
    /// A holder whose claim lasts while `live()` is true.
    WhileLive(Arc<dyn Fn() -> bool + Send + Sync>),
}

impl fmt::Debug for Holder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => f.write_str("None"),
            Self::Held => f.write_str("Held"),
            Self::WhileLive(_) => f.write_str("WhileLive"),
        }
    }
}

/// One logical volume: created once, active while it has a device node.
#[derive(Debug, Default)]
struct Volume {
    active: bool,
    holder: Holder,
}

/// The modelled pool.
#[derive(Debug)]
struct Pool {
    free: u64,
    volumes: BTreeMap<WorkloadId, Volume>,
    volumes_created: usize,
    /// Workloads whose next `ensure` parks before activation.
    pause_armed: Vec<WorkloadId>,
    /// Workloads with an `ensure` parked before activation.
    parked: Vec<WorkloadId>,
    fail_next_command: bool,
    fail_next_claim_state: bool,
}

/// In-memory volume pool with scripted faults. Cheap to clone; clones share state.
#[derive(Clone, Debug)]
pub struct SimVolumeBacking {
    pool: Arc<Mutex<Pool>>,
    support: Arc<CallSupport>,
}

/// Test support (launch supersession acceptance tests): counts the port calls the driver makes,
/// and parks one `claim_state` call in flight until the test releases it.
#[derive(Debug, Default)]
struct CallSupport {
    ensure_calls: AtomicUsize,
    claim_state_calls: AtomicUsize,
    park_claim_state: AtomicBool,
    claim_state_parked: AtomicBool,
    claim_state_release: tokio::sync::Notify,
    claim_state_released: AtomicBool,
    ensure_resume: tokio::sync::Notify,
}

/// The device-mapper node of the workload's volume: `/dev/mapper/overdrive-ovd--<hex>`, the dm
/// name `<vg>-<lv>` with the `-` of the `ovd-` prefix doubled (the hex has none).
fn device_node(workload: &WorkloadId) -> PathBuf {
    let digest = Sha256::digest(workload.as_str().as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        // Writing into a `String` cannot fail.
        let _ = write!(hex, "{byte:02x}");
    }
    PathBuf::from(format!("/dev/mapper/overdrive-ovd--{}", &hex[..NAME_HEX_CHARS]))
}

fn command_failure(operation: VolumeOperation) -> VolumeBackingError {
    VolumeBackingError::PoolCommand {
        operation,
        source: std::io::Error::other("scripted lvm command failure"),
    }
}

impl SimVolumeBacking {
    /// A composed pool with `capacity` of free extents.
    #[must_use]
    pub fn with_capacity(capacity: VolumeSize) -> Self {
        let pool = Pool {
            free: capacity.bytes(),
            volumes: BTreeMap::new(),
            volumes_created: 0,
            pause_armed: Vec::new(),
            parked: Vec::new(),
            fail_next_command: false,
            fail_next_claim_state: false,
        };
        Self { pool: Arc::new(Mutex::new(pool)), support: Arc::new(CallSupport::default()) }
    }

    /// Test support: how many times the driver called `ensure` (every call counts, whatever its
    /// result).
    #[must_use]
    pub fn ensure_calls(&self) -> usize {
        self.support.ensure_calls.load(Ordering::SeqCst)
    }

    /// Test support: how many times the driver called `claim_state` (every call counts,
    /// including one still parked by `park_next_claim_state`).
    #[must_use]
    pub fn claim_state_calls(&self) -> usize {
        self.support.claim_state_calls.load(Ordering::SeqCst)
    }

    /// Test support: the next `claim_state` call parks in flight (after it is counted) until
    /// `release_claim_state`, then answers as `claim_state` normally does.
    pub fn park_next_claim_state(&self) {
        self.support.claim_state_released.store(false, Ordering::SeqCst);
        self.support.park_claim_state.store(true, Ordering::SeqCst);
    }

    /// Test support: a `claim_state` call is parked in flight (set by `park_next_claim_state`).
    #[must_use]
    pub fn claim_state_parked(&self) -> bool {
        self.support.claim_state_parked.load(Ordering::SeqCst)
    }

    /// Test support: release the parked `claim_state` call.
    pub fn release_claim_state(&self) {
        self.support.claim_state_released.store(true, Ordering::SeqCst);
        self.support.claim_state_release.notify_waiters();
    }

    /// Volumes created so far (a repeat `ensure` does not add one).
    #[must_use]
    pub fn volumes_created(&self) -> usize {
        self.pool.lock().volumes_created
    }

    /// A VMM opens the workload's device exclusively (the claim becomes `Held`). A volume with no
    /// device node (absent or inactive) cannot be opened: the call changes nothing.
    pub fn hold_claim(&self, workload: &WorkloadId) {
        self.set_holder(workload, Holder::Held);
    }

    /// A VMM process opens the workload's device and holds its claim for as long as that process
    /// is live: the claim reads `Held` while `live()` is true and `Free` once it is false (the
    /// process's death closes its open and its in-flight I/O has drained). A test links the claim
    /// to a `SimVmm` process with `move || vmm.is_live(pid)`. Models a VMM whose lifetime owns the
    /// claim, such as the orphan a crashed control plane leaves behind; `hold_claim` plus `drain`
    /// models a claim that outlives its holder. Test support: no port method carries it.
    pub fn hold_claim_while_live(
        &self,
        workload: &WorkloadId,
        live: impl Fn() -> bool + Send + Sync + 'static,
    ) {
        self.set_holder(workload, Holder::WhileLive(Arc::new(live)));
    }

    /// The holder's in-flight I/O drained: the claim is released.
    pub fn drain(&self, workload: &WorkloadId) {
        if let Some(volume) = self.pool.lock().volumes.get_mut(workload) {
            volume.holder = Holder::None;
        }
    }

    /// The next `ensure` for the workload completes only its create effect and parks before
    /// activation (the conformance driver steps `LaunchEnsureCreate` / `LaunchEnsureActivate`).
    pub fn pause_ensure_after_create(&self, workload: &WorkloadId) {
        self.pool.lock().pause_armed.push(workload.clone());
    }

    /// Release a parked `ensure` so it activates the volume. A pause armed but not yet reached is
    /// disarmed.
    pub fn resume_ensure(&self, workload: &WorkloadId) {
        {
            let mut pool = self.pool.lock();
            pool.pause_armed.retain(|armed| armed != workload);
            pool.parked.retain(|parked| parked != workload);
        }
        self.support.ensure_resume.notify_waiters();
    }

    /// The pool has no free extent (the model's `PoolFull`).
    pub fn exhaust_space(&self) {
        self.pool.lock().free = 0;
    }

    /// The next `lvm` command fails (the model's `LaunchCmdFail`).
    pub fn fail_next_command(&self) {
        self.pool.lock().fail_next_command = true;
    }

    /// The next `claim_state` fails with `PoolCommand { operation: Claim }`, an `open(2)` failure
    /// other than `EBUSY` and `ENOENT` (one-shot: the call after it answers normally).
    pub fn fail_next_claim_state(&self) {
        self.pool.lock().fail_next_claim_state = true;
    }

    /// Node reboot: every volume is inactive (no device node) and every claim released.
    pub fn node_reboot(&self) {
        for volume in self.pool.lock().volumes.values_mut() {
            volume.active = false;
            volume.holder = Holder::None;
        }
    }

    /// A launch's `ensure` for `workload` is parked after its create effect
    /// (set by `pause_ensure_after_create`, cleared by `resume_ensure`). The
    /// conformance driver waits on it to step `LaunchEnsureCreate`.
    #[must_use]
    pub fn ensure_parked(&self, workload: &WorkloadId) -> bool {
        self.pool.lock().parked.contains(workload)
    }

    fn set_holder(&self, workload: &WorkloadId, holder: Holder) {
        if let Some(volume) = self.pool.lock().volumes.get_mut(workload).filter(|v| v.active) {
            volume.holder = holder;
        }
    }

    /// The report and create steps of `ensure`; the volume exists (inactive or active) on `Ok`.
    fn report_and_create(
        &self,
        workload: &WorkloadId,
        size: VolumeSize,
    ) -> Result<(), VolumeBackingError> {
        let mut pool = self.pool.lock();
        if std::mem::take(&mut pool.fail_next_command) {
            return Err(command_failure(VolumeOperation::Report));
        }
        if pool.volumes.contains_key(workload) {
            return Ok(());
        }
        if size.bytes() > pool.free {
            return Err(VolumeBackingError::InsufficientSpace {
                needed: size,
                free: ByteCount::new(pool.free),
            });
        }
        pool.free -= size.bytes();
        pool.volumes.insert(workload.clone(), Volume::default());
        pool.volumes_created += 1;
        drop(pool);
        Ok(())
    }

    /// Park before activation when a pause is armed for `workload`, until `resume_ensure`.
    async fn park_if_armed(&self, workload: &WorkloadId) {
        {
            let mut pool = self.pool.lock();
            let Some(position) = pool.pause_armed.iter().position(|armed| armed == workload) else {
                return;
            };
            pool.pause_armed.remove(position);
            pool.parked.push(workload.clone());
        }
        loop {
            let resumed = self.support.ensure_resume.notified();
            tokio::pin!(resumed);
            resumed.as_mut().enable();
            if !self.ensure_parked(workload) {
                return;
            }
            resumed.await;
        }
    }
}

#[async_trait]
impl VolumeBacking for SimVolumeBacking {
    async fn ensure(
        &self,
        workload: &WorkloadId,
        size: VolumeSize,
    ) -> Result<PathBuf, VolumeBackingError> {
        self.support.ensure_calls.fetch_add(1, Ordering::SeqCst);
        self.report_and_create(workload, size)?;
        self.park_if_armed(workload).await;
        if let Some(volume) = self.pool.lock().volumes.get_mut(workload) {
            volume.active = true;
        }
        Ok(device_node(workload))
    }

    async fn claim_state(&self, workload: &WorkloadId) -> Result<ClaimState, VolumeBackingError> {
        self.support.claim_state_calls.fetch_add(1, Ordering::SeqCst);
        if self.support.park_claim_state.swap(false, Ordering::SeqCst) {
            self.support.claim_state_parked.store(true, Ordering::SeqCst);
            loop {
                let released = self.support.claim_state_release.notified();
                tokio::pin!(released);
                released.as_mut().enable();
                if self.support.claim_state_released.load(Ordering::SeqCst) {
                    break;
                }
                released.await;
            }
            self.support.claim_state_parked.store(false, Ordering::SeqCst);
        }
        let holder = {
            let mut pool = self.pool.lock();
            if std::mem::take(&mut pool.fail_next_claim_state) {
                return Err(command_failure(VolumeOperation::Claim));
            }
            match pool.volumes.get(workload) {
                Some(volume) if volume.active => volume.holder.clone(),
                _ => return Err(VolumeBackingError::NotFound { workload: workload.clone() }),
            }
        };
        let held = match holder {
            Holder::None => false,
            Holder::Held => true,
            Holder::WhileLive(live) => live(),
        };
        Ok(if held { ClaimState::Held } else { ClaimState::Free })
    }
}

/// The probe twin of [`SimVolumeBacking`]: what the composition root receives
/// from `VmAdapters.volume_pool` in a simulated node.
#[derive(Debug)]
pub struct SimVolumePoolProbe {
    outcome: ProbeOutcome,
}

#[derive(Debug)]
enum ProbeOutcome {
    Composed(SimVolumeBacking),
    Absent,
    Failing(VolumeProbeError),
}

impl SimVolumePoolProbe {
    /// A node whose pool passed its probe: `probe()` yields `Some` backing over `backing`.
    #[must_use]
    pub const fn composed(backing: SimVolumeBacking) -> Self {
        Self { outcome: ProbeOutcome::Composed(backing) }
    }

    /// A node with no labelled partition: `probe()` yields `None`.
    #[must_use]
    pub const fn absent() -> Self {
        Self { outcome: ProbeOutcome::Absent }
    }

    /// A node whose present pool is broken: `probe()` yields `error`.
    #[must_use]
    pub const fn failing(error: VolumeProbeError) -> Self {
        Self { outcome: ProbeOutcome::Failing(error) }
    }
}

#[async_trait]
impl VolumePoolProbe for SimVolumePoolProbe {
    async fn probe(&self) -> Result<Option<Arc<dyn VolumeBacking>>, VolumeProbeError> {
        match &self.outcome {
            ProbeOutcome::Composed(backing) => {
                Ok(Some(Arc::new(backing.clone()) as Arc<dyn VolumeBacking>))
            }
            ProbeOutcome::Absent => Ok(None),
            ProbeOutcome::Failing(error) => {
                Err(VolumeProbeError { scenario: error.scenario, detail: replay(&error.detail) })
            }
        }
    }
}

/// The probe may be called again, and an `io::Error` is not `Clone`: the scripted detail is
/// replayed field for field (an `Io` keeps its kind and text). A variant added to the
/// `#[non_exhaustive]` detail later is replayed as an `Io` carrying its rendered text.
fn replay(detail: &VolumeProbeDetail) -> VolumeProbeDetail {
    match detail {
        VolumeProbeDetail::Io(error) => {
            VolumeProbeDetail::Io(std::io::Error::new(error.kind(), error.to_string()))
        }
        VolumeProbeDetail::CommandFailed { exit_status, stderr } => {
            VolumeProbeDetail::CommandFailed { exit_status: *exit_status, stderr: stderr.clone() }
        }
        VolumeProbeDetail::AmbiguousLabel { nodes } => {
            VolumeProbeDetail::AmbiguousLabel { nodes: nodes.clone() }
        }
        VolumeProbeDetail::NodeMissing { path } => {
            VolumeProbeDetail::NodeMissing { path: path.clone() }
        }
        other => VolumeProbeDetail::Io(std::io::Error::other(other.to_string())),
    }
}
