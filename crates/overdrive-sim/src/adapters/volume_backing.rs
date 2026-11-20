//! `SimVolumeBacking` — the simulation twin of the volume pool port.
//!
//! GH #348. RED scaffold (DISTILL): the twin holds no state and scripts nothing until DELIVER
//! implements it. Its fault-injection methods are accepted and ignored; `ensure` fails with a
//! typed `PoolCommand` naming the scaffold and `claim_state` reports `NotFound`, so a test or a
//! conformance step that drives it reaches its own assertion and fails there. The inspection
//! and fault-injection methods are test-support surface, which DISTILL shapes
//! (`.claude/rules/design.md` scope note); the design pins only that the sim offers "claim held
//! until a `drain` step, free-space exhaustion, command failure" and reports `NotFound` while a
//! volume is inactive. A backing has no "pool absent" route: it exists only after a probe found a
//! pool; a node without one composes `SimVolumePoolProbe::absent()` and the driver holds no backing.

#![allow(
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_value,
    clippy::unused_async,
    clippy::unused_self,
    reason = "DISTILL RED scaffold"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use overdrive_core::id::WorkloadId;
use overdrive_core::traits::volume_backing::{
    ClaimState, VolumeBacking, VolumeBackingError, VolumePoolProbe, VolumeProbeDetail,
    VolumeProbeError,
};
use overdrive_core::volume::{VolumeOperation, VolumeSize};

const RED: &str = "SimVolumeBacking contract not implemented (GH #348, ADR-0312)";

/// In-memory volume pool with scripted faults. Cheap to clone; clones share state.
#[derive(Clone, Debug)]
pub struct SimVolumeBacking {
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
}

impl SimVolumeBacking {
    /// A composed pool with `capacity` of free extents.
    #[must_use]
    pub fn with_capacity(_capacity: VolumeSize) -> Self {
        Self { support: Arc::new(CallSupport::default()) }
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
        0
    }

    /// A VMM opens the workload's device exclusively (the claim becomes `Held`).
    pub fn hold_claim(&self, _workload: &WorkloadId) {}

    /// A VMM process opens the workload's device and holds its claim for as long as that process
    /// is live: the claim reads `Held` while `live()` is true and `Free` once it is false (the
    /// process's death closes its open and its in-flight I/O has drained). A test links the claim
    /// to a `SimVmm` process with `move || vmm.is_live(pid)`. Models a VMM whose lifetime owns the
    /// claim, such as the orphan a crashed control plane leaves behind; `hold_claim` plus `drain`
    /// models a claim that outlives its holder. Test support: no port method carries it.
    pub fn hold_claim_while_live(
        &self,
        _workload: &WorkloadId,
        _live: impl Fn() -> bool + Send + Sync + 'static,
    ) {
    }

    /// The holder's in-flight I/O drained: the claim is released.
    pub fn drain(&self, _workload: &WorkloadId) {}

    /// The next `ensure` for the workload completes only its create effect and parks before
    /// activation (the conformance driver steps `LaunchEnsureCreate` / `LaunchEnsureActivate`).
    pub fn pause_ensure_after_create(&self, _workload: &WorkloadId) {}

    /// Release a parked `ensure` so it activates the volume.
    pub fn resume_ensure(&self, _workload: &WorkloadId) {}

    /// The pool has no free extent (the model's `PoolFull`).
    pub fn exhaust_space(&self) {}

    /// The next `lvm` command fails (the model's `LaunchCmdFail`).
    pub fn fail_next_command(&self) {}

    /// The next `claim_state` fails with `PoolCommand { operation: Claim }`, an `open(2)` failure
    /// other than `EBUSY` and `ENOENT` (one-shot: the call after it answers normally).
    pub fn fail_next_claim_state(&self) {}

    /// Node reboot: every volume is inactive (no device node) and every claim released.
    pub fn node_reboot(&self) {}

    // ----- conformance-driver inspection (test support, RED scaffold) -----

    /// A launch's `ensure` for `workload` is parked after its create effect
    /// (set by `pause_ensure_after_create`, cleared by `resume_ensure`). The
    /// conformance driver waits on it to step `LaunchEnsureCreate`.
    #[must_use]
    pub fn ensure_parked(&self, _workload: &WorkloadId) -> bool {
        false
    }
}

#[async_trait]
impl VolumeBacking for SimVolumeBacking {
    async fn ensure(
        &self,
        _workload: &WorkloadId,
        _size: VolumeSize,
    ) -> Result<PathBuf, VolumeBackingError> {
        self.support.ensure_calls.fetch_add(1, Ordering::SeqCst);
        Err(VolumeBackingError::PoolCommand {
            operation: VolumeOperation::Create,
            source: std::io::Error::other(RED),
        })
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
        Err(VolumeBackingError::NotFound { workload: workload.clone() })
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
